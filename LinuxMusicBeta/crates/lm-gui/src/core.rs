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
use lm_gui_plugins::{Gesture, Gestures, Outcome, PortHost};
use lm_port::{CorePort, EditOp, Error, Label, NodeId, Pending, RequestId};

use crate::bench::Bench;
use crate::view;

/// How many errors wait for the workspace at once (a failing loop does not pile them up)
const ERRORS_MAX: usize = 16;

pub struct Core {
    port: Box<dyn CorePort>,
    pub bench: Bench,
    /// The meters as drawn: the latest telemetry with the fall smoothed per frame (H16), by node
    pub shown: HashMap<NodeId, f32>,
    /// How many times the pump ran (the tests check H2 with it)
    pub pumps: u64,
    /// The interface's actions mapped MIDI controls reached (I-21.2), for the workspace to run
    pub midi_ui: Vec<lm_port::MidiUiAction>,
    /// The MIDI activity's counter and the learn, last seen: a change redraws
    midi_seen: (u64, Option<lm_port::LearnId>),
    /// The gestures open in the core: every drag opens its own here, and none stays open after a release (see
    /// `lm_gui_plugins::host`)
    gestures: Gestures,
    /// The requests somebody waits for, with the pump they were made at: an answer is kept only for these, and each
    /// leaves when its answer is taken (or when it is forgotten, or after `REQUEST_LIFE`)
    waiting: HashMap<RequestId, u64>,
    /// The answers of imports that waited (the RemotePort's), until a plugin panel takes them by request
    imported: HashMap<RequestId, lm_port::Result<lm_port::Resource>>,
    /// The answers of the browser's listings, searches and previews that waited, until the browser takes them
    entries: HashMap<RequestId, lm_port::Result<Vec<lm_port::BrowseEntry>>>,
    previews: HashMap<RequestId, lm_port::Result<lm_port::Preview>>,
    /// The errors of the calls to the port, for the workspace to show as notices (`take_errors`)
    errors: Vec<Error>,
    /// The peaks that were asked and have arrived (the tiles ask again), or failed
    peaks_arrived: Vec<RequestId>,
    peaks_failed: Vec<(RequestId, Error)>,
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
        let mut core = Core {
            port,
            bench,
            shown: HashMap::new(),
            pumps: 0,
            midi_ui: Vec::new(),
            midi_seen: (0, None),
            gestures: Gestures::default(),
            waiting: HashMap::new(),
            imported: HashMap::new(),
            entries: HashMap::new(),
            previews: HashMap::new(),
            errors: Vec::new(),
            peaks_arrived: Vec::new(),
            peaks_failed: Vec::new(),
            last_meters: Instant::now(),
            _pump: pump,
        };
        if core.bench.wants_play() {
            core.report(|port| port.run_action("transport.play".into(), lm_port::Value::NULL));
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
        self.port.drain_midi_ui(&mut self.midi_ui);
        let mut redraw = false;
        for (id, answer) in self.port.finished() {
            let wanted = self.waiting.contains_key(&id);
            match answer {
                lm_port::Finished::Imported(r) if wanted => {
                    self.imported.insert(id, r);
                }
                lm_port::Finished::Entries(r) if wanted => {
                    self.entries.insert(id, r);
                }
                lm_port::Finished::Preview(r) if wanted => {
                    self.previews.insert(id, r);
                }
                // nobody waits for it any more (its screen closed): an error is still the user's to know
                lm_port::Finished::Imported(Err(e)) => self.fail(e),
                lm_port::Finished::Imported(Ok(_)) | lm_port::Finished::Entries(_) | lm_port::Finished::Preview(_) => {}
                lm_port::Finished::Applied(Err(e))
                | lm_port::Finished::Done(Err(e))
                | lm_port::Finished::Learned(Err(e))
                | lm_port::Finished::Devices(Err(e)) => self.fail(e),
                lm_port::Finished::Applied(Ok(_))
                | lm_port::Finished::Done(Ok(()))
                | lm_port::Finished::Learned(Ok(_))
                | lm_port::Finished::Devices(Ok(_)) => {}
                // a block of peaks arrived: the tiles that asked for it ask again (the port keeps it now)
                lm_port::Finished::Peaks(Ok(_)) => {
                    self.peaks_arrived.push(id);
                    redraw = true;
                }
                lm_port::Finished::Peaks(Err(e)) => {
                    self.peaks_failed.push((id, e));
                    redraw = true;
                }
            }
        }
        self.expire();
        let midi = (self.port.midi_activity(u64::MAX).0, self.port.learning());
        let midi_changed = midi != self.midi_seen || !self.midi_ui.is_empty();
        self.midi_seen = midi;
        if changed || redraw || midi_changed || self.shown.values().any(|m| *m > 0.0) {
            cx.notify();
        }
    }

    /// The requests nobody takes the answer of stop being kept after this many pumps (10 ms each: two minutes)
    const REQUEST_LIFE: u64 = 12_000;

    fn expire(&mut self) {
        if self.pumps % 100 != 0 {
            return;
        }
        let now = self.pumps;
        self.waiting.retain(|_, at| now.saturating_sub(*at) < Self::REQUEST_LIFE);
        let waiting = &self.waiting;
        self.imported.retain(|id, _| waiting.contains_key(id));
        self.entries.retain(|id, _| waiting.contains_key(id));
        self.previews.retain(|id, _| waiting.contains_key(id));
    }

    /// How many requests are waited for (the tests check that none stays)
    #[cfg(test)]
    pub fn waiting_requests(&self) -> usize {
        self.waiting.len() + self.imported.len() + self.entries.len() + self.previews.len()
    }

    /// Somebody waits for the answer of this request (`Finished`): it is kept until taken or forgotten
    pub fn expect(&mut self, id: RequestId) {
        self.waiting.insert(id, self.pumps);
    }

    /// The call as asked of the port, its `Waiting` request kept for its answer: for the calls whose answer a screen
    /// takes (`take_entries`, `take_preview`, `take_imported`)
    fn expecting<T>(&mut self, pending: Pending<T>) -> Pending<T> {
        if let Pending::Waiting(id) = &pending {
            self.expect(*id);
        }
        self.settle(pending)
    }

    /// An import through the port; its `Waiting` answer is kept for `take_imported`
    pub fn import(&mut self, path: std::path::PathBuf) -> Pending<lm_port::Resource> {
        PortHost::import(self, path)
    }

    pub fn browse(
        &mut self,
        page: lm_port::BrowsePage,
        parent: Option<lm_port::EntryId>,
        hidden: bool,
    ) -> Pending<Vec<lm_port::BrowseEntry>> {
        let p = self.port.browse(page, parent, hidden);
        self.expecting(p)
    }

    pub fn search(&mut self, query: &str) -> Pending<Vec<lm_port::BrowseEntry>> {
        let p = self.port.search(query);
        self.expecting(p)
    }

    pub fn prepare_preview(&mut self, entry: lm_port::EntryId) -> Pending<lm_port::Preview> {
        let p = self.port.prepare_preview(entry);
        self.expecting(p)
    }

    /// The answer of a browser listing or search that waited (once)
    pub fn take_entries(&mut self, id: RequestId) -> Option<lm_port::Result<Vec<lm_port::BrowseEntry>>> {
        let a = self.entries.remove(&id);
        if a.is_some() {
            self.waiting.remove(&id);
        }
        a
    }

    /// The answer of a preview that waited (once)
    pub fn take_preview(&mut self, id: RequestId) -> Option<lm_port::Result<lm_port::Preview>> {
        let a = self.previews.remove(&id);
        if a.is_some() {
            self.waiting.remove(&id);
        }
        a
    }

    /// Nobody waits for the request any more (the screen closed, or asked something else): its answer is dropped
    pub fn forget_request(&mut self, id: RequestId) {
        self.waiting.remove(&id);
        self.imported.remove(&id);
        self.entries.remove(&id);
        self.previews.remove(&id);
    }

    /// The peaks that arrived since the last call, and the ones that failed
    pub fn take_peak_answers(&mut self) -> (Vec<RequestId>, Vec<(RequestId, Error)>) {
        (std::mem::take(&mut self.peaks_arrived), std::mem::take(&mut self.peaks_failed))
    }

    // --- the one place the results of the calls to the port come to

    /// An error to show the user (a cancelled operation is not one)
    pub fn fail(&mut self, e: Error) {
        if !e.is_cancelled() && self.errors.len() < ERRORS_MAX {
            self.errors.push(e);
        }
    }

    /// The errors since the last call, for the workspace's notices
    pub fn take_errors(&mut self) -> Vec<Error> {
        std::mem::take(&mut self.errors)
    }

    /// A call to the port, its result settled here: an error is shown, a request that waits is followed (its error
    /// is shown when it comes). Gives the value when the answer was ready and good
    pub fn report<T>(&mut self, call: impl FnOnce(&mut dyn CorePort) -> Pending<T>) -> Option<T> {
        PortHost::report(self, call)
    }

    /// The result of a call already made, settled here (see `report`), and given back for the caller to read
    pub fn settle<T>(&mut self, pending: Pending<T>) -> Pending<T> {
        PortHost::settle(self, Outcome::of(&pending));
        pending
    }

    // --- gestures

    pub fn begin_gesture(&mut self, label: Label) -> Gesture {
        PortHost::gesture_begin(self, label)
    }

    /// `false`: the gesture was closed (the drag's release was lost): the caller drops its drag
    pub fn gesture_apply(&mut self, g: Gesture, ops: Vec<EditOp>) -> bool {
        PortHost::gesture_apply(self, g, ops)
    }

    pub fn gesture_commit(&mut self, g: Gesture) -> bool {
        PortHost::gesture_commit(self, g)
    }

    pub fn gesture_cancel(&mut self, g: Gesture) -> bool {
        PortHost::gesture_cancel(self, g)
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

/// The plugin panels (lm-gui-plugins, I-22) work on the core through this entity: the port, and the imports' answers
impl PortHost for Core {
    fn port(&self) -> &dyn CorePort {
        &*self.port
    }

    fn port_mut(&mut self) -> &mut dyn CorePort {
        &mut *self.port
    }

    fn gestures_and_port(&mut self) -> (&mut dyn CorePort, &mut Gestures) {
        (&mut *self.port, &mut self.gestures)
    }

    fn settle(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Fine => {}
            Outcome::Failed(e) => self.fail(e),
            // the answer's error is shown when it arrives (`on_pump`)
            Outcome::Waiting(_) => {}
        }
    }

    fn import(&mut self, path: std::path::PathBuf) -> Pending<lm_port::Resource> {
        let p = self.port.import(path);
        self.expecting(p)
    }

    fn take_imported(&mut self, id: RequestId) -> Option<lm_port::Result<lm_port::Resource>> {
        let a = self.imported.remove(&id);
        if a.is_some() {
            self.waiting.remove(&id);
        }
        a
    }

    fn forget_request(&mut self, id: RequestId) {
        Core::forget_request(self, id);
    }
}
