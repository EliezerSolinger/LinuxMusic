//! Keymaps per screen (user, 26/11). Every key and every mouse gesture of the program belongs to a CONTEXT (Global,
//! Transport, Composition, Mixing, PianoRoll, Automation, Steps, Browser, Instrument, Effect). A context has its
//! own PRESET (the data of `keymaps/<context>/<preset>.json`: LMMS, REAPER, the user's own) and the user's changes on
//! top of it, both in the preferences (`keymaps.<context>`). What a key does is an action of the registry
//! (`actions.rs`); what a mouse gesture does is a mouse action of the table (`mouse_defs.rs`), read by the handlers
//! through [`fires`], [`holds`] and [`pick_wheel`], never by looking at the modifiers themselves.
//!
//! The order of precedence for a key or a gesture: the user's change, else the chosen preset, else the built-in
//! default (the registry's and the table's own values: the preset "Default").

mod gesture;
mod mouse_defs;
pub mod presets;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Modifiers, MouseButton};
use lm_port::Value;

use crate::actions::ActionDef;
pub use gesture::{Btn, Gesture, Kind};
pub use mouse_defs::MOUSE;
pub use presets::Preset;

/// The name of the built-in preset: the registry's and the table's own values
pub const DEFAULT: &str = "Default";

pub struct CtxDef {
    pub id: &'static str,
    /// English, translated by `ui::tr`
    pub label: &'static str,
}

/// The contexts, in the order the settings list them
pub const CONTEXTS: &[CtxDef] = &[
    CtxDef { id: "Global", label: "Global" },
    CtxDef { id: "Transport", label: "Transport" },
    CtxDef { id: "Composition", label: "Composition" },
    CtxDef { id: "Mixing", label: "Mixing" },
    CtxDef { id: "PianoRoll", label: "Piano Roll" },
    CtxDef { id: "Automation", label: "Automation" },
    CtxDef { id: "Steps", label: "Steps" },
    CtxDef { id: "Browser", label: "Browser" },
    CtxDef { id: "Instrument", label: "Instrument" },
    CtxDef { id: "Effect", label: "Effect" },
];

/// One mouse action of the table
#[derive(Clone, Copy, Debug)]
pub struct MouseDef {
    pub id: &'static str,
    pub ctx: &'static str,
    pub label: &'static str,
    /// What the pointer is over: "Item", "Item edge", "Empty row", "Ruler", "Note", "Wheel"...
    pub target: &'static str,
    pub gestures: &'static [&'static str],
}

/// The keymap context of an action: where its keys work
pub fn ctx_of(def: &ActionDef) -> &'static str {
    match def.context {
        None => "Global",
        Some("Pages") => "Transport",
        Some("AutomationEditor") => "Automation",
        Some("StepEditor") => "Steps",
        Some(c) => c,
    }
}

// --- the choice and the changes, in the preferences -------------------------------------------------------------------

fn ctx_node<'a>(prefs: &'a Value, ctx: &str) -> Option<&'a Value> {
    prefs.get("keymaps").and_then(|k| k.get(ctx))
}

/// The preset chosen for a context
pub fn chosen<'a>(prefs: &'a Value, ctx: &str) -> &'a str {
    ctx_node(prefs, ctx).and_then(|n| n.get("preset")).and_then(Value::as_text).unwrap_or(DEFAULT)
}

fn list_of(v: &Value) -> Vec<String> {
    match v.as_text() {
        Some("") => Vec::new(),
        Some(t) => vec![t.to_string()],
        None => v.items().filter_map(Value::as_text).map(str::to_string).collect(),
    }
}

/// What the user changed: `Some(vec![])` is "none" on purpose
pub fn user_change(prefs: &Value, ctx: &str, section: &str, id: &str) -> Option<Vec<String>> {
    ctx_node(prefs, ctx).and_then(|n| n.get(section)).and_then(|s| s.get(id)).map(list_of)
}

fn preset_of<'a>(prefs: &Value, ctx: &str, user: &'a [Preset]) -> Option<&'a Preset> {
    match chosen(prefs, ctx) {
        DEFAULT => None,
        name => presets::find(ctx, name, user),
    }
}

/// The keys of an action: the user's change, else the chosen preset, else the defaults
pub fn keys_for(def: &ActionDef, prefs: &Value, user: &[Preset]) -> Vec<String> {
    let ctx = ctx_of(def);
    user_change(prefs, ctx, "keys", def.id)
        .or_else(|| preset_of(prefs, ctx, user).and_then(|p| p.keys.get(def.id).cloned()))
        .unwrap_or_else(|| def.keys.iter().map(|k| k.to_string()).collect())
}

/// The gestures of a mouse action, same order of precedence
pub fn gestures_for(def: &MouseDef, prefs: &Value, user: &[Preset]) -> Vec<Gesture> {
    let texts = user_change(prefs, def.ctx, "mouse", def.id)
        .or_else(|| preset_of(prefs, def.ctx, user).and_then(|p| p.mouse.get(def.id).cloned()))
        .unwrap_or_else(|| def.gestures.iter().map(|g| g.to_string()).collect());
    texts.iter().filter_map(|t| Gesture::parse(t)).collect()
}

fn node_or_empty(prefs: &Value, ctx: &str) -> Value {
    ctx_node(prefs, ctx).cloned().unwrap_or_else(Value::empty_object)
}

fn to_value(list: &[String]) -> Value {
    match list {
        [] => Value::text(""),
        [one] => Value::text(one.as_str()),
        many => Value::list(many.iter().map(|t| Value::text(t.as_str()))),
    }
}

/// The context's node of the preferences with a preset chosen (the user's changes stay)
pub fn with_preset(prefs: &Value, ctx: &str, name: &str) -> Value {
    node_or_empty(prefs, ctx).with("preset", Value::text(name))
}

/// The context's node with one change of a key (`section` "keys") or a gesture (`"mouse"`); `None` takes the
/// change back
pub fn with_change(prefs: &Value, ctx: &str, section: &str, id: &str, to: Option<&[String]>) -> Value {
    let node = node_or_empty(prefs, ctx);
    let sec = node.get(section).cloned().unwrap_or_else(Value::empty_object);
    let sec = match to {
        Some(list) => sec.with(id, to_value(list)),
        None => sec.without(id),
    };
    node.with(section, sec)
}

/// "Restore the preset": the context's changes are gone, its choice stays
pub fn restored(prefs: &Value, ctx: &str) -> Value {
    node_or_empty(prefs, ctx).without("keys").without("mouse")
}

/// Whether a context has any change of the user's
pub fn has_changes(prefs: &Value, ctx: &str) -> bool {
    ctx_node(prefs, ctx)
        .is_some_and(|n| ["keys", "mouse"].iter().any(|s| n.get(s).is_some_and(|v| v.entries().next().is_some())))
}

/// The whole of a context as a preset: what its actions have now (the base of "save as a preset" and "export")
pub fn snapshot(ctx: &str, name: &str, prefs: &Value, user: &[Preset]) -> Preset {
    let keys = crate::actions::REGISTRY
        .iter()
        .filter(|d| ctx_of(d) == ctx)
        .map(|d| (d.id.to_string(), keys_for(d, prefs, user)))
        .collect();
    let mouse = MOUSE
        .iter()
        .filter(|d| d.ctx == ctx)
        .map(|d| (d.id.to_string(), gestures_for(d, prefs, user).iter().map(Gesture::text).collect()))
        .collect();
    Preset { ctx: ctx.to_string(), name: name.to_string(), keys, mouse, unsure: Default::default(), user: true }
}

// --- the table the handlers read ---------------------------------------------------------------------------------------

struct Table {
    rows: Vec<(&'static str, &'static str, Vec<Gesture>)>,
}

thread_local! {
    static TABLE: RefCell<Option<Rc<Table>>> = const { RefCell::new(None) };
    static USER: RefCell<Rc<Vec<Preset>>> = RefCell::new(Rc::new(Vec::new()));
}

/// The user's presets read from disk, as the last `install` had them
pub fn user_presets() -> Rc<Vec<Preset>> {
    USER.with(|u| u.borrow().clone())
}

/// The user's presets, as read from the config folder (at the start, and after the settings changed one)
pub fn set_user_presets(user: Vec<Preset>) {
    USER.with(|u| *u.borrow_mut() = Rc::new(user));
}

/// Makes these preferences (and the user presets set) the ones the handlers read
pub fn install(prefs: &Value) {
    let user = user_presets();
    let rows = MOUSE.iter().map(|d| (d.ctx, d.id, gestures_for(d, prefs, &user))).collect();
    TABLE.with(|t| *t.borrow_mut() = Some(Rc::new(Table { rows })));
}

fn table() -> Rc<Table> {
    TABLE.with(|t| {
        t.borrow_mut()
            .get_or_insert_with(|| {
                let rows = MOUSE.iter().map(|d| (d.ctx, d.id, gestures_for(d, &Value::NULL, &[]))).collect();
                Rc::new(Table { rows })
            })
            .clone()
    })
}

fn gestures(ctx: &str, id: &str) -> Vec<Gesture> {
    let t = table();
    t.rows.iter().find(|(c, i, _)| *c == ctx && *i == id).map(|(_, _, g)| g.clone()).unwrap_or_default()
}

/// A press (or the moves of a drag, `clicks` 1) is this mouse action
pub fn fires(ctx: &str, id: &str, button: MouseButton, clicks: usize, m: &Modifiers) -> bool {
    let b = Btn::of(button);
    gestures(ctx, id).iter().any(|g| g.matches(b, clicks, false, m))
}

/// The modifiers held are the ones of this mouse action (for what a modifier switches while the pointer moves or
/// when the button comes up: free position, copy, the notes under)
pub fn holds(ctx: &str, id: &str, m: &Modifiers) -> bool {
    let secondary = m.control || m.platform;
    gestures(ctx, id).iter().any(|g| g.shift == m.shift && g.alt == m.alt && g.secondary == secondary)
}

/// A wheel turn is this mouse action
pub fn wheel(ctx: &str, id: &str, m: &Modifiers) -> bool {
    gestures(ctx, id).iter().any(|g| g.matches(Btn::Left, 0, true, m))
}

/// The first of `ids` that this wheel turn is
pub fn pick_wheel(ctx: &str, ids: &[&'static str], m: &Modifiers) -> Option<&'static str> {
    ids.iter().copied().find(|id| wheel(ctx, id, m))
}

/// A key the plain typing of a text field must keep: no Cmd/Ctrl/Alt, and not Escape, a function key or Tab. Its
/// binding is off while a text field has the focus (rule of the contexts, 26/11)
pub fn is_typing(keys: &str) -> bool {
    keys.split(' ').next().is_some_and(|stroke| {
        let key = stroke.rsplit('-').next().unwrap_or("");
        let key = if stroke.ends_with("--") { "-" } else { key };
        let plain = !stroke.contains("secondary-")
            && !stroke.contains("ctrl-")
            && !stroke.contains("cmd-")
            && !stroke.contains("alt-");
        plain && !(key == "escape" || key == "tab" || (key.starts_with('f') && key.len() > 1))
    })
}
