//! Dragging on the ruler moves the cursor (F-REG-005): a press locates, the pointer then drags it while the left
//! button is down. The loop's gestures (loop_down) and Ctrl+drag (the selection) are in edit.rs and composition.rs;
//! the cursor itself is the transport's own action (`transport.locate`), nothing here is in the document.

use gpui::{MouseDownEvent, MouseMoveEvent};

use super::*;

impl CompositionPage {
    fn ruler_tick(&self, x: f32) -> i64 {
        self.view.tick_at(x).max(0.0) as i64
    }

    /// A plain left press on the ruler (after the loop and the selection had their turn): the cursor goes there
    /// and follows the pointer until the button goes up
    pub(super) fn ruler_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let b = *self.timeline_bounds.borrow();
        let tick = self.ruler_tick((ev.position.x - b.origin.x).as_f32());
        self.cursor_drag = true;
        self.action("transport.locate", Value::int(tick), cx);
    }

    pub(super) fn ruler_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.cursor_drag {
            return;
        }
        if ev.pressed_button != Some(MouseButton::Left) {
            self.cursor_drag = false;
            return;
        }
        let b = *self.timeline_bounds.borrow();
        let tick = self.ruler_tick((ev.position.x - b.origin.x).as_f32());
        self.action("transport.locate", Value::int(tick), cx);
    }

    pub(super) fn ruler_up(&mut self) {
        self.cursor_drag = false;
    }
}
