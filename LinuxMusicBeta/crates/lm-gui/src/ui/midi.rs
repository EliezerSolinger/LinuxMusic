//! MIDI mapping in the interface (I-21.2): every control's menu offers "MIDI learn", "Remove MIDI mapping" and the
//! binding window; these helpers build those items for any view, name targets and sources for people, and read the
//! mappings of both scopes. Everything goes through the port.

use gpui::{AnyElement, App, Entity, IntoElement, SharedString, WeakEntity, Window, div, prelude::*, px};
use lm_port::{CorePort, DocSource, MapScope, MapTarget, Mapping, NodeId, NodeRef, PathStep, Value};

use super::{THEME, c, tr, trf};
use crate::core::Core;
use crate::screens::Screen;
use crate::workspace::Workspace;

/// A parameter of a node as a mapping's target (`volume`, `pan`, `rack.eq.high`, a send's `amount`)
pub fn param(node: NodeId, key: &str) -> MapTarget {
    MapTarget::Param { node: NodeRef::Id(node), key: key.to_string() }
}

/// A field of a node (a switch or a choice: mute, solo, a section's model)
pub fn field(node: NodeId, path: &[&str]) -> MapTarget {
    MapTarget::Field { node: NodeRef::Id(node), path: path.iter().map(|k| PathStep::key(k)).collect() }
}

/// The mappings of a scope: the project's are in its tree, the machine's in the preferences
pub fn mappings(port: &dyn CorePort, scope: MapScope) -> Vec<Mapping> {
    let list = match scope {
        MapScope::Project => port.root().get("midiMap"),
        MapScope::Machine => port.preferences().get("midiMap"),
    };
    list.map(|l| l.items().filter_map(|m| Mapping::from_value(m).ok()).collect()).unwrap_or_default()
}

/// The mappings on a target, with their scope
pub fn mappings_of(port: &dyn CorePort, target: &MapTarget) -> Vec<(MapScope, usize, Mapping)> {
    let mut out = Vec::new();
    for scope in [MapScope::Project, MapScope::Machine] {
        for (i, m) in mappings(port, scope).into_iter().enumerate() {
            if &m.target == target {
                out.push((scope, i, m));
            }
        }
    }
    out
}

fn node_name(port: &dyn CorePort, id: NodeId) -> String {
    let n = port.find(id);
    let name = n.as_ref().and_then(|n| n.get("name").and_then(Value::as_text).map(str::to_string));
    let plugin = n.as_ref().and_then(|n| n.get("plugin").and_then(Value::as_text).map(str::to_string));
    name.filter(|s| !s.is_empty()).or(plugin).unwrap_or_else(|| id.to_string())
}

/// A target as people read it: "Bass · volume", "Bass · rack.eq.high", "Selected track +1 · mute", an action's name
pub fn describe(port: &dyn CorePort, t: &MapTarget) -> String {
    let node = |r: &NodeRef| match r {
        NodeRef::Id(id) => node_name(port, *id),
        NodeRef::Selected { role, offset } => {
            let base = trf(
                "Selected {0}",
                &[&tr(match role.name() {
                    "track" => "track",
                    "channel" => "channel",
                    _ => "plugin",
                })],
            );
            if *offset == 0 { base } else { format!("{base} +{offset}") }
        }
    };
    match t {
        MapTarget::Param { node: n, key } => format!("{} · {key}", node(n)),
        MapTarget::Field { node: n, path } => {
            let p: Vec<String> = path
                .iter()
                .map(|s| match s {
                    PathStep::Key(k) => k.to_string(),
                    PathStep::Index(i) => i.to_string(),
                    PathStep::Note(n) => n.to_string(),
                })
                .collect();
            format!("{} · {}", node(n), p.join("."))
        }
        MapTarget::Action { id, .. } => {
            crate::actions::find(id).map_or_else(|| id.to_string(), |d| tr(d.label).to_string())
        }
    }
}

/// A source as people read it: "Controlador 1 · ch 1 · CC 7"
pub fn describe_source(s: &DocSource) -> String {
    let port = if s.port.is_empty() { tr("Any port").to_string() } else { s.port.to_string() };
    let ch = s.channel.map_or_else(|| tr("any channel").to_string(), |c| format!("ch {}", c + 1));
    let msg = s.message;
    let what = match msg.kind_name() {
        "cc" => format!("CC {}", msg.number()),
        "cc14" => format!("CC {}/{} (14 bit)", msg.number(), msg.number() + 32),
        "nrpn" => format!("NRPN {}", msg.number()),
        "rpn" => format!("RPN {}", msg.number()),
        "note" => format!("{} {}", tr("Note"), msg.number()),
        "pitchBend" => tr("Pitch bend").to_string(),
        "channelPressure" => tr("Aftertouch").to_string(),
        "polyPressure" => format!("{} {}", tr("Poly aftertouch"), msg.number()),
        _ => tr("Program change").to_string(),
    };
    format!("{port} · {ch} · {what}")
}

/// Starts learning for `target` (the first control moved becomes its source)
pub fn learn(core: &Entity<Core>, target: MapTarget, scope: MapScope, cx: &mut App) {
    core.update(cx, |core, cx| {
        core.report(|pt| pt.midi_learn(target, scope));
        cx.notify();
    });
}

/// Removes every mapping on `target`, in both scopes
pub fn unmap(core: &Entity<Core>, target: &MapTarget, cx: &mut App) {
    core.update(cx, |core, cx| {
        for scope in [MapScope::Project, MapScope::Machine] {
            core.report(|pt| pt.unmap(target, scope));
        }
        cx.notify();
    });
}

/// Opens the binding window on `target`
pub fn open_binding(workspace: &WeakEntity<Workspace>, target: MapTarget, window: &mut Window, cx: &mut App) {
    let _ = workspace.update(cx, |ws, cx| {
        ws.binding = Some(target);
        ws.open(Screen::MidiBinding, None, window, cx);
    });
}

/// "Modular…": the parameter's modulator (the one it has, or a new LFO with the parameter as its target) and its
/// editor. Only a parameter of a node can be modulated
pub fn open_modulator(
    core: &Entity<Core>,
    workspace: &WeakEntity<Workspace>,
    target: MapTarget,
    window: &mut Window,
    cx: &mut App,
) {
    let MapTarget::Param { node: NodeRef::Id(node), key } = &target else { return };
    let params = Value::object([("node", Value::text(node.to_string())), ("param", Value::text(key.as_str()))]);
    core.update(cx, |core, cx| {
        core.report(|pt| pt.run_action("modulator.attach".into(), params));
        cx.notify();
    });
    let _ = workspace.update(cx, |ws, cx| {
        ws.binding = Some(target);
        ws.open(Screen::Modulator, None, window, cx);
    });
}

fn item(id: &'static str, label: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px(px(10.))
        .py(px(3.))
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|d| d.bg(c(THEME.bg4)))
        .child(label.into())
}

/// The three MIDI items of a control's menu (F-CTL-005, I-21.2). `close` runs after any of them (to close the menu)
pub fn menu_items(
    core: &Entity<Core>,
    workspace: &WeakEntity<Workspace>,
    target: MapTarget,
    close: impl Fn(&mut App) + Clone + 'static,
    cx: &App,
) -> Vec<AnyElement> {
    let mapped = !mappings_of(core.read(cx).port(), &target).is_empty();
    let mut items = vec![div().my(px(3.)).h(px(1.)).bg(c(THEME.line)).into_any_element()];
    let (c1, t1, k1) = (core.clone(), target.clone(), close.clone());
    items.push(
        item("menu-midi-learn", tr("MIDI learn"))
            .on_click(move |_, _, cx| {
                learn(&c1, t1.clone(), MapScope::Project, cx);
                k1(cx);
            })
            .into_any_element(),
    );
    if mapped {
        let (c2, t2, k2) = (core.clone(), target.clone(), close.clone());
        items.push(
            item("menu-midi-unmap", tr("Remove MIDI mapping"))
                .on_click(move |_, _, cx| {
                    unmap(&c2, &t2, cx);
                    k2(cx);
                })
                .into_any_element(),
        );
    }
    if let MapTarget::Param { node: NodeRef::Id(_), .. } = &target {
        let (c4, w4, t4, k4) = (core.clone(), workspace.clone(), target.clone(), close.clone());
        items.push(
            item("menu-modulate", tr("Modulate…"))
                .on_click(move |_, window, cx| {
                    open_modulator(&c4, &w4, t4.clone(), window, cx);
                    k4(cx);
                })
                .into_any_element(),
        );
    }
    let (w3, t3) = (workspace.clone(), target);
    items.push(
        item("menu-midi-binding", tr("MIDI binding…"))
            .on_click(move |_, window, cx| {
                open_binding(&w3, t3.clone(), window, cx);
                close(cx);
            })
            .into_any_element(),
    );
    items
}
