//! The core as a GPUI entity. The `!Send` `Session` lives here, owned by the UI (I-15); every view holds an
//! `Entity<Core>` and reads it in `render` (`self.core.read(cx)`) or changes it in a handler
//! (`self.core.update(cx, |core, cx| ...)`). Views never keep references into it: only ids, feeds and gesture ids.
//!
//! The pump is a task on GPUI's foreground executor, on a 10 ms timer of GPUI's clock that does not depend on
//! frames (H2). It notifies the entity only when something shown changed; the workspace observes the entity and
//! redraws the window.

use std::time::{Duration, Instant};

use fake_core::bench::Bench;
use fake_core::view;
use fake_core::{Config, MAX_METERS, Session};
use gpui::{Context, Task};

pub struct Core {
    pub session: Session,
    pub bench: Bench,
    /// The meters as drawn: the latest telemetry with the fall smoothed per frame (H16)
    pub shown: [f32; MAX_METERS],
    last_meters: Instant,
    _pump: Task<()>,
}

impl Core {
    pub fn new(cx: &mut Context<Self>) -> Core {
        let mut session = Session::create(Config::default());
        let bench = Bench::from_env("gpui");
        if bench.wants_play() {
            session.play();
        }
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
        Core { session, bench, shown: [0.0; MAX_METERS], last_meters: Instant::now(), _pump: pump }
    }

    fn on_pump(&mut self, cx: &mut Context<Self>) {
        let changed = self.session.pump();
        if self.bench.tick() {
            cx.quit();
            return;
        }
        if changed || self.shown.iter().any(|&m| m > 0.0) {
            cx.notify();
        }
    }

    /// Once per frame, from the workspace's render
    pub fn advance_meters(&mut self, now: Instant) {
        let dt = now.duration_since(self.last_meters).as_secs_f32().min(0.1);
        self.last_meters = now;
        let meters = *self.session.meters();
        for (s, m) in self.shown.iter_mut().zip(meters.iter()) {
            *s = view::meter_ballistics(*s, view::meter_fill(*m), dt);
        }
    }
}
