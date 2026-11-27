//! What the interface's windows share with the core through one door: gestures that are always closed, and one
//! place that receives the result of every call to the port.
//!
//! A gesture (I-3.7) is opened when a drag starts and committed when it ends. A release outside the element or the
//! window, or a window that loses the focus in the middle of a drag, used to leave it open for good. Every view now
//! opens its gestures here: the registry owns the port's handle and gives the view a `Gesture` token (a number, `Copy`).
//! The end of a drag is the view's own `commit` or `cancel`; whatever is still open when a mouse button is released
//! anywhere, or when the window is deactivated, is committed by [`Gestures::commit_all`], run by the guards below.
//! A token whose gesture is closed is refused (`apply` answers `false`), so a stale drag cannot write.
//!
//! A call to the port answers `Pending<T>`: an error in a `Ready` is shown at once, one in a `Waiting` when its
//! answer comes (`Finished`). The interface passes every call through [`PortHost::report`], which settles the result
//! with the host; the host shows the error as a short notice. Nothing is dropped with `let _ =`.

use std::collections::HashMap;

use gpui::{
    Context, DispatchPhase, Entity, IntoElement, MouseUpEvent, Subscription, WeakEntity, Window, canvas, prelude::*, px,
};
use lm_port::{CorePort, EditOp, Error, GestureId, Label, Pending, RequestId};

/// A gesture of a drag, by number: what the views keep between mouse events
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Gesture(u64);

impl Gesture {
    /// The number (the tests tell gestures apart with it)
    pub fn number(self) -> u64 {
        self.0
    }
}

/// The gestures open in the core and the handles of them
#[derive(Debug, Default)]
pub struct Gestures {
    open: HashMap<u64, GestureId>,
    next: u64,
}

impl Gestures {
    pub fn begin(&mut self, port: &mut dyn CorePort, label: Label) -> Gesture {
        self.next += 1;
        self.open.insert(self.next, port.begin_gesture(label));
        Gesture(self.next)
    }

    /// Whether the gesture is still open (a drag whose gesture was closed under it stops)
    pub fn is_open(&self, g: Gesture) -> bool {
        self.open.contains_key(&g.0)
    }

    /// How many are open
    pub fn count(&self) -> usize {
        self.open.len()
    }

    /// The edits of a drag step; `false` when the gesture is closed (nothing is written)
    pub fn apply(&mut self, port: &mut dyn CorePort, g: Gesture, ops: Vec<EditOp>) -> bool {
        match self.open.get(&g.0) {
            Some(id) => {
                port.gesture_apply(id, ops);
                true
            }
            None => false,
        }
    }

    /// The end of a drag: its step enters the history. `false` when it was closed already
    pub fn commit(&mut self, port: &mut dyn CorePort, g: Gesture) -> bool {
        match self.open.remove(&g.0) {
            Some(id) => {
                port.gesture_commit(id);
                true
            }
            None => false,
        }
    }

    /// The drag was undone (Escape): everything it changed goes back
    pub fn cancel(&mut self, port: &mut dyn CorePort, g: Gesture) -> bool {
        match self.open.remove(&g.0) {
            Some(id) => {
                port.gesture_cancel(id);
                true
            }
            None => false,
        }
    }

    /// Closes every open gesture by committing it (the user let go: what was done stays, as one step). Gives how
    /// many were closed
    pub fn commit_all(&mut self, port: &mut dyn CorePort) -> usize {
        let mut open: Vec<(u64, GestureId)> = self.open.drain().collect();
        open.sort_by_key(|(n, _)| *n);
        let n = open.len();
        for (_, id) in open {
            port.gesture_commit(id);
        }
        n
    }
}

/// What a call to the port came to, for [`PortHost::settle`]
#[derive(Debug)]
pub enum Outcome {
    Fine,
    Failed(Error),
    /// Asked: the answer comes as `Finished` for this request
    Waiting(RequestId),
}

impl Outcome {
    pub fn of<T>(p: &Pending<T>) -> Outcome {
        match p {
            Pending::Ready(Ok(_)) => Outcome::Fine,
            Pending::Ready(Err(e)) => Outcome::Failed(e.clone()),
            Pending::Waiting(id) => Outcome::Waiting(*id),
        }
    }
}

/// What a panel needs of its host: the port, read and changed (lm-gui's `Core` entity implements it)
pub trait PortHost: 'static {
    fn port(&self) -> &dyn CorePort;
    fn port_mut(&mut self) -> &mut dyn CorePort;
    /// The port and the gestures open on it, to work on both at once
    fn gestures_and_port(&mut self) -> (&mut dyn CorePort, &mut Gestures);
    /// Takes the result of a call (see the module's text): shows an error, follows a request that waits
    fn settle(&mut self, outcome: Outcome);
    /// An import through the port, its answer kept for [`PortHost::take_imported`] while somebody waits for it
    fn import(&mut self, path: std::path::PathBuf) -> Pending<lm_port::Resource>;
    /// The answer of an import that answered `Waiting` (`Finished::Imported` for `id`), once it arrived. The host
    /// collects `CorePort::finished()` in its pump and keeps the imports' answers until asked; a host that does not
    /// (the tests' fake answers at once) returns `None`
    fn take_imported(&mut self, id: RequestId) -> Option<lm_port::Result<lm_port::Resource>> {
        let _ = id;
        None
    }
    /// Nobody waits for the request any more: its answer, if it comes, is dropped
    fn forget_request(&mut self, id: RequestId) {
        let _ = id;
    }

    fn gesture_begin(&mut self, label: Label) -> Gesture {
        let (port, gestures) = self.gestures_and_port();
        gestures.begin(port, label)
    }
    fn gesture_apply(&mut self, g: Gesture, ops: Vec<EditOp>) -> bool {
        let (port, gestures) = self.gestures_and_port();
        gestures.apply(port, g, ops)
    }
    fn gesture_commit(&mut self, g: Gesture) -> bool {
        let (port, gestures) = self.gestures_and_port();
        gestures.commit(port, g)
    }
    fn gesture_cancel(&mut self, g: Gesture) -> bool {
        let (port, gestures) = self.gestures_and_port();
        gestures.cancel(port, g)
    }
    fn gesture_is_open(&mut self, g: Gesture) -> bool {
        self.gestures_and_port().1.is_open(g)
    }
    /// Commits every gesture still open; how many
    fn close_gestures(&mut self) -> usize {
        let (port, gestures) = self.gestures_and_port();
        gestures.commit_all(port)
    }

    /// A call to the port, its result settled here: an error is shown, a request that waits is followed. Gives the
    /// value when the answer was ready and good
    fn report<T>(&mut self, call: impl FnOnce(&mut dyn CorePort) -> Pending<T>) -> Option<T>
    where
        Self: Sized,
    {
        let pending = call(self.port_mut());
        self.settle(Outcome::of(&pending));
        pending.ok()
    }
}

/// An element that closes the gestures still open when a mouse button is released anywhere (inside the window or
/// not): the views' own release handlers have run by then, so only what they missed is left. Put it once in every
/// window
pub fn release_guard<H: PortHost>(host: WeakEntity<H>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let host = host.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    let host = host.clone();
                    // after the whole dispatch of this event
                    cx.defer(move |cx| {
                        let _ = host.update(cx, |h, _| h.close_gestures());
                    });
                }
            });
        },
    )
    .absolute()
    .size(px(0.))
}

/// A window that loses the focus in the middle of a drag closes its gestures (the release will never come)
pub fn guard_activation<H: PortHost, V: 'static>(
    host: Entity<H>,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Subscription {
    cx.observe_window_activation(window, move |_, window, cx| {
        if !window.is_window_active() {
            host.update(cx, |h, _| h.close_gestures());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lm_port::{NodeId, Value};
    use lm_port_fake::{Config, FakePort};

    fn port() -> FakePort {
        FakePort::new(Config { tracks: 2, slow_delay: None, slow_track: usize::MAX }).expect("the demo")
    }

    fn a_track(port: &FakePort) -> NodeId {
        let t = port.root().get("tracks").and_then(|l| l.at(0)).expect("a track");
        NodeId::parse(t.get("id").and_then(Value::as_text).expect("an id")).expect("a valid id")
    }

    #[test]
    fn a_closed_gesture_refuses_its_edits() {
        let mut p = port();
        let track = a_track(&p);
        let mut gestures = Gestures::default();
        let g = gestures.begin(&mut p, Label::new("test"));
        assert!(gestures.is_open(g));
        assert!(gestures.apply(&mut p, g, vec![EditOp::set(track, &["name"], Value::text("A"))]));
        assert_eq!(gestures.commit_all(&mut p), 1);
        assert_eq!(gestures.count(), 0);
        // the drag that still holds the token writes nothing and commits nothing
        assert!(!gestures.apply(&mut p, g, vec![EditOp::set(track, &["name"], Value::text("B"))]));
        assert!(!gestures.commit(&mut p, g));
        assert!(!gestures.cancel(&mut p, g));
        let name = p.root().get("tracks").and_then(|l| l.at(0)).and_then(|t| t.get("name")).and_then(Value::as_text);
        assert_eq!(name, Some("A"));
    }

    #[test]
    fn cancel_puts_the_values_back() {
        let mut p = port();
        let track = a_track(&p);
        let before = p.root().get("tracks").and_then(|l| l.at(0)).and_then(|t| t.get("name")).cloned();
        let mut gestures = Gestures::default();
        let g = gestures.begin(&mut p, Label::new("test"));
        gestures.apply(&mut p, g, vec![EditOp::set(track, &["name"], Value::text("changed"))]);
        assert!(gestures.cancel(&mut p, g));
        let after = p.root().get("tracks").and_then(|l| l.at(0)).and_then(|t| t.get("name")).cloned();
        assert_eq!(before, after);
    }

    #[test]
    fn an_outcome_tells_the_three_results() {
        let ok: Pending<()> = Pending::Ready(Ok(()));
        let bad: Pending<()> = Pending::Ready(Err(Error::new("test.bad")));
        let wait: Pending<()> = Pending::Waiting(7);
        assert!(matches!(Outcome::of(&ok), Outcome::Fine));
        assert!(matches!(Outcome::of(&bad), Outcome::Failed(e) if e.key() == "test.bad"));
        assert!(matches!(Outcome::of(&wait), Outcome::Waiting(7)));
    }
}
