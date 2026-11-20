//! The core as a GPUI entity: it owns the port (I-15) — the LocalPort of the session, or the RemotePort of a client,
//! or, in the tests, the fake — and nothing else reaches it. Every view holds an `Entity<Core>`, reads through
//! `core.read(cx).port()` in `render` and changes through `core.update(cx, |core, _| core.port_mut()...)` in its
//! handlers. Views keep no references into the core: only ids, feeds, gesture ids, operation ids and pendings.
//!
//! The pump: a task on GPUI's foreground executor, on a 10 ms timer of GPUI's clock that does not depend on frames
//! (H2). It notifies the entity only when something shown changed; the workspace observes the entity and redraws.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{Context, Task};
use lm_port::{CorePort, NodeId};

use crate::bench::Bench;
use crate::view;

pub struct Core {
    port: Box<dyn CorePort>,
    pub bench: Bench,
    /// The meters as drawn: the latest telemetry with the fall smoothed per frame (H16), by node
    pub shown: HashMap<NodeId, f32>,
    /// How many times the pump ran (the tests check H2 with it)
    pub pumps: u64,
    last_meters: Instant,
    _pump: Task<()>,
}

impl Core {
    pub fn new(port: Box<dyn CorePort>, cx: &mut Context<Self>) -> Core {
        let bench = Bench::from_env("lm-gui");
        let pump = cx.spawn(async move |this, cx| {
            let period = Duration::from_millis(10);
            // GPUI's clock, not Instant: the same code runs under the test scheduler's fake clock
            let clock = cx.background_executor().clone();
            let mut next = clock.now() + period;
            loop {
                let wait = next.saturating_duration_since(clock.now());
                clock.timer(wait).await;
                next += period;
                if next < clock.now() {
                    next = clock.now() + period;
                }
                if this.update(cx, |core, cx| core.on_pump(cx)).is_err() {
                    break;
                }
            }
        });
        let mut core = Core { port, bench, shown: HashMap::new(), pumps: 0, last_meters: Instant::now(), _pump: pump };
        if core.bench.wants_play() {
            let _ = core.port.run_action("transport.play".into(), lm_port::Value::NULL);
        }
        core
    }

    /// The core as the interface may use it: data in, data out, no closures (I-15)
    pub fn port(&self) -> &dyn CorePort {
        &*self.port
    }

    pub fn port_mut(&mut self) -> &mut dyn CorePort {
        &mut *self.port
    }

    fn on_pump(&mut self, cx: &mut Context<Self>) {
        self.pumps += 1;
        let changed = self.port.pump();
        if self.bench.tick() {
            cx.quit();
            return;
        }
        if changed || self.shown.values().any(|m| *m > 0.0) {
            cx.notify();
        }
    }

    /// Once per frame, from the workspace's render
    pub fn advance_meters(&mut self, now: Instant) {
        let dt = now.duration_since(self.last_meters).as_secs_f32().min(0.1);
        self.last_meters = now;
        let t = self.port.telemetry();
        let mut next = HashMap::with_capacity(t.meters.len());
        for id in t.meters.keys().chain(self.shown.keys()) {
            let target = view::meter_fill(t.peak(*id));
            let was = self.shown.get(id).copied().unwrap_or(0.0);
            let v = view::meter_ballistics(was, target, dt);
            if v > 0.0 {
                next.insert(*id, v);
            }
        }
        self.shown = next;
    }

    /// A node's meter as drawn (0..1 of the bar)
    pub fn meter(&self, id: NodeId) -> f32 {
        self.shown.get(&id).copied().unwrap_or(0.0)
    }
}
