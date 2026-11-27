//! The shared interface kit: the theme as data, the translation, and the widgets every screen uses.

pub mod i18n;
pub mod icons;
pub mod midi;
pub mod theme;
pub mod widgets;

use std::sync::atomic::{AtomicBool, Ordering};

/// `interface.tooltips`, kept by the workspace from the preferences: every tooltip of the program asks it
static TOOLTIPS: AtomicBool = AtomicBool::new(true);

pub fn set_tooltips(on: bool) {
    TOOLTIPS.store(on, Ordering::Relaxed);
}

/// `interface.noteNames` (the Piano Roll writes the name on each note)
static NOTE_NAMES: AtomicBool = AtomicBool::new(true);

pub fn set_note_names(on: bool) {
    NOTE_NAMES.store(on, Ordering::Relaxed);
}

pub fn note_names() -> bool {
    NOTE_NAMES.load(Ordering::Relaxed)
}

pub fn tooltips() -> bool {
    TOOLTIPS.load(Ordering::Relaxed)
}

pub use i18n::{tr, tr_str, trf};
pub use icons::icon;
pub use theme::{THEME, c, track_color};
pub use widgets::*;
