//! The mouse actions of the Composition page (`keymap/mouse_defs.rs`): the handlers ask these, never the modifiers

use gpui::{Modifiers, MouseButton};

use crate::keymap;

const CTX: &str = "Composition";

/// The modifiers held are the ones of this action (free position, copy, a step's size)
pub(super) fn holds(id: &str, m: &Modifiers) -> bool {
    keymap::holds(CTX, id, m)
}

/// A press (or a double click, a right click) is this action
pub(super) fn fires(id: &str, button: MouseButton, clicks: usize, m: &Modifiers) -> bool {
    keymap::fires(CTX, id, button, clicks, m)
}

pub(super) fn pick_wheel(ids: &[&'static str], m: &Modifiers) -> Option<&'static str> {
    keymap::pick_wheel(CTX, ids, m)
}
