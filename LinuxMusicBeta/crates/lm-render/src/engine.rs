//! The two halves of the engine (interfaces.md, I-5.1): `engine()` makes them, the main thread keeps the
//! [`EngineControl`], the device callback gets the [`Renderer`]

use std::sync::Arc;

use lm_base::ring::{Consumer, capacity_for, ring};
use lm_base::{Error, Result};

use crate::command::{COMMAND_RING_CAPACITY, CommandSender, command_ring};
use crate::exec::{Watch, Workers};
use crate::handoff::{HandoffSender, handoff};
use crate::ids::{ParamSlot, ProducerIndex};
use crate::install::{Install, Returned};
use crate::params::ParamSlots;
use crate::renderer::{Outbound, Renderer};
use crate::spec::{Limits, ProcessSpec, RenderThreads};
use crate::telemetry::{PanicReport, PreviewEnded, Telemetry, TelemetryFrame, Touched, TransportEvent};

/// The main thread's half: installs, parameter slots, the main command ring, and everything that comes back
pub struct EngineControl {
    spec: ProcessSpec,
    limits: Limits,
    installs: HandoffSender<Install>,
    params: ParamSlots,
    commands: CommandSender,
    frame: triple_buffer::Output<TelemetryFrame>,
    events: Consumer<Telemetry>,
    transport: Consumer<TransportEvent>,
    touched: Consumer<Touched>,
    previews: Consumer<PreviewEnded>,
    panics: Consumer<PanicReport>,
    watch: Arc<Watch>,
    /// Slots bound by the last program sent (to write the initial value of a slot that is new, S20)
    bound: Vec<u64>,
}

impl std::fmt::Debug for EngineControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineControl")
            .field("spec", &self.spec)
            .field("in_flight", &self.installs.in_flight())
            .field("queued", &self.installs.queued())
            .finish()
    }
}

/// Makes an engine whose render threads run without a hook (tests, export without real time)
pub fn engine(spec: ProcessSpec, limits: Limits) -> (EngineControl, Renderer) {
    engine_with(spec, limits, RenderThreads::default())
}

/// Makes an engine. Spawns `limits.workers` render workers, each running inside `threads`' hook (H1, H6).
///
/// The ring sizes, each for 10 s of a stopped pump (C4, H2), with `b = sample_rate / max_block` blocks per second:
/// - **events** ([`Telemetry`], loses and counts): `4096`. At most a few per block in steady state (an xrun, a
///   request), so 10 s of small blocks can exceed it; the loss is counted and reported (`Dropped`), which C4
///   allows for this ring;
/// - **transport events** (lossless): `10 · (b + 3 · 20 + 10)`, at least 1024. One capture gap per block at most,
///   and at most 20 loop wraps per second (the loop is at least 50 ms long) with a punch-in and a punch-out each,
///   plus starts and stops;
/// - **touched values** (lossless, an explicit stop when full, C3): `10 · b · 8`, at least 4096: one value per
///   touched slot per block, for 8 gestures at once (more at once is the explicit error);
/// - **finished previews** (lossless): `max_previews`; a preview slot holds its return until there is room;
/// - **panic reports** (lossless): `max_nodes · (installs_in_flight + 1) + max_previews`: a dead node is never
///   called again, so only processors installed since the last collect can panic;
/// - **installs and returns**: `installs_in_flight` each (see `handoff.rs`).
pub fn engine_with(spec: ProcessSpec, limits: Limits, threads: RenderThreads) -> (EngineControl, Renderer) {
    assert!(spec.max_block > 0 && spec.sample_rate > 0, "a ProcessSpec with no rate or no block");
    assert!(limits.max_command_producers > 0 && limits.max_nodes > 0, "limits with no room");
    let blocks_per_second = f64::from(spec.sample_rate) / spec.max_block as f64;

    let (installs_tx, installs_rx) = handoff(limits.installs_in_flight);
    let params = ParamSlots::new(limits.max_params);
    let (commands, main_rx) = command_ring(ProducerIndex(0), COMMAND_RING_CAPACITY);
    let (frame_in, frame_out) = triple_buffer::triple_buffer(&TelemetryFrame::new(limits.max_nodes, limits.max_params));
    let (events_tx, events_rx) = ring(4096);
    let (transport_tx, transport_rx) = ring(capacity_for(blocks_per_second + 70.0, 10.0, 1024));
    let (touched_tx, touched_rx) = ring(capacity_for(blocks_per_second * 8.0, 10.0, 4096));
    let (previews_tx, previews_rx) = ring(limits.max_previews.max(1));
    let (panics_tx, panics_rx) = ring(limits.max_nodes * (limits.installs_in_flight + 1) + limits.max_previews);
    let watch = Arc::new(Watch::new(limits.workers + 1));
    let workers = Workers::spawn(limits.workers, &threads, &watch);

    let out = Outbound {
        frame: frame_in,
        events: events_tx,
        transport: transport_tx,
        touched: touched_tx,
        previews: previews_tx,
        panics: panics_tx,
    };
    let renderer = Renderer::new(spec, limits, installs_rx, main_rx, params.clone(), out, workers, watch.clone());
    let control = EngineControl {
        spec,
        limits,
        installs: installs_tx,
        params,
        commands,
        frame: frame_out,
        events: events_rx,
        transport: transport_rx,
        touched: touched_rx,
        previews: previews_rx,
        panics: panics_rx,
        watch,
        bound: vec![0; limits.max_params.div_ceil(64)],
    };
    (control, renderer)
}

impl EngineControl {
    pub fn spec(&self) -> ProcessSpec {
        self.spec
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Sends an install. Never blocks: past `installs_in_flight` it waits in a main-thread queue (I-5.3).
    ///
    /// Checks first that the program was built for this engine and that every index is inside the limits
    /// (`render.install.program_limits`, `render.install.node_out_of_range`, `render.install.producer_invalid`);
    /// reserves the room for what will come back (D2); writes the initial value of every slot the previous program
    /// did not bind (S20)
    pub fn install(&mut self, mut install: Install) -> Result<()> {
        if let Some(p) = &install.program
            && (p.node_map.len() != self.limits.max_nodes
                || p.slot_map.len() != self.limits.max_params
                || p.spec != self.spec)
        {
            return Err(Error::new("render.install.program_limits"));
        }
        let max = self.limits.max_nodes;
        let bad_node = install.add.iter().map(|(n, _)| *n).chain(install.remove.iter().copied()).any(|n| n.slot() >= max)
            || install.replace.iter().any(|r| r.node.slot() >= max || r.tail.is_some_and(|t| t.slot() >= max));
        if bad_node {
            return Err(Error::new("render.install.node_out_of_range"));
        }
        let producers = self.limits.max_command_producers;
        let bad_producer = install.producers.iter().map(|r| r.producer()).chain(install.remove_producers.iter().copied())
            .any(|p| p.0 == 0 || usize::from(p.0) >= producers);
        if bad_producer {
            return Err(Error::new("render.install.producer_invalid"));
        }
        if let Some(p) = &install.program {
            let mut now = vec![0u64; self.bound.len()];
            for (slot, initial) in p.param_initials() {
                let s = slot.0 as usize;
                now[s / 64] |= 1 << (s % 64);
                if self.bound[s / 64] & (1 << (s % 64)) == 0 {
                    self.params.write(slot, initial);
                }
            }
            self.bound = now;
        }
        install.reserve_returns();
        self.installs.send(Box::new(install));
        Ok(())
    }

    /// Takes everything the audio side gave back, in order, and sends what waited in the queue. `f` gets each
    /// install's [`Returned`] (to tell the compiler which indices are free, S6, and to send heavy things to a
    /// disposal thread, U15). Returns how many came back
    pub fn collect(&mut self, mut f: impl FnMut(Returned)) -> usize {
        self.installs.collect(|mut install| f(std::mem::take(&mut install.returned)))
    }

    /// Installs on the audio side right now
    pub fn installs_in_flight(&self) -> usize {
        self.installs.in_flight()
    }

    /// Installs waiting in the main-thread queue
    pub fn installs_queued(&self) -> usize {
        self.installs.queued()
    }

    /// The parameter slots (cloneable: the session keeps one to write undo values, I-6.3)
    pub fn params(&self) -> &ParamSlots {
        &self.params
    }

    /// Writes a parameter slot
    pub fn param_write(&self, slot: ParamSlot, value: f32) {
        self.params.write(slot, value);
    }

    /// The main thread's command ring (producer 0)
    pub fn commands(&mut self) -> &mut CommandSender {
        &mut self.commands
    }

    /// The newest telemetry frame
    pub fn frame(&mut self) -> &TelemetryFrame {
        self.frame.read()
    }

    /// Drains the event ring
    pub fn events(&mut self, mut f: impl FnMut(Telemetry)) {
        while let Ok(e) = self.events.pop() {
            f(e);
        }
    }

    /// Drains the transport events (lossless)
    pub fn transport_events(&mut self, mut f: impl FnMut(TransportEvent)) {
        while let Ok(e) = self.transport.pop() {
            f(e);
        }
    }

    /// Drains the touched values (lossless, C3)
    pub fn touched(&mut self, mut f: impl FnMut(Touched)) {
        while let Ok(e) = self.touched.pop() {
            f(e);
        }
    }

    /// Drains the finished previews; dropping them frees what they played (S7)
    pub fn previews_ended(&mut self, mut f: impl FnMut(PreviewEnded)) {
        while let Ok(e) = self.previews.pop() {
            f(e);
        }
    }

    /// Drains the panics caught on the render threads; the session prints them (S11)
    pub fn panics(&mut self, mut f: impl FnMut(PanicReport)) {
        while let Ok(e) = self.panics.pop() {
            f(e);
        }
    }

    /// The watchdog's view (U21)
    pub fn watch(&self) -> Arc<Watch> {
        self.watch.clone()
    }
}
