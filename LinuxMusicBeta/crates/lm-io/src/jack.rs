//! The JACK backend (interfaces.md, I-10, I-10.1, I-21.1; F-REC-125), over `lm-sys-jack`. It works with JACK2, JACK1
//! and PipeWire's JACK library (`pw-jack`): only the portable client API is used, no server-specific call.
//!
//! **A client with ports, not a device.** The device name is `"jack"` (the default server) or `"jack:<server>"`. The
//! backend opens one client ([`JackOptions::client_name`]), registers N audio inputs (0 to 32, [`JackOptions::inputs`]
//! or `DeviceConfig::capture_channels`) and 2 or more audio outputs (`playback_channels`), and, when the session gave
//! it a [`JackMidiShared`], the MIDI ports of [`crate::jackmidi`] on the same client. By default every port is
//! connected to the server's physical ports (the hardware: capture ports to our inputs, our outputs to the playback
//! ports), as the legacy `AudioJack` did when nothing was saved; [`JackOptions::auto_connect`] turns it off (the user
//! routes with a patchbay).
//!
//! **The server decides the clock.** The period and the sample rate are the server's: `DeviceConfig::rate` and
//! `period` are wishes JACK may not grant, and the negotiated ones come back in [`DeviceInfo`] (C7). A server at
//! another rate than the project's gets the edge resampler (only when `allow_edge_resampler`; otherwise
//! `io.device.rate_unsupported`), as in the other backends. The server's period may change while the client runs
//! (JACK calls the process callback with the new size): the cycle renders it in pieces of the period negotiated at
//! open. Input and output are one clock, so `drift_correction` has no meaning and is ignored.
//!
//! **Threads and the barrier (S16).** libjack runs the process callback on its own real-time thread. The callback
//! takes the engine (the [`CallbackHandle`], the planar buffers, the edge) from a mutex nobody else touches while the
//! client runs (`try_lock`: one atomic). `stop` marks the stop (the [`crate::CallbackGuard`] already is) and wakes the
//! control thread; **the next cycle releases the handle**, which hands the callback back, and from then on no
//! callback runs. `join` calls `jack_deactivate` (it returns after the last cycle) and `close` `jack_client_close`.
//! If no cycle comes (a stalled server), the control thread deactivates the client itself after a grace period and
//! releases the handle, so the barrier always passes. A **control thread** per start also: reports what the server
//! says (`Disconnected` on a shutdown, with the handle released at once; `Panicked`), reconnects the MIDI ports when
//! ports appear, and sleeps otherwise.
//!
//! **Xruns** from the server's xrun callback are counted on JACK's notification thread and reported by the next cycle
//! as `DeviceEvent::Xrun` (the delay, in frames, as `late_by`).
//!
//! **Latency** is read from the physical ports' latency ranges (capture: the hardware's, to the program; playback:
//! from the program to the converter), at least one period each way (a block written in a cycle leaves in the next),
//! scaled to the project's rate, plus the edge resampler's. The ranges are the server's own numbers, which is what
//! the delay compensation (PDC) and the loopback calibration start from.
//!
//! **Time (C6).** The block's host time is the cycle's first frame (JACK's cycle times) moved onto `CLOCK_MONOTONIC`.

use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lm_base::{Error, HostTime, Result, Role};
use lm_render::DeviceBlock;
use lm_sys_jack::{
    ActiveClient, Client, Cycle, Direction, LatencyMode, Notify, Port, PortFilter, PortKind, Process, jack_time,
};

use crate::backend::{Backend, CallbackHandle, DeviceConfig, DeviceEvent, DeviceInfo};
use crate::choice::{ChoiceCaps, DeviceChoice};
use crate::edge::{Edge, MAX_CHANNELS};
use crate::jackmidi::{self, JackMidiShared, MidiControl, MidiCycle};
use crate::mididev::CLIENT_NAME;
use crate::rt::RtBuffer;

/// The most audio inputs (and outputs) the client registers
pub const JACK_MAX_CHANNELS: u32 = 32;
/// Audio inputs registered when neither the options nor the configuration say
pub const JACK_DEFAULT_INPUTS: u32 = 2;
/// The scratch buffers hold at least this many frames per channel, so a server that grows its period while the
/// client runs never needs an allocation in the cycle
const IO_FRAMES_MIN: usize = 16_384;
/// How long the control thread waits for the cycle to release the handle after a stop, before it does it itself
const RELEASE_GRACE: Duration = Duration::from_millis(500);
/// The control thread looks at the process handler's panic flag this often
const POLL: Duration = Duration::from_millis(100);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What a JACK backend is made with
#[derive(Clone, Debug)]
pub struct JackOptions {
    /// The client's name on the server (JACK may change it if taken)
    pub client_name: String,
    /// Audio inputs when the configuration does not say (`capture_channels`); 0 to [`JACK_MAX_CHANNELS`]
    pub inputs: u32,
    /// Audio outputs when the configuration does not say (`playback_channels`); at least 2
    pub outputs: u32,
    /// Connect the ports to the server's physical ports when starting
    pub auto_connect: bool,
    /// The MIDI half, shared with the session's [`crate::JackMidiLink`]
    pub midi: Option<JackMidiShared>,
}

impl Default for JackOptions {
    fn default() -> Self {
        Self {
            client_name: CLIENT_NAME.to_owned(),
            inputs: JACK_DEFAULT_INPUTS,
            outputs: 2,
            auto_connect: true,
            midi: None,
        }
    }
}

/// The server a device name asks for: `jack` is the default server (`None`), `jack:<server>` a named one
pub(crate) fn parse_device(name: &str) -> Result<Option<String>> {
    if name == "jack" {
        return Ok(None);
    }
    match name.strip_prefix("jack:") {
        Some(server) if !server.is_empty() => Ok(Some(server.to_owned())),
        _ => Err(Error::new("io.device.not_found").arg("device", name)),
    }
}

/// Frames at one rate as frames at another, rounded
pub(crate) fn scale_frames(frames: u32, from_rate: u32, to_rate: u32) -> u32 {
    (f64::from(frames) * f64::from(to_rate) / f64::from(from_rate.max(1))).round() as u32
}

/// The latency of one direction in project frames: the server's number (at least one period) at the server's rate,
/// scaled, plus the edge's share. Zero when there are no channels
pub(crate) fn reported_latency(
    range_max: u32,
    period: u32,
    channels: usize,
    device_rate: u32,
    rate: u32,
    edge_share: u32,
) -> u32 {
    if channels == 0 {
        return 0;
    }
    scale_frames(range_max.max(period), device_rate, rate) + edge_share
}

/// What the control thread waits on
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WakeState {
    pub(crate) stop: bool,
    pub(crate) gone: bool,
    /// Ports appeared or went away, or the MIDI settings changed
    pub(crate) dirty: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Wake {
    state: Mutex<WakeState>,
    changed: Condvar,
}

impl Wake {
    pub(crate) fn set(&self, f: impl FnOnce(&mut WakeState)) {
        let mut s = lock(&self.state);
        f(&mut s);
        self.changed.notify_all();
    }

    /// Waits up to `timeout` for something to be set; returns what is set and clears `dirty`
    fn wait(&self, timeout: Duration) -> WakeState {
        let mut s = lock(&self.state);
        if !s.stop && !s.gone && !s.dirty {
            s = self.changed.wait_timeout(s, timeout).unwrap_or_else(|p| p.into_inner()).0;
        }
        let snapshot = *s;
        s.dirty = false;
        snapshot
    }
}

/// What the notification thread tells the cycle
#[derive(Debug)]
struct Shared {
    xruns: AtomicU32,
    xrun_usecs: AtomicU64,
    wake: Arc<Wake>,
}

struct JackNotify(Arc<Shared>);

impl fmt::Debug for JackNotify {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JackNotify")
    }
}

impl Notify for JackNotify {
    fn xrun(&self, delayed_usecs: f32) {
        self.0.xrun_usecs.fetch_add(delayed_usecs.max(0.0) as u64, Ordering::Relaxed);
        self.0.xruns.fetch_add(1, Ordering::Relaxed);
    }

    fn shutdown(&self, _reason: &str) {
        self.0.wake.set(|s| s.gone = true);
    }

    fn port_registration(&self, _name: &str, _registered: bool) {
        self.0.wake.set(|s| s.dirty = true);
    }
}

/// What `open` made, moved to the engine by `start`
struct Opened {
    info: DeviceInfo,
    client: Client,
    in_ports: Vec<Port>,
    out_ports: Vec<Port>,
    midi: Option<(JackMidiShared, jackmidi::MidiPorts)>,
    edge: Option<Edge>,
    physical_in: Vec<String>,
    physical_out: Vec<String>,
}

/// What `start` made: the client, the engine the cycle and the control thread share, the control thread
struct Running {
    wake: Arc<Wake>,
    engine: Arc<Mutex<Engine>>,
    active: Arc<Mutex<Option<ActiveClient>>>,
    thread: Option<JoinHandle<()>>,
    midi: Option<JackMidiShared>,
}

/// The JACK backend
pub struct JackBackend {
    options: JackOptions,
    open: Option<Opened>,
    run: Option<Running>,
}

impl fmt::Debug for JackBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JackBackend")
            .field("options", &self.options)
            .field("open", &self.open.as_ref().map(|o| &o.info))
            .field("running", &self.run.is_some())
            .finish()
    }
}

impl Default for JackBackend {
    fn default() -> Self {
        Self::new(JackOptions::default())
    }
}

impl JackBackend {
    pub fn new(options: JackOptions) -> Self {
        Self { options, open: None, run: None }
    }
}

/// The server a configuration asks for: both directions must name the same one
fn server_of(config: &DeviceConfig) -> Result<Option<String>> {
    let play = config.playback.as_deref().map(parse_device).transpose()?;
    let cap = config.capture.as_deref().map(parse_device).transpose()?;
    match (play, cap) {
        (Some(p), Some(c)) if p != c => Err(Error::new("io.device.two_cards")
            .arg("playback", config.playback.clone().unwrap_or_default())
            .arg("capture", config.capture.clone().unwrap_or_default())),
        (Some(p), _) => Ok(p),
        (None, Some(c)) => Ok(c),
        (None, None) => Err(Error::new("io.device.bad_config")),
    }
}

impl Backend for JackBackend {
    fn open(&mut self, config: &DeviceConfig) -> Result<DeviceInfo> {
        if self.run.is_some() {
            return Err(Error::new("io.device.running"));
        }
        self.open = None;
        let server = server_of(config)?;
        let inputs = if config.capture.is_some() {
            config.capture_channels.unwrap_or(self.options.inputs).min(JACK_MAX_CHANNELS) as usize
        } else {
            0
        };
        let outputs = if config.playback.is_some() {
            config.playback_channels.unwrap_or(self.options.outputs).clamp(2, JACK_MAX_CHANNELS) as usize
        } else {
            0
        };
        let mut client = Client::open(&self.options.client_name, server.as_deref())?;
        let device_rate = client.sample_rate().max(1);
        if device_rate != config.rate && !config.allow_edge_resampler {
            return Err(Error::new("io.device.rate_unsupported").arg("rate", i64::from(config.rate)));
        }
        let period = client.buffer_size().max(1);
        let mut in_ports = Vec::with_capacity(inputs);
        for i in 0..inputs {
            in_ports.push(client.register(&format!("in_{}", i + 1), PortKind::Audio, Direction::Input)?);
        }
        let mut out_ports = Vec::with_capacity(outputs);
        for i in 0..outputs {
            out_ports.push(client.register(&format!("out_{}", i + 1), PortKind::Audio, Direction::Output)?);
        }
        let midi = match &self.options.midi {
            Some(shared) => Some((shared.clone(), jackmidi::register_ports(&mut client)?)),
            None => None,
        };
        // The hardware's ports, in the server's order (what the user sees as capture_1, playback_1, ...)
        let physical_in = client.ports(PortFilter {
            kind: Some(PortKind::Audio),
            direction: Some(Direction::Output),
            physical: true,
        });
        let physical_out =
            client.ports(PortFilter { kind: Some(PortKind::Audio), direction: Some(Direction::Input), physical: true });
        let capture_range =
            physical_in.first().and_then(|n| client.port_latency(n, LatencyMode::Capture)).map_or(period, |r| r.max);
        let playback_range =
            physical_out.first().and_then(|n| client.port_latency(n, LatencyMode::Playback)).map_or(period, |r| r.max);
        let edge = if device_rate != config.rate {
            Some(Edge::new(config.rate, device_rate, inputs, outputs, period as usize * 2, false)?)
        } else {
            None
        };
        let edge_latency = edge.as_ref().map_or(0, Edge::latency);
        let periods = config.periods.clamp(2, 8);
        let info = DeviceInfo {
            name: config.name(),
            rate: config.rate,
            device_rate,
            period,
            periods,
            buffer: period * periods,
            max_block: edge.as_ref().map_or(period as usize, Edge::block),
            inputs,
            outputs,
            input_latency: reported_latency(capture_range, period, inputs, device_rate, config.rate, edge_latency / 2),
            output_latency: reported_latency(
                playback_range,
                period,
                outputs,
                device_rate,
                config.rate,
                edge_latency - edge_latency / 2,
            ),
            edge_resampler: device_rate != config.rate,
            drift_correction: false,
            // One server, one clock
            linked: inputs > 0 && outputs > 0,
        };
        self.open =
            Some(Opened { info: info.clone(), client, in_ports, out_ports, midi, edge, physical_in, physical_out });
        Ok(info)
    }

    fn start(&mut self, callback: CallbackHandle) -> Result<()> {
        if self.run.is_some() {
            return Err(Error::new("io.device.running"));
        }
        let Some(opened) = self.open.take() else {
            return Err(Error::new("io.device.not_open"));
        };
        let Opened { info, client, in_ports, out_ports, midi, edge, physical_in, physical_out } = opened;
        let period = info.period as usize;
        let io_frames = period.max(IO_FRAMES_MIN);
        let wake = Arc::new(Wake::default());
        let shared = Arc::new(Shared { xruns: AtomicU32::new(0), xrun_usecs: AtomicU64::new(0), wake: wake.clone() });
        let (midi_cycle, midi_control) = match midi {
            Some((hub, ports)) => (Some(ports.cycle(&hub)), Some((hub, ports))),
            None => (None, None),
        };
        let own_names: Vec<String> = in_ports.iter().chain(out_ports.iter()).map(|p| client.port_name(*p)).collect();
        let engine = Arc::new(Mutex::new(Engine {
            handle: Some(callback),
            in_ports: in_ports.clone(),
            out_ports: out_ports.clone(),
            inputs: (0..in_ports.len()).map(|_| RtBuffer::zeroed(io_frames)).collect(),
            outputs: (0..out_ports.len()).map(|_| RtBuffer::zeroed(io_frames)).collect(),
            io_frames,
            cap: period,
            device_rate: info.device_rate,
            edge,
            shared: shared.clone(),
            midi: midi_cycle,
            lost: 0,
            late: 0,
            role_set: false,
        }));
        let activated =
            client.activate(Box::new(EngineProcess { engine: engine.clone() }), Arc::new(JackNotify(shared)));
        let active = match activated {
            Ok(a) => a,
            Err((e, _client)) => {
                // The callback comes back: the engine (and with it the handle) goes
                drop(engine);
                return Err(e);
            }
        };
        // The connections: the hardware's ports by default (not an error if the server has fewer than we have)
        if self.options.auto_connect {
            let c = active.client();
            let n_in = in_ports.len();
            for (src, dst) in physical_in.iter().zip(own_names.iter().take(n_in)) {
                let _ = c.connect(src, dst);
            }
            for (src, dst) in own_names.iter().skip(n_in).zip(physical_out.iter()) {
                let _ = c.connect(src, dst);
            }
        }
        if let Some((hub, _)) = &midi_control {
            hub.set_wake(Some(wake.clone()));
        }
        if let Ok(mut e) = engine.lock() {
            if let Some(h) = e.handle.as_mut() {
                h.report(DeviceEvent::Started { realtime: None });
            }
        }
        let active = Arc::new(Mutex::new(Some(active)));
        let hub = midi_control.as_ref().map(|(h, _)| h.clone());
        let control = midi_control.map(|(hub, ports)| ports.control(&hub));
        let thread = {
            let (wake, engine, active) = (wake.clone(), engine.clone(), active.clone());
            std::thread::Builder::new()
                .name("lm-jack-control".to_owned())
                .spawn(move || control_loop(&wake, &engine, &active, control))
                .map_err(|e| Error::new("io.device.thread_failed").arg("detail", e.to_string()))?
        };
        self.run = Some(Running { wake, engine, active, thread: Some(thread), midi: hub });
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(r) = &self.run {
            r.wake.set(|s| s.stop = true);
        }
    }

    fn join(&mut self) {
        let Some(r) = self.run.as_mut() else { return };
        r.wake.set(|s| s.stop = true);
        if let Some(t) = r.thread.take() {
            let _ = t.join();
        }
        // jack_deactivate returns after the last cycle: from here no callback can run
        if let Some(a) = lock(&r.active).as_mut() {
            a.deactivate();
        }
        // (a no-op when a cycle released the handle already)
        lock(&r.engine).release();
        if let Some(m) = &r.midi {
            m.set_wake(None);
        }
    }

    fn close(&mut self) {
        self.open = None;
        if let Some(r) = self.run.take() {
            // jack_client_close
            drop(lock(&r.active).take());
            if let Some(m) = &r.midi {
                m.set_wake(None);
            }
        }
    }

    fn kind(&self) -> &'static str {
        "jack"
    }
}

/// The control thread: reports a shutdown or a panic, keeps the MIDI connections, and after a stop makes sure the
/// handle was released
fn control_loop(
    wake: &Wake,
    engine: &Mutex<Engine>,
    active: &Mutex<Option<ActiveClient>>,
    mut midi: Option<MidiControl>,
) {
    let reconcile = |midi: &mut Option<MidiControl>| {
        if let Some(m) = midi.as_mut()
            && let Some(a) = lock(active).as_ref()
        {
            m.reconcile(a.client());
        }
    };
    reconcile(&mut midi);
    loop {
        let state = wake.wait(POLL);
        if state.stop {
            // The next cycle releases the handle; a server that runs no cycles gets the barrier from here
            let deadline = Instant::now() + RELEASE_GRACE;
            while !lock(engine).released() {
                if Instant::now() >= deadline {
                    if let Some(a) = lock(active).as_mut() {
                        a.deactivate();
                    }
                    lock(engine).release();
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            return;
        }
        if state.gone {
            let mut e = lock(engine);
            e.report(DeviceEvent::Disconnected);
            // The server is gone: no cycle comes again, so the handle can only be released here
            e.release();
            return;
        }
        if lock(active).as_ref().is_some_and(ActiveClient::panicked) {
            let mut e = lock(engine);
            e.report(DeviceEvent::Panicked);
            e.release();
            return;
        }
        if state.dirty {
            reconcile(&mut midi);
        }
    }
}

/// JACK's real-time thread runs this
struct EngineProcess {
    engine: Arc<Mutex<Engine>>,
}

impl Process for EngineProcess {
    fn process(&mut self, cycle: &mut Cycle<'_>) {
        match self.engine.try_lock() {
            Ok(mut e) => e.cycle(cycle),
            // Held (the control thread releasing) or poisoned (a panic in an earlier cycle): silence this cycle
            Err(_) => {}
        }
    }
}

/// What runs inside the process callback: the callback, the planar buffers, the edge and the MIDI half
struct Engine {
    handle: Option<CallbackHandle>,
    in_ports: Vec<Port>,
    out_ports: Vec<Port>,
    inputs: Vec<RtBuffer>,
    outputs: Vec<RtBuffer>,
    /// Frames each buffer holds: the most a cycle can bring (a bigger one is cut, never allocated for)
    io_frames: usize,
    /// Frames of one render piece: the period negotiated at open
    cap: usize,
    device_rate: u32,
    edge: Option<Edge>,
    shared: Arc<Shared>,
    midi: Option<MidiCycle>,
    lost: u32,
    late: u32,
    role_set: bool,
}

impl Engine {
    /// The barrier (S16): dropping the handle runs the callback's `finish` and hands it back
    fn release(&mut self) {
        self.handle = None;
    }

    fn released(&self) -> bool {
        self.handle.is_none()
    }

    fn report(&mut self, event: DeviceEvent) {
        if let Some(h) = self.handle.as_mut() {
            h.report(event);
        }
    }

    /// One process cycle (JACK's real-time thread). Never allocates (the `CallbackHandle` runs the renderer under the
    /// allocation trap in debug builds)
    fn cycle(&mut self, c: &mut Cycle<'_>) {
        if !self.role_set {
            lm_base::set_role(Role::Audio);
            self.role_set = true;
        }
        match self.handle.as_ref() {
            None => return,
            Some(h) if h.should_stop() => {
                // The next callback after a stop releases the handle
                self.release();
                return;
            }
            Some(_) => {}
        }
        let rate = self.device_rate;
        let xruns = self.shared.xruns.swap(0, Ordering::Relaxed);
        if xruns > 0 {
            let usecs = self.shared.xrun_usecs.swap(0, Ordering::Relaxed);
            let late = u32::try_from(usecs * u64::from(rate) / 1_000_000).unwrap_or(u32::MAX).max(1);
            self.late = self.late.saturating_add(late);
            self.report(DeviceEvent::Xrun { capture_lost: 0, late_by: late });
        }
        let frames = (c.frames() as usize).min(self.io_frames);
        // C6: the cycle's first frame on the program's monotonic clock (JACK's clock differs; the difference now)
        let now = HostTime::now().0;
        let host0 = match c.times() {
            Some(t) => now.saturating_sub(jack_time().saturating_sub(t.usecs).saturating_mul(1000)),
            None => now,
        };
        if let Some(m) = &self.midi {
            m.input(c, host0, rate);
        }
        if frames > 0 {
            for (port, buf) in self.in_ports.iter().zip(self.inputs.iter_mut()) {
                c.read_audio(*port, &mut buf[..frames]);
            }
            let mut done = 0;
            while done < frames {
                let n = (frames - done).min(self.cap);
                let t = HostTime(host0 + done as u64 * 1_000_000_000 / u64::from(rate));
                self.render(t, done, n);
                done += n;
            }
            for (port, buf) in self.out_ports.iter().zip(self.outputs.iter()) {
                c.write_audio(*port, &buf[..frames]);
            }
        }
        let cycle_frames = c.frames();
        if let Some(m) = &self.midi {
            m.output(c, host0, cycle_frames, rate);
        }
    }

    /// Runs the renderer on `n` frames from `done` of the planar buffers (through the edge when the rates differ)
    fn render(&mut self, host_time: HostTime, done: usize, n: usize) {
        let (ch_in, ch_out) = (self.inputs.len().min(MAX_CHANNELS), self.outputs.len().min(MAX_CHANNELS));
        let mut src: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        for (s, b) in src.iter_mut().zip(self.inputs.iter()) {
            *s = &b[done..done + n];
        }
        let mut dst: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
        for (d, b) in dst.iter_mut().zip(self.outputs.iter_mut()) {
            *d = &mut b[done..done + n];
        }
        let Some(handle) = self.handle.as_mut() else { return };
        if let Some(edge) = self.edge.as_mut() {
            if self.lost > 0 {
                edge.add_capture_lost(std::mem::take(&mut self.lost));
            }
            if self.late > 0 {
                edge.add_late(std::mem::take(&mut self.late));
            }
            edge.push_capture(&src[..ch_in], n);
            edge.period(host_time, &mut dst[..ch_out], n, &mut |b| handle.call(b));
        } else {
            handle.call(DeviceBlock {
                frames: n,
                host_time,
                inputs: &src[..ch_in],
                outputs: &mut dst[..ch_out],
                capture_lost: std::mem::take(&mut self.lost),
                late_by: std::mem::take(&mut self.late),
            });
        }
    }
}

// ---- the list (U17) ----

/// Looks at the default server for the device list and the presence check: a client of its own, closed at once.
/// `None` when no server runs (not an error: JACK may simply not be in use)
fn probe(server: Option<&str>) -> Option<(Client, Vec<String>, Vec<String>)> {
    let client = Client::open(&format!("{CLIENT_NAME}-scan"), server).ok()?;
    let audio = |direction| {
        client.ports(PortFilter { kind: Some(PortKind::Audio), direction: Some(direction), physical: true })
    };
    let (capture, playback) = (audio(Direction::Output), audio(Direction::Input));
    Some((client, capture, playback))
}

/// The JACK device the machine offers (U17): one entry, `jack`, with the server's rate and the physical ports'
/// counts; empty when no server runs. Talks to the server: run it in an `Operation`
pub fn jack_list_devices(cancel: &lm_base::Cancel, progress: &lm_base::Progress) -> Result<Vec<DeviceChoice>> {
    cancel.check()?;
    progress.set(0.0);
    let Some((client, capture, playback)) = probe(None) else {
        progress.set(1.0);
        return Ok(Vec::new());
    };
    let rate = client.sample_rate();
    let period = client.buffer_size();
    let caps = |ports: &[String]| ChoiceCaps {
        channels_min: 1,
        channels_max: u32::try_from(ports.len()).unwrap_or(JACK_MAX_CHANNELS).clamp(2, JACK_MAX_CHANNELS),
        rates: vec![rate],
    };
    progress.set(1.0);
    Ok(vec![DeviceChoice {
        name: "jack".to_owned(),
        card: "JACK".to_owned(),
        description: format!("JACK ({rate} Hz, {period})"),
        playback: Some(caps(&playback)),
        capture: Some(caps(&capture)),
        busy: false,
        denied: false,
    }])
}

/// Whether the server the name asks for answers (U11: the periodic scan that offers to reconnect)
pub fn jack_device_present(name: &str) -> bool {
    match parse_device(name) {
        Ok(server) => probe(server.as_deref()).is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::sync_channel;

    use crate::backend::DeviceCallback;

    #[test]
    fn device_names_pick_the_server() {
        assert_eq!(parse_device("jack").unwrap(), None);
        assert_eq!(parse_device("jack:studio").unwrap().as_deref(), Some("studio"));
        assert!(parse_device("jack:").is_err());
        assert!(parse_device("hw:CARD=PCH,DEV=0").is_err());
        assert!(parse_device("jackal").is_err());
    }

    #[test]
    fn two_servers_in_one_configuration_are_refused() {
        let mut c = DeviceConfig::duplex("jack", 48_000, 256);
        assert_eq!(server_of(&c).unwrap(), None);
        c.capture = Some("jack:other".to_owned());
        assert_eq!(server_of(&c).unwrap_err().key(), "io.device.two_cards");
        c.capture = None;
        c.playback = None;
        assert!(server_of(&c).is_err());
    }

    #[test]
    fn latency_is_at_least_a_period_and_follows_the_project_rate() {
        // The server says 100 frames, the period is 256: a period
        assert_eq!(reported_latency(100, 256, 2, 48_000, 48_000, 0), 256);
        // The server says 512 at 48 kHz, the project runs at 96 kHz: twice the frames
        assert_eq!(reported_latency(512, 256, 2, 48_000, 96_000, 0), 1024);
        // The edge's share is added; no channels, no latency
        assert_eq!(reported_latency(512, 256, 2, 48_000, 48_000, 30), 542);
        assert_eq!(reported_latency(512, 256, 0, 48_000, 48_000, 30), 0);
        assert_eq!(scale_frames(441, 44_100, 48_000), 480);
    }

    #[test]
    fn the_wake_reports_what_was_set_and_clears_the_ports_flag() {
        let w = Wake::default();
        assert!(!w.wait(Duration::from_millis(1)).dirty);
        w.set(|s| s.dirty = true);
        assert!(w.wait(Duration::from_millis(1)).dirty);
        assert!(!w.wait(Duration::from_millis(1)).dirty);
        w.set(|s| s.gone = true);
        assert!(w.wait(Duration::from_millis(1)).gone);
        w.set(|s| s.stop = true);
        let s = w.wait(Duration::from_secs(5));
        assert!(s.stop && s.gone, "stop and gone stay set");
    }

    #[test]
    fn a_name_without_a_server_is_a_clear_error() {
        // (no server of this name runs anywhere)
        let mut backend = JackBackend::default();
        let config = DeviceConfig::duplex("jack:lm-no-such-server", 48_000, 256);
        let error = backend.open(&config).unwrap_err();
        assert!(error.key().starts_with("jack."), "{}", error.key());
        assert!(!jack_device_present("jack:lm-no-such-server"));
        assert!(!jack_device_present("hw:0"));
    }

    // ---- against a real server (skipped without jackd) ----

    struct Server {
        name: String,
        child: Child,
    }

    impl Server {
        fn start(tag: &str) -> Option<Server> {
            let name = format!("lmio-{}-{tag}", std::process::id());
            let child = Command::new("jackd")
                .args(["--no-realtime", "-n", &name, "-d", "dummy", "-r", "48000", "-p", "256", "-C", "2", "-P", "2"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let server = Server { name, child };
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if Client::open("lmio-probe", Some(&server.name)).is_ok() {
                    return Some(server);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            None
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Copies input 0 to every output and counts the cycles
    struct Loop(Arc<AtomicU64>);

    impl DeviceCallback for Loop {
        fn process(&mut self, block: DeviceBlock<'_, '_>) {
            for out in block.outputs.iter_mut() {
                match block.inputs.first() {
                    Some(src) => out[..block.frames].copy_from_slice(&src[..block.frames]),
                    None => out[..block.frames].fill(0.0),
                }
            }
            self.0.fetch_add(1, Ordering::Relaxed);
        }

        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send> {
            self
        }
    }

    #[test]
    fn it_runs_against_a_server_and_the_barrier_passes_on_stop() {
        let Some(server) = Server::start("run") else {
            eprintln!("jackd is not available: the JACK backend test is skipped");
            return;
        };
        let cycles = Arc::new(AtomicU64::new(0));
        let mut backend =
            JackBackend::new(JackOptions { client_name: "lmio-test".to_owned(), ..JackOptions::default() });
        let config = DeviceConfig::duplex(&format!("jack:{}", server.name), 48_000, 256);
        let info = backend.open(&config).expect("opens");
        assert_eq!((info.rate, info.device_rate, info.period), (48_000, 48_000, 256));
        assert_eq!((info.inputs, info.outputs), (2, 2));
        assert!(info.input_latency >= 256 && info.output_latency >= 256);
        let (back_tx, back_rx) = sync_channel(1);
        let (handle, _events, guard) = CallbackHandle::for_test(Box::new(Loop(cycles.clone())), back_tx);
        backend.start(handle).expect("starts");
        let deadline = Instant::now() + Duration::from_secs(10);
        while cycles.load(Ordering::Relaxed) < 10 {
            assert!(Instant::now() < deadline, "no cycles ran");
            std::thread::sleep(Duration::from_millis(10));
        }
        // The barrier: mark, stop, the callback comes back, nothing runs after
        guard.request_stop();
        backend.stop();
        let callback = back_rx.recv_timeout(Duration::from_secs(5)).expect("the callback came back");
        backend.join();
        let after = cycles.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(cycles.load(Ordering::Relaxed), after, "no callback runs after the barrier");
        drop(callback);
        backend.close();
    }

    #[test]
    fn a_server_that_goes_away_gives_disconnected_and_the_callback_back() {
        let Some(server) = Server::start("gone") else {
            eprintln!("jackd is not available: the JACK backend test is skipped");
            return;
        };
        let cycles = Arc::new(AtomicU64::new(0));
        let mut backend =
            JackBackend::new(JackOptions { client_name: "lmio-gone".to_owned(), ..JackOptions::default() });
        backend.open(&DeviceConfig::duplex(&format!("jack:{}", server.name), 48_000, 256)).expect("opens");
        let (back_tx, back_rx) = sync_channel(1);
        let (handle, mut events, _guard) = CallbackHandle::for_test(Box::new(Loop(cycles.clone())), back_tx);
        backend.start(handle).expect("starts");
        std::thread::sleep(Duration::from_millis(200));
        drop(server);
        let _callback = back_rx.recv_timeout(Duration::from_secs(10)).expect("the callback came back");
        let mut disconnected = false;
        while let Ok(e) = events.pop() {
            disconnected |= matches!(e, DeviceEvent::Disconnected);
        }
        assert!(disconnected, "the server's shutdown is reported");
        backend.stop();
        backend.join();
        backend.close();
    }
}
