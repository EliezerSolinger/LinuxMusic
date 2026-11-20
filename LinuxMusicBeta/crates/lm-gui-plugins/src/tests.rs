//! One headless test per panel (I-22, "pronto quando"): the panel opens on the fake core with the real plugin's
//! schema, every control is moved by the gestures a user makes (drag, Shift drag, wheel, double click and typing,
//! middle click, the menu, clicks on buttons and LEDs), the parameter or field is checked in the fake core's tree
//! after each, one drag is one undo step, and every control is a valid MIDI mapping target and, for a parameter, an
//! automation lane (I-21).

use std::time::Duration;

use gpui::{
    AppContext, Entity, Modifiers, MouseButton, Pixels, Point, ScrollDelta, ScrollEvent, TestAppContext,
    VisualTestContext, point, px,
};
use lm_plugin::Descriptor;
use lm_port::{CorePort, EditOp, Label, NodeId, PathStep, Ref, Value};
use lm_port_fake::{Config, FakePort, RealPlugin};

use crate::panel::PluginPanel;
use crate::spec::{Bind, Kind, Target};
use crate::widgets::Range;
use crate::{PortHost, panels};

struct Host {
    port: FakePort,
}

impl PortHost for Host {
    fn port(&self) -> &dyn CorePort {
        &self.port
    }
    fn port_mut(&mut self) -> &mut dyn CorePort {
        &mut self.port
    }
}

/// The fake core with the real plugins, and the node of the first `plugin` (an effect the demo lacks is inserted)
fn host(plugins: &[(&Descriptor, bool)], plugin: &str) -> (FakePort, NodeId) {
    let real =
        plugins.iter().map(|(d, instrument)| RealPlugin { schema: d.schema(), instrument: *instrument }).collect();
    let mut port = FakePort::with_plugins(Config { tracks: 6, slow_delay: None, slow_track: usize::MAX }, real)
        .expect("the demo with the real plugins");
    let node = find_plugin(port.root(), plugin).unwrap_or_else(|| insert_effect(&mut port, plugin));
    (port, node)
}

fn find_plugin(root: &Value, plugin: &str) -> Option<NodeId> {
    let mut found = None;
    root.walk(&mut |_, v| {
        if found.is_none() && v.get("plugin").and_then(Value::as_text) == Some(plugin) {
            found = v.get("id").and_then(Value::as_text).and_then(|s| NodeId::parse(s).ok());
        }
    });
    found
}

fn insert_effect(port: &mut FakePort, plugin: &str) -> NodeId {
    let channel = port
        .root()
        .get("mixer")
        .and_then(|m| m.get("channels"))
        .and_then(|c| c.at(0))
        .and_then(|c| c.get("id"))
        .and_then(Value::as_text)
        .and_then(|s| NodeId::parse(s).ok())
        .expect("the demo has channels");
    let node = Value::object([("plugin", Value::text(plugin)), ("state", Value::empty_object())]);
    let op = EditOp::Insert { temp: 1, parent: Ref::Id(channel), list: "inserts".into(), index: 0, node };
    let applied = port.apply(Label::new("test"), vec![op]).ok().expect("the effect is inserted");
    applied.created[0].1
}

struct Opened<'a> {
    host: Entity<Host>,
    panel: Entity<PluginPanel<Host>>,
    cx: &'a mut VisualTestContext,
    node: NodeId,
}

fn open<'a>(cx: &'a mut TestAppContext, plugins: &[(&Descriptor, bool)], plugin: &str) -> Opened<'a> {
    let (port, node) = host(plugins, plugin);
    let host = cx.new(|_| Host { port });
    let spec = panels::spec(plugin).expect("the plugin has a panel");
    let h = host.clone();
    let (panel, vcx) = cx.add_window_view(move |_, cx| PluginPanel::new(h, node, spec, cx));
    vcx.run_until_parked();
    Opened { host, panel, cx: vcx, node }
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

impl Opened<'_> {
    fn state(&mut self) -> Value {
        let node = self.node;
        self.host.read_with(self.cx, |h, _| h.port.find(node).and_then(|n| n.get("state").cloned()).unwrap_or_default())
    }

    fn param(&mut self, key: &str) -> f64 {
        let key = key.to_owned();
        self.panel.read_with(self.cx, |p, cx| p.param(&key, cx))
    }

    /// The value in the tree itself (not the panel's reading)
    fn stored_param(&mut self, key: &str) -> Option<f64> {
        self.state().get("params").and_then(|p| p.get(key)).and_then(Value::as_real)
    }

    fn field(&mut self, key: &str) -> Value {
        let key = key.to_owned();
        self.panel.read_with(self.cx, |p, cx| p.field(&key, cx))
    }

    fn center(&mut self, id: &'static str) -> Point<Pixels> {
        self.cx.debug_bounds(id).unwrap_or_else(|| panic!("{id} is drawn")).center()
    }

    fn settle(&mut self) {
        self.cx.executor().advance_clock(Duration::from_millis(20));
        self.cx.run_until_parked();
    }

    fn drag(&mut self, id: &'static str, dy: f32, m: Modifiers) {
        let p = self.center(id);
        self.cx.simulate_mouse_move(p, None, Modifiers::none());
        // Shift at the press would carry the value (F-CTL-006): it is pressed during the drag
        self.cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
        for k in 1..=10 {
            let f = k as f32 / 10.0;
            self.cx.simulate_mouse_move(point(p.x, p.y + px(dy * f)), MouseButton::Left, m);
        }
        self.cx.simulate_mouse_up(point(p.x, p.y + px(dy)), MouseButton::Left, m);
        self.settle();
    }

    fn click(&mut self, id: &'static str, button: MouseButton) {
        let p = self.center(id);
        self.cx.simulate_mouse_move(p, None, Modifiers::none());
        self.cx.simulate_mouse_down(p, button, Modifiers::none());
        self.cx.simulate_mouse_up(p, button, Modifiers::none());
        self.settle();
    }

    fn wheel(&mut self, id: &'static str, up: bool, m: Modifiers) {
        let p = self.center(id);
        self.cx.simulate_mouse_move(p, None, Modifiers::none());
        self.cx.simulate_event(ScrollEvent {
            position: p,
            delta: ScrollDelta::Lines(point(0.0, if up { 1.0 } else { -1.0 })),
            modifiers: m,
            ..Default::default()
        });
        self.settle();
    }

    fn double_click(&mut self, id: &'static str) {
        let p = self.center(id);
        self.cx.simulate_mouse_move(p, None, Modifiers::none());
        self.cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
        self.cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
        self.cx.simulate_event(gpui::MouseDownEvent {
            button: MouseButton::Left,
            position: p,
            modifiers: Modifiers::none(),
            click_count: 2,
            first_mouse: false,
        });
        self.cx.simulate_event(gpui::MouseUpEvent {
            button: MouseButton::Left,
            position: p,
            modifiers: Modifiers::none(),
            click_count: 2,
        });
        self.settle();
    }

    fn undo(&mut self) {
        self.host.update(self.cx, |h, _| {
            let _ = h.port.undo();
        });
        self.settle();
    }
}

/// Moves every control of a panel with every gesture and checks the tree after each
fn exercise(o: &mut Opened<'_>) {
    let spec = o.panel.read_with(o.cx, |p, _| p.spec());
    let schema = o.host.read_with(o.cx, |h, _| h.port.plugin_schema(spec.plugin)).expect("the schema");
    let b = o.cx.debug_bounds(leak(format!("panel-{}", spec.plugin))).expect("the panel is drawn");
    assert_eq!((b.size.width, b.size.height), (px(spec.size.0), px(spec.size.1)), "the legacy size");

    for (i, c) in spec.controls.iter().enumerate() {
        let id = c.id;
        if let Kind::Button(b) = &c.kind
            && let Bind::Press(_) = b.bind
        {
            // a push button changes several things in one step, with no model of its own
            let before = o.state();
            o.click(id, MouseButton::Left);
            assert_ne!(o.state(), before, "{id}: the press changes the state");
            o.undo();
            assert_eq!(o.state(), before, "{id}: in one undo step");
            continue;
        }
        // the control is a mapping target and the schema has its key (I-21: every control is mappable)
        match c.target() {
            Target::Param(k) => {
                let p = schema.find_param(k).unwrap_or_else(|| panic!("{id}: {k} is a parameter of the schema"));
                assert!(p.automatable, "{id}: {k} is automatable");
            }
            Target::Field(k) => assert!(schema.find_field(k).is_some(), "{id}: {k} is a field of the schema"),
        }
        if let Kind::Graph(_) = &c.kind {
            // a graph is drawn, not set to a value: no menu in the legacy, nothing a controller could move
        } else {
            learn_and_forget(o, i, id);
        }

        match &c.kind {
            Kind::Knob(k) if k.choices.is_some() => {
                // a knob over a choice field: dragging up walks the keys, the wheel steps one
                let keys = k.choices.unwrap();
                let key = k.param;
                o.click(id, MouseButton::Middle);
                let def = o.field(key);
                o.drag(id, -400.0, Modifiers::none());
                assert_eq!(o.field(key).as_text(), keys.last().copied(), "{id}: dragged to the last key");
                o.wheel(id, false, Modifiers::none());
                assert_eq!(o.field(key).as_text(), Some(keys[keys.len() - 2]), "{id}: wheel");
                o.click(id, MouseButton::Middle);
                assert_eq!(o.field(key), def, "{id}: middle click resets");
            }
            Kind::Knob(k) => {
                let key = k.param;
                let p = schema.find_param(key).unwrap().clone();
                let r = Range { min: p.min, max: p.max, step: k.step, center: k.center.unwrap_or(p.min), log: k.log };
                // from the middle, so a drag both ways moves it
                set_param(o, key, r.fit(r.min + r.span() / 2.0));
                let before = o.param(key);
                // drag up 100 px: 40 % of the range (0.4 % a pixel), one undo step
                let mut left = 0.0;
                let mut expect = before;
                for _ in 0..10 {
                    expect = r.drag(expect, -10.0, false, &mut left);
                }
                assert_ne!(expect, before, "{id}: the drag moves it");
                o.drag(id, -100.0, Modifiers::none());
                assert_eq!(o.stored_param(key), Some(expect), "{id}: drag up");
                let written =
                    o.host.read_with(o.cx, |h, _| h.port.last_param_write().map(|(n, k, v)| (n, k.to_owned(), v)));
                assert_eq!(written, Some((o.node, key.to_owned(), expect as f32)), "{id}: the live write");
                o.undo();
                assert_eq!(o.param(key), before, "{id}: one undo step per drag");
                // Shift: four times slower
                let mut left = 0.0;
                let mut slow = before;
                for _ in 0..10 {
                    slow = r.drag(slow, 4.0, true, &mut left);
                }
                o.drag(id, 40.0, Modifiers::shift());
                assert_eq!(o.param(key), slow, "{id}: Shift drag");
                // the wheel: one notch of 100 the sweep, Shift 10
                let v = o.param(key);
                o.wheel(id, true, Modifiers::none());
                assert_eq!(o.param(key), r.wheel(v, true, 100.0), "{id}: wheel");
                let v = o.param(key);
                o.wheel(id, false, Modifiers::shift());
                assert_eq!(o.param(key), r.wheel(v, false, 10.0), "{id}: Shift wheel");
                // the middle button: the default
                o.click(id, MouseButton::Middle);
                assert_eq!(o.param(key), p.def, "{id}: middle click resets");
                // double click and type a value
                o.double_click(id);
                assert!(o.cx.debug_bounds("value-entry").is_some(), "{id}: the value entry opens");
                let quarter = r.fit(r.min + r.span() * 0.25);
                let typed = if k.volume { "-6".to_string() } else { crate::widgets::qt_number(quarter) };
                for _ in 0..12 {
                    o.cx.simulate_keystrokes("backspace");
                }
                for ch in typed.chars() {
                    o.cx.simulate_keystrokes(&ch.to_string());
                }
                o.cx.simulate_keystrokes("enter");
                o.settle();
                let want = if k.volume { r.fit(10f64.powf(-6.0 / 20.0) * k.zero_dbfs) } else { quarter };
                assert!((o.param(key) - want).abs() < 1e-9, "{id}: typed {typed}: {} vs {want}", o.param(key));
                // the menu: reset
                o.click(id, MouseButton::Right);
                assert!(o.cx.debug_bounds("control-menu").is_some(), "{id}: the menu opens");
                o.click("menu-reset", MouseButton::Left);
                assert_eq!(o.param(key), p.def, "{id}: menu reset");
                // the menu: add the automation lane, then it offers to remove it
                o.click(id, MouseButton::Right);
                o.click("menu-add-lane", MouseButton::Left);
                o.click(id, MouseButton::Right);
                assert!(o.cx.debug_bounds("menu-remove-lane").is_some(), "{id}: the lane exists");
                o.click("menu-remove-lane", MouseButton::Left);
            }
            Kind::Button(b) => check_bind(o, id, b.bind),
            Kind::Led(l) => check_bind(o, id, l.bind),
            Kind::Graph(g) => {
                // draw a line across the graph: one gesture, one undo step
                let before = o.field(g.field);
                let b = o.cx.debug_bounds(id).expect("the graph is drawn");
                let (x0, y0) = (b.origin.x + px(4.0), b.origin.y + px(4.0));
                let (x1, y1) = (b.origin.x + b.size.width - px(4.0), b.origin.y + b.size.height - px(4.0));
                o.cx.simulate_mouse_move(point(x0, y0), None, Modifiers::none());
                o.cx.simulate_mouse_down(point(x0, y0), MouseButton::Left, Modifiers::none());
                for k in 1..=10 {
                    let f = k as f32 / 10.0;
                    o.cx.simulate_mouse_move(
                        point(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f),
                        MouseButton::Left,
                        Modifiers::none(),
                    );
                }
                o.cx.simulate_mouse_up(point(x1, y1), MouseButton::Left, Modifiers::none());
                o.settle();
                let after = o.field(g.field);
                assert_ne!(after, before, "{id}: drawing changes the samples");
                let first = after.at(0).and_then(Value::as_real).unwrap();
                let last = after.at(g.length - 1).and_then(Value::as_real).unwrap();
                assert!(first > last, "{id}: the line goes down, {first} to {last}");
                o.undo();
                assert_eq!(o.field(g.field), before, "{id}: one undo step");
            }
            Kind::Combo(cb) => {
                // the list opens under the box; a click on the last choice selects it
                o.click(id, MouseButton::Left);
                assert!(o.cx.debug_bounds("combo-list").is_some(), "{id}: the list opens");
                let last = cb.items.last().unwrap().key;
                o.click(leak(format!("choice-{last}")), MouseButton::Left);
                assert_eq!(o.field(cb.field).as_text(), Some(last), "{id}: selects {last}");
                // the wheel up goes to the previous choice
                o.wheel(id, true, Modifiers::none());
                let prev = cb.items[cb.items.len() - 2].key;
                assert_eq!(o.field(cb.field).as_text(), Some(prev), "{id}: wheel");
                o.click(id, MouseButton::Middle);
                let def = schema.find_field(cb.field).and_then(|f| f.default.clone());
                assert_eq!(Some(o.field(cb.field)), def, "{id}: middle click resets");
            }
            Kind::Lcd(l) => {
                let key = l.param;
                let before = o.param(key);
                o.wheel(id, true, Modifiers::none());
                assert!(o.param(key) > before || before == schema.find_param(key).unwrap().max, "{id}: wheel");
            }
        }
    }
}

/// MIDI learn from the control's menu, a controller moves: the project has the mapping to this control; then the
/// menu forgets it
fn learn_and_forget(o: &mut Opened<'_>, i: usize, id: &'static str) {
    o.click(id, MouseButton::Right);
    o.click("menu-midi-learn", MouseButton::Left);
    let target = o.panel.read_with(o.cx, |p, _| p.map_target(i));
    o.host.update(o.cx, |h, _| h.port.midi_in("", 0, lm_port::MidiMessage::Cc { number: (i % 120) as u8 }, 64));
    o.settle();
    let learned = o.host.read_with(o.cx, |h, _| {
        h.port.mappings(lm_port::MapScope::Project).into_iter().filter(|m| m.target == target).count()
    });
    assert_eq!(learned, 1, "{id}: MIDI learn maps it");
    o.click(id, MouseButton::Right);
    o.click("menu-midi-forget", MouseButton::Left);
    let left = o.host.read_with(o.cx, |h, _| {
        h.port.mappings(lm_port::MapScope::Project).into_iter().filter(|m| m.target == target).count()
    });
    assert_eq!(left, 0, "{id}: the mapping is removed");
}

fn set_param(o: &mut Opened<'_>, key: &str, v: f64) {
    let node = o.node;
    let op = EditOp::Set {
        id: Ref::Id(node),
        path: vec![PathStep::key("state"), PathStep::key("params"), PathStep::key(key)],
        value: Value::real(v).unwrap(),
    };
    o.host.update(o.cx, |h, _| {
        let _ = h.port.apply(Label::new("test"), vec![op]);
    });
    o.settle();
}

fn check_bind(o: &mut Opened<'_>, id: &'static str, bind: Bind) {
    match bind {
        Bind::Choice { field, key } => {
            o.click(id, MouseButton::Left);
            assert_eq!(o.field(field).as_text(), Some(key), "{id}: selects {key}");
        }
        Bind::Switch { param } => {
            let before = o.param(param);
            o.click(id, MouseButton::Left);
            assert_eq!(o.param(param), 1.0 - before, "{id}: toggles");
            o.click(id, MouseButton::Left);
            assert_eq!(o.param(param), before, "{id}: toggles back");
        }
        Bind::Flag { field } => {
            let before = o.field(field).as_bool() == Some(true);
            o.click(id, MouseButton::Left);
            assert_eq!(o.field(field).as_bool(), Some(!before), "{id}: toggles");
        }
        Bind::Press(_) => {}
    }
}

#[gpui::test]
fn tripleoscillator(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_tripleoscillator::DESCRIPTOR, true)], "tripleoscillator");
    exercise(&mut o);
}

#[gpui::test]
fn kicker(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_instruments::kicker::DESCRIPTOR, true)], "kicker");
    exercise(&mut o);
}

fn effect(cx: &mut TestAppContext, d: &'static Descriptor) {
    let mut o = open(cx, &[(d, false)], d.id);
    exercise(&mut o);
}

#[gpui::test]
fn lb302(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_instruments::lb302::DESCRIPTOR, true)], "lb302");
    exercise(&mut o);
}

#[gpui::test]
fn sid(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_sid::DESCRIPTOR, true)], "sid");
    exercise(&mut o);
}

#[gpui::test]
fn organic(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_instruments::organic::DESCRIPTOR, true)], "organic");
    exercise(&mut o);
}

#[gpui::test]
fn sfxr(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_instruments::sfxr::DESCRIPTOR, true)], "sfxr");
    exercise(&mut o);
}

#[gpui::test]
fn nes(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_instruments::nes::DESCRIPTOR, true)], "nes");
    exercise(&mut o);
}

#[gpui::test]
fn freeboy(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_freeboy::DESCRIPTOR, true)], "freeboy");
    exercise(&mut o);
}

#[gpui::test]
fn opulenz(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_opulenz::DESCRIPTOR, true)], "opulenz");
    exercise(&mut o);
}

#[gpui::test]
fn bitcrush(cx: &mut TestAppContext) {
    effect(cx, &lm_plugins_effects::bitcrush::DESCRIPTOR);
}

#[gpui::test]
fn bassbooster(cx: &mut TestAppContext) {
    effect(cx, &lm_plugins_effects::bass_booster::DESCRIPTOR);
}

#[gpui::test]
fn reverbsc(cx: &mut TestAppContext) {
    effect(cx, &lm_plugins_effects::reverb_sc::DESCRIPTOR);
}

#[gpui::test]
fn flanger(cx: &mut TestAppContext) {
    effect(cx, &lm_plugins_effects::flanger::DESCRIPTOR);
}

#[gpui::test]
fn amplifier(cx: &mut TestAppContext) {
    effect(cx, &lm_plugins_effects::amplifier::DESCRIPTOR);
}

#[gpui::test]
fn stereomatrix(cx: &mut TestAppContext) {
    effect(cx, &lm_plugins_effects::stereo_matrix::DESCRIPTOR);
}

#[gpui::test]
fn dualfilter(cx: &mut TestAppContext) {
    effect(cx, &lm_plugins_effects::dual_filter::DESCRIPTOR);
}

#[gpui::test]
fn the_value_float_shows_after_700_ms(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_tripleoscillator::DESCRIPTOR, true)], "tripleoscillator");
    let p = o.center("vol1");
    o.cx.simulate_mouse_move(p, None, Modifiers::none());
    o.cx.executor().advance_clock(Duration::from_millis(300));
    o.cx.run_until_parked();
    assert!(o.cx.debug_bounds("value-float").is_none(), "not before 700 ms");
    o.cx.executor().advance_clock(Duration::from_millis(500));
    o.cx.run_until_parked();
    assert!(o.cx.debug_bounds("value-float").is_some(), "after 700 ms");
    o.cx.executor().advance_clock(Duration::from_millis(2100));
    o.cx.run_until_parked();
    assert!(o.cx.debug_bounds("value-float").is_none(), "gone after 2 s");
}

#[gpui::test]
fn shift_drag_carries_a_value_to_another_knob(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_tripleoscillator::DESCRIPTOR, true)], "tripleoscillator");
    set_param(&mut o, "fineL1", 37.0);
    let from = o.center("fineL1");
    let to = o.center("fineR1");
    o.cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::shift());
    o.cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::shift());
    o.cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::shift());
    o.settle();
    assert_eq!(o.param("fineR1"), 37.0);
    assert_eq!(o.param("fineL1"), 37.0);
}

#[gpui::test]
fn midi_binding_asks_the_host(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_tripleoscillator::DESCRIPTOR, true)], "tripleoscillator");
    let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = got.clone();
    let panel = o.panel.clone();
    let _sub =
        o.cx.update(|_, cx| cx.subscribe(&panel, move |_, e: &crate::PanelEvent, _| sink.borrow_mut().push(e.clone())));
    o.click("pan2", MouseButton::Right);
    o.click("menu-midi-binding", MouseButton::Left);
    let node = o.node;
    assert_eq!(
        *got.borrow(),
        vec![crate::PanelEvent::OpenBinding(lm_port::MapTarget::Param {
            node: lm_port::NodeRef::Id(node),
            key: "pan2".into()
        })]
    );
}

#[gpui::test]
fn copy_and_paste_a_value(cx: &mut TestAppContext) {
    let mut o = open(cx, &[(&lm_plugins_tripleoscillator::DESCRIPTOR, true)], "tripleoscillator");
    set_param(&mut o, "phase1", 90.0);
    o.click("phase1", MouseButton::Right);
    o.click("menu-copy", MouseButton::Left);
    o.click("phase2", MouseButton::Right);
    o.click("menu-paste", MouseButton::Left);
    assert_eq!(o.param("phase2"), 90.0);
}

#[gpui::test]
fn the_panel_follows_the_tree(cx: &mut TestAppContext) {
    // an undo, another client or the automation changes the tree: the panel reads it, it keeps no copy
    let mut o = open(cx, &[(&lm_plugins_tripleoscillator::DESCRIPTOR, true)], "tripleoscillator");
    set_param(&mut o, "coarse3", 7.0);
    assert_eq!(o.param("coarse3"), 7.0);
    o.undo();
    assert_eq!(o.param("coarse3"), -24.0);
}
