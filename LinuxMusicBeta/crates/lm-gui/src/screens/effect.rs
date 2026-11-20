//! Owner: agent C. The effect window: a desk channel's effects rack, as the legacy's `EffectRackView` (F-FX-001/002;
//! the layout, the density and the look are the legacy's, changed only where the new model forces it, each change
//! in decisoes-autonomas.md). One card per insert in chain order: the LED that bypasses it, the gear that opens its
//! controls (the generic plugin editor, under the card), its name; a vertical drag moves it up or down; the right
//! button opens its menu (move up, move down, replace with, presets, its point, remove). "Add effect" at the end.
//! In analog mode every card shows its point among the rack's sections (P-20); the digital desk has no sections, so
//! the rack is just the chain, as the legacy's digital mode.
//!
//! Opened on a channel or on one of its inserts (that insert's controls open). Through the port only.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyView, Context, Entity, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Render, Subscription, Window, div, prelude::*, px,
};
use lm_port::{BrowsePage, EditOp, Feed, Loading, NodeId, Pending, PluginInfo, Ref, Target, Topic, Value};

use super::ScreenCx;
use super::browser::{ActiveChannel, Dragged};
use super::instrument::{
    EditorState, MenuItem, ParamHost, Popup, PresetEntry, apply_ops, cancel_target, controls_of, drag_end, drag_move,
    grouped, plugin_chip, plugin_name, popup_layer, run_action, schema_of, tr_first,
};
use crate::core::Core;
use crate::model::{self, ChannelView, PluginView};
use crate::ui::{THEME, button, c, caption, loading_component, tr, trf};

/// The points among the rack's sections (lm-doc's `INSERT_POINTS`, P-20)
const POINTS: [&str; 6] = ["pre-gain", "post-gain", "post-filter", "post-dynamics", "post-eq", "pre-pan"];

fn point_label(p: &str) -> &'static str {
    match p {
        "pre-gain" => "Before the gain",
        "post-gain" => "After the gain",
        "post-filter" => "After the filters",
        "post-dynamics" => "After the dynamics",
        "pre-pan" => "Before the pan",
        _ => "After the EQ",
    }
}

/// The legacy's card: 60 px tall
const CARD_H: f32 = 62.0;

#[derive(Clone, Debug)]
pub enum EfPopup {
    Channels,
    Add,
    Menu(NodeId),
    Replace(NodeId),
    Presets(NodeId),
    Point(NodeId),
    Sidechain(NodeId),
}

/// The channels a channel feeds: by its output, and by its sends (to a channel, or to an insert's sidechain, which
/// feeds the insert's channel)
fn feeds(chs: &[ChannelView], from: NodeId, to: NodeId) -> bool {
    let mut seen = vec![from];
    let mut at = 0;
    while at < seen.len() {
        let id = seen[at];
        at += 1;
        if id == to {
            return true;
        }
        let Some(ch) = chs.iter().find(|c| c.id == id) else { continue };
        let mut next: Vec<NodeId> = ch.output.into_iter().collect();
        for s in &ch.sends {
            let dest = chs.iter().find(|c| c.id == s.to || c.inserts.iter().any(|x| x.id == s.to)).map(|c| c.id);
            next.extend(dest);
        }
        for n in next {
            if !seen.contains(&n) {
                seen.push(n);
            }
        }
    }
    false
}

/// The sends feeding an insert's sidechain port: (the source channel, the send)
fn sidechain_sends(chs: &[ChannelView], ins: NodeId) -> Vec<(NodeId, NodeId)> {
    chs.iter().flat_map(|c| c.sends.iter().filter(|s| s.to == ins).map(|s| (c.id, s.id)).collect::<Vec<_>>()).collect()
}

/// A card being dragged up or down (the legacy's gesture: past the card's edge it moves one place)
struct CardDrag {
    id: NodeId,
    start_y: f32,
}

pub struct EffectWindow {
    scx: ScreenCx,
    channel: Option<NodeId>,
    feed: Option<Feed>,
    /// the inserts whose controls are open (the legacy's gear opened a window per effect)
    open: Vec<NodeId>,
    card_drag: Option<CardDrag>,
    presets: Vec<PresetEntry>,
    editor: EditorState<EfPopup>,
    /// The open inserts' own panels (I-22, lm-gui-plugins), by insert, for the plugin they were made for
    panels: HashMap<NodeId, (Arc<str>, AnyView, Subscription)>,
    _subs: Vec<Subscription>,
}

impl EffectWindow {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&scx.core, |_, _, cx| cx.notify());
        let (channel, open) = {
            let chs = model::channels(scx.core.read(cx).port().root());
            match scx.target {
                Some(t) if t.kind == b'm' => (Some(t), Vec::new()),
                Some(t) => (chs.iter().find(|c| c.inserts.iter().any(|x| x.id == t)).map(|c| c.id), vec![t]),
                None => (chs.iter().find(|c| !c.inserts.is_empty()).or(chs.first()).map(|c| c.id), Vec::new()),
            }
        };
        let mut w = EffectWindow {
            scx,
            channel: None,
            feed: None,
            open,
            card_drag: None,
            presets: Vec::new(),
            editor: EditorState::default(),
            panels: HashMap::new(),
            _subs: vec![observe],
        };
        w.set_channel(channel, cx);
        w
    }

    fn set_channel(&mut self, ch: Option<NodeId>, cx: &mut Context<Self>) {
        if ch != self.channel {
            self.feed = ch.map(|c| self.scx.core.update(cx, |core, _| core.port_mut().feed(Topic::Node(c))));
        }
        self.channel = ch;
        if let Some(c) = ch {
            cx.set_global(ActiveChannel(c));
        }
        cx.notify();
    }

    fn channel_view(&self, cx: &gpui::App) -> Option<ChannelView> {
        let id = self.channel?;
        model::channels(self.scx.core.read(cx).port().root()).into_iter().find(|c| c.id == id)
    }

    fn step_channel(&mut self, d: i32, cx: &mut Context<Self>) {
        let ids: Vec<NodeId> = model::channels(self.scx.core.read(cx).port().root()).iter().map(|c| c.id).collect();
        if ids.is_empty() {
            return;
        }
        let at = self.channel.and_then(|c| ids.iter().position(|x| *x == c)).unwrap_or(0) as i32;
        self.set_channel(Some(ids[(at + d).rem_euclid(ids.len() as i32) as usize]), cx);
    }

    fn new_node(plugin: &str) -> Value {
        Value::object([("plugin", Value::text(plugin)), ("state", Value::empty_object())])
    }

    fn add(&mut self, plugin: &str, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(ch) = self.channel_view(cx) else { return };
        let op = EditOp::Insert {
            temp: 1,
            parent: Ref::Id(ch.id),
            list: "inserts".into(),
            index: index.unwrap_or(ch.inserts.len()).min(ch.inserts.len()),
            node: Self::new_node(plugin),
        };
        let core = self.scx.core.clone();
        apply_ops(&core, "Add an effect", vec![op], cx);
    }

    fn move_to(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        let Some(ch) = self.channel else { return };
        let core = self.scx.core.clone();
        let op = EditOp::Move { id: Ref::Id(id), parent: Ref::Id(ch), list: "inserts".into(), index };
        apply_ops(&core, "Move an effect", vec![op], cx);
    }

    fn reopen(&mut self, p: EfPopup, cx: &mut Context<Self>) {
        let at = self.editor.popup.as_ref().map_or(gpui::point(px(40.), px(80.)), |(_, at)| *at);
        self.editor.popup = Some((Popup::Own(p), at));
        cx.notify();
    }

    fn refresh_presets(&mut self, plugin: &str, cx: &mut Context<Self>) {
        let r = self.scx.core.update(cx, |core, _| core.port_mut().browse(BrowsePage::Presets, None, false));
        if let Pending::Ready(Ok(entries)) = r {
            let mut v: Vec<PresetEntry> = entries
                .into_iter()
                .filter(|e| e.info.get("plugin").and_then(Value::as_text) == Some(plugin))
                .map(|e| (e.id, e.name.clone(), e.factory))
                .collect();
            v.sort_by_key(|(_, n, f)| (!f, n.to_lowercase()));
            self.presets = v;
        }
    }

    /// The legacy's card for one insert
    fn card(
        &self,
        index: usize,
        x: &PluginView,
        plugins: &[PluginInfo],
        analog: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = x.id;
        let on = !x.bypass;
        let info = plugins.iter().find(|p| p.id == x.plugin);
        let installed = info.is_some();
        let loading = self.scx.core.read(cx).port().loading(&Target::Node(id));
        let failed = matches!(loading, Loading::Failed(_));
        let open = self.open.contains(&id);
        let latency = info.map_or(0, |i| i.latency);
        let point: Arc<str> = x.point.clone();
        let dragging = self.card_drag.as_ref().is_some_and(|d| d.id == id);
        div()
            .id(("ef-card", id.number as usize))
            .debug_selector(move || format!("ef-card-{index}"))
            .h(px(CARD_H))
            .flex_none()
            .relative()
            .rounded(px(3.))
            .border_1()
            .border_color(c(if failed { THEME.red } else { THEME.line2 }))
            .bg(c(THEME.bg3))
            .when(dragging, |d| d.opacity(0.3))
            .cursor_ns_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                    w.card_drag = Some(CardDrag { id, start_y: ev.position.y.as_f32() });
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                    w.editor.popup = Some((Popup::Own(EfPopup::Menu(id)), ev.position));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_drop(cx.listener(move |w, d: &Dragged, _, cx| {
                if let Dragged::Plugin(p) = d {
                    let p = p.clone();
                    w.add(&p, Some(index), cx);
                }
            }))
            // the LED: on or bypassed (red when the plugin did not build)
            .child(
                div()
                    .id(("ef-led", id.number as usize))
                    .debug_selector(move || format!("ef-led-{index}"))
                    .absolute()
                    .left(px(4.))
                    .top(px(4.))
                    .cursor_pointer()
                    // the legacy's LED images: green on, off, red when the plugin did not build
                    .child(
                        crate::ui::icon(if failed {
                            "led_red"
                        } else if on {
                            "led_green"
                        } else {
                            "led_off"
                        })
                        .size(px(14.)),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |w, _, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Bypass", vec![EditOp::set(id, &["bypass"], Value::bool(on))], cx);
                    })),
            )
            // W/D: the insert's wet/dry mix (the legacy EffectView's knob; the field `wet`, absent = all wet)
            .child(div().absolute().left(px(26.)).top(px(3.)).child(self.wet_knob(id, x, cx)))
            // the gear: the effect's controls
            .when(installed, |d| {
                d.child(
                    div()
                        .id(("ef-gear", id.number as usize))
                        .debug_selector(move || format!("ef-gear-{index}"))
                        .absolute()
                        .right(px(6.))
                        .top(px(10.))
                        .size(px(24.))
                        .rounded(px(3.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(c(if open { THEME.bg4 } else { THEME.bg2 }))
                        .border_1()
                        .border_color(c(THEME.line2))
                        .cursor_pointer()
                        .child(crate::ui::icon("gear").size(px(18.)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |w, _, _, cx| {
                            if let Some(k) = w.open.iter().position(|x| *x == id) {
                                w.open.remove(k);
                            } else {
                                w.open.push(id);
                            }
                            cx.notify();
                        })),
                )
            })
            // the point among the sections (analog desk, P-20)
            .when(analog, |d| {
                d.child(
                    div()
                        .id(("ef-point", id.number as usize))
                        .debug_selector(move || format!("ef-point-{index}"))
                        .absolute()
                        .left(px(60.))
                        .top(px(3.))
                        .text_size(px(9.5))
                        .text_color(c(THEME.text3))
                        .cursor_pointer()
                        .child(format!("{} ▾", tr(point_label(&point))))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                                w.editor.popup = Some((Popup::Own(EfPopup::Point(id)), ev.position));
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
            })
            .when(latency > 0, |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(36.))
                        .top(px(3.))
                        .text_size(px(9.))
                        .text_color(c(THEME.yellow))
                        .child(trf("{0} smp", &[&latency])),
                )
            })
            // the name, bold, at the bottom (the legacy's place); the plugin's chip before it
            .child(
                div()
                    .absolute()
                    .left(px(5.))
                    .bottom(px(4.))
                    .right(px(36.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(plugin_chip(info, &x.plugin))
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(c(if !installed || !on { THEME.text3 } else { THEME.text }))
                            .child(if installed {
                                plugin_name(plugins, &x.plugin)
                            } else {
                                trf("{0} (not installed)", &[&x.plugin]).into()
                            }),
                    ),
            )
            .into_any_element()
    }

    /// The W/D knob of a card: a small knob of the generic editor's gestures (drag, wheel, middle click, menu)
    fn wet_knob(&self, id: NodeId, x: &PluginView, cx: &Context<Self>) -> AnyElement {
        let wet =
            self.scx.core.read(cx).port().find(id).and_then(|n| n.get("wet").and_then(Value::as_real)).unwrap_or(1.0);
        let k = super::instrument::Ctl {
            node: id,
            plugin: x.plugin.clone(),
            slot: super::instrument::Slot::Wet,
            kind: super::instrument::CtlKind::Knob { min: 0.0, max: 1.0, def: 1.0, log: false },
            unit: "",
            automatable: true,
            value: wet,
            choice: Arc::from(""),
            automated: super::instrument::lanes_of(self.scx.core.read(cx).port().root(), id)
                .iter()
                .any(|l| &*l.2 == "wet"),
            enabled: true,
            shown: None,
        };
        let (kd, km) = (k.clone(), k);
        div()
            .id(("ef-wet", id.number as usize))
            .debug_selector(move || format!("ef-wet-{}", id.number))
            .flex()
            .flex_col()
            .items_center()
            .cursor_ns_resize()
            .child(crate::ui::knob_face(wet, 22.0))
            .child(div().text_size(px(8.5)).text_color(c(THEME.text2)).child(tr("W/D")))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |w, ev: &MouseDownEvent, window, cx| {
                    super::instrument::begin_drag(w, kd.clone(), ev, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                    w.editor.popup = Some((Popup::Param(km.clone()), ev.position));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    /// The controls of an open insert, under its card: the loading in their place while it builds (I-2.3)
    fn controls(&self, x: &PluginView, plugins: &[PluginInfo], cx: &mut Context<Self>) -> Vec<AnyElement> {
        let target = Target::Node(x.id);
        let loading = self.scx.core.read(cx).port().loading(&target);
        let core = self.scx.core.clone();
        if let Some(l) =
            loading_component("ef-loading", &loading, move |_, cx| cancel_target(&core, target.clone(), cx))
        {
            return vec![div().h(px(28.)).debug_selector(|| "ef-loading".into()).child(l).into_any_element()];
        }
        let core = self.scx.core.clone();
        let Some(schema) = schema_of(&core, &x.plugin, cx) else { return Vec::new() };
        let port = core.read(cx).port();
        let Some(node) = port.find(x.id) else { return Vec::new() };
        // the plugin's own look, faithful to the legacy (I-22)
        if let Some((p, view, _)) = self.panels.get(&x.id)
            && *p == x.plugin
        {
            let (w, h) = lm_gui_plugins::panel_size(&x.plugin).unwrap_or((250.0, 250.0));
            return vec![div().flex_none().w(px(w)).h(px(h)).child(view.clone()).into_any_element()];
        }
        let ctls = controls_of(x.id, &node, &schema, port.root());
        grouped(self, ctls, &plugin_name(plugins, &x.plugin), cx)
    }

    /// Keeps a panel for each open insert whose plugin has one, remade when the plugin changes (I-22)
    fn update_panels(&mut self, inserts: &[PluginView], window: &mut Window, cx: &mut Context<Self>) {
        let want: Vec<(NodeId, Arc<str>)> = inserts
            .iter()
            .filter(|x| self.open.contains(&x.id) && lm_gui_plugins::has_panel(&x.plugin))
            .map(|x| (x.id, x.plugin.clone()))
            .collect();
        self.panels.retain(|id, (p, ..)| want.iter().any(|(w, wp)| w == id && wp == p));
        for (id, plugin) in want {
            if self.panels.contains_key(&id) {
                continue;
            }
            let Some(entity) = lm_gui_plugins::open_entity(&plugin, id, self.scx.core.clone(), cx) else { continue };
            let workspace = self.scx.workspace.clone();
            let sub = cx.subscribe_in(&entity, window, move |_, _, e: &lm_gui_plugins::PanelEvent, window, cx| {
                let lm_gui_plugins::PanelEvent::OpenBinding(target) = e;
                crate::ui::midi::open_binding(&workspace, target.clone(), window, cx);
            });
            self.panels.insert(id, (plugin, entity.into(), sub));
        }
    }
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
    fn workspace(&self) -> gpui::WeakEntity<crate::workspace::Workspace> {
        self.scx.workspace.clone()
    }
    fn own_menu(&self, p: &EfPopup, cx: &Context<Self>) -> Vec<MenuItem<Self>> {
        let port = self.scx.core.read(cx).port();
        let chs = model::channels(port.root());
        let Some(ch) = self.channel.and_then(|id| chs.iter().find(|c| c.id == id)) else { return Vec::new() };
        let chid = ch.id;
        let plugins = port.plugins();
        let effects = |make: &dyn Fn(Arc<str>) -> MenuItem<Self>| {
            let mut list: Vec<&PluginInfo> = plugins.iter().filter(|p| !p.instrument).collect();
            list.sort_by_key(|p| (p.third_party, p.category.clone(), p.id.clone()));
            let mut items = Vec::new();
            let mut last: Option<Arc<str>> = None;
            for p in list {
                if last.as_ref() != Some(&p.category) {
                    items.push(MenuItem::heading(tr_first(&[&format!("category.{}", p.category)], &p.category)));
                    last = Some(p.category.clone());
                }
                items.push(make(p.id.clone()));
            }
            items
        };
        match p {
            EfPopup::Channels => chs
                .iter()
                .map(|c| {
                    let id = c.id;
                    MenuItem::new(c.name.to_string(), move |w: &mut Self, _, cx| w.set_channel(Some(id), cx))
                        .checked(id == chid)
                })
                .collect(),
            EfPopup::Add => effects(&|id| {
                let name = plugin_name(&plugins, &id);
                MenuItem::new(name, move |w: &mut Self, _, cx| w.add(&id, None, cx))
            }),
            EfPopup::Replace(ins) => {
                let ins = *ins;
                let Some(at) = ch.inserts.iter().position(|x| x.id == ins) else { return Vec::new() };
                effects(&|id| {
                    let name = plugin_name(&plugins, &id);
                    MenuItem::new(name, move |w: &mut Self, _, cx| {
                        // in the same place, one undo step
                        let op = EditOp::Insert {
                            temp: 1,
                            parent: Ref::Id(chid),
                            list: "inserts".into(),
                            index: at,
                            node: Self::new_node(&id),
                        };
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Replace an effect", vec![EditOp::Remove { id: Ref::Id(ins) }, op], cx);
                    })
                })
            }
            EfPopup::Menu(ins) => {
                let ins = *ins;
                let Some(at) = ch.inserts.iter().position(|x| x.id == ins) else { return Vec::new() };
                let x = &ch.inserts[at];
                let installed = plugins.iter().any(|p| p.id == x.plugin);
                let len = ch.inserts.len();
                let plugin = x.plugin.clone();
                let mut items = vec![
                    MenuItem::new(tr("Move up"), move |w: &mut Self, _, cx| w.move_to(ins, at.saturating_sub(1), cx))
                        .enabled(at > 0),
                    MenuItem::new(tr("Move down"), move |w: &mut Self, _, cx| w.move_to(ins, at + 1, cx))
                        .enabled(at + 1 < len),
                    MenuItem::new(tr("Replace with…"), move |w: &mut Self, _, cx| {
                        w.reopen(EfPopup::Replace(ins), cx)
                    }),
                    MenuItem::new(tr("Load preset…"), move |w: &mut Self, _, cx| {
                        w.refresh_presets(&plugin, cx);
                        w.reopen(EfPopup::Presets(ins), cx);
                    })
                    .enabled(installed),
                    MenuItem::new(tr("Save preset"), move |w: &mut Self, _, cx| {
                        let name = format!("{} {}", tr("Preset"), ins.number);
                        let params = Value::object([
                            ("node", Value::text(ins.to_string())),
                            ("name", Value::text(name.as_str())),
                        ]);
                        let core = w.scx.core.clone();
                        run_action(&core, "plugin.save_preset", params, cx);
                    })
                    .enabled(installed),
                ];
                if plugins.iter().any(|p| p.id == x.plugin && p.sidechain) {
                    items.push(MenuItem::new(tr("Sidechain source…"), move |w: &mut Self, _, cx| {
                        w.reopen(EfPopup::Sidechain(ins), cx)
                    }));
                }
                if model::song(port.root()).analog {
                    items.push(MenuItem::new(tr("Point in the rack…"), move |w: &mut Self, _, cx| {
                        w.reopen(EfPopup::Point(ins), cx)
                    }));
                }
                items.push(MenuItem::new(tr("Remove this plugin"), move |w: &mut Self, _, cx| {
                    let core = w.scx.core.clone();
                    apply_ops(&core, "Remove an effect", vec![EditOp::Remove { id: Ref::Id(ins) }], cx);
                    w.open.retain(|x| *x != ins);
                }));
                items
            }
            EfPopup::Presets(ins) => {
                let ins = *ins;
                let mut items: Vec<MenuItem<Self>> = self
                    .presets
                    .iter()
                    .map(|(entry, name, _)| {
                        let entry = *entry;
                        MenuItem::new(name.to_string(), move |w: &mut Self, _, cx| {
                            let params = Value::object([
                                ("node", Value::text(ins.to_string())),
                                ("entry", Value::int(entry.0 as i64)),
                            ]);
                            let core = w.scx.core.clone();
                            run_action(&core, "plugin.load_preset", params, cx);
                        })
                    })
                    .collect();
                if items.is_empty() {
                    items.push(MenuItem::heading(tr("No presets")));
                }
                items
            }
            EfPopup::Sidechain(ins) => {
                // the source is a send from another channel to the insert's sidechain port (arquitetura-nova.md
                // item 3); a channel this one feeds would make a loop: shown, refused
                let ins = *ins;
                let current = sidechain_sends(&chs, ins);
                let remove: Vec<EditOp> = current.iter().map(|(_, s)| EditOp::Remove { id: Ref::Id(*s) }).collect();
                let none = remove.clone();
                let mut items = vec![
                    MenuItem::new(tr("None"), move |w: &mut Self, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Sidechain source", none.clone(), cx);
                    })
                    .checked(current.is_empty()),
                ];
                for src in chs.iter().filter(|c| c.id != chid && c.kind != model::ChannelKind::Master) {
                    let (sid, count) = (src.id, src.sends.len());
                    let looped = feeds(&chs, chid, sid);
                    let checked = current.iter().any(|(c, _)| *c == sid);
                    let mut ops = remove.clone();
                    let node = Value::object([
                        ("to", Value::text(ins.to_string())),
                        ("port", Value::text("sidechain")),
                        ("amount", Value::real(1.0).unwrap_or_default()),
                        ("pre", Value::bool(false)),
                    ]);
                    let index = count - current.iter().filter(|(c, _)| *c == sid).count();
                    ops.push(EditOp::Insert { temp: 1, parent: Ref::Id(sid), list: "sends".into(), index, node });
                    items.push(
                        MenuItem::new(src.name.to_string(), move |w: &mut Self, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Sidechain source", ops.clone(), cx);
                        })
                        .checked(checked)
                        .enabled(!looped),
                    );
                }
                items
            }
            EfPopup::Point(ins) => {
                let ins = *ins;
                let cur = ch.inserts.iter().find(|x| x.id == ins).map_or(Arc::from("post-eq"), |x| x.point.clone());
                POINTS
                    .iter()
                    .map(|p| {
                        let p: &'static str = p;
                        MenuItem::new(tr(point_label(p)), move |w: &mut Self, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Insert point", vec![EditOp::set(ins, &["point"], Value::text(p))], cx);
                        })
                        .checked(*cur == *p)
                    })
                    .collect()
            }
        }
    }
}

impl Render for EffectWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // U38: follow the channel by its feed; a removed channel goes to the master; removed inserts close
        if let Some(feed) = self.feed.as_mut() {
            let _ = self.scx.core.read(cx).port().drain(feed);
        }
        let mut ch = self.channel_view(cx);
        if ch.is_none() && self.channel.is_some() {
            let master = model::channels(self.scx.core.read(cx).port().root())
                .into_iter()
                .find(|c| c.kind == model::ChannelKind::Master);
            self.set_channel(master.as_ref().map(|m| m.id), cx);
            ch = master;
        }
        let Some(ch) = ch else {
            return div().id("effect-window").size_full().bg(c(THEME.bg1)).p(px(10.)).child(caption(tr("No channel.")));
        };
        self.open.retain(|x| ch.inserts.iter().any(|i| i.id == *x));
        self.update_panels(&ch.inserts, window, cx);
        let plugins = self.scx.core.read(cx).port().plugins();
        let analog = model::song(self.scx.core.read(cx).port().root()).analog;
        let latency: u32 =
            ch.inserts.iter().map(|x| plugins.iter().find(|p| p.id == x.plugin).map_or(0, |p| p.latency)).sum();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (k, x) in ch.inserts.iter().enumerate() {
            rows.push(self.card(k, x, &plugins, analog, cx));
            if self.open.contains(&x.id) {
                let ctl = self.controls(x, &plugins, cx);
                rows.push(div().flex().flex_col().gap(px(6.)).pl(px(8.)).children(ctl).into_any_element());
            }
        }
        if ch.inserts.is_empty() {
            rows.push(caption(tr("No effects yet.")).into_any_element());
        }
        div()
            .id("effect-window")
            .debug_selector(|| "effect-window".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .on_mouse_move(cx.listener(|w, ev: &MouseMoveEvent, _, cx| {
                // a card dragged past its edge moves one place, as the legacy's
                if let Some(d) = &w.card_drag {
                    let dy = ev.position.y.as_f32() - d.start_y;
                    if dy.abs() > CARD_H {
                        let id = d.id;
                        let at = w.channel_view(cx).and_then(|c| c.inserts.iter().position(|x| x.id == id));
                        if let Some(at) = at {
                            let to = if dy < 0.0 { at.saturating_sub(1) } else { at + 1 };
                            w.move_to(id, to, cx);
                        }
                        w.card_drag = Some(CardDrag { id, start_y: ev.position.y.as_f32() });
                    }
                    return;
                }
                drag_move(w, ev, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|w, _: &MouseUpEvent, _, cx| {
                    w.card_drag = None;
                    drag_end(w, cx)
                }),
            )
            .child(
                div()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .bg(c(THEME.bg2))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(c(THEME.text2))
                            .child(tr("EFFECTS CHAIN")),
                    )
                    .child(button("ef-prev", "‹", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_channel(-1, cx))))
                    .child(
                        button("ef-channel", format!("{} ▾", ch.name), false, 0)
                            .debug_selector(|| "ef-channel".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|w, ev: &MouseDownEvent, _, cx| {
                                    w.editor.popup = Some((Popup::Own(EfPopup::Channels), ev.position));
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(button("ef-next", "›", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_channel(1, cx))))
                    .child(div().flex_1())
                    .when(latency > 0, |d| {
                        d.child(
                            div()
                                .debug_selector(|| "ef-latency".into())
                                .text_size(px(10.))
                                .text_color(c(THEME.yellow))
                                .child(trf("Latency {0} smp", &[&latency])),
                        )
                    })
                    .child(button("ef-close", "×", false, 0).on_click(cx.listener(|w, _, _, cx| w.scx.close(cx)))),
            )
            .child(
                div().id("ef-rack").flex_1().overflow_y_scroll().p(px(6.)).flex().flex_col().gap(px(4.)).children(rows),
            )
            .child(
                div().p(px(6.)).border_t_1().border_color(c(THEME.line)).child(
                    button("ef-add", tr("Add effect"), false, 0)
                        .h(px(24.))
                        .w_full()
                        .debug_selector(|| "ef-add".into())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|w, ev: &MouseDownEvent, _, cx| {
                                w.editor.popup = Some((Popup::Own(EfPopup::Add), ev.position));
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        )
                        .on_drop(cx.listener(|w, d: &Dragged, _, cx| {
                            if let Dragged::Plugin(p) = d {
                                let p = p.clone();
                                w.add(&p, None, cx);
                            }
                        })),
                ),
            )
            .children(popup_layer(self, cx))
    }
}
