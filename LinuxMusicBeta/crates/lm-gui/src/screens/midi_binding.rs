//! Owner: agent A. The binding window (I-21.2, "a janela de ligação"): one control's MIDI mappings, in the project
//! and on this machine, each with its source and its transform (range, inversion, mode, takeover, button, feedback,
//! consume), to learn a new one (for this project or every project) or remove one. Opened from any control's menu
//! ("MIDI binding…"); the control is the workspace's `binding`. Through the port: the project's mappings are nodes
//! (`midi.mapping`, edits with undo), the machine's are in the preferences.

use gpui::{Context, FontWeight, Render, SharedString, Window, div, prelude::*, px};
use lm_port::{CorePort, EditOp, Label, MapScope, MapTarget, PathStep, Value};

use super::ScreenCx;
use crate::ui::midi::{self, describe, describe_source, mappings_of};
use crate::ui::{THEME, button, c, caption, tr, tr_str};

const MODES: &[&str] = &["absolute", "twos", "signMagnitude", "offset64"];
const TAKEOVERS: &[&str] = &["jump", "pickup", "scale"];
const BUTTONS: &[&str] = &["momentary", "toggle", "trigger"];

pub struct MidiBinding {
    scx: ScreenCx,
    target: Option<MapTarget>,
}

impl MidiBinding {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = cx;
        let target = scx.binding.clone();
        MidiBinding { scx, target }
    }

    /// Sets a key path of a mapping: a node edit in the project, a preference on this machine
    fn set(&mut self, scope: MapScope, index: usize, path: &[&str], value: Value, cx: &mut Context<Self>) {
        self.scx.core.update(cx, |core, _| {
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

    fn remove(&mut self, scope: MapScope, index: usize, cx: &mut Context<Self>) {
        self.scx.core.update(cx, |core, _| {
            match scope {
                MapScope::Project => {
                    let id =
                        core.port().root().get("midiMap").and_then(|l| l.items().nth(index)).and_then(crate::model::id_of);
                    if let Some(id) = id {
                        core.report(|pt| pt.apply(Label::new("MIDI: tirar o mapa"), vec![EditOp::Remove { id: lm_port::Ref::Id(id) }]));
                    }
                }
                MapScope::Machine => {
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

fn node_of(port: &dyn CorePort, scope: MapScope, index: usize) -> Value {
    let list = match scope {
        MapScope::Project => port.root().get("midiMap"),
        MapScope::Machine => port.preferences().get("midiMap"),
    };
    list.and_then(|l| l.items().nth(index)).cloned().unwrap_or_default()
}

impl Render for MidiBinding {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scx = self.scx.clone();
        let Some(target) = self.target.clone() else {
            return div().id("midi-binding").p(px(16.)).child(tr("No control chosen"));
        };
        let port = self.scx.core.read(cx).port();
        let title = describe(port, &target);
        let learning = port.learning().is_some();
        let rows: Vec<_> = mappings_of(port, &target)
            .into_iter()
            .enumerate()
            .map(|(row, (scope, index, m))| {
                let node = node_of(port, scope, index);
                let tf = |k: &str| node.get("transform").and_then(|t| t.get(k)).cloned().unwrap_or_default();
                let text = |k: &str, def: &str| tf(k).as_text().unwrap_or(def).to_string();
                let real = |k: &str, def: f64| tf(k).as_real().unwrap_or(def);
                let flag = |v: Option<&Value>, def: bool| v.and_then(Value::as_bool).unwrap_or(def);
                let (min, max, invert) = (real("min", 0.0), real("max", 1.0), tf("invert").as_bool().unwrap_or(false));
                let (feedback, consume) = (flag(node.get("feedback"), true), flag(node.get("consume"), true));
                let cycle = |key: &'static str, list: &'static [&'static str], now: String| {
                    let next = list[list.iter().position(|x| *x == now).map_or(0, |i| (i + 1) % list.len())];
                    button(SharedString::from(format!("b-{row}-{key}")), tr_str(&now), false, 0)
                        .debug_selector(move || format!("binding-{row}-{key}"))
                        .on_click(cx.listener(move |b, _, _, cx| {
                            b.set(scope, index, &["transform", key], Value::text(next), cx)
                        }))
                };
                let step = |key: &'static str, now: f64, by: f64, label: &'static str| {
                    button(SharedString::from(format!("b-{row}-{key}-{label}")), label, false, 0)
                        .debug_selector(move || format!("binding-{row}-{key}{label}"))
                        .on_click(cx.listener(move |b, _, _, cx| {
                            let v = ((now + by).clamp(0.0, 1.0) * 100.0).round() / 100.0;
                            b.set(scope, index, &["transform", key], Value::real(v).unwrap_or_default(), cx)
                        }))
                };
                let toggle = |key: &'static str, path: &'static [&'static str], on: bool, label: &'static str| {
                    button(SharedString::from(format!("b-{row}-{key}")), tr(label), on, THEME.blue)
                        .debug_selector(move || format!("binding-{row}-{key}"))
                        .on_click(cx.listener(move |b, _, _, cx| b.set(scope, index, path, Value::bool(!on), cx)))
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(8.))
                    .rounded(px(6.))
                    .bg(c(THEME.bg3))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(describe_source(&m.source)),
                            )
                            .child(caption(if scope == MapScope::Project {
                                tr("This project")
                            } else {
                                tr("Every project")
                            }))
                            .child(
                                button(SharedString::from(format!("b-{row}-remove")), tr("Remove"), false, 0)
                                    .debug_selector(move || format!("binding-{row}-remove"))
                                    .on_click(cx.listener(move |b, _, _, cx| b.remove(scope, index, cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.))
                            .child(tr("Range"))
                            .child(step("min", min, -0.05, "−"))
                            .child(format!("{:.0}%", min * 100.0))
                            .child(step("min", min, 0.05, "+"))
                            .child("…")
                            .child(step("max", max, -0.05, "−"))
                            .child(format!("{:.0}%", max * 100.0))
                            .child(step("max", max, 0.05, "+"))
                            .child(toggle("invert", &["transform", "invert"], invert, "Invert")),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.))
                            .child(tr("Mode"))
                            .child(cycle("mode", MODES, text("mode", "absolute")))
                            .child(tr("Takeover"))
                            .child(cycle("takeover", TAKEOVERS, text("takeover", "pickup")))
                            .child(tr("Button"))
                            .child(cycle("button", BUTTONS, text("button", "toggle")))
                            .child(toggle("feedback", &["feedback"], feedback, "Feedback"))
                            .child(toggle("consume", &["consume"], consume, "Consume")),
                    )
            })
            .collect();
        let none = rows.is_empty();
        let (t1, t2) = (target.clone(), target);
        let (c1, c2) = (self.scx.core.clone(), self.scx.core.clone());
        div()
            .id("midi-binding")
            .w(px(640.))
            .max_h(px(600.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .rounded(px(10.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().text_size(px(16.)).font_weight(FontWeight::BOLD).child(format!(
                        "{} — {}",
                        tr("MIDI binding"),
                        title
                    )))
                    .child(button("binding-close", tr("Close"), false, 0).on_click(move |_, _, cx| scx.close(cx))),
            )
            .when(none, |d| d.child(caption(tr("No mappings"))))
            .child(div().id("binding-rows").flex().flex_col().gap(px(8.)).overflow_y_scroll().children(rows))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        button(
                            "binding-learn",
                            if learning { tr("Move a MIDI control…") } else { tr("Learn for this project") },
                            learning,
                            THEME.yellow,
                        )
                        .debug_selector(|| "binding-learn".into())
                        .on_click(move |_, _, cx| midi::learn(&c1, t1.clone(), MapScope::Project, cx)),
                    )
                    .child(
                        button("binding-learn-machine", tr("Learn for every project"), false, 0)
                            .debug_selector(|| "binding-learn-machine".into())
                            .on_click(move |_, _, cx| midi::learn(&c2, t2.clone(), MapScope::Machine, cx)),
                    ),
            )
    }
}
