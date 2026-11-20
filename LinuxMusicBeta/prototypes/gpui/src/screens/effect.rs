//! Owner: agent C. The effect window: a desk channel's whole signal path, docked under the page. On the left, the
//! chain in signal order (input, the analog rack's sections, the inserts at their points among the sections (P-20),
//! the fader and pan); on the right, the editor of what is selected: a rack section's controls, or an insert's
//! generic editor (the shared one of `instrument.rs`) with its presets, wet/dry, sidechain source and point.
//!
//! Inserts (F-FX-001/002): add (the "+" list by category, or a drop from the browser where it lands), remove,
//! reorder (drag, or move up/down), replace in place, bypass (one or all), presets, the latency of each and of the
//! chain (the desk compensates it, PDC), the effect that wrote NaN (muted, with "reset"), the plugin that is not
//! installed (its state kept), third-party plugins marked as running outside. Every change is an `EditOp` through
//! the `CorePort` (I-15); the window knows only ids and closes on what was removed (U38).

use std::cell::RefCell;
use std::collections::HashMap;

use fake_core::mixer::{self, ChannelKind};
use fake_core::plugins::{self, Descriptor, Host, Kind, ParamSpec, Role, Scale};
use fake_core::port::{EditOp, Ref};
use fake_core::{Doc, Feed, Field, Loading, NodeId, SAMPLE_RATE, Target, Topic, Value};
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Global, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Render, SharedString, Subscription, Window, div, prelude::*, px,
};

use super::instrument::{
    Ctl, EditorState, MenuItem, ParamHost, Popup, apply_ops, cancel_target, control, ctl, drag_end, drag_move,
    generic_editor, group_card, notice, ops, plugin_chip, popup_layer,
};
use super::{Screen, ScreenCx};
use crate::core::Core;
use crate::ui::{THEME, button, c, caption, loading_component, tr, trf};

/// The rack's parameters as the generic editor's specs (the desk keeps choices as indices: they show as steppers
/// with the choice's name; on/off as switches)
pub fn rack_spec(key: &str) -> Option<&'static ParamSpec> {
    thread_local! {
        static CACHE: RefCell<HashMap<&'static str, &'static ParamSpec>> = RefCell::new(HashMap::new());
    }
    if let Some(s) = CACHE.with(|c| c.borrow().get(key).copied()) {
        return Some(s);
    }
    let made: ParamSpec = if let Some(sec) = key.strip_prefix("rack.").and_then(|k| k.strip_suffix(".on")) {
        let k = mixer::section_key(sec);
        ParamSpec { key: k, label: "On", group: "", kind: Kind::Bool { def: true }, unit: "", automatable: true }
    } else {
        let m = mixer::spec(key)?;
        let kind = if m.choices == ["Off", "On"] {
            Kind::Bool { def: m.default > 0.5 }
        } else if !m.choices.is_empty() {
            Kind::Real { min: m.min, max: m.max, def: m.default, scale: Scale::Stepped }
        } else if m.unit == "Hz" && m.min > 0.0 {
            Kind::Real { min: m.min, max: m.max, def: m.default, scale: Scale::Log }
        } else {
            Kind::Real { min: m.min, max: m.max, def: m.default, scale: Scale::Linear }
        };
        ParamSpec { key: m.key, label: m.label, group: "", kind, unit: m.unit, automatable: true }
    };
    let s: &'static ParamSpec = Box::leak(Box::new(made));
    CACHE.with(|c| c.borrow_mut().insert(s.key, s));
    Some(s)
}

/// Which channel (and insert) the effect window shows: set by whoever opens it
pub struct EffectFocus(pub NodeId, pub Option<NodeId>);
impl Global for EffectFocus {}

/// Opens the effect window on a desk channel (or moves it there)
pub fn show_channel(scx: &ScreenCx, channel: NodeId, window: &mut Window, cx: &mut App) {
    cx.set_global(EffectFocus(channel, None));
    scx.open(Screen::Effect, window, cx);
}

/// Opens it on one insert
pub fn show_insert(scx: &ScreenCx, channel: NodeId, insert: NodeId, window: &mut Window, cx: &mut App) {
    cx.set_global(EffectFocus(channel, Some(insert)));
    scx.open(Screen::Effect, window, cx);
}

/// What is selected in the chain
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sel {
    Section(&'static str),
    Insert(NodeId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum EfPopup {
    Channels,
    /// the list of effects to add at this point, at the end of its group
    Add(u8),
    Menu(NodeId),
    Replace(NodeId),
    Presets(NodeId),
    Point(NodeId),
    Sidechain(NodeId),
}

/// An insert being dragged to another place in the chain
#[derive(Clone, Debug)]
pub struct DraggedInsert {
    pub id: NodeId,
    pub label: SharedString,
}

/// The chip under the mouse while dragging
pub struct DragChip(pub SharedString);
impl Render for DragChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(3.))
            .rounded(px(4.))
            .bg(c(THEME.bg4))
            .border_1()
            .border_color(c(THEME.green))
            .text_size(px(11.))
            .text_color(c(THEME.text))
            .child(self.0.clone())
    }
}

pub struct EffectWindow {
    scx: ScreenCx,
    channel: Option<NodeId>,
    feed: Option<Feed>,
    sel: Option<Sel>,
    editor: EditorState<EfPopup>,
    _subs: Vec<Subscription>,
}

const LIST_W: f32 = 300.0;

/// The rack's sections and where each insert point falls: (section key, label, the point right after it)
const FLOW: [(&str, &str, Option<u8>); 7] = [
    ("input", "Input", Some(0)),
    ("gain", "Gain stage", Some(1)),
    ("filter", "Filters", Some(2)),
    ("dynamics", "Dynamics", Some(3)),
    ("eq", "EQ", Some(4)),
    ("transformer", "Transformer", None),
    ("tolerance", "Tolerance", None),
];

fn latency_text(samples: u32) -> String {
    format!("{samples} · {:.1} ms", samples as f64 * 1000.0 / SAMPLE_RATE as f64)
}

impl ParamHost<EfPopup> for EffectWindow {
    fn core(&self) -> &Entity<Core> {
        &self.scx.core
    }
    fn editor(&mut self) -> &mut EditorState<EfPopup> {
        &mut self.editor
    }
    fn editor_ref(&self) -> &EditorState<EfPopup> {
        &self.editor
    }
    fn own_menu(&self, p: &EfPopup, cx: &Context<Self>) -> Vec<MenuItem<Self>> {
        let root = self.scx.core.read(cx).port().root();
        let Some(ch) = self.channel.and_then(|id| root.channels.iter().find(|c| c.id == id)) else { return Vec::new() };
        let chid = ch.id;
        let effects = || {
            let mut list: Vec<&'static Descriptor> = plugins::all().filter(|d| d.role == Role::Effect).collect();
            list.sort_by_key(|d| (d.host != Host::Internal, d.category, d.id));
            list
        };
        let by_category = |make: &dyn Fn(&'static str) -> MenuItem<Self>| {
            let mut items = Vec::new();
            let mut last = "";
            for d in effects() {
                if d.category != last {
                    items.push(MenuItem::heading(tr(d.category)));
                    last = d.category;
                }
                items.push(make(d.id));
            }
            items
        };
        match p {
            EfPopup::Channels => root
                .channels
                .iter()
                .map(|c| {
                    let id = c.id;
                    MenuItem::new(c.name.clone(), move |w: &mut Self, _, cx| w.set_channel(Some(id), None, cx)).checked(id == chid)
                })
                .collect(),
            EfPopup::Add(point) => {
                let point = *point;
                // at the end of the point's group
                let index = ch.inserts.iter().rposition(|x| x.point <= point).map_or(0, |k| k + 1);
                by_category(&move |id| {
                    MenuItem::new(tr(id), move |w: &mut Self, _, cx| w.add(id, index, point, cx))
                })
            }
            EfPopup::Replace(ins) => {
                let ins = *ins;
                let Some(at) = ch.inserts.iter().position(|x| x.id == ins) else { return Vec::new() };
                let point = ch.inserts[at].point;
                by_category(&move |id| {
                    MenuItem::new(tr(id), move |w: &mut Self, _, cx| {
                        let core = w.scx.core.clone();
                        // remove and add in the same place: one step; the point is set on the new id after
                        let edits = vec![EditOp::Remove { id: Ref::Id(ins) }, ops::insert(chid, at, id)];
                        if let Some(a) = apply_ops(&core, "Replace an effect", edits, cx) {
                            if let Some((_, new)) = a.created.first() {
                                apply_ops(&core, "Replace an effect", vec![EditOp::set(chid, Field::InsertPoint(*new), Value::Real(f64::from(point)))], cx);
                                w.sel = Some(Sel::Insert(*new));
                            }
                        }
                    })
                })
            }
            EfPopup::Menu(ins) => {
                let ins = *ins;
                let Some(at) = ch.inserts.iter().position(|x| x.id == ins) else { return Vec::new() };
                let x = &ch.inserts[at];
                let d = plugins::descriptor(x.plugin);
                let n = root.plugins.node(ins);
                let faulted = n.is_some_and(|n| n.fault.is_some());
                let len = ch.inserts.len();
                let bypass = x.bypass;
                let mut items = vec![
                    MenuItem::new(if bypass { tr("Turn on") } else { tr("Bypass") }, move |w: &mut Self, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Bypass", vec![EditOp::set(chid, Field::Bypass(ins), Value::Bool(!bypass))], cx);
                    }),
                    MenuItem::new(tr("Move up"), move |w: &mut Self, _, cx| w.move_to(ins, at.saturating_sub(1), None, cx)).enabled(at > 0),
                    MenuItem::new(tr("Move down"), move |w: &mut Self, _, cx| w.move_to(ins, at + 1, None, cx)).enabled(at + 1 < len),
                    MenuItem::new(tr("Replace with…"), move |w: &mut Self, _, cx| w.reopen(EfPopup::Replace(ins), cx)).enabled(true),
                    MenuItem::new(tr("Point in the rack…"), move |w: &mut Self, _, cx| w.reopen(EfPopup::Point(ins), cx)),
                ];
                if d.is_some_and(|d| !d.presets.is_empty() || !plugins::user_presets(d.id).is_empty()) {
                    items.push(MenuItem::new(tr("Presets…"), move |w: &mut Self, _, cx| w.reopen(EfPopup::Presets(ins), cx)));
                }
                if let Some(n) = n.filter(|_| d.is_some()) {
                    let name = plugins::new_preset_name(n);
                    items.push(MenuItem::new(tr("Save preset"), move |w: &mut Self, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Save a preset", vec![ops::text(ins, "preset.save", &name)], cx);
                    }));
                }
                if d.is_some_and(|d| d.sidechain) {
                    items.push(MenuItem::new(tr("Sidechain source…"), move |w: &mut Self, _, cx| w.reopen(EfPopup::Sidechain(ins), cx)));
                }
                if faulted {
                    items.push(MenuItem::new(tr("Reset the effect"), move |w: &mut Self, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Reset the effect", vec![ops::state(ins, "fault", Value::Bool(false))], cx);
                    }));
                }
                items.push(MenuItem::new(tr("Remove"), move |w: &mut Self, _, cx| w.remove(ins, cx)));
                items
            }
            EfPopup::Presets(ins) => {
                let ins = *ins;
                let Some(n) = root.plugins.node(ins) else { return Vec::new() };
                let cur = n.preset.clone();
                plugins::presets_of(n.plugin)
                    .into_iter()
                    .map(|name| {
                        let checked = cur.as_deref() == Some(name.as_str());
                        let nm = name.clone();
                        MenuItem::new(name, move |w: &mut Self, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Apply a preset", vec![ops::text(ins, "preset", &nm)], cx);
                        })
                        .checked(checked)
                    })
                    .collect()
            }
            EfPopup::Point(ins) => {
                let ins = *ins;
                let cur = ch.inserts.iter().find(|x| x.id == ins).map_or(1, |x| x.point);
                mixer::INSERT_POINTS
                    .iter()
                    .enumerate()
                    .map(|(k, l)| {
                        MenuItem::new(tr(l), move |w: &mut Self, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Insert point", vec![EditOp::set(chid, Field::InsertPoint(ins), Value::Real(k as f64))], cx);
                        })
                        .checked(k as u8 == cur)
                    })
                    .collect()
            }
            EfPopup::Sidechain(ins) => {
                let ins = *ins;
                let cur = root.plugins.node(ins).and_then(|n| n.sidechain);
                let mut items = vec![MenuItem::new(tr("None"), move |w: &mut Self, _, cx| {
                    let core = w.scx.core.clone();
                    apply_ops(&core, "Sidechain", vec![ops::state(ins, "sidechain", Value::Id(fake_core::ROOT))], cx);
                })
                .checked(cur.is_none())];
                for c in root.channels.iter().filter(|c| c.id != chid && c.kind != ChannelKind::Master) {
                    let src = c.id;
                    // a source this channel feeds would be a cycle: the core refuses it, the list greys it out
                    let feeds = feeds(root, chid, src);
                    items.push(
                        MenuItem::new(c.name.clone(), move |w: &mut Self, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Sidechain", vec![ops::state(ins, "sidechain", Value::Id(src))], cx);
                        })
                        .checked(cur == Some(src))
                        .enabled(!feeds),
                    );
                }
                items
            }
        }
    }
}

/// Whether `from` reaches `to` through the channels' outputs
fn feeds(root: &Doc, from: NodeId, to: NodeId) -> bool {
    let mut at = Some(from);
    let mut guard = 0;
    while let Some(n) = at {
        if n == to {
            return true;
        }
        guard += 1;
        if guard > 256 {
            return false;
        }
        at = root.channels.iter().find(|c| c.id == n).and_then(|c| c.output);
    }
    false
}

impl EffectWindow {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&scx.core, |_, _, cx| cx.notify());
        let focus = cx.observe_global::<EffectFocus>(|w: &mut Self, cx| {
            let f = cx.global::<EffectFocus>();
            let (ch, ins) = (f.0, f.1);
            w.set_channel(Some(ch), ins, cx);
        });
        let mut w = EffectWindow { scx, channel: None, feed: None, sel: None, editor: EditorState::default(), _subs: vec![observe, focus] };
        let (ch, ins) = match cx.try_global::<EffectFocus>() {
            Some(f) => (Some(f.0), f.1),
            None => {
                // the first channel with inserts
                let root = w.scx.core.read(cx).port().root();
                (root.channels.iter().find(|c| !c.inserts.is_empty()).or(root.channels.first()).map(|c| c.id), None)
            }
        };
        w.set_channel(ch, ins, cx);
        w
    }

    fn set_channel(&mut self, ch: Option<NodeId>, ins: Option<NodeId>, cx: &mut Context<Self>) {
        if ch != self.channel {
            self.feed = ch.map(|c| self.scx.core.update(cx, |core, _| core.port_mut().feed(Topic::Node(c))));
        }
        self.channel = ch;
        self.sel = ins.map(Sel::Insert).or_else(|| {
            let root = self.scx.core.read(cx).port().root();
            let first = ch.and_then(|id| root.channels.iter().find(|c| c.id == id)).and_then(|c| c.inserts.first()).map(|x| x.id);
            Some(first.map_or(Sel::Section("input"), Sel::Insert))
        });
        cx.notify();
    }

    fn step_channel(&mut self, d: i32, cx: &mut Context<Self>) {
        let root = self.scx.core.read(cx).port().root();
        let ids: Vec<NodeId> = root.channels.iter().map(|c| c.id).collect();
        if ids.is_empty() {
            return;
        }
        let at = self.channel.and_then(|c| ids.iter().position(|x| *x == c)).unwrap_or(0) as i32;
        let next = ids[(at + d).rem_euclid(ids.len() as i32) as usize];
        self.set_channel(Some(next), None, cx);
    }

    fn reopen(&mut self, p: EfPopup, cx: &mut Context<Self>) {
        // the item's action ran after the popup closed: open the next one where the last was
        let at = self.editor.popup.as_ref().map(|(_, at)| *at);
        let at = at.unwrap_or_else(|| gpui::point(px(LIST_W), px(80.)));
        self.editor.popup = Some((Popup::Own(p), at));
        cx.notify();
    }

    fn add(&mut self, plugin: &'static str, index: usize, point: u8, cx: &mut Context<Self>) {
        let Some(ch) = self.channel else { return };
        let core = self.scx.core.clone();
        if let Some(a) = apply_ops(&core, "Add an effect", vec![ops::insert(ch, index, plugin)], cx) {
            if let Some((_, id)) = a.created.first() {
                if point != 1 {
                    apply_ops(&core, "Add an effect", vec![EditOp::set(ch, Field::InsertPoint(*id), Value::Real(f64::from(point)))], cx);
                }
                self.sel = Some(Sel::Insert(*id));
            }
        }
    }

    fn remove(&mut self, ins: NodeId, cx: &mut Context<Self>) {
        let core = self.scx.core.clone();
        apply_ops(&core, "Remove an effect", vec![EditOp::Remove { id: Ref::Id(ins) }], cx);
        if self.sel == Some(Sel::Insert(ins)) {
            self.sel = Some(Sel::Section("input"));
        }
    }

    /// Moves an insert to `index` of the chain; to another point when dropped in another group
    fn move_to(&mut self, ins: NodeId, index: usize, point: Option<u8>, cx: &mut Context<Self>) {
        let Some(ch) = self.channel else { return };
        let core = self.scx.core.clone();
        let mut edits = vec![EditOp::Move { id: Ref::Id(ins), parent: Ref::Id(ch), list: "inserts", index }];
        if let Some(p) = point {
            edits.push(EditOp::set(ch, Field::InsertPoint(ins), Value::Real(f64::from(p))));
        }
        apply_ops(&core, "Move an effect", edits, cx);
    }

    fn bypass_all(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(ch) = self.channel else { return };
        let ids: Vec<NodeId> = {
            let root = self.scx.core.read(cx).port().root();
            root.channels.iter().find(|c| c.id == ch).map(|c| c.inserts.iter().map(|x| x.id).collect()).unwrap_or_default()
        };
        let core = self.scx.core.clone();
        apply_ops(&core, "Bypass all", ids.into_iter().map(|x| EditOp::set(ch, Field::Bypass(x), Value::Bool(on))).collect(), cx);
    }

    /// What a drop does on a row: an insert moves there; an effect from the browser is added there; an effect preset
    /// from the browser is added with its preset
    fn drop_at(&mut self, index: usize, point: u8, d: &dyn std::any::Any, cx: &mut Context<Self>) {
        let Some(ch) = self.channel else { return };
        if let Some(x) = d.downcast_ref::<DraggedInsert>() {
            self.move_to(x.id, index, Some(point), cx);
            return;
        }
        let core = self.scx.core.clone();
        match d.downcast_ref::<super::browser::Dragged>() {
            Some(super::browser::Dragged::Plugin(id)) if plugins::descriptor(id).is_some_and(|d| d.role == Role::Effect) => {
                self.add(id, index, point, cx);
            }
            Some(super::browser::Dragged::Entry(e)) => {
                let Some(e) = fake_core::browser::entry(*e) else { return };
                let Some(p) = e.plugin.filter(|p| plugins::descriptor(p).is_some_and(|d| d.role == Role::Effect)) else { return };
                let edits = vec![ops::insert(ch, index, p), ops::on_new("preset", Value::Text(e.name.clone()))];
                if let Some(a) = apply_ops(&core, "Add an effect", edits, cx) {
                    if let Some((_, id)) = a.created.first() {
                        apply_ops(&core, "Add an effect", vec![EditOp::set(ch, Field::InsertPoint(*id), Value::Real(f64::from(point)))], cx);
                        self.sel = Some(Sel::Insert(*id));
                    }
                }
            }
            _ => {}
        }
    }

    // --- the chain ----------------------------------------------------------------------------------------------

    fn top_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let core = self.scx.core.read(cx);
        let root = core.port().root();
        let ch = self.channel.and_then(|id| root.channels.iter().find(|c| c.id == id));
        let name = ch.map_or(String::new(), |c| c.name.clone());
        let latency: u32 = ch.map_or(0, |c| c.inserts.iter().map(|x| plugins::latency(x.plugin)).sum());
        let any_on = ch.is_some_and(|c| c.inserts.iter().any(|x| !x.bypass));
        let kind = ch.map_or("", |c| match c.kind {
            ChannelKind::Normal => "Channel",
            ChannelKind::Group => "Group",
            ChannelKind::Return => "Return",
            ChannelKind::Master => "Master",
        });
        div()
            .h(px(32.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(div().text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr("Effects")))
            .child(button("ef-prev", "‹", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_channel(-1, cx))))
            .child(
                button("ef-channel", format!("{name} ▾"), false, 0)
                    .h(px(22.))
                    .debug_selector(|| "ef-channel".into())
                    .on_mouse_down(MouseButton::Left, cx.listener(|w, ev: &MouseDownEvent, _, cx| {
                        w.editor.popup = Some((Popup::Own(EfPopup::Channels), ev.position));
                        cx.stop_propagation();
                        cx.notify();
                    })),
            )
            .child(button("ef-next", "›", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_channel(1, cx))))
            .child(caption(tr(kind)))
            .child(div().flex_1())
            .child(
                div()
                    .debug_selector(|| "ef-latency".into())
                    .text_size(px(11.))
                    .text_color(c(if latency > 0 { THEME.yellow } else { THEME.text3 }))
                    .child(trf("Chain latency: {0} (compensated)", &[&latency_text(latency)])),
            )
            .child(button("ef-bypass-all", if any_on { tr("Bypass all") } else { tr("Turn all on") }, !any_on, THEME.orange).h(px(22.)).on_click(
                cx.listener(move |w, _, _, cx| w.bypass_all(any_on, cx)),
            ))
            .child(button("ef-close", "×", false, 0).on_click(cx.listener(|w, _, _, cx| w.scx.close(cx))))
    }

    fn section_row(&self, key: &'static str, label: &'static str, on: Option<bool>, cx: &Context<Self>) -> AnyElement {
        let selected = self.sel == Some(Sel::Section(key));
        let chid = self.channel;
        div()
            .id(SharedString::from(format!("ef-sec-{key}")))
            .debug_selector(move || format!("ef-sec-{key}"))
            .h(px(22.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(4.))
            .bg(c(if selected { THEME.bg4 } else { THEME.bg2 }))
            .cursor_pointer()
            .on_click(cx.listener(move |w, _, _, cx| {
                w.sel = Some(Sel::Section(key));
                cx.notify();
            }))
            .child(div().w(px(3.)).h(px(14.)).rounded(px(1.)).bg(c(THEME.orange)))
            .child(div().flex_1().text_size(px(11.5)).text_color(c(if on == Some(false) { THEME.text3 } else { THEME.text })).child(tr(label)))
            .when_some(on, |d, on| {
                let k = mixer::section_key(key);
                d.child(
                    button(SharedString::from(format!("ef-sec-on-{key}")), if on { tr("On") } else { tr("Off") }, on, THEME.orange).on_click(cx.listener(move |w, _, _, cx| {
                        if let Some(ch) = chid {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Rack section", vec![ops::param(ch, k, if on { 0.0 } else { 1.0 })], cx);
                        }
                        cx.stop_propagation();
                    })),
                )
            })
            .into_any_element()
    }

    fn insert_row(&self, index: usize, x: &mixer::Insert, cx: &Context<Self>) -> AnyElement {
        let core = self.scx.core.read(cx);
        let root = core.port().root();
        let id = x.id;
        let d = plugins::descriptor(x.plugin);
        let n = root.plugins.node(id);
        let faulted = n.is_some_and(|n| n.fault.is_some());
        let loading = !matches!(core.port().loading(&Target::Node(id)), Loading::Ready | Loading::Pending);
        let selected = self.sel == Some(Sel::Insert(id));
        let on = !x.bypass;
        let chid = self.channel.unwrap_or(fake_core::ROOT);
        let point = x.point;
        let label: SharedString = tr(x.plugin).into();
        let lat = plugins::latency(x.plugin);
        let name_color = if faulted { THEME.red } else if d.is_none() { THEME.text3 } else if on { THEME.text } else { THEME.text3 };
        div()
            .id(SharedString::from(format!("ef-ins-{}", id.number)))
            .debug_selector(move || format!("ef-ins-{index}"))
            .h(px(24.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(px(4.))
            .border_1()
            .border_color(c(if faulted { THEME.red } else if selected { THEME.green } else { THEME.bg1 }))
            .bg(c(if selected { THEME.bg4 } else { THEME.bg3 }))
            .cursor_pointer()
            .on_click(cx.listener(move |w, _, _, cx| {
                w.sel = Some(Sel::Insert(id));
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                w.sel = Some(Sel::Insert(id));
                w.editor.popup = Some((Popup::Own(EfPopup::Menu(id)), ev.position));
                cx.notify();
            }))
            .on_drag(DraggedInsert { id, label: label.clone() }, |d: &DraggedInsert, _, _, cx| cx.new(|_| DragChip(d.label.clone())))
            .drag_over::<DraggedInsert>(|s, _, _, _| s.border_color(c(THEME.green)))
            .drag_over::<super::browser::Dragged>(|s, _, _, _| s.border_color(c(THEME.green)))
            .on_drop(cx.listener(move |w, d: &DraggedInsert, _, cx| w.drop_at(index, point, d, cx)))
            .on_drop(cx.listener(move |w, d: &super::browser::Dragged, _, cx| w.drop_at(index, point, d, cx)))
            .child(div().text_size(px(10.)).text_color(c(THEME.text3)).child("⋮⋮"))
            .child(
                div()
                    .id(SharedString::from(format!("ef-pwr-{}", id.number)))
                    .debug_selector(move || format!("ef-pwr-{index}"))
                    .size(px(12.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(c(THEME.line2))
                    .bg(c(if faulted { THEME.red } else if on { THEME.green } else { THEME.bg1 }))
                    .on_click(cx.listener(move |w, _, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Bypass", vec![EditOp::set(chid, Field::Bypass(id), Value::Bool(on))], cx);
                        cx.stop_propagation();
                    })),
            )
            .child(plugin_chip(d))
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_size(px(11.5)).text_color(c(name_color)).child(label))
            .when(loading, |e| e.child(div().text_size(px(9.5)).text_color(c(THEME.text3)).child(tr("loading…"))))
            .when(lat > 0, |e| e.child(div().text_size(px(9.)).text_color(c(THEME.yellow)).child(format!("{lat}"))))
            .when(d.is_some_and(|d| d.host != Host::Internal), |e| e.child(div().text_size(px(9.)).text_color(c(THEME.purple)).child("ext")))
            .child(
                button(SharedString::from(format!("ef-more-{}", id.number)), "⋯", false, 0)
                    .debug_selector(move || format!("ef-more-{index}"))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                        w.sel = Some(Sel::Insert(id));
                        w.editor.popup = Some((Popup::Own(EfPopup::Menu(id)), ev.position));
                        cx.stop_propagation();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// A point among the sections: its inserts, and a slot to add or drop into
    fn point_group(&self, point: u8, cx: &Context<Self>) -> AnyElement {
        let root = self.scx.core.read(cx).port().root();
        let Some(ch) = self.channel.and_then(|id| root.channels.iter().find(|c| c.id == id)) else { return div().into_any_element() };
        let rows: Vec<AnyElement> =
            ch.inserts.iter().enumerate().filter(|(_, x)| x.point == point).map(|(k, x)| self.insert_row(k, x, cx)).collect();
        let end = ch.inserts.iter().rposition(|x| x.point <= point).map_or(0, |k| k + 1);
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .pl(px(14.))
            .children(rows)
            .child(
                div()
                    .id(SharedString::from(format!("ef-add-{point}")))
                    .debug_selector(move || format!("ef-add-{point}"))
                    .h(px(16.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .border_1()
                    .border_dashed()
                    .border_color(c(THEME.line2))
                    .text_size(px(9.5))
                    .text_color(c(THEME.text3))
                    .cursor_pointer()
                    .child(trf("+ effect · {0}", &[&tr(mixer::INSERT_POINTS[point as usize])]))
                    .drag_over::<DraggedInsert>(|s, _, _, _| s.border_color(c(THEME.green)).bg(c(THEME.bg3)))
                    .drag_over::<super::browser::Dragged>(|s, _, _, _| s.border_color(c(THEME.green)).bg(c(THEME.bg3)))
                    .on_drop(cx.listener(move |w, d: &DraggedInsert, _, cx| w.drop_at(end, point, d, cx)))
                    .on_drop(cx.listener(move |w, d: &super::browser::Dragged, _, cx| w.drop_at(end, point, d, cx)))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                        w.editor.popup = Some((Popup::Own(EfPopup::Add(point)), ev.position));
                        cx.stop_propagation();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn chain(&self, cx: &Context<Self>) -> impl IntoElement {
        let root = self.scx.core.read(cx).port().root();
        let ch = self.channel.and_then(|id| root.channels.iter().find(|c| c.id == id));
        let master = ch.is_some_and(|c| c.kind == ChannelKind::Master);
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(ch) = ch {
            if master {
                // the master's own section; its inserts are after its PRE stage (the legacy's master point)
                for (k, (key, label, _)) in mixer::MASTER.iter().enumerate() {
                    rows.push(self.section_row(key, label, None, cx));
                    if k == 0 {
                        for p in 0..6u8 {
                            if ch.inserts.iter().any(|x| x.point == p) || p == 4 {
                                rows.push(self.point_group(p, cx));
                            }
                        }
                    }
                }
            } else {
                for (key, label, point) in FLOW {
                    let on = ch.params.get(mixer::section_key(key)).map(|v| *v > 0.5);
                    rows.push(self.section_row(key, label, on, cx));
                    if let Some(p) = point {
                        rows.push(self.point_group(p, cx));
                    }
                }
                rows.push(self.section_row("fader", "Fader", None, cx));
                rows.push(self.point_group(5, cx));
                rows.push(self.section_row("pan", "Pan and output", None, cx));
            }
        }
        div()
            .id("ef-chain")
            .w(px(LIST_W))
            .h_full()
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(3.))
            .bg(c(THEME.bg1))
            .border_r_1()
            .border_color(c(THEME.line))
            .children(rows)
    }

    // --- the editor of the selection -----------------------------------------------------------------------------

    fn editor_area(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let core = self.scx.core.read(cx);
        let root = core.port().root();
        let Some(ch) = self.channel.and_then(|id| root.channels.iter().find(|c| c.id == id)) else {
            return vec![caption(tr("No channel.")).into_any_element()];
        };
        let chid = ch.id;
        match self.sel {
            Some(Sel::Section(sec)) => {
                let specs: Vec<&'static str> = mixer::RACK
                    .iter()
                    .chain(mixer::MASTER.iter())
                    .find(|(k, _, _)| *k == sec)
                    .map(|(_, _, ps)| ps.iter().map(|p| p.key).collect())
                    .unwrap_or_default();
                let title = mixer::RACK.iter().chain(mixer::MASTER.iter()).find(|(k, _, _)| *k == sec).map_or(sec, |(_, l, _)| l);
                if specs.is_empty() {
                    let text = match sec {
                        "tolerance" => tr("The components' tolerance: fixed per channel, seeded by the channel's id (P-21)."),
                        "fader" => tr("The fader and the meter are on the mixing page."),
                        "pan" => tr("The pan, the sends and the output are on the mixing page."),
                        _ => tr("No controls."),
                    };
                    return vec![group_card(tr(title), vec![caption(text).into_any_element()])];
                }
                let controls: Vec<AnyElement> = specs
                    .into_iter()
                    .filter_map(|k| {
                        let spec = rack_spec(k)?;
                        let v = ch.params.get(k).copied().unwrap_or(spec.default_value());
                        let shown = mixer::spec(k).filter(|m| !m.choices.is_empty() && m.choices != ["Off", "On"]).map(|m| {
                            m.choices.get(v.round() as usize).copied().unwrap_or("?").to_string()
                        });
                        Some(control(self, Ctl { node: chid, spec, value: v, choice: "", automated: false, cc: None, enabled: true, shown }, cx))
                    })
                    .collect();
                vec![group_card(tr(title), controls)]
            }
            Some(Sel::Insert(id)) => {
                let Some(at) = ch.inserts.iter().position(|x| x.id == id) else {
                    return vec![caption(tr("Select an effect in the chain.")).into_any_element()];
                };
                let x = &ch.inserts[at];
                let mut out = Vec::new();
                let target = Target::Node(id);
                let loading = core.port().loading(&target);
                let n = root.plugins.node(id).cloned();
                let d = plugins::descriptor(x.plugin);
                // the header: name, where it runs, latency, preset, point
                let preset = n.as_ref().and_then(|n| n.preset.clone().map(|p| format!("{p}{}", if n.modified { " *" } else { "" })));
                out.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(plugin_chip(d))
                        .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child(tr(x.plugin)))
                        .when_some(d.filter(|d| d.host != Host::Internal), |e, _| e.child(caption(tr("runs outside the program"))))
                        .child(caption(trf("latency {0}", &[&latency_text(plugins::latency(x.plugin))])))
                        .child(caption(trf("point: {0}", &[&tr(mixer::INSERT_POINTS[x.point as usize])])))
                        .when_some(preset, |e, p| e.child(caption(trf("Preset: {0}", &[&p]))))
                        .child(div().flex_1())
                        .child(
                            button("ef-menu", tr("More ▾"), false, 0)
                                .h(px(22.))
                                .on_mouse_down(MouseButton::Left, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                                    w.editor.popup = Some((Popup::Own(EfPopup::Menu(id)), ev.position));
                                    cx.stop_propagation();
                                    cx.notify();
                                })),
                        )
                        .into_any_element(),
                );
                let core_e = self.scx.core.clone();
                if let Some(l) = loading_component("ef-loading", &loading, move |_, cx| cancel_target(&core_e, target, cx)) {
                    out.push(div().h(px(36.)).rounded(px(6.)).overflow_hidden().debug_selector(|| "ef-loading".into()).child(l).into_any_element());
                    if matches!(loading, Loading::Visible { .. }) {
                        return out;
                    }
                }
                let Some(n) = n else { return out };
                if let Some(f) = n.fault {
                    out.push(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .p(px(8.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(c(THEME.red))
                            .child(div().flex_1().text_size(px(11.)).text_color(c(THEME.red)).child(tr(f)))
                            .child(button("ef-reset", tr("Reset the effect"), false, 0).h(px(22.)).on_click(cx.listener(move |w, _, _, cx| {
                                let core = w.scx.core.clone();
                                apply_ops(&core, "Reset the effect", vec![ops::state(id, "fault", Value::Bool(false))], cx);
                            })))
                            .into_any_element(),
                    );
                }
                let Some(d) = d else {
                    // not installed: silent, its state kept and saved back (F-INS-024)
                    let kept: Vec<AnyElement> =
                        n.reals.iter().map(|(k, v)| caption(format!("{k} = {v:.3}")).into_any_element()).collect();
                    out.push(notice(trf("\"{0}\" is not installed. The effect passes the sound through; its settings are kept and saved with the project.", &[&x.plugin])));
                    out.push(group_card(tr("Kept settings"), kept));
                    return out;
                };
                // the common controls of every effect (F-FX-001), then the plugin's own groups
                let wet = plugins::spec(d.id, "wet").unwrap();
                let mut common = vec![control(self, ctl(&n, wet), cx)];
                if d.sidechain {
                    let src = n.sidechain.and_then(|s| root.channels.iter().find(|c| c.id == s)).map_or(tr("None").to_string(), |c| c.name.clone());
                    common.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(caption(tr("Sidechain source")))
                            .child(button("ef-sc", format!("{src} ▾"), false, 0).h(px(20.)).on_mouse_down(MouseButton::Left, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                                w.editor.popup = Some((Popup::Own(EfPopup::Sidechain(id)), ev.position));
                                cx.stop_propagation();
                                cx.notify();
                            })))
                            .into_any_element(),
                    );
                }
                out.push(group_card(tr("Common"), common));
                out.extend(generic_editor(self, &n, d, None, cx));
                out.push(caption(tr(d.description)).into_any_element());
                out
            }
            None => vec![caption(tr("Select a section or an effect in the chain.")).into_any_element()],
        }
    }
}

impl Render for EffectWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // U38: follow the channel by its feed; if it is gone, show the master; a removed insert loses the selection
        if let Some(feed) = self.feed.as_mut() {
            let core = self.scx.core.read(cx);
            let _ = core.port().drain(feed);
        }
        let (gone, master, sel_gone) = {
            let root = self.scx.core.read(cx).port().root();
            let ch = self.channel.and_then(|id| root.channels.iter().find(|c| c.id == id));
            let sel_gone = matches!(self.sel, Some(Sel::Insert(i)) if !ch.is_some_and(|c| c.inserts.iter().any(|x| x.id == i)));
            (self.channel.is_some() && ch.is_none(), root.channels.iter().find(|c| c.kind == ChannelKind::Master).map(|c| c.id), sel_gone)
        };
        if gone {
            self.channel = None;
            if let Some(m) = master {
                self.set_channel(Some(m), None, cx);
            }
        } else if sel_gone {
            self.sel = Some(Sel::Section("input"));
        }
        div()
            .id("effect-window")
            .debug_selector(|| "effect-window".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .on_mouse_move(cx.listener(|w, ev: &MouseMoveEvent, _, cx| drag_move(w, ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|w, _: &MouseUpEvent, _, cx| drag_end(w, cx)))
            .child(self.top_bar(cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(self.chain(cx))
                    .child(div().id("ef-editor").flex_1().h_full().overflow_y_scroll().p(px(10.)).flex().flex_col().gap(px(8.)).children(self.editor_area(cx))),
            )
            .children(popup_layer(self, cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::screens::instrument::tests::{click, open};

    /// Add from the "+" list, bypass, move, remove: each one step through the port; the latency adds up
    #[gpui::test]
    fn the_chain_add_bypass_move_remove(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Screen::Effect);
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        let ch = cx.update(|_, cx| cx.try_global::<EffectFocus>().map(|f| f.0));
        let ch = ch.unwrap_or_else(|| core.read_with(cx, |core, _| core.port().root().channels.iter().find(|c| !c.inserts.is_empty()).unwrap().id));
        let count = |core: &Entity<Core>, cx: &mut gpui::VisualTestContext| {
            core.read_with(cx, |core, _| core.port().root().channels.iter().find(|c| c.id == ch).unwrap().inserts.len())
        };
        let before = count(&core, cx);
        // "+ effect" after the EQ: the list's first heading is a category; the second item is its first effect
        click(cx, "ef-add-4");
        assert!(cx.debug_bounds("popup").is_some());
        click(cx, "popup-item-1");
        assert_eq!(count(&core, cx), before + 1);
        let (new, plugin) = core.read_with(cx, |core, _| {
            let c = core.port().root().channels.iter().find(|c| c.id == ch).unwrap();
            let x = c.inserts.iter().find(|x| x.point == 4 && core.port().root().plugins.node(x.id).is_some_and(|n| !n.modified && n.preset.is_none())).unwrap();
            (x.id, x.plugin)
        });
        assert!(plugins::descriptor(plugin).is_some_and(|d| d.role == Role::Effect));
        // bypass with its light
        let idx = core.read_with(cx, |core, _| core.port().root().channels.iter().find(|c| c.id == ch).unwrap().inserts.iter().position(|x| x.id == new).unwrap());
        click(cx, &format!("ef-pwr-{idx}"));
        let bypassed = core.read_with(cx, |core, _| core.port().root().channels.iter().find(|c| c.id == ch).unwrap().inserts[idx].bypass);
        assert!(bypassed);
        // the menu: remove
        click(cx, &format!("ef-more-{idx}"));
        // "Remove" is the last item
        let last = (0..20).rev().find(|k| cx.debug_bounds(Box::leak(format!("popup-item-{k}").into_boxed_str())).is_some()).unwrap();
        click(cx, &format!("popup-item-{last}"));
        assert_eq!(count(&core, cx), before);
        assert!(cx.debug_bounds("ef-latency").is_some());
    }
}
