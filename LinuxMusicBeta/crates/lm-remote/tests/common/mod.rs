//! A core for the protocol's tests: the interface's fake (a real Document), with what the fake does not do —
//! hour-long waveforms (I-20.8's 2 MB measure), the project actions as operations, imports — done here, so the
//! server is exercised against everything a session offers
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use lm_base::{Error, Label, OperationId, Result};
use lm_doc::{Diff, Feed, GestureId, Topic};
use lm_port::{
    ActionId, Applied, BrowseEntry, BrowsePage, CorePort, DeviceInfo, DisplayView, EditOp, EntryId, Finished, Loading,
    Notice, OperationInfo, PathStep, PeakBlock, Pending, Preview, RequestId, Resource, Target, TelemetryView, Value,
};
use lm_port_fake::{Config, FakePort};
use lm_state::NodeId;

/// One hour at 48 kHz
pub const HOUR: u64 = 3600 * 48_000;

/// What the test core did, for the tests to look at
#[derive(Debug, Default)]
pub struct Log {
    pub actions: Vec<(String, Value)>,
    pub imports: Vec<PathBuf>,
    pub peaks_served: u64,
}

pub struct TestCore {
    pub fake: FakePort,
    pub log: Log,
    /// Operations "running": (request, done at, result)
    ops: Vec<(RequestId, Instant, Finished)>,
    next: RequestId,
    /// How long a project action or an import takes (the 200 ms rule: they are operations)
    pub op_time: Duration,
}

impl TestCore {
    pub fn new(tracks: usize) -> TestCore {
        let fake = FakePort::new(Config { tracks, slow_delay: None, slow_track: 0 }).expect("fake");
        TestCore { fake, log: Log::default(), ops: Vec::new(), next: 1 << 40, op_time: Duration::from_millis(30) }
    }

    fn later<T>(&mut self, f: Finished) -> Pending<T> {
        let id = self.next;
        self.next += 1;
        self.ops.push((id, Instant::now() + self.op_time, f));
        Pending::Waiting(id)
    }
}

/// An hour-long waveform, made up on demand: pair `i` of `level` of resource `key`
fn pair(key: &str, level: u8, i: u64) -> (i16, i16) {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in key.bytes().chain(level.to_le_bytes()).chain(i.to_le_bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
    }
    let a = (h & 0x3fff) as i16;
    (-a, a)
}

impl CorePort for TestCore {
    fn pump(&mut self) -> bool {
        let now = Instant::now();
        let mut changed = self.fake.pump();
        changed |= self.ops.iter().any(|(_, at, _)| *at <= now);
        changed
    }
    fn finished(&mut self) -> Vec<(RequestId, Finished)> {
        let now = Instant::now();
        let mut out = self.fake.finished();
        let (done, wait): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.ops).into_iter().partition(|(_, at, _)| *at <= now);
        self.ops = wait;
        out.extend(done.into_iter().map(|(id, _, f)| (id, f)));
        out
    }
    fn root(&self) -> &Value {
        self.fake.root()
    }
    fn find(&self, id: NodeId) -> Option<Value> {
        self.fake.find(id)
    }
    fn feed(&mut self, topic: Topic) -> Feed {
        self.fake.feed(topic)
    }
    fn drain(&self, feed: &mut Feed) -> Option<Diff> {
        self.fake.drain(feed)
    }
    fn modified(&self) -> bool {
        self.fake.modified()
    }
    fn can_undo(&self) -> bool {
        self.fake.can_undo()
    }
    fn can_redo(&self) -> bool {
        self.fake.can_redo()
    }
    fn apply(&mut self, label: Label, ops: Vec<EditOp>) -> Pending<Applied> {
        self.fake.apply(label, ops)
    }
    fn run_action(&mut self, action: ActionId, params: Value) -> Pending<()> {
        if action.as_str().starts_with("project.") {
            self.log.actions.push((action.as_str().to_owned(), params));
            return self.later(Finished::Done(Ok(())));
        }
        self.fake.run_action(action, params)
    }
    fn undo(&mut self) -> Pending<()> {
        self.fake.undo()
    }
    fn redo(&mut self) -> Pending<()> {
        self.fake.redo()
    }
    fn begin_gesture(&mut self, label: Label) -> GestureId {
        self.fake.begin_gesture(label)
    }
    fn gesture_apply(&mut self, g: &GestureId, ops: Vec<EditOp>) {
        self.fake.gesture_apply(g, ops)
    }
    fn gesture_commit(&mut self, g: GestureId) {
        self.fake.gesture_commit(g)
    }
    fn gesture_cancel(&mut self, g: GestureId) {
        self.fake.gesture_cancel(g)
    }
    fn param_write(&mut self, node: NodeId, param: &str, value: f32) {
        self.fake.param_write(node, param, value)
    }
    fn loading(&self, target: &Target) -> Loading {
        self.fake.loading(target)
    }
    fn activity(&self) -> Vec<OperationInfo> {
        self.fake.activity()
    }
    fn cancel(&mut self, op: OperationId) {
        self.fake.cancel(op)
    }
    fn notices(&mut self) -> Vec<Notice> {
        self.fake.notices()
    }
    fn dismiss_notice(&mut self, id: u64) {
        self.fake.dismiss_notice(id)
    }
    fn telemetry(&self) -> &TelemetryView {
        self.fake.telemetry()
    }
    fn peaks(&mut self, resource: &Resource, level: u8, from: u64, to: u64) -> Pending<PeakBlock> {
        let spp = 64u64 * 4u64.pow(u32::from(level.min(12)));
        let total = HOUR.div_ceil(spp);
        let (a, z) = (from.min(total), to.clamp(from.min(total), total));
        let pairs: Vec<(i16, i16)> = (a..z).map(|i| pair(resource.key(), level, i)).collect();
        self.log.peaks_served += pairs.len() as u64;
        Pending::Ready(Ok(PeakBlock { level, from: a, samples_per_pair: spp, pairs: pairs.into(), length: HOUR }))
    }
    fn display(&self, node: NodeId) -> Option<DisplayView> {
        self.fake.display(node)
    }
    fn preferences(&self) -> &Value {
        self.fake.preferences()
    }
    fn set_preference(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()> {
        self.fake.set_preference(path, value)
    }
    fn status(&self) -> &Value {
        self.fake.status()
    }
    fn devices(&mut self) -> Pending<Vec<DeviceInfo>> {
        self.fake.devices()
    }
    fn browse(&mut self, page: BrowsePage, parent: Option<EntryId>, show_hidden: bool) -> Pending<Vec<BrowseEntry>> {
        self.fake.browse(page, parent, show_hidden)
    }
    fn search(&mut self, query: &str) -> Pending<Vec<BrowseEntry>> {
        self.fake.search(query)
    }
    fn prepare_preview(&mut self, entry: EntryId) -> Pending<Preview> {
        self.fake.prepare_preview(entry)
    }
    fn preview_play(&mut self, entry: EntryId, from: i64) {
        self.fake.preview_play(entry, from)
    }
    fn preview_stop(&mut self) {
        self.fake.preview_stop()
    }
    fn set_favorite(&mut self, entry: EntryId, on: bool) -> Pending<()> {
        self.fake.set_favorite(entry, on)
    }
    fn import(&mut self, path: PathBuf) -> Pending<Resource> {
        self.log.imports.push(path.clone());
        let r = std::fs::read(&path)
            .map_err(|e| Error::new("test.import").arg("error", e.to_string()))
            .and_then(|bytes| Resource::of_content("audio", &bytes));
        self.later(Finished::Imported(r))
    }
}

pub fn result<T>(r: Result<T>) -> T {
    r.expect("ok")
}
