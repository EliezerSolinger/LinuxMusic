//! The shared interface kit: the theme as data, the translation, and the widgets every screen uses.

pub mod i18n;
pub mod icons;
pub mod midi;
pub mod theme;
pub mod widgets;

pub use i18n::{tr, tr_str, trf};
pub use icons::icon;
pub use theme::{THEME, c, track_color};
pub use widgets::*;
