//! The MIDI maps page (I-21.2): every mapping of the project and of this machine in one list, with search, editing in
//! place (the mode, the takeover, the feedback) and in the binding window (everything else), removal, and the
//! activity monitor (the raw MIDI as it arrives, mapped or not). Through the port only.

use gpui::{Context, Entity, FontWeight, Render, SharedString, Subscription, WeakEntity, Window, div, prelude::*, px};
use gpui_component::input::{Input, InputEvent, InputState};
use lm_port::{EditOp, Label, MapScope, Mapping, MidiActivity, PathStep, Ref, Value};

use crate::core::Core;
use crate::ui::midi::{self, describe, describe_source};
use crate::ui::{THEME, button, c, caption, tr, tr_str};
use crate::workspace::Workspace;

/// The activity kept on screen
const SHOWN: usize = 64;
const MODES: &[&str] = &["absolute", "twos", "signMagnitude", "offset64"];
const TAKEOVERS: &[&str] = &["jump", "pickup", "scale"];

pub struct MidiMapsPage {
    core: Entity<Core>,
    workspace: WeakEntity<Workspace>,
    search: Entity<InputState>,
    activity: Vec<MidiActivity>,
    seen: u64,
    _sub: Subscription,
}

impl MidiMapsPage {
    pub fn new(
        core: Entity<Core>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Search the mappings…")));
        let sub = cx.subscribe(&search, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        MidiMapsPage { core, workspace, search, activity: Vec::new(), seen: 0, _sub: sub }
    }

    /// Sets a key path of a mapping: a node edit in the project, a preference on this machine
    fn set(&mut self, scope: MapScope, index: usize, path: &[&str], value: Value, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            match scope {
                MapScope::Project => {
                    let id =
                        core.port().root().get("midiMap").and_then(|l| l.items().nth(index)).and_then(crate::model::id_of);
                    if let Some(id) = id {
                        core.report(|pt| pt.apply(Label::new("MIDI mapping"), vec![EditOp::set(id, path, value)]));
                    }
                }
                MapScope::Machine => {
                    let mut p = vec![PathStep::key("midiMap"), PathStep::Index(index)];
                    p.extend(path.iter().map(|k| PathStep::key(k)));
                    core.report(|pt| pt.set_preference(p, value));
                }
            }
        });
        cx.notify();
    }

    fn remove(&mut self, scope: MapScope, m: &Mapping, index: usize, cx: &mut Context<Self>) {
        let id = m.id;
        self.core.update(cx, |core, _| {
            match (scope, id) {
                (MapScope::Project, Some(id)) => {
                    core.report(|pt| pt.apply(Label::new("MIDI: tirar o mapa"), vec![EditOp::Remove { id: Ref::Id(id) }]));
                }
                _ => {
                    let kept: Vec<Value> = core
                        .port()
                        .preferences()
                        .get("midiMap")
                        .map(|l| l.items().enumerate().filter(|(i, _)| *i != index).map(|(_, v)| v.clone()).collect())
                        .unwrap_or_default();
                    core.report(|pt| pt.set_preference(vec![PathStep::key("midiMap")], Value::list(kept)));
                }
            }
        });
        cx.notify();
    }
}

fn next(list: &'static [&'static str], now: &str) -> &'static str {
    list[list.iter().position(|x| *x == now).map_or(0, |i| (i + 1) % list.len())]
}

impl Render for MidiMapsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // the activity since the last frame (the core redraws when its counter moves)
        let (n, new) = self.core.read(cx).port().midi_activity(self.seen);
        self.seen = n;
        self.activity.extend(new);
        let drop = self.activity.len().saturating_sub(SHOWN);
        self.activity.drain(..drop);
        let query = self.search.read(cx).value().to_lowercase();
        let port = self.core.read(cx).port();
        let mut rows = Vec::new();
        for scope in [MapScope::Project, MapScope::Machine] {
            for (index, m) in midi::mappings(port, scope).into_iter().enumerate() {
                let target = describe(port, &m.target);
                let source = describe_source(&m.source);
                if !query.is_empty() && !format!("{target} {source}").to_lowercase().contains(&query) {
                    continue;
                }
                let row = rows.len();
                let node = match scope {
                    MapScope::Project => port.root().get("midiMap"),
                    MapScope::Machine => port.preferences().get("midiMap"),
                }
                .and_then(|l| l.items().nth(index))
                .cloned()
                .unwrap_or_default();
                let tf = |k: &str| {
                    node.get("transform").and_then(|t| t.get(k)).and_then(Value::as_text).unwrap_or("").to_string()
                };
                let (mode, takeover) = (tf("mode"), tf("takeover"));
                let feedback = node.get("feedback").and_then(Value::as_bool).unwrap_or(true);
                let (m1, m2, t1) = (m.clone(), m.target.clone(), mode.clone());
                let t2 = takeover.clone();
                rows.push(
                    div()
                        .id(SharedString::from(format!("map-{row}")))
                        .debug_selector(move || format!("map-{row}"))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(4.))
                        .border_b_1()
                        .border_color(c(THEME.line))
                        .text_size(px(11.))
                        .child(div().w(px(240.)).overflow_hidden().child(source))
                        .child(div().flex_1().overflow_hidden().child(target))
                        .child(
                            button(SharedString::from(format!("mode-{row}")), tr_str(&mode), false, 0)
                                .debug_selector(move || format!("map-{row}-mode"))
                                .on_click(cx.listener(move |p, _, _, cx| {
                                    p.set(scope, index, &["transform", "mode"], Value::text(next(MODES, &t1)), cx)
                                })),
                        )
                        .child(
                            button(SharedString::from(format!("take-{row}")), tr_str(&takeover), false, 0)
                                .debug_selector(move || format!("map-{row}-takeover"))
                                .on_click(cx.listener(move |p, _, _, cx| {
                                    p.set(
                                        scope,
                                        index,
                                        &["transform", "takeover"],
                                        Value::text(next(TAKEOVERS, &t2)),
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            button(SharedString::from(format!("fb-{row}")), tr("Feedback"), feedback, THEME.blue)
                                .debug_selector(move || format!("map-{row}-feedback"))
                                .on_click(cx.listener(move |p, _, _, cx| {
                                    p.set(scope, index, &["feedback"], Value::bool(!feedback), cx)
                                })),
                        )
                        .child(caption(if scope == MapScope::Project {
                            tr("This project")
                        } else {
                            tr("Every project")
                        }))
                        .child(
                            button(SharedString::from(format!("edit-{row}")), tr("Edit…"), false, 0)
                                .debug_selector(move || format!("map-{row}-edit"))
                                .on_click(cx.listener(move |p, _, window, cx| {
                                    midi::open_binding(&p.workspace, m2.clone(), window, cx)
                                })),
                        )
                        .child(
                            button(SharedString::from(format!("rm-{row}")), tr("Remove"), false, 0)
                                .debug_selector(move || format!("map-{row}-remove"))
                                .on_click(cx.listener(move |p, _, _, cx| p.remove(scope, &m1, index, cx))),
                        ),
                );
            }
        }
        let empty = rows.is_empty();
        let activity: Vec<_> = self
            .activity
            .iter()
            .rev()
            .map(|a| {
                let source = lm_port::DocSource { port: a.port.clone(), channel: Some(a.channel), message: a.message };
                div()
                    .flex()
                    .gap(px(6.))
                    .text_size(px(10.))
                    .text_color(c(if a.mapped { THEME.green } else { THEME.text2 }))
                    .child(div().flex_1().overflow_hidden().child(describe_source(&source)))
                    .child(div().w(px(40.)).child(a.value.to_string()))
            })
            .collect();
        div()
            .id("midi-maps")
            .size_full()
            .flex()
            .flex_row()
            .bg(c(THEME.bg1))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .p(px(12.))
                    .gap(px(8.))
                    .child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child(tr("MIDI maps")))
                    .child(crate::ui::text_entry(Input::new(&self.search)))
                    .when(empty, |d| d.child(caption(tr("No mappings"))))
                    .child(div().id("maps").flex_1().overflow_y_scroll().children(rows)),
            )
            .child(
                div()
                    .id("midi-activity")
                    .debug_selector(|| "midi-activity".into())
                    .w(px(300.))
                    .h_full()
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .border_l_1()
                    .border_color(c(THEME.line))
                    .overflow_y_scroll()
                    .child(div().text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr("MIDI activity")))
                    .children(activity),
            )
    }
}
