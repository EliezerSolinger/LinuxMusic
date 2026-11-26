//! The current key of a menu entry: the menus show what the keymap says now, not what the program shipped with.

use gpui::{SharedString, Stateful, div, prelude::*, px};

use super::{ClipCmd, CompositionPage};
use crate::actions::keys_of;
use crate::screens::palette::keys_label;
use crate::ui::{THEME, c};
use lm_port::Value;

/// The action a clip command is the menu's way of running
fn action_of(cmd: &ClipCmd) -> Option<&'static str> {
    Some(match cmd {
        ClipCmd::Delete => "arrange.delete",
        ClipCmd::Cut => "arrange.cut_selection",
        ClipCmd::Copy => "arrange.copy_selection",
        ClipCmd::Paste => "arrange.paste_at_cursor",
        ClipCmd::Split => "arrange.split_at_cursor",
        ClipCmd::Duplicate => "arrange.duplicate_selection",
        ClipCmd::Glue => "clip.glue_selection",
        ClipCmd::Open => "arrange.open_item",
        _ => return None,
    })
}

/// The keys of the command, as the platform writes them ("⌘C"); `None` when it has none or is not an action
pub(super) fn hint_of(cmd: &ClipCmd, prefs: &Value) -> Option<String> {
    let def = crate::actions::find(action_of(cmd)?)?;
    keys_of(def, prefs).first().map(|k| keys_label(k))
}

impl CompositionPage {
    /// `menu_row` with the key at the right
    pub(super) fn menu_row_hint(id: String, label: String, hint: Option<String>, enabled: bool) -> Stateful<gpui::Div> {
        let sel = id.clone();
        div()
            .id(SharedString::from(id))
            .debug_selector(move || sel.clone())
            .px(px(10.))
            .py(px(3.))
            .flex()
            .gap(px(12.))
            .text_size(px(12.))
            .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(c(THEME.bg4))))
            .when(!enabled, |d| d.text_color(c(THEME.text3)))
            .child(div().flex_1().child(label))
            .children(hint.map(|h| div().text_color(c(THEME.text3)).child(h)))
    }
}
