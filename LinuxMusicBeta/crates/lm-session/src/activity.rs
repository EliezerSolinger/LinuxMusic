//! Long operations and what the interface draws for them (I-2, T2, T10).
//!
//! Each kind of operation keeps its typed handles in an [`Ops`] list (the session applies each kind's result in its
//! own way, in the pump: T18); the [`Activity`] reads every list for the activity indicator and the `loading` of a
//! target, with the session's injectable clock so "never an intermediate state" is deterministic in the tests.

use std::time::Duration;

use lm_base::{Clock, Error, HostTime, Label, OperationHandle, OperationId, Phase, Result};
use lm_port::{Loading, OperationInfo, Target};

/// The operations that ran: their duration goes to the session's profile (I-2.3)
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub label: Label,
    pub target: Target,
    pub took: Duration,
    pub ok: bool,
}

/// One running operation of a kind, with the data its result needs
#[derive(Debug)]
pub struct Op<T, D> {
    pub handle: OperationHandle<T>,
    pub label: Label,
    pub target: Target,
    pub cancellable: bool,
    pub started: HostTime,
    /// What the session needs to apply the result
    pub data: D,
}

/// The running operations of one kind
#[derive(Debug)]
pub struct Ops<T, D> {
    pub list: Vec<Op<T, D>>,
}

impl<T, D> Default for Ops<T, D> {
    fn default() -> Self {
        Self { list: Vec::new() }
    }
}

/// What every kind's list answers, whatever its types
pub trait OpList {
    fn infos(&self, now: HostTime, out: &mut Vec<OperationInfo>);
    fn cancel(&self, id: OperationId) -> bool;
    fn cancel_all(&self);
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn info<T, D>(op: &Op<T, D>, now: HostTime) -> OperationInfo {
    let (progress, stage) = match op.handle.phase() {
        Phase::Running { progress, stage } => (progress, stage),
        _ => (None, None),
    };
    OperationInfo {
        id: op.handle.id(),
        label: op.label.clone(),
        target: op.target.clone(),
        progress,
        stage,
        cancellable: op.cancellable,
        since: Duration::from_nanos(now.since(op.started)),
    }
}

impl<T, D> OpList for Ops<T, D> {
    fn infos(&self, now: HostTime, out: &mut Vec<OperationInfo>) {
        out.extend(self.list.iter().map(|op| info(op, now)));
    }

    fn cancel(&self, id: OperationId) -> bool {
        match self.list.iter().find(|op| op.handle.id() == id) {
            Some(op) if op.cancellable => {
                op.handle.cancel();
                true
            }
            _ => false,
        }
    }

    fn cancel_all(&self) {
        for op in &self.list {
            op.handle.cancel();
        }
    }

    fn len(&self) -> usize {
        self.list.len()
    }
}

impl<T, D> Ops<T, D> {
    pub fn push(&mut self, op: Op<T, D>) {
        self.list.push(op);
    }

    /// The finished ones, taken out with their results
    pub fn finished(&mut self, now: HostTime, done: &mut Vec<Finished>) -> Vec<(Op<T, D>, Result<T>)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.list.len() {
            if self.list[i].handle.is_finished() {
                let mut op = self.list.swap_remove(i);
                let result = op.handle.poll().unwrap_or_else(|| Err(Error::new("session.operation.lost")));
                done.push(Finished {
                    label: op.label.clone(),
                    target: op.target.clone(),
                    took: Duration::from_nanos(now.since(op.started)),
                    ok: result.is_ok(),
                });
                out.push((op, result));
            } else {
                i += 1;
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// The failures shown on targets until the operation runs again (with "try again", I-14), and the profile
#[derive(Debug, Default)]
pub struct Activity {
    pub failures: Vec<(Target, Error)>,
    pub profile: Vec<Finished>,
}

impl Activity {
    pub fn fail(&mut self, target: Target, error: Error) {
        self.failures.retain(|(t, _)| *t != target);
        self.failures.push((target, error));
    }

    pub fn clear(&mut self, target: &Target) {
        self.failures.retain(|(t, _)| t != target);
    }

    pub fn failure(&self, target: &Target) -> Option<&Error> {
        self.failures.iter().find(|(t, _)| t == target).map(|(_, e)| e)
    }

    /// Keeps the last 1000 finished operations
    pub fn record(&mut self, done: Vec<Finished>) {
        self.profile.extend(done);
        if self.profile.len() > 1000 {
            let extra = self.profile.len() - 1000;
            self.profile.drain(..extra);
        }
    }
}

/// What the interface draws for the operations `infos` of one target (I-2.2, T10): nothing in the first 200 ms;
/// after, the oldest one's time, the smallest progress, cancellable if any is
pub fn loading_of(infos: &[&OperationInfo]) -> Loading {
    let Some(oldest) = infos.iter().map(|i| i.since).max() else { return Loading::Ready };
    if oldest < lm_port::LOADING_THRESHOLD {
        return Loading::Pending;
    }
    let progress = infos.iter().map(|i| i.progress).try_fold(f32::MAX, |m, p| p.map(|p| m.min(p)));
    let stage = infos.iter().max_by_key(|i| i.since).and_then(|i| i.stage.clone());
    Loading::Visible {
        progress: progress.filter(|p| *p <= 1.0),
        stage,
        cancellable: infos.iter().any(|i| i.cancellable),
        since: oldest,
    }
}

/// The time now by the session's clock
pub fn now(clock: &dyn Clock) -> HostTime {
    clock.now()
}
