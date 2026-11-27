//! The modulator editor (decisoes-autonomas, 26/11, item 1): the modulator that moves one parameter, opened from the
//! control's menu ("Modulate…"; the control is the workspace's `binding`). Its kind (LFO, envelope, peak, random,
//! macro), the knobs of that kind, the peak's input, and its targets with their amount and range. Everything is an
//! edit of the `modulator` node through the port (undoable); the knobs are parameters, so they are automatable and
//! MIDI-mappable like any other.

use gpui::{Context, FontWeight, Render, SharedString, Window, div, prelude::*, px};
use lm_port::{CorePort, EditOp, Label, MapTarget, NodeId, NodeRef, Ref, Value};

use super::ScreenCx;
use crate::model::{id_of, ref_of};
use crate::ui::midi::describe;
use crate::ui::{THEME, button, c, caption, tr, tr_str};

const KINDS: &[&str] = &["lfo", "envelope", "peak", "random", "macro"];
const WAVES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "Moog saw", "Exponential", "Noise", "User"];
const MULTIPLIERS: &[&str] = &["x1", "/100", "x100"];
const SYNCS: &[&str] = &["Free", "8", "1", "1/2", "1/4", "1/8", "1/16", "1/32", "Custom"];

/// A knob of a kind: key, label, range, step, and the names of a stepped one
struct Knob {
    key: &'static str,
    label: &'static str,
    min: f64,
    max: f64,
    step: f64,
    names: &'static [&'static str],
}

const fn knob(key: &'static str, label: &'static str, min: f64, max: f64, step: f64) -> Knob {
    Knob { key, label, min, max, step, names: &[] }
}

fn knobs(kind: &str, sync: f64) -> Vec<Knob> {
    match kind {
        "envelope" => vec![
            knob("attack", "Attack", 0.0, 5000.0, 10.0),
            knob("decay", "Decay", 0.0, 5000.0, 10.0),
            knob("sustain", "Sustain", 0.0, 1.0, 0.05),
            knob("release", "Release", 0.0, 5000.0, 10.0),
        ],
        "peak" => vec![
            knob("peakAttack", "Attack", 0.0, 0.999, 0.05),
            knob("peakDecay", "Decay", 0.0, 0.999, 0.05),
            knob("threshold", "Threshold", 0.0, 1.0, 0.05),
            knob("gain", "Gain", 0.0, 32.0, 0.25),
            Knob { key: "abs", label: "Abs", min: 0.0, max: 1.0, step: 1.0, names: &["Off", "On"] },
        ],
        "random" => vec![knob("rate", "Rate", 0.01, 50.0, 0.25), knob("smooth", "Smooth", 0.0, 1.0, 0.05)],
        "macro" => vec![knob("value", "Value", 0.0, 1.0, 0.05)],
        _ => {
            let mut v = vec![
                Knob { key: "wave", label: "Wave", min: 0.0, max: 7.0, step: 1.0, names: WAVES },
                knob("speed", "Speed", 0.01, 20.0, 0.05),
                Knob { key: "multiplier", label: "Multiplier", min: 0.0, max: 2.0, step: 1.0, names: MULTIPLIERS },
                Knob { key: "sync", label: "Sync", min: 0.0, max: 8.0, step: 1.0, names: SYNCS },
            ];
            if sync.round() as i64 == 8 {
                v.push(knob("syncNumerator", "Numerator", 1.0, 32.0, 1.0));
                v.push(knob("syncDenominator", "Denominator", 1.0, 32.0, 1.0));
            }
            v.push(knob("phase", "Phase", 0.0, 360.0, 15.0));
            v.push(knob("depth", "Depth", 0.0, 1.0, 0.05));
            v
        }
    }
}

pub struct Modulator {
    scx: ScreenCx,
    target: Option<MapTarget>,
}

impl Modulator {
    pub fn new(scx: ScreenCx, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let target = scx.binding.clone();
        Modulator { scx, target }
    }

    /// The modulator that has the control as a target
    fn find(&self, port: &dyn CorePort) -> Option<Value> {
        let Some(MapTarget::Param { node: NodeRef::Id(node), key }) = &self.target else { return None };
        let want = node.to_string();
        let mut found = None;
        port.root().walk(&mut |_, v| {
            if found.is_some() || !id_of(v).is_some_and(|i| i.kind == b'n') {
                return;
            }
            let hit = v.get("targets").is_some_and(|ts| {
                ts.items().any(|t| {
                    t.get("node").and_then(Value::as_text) == Some(want.as_str())
                        && t.get("param").and_then(Value::as_text) == Some(key.as_str())
                })
            });
            if hit {
                found = Some(v.clone());
            }
        });
        found
    }

    fn apply(&mut self, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<Self>) {
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.apply(Label::new(label), ops));
        });
        cx.notify();
    }

    fn set_param(&mut self, id: NodeId, key: &str, value: f64, cx: &mut Context<Self>) {
        let v = Value::real(value).unwrap_or_default();
        self.apply("Modulator", vec![EditOp::set(id, &["params", key], v)], cx);
    }

    fn set_field(&mut self, id: NodeId, path: &[&str], value: Value, cx: &mut Context<Self>) {
        self.apply("Modulator", vec![EditOp::set(id, path, value)], cx);
    }

    fn remove(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.apply("Remove modulator", vec![EditOp::Remove { id: Ref::Id(id) }], cx);
        self.scx.close(cx);
    }
}

/// The nodes a peak follower can listen to: the tracks and the desk channels, by name
fn inputs(port: &dyn CorePort) -> Vec<(NodeId, String)> {
    let root = port.root();
    let name = |v: &Value, fallback: &str| {
        v.get("name").and_then(Value::as_text).filter(|n| !n.is_empty()).unwrap_or(fallback).to_string()
    };
    let mut out = Vec::new();
    if let Some(ts) = root.get("tracks") {
        out.extend(ts.items().filter_map(|t| Some((id_of(t)?, name(t, "Track")))));
    }
    if let Some(cs) = root.get("mixer").and_then(|m| m.get("channels")) {
        out.extend(cs.items().filter_map(|t| Some((id_of(t)?, name(t, "Channel")))));
    }
    out
}

impl Render for Modulator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scx = self.scx.clone();
        let port = self.scx.core.read(cx).port();
        let title = self.target.as_ref().map(|t| describe(port, t)).unwrap_or_default();
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(16.))
                    .font_weight(FontWeight::BOLD)
                    .child(format!("{} — {title}", tr("Modulator"))),
            )
            .child(button("mod-close", tr("Close"), false, 0).on_click(move |_, _, cx| scx.close(cx)));
        let shell = |body: gpui::Div| {
            div()
                .id("modulator")
                .w(px(520.))
                .max_h(px(620.))
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .rounded(px(10.))
                .bg(c(THEME.bg2))
                .border_1()
                .border_color(c(THEME.line2))
                .child(header)
                .child(body)
        };
        let Some(m) = self.find(port) else {
            return shell(div().child(caption(tr("No modulator"))));
        };
        let Some(id) = id_of(&m) else { return shell(div()) };
        let kind = m.get("kind").and_then(Value::as_text).unwrap_or("lfo").to_string();
        let param = |k: &str, def: f64| m.get("params").and_then(|p| p.get(k)).and_then(Value::as_real).unwrap_or(def);

        // The kinds
        let kind_row = div().flex().gap(px(4.)).children(KINDS.iter().map(|k| {
            let k = *k;
            button(SharedString::from(format!("mod-kind-{k}")), tr_str(k), kind == k, THEME.blue)
                .debug_selector(move || format!("mod-kind-{k}"))
                .on_click(cx.listener(move |me, _, _, cx| me.set_field(id, &["kind"], Value::text(k), cx)))
        }));

        // The knobs of the kind
        let knob_rows: Vec<_> = knobs(&kind, param("sync", 0.0))
            .into_iter()
            .map(|kn| {
                let now = param(kn.key, if kn.key == "speed" { 2.0 } else { 0.0 }).clamp(kn.min, kn.max);
                let key = kn.key;
                let shown = if kn.names.is_empty() {
                    format!("{now:.2}")
                } else {
                    tr_str(kn.names[(now.round() as usize).min(kn.names.len() - 1)]).to_string()
                };
                let (lo, hi, step) = (kn.min, kn.max, kn.step);
                let stepper = |sign: f64, label: &'static str| {
                    button(SharedString::from(format!("mod-{key}-{label}")), label, false, 0)
                        .debug_selector(move || format!("mod-{key}{label}"))
                        .on_click(cx.listener(move |me, _, _, cx| {
                            let v = ((now + sign * step).clamp(lo, hi) * 1000.0).round() / 1000.0;
                            me.set_param(id, key, v, cx)
                        }))
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .child(div().w(px(90.)).child(tr_str(kn.label)))
                    .child(stepper(-1.0, "−"))
                    .child(div().w(px(70.)).child(shown))
                    .child(stepper(1.0, "+"))
            })
            .collect();

        // The peak's input
        let input_row = (kind == "peak").then(|| {
            let list = inputs(port);
            let now = ref_of(&m, "input");
            let label = now
                .and_then(|n| list.iter().find(|(i, _)| *i == n).map(|(_, l)| l.clone()))
                .unwrap_or_else(|| "—".into());
            let next = match now.and_then(|n| list.iter().position(|(i, _)| *i == n)) {
                Some(i) if i + 1 < list.len() => Some(list[i + 1].0),
                Some(_) => None,
                None => list.first().map(|(i, _)| *i),
            };
            div().flex().items_center().gap(px(6.)).text_size(px(11.)).child(div().w(px(90.)).child(tr("Input"))).child(
                button("mod-input", label, now.is_some(), THEME.blue).debug_selector(|| "mod-input".into()).on_click(
                    cx.listener(move |me, _, _, cx| {
                        let v = next.map_or(Value::NULL, |n| Value::text(n.to_string()));
                        me.set_field(id, &["input"], v, cx)
                    }),
                ),
            )
        });

        // The targets
        let targets: Vec<_> = m
            .get("targets")
            .map(|ts| ts.items().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .filter_map(|(row, t)| {
                let tid = id_of(&t)?;
                let tnode = ref_of(&t, "node")?;
                let tkey = t.get("param").and_then(Value::as_text).unwrap_or("").to_string();
                let real = |k: &str, d: f64| t.get(k).and_then(Value::as_real).unwrap_or(d);
                let (amount, min, max) = (real("amount", 0.5), real("min", 0.0), real("max", 1.0));
                let name = describe(port, &MapTarget::Param { node: NodeRef::Id(tnode), key: tkey });
                let step = |field: &'static str, now: f64, by: f64, lo: f64, label: &'static str| {
                    button(SharedString::from(format!("t-{row}-{field}-{label}")), label, false, 0)
                        .debug_selector(move || format!("target-{row}-{field}{label}"))
                        .on_click(cx.listener(move |me, _, _, cx| {
                            let v = ((now + by).clamp(lo, 1.0) * 100.0).round() / 100.0;
                            me.set_field(tid, &[field], Value::real(v).unwrap_or_default(), cx)
                        }))
                };
                Some(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.))
                        .p(px(6.))
                        .rounded(px(6.))
                        .bg(c(THEME.bg3))
                        .text_size(px(11.))
                        .child(div().flex_1().child(name))
                        .child(step("amount", amount, -0.1, -1.0, "−"))
                        .child(format!("{:.0}%", amount * 100.0))
                        .child(step("amount", amount, 0.1, -1.0, "+"))
                        .child(step("min", min, -0.05, 0.0, "−"))
                        .child(format!("{:.0}", min * 100.0))
                        .child(step("min", min, 0.05, 0.0, "+"))
                        .child("…")
                        .child(step("max", max, -0.05, 0.0, "−"))
                        .child(format!("{:.0}", max * 100.0))
                        .child(step("max", max, 0.05, 0.0, "+"))
                        .child(
                            button(SharedString::from(format!("t-{row}-remove")), tr("Remove"), false, 0)
                                .debug_selector(move || format!("target-{row}-remove"))
                                .on_click(cx.listener(move |me, _, _, cx| {
                                    me.apply("Remove target", vec![EditOp::Remove { id: Ref::Id(tid) }], cx)
                                })),
                        ),
                )
            })
            .collect();

        shell(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(kind_row)
                .children(knob_rows)
                .children(input_row)
                .child(caption(tr("Targets")))
                .child(
                    div().id("modulator-targets").flex().flex_col().gap(px(6.)).overflow_y_scroll().children(targets),
                )
                .child(
                    button("mod-remove", tr("Remove modulator"), false, 0)
                        .debug_selector(|| "mod-remove".into())
                        .on_click(cx.listener(move |me, _, _, cx| me.remove(id, cx))),
                ),
        )
    }
}
