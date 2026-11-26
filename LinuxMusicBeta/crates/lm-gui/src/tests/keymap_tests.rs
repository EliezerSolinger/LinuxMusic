//! The keymaps in the running interface: a preset chosen for a context rebinds its keys, and the Shortcuts page
//! chooses it and restores it.

use gpui::TestAppContext;
use lm_port::Value;

use super::*;
use crate::keymap;

/// A preset chosen in the preferences binds its keys
#[gpui::test]
fn a_chosen_preset_rebinds_the_contexts_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let prefs =
        Value::object([("keymaps", Value::object([("Global", keymap::with_preset(&Value::NULL, "Global", "LMMS"))]))]);
    cx.update(|_, cx| {
        cx.clear_key_bindings();
        crate::actions::bind_keys(cx, &prefs)
    });
    keys(cx, "ctrl-5");
    assert_eq!(ws.read_with(cx, |ws, _| ws.page), Page::Mixing, "the LMMS preset puts the mixer on Ctrl+5");
}

/// The Shortcuts page: a preset is chosen from the context's menu; "Restore the preset" clears the changes
#[gpui::test]
fn settings_choose_and_restore_a_preset(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Settings, Some("keys"));
    click(cx, "kpreset-0");
    click(cx, "kpreset-option-0-1");
    let chosen = pref_at(&ws, cx, &["keymaps", "Global", "preset"]).and_then(|v| v.as_text().map(str::to_string));
    assert_eq!(chosen.as_deref(), Some("LMMS"));
    let i = REGISTRY.iter().position(|d| d.id == "view.mixing").unwrap();
    click(cx, &format!("key-change-{i}"));
    keys(cx, "alt-9");
    let own =
        pref_at(&ws, cx, &["keymaps", "Global", "keys", "view.mixing"]).and_then(|v| v.as_text().map(str::to_string));
    assert_eq!(own.as_deref(), Some("alt-9"));
    click(cx, "krestore");
    assert!(pref_at(&ws, cx, &["keymaps", "Global", "keys"]).is_none(), "the changes are gone");
    let chosen = pref_at(&ws, cx, &["keymaps", "Global", "preset"]).and_then(|v| v.as_text().map(str::to_string));
    assert_eq!(chosen.as_deref(), Some("LMMS"), "the choice stays");
}

/// A mouse action's gesture is captured on its box
#[gpui::test]
fn settings_change_a_mouse_gesture(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Settings, Some("keys"));
    let i = keymap::CONTEXTS.iter().position(|c| c.id == "Automation").unwrap();
    click(cx, &format!("kctx-{i}"));
    click(cx, "ktab-1");
    let at = keymap::MOUSE.iter().position(|m| m.id == "au.extend").unwrap();
    click(cx, &format!("mouse-change-{at}"));
    click(cx, "mouse-capture");
    let own = pref_at(&ws, cx, &["keymaps", "Automation", "mouse", "au.extend"])
        .and_then(|v| v.as_text().map(str::to_string));
    assert_eq!(own.as_deref(), Some("click"), "a plain click was captured");
}
