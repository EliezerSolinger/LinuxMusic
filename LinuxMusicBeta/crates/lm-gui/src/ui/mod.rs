//! The shared interface kit: the theme as data, the translation, and the widgets every screen uses.

pub mod i18n;
pub mod theme;
pub mod widgets;

pub use i18n::{tr, trf};
pub use theme::{THEME, c, track_color};
pub use widgets::*;
