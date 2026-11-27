//! The modulator editor (decisoes-autonomas, 26/11, item 1): the modulator that moves one parameter, opened from the
//! control's menu ("Modulate…"; the control is the workspace's `binding`). Its kind (LFO, envelope, peak, random,
//! macro), the knobs of that kind, the peak's input, and its targets with their amount and range. Everything is an
//! edit of the `modulator` node through the port (undoable); the knobs are parameters, so they are automatable and
//! MIDI-mappable like any other.

use gpui::{
    Context, FontWeight, MouseButton, MouseDownEvent, Pixels, Point, Render, SharedString, Window, div, point,
    prelude::*, px,
};
use lm_port::{CorePort, EditOp, Label, MapTarget, NodeId, NodeRef, Ref, Value};

use super::ScreenCx;
use super::piano_roll::kit::{MenuItem, popup};
use crate::model::{id_of, ref_of};
use crate::ui::midi::describe;
use crate::ui::{THEME, button, c, caption, tr, tr_str};

/// The kinds of node (the letter of their ids) whose parameters a modulator can move: tracks, channels, plugins
const MODULATABLE: &[u8] = &[b't', b'm', b'p'];
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

/// The list the "add target" button opens: the nodes with something to move, then the parameters of one
#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Node,
    Param(NodeId),
}

pub struct Modulator {
    scx: ScreenCx,
    target: Option<MapTarget>,
    pick: Option<(Pick, Point<Pixels>)>,
    /// the file being read for the LFO's own wave (shown while it loads: a file may take long)
    loading_wave: bool,
    _read: Option<gpui::Task<()>>,
}

impl Modulator {
    pub fn new(scx: ScreenCx, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let target = scx.binding.clone();
        Modulator { scx, target, pick: None, loading_wave: false, _read: None }
    }

    /// Asks for an audio file and gives it to the core as the LFO's own wave (`plugin.load_into_field` on the
    /// modulator's `userWave`): the file is read off the main thread, and the wave is the first cycle of it
    fn choose_wave(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Open audio file").to_string().into()),
        });
        self.loading_wave = true;
        cx.notify();
        self._read = Some(cx.spawn(async move |this, cx| {
            let chosen = paths.await.ok().and_then(Result::ok).flatten().and_then(|mut v| v.pop());
            let Some(path) = chosen else {
                let _ = this.update(cx, |me, cx| {
                    me.loading_wave = false;
                    cx.notify();
                });
                return;
            };
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            let read = cx
                .background_executor()
                .spawn(async move { std::fs::read(&path).ok().filter(|b| b.len() <= 64 << 20) });
            let bytes = read.await;
            let _ = this.update(cx, |me, cx| {
                me.loading_wave = false;
                if let Some(bytes) = bytes {
                    let params = Value::object([
                        ("node", Value::text(id.to_string())),
                        ("field", Value::text("userWave")),
                        ("name", Value::text(name)),
                        ("bytes", Value::list(bytes.iter().map(|b| Value::int(i64::from(*b))))),
                    ]);
                    me.scx.core.update(cx, |core, _| {
                        core.report(|pt| pt.run_action("plugin.load_into_field".into(), params));
                    });
                }
                cx.notify();
            });
        }));
    }

    /// A parameter of a node becomes a target of the modulator (`modulator.add_target`)
    fn add_target(&mut self, modulator: NodeId, node: NodeId, key: &str, cx: &mut Context<Self>) {
        let params = Value::object([
            ("modulator", Value::text(modulator.to_string())),
            ("node", Value::text(node.to_string())),
            ("param", Value::text(key)),
        ]);
        self.pick = None;
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.run_action("modulator.add_target".into(), params));
        });
        cx.notify();
    }

    fn pick_items(&self, port: &dyn CorePort, pick: Pick) -> Vec<MenuItem> {
        match pick {
            Pick::Node => modulatable(port).into_iter().map(|(_, l)| MenuItem::new(l, false)).collect(),
            Pick::Param(n) => params_of(port, n)
                .into_iter()
                .map(|k| MenuItem::new(describe(port, &MapTarget::Param { node: NodeRef::Id(n), key: k }), false))
                .collect(),
        }
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

/// The nodes whose parameters a modulator can move: instruments and effects, tracks and channels, by name
fn modulatable(port: &dyn CorePort) -> Vec<(NodeId, String)> {
    let mut out = Vec::new();
    port.root().walk(&mut |_, v| {
        let Some(id) = id_of(v) else { return };
        if !MODULATABLE.contains(&id.kind) || params_of(port, id).is_empty() {
            return;
        }
        let text = |k: &str| v.get(k).and_then(Value::as_text).filter(|t| !t.is_empty());
        out.push((id, text("name").or(text("plugin")).map_or_else(|| id.to_string(), str::to_string)));
    });
    out
}

/// The parameters of a node a modulator can move: a plugin's automatable ones, the volume and pan of a track or a
/// channel
fn params_of(port: &dyn CorePort, node: NodeId) -> Vec<String> {
    let Some(v) = port.find(node) else { return Vec::new() };
    if let Some(plugin) = v.get("plugin").and_then(Value::as_text) {
        return port
            .plugin_schema(plugin)
            .map(|s| s.params.iter().filter(|p| p.automatable).map(|p| p.key.to_string()).collect())
            .unwrap_or_default();
    }
    if node.kind == b't' || node.kind == b'm' {
        return vec!["volume".to_string(), "pan".to_string()];
    }
    Vec::new()
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

        // The LFO's own wave: an audio file (one cycle of it), played when the wave is "User"
        let wave_row = (kind == "lfo").then(|| {
            let loaded = m.get("userWave").and_then(Value::as_resource).and_then(|r| r.name().map(str::to_string));
            let label: SharedString = if self.loading_wave {
                tr("Loading the file").to_string().into()
            } else {
                loaded.filter(|n| !n.is_empty()).unwrap_or_else(|| tr("User wave").to_string()).into()
            };
            div().flex().items_center().gap(px(6.)).text_size(px(11.)).child(div().w(px(90.)).child(tr("User"))).child(
                button("mod-wave-file", label, m.get("userWave").is_some_and(|w| !w.is_null()), THEME.blue)
                    .debug_selector(|| "mod-wave-file".into())
                    .on_click(cx.listener(move |me, _, _, cx| me.choose_wave(id, cx))),
            )
        });

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

        let screen = shell(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(kind_row)
                .children(knob_rows)
                .children(wave_row)
                .children(input_row)
                .child(caption(tr("Targets")))
                .child(
                    div().id("modulator-targets").flex().flex_col().gap(px(6.)).overflow_y_scroll().children(targets),
                )
                .child(
                    button("mod-add-target", tr("Add target"), self.pick.is_some(), THEME.blue)
                        .debug_selector(|| "mod-add-target".into())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|me, ev: &MouseDownEvent, _, cx| {
                                me.pick = if me.pick.is_some() {
                                    None
                                } else {
                                    Some((Pick::Node, ev.position + point(px(0.), px(12.))))
                                };
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    button("mod-remove", tr("Remove modulator"), false, 0)
                        .debug_selector(|| "mod-remove".into())
                        .on_click(cx.listener(move |me, _, _, cx| me.remove(id, cx))),
                ),
        );
        let entity = cx.entity();
        let pick = self.pick.map(|(pk, at)| {
            let items = self.pick_items(port, pk);
            let (e1, e2) = (entity.downgrade(), entity.downgrade());
            let (nodes, node_params): (Vec<NodeId>, Vec<String>) = match pk {
                Pick::Node => (modulatable(port).into_iter().map(|(n, _)| n).collect(), Vec::new()),
                Pick::Param(n) => (vec![n], params_of(port, n)),
            };
            popup(
                "mod-pick",
                at,
                items,
                move |i, _, cx| {
                    let _ = e1.update(cx, |me, cx| match pk {
                        Pick::Node => {
                            if let Some(n) = nodes.get(i) {
                                me.pick = me.pick.map(|(_, at)| (Pick::Param(*n), at));
                            }
                            cx.notify();
                        }
                        Pick::Param(n) => {
                            if let Some(k) = node_params.get(i) {
                                me.add_target(id, n, k, cx);
                            }
                        }
                    });
                },
                move |_, cx| {
                    let _ = e2.update(cx, |me, cx| {
                        me.pick = None;
                        cx.notify();
                    });
                },
            )
        });
        screen.children(pick)
    }
}
