//! The `CorePort` (interfaces.md, I-15, user's decision 19/11): everything the interface may do with the core, with
//! DATA as arguments and results (no closures, no references into the core), so the same interface works with the
//! core in the same process (`LocalPort`) and over the network (`RemotePort`, which keeps a mirror of the tree).
//!
//! The fake: `Session` is the `LocalPort`. Its reads answer from the document; its changes are `EditOp`s that it
//! turns into one transaction. Results that may take time come as `Pending`: here always `Ready`, but the interface
//! must handle `Waiting` (in the client they arrive in a later pump).
//!
//! Differences from the real form, on purpose and small: the path of an edit is a typed `Field` (the real core uses
//! a `Vec<PathStep>` of permanent keys); the root is the fake's `Doc` struct (the real one is a `Value` tree).

use std::sync::Arc;

use crate::notices::{Activity, Notice};
use crate::{
    Diff, Doc, Error, Feed, Field, GestureId, Loading, MAX_METERS, NodeId, Peaks, Position, ResourceId, Result,
    Session, Target, Topic, Value,
};

pub type Label = &'static str;
pub type RequestId = u64;
pub type ActionId = &'static str;

/// A result that may not be there yet: in-process it is ready at once; over the network it arrives in a later pump
#[derive(Debug)]
#[must_use]
pub enum Pending<T> {
    Ready(Result<T>),
    Waiting(RequestId),
}

impl<T> Pending<T> {
    pub fn is_waiting(&self) -> bool {
        matches!(self, Pending::Waiting(_))
    }
    /// The value if it is ready and good
    pub fn ok(self) -> Option<T> {
        match self {
            Pending::Ready(Ok(v)) => Some(v),
            _ => None,
        }
    }
}

/// A node named by its id, or by a number given to a node created earlier in the same list of edits
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ref {
    Id(NodeId),
    Temp(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteRef {
    N(u64),
    Temp(u32),
}

/// One edit as data; a list of them is one transaction (one undo step)
#[derive(Clone, Debug)]
pub enum EditOp {
    Set { id: Ref, field: Field, value: Value },
    Insert { temp: u32, parent: Ref, list: &'static str, index: usize, node: Value },
    Remove { id: Ref },
    Move { id: Ref, parent: Ref, list: &'static str, index: usize },
    AddNote { temp: u32, item: Ref, note: Value },
    SetNote { item: Ref, n: NoteRef, field: Field, value: Value },
    RemoveNote { item: Ref, n: NoteRef },
}

impl EditOp {
    /// The common case: one field of a node
    pub fn set(id: NodeId, field: Field, value: Value) -> EditOp {
        EditOp::Set { id: Ref::Id(id), field, value }
    }
}

#[derive(Debug, Default)]
pub struct Applied {
    pub diff: Diff,
    pub created: Vec<(u32, NodeId)>,
    pub notes: Vec<(u32, u64)>,
}

/// A take being recorded, for the waveform that grows (U3)
#[derive(Clone, Debug)]
pub struct RecTake {
    pub track: NodeId,
    pub start: i64,
    pub length: i64,
    pub resource: ResourceId,
}

/// The continuous values: meters (by meter index: tracks, then channels), the position with its host time (the
/// interface interpolates the cursor), the takes being recorded, the count-in, the xrun count (P-14)
#[derive(Clone, Debug)]
pub struct TelemetryView {
    pub meters: [f32; MAX_METERS],
    pub position: Position,
    pub recording: Vec<RecTake>,
    pub counting_in: bool,
    pub xruns: u32,
    /// where the xruns happened, in samples (red marks on the timeline, F-CLP-063)
    pub xrun_marks: Vec<i64>,
    /// the inputs' levels (linear peak), shown in the armed tracks' headers even without monitoring (F-REC-009)
    pub inputs: [f32; 8],
}

impl TelemetryView {
    /// The meter index of the channel at `i` in the root's channels
    pub fn channel_meter(&self, i: usize) -> usize {
        (crate::CHANNEL_METERS + i).min(MAX_METERS - 1)
    }
}

/// A piece of a resource's peak pyramid (I-20.4): `pairs` are the (min, max) of `level`, from pair `from` on.
/// Immutable for a resource's hash: the interface keeps what it got.
#[derive(Clone, Debug)]
pub struct PeakBlock {
    pub level: u8,
    pub from: u64,
    pub samples_per_pair: u64,
    pub pairs: Arc<[(i16, i16)]>,
    /// the resource's length in samples
    pub length: u64,
}

/// What a node is, as a copy (never a pointer into the core)
#[derive(Clone, Debug)]
pub enum NodeView {
    Track(crate::Track),
    Channel(crate::Channel),
    Item(crate::Item),
}

/// Everything the GUI may do with the core (I-15)
pub trait CorePort {
    /// Each frame and on the 10 ms timer (H2). True when something shown may have changed
    fn pump(&mut self) -> bool;
    // --- reading: the port's mirror of the tree
    fn root(&self) -> &Doc;
    fn find(&self, id: NodeId) -> Option<NodeView>;
    fn feed(&mut self, topic: Topic) -> Feed;
    /// Drains a feed (I-3.6)
    fn drain(&self, feed: &mut Feed) -> Option<Diff>;
    // --- changing: edits as data, actions by id, gestures by id
    fn apply(&mut self, label: Label, ops: Vec<EditOp>) -> Pending<Applied>;
    fn run_action(&mut self, action: &str, params: Value) -> Pending<()>;
    fn undo(&mut self) -> Pending<()>;
    fn redo(&mut self) -> Pending<()>;
    fn begin_gesture(&mut self, label: Label) -> GestureId;
    fn gesture_apply(&mut self, g: &GestureId, ops: Vec<EditOp>);
    fn gesture_commit(&mut self, g: GestureId);
    fn gesture_cancel(&mut self, g: GestureId);
    /// Live, while dragging, without an edit (the value reaches the audio's slot at once)
    fn param_write(&mut self, node: NodeId, param: &str, value: f32);
    // --- long operations, notices, telemetry, heavy data
    fn loading(&self, target: &Target) -> Loading;
    fn activity(&self) -> Vec<Activity>;
    fn cancel(&mut self, op: u64);
    fn notices(&mut self) -> Vec<Notice>;
    fn telemetry(&self) -> TelemetryView;
    fn peaks(&mut self, resource: ResourceId, level: u8, from: u64, to: u64) -> Pending<PeakBlock>;
    /// The read side of the services the screens show (devices, preferences, recent projects, the export's
    /// report): in the real core part of the mirrored root and of the management surface (I-20)
    fn services(&self) -> &crate::project::Services;
    /// The pumps so far (the tests count them)
    fn pumps(&self) -> u64;
}

/// Edits that another module of the fake handles (notes, plugins...): ONE LINE PER MODULE. A handler returns None
/// when the edit is not its own.
type OpHandler = fn(&mut crate::Edit, &EditOp, &[(u32, NodeId)]) -> Option<Result<Option<(u32, NodeId)>>>;
const OP_HANDLERS: &[OpHandler] = &[crate::mixer::apply_op, crate::arrange::apply_op, crate::notes::apply_op, crate::notes::apply_point_op, crate::plugins::apply_op];

fn resolve(r: Ref, created: &[(u32, NodeId)]) -> Result<NodeId> {
    match r {
        Ref::Id(id) => Ok(id),
        Ref::Temp(t) => created.iter().find(|(k, _)| *k == t).map(|(_, id)| *id).ok_or(Error::WrongType),
    }
}

fn apply_ops(e: &mut crate::Edit, ops: &[EditOp]) -> Result<Vec<(u32, NodeId)>> {
    let mut created = Vec::new();
    for op in ops {
        if let EditOp::Set { id, field, value } = op {
            let id = resolve(*id, &created)?;
            e.set(id, *field, value.clone())?;
            continue;
        }
        let mut done = false;
        for h in OP_HANDLERS {
            if let Some(r) = h(e, op, &created) {
                if let Some(c) = r? {
                    created.push(c);
                }
                done = true;
                break;
            }
        }
        if !done {
            return Err(Error::WrongType);
        }
    }
    Ok(created)
}

impl CorePort for Session {
    fn pump(&mut self) -> bool {
        Session::pump(self)
    }

    fn root(&self) -> &Doc {
        self.doc()
    }

    fn find(&self, id: NodeId) -> Option<NodeView> {
        let d = self.doc();
        d.tracks
            .iter()
            .find(|t| t.id == id)
            .map(|t| NodeView::Track(t.clone()))
            .or_else(|| d.channels.iter().find(|c| c.id == id).map(|c| NodeView::Channel(c.clone())))
            .or_else(|| d.tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.id == id).map(|i| NodeView::Item(i.clone())))
    }

    fn feed(&mut self, topic: Topic) -> Feed {
        Session::feed(self, topic)
    }

    fn drain(&self, feed: &mut Feed) -> Option<Diff> {
        feed.drain(self)
    }

    fn apply(&mut self, label: Label, ops: Vec<EditOp>) -> Pending<Applied> {
        let mut created = Vec::new();
        let r = self.transact(label, |e| {
            created = apply_ops(e, &ops)?;
            Ok(())
        });
        Pending::Ready(r.map(|diff| Applied { diff, created, notes: Vec::new() }))
    }

    fn run_action(&mut self, action: &str, params: Value) -> Pending<()> {
        let num = |v: &Value| match v {
            Value::Real(x) => Some(*x),
            _ => None,
        };
        let r = match action {
            "transport.play" => Ok(self.play()),
            "transport.stop" => Ok(self.stop()),
            "transport.record" => Ok(self.record(match params {
                Value::Id(t) => Some(t),
                _ => None,
            })),
            "transport.toggle" => Ok(if self.position().playing { self.stop() } else { self.play() }),
            "transport.locate" => num(&params).map(|s| self.locate(s as i64)).ok_or(Error::WrongType),
            "take.reload" => num(&params).map(|t| self.reload_take(t as usize)).ok_or(Error::WrongType),
            "mixer.add_channel" => self.add_channel().map(|_| ()),
            // the arrangement (arrange.rs): at the cursor where it applies
            "item.split" => match params {
                Value::Id(item) => self.split_item(item, self.position().samples),
                _ => Err(Error::WrongType),
            },
            "item.duplicate" => match params {
                Value::Id(item) => self.duplicate_item(item),
                _ => Err(Error::WrongType),
            },
            "item.empty" => num(&params).ok_or(Error::WrongType).and_then(|t| self.empty_item(t as usize, self.position().samples)),
            "marker.add" => self.add_marker(self.position().samples),
            "marker.goto" => num(&params)
                .and_then(|k| self.doc().song.markers.get(k as usize).map(|m| m.at))
                .map(|at| self.locate(at))
                .ok_or(Error::WrongType),
            "loop.from_region" => match params {
                Value::Id(r) => self.loop_from_region(r),
                _ => Err(Error::WrongType),
            },
            "region.from_loop" => self.region_from_loop(),
            // the recorded items stretched to a new tempo (F-REC-090): a long operation, cancellable
            "items.rerender" => {
                let n = self.doc().tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.resource().is_some()).count();
                let _ = self.start("Re-rendering the recorded items", Target::Project, true, move |cancel, progress| {
                    for k in 0..n {
                        if cancel.is_cancelled() {
                            return Err("cancelled".to_string());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(40));
                        progress.set((k + 1) as f32 / n as f32);
                    }
                    Ok(())
                });
                Ok(())
            }
            "mixer.create_channels_for_outputs" => {
                num(&params).ok_or(Error::WrongType).and_then(|t| self.create_channels_for_outputs(t as usize))
            }
            other => match crate::actions::find(other).map(|a| a.run) {
                Some(crate::actions::Run::Core(f)) => Ok(f(self)),
                Some(crate::actions::Run::CoreWith(f)) => Ok(f(self, &crate::actions::param_text(&params))),
                _ => Err(Error::WrongType), // the screen and page actions are the interface's own
            },
        };
        Pending::Ready(r)
    }

    fn undo(&mut self) -> Pending<()> {
        Pending::Ready(Session::undo(self).map(|_| ()))
    }

    fn redo(&mut self) -> Pending<()> {
        Pending::Ready(Err(Error::WrongType)) // the fake keeps no redo stack yet
    }

    fn begin_gesture(&mut self, label: Label) -> GestureId {
        Session::begin_gesture(self, label)
    }

    fn gesture_apply(&mut self, g: &GestureId, ops: Vec<EditOp>) {
        let _ = Session::gesture_update(self, g, |e| apply_ops(e, &ops).map(|_| ()));
    }

    fn gesture_commit(&mut self, g: GestureId) {
        let _ = Session::gesture_commit(self, g);
    }

    fn gesture_cancel(&mut self, g: GestureId) {
        Session::gesture_cancel(self, g)
    }

    fn param_write(&mut self, node: NodeId, param: &str, value: f32) {
        self.write_slot(node, param, value);
    }

    fn loading(&self, target: &Target) -> Loading {
        Session::loading(self, target)
    }

    fn activity(&self) -> Vec<Activity> {
        Session::activity(self)
    }

    fn cancel(&mut self, op: u64) {
        Session::cancel(self, op)
    }

    fn notices(&mut self) -> Vec<Notice> {
        self.services.notices.iter().cloned().collect()
    }

    fn telemetry(&self) -> TelemetryView {
        TelemetryView {
            meters: *self.meters(),
            position: self.position(),
            recording: self
                .recording_takes()
                .into_iter()
                .map(|(t, start, length, _)| RecTake {
                    track: self.doc().tracks[t].id,
                    start,
                    length,
                    resource: self.doc().tracks[t].main_resource().unwrap_or(ResourceId(0)),
                })
                .collect(),
            counting_in: self.counting_in(),
            xruns: self.xruns(),
            xrun_marks: self.xrun_marks.clone(),
            inputs: {
                // the fake inputs: a voice-like level on each, moving with the clock
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f32());
                std::array::from_fn(|k| (0.35 + 0.3 * (t * (1.3 + k as f32 * 0.4)).sin().abs()) * if k % 3 == 2 { 0.2 } else { 1.0 })
            },
        }
    }

    fn peaks(&mut self, resource: ResourceId, level: u8, from: u64, to: u64) -> Pending<PeakBlock> {
        let Some(p) = Session::peaks(self, resource) else {
            return Pending::Waiting(resource.0 as u64); // still being generated: asked again next frame
        };
        Pending::Ready(block(p, level, from, to))
    }

    fn services(&self) -> &crate::project::Services {
        &self.services
    }

    fn pumps(&self) -> u64 {
        Session::pumps(self)
    }
}

fn block(p: &Peaks, level: u8, from: u64, to: u64) -> Result<PeakBlock> {
    let l = (level as usize).min(p.levels.len() - 1);
    let pairs = &p.levels[l];
    let a = (from as usize).min(pairs.len());
    let z = (to as usize).clamp(a, pairs.len());
    Ok(PeakBlock {
        level: l as u8,
        from: a as u64,
        samples_per_pair: Peaks::samples_per_pair(l),
        pairs: pairs[a..z].into(),
        length: p.length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use std::time::Duration;

    fn small() -> Session {
        Session::create(Config { tracks: 6, channels: 6, slow_delay: Duration::from_millis(1), slow_track: 2, item_seconds: 5.0 })
    }

    #[test]
    fn apply_is_one_undo_step() {
        let mut s = small();
        let (a, b) = (s.root().tracks[1].id, s.root().tracks[2].id);
        let r = s.apply("Two", vec![EditOp::set(a, Field::Volume, Value::Real(-3.0)), EditOp::set(b, Field::Mute, Value::Bool(true))]);
        assert!(r.ok().is_some());
        assert_eq!(s.root().tracks[1].volume_db, -3.0);
        let _ = CorePort::undo(&mut s);
        assert_eq!(s.root().tracks[1].volume_db, 0.0);
        assert!(!s.root().tracks[2].mute);
    }

    #[test]
    fn a_bad_list_changes_nothing() {
        let mut s = small();
        let a = s.root().tracks[1].id;
        let r = s.apply("Bad", vec![EditOp::set(a, Field::Volume, Value::Real(-3.0)), EditOp::set(a, Field::Volume, Value::Bool(true))]);
        assert!(matches!(r, Pending::Ready(Err(_))));
        assert_eq!(s.root().tracks[1].volume_db, 0.0);
    }

    #[test]
    fn peaks_come_in_blocks() {
        let mut s = small();
        std::thread::sleep(Duration::from_millis(80));
        CorePort::pump(&mut s);
        let b = CorePort::peaks(&mut s, ResourceId(1), 2, 0, 100).ok().expect("ready");
        assert_eq!(b.pairs.len(), 100);
        assert_eq!(b.samples_per_pair, 1024);
    }
}
