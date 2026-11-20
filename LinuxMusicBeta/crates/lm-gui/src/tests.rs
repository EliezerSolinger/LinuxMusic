//! Headless tests of every gesture and every shortcut: GPUI's test harness drives the real views on the fake core
//! (`lm-port-fake`, a real lm-doc document behind a real `CorePort`), with no window server, a fake clock and
//! simulated input. Shortcuts are typed as keystrokes through the bindings built from the registry, so a key that is
//! not bound fails here.

use std::time::Duration;

use gpui::{
    Entity, Modifiers, MouseButton, MouseDownEvent, Pixels, Point, TestAppContext, VisualTestContext, point, px,
};
use lm_port::{CorePort, NodeId, Value};
use lm_port_fake::{Config, FakePort};

use crate::actions::{REGISTRY, keys_for_platform};
use crate::core::Core;
use crate::model::{self, ChannelKind};
use crate::pages::Page;
use crate::pages::mixing::{MASTER_KNOBS, STRIP_KNOBS, Scale};
use crate::screens::Screen;
use crate::view;
use crate::workspace::{self, Pending, Workspace};

fn port() -> Box<dyn CorePort> {
    Box::new(FakePort::new(Config { tracks: 12, slow_delay: None, slow_track: usize::MAX }).expect("the demo project"))
}

fn open(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let p = port();
    let prefs = p.preferences().clone();
    cx.update(|cx| workspace::start(cx, &prefs));
    let mut p = Some(p);
    let (ws, vcx) = cx.add_window_view(move |window, cx| Workspace::new(p.take().unwrap(), window, cx));
    vcx.run_until_parked();
    (ws, vcx)
}

fn core(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<Core> {
    ws.read_with(cx, |ws, _| ws.core.clone())
}

fn root(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Value {
    let core = core(ws, cx);
    core.read_with(cx, |core, _| core.port().root().clone())
}

/// Lets the pump run (20 ms of GPUI's fake clock) and the views redraw
fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(20));
    cx.run_until_parked();
}

fn keys(cx: &mut VisualTestContext, keys: &str) {
    cx.simulate_keystrokes(&keys_for_platform(keys));
    cx.run_until_parked();
}

fn bounds(cx: &mut VisualTestContext, selector: &str) -> gpui::Bounds<Pixels> {
    let s: &'static str = Box::leak(selector.to_string().into_boxed_str());
    cx.debug_bounds(s).unwrap_or_else(|| panic!("{selector} is drawn"))
}

fn center(cx: &mut VisualTestContext, selector: &str) -> Point<Pixels> {
    bounds(cx, selector).center()
}

fn drag(cx: &mut VisualTestContext, selector: &str, dx: f32, dy: f32) {
    let p = center(cx, selector);
    drag_at(cx, p, dx, dy, Modifiers::none());
}

fn drag_at(cx: &mut VisualTestContext, p: Point<Pixels>, dx: f32, dy: f32, m: Modifiers) {
    cx.simulate_mouse_down(p, MouseButton::Left, m);
    for k in 1..=10 {
        let f = k as f32 / 10.0;
        cx.simulate_mouse_move(point(p.x + px(dx * f), p.y + px(dy * f)), MouseButton::Left, m);
    }
    cx.simulate_mouse_up(point(p.x + px(dx), p.y + px(dy)), MouseButton::Left, m);
    cx.run_until_parked();
}

fn click(cx: &mut VisualTestContext, selector: &str) {
    let p = center(cx, selector);
    cx.simulate_click(p, Modifiers::none());
    cx.run_until_parked();
}

fn button_at(cx: &mut VisualTestContext, p: Point<Pixels>, button: MouseButton, clicks: usize) {
    cx.simulate_event(MouseDownEvent {
        position: p,
        modifiers: Modifiers::none(),
        button,
        click_count: clicks,
        first_mouse: false,
    });
    cx.simulate_mouse_up(p, button, Modifiers::none());
    cx.run_until_parked();
}

fn undo(cx: &mut VisualTestContext) {
    keys(cx, "secondary-z");
}

fn track_volume(ws: &Entity<Workspace>, cx: &mut VisualTestContext, index: usize) -> f64 {
    model::tracks(&root(ws, cx))[index].volume
}

fn channel(ws: &Entity<Workspace>, cx: &mut VisualTestContext, id: NodeId) -> model::ChannelView {
    model::channels(&root(ws, cx)).into_iter().find(|c| c.id == id).expect("the channel")
}

fn show(ws: &Entity<Workspace>, cx: &mut VisualTestContext, page: Page) {
    keys(cx, if page == Page::Mixing { "secondary-2" } else { "secondary-1" });
    assert_eq!(ws.read_with(cx, |ws, _| ws.page), page);
}

/// A normal channel with a track behind it (the drum group, its pieces and the returns come first)
fn some_channel(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> NodeId {
    model::channels(&root(ws, cx)).iter().filter(|c| c.kind == ChannelKind::Normal).nth(5).expect("a channel").id
}

/// A click on a strip's name label: selects it and focuses the page
fn click_strip(cx: &mut VisualTestContext, id: NodeId, m: Modifiers) {
    let p = center(cx, &format!("name-{}", id.number));
    cx.simulate_click(p, m);
    cx.run_until_parked();
}

fn select_strip(ws: &Entity<Workspace>, cx: &mut VisualTestContext, id: NodeId) {
    let mixing = ws.read_with(cx, |ws, _| ws.mixing.clone());
    mixing.update(cx, |m, cx| m.select_channel(id, cx));
    cx.run_until_parked();
    click_strip(cx, id, Modifiers::none());
    assert_eq!(mixing.read_with(cx, |m, _| m.selected), Some(id));
}

// --- the registry and the keys ----------------------------------------------------------------------------------

/// Every entry of the registry names a real action, and its keys parse on this platform
#[gpui::test]
fn every_registry_entry_binds(cx: &mut TestAppContext) {
    let (_ws, cx) = open(cx);
    let bad = cx.update(|_, cx| crate::actions::bind_keys(cx, &Value::NULL));
    assert!(bad.is_empty(), "not bound: {bad:?}");
    assert!(REGISTRY.len() >= 30);
}

/// A key the user changed in the preferences replaces the default
#[gpui::test]
fn the_users_shortcut_works(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let prefs = Value::object([("shortcuts", Value::object([("view.mixing", Value::text("f2"))]))]);
    cx.update(|_, cx| {
        cx.clear_key_bindings();
        crate::actions::bind_keys(cx, &prefs)
    });
    keys(cx, "f2");
    assert_eq!(ws.read_with(cx, |ws, _| ws.page), Page::Mixing);
}

/// Cmd/Ctrl+1 and +2, Ctrl+Tab: the pages
#[gpui::test]
fn page_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    show(&ws, cx, Page::Composition);
    keys(cx, "ctrl-tab");
    assert_eq!(ws.read_with(cx, |ws, _| ws.page), Page::Mixing);
}

/// Space, Shift+Space, Home, L, R: the transport
#[gpui::test]
fn transport_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let core = core(&ws, cx);
    let playing = |cx: &mut VisualTestContext| core.read_with(cx, |c, _| c.port().telemetry().position.playing);
    keys(cx, "space");
    settle(cx);
    assert!(playing(cx), "space plays");
    keys(cx, "space");
    settle(cx);
    assert!(!playing(cx), "space stops");
    keys(cx, "space");
    settle(cx);
    keys(cx, "shift-space");
    settle(cx);
    assert!(!playing(cx), "shift+space stops");
    keys(cx, "home");
    settle(cx);
    assert_eq!(core.read_with(cx, |c, _| c.port().telemetry().position.samples), 0, "home rewinds");
    let before = model::song(&root(&ws, cx)).loop_on;
    keys(cx, "l");
    assert_ne!(model::song(&root(&ws, cx)).loop_on, before, "L toggles the loop");
    keys(cx, "r");
    settle(cx);
    let t = core.read_with(cx, |c, _| {
        (
            c.port().telemetry().position.recording,
            c.port().telemetry().counting_in,
            c.port().telemetry().position.playing,
        )
    });
    assert!(t.0 || t.1 || t.2, "R records: {t:?}");
    keys(cx, "shift-space");
    settle(cx);
}

/// Cmd/Ctrl+Z and Cmd/Ctrl+Shift+Z (and Ctrl+Y): undo and redo
#[gpui::test]
fn undo_and_redo_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let before = track_volume(&ws, cx, 1);
    drag(cx, "tknob-1", 0.0, -50.0);
    let after = track_volume(&ws, cx, 1);
    assert!(after > before);
    undo(cx);
    assert_eq!(track_volume(&ws, cx, 1), before);
    keys(cx, "secondary-shift-z");
    assert_eq!(track_volume(&ws, cx, 1), after);
    undo(cx);
    keys(cx, "ctrl-y");
    assert_eq!(track_volume(&ws, cx, 1), after);
}

/// Cmd/Ctrl+K, Cmd/Ctrl+, and Escape: the palette, the settings, closing
#[gpui::test]
fn palette_and_settings_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    keys(cx, "secondary-k");
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::Palette)));
    keys(cx, "escape");
    assert!(!ws.read_with(cx, |ws, _| ws.is_open(Screen::Palette)));
    keys(cx, "secondary-,");
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::Settings)));
}

/// Cmd/Ctrl+S saves: the project is no longer modified
#[gpui::test]
fn save_key(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    drag(cx, "tknob-1", 0.0, -30.0);
    settle(cx);
    let core = core(&ws, cx);
    assert!(core.read_with(cx, |c, _| c.port().modified()));
    keys(cx, "secondary-s");
    settle(cx);
    assert!(!core.read_with(cx, |c, _| c.port().modified()));
}

/// Quit with changes asks first (Save, Don't save, Cancel); Escape is Cancel and nothing is lost
#[gpui::test]
fn quitting_with_changes_asks(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    drag(cx, "tknob-1", 0.0, -30.0);
    settle(cx);
    keys(cx, "secondary-q");
    assert_eq!(ws.read_with(cx, |ws, _| ws.confirm), Some(Pending::Quit));
    keys(cx, "escape");
    assert_eq!(ws.read_with(cx, |ws, _| ws.confirm), None);
    // the window's close button asks the same way
    ws.update_in(cx, |ws, window, cx| ws.request(Pending::Close, window, cx));
    assert_eq!(ws.read_with(cx, |ws, _| ws.confirm), Some(Pending::Close));
}

// --- the core's pump --------------------------------------------------------------------------------------------

/// H2: the pump runs every 10 ms on GPUI's timer whether or not anything is drawn
#[gpui::test]
fn pump_runs_without_frames(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let core = core(&ws, cx);
    let before = core.read_with(cx, |c, _| c.pumps);
    cx.executor().advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();
    let n = core.read_with(cx, |c, _| c.pumps) - before;
    assert!((95..=105).contains(&n), "{n} pumps in 1 s");
}

/// Every screen opens and draws, and closes, without panicking (B, C and D fill them in)
#[gpui::test]
fn every_screen_opens(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    for s in Screen::ALL {
        ws.update_in(cx, |ws, window, cx| ws.open(s, None, window, cx));
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| ws.is_open(s)), "{s:?}");
        ws.update(cx, |ws, cx| ws.close(s, cx));
    }
    show(&ws, cx, Page::Mixing);
}

// --- composition ------------------------------------------------------------------------------------------------

/// A drag on a header's volume knob: one gesture, one undo step; a double click goes back to 0 dB
#[gpui::test]
fn track_knob_drag_and_double_click(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    drag(cx, "tknob-1", 0.0, -50.0);
    let v = track_volume(&ws, cx, 1);
    assert!(view::gain_to_db(v) > 3.0, "50 px up: {} dB", view::gain_to_db(v));
    undo(cx);
    assert_eq!(track_volume(&ws, cx, 1), 1.0, "the whole drag is one undo step");
    drag(cx, "tknob-1", 0.0, 40.0);
    let p = center(cx, "tknob-1");
    button_at(cx, p, MouseButton::Left, 2);
    assert_eq!(track_volume(&ws, cx, 1), 1.0, "double click: 0 dB");
}

/// The first item of track `t`, selected by a click on it (which also focuses the arrangement)
fn pick_item(ws: &Entity<Workspace>, cx: &mut VisualTestContext, t: usize) -> (NodeId, Point<Pixels>) {
    let item = model::tracks(&root(ws, cx))[t].items[0].id;
    let comp = ws.read_with(cx, |ws, _| ws.composition.clone());
    let p = comp.read_with(cx, |c, _| c.item_point(item)).expect("the item is drawn");
    cx.simulate_click(p, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(comp.read_with(cx, |c, _| c.selected_item), Some(item));
    (item, p)
}

fn items(ws: &Entity<Workspace>, cx: &mut VisualTestContext, t: usize) -> Vec<model::ItemView> {
    model::tracks(&root(ws, cx))[t].items.clone()
}

/// Dragging an item moves it, snapped to the beat, in one undo step
#[gpui::test]
fn item_drag_moves_it(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (item, p) = pick_item(&ws, cx, 2);
    let start = items(&ws, cx, 2)[0].start;
    drag_at(cx, p, 120.0, 0.0, Modifiers::none());
    let moved = items(&ws, cx, 2).into_iter().find(|i| i.id == item).unwrap().start;
    assert!(moved > start, "{start} -> {moved}");
    assert_eq!(moved % 960, 0, "snapped to the beat");
    undo(cx);
    assert_eq!(items(&ws, cx, 2)[0].start, start);
}

/// S splits the selected item at the cursor; Cmd/Ctrl+D duplicates; each is one undo step
#[gpui::test]
fn split_and_duplicate_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (item, _) = pick_item(&ws, cx, 2);
    let n = items(&ws, cx, 2).len();
    let it = items(&ws, cx, 2)[0].clone();
    let core = core(&ws, cx);
    core.update(cx, |c, _| {
        c.port_mut().run_action("transport.locate".into(), Value::int(it.start + it.length / 2)).ok()
    });
    settle(cx);
    keys(cx, "s");
    assert_eq!(items(&ws, cx, 2).len(), n + 1, "S splits");
    undo(cx);
    assert_eq!(items(&ws, cx, 2).len(), n);
    let comp = ws.read_with(cx, |ws, _| ws.composition.clone());
    comp.update(cx, |c, _| c.selected_item = Some(item));
    keys(cx, "secondary-d");
    assert_eq!(items(&ws, cx, 2).len(), n + 1, "Cmd/Ctrl+D duplicates");
    undo(cx);
    assert_eq!(items(&ws, cx, 2).len(), n);
}

/// Delete and Backspace remove the selected item; Shift+E makes an empty item a bar long on its track
#[gpui::test]
fn delete_and_empty_item_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (item, _) = pick_item(&ws, cx, 2);
    let n = items(&ws, cx, 2).len();
    keys(cx, "delete");
    assert!(!items(&ws, cx, 2).iter().any(|i| i.id == item), "delete");
    undo(cx);
    pick_item(&ws, cx, 2);
    keys(cx, "backspace");
    assert!(!items(&ws, cx, 2).iter().any(|i| i.id == item), "backspace");
    undo(cx);
    pick_item(&ws, cx, 2);
    keys(cx, "shift-e");
    let now = items(&ws, cx, 2);
    assert_eq!(now.len(), n + 1, "Shift+E adds an empty item");
    assert!(now.iter().any(|i| i.length == 3840 && i.audio));
}

/// Enter on a MIDI item opens the Piano Roll on it; a double click does the same
#[gpui::test]
fn opening_an_item(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (_, p) = pick_item(&ws, cx, 1);
    keys(cx, "enter");
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::PianoRoll)), "Enter opens the editor");
    ws.update(cx, |ws, cx| ws.close(Screen::PianoRoll, cx));
    cx.run_until_parked();
    button_at(cx, p, MouseButton::Left, 2);
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::PianoRoll)), "a double click opens the editor");
}

/// = and - zoom the arrangement
#[gpui::test]
fn zoom_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    pick_item(&ws, cx, 2);
    let comp = ws.read_with(cx, |ws, _| ws.composition.clone());
    let z = |cx: &mut VisualTestContext| comp.read_with(cx, |c, _| c.view.ticks_per_px);
    let before = z(cx);
    keys(cx, "=");
    assert!(z(cx) < before, "= zooms in");
    keys(cx, "-");
    keys(cx, "-");
    assert!(z(cx) > before, "- zooms out");
}

/// The wheel (REAPER's): a mouse wheel zooms; Alt+wheel scrolls sideways; Ctrl+wheel changes the track height
#[gpui::test]
fn wheel_modes(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let comp = ws.read_with(cx, |ws, _| ws.composition.clone());
    let p = center(cx, "timeline");
    let wheel = |cx: &mut VisualTestContext, dy: f32, m: Modifiers| {
        cx.simulate_event(gpui::ScrollEvent {
            position: p,
            delta: gpui::ScrollDelta::Lines(point(0.0, dy)),
            modifiers: m,
            touch_phase: gpui::TouchPhase::Moved,
            ..Default::default()
        });
        cx.run_until_parked();
    };
    let state =
        |cx: &mut VisualTestContext| comp.read_with(cx, |c, _| (c.view.ticks_per_px, c.view.scroll_ticks, c.track_h));
    let (z, _, h) = state(cx);
    wheel(cx, 3.0, Modifiers::none());
    assert_ne!(state(cx).0, z, "the wheel zooms");
    let (z, s, _) = state(cx);
    wheel(cx, -3.0, Modifiers { alt: true, ..Default::default() });
    assert_eq!(state(cx).0, z, "Alt+wheel does not zoom");
    assert_ne!(state(cx).1, s, "Alt+wheel scrolls sideways");
    wheel(cx, 3.0, Modifiers { control: true, ..Default::default() });
    assert_ne!(state(cx).2, h, "Ctrl+wheel changes the height");
}

/// A click on the ruler moves the cursor there
#[gpui::test]
fn ruler_click_locates(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    click(cx, "ruler");
    settle(cx);
    let core = core(&ws, cx);
    assert!(core.read_with(cx, |c, _| c.port().telemetry().position.samples) > 0);
}

// --- transport bar ----------------------------------------------------------------------------------------------

/// The tempo drags up in one undo step; a click on the display changes its format
#[gpui::test]
fn tempo_drag_and_display_format(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let bpm = |cx: &mut VisualTestContext| model::tempo_map(&root(&ws, cx)).bpm_at_sample(0);
    let before = bpm(cx);
    drag(cx, "tempo", 0.0, -40.0);
    assert!(bpm(cx) > before, "up raises the tempo");
    undo(cx);
    assert!((bpm(cx) - before).abs() < 1e-9, "one undo step");
    let f = ws.read_with(cx, |ws, _| ws.display_format);
    click(cx, "lcd");
    assert_ne!(ws.read_with(cx, |ws, _| ws.display_format), f);
}

// --- mixing -----------------------------------------------------------------------------------------------------

fn rack_text(ws: &Entity<Workspace>, cx: &mut VisualTestContext, id: NodeId, sec: &str, key: &str) -> String {
    model::at(&channel(ws, cx, id).rack, &[sec, key]).and_then(Value::as_text).unwrap_or("").to_string()
}

fn mixing(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<crate::pages::mixing::MixingPage> {
    ws.read_with(cx, |ws, _| ws.mixing.clone())
}

fn drawn(cx: &mut VisualTestContext, selector: &str) -> bool {
    let s: &'static str = Box::leak(selector.to_string().into_boxed_str());
    cx.debug_bounds(s).is_some()
}

/// The first normal channel that has inserts
fn channel_with_inserts(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> model::ChannelView {
    model::channels(&root(ws, cx))
        .into_iter()
        .find(|c| c.kind == ChannelKind::Normal && !c.inserts.is_empty())
        .expect("a channel with inserts")
}

/// The fader over the meter: a drag is one gesture; a double click and the middle button go back to 0 dB
#[gpui::test]
fn fader_drag_and_double_click(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let id = some_channel(&ws, cx);
    select_strip(&ws, cx, id);
    let sel = format!("fader-{}", id.number);
    drag(cx, &sel, 0.0, 60.0);
    let v = channel(&ws, cx, id).volume;
    assert!(view::gain_to_db(v) < -6.0, "the fader went down: {} dB", view::gain_to_db(v));
    undo(cx);
    assert_eq!(channel(&ws, cx, id).volume, 1.0, "one undo step");
    drag(cx, &sel, 0.0, 60.0);
    let p = center(cx, &sel);
    button_at(cx, p, MouseButton::Left, 2);
    assert!((channel(&ws, cx, id).volume - 1.0).abs() < 1e-9, "double click: 0 dB");
    drag(cx, &sel, 0.0, 60.0);
    button_at(cx, p, MouseButton::Middle, 1);
    assert!((channel(&ws, cx, id).volume - 1.0).abs() < 1e-9, "middle button: 0 dB");
}

/// A knob of the analog strip: a drag is one undo step, Shift is fine, the right button's menu resets, copies and
/// pastes the value
#[gpui::test]
fn strip_knob_gestures(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let id = some_channel(&ws, cx);
    select_strip(&ws, cx, id);
    let sel = format!("rk-{}-eq-high", id.number);
    let high = |cx: &mut VisualTestContext| {
        model::at(&channel(&ws, cx, id).rack, &["eq", "high"]).and_then(Value::as_real).unwrap_or(0.0)
    };
    drag(cx, &sel, 0.0, -40.0);
    let coarse = high(cx);
    assert!(coarse > 4.0, "40 px up: {coarse} dB");
    undo(cx);
    assert_eq!(high(cx), 0.0);
    let p = center(cx, &sel);
    drag_at(cx, p, 0.0, -40.0, Modifiers::shift());
    let fine = high(cx);
    assert!(fine > 0.0 && fine < coarse / 5.0, "Shift is fine: {fine}");
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "menu-copy");
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "menu-default");
    assert_eq!(high(cx), 0.0, "reset");
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "menu-paste");
    assert!((high(cx) - fine).abs() < 1e-6, "paste");
    // a stepped knob moves through its choices: ATK
    drag(cx, &format!("rk-{}-dynamics-compSpeed", id.number), 0.0, -120.0);
    assert_eq!(rack_text(&ws, cx, id, "dynamics", "compSpeed"), "slow");
    // the channel's own knobs: TRIM and PAN
    let field = |cx: &mut VisualTestContext, key: &str| {
        let core = core(&ws, cx);
        core.read_with(cx, |c, _| c.port().find(id).and_then(|n| n.get(key).and_then(Value::as_real)).unwrap_or(0.0))
    };
    drag(cx, &format!("rk-{}-ch-trim", id.number), 0.0, -40.0);
    assert!(field(cx, "trim") > 0.0, "trim up");
    let pan = field(cx, "pan");
    drag(cx, &format!("rk-{}-ch-pan", id.number), 0.0, -30.0);
    assert!(field(cx, "pan") > pan, "pan right");
}

/// The desk switches: the gain stage's model and the compressor's (click = next, right button = the list), the tube's
/// lamp (light or realistic), MONO, LCUT, Ø, ⇄, M, S
#[gpui::test]
fn strip_switches(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let id = some_channel(&ws, cx);
    select_strip(&ws, cx, id);
    let n = id.number;
    assert_eq!(rack_text(&ws, cx, id, "tube", "model"), "4558");
    click(cx, &format!("choice-{n}-tube-model"));
    assert_eq!(rack_text(&ws, cx, id, "tube", "model"), "tl072", "a click: the next model");
    let p = center(cx, &format!("choice-{n}-tube-model"));
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "menu-choice-12ax7");
    assert_eq!(rack_text(&ws, cx, id, "tube", "model"), "12ax7", "the list");
    click(cx, &format!("sat-{n}"));
    assert_eq!(
        model::at(&channel(&ws, cx, id).rack, &["tube", "realistic"]).and_then(Value::as_bool),
        Some(true),
        "the lamp"
    );
    click(cx, &format!("choice-{n}-dynamics-compModel"));
    assert_eq!(rack_text(&ws, cx, id, "dynamics", "compModel"), "api2500");
    click(cx, &format!("choice-{n}-dynamics-gateMode"));
    assert_eq!(rack_text(&ws, cx, id, "dynamics", "gateMode"), "kick");
    for (key, read) in [
        ("mono", Box::new(|c: &model::ChannelView| c.mono) as Box<dyn Fn(&model::ChannelView) -> bool>),
        ("phase", Box::new(|c| c.phase_invert)),
        ("swap", Box::new(|c| c.swap)),
        ("mute", Box::new(|c| c.mute)),
        ("solo", Box::new(|c| c.solo)),
    ] {
        click(cx, &format!("{key}-{n}"));
        assert!(read(&channel(&ws, cx, id)), "{key}");
    }
    click(cx, &format!("lcut-{n}"));
    assert_eq!(model::at(&channel(&ws, cx, id).rack, &["tube", "lowCut"]).and_then(Value::as_bool), Some(true));
}

/// ANALOG / DIGITAL: the switch changes the whole page; the digital strip is plain (no analog section, the effects by
/// name: the LED bypasses one), and the strip's settings are kept for when the user switches back
#[gpui::test]
fn analog_and_digital(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let ch = channel_with_inserts(&ws, cx);
    let id = ch.id;
    mixing(&ws, cx).update(cx, |m, cx| m.select_channel(id, cx));
    cx.run_until_parked();
    drag(cx, &format!("rk-{}-eq-high", id.number), 0.0, -40.0);
    let high = model::at(&channel(&ws, cx, id).rack, &["eq", "high"]).and_then(Value::as_real);
    assert!(drawn(cx, &format!("led-{}-0", id.number)), "analog: the effects as LEDs");
    click(cx, "mode-digital");
    assert!(!model::song(&root(&ws, cx)).analog);
    assert!(!drawn(cx, &format!("rk-{}-eq-high", id.number)), "digital: no analog strip");
    assert!(!drawn(cx, &format!("led-{}-0", id.number)), "digital: no LEDs");
    let insert = ch.inserts[0].id;
    click(cx, &format!("slot-led-{}", insert.number));
    assert!(channel(&ws, cx, id).inserts[0].bypass, "the slot's LED bypasses the effect");
    click(cx, "mode-analog");
    assert!(model::song(&root(&ws, cx)).analog);
    assert_eq!(
        model::at(&channel(&ws, cx, id).rack, &["eq", "high"]).and_then(Value::as_real),
        high,
        "the strip's settings stay"
    );
}

/// The effects: a double click on an LED opens its controls, on an empty one the browser to add one; in the rack the
/// LED bypasses, the point cycles through the strip's points, the menu moves and removes
#[gpui::test]
fn effects(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let ch = channel_with_inserts(&ws, cx);
    let id = ch.id;
    mixing(&ws, cx).update(cx, |m, cx| m.select_channel(id, cx));
    cx.run_until_parked();
    let p = center(cx, &format!("led-{}-0", id.number));
    button_at(cx, p, MouseButton::Left, 2);
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::Effect)), "the effect's controls");
    ws.update(cx, |ws, cx| ws.close(Screen::Effect, cx));
    let p = center(cx, &format!("led-{}-4", id.number));
    button_at(cx, p, MouseButton::Left, 2);
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::Browser)), "an empty slot: add an effect");
    ws.update(cx, |ws, cx| ws.close(Screen::Browser, cx));
    cx.run_until_parked();
    let insert = ch.inserts[0].id;
    click(cx, &format!("rack-led-{}", insert.number));
    assert!(channel(&ws, cx, id).inserts[0].bypass);
    click(cx, &format!("point-{}", insert.number));
    assert_eq!(&*channel(&ws, cx, id).inserts[0].point, "pre-pan", "post-eq → pre-pan");
    click(cx, "add-effect");
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::Browser)));
    let n = channel(&ws, cx, id).inserts.len();
    let p = center(cx, &format!("rack-{}", insert.number));
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "menu-effect-remove");
    assert_eq!(channel(&ws, cx, id).inserts.len(), n - 1);
}

/// The send button on another strip: the selected channel's send to it, made and removed; its knob moves the amount.
/// An FX knob makes the send to its return and moves it in the same gesture
#[gpui::test]
fn sends(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let normal: Vec<NodeId> = model::channels(&root(&ws, cx))
        .iter()
        .filter(|c| c.kind == ChannelKind::Normal)
        .skip(5)
        .take(2)
        .map(|c| c.id)
        .collect();
    let (a, b) = (normal[0], normal[1]);
    select_strip(&ws, cx, a);
    click(cx, &format!("send-{}", b.number));
    let send = channel(&ws, cx, a).sends.iter().find(|s| s.to == b).map(|s| s.id).expect("a send to b");
    drag(cx, &format!("sendknob-{}", b.number), 0.0, -20.0);
    let amount = channel(&ws, cx, a).sends.iter().find(|s| s.id == send).unwrap().amount;
    assert!(amount > 1.0, "the knob raised the send: {amount}");
    let p = center(cx, &format!("sendknob-{}", b.number));
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "menu-send-pre");
    assert!(channel(&ws, cx, a).sends.iter().find(|s| s.id == send).unwrap().pre, "before the fader");
    click(cx, &format!("send-{}", b.number));
    assert!(!channel(&ws, cx, a).sends.iter().any(|s| s.to == b), "a second click removes it");
    // the FX knob of a channel without a send to the first return
    let ret = model::channels(&root(&ws, cx)).into_iter().find(|c| c.kind == ChannelKind::Return).unwrap().id;
    let c = model::channels(&root(&ws, cx))
        .into_iter()
        .find(|c| c.kind == ChannelKind::Normal && !c.sends.iter().any(|s| s.to == ret))
        .unwrap()
        .id;
    mixing(&ws, cx).update(cx, |m, cx| m.select_channel(c, cx));
    cx.run_until_parked();
    drag(cx, &format!("fx-{}-1", c.number), 0.0, -60.0);
    let s = channel(&ws, cx, c).sends.into_iter().find(|s| s.to == ret).expect("the FX send was made");
    assert!(s.amount > 0.0);
}

/// ←/→ select; ↑/↓ the selected fader 1 dB, Shift 3 dB, Alt 0.1 dB; Alt+→ moves the channel; Delete removes it;
/// Shift+Insert adds one
#[gpui::test]
fn desk_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let id = some_channel(&ws, cx);
    select_strip(&ws, cx, id);
    let db = |cx: &mut VisualTestContext| view::gain_to_db(channel(&ws, cx, id).volume);
    keys(cx, "up");
    assert!((db(cx) - 1.0).abs() < 1e-6, "up: +1 dB, {}", db(cx));
    keys(cx, "shift-down");
    assert!((db(cx) + 2.0).abs() < 1e-6, "shift+down: -3 dB, {}", db(cx));
    keys(cx, "alt-up");
    assert!((db(cx) + 1.9).abs() < 1e-6, "alt+up: +0.1 dB, {}", db(cx));
    let m = mixing(&ws, cx);
    keys(cx, "right");
    assert_ne!(m.read_with(cx, |m, _| m.selected), Some(id), "right selects the next one");
    keys(cx, "left");
    assert_eq!(m.read_with(cx, |m, _| m.selected), Some(id), "left goes back");
    let place = |cx: &mut VisualTestContext| model::channels(&root(&ws, cx)).iter().position(|c| c.id == id);
    let before = place(cx);
    keys(cx, "alt-right");
    assert_eq!(place(cx), before.map(|p| p + 1), "alt+right moves the channel");
    let n = model::channels(&root(&ws, cx)).len();
    keys(cx, "shift-insert");
    assert_eq!(model::channels(&root(&ws, cx)).len(), n + 1, "shift+insert adds a channel");
    keys(cx, "delete");
    assert_eq!(model::channels(&root(&ws, cx)).len(), n, "delete removes the selected channel");
}

/// F2 (or Enter, or a double click on the strip) renames the channel in place
#[gpui::test]
fn renaming_a_channel(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let id = some_channel(&ws, cx);
    select_strip(&ws, cx, id);
    keys(cx, "f2");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" });
    cx.simulate_input("Baixo DI");
    keys(cx, "enter");
    assert_eq!(&*channel(&ws, cx, id).name, "Baixo DI");
}

/// The strip's menu: move, rename, remove, remove unused, color, track channels, output
#[gpui::test]
fn strip_menu(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let id = some_channel(&ws, cx);
    select_strip(&ws, cx, id);
    let menu = |cx: &mut VisualTestContext| {
        let p = center(cx, &format!("name-{}", id.number));
        button_at(cx, p, MouseButton::Right, 1);
    };
    menu(cx);
    click(cx, "menu-channels-4");
    assert_eq!(channel(&ws, cx, id).channels, 4, "track channels");
    menu(cx);
    click(cx, "menu-color-2");
    assert!(channel(&ws, cx, id).color.is_some(), "a color");
    menu(cx);
    click(cx, "menu-color-reset");
    assert!(channel(&ws, cx, id).color.is_none(), "reset");
    let group = model::channels(&root(&ws, cx)).iter().find(|c| c.kind == ChannelKind::Group).unwrap().id;
    menu(cx);
    click(cx, &format!("menu-output-{}", group.number));
    assert_eq!(channel(&ws, cx, id).output, Some(group), "the output");
    // remove unused: a new channel nothing uses goes away
    let n = model::channels(&root(&ws, cx)).len();
    keys(cx, "shift-insert");
    menu(cx);
    click(cx, "menu-remove-unused");
    assert!(model::channels(&root(&ws, cx)).len() <= n, "the unused channel is gone");
}

/// A group channel's marker hides and shows the channels that output to it
#[gpui::test]
fn collapsing_a_group(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let chs = model::channels(&root(&ws, cx));
    let group = chs.iter().find(|c| c.kind == ChannelKind::Group).unwrap().id;
    let member = chs.iter().find(|c| c.output == Some(group)).unwrap().id;
    assert!(drawn(cx, &format!("strip-{}", member.number)));
    click(cx, &format!("group-{}", group.number));
    assert!(!drawn(cx, &format!("strip-{}", member.number)), "hidden");
    click(cx, &format!("group-{}", group.number));
    assert!(drawn(cx, &format!("strip-{}", member.number)), "shown again");
}

/// Ctrl+click selects several strips; linked faders move together, in one gesture (P-18)
#[gpui::test]
fn linked_faders_move_together(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let chs: Vec<NodeId> = model::channels(&root(&ws, cx))
        .iter()
        .filter(|c| c.kind == ChannelKind::Normal)
        .skip(5)
        .take(2)
        .map(|c| c.id)
        .collect();
    let (a, b) = (chs[0], chs[1]);
    select_strip(&ws, cx, a);
    click_strip(cx, b, if cfg!(target_os = "macos") { Modifiers::command() } else { Modifiers::control() });
    click(cx, "link");
    drag(cx, &format!("fader-{}", a.number), 0.0, 40.0);
    let (va, vb) = (channel(&ws, cx, a).volume, channel(&ws, cx, b).volume);
    assert!(va < 1.0 && (va - vb).abs() < 1e-9, "{va} {vb}");
    undo(cx);
    assert_eq!((channel(&ws, cx, a).volume, channel(&ws, cx, b).volume), (1.0, 1.0), "one undo step");
}

/// The master: its PRE faders move together while LINK is on, alone when off; the transformer and the bus
/// compressor's IN; the master section's knobs; the monitoring goes to the preferences, not to the document
#[gpui::test]
fn the_master(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let master = model::channels(&root(&ws, cx)).into_iter().find(|c| c.kind == ChannelKind::Master).unwrap().id;
    drag(cx, "preLeft", 0.0, 40.0);
    let (l, r, link) = channel(&ws, cx, master).pre;
    assert!(link && l < 1.0 && (l - r).abs() < 1e-9, "LINK: both, {l} {r}");
    undo(cx);
    click(cx, "pre-link");
    drag(cx, "preLeft", 0.0, 40.0);
    let (l, r, _) = channel(&ws, cx, master).pre;
    assert!(l < 1.0 && r == 1.0, "unlinked: only the left one, {l} {r}");
    click(cx, "master-xfmr");
    assert_eq!(model::at(&channel(&ws, cx, master).rack, &["transformer", "on"]).and_then(Value::as_bool), Some(true));
    click(cx, "master-comp-in");
    assert_eq!(model::at(&channel(&ws, cx, master).rack, &["dynamics", "on"]).and_then(Value::as_bool), Some(true));
    drag(cx, &format!("rk-{}-stereo-width", master.number), 0.0, -30.0);
    assert!(model::at(&channel(&ws, cx, master).rack, &["stereo", "width"]).and_then(Value::as_real).unwrap() > 100.0);
    let core = core(&ws, cx);
    let prefs = |cx: &mut VisualTestContext, k: &str| {
        core.read_with(cx, |c, _| model::at(c.port().preferences(), &["monitor", k]).cloned())
    };
    let modified = core.read_with(cx, |c, _| c.port().can_undo());
    click(cx, "master-dim");
    assert_eq!(prefs(cx, "dim"), Some(Value::bool(true)), "DIM");
    click(cx, "master-mono");
    click(cx, "master-side");
    assert_eq!(
        (prefs(cx, "mono"), prefs(cx, "side")),
        (Some(Value::bool(false)), Some(Value::bool(true))),
        "SIDE takes MONO off"
    );
    drag(cx, "monitor-level", 0.0, 40.0);
    assert!(prefs(cx, "level").and_then(|v| v.as_real()).unwrap() < 0.0, "the level");
    assert_eq!(core.read_with(cx, |c, _| c.port().can_undo()), modified, "the monitoring is not an edit");
}

/// FX on a track's header shows its channel on the mixing page
#[gpui::test]
fn fx_on_a_header_shows_the_channel(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let to = model::tracks(&root(&ws, cx))[2].outputs[0].to;
    click(cx, "fx-2");
    assert_eq!(ws.read_with(cx, |ws, _| ws.page), Page::Mixing);
    assert_eq!(mixing(&ws, cx).read_with(cx, |m, _| m.selected), Some(to));
}

/// The desk's tables name the document's keys: every knob's key is in the channels' and the master's nodes (the
/// optional ones may be absent), and every selector's value is one of its choices
#[gpui::test]
fn the_desk_tables_name_the_documents_keys(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let chs = model::channels(&root(&ws, cx));
    let normal = chs.iter().find(|c| c.kind == ChannelKind::Normal).unwrap();
    let master = chs.iter().find(|c| c.kind == ChannelKind::Master).unwrap();
    for (table, ch) in [(STRIP_KNOBS, normal), (MASTER_KNOBS, master)] {
        for k in table {
            if k.sec.is_empty() {
                continue;
            }
            let v = model::at(&ch.rack, &[k.sec, k.key]);
            assert!(v.is_some(), "{}: rack.{}.{}", ch.name, k.sec, k.key);
            if let Scale::Choice(choices) = k.scale {
                let now = v.and_then(Value::as_text).unwrap_or("");
                assert!(choices.contains(&now), "rack.{}.{}: {now}", k.sec, k.key);
            }
        }
    }
}

// --- routing ----------------------------------------------------------------------------------------------------

/// Routing: an output goes to the channel picked; "Create channels for the outputs" is one undo step
#[gpui::test]
fn routing(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    ws.update_in(cx, |ws, window, cx| ws.open(Screen::Routing, None, window, cx));
    cx.run_until_parked();
    let out = model::tracks(&root(&ws, cx))[2].outputs[0].id;
    let master = model::channels(&root(&ws, cx)).iter().find(|c| c.kind == ChannelKind::Master).unwrap().id;
    click(cx, &format!("route-{}", out.number));
    click(cx, &format!("pick-{}-{}", out.number, master.number));
    assert_eq!(model::tracks(&root(&ws, cx))[2].outputs[0].to, master);
    let drums = model::tracks(&root(&ws, cx))[1].clone();
    let n = model::channels(&root(&ws, cx)).len();
    click(cx, &format!("create-channels-{}", drums.id.number));
    assert_eq!(model::channels(&root(&ws, cx)).len(), n + 1 + drums.outputs.len(), "a group and a channel per output");
    undo(cx);
    assert_eq!(model::channels(&root(&ws, cx)).len(), n, "one undo step");
}

// --- the note editors (agent B): the Piano Roll ----------------------------------------------------------------

mod notes_editors {
    use std::collections::BTreeSet;

    use gpui::{AppContext, WeakEntity};

    use super::*;
    use crate::screens::ScreenCx;
    use crate::screens::piano_roll::music::{BEAT, Clip};
    use crate::screens::piano_roll::{PianoRoll, Tool};

    /// A screen alone in a window, on the fake core, with the registry's keys bound
    pub(super) fn open_screen<V: gpui::Render + 'static>(
        cx: &mut TestAppContext,
        screen: Screen,
        target: impl FnOnce(&Value) -> Option<NodeId> + 'static,
        make: fn(ScreenCx, &mut gpui::Window, &mut gpui::Context<V>) -> V,
    ) -> (Entity<V>, Entity<Core>, &mut VisualTestContext) {
        let p = port();
        let prefs = p.preferences().clone();
        cx.update(|cx| workspace::start(cx, &prefs));
        let mut p = Some(p);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let s2 = slot.clone();
        let (v, vcx) = cx.add_window_view(move |window, cx| {
            let core = cx.new(|cx| Core::new(p.take().unwrap(), cx));
            let t = target(core.read(cx).port().root());
            *s2.borrow_mut() = Some(core.clone());
            make(ScreenCx { core, workspace: WeakEntity::new_invalid(), screen, target: t }, window, cx)
        });
        vcx.run_until_parked();
        v.update(vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
        let core = slot.borrow_mut().take().unwrap();
        (v, core, vcx)
    }

    fn clip(pr: &Entity<PianoRoll>, cx: &mut VisualTestContext) -> Clip {
        pr.read_with(cx, |pr, cx| pr.clip(cx).expect("an item is open"))
    }

    /// Where a tick and key are on the screen
    fn at(pr: &Entity<PianoRoll>, cx: &mut VisualTestContext, tick: f64, key: i32) -> Point<Pixels> {
        let o = bounds(cx, "pr-grid").origin;
        pr.read_with(cx, |pr, _| o + point(px(pr.tick_to_x(tick) + 1.0), px(pr.key_to_y(key) + pr.key_h / 2.0)))
    }

    /// A key on screen with no note in the item
    fn free_key(pr: &Entity<PianoRoll>, cx: &mut VisualTestContext) -> i32 {
        let c = clip(pr, cx);
        let g = bounds(cx, "pr-grid");
        let used: BTreeSet<u8> = c.notes.values().map(|x| x.key).collect();
        pr.read_with(cx, |pr, _| {
            (0..128).rev().find(|k| {
                let y = pr.key_to_y(*k);
                y > 4.0 && y < g.size.height.as_f32() - 20.0 && !used.contains(&(*k as u8))
            })
        })
        .expect("a free key on screen")
    }

    fn piano(cx: &mut TestAppContext) -> (Entity<PianoRoll>, Entity<Core>, &mut VisualTestContext) {
        open_screen(cx, Screen::PianoRoll, crate::screens::piano_roll::music::first_midi, PianoRoll::new)
    }

    fn count(pr: &Entity<PianoRoll>, cx: &mut VisualTestContext) -> usize {
        clip(pr, cx).notes.len()
    }

    #[gpui::test]
    fn piano_roll_draws_moves_and_undoes(cx: &mut TestAppContext) {
        let (pr, core, cx) = piano(cx);
        let before = count(&pr, cx);
        let key = free_key(&pr, cx);
        let p = at(&pr, cx, (BEAT * 2) as f64 + 10.0, key);
        cx.simulate_click(p, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(count(&pr, cx), before + 1, "a click draws one note");
        let (n, note) =
            clip(&pr, cx).notes.iter().find(|(_, x)| x.key as i32 == key).map(|(n, x)| (*n, x.clone())).unwrap();
        assert_eq!(note.start, BEAT * 2, "on the grid");
        // dragging its body a beat right moves it: one gesture
        let from = at(&pr, cx, note.start as f64 + 20.0, key);
        let dx = pr.read_with(cx, |pr, _| (BEAT as f64 / pr.tpp) as f32);
        drag_at(cx, from, dx, 0.0, Modifiers::none());
        assert_eq!(clip(&pr, cx).notes[&n].start, BEAT * 3);
        settle(cx);
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(clip(&pr, cx).notes[&n].start, BEAT * 2, "the drag is one undo step");
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(count(&pr, cx), before, "and the drawing another");
    }

    #[gpui::test]
    fn piano_roll_right_drag_erases_in_one_step(cx: &mut TestAppContext) {
        let (pr, core, cx) = piano(cx);
        let c = clip(&pr, cx);
        let (_, first) = c.notes.iter().min_by_key(|(_, x)| (x.start, x.key)).map(|(n, x)| (*n, x.clone())).unwrap();
        let before = c.notes.len();
        let row: Vec<i64> = c.notes.values().filter(|x| x.key == first.key).map(|x| x.start).collect();
        let p = at(&pr, cx, first.start as f64 + 10.0, first.key as i32);
        let span = pr.read_with(cx, |pr, _| (crate::screens::piano_roll::music::BAR as f64 / pr.tpp) as f32);
        cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());
        for k in 1..=20 {
            cx.simulate_mouse_move(point(p.x + px(span * k as f32 / 20.0), p.y), MouseButton::Right, Modifiers::none());
        }
        cx.simulate_mouse_up(point(p.x + px(span), p.y), MouseButton::Right, Modifiers::none());
        cx.run_until_parked();
        let erased = before - count(&pr, cx);
        let expected = row
            .iter()
            .filter(|s| **s >= first.start && **s < first.start + crate::screens::piano_roll::music::BAR)
            .count();
        assert!(erased >= expected.min(1), "{erased} erased along the row");
        settle(cx);
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(count(&pr, cx), before, "one undo step");
    }

    #[gpui::test]
    fn piano_roll_velocity_area_paints_in_one_step(cx: &mut TestAppContext) {
        let (pr, core, cx) = piano(cx);
        let lane = bounds(cx, "pr-lane");
        let y = lane.origin.y + lane.size.height * 0.75;
        drag_at(cx, point(lane.origin.x + px(1.), y), (lane.size.width - px(3.)).as_f32(), 0.0, Modifiers::none());
        let low = clip(&pr, cx).notes.values().filter(|x| (x.velocity - 0.25).abs() < 0.06).count();
        assert!(low > 0, "the sweep set the velocities");
        settle(cx);
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(
            clip(&pr, cx).notes.values().filter(|x| (x.velocity - 0.25).abs() < 0.06).count(),
            0,
            "one undo step"
        );
    }

    #[gpui::test]
    fn piano_roll_shortcuts(cx: &mut TestAppContext) {
        let (pr, _core, cx) = piano(cx);
        // focus the editor (a right click on an empty place erases nothing)
        let key = free_key(&pr, cx);
        let p = at(&pr, cx, 10.0, key);
        button_at(cx, p, MouseButton::Right, 1);
        let keys_before: Vec<u8> = clip(&pr, cx).notes.values().map(|x| x.key).collect();
        keys(cx, "secondary-a shift-up");
        let keys_after: Vec<u8> = clip(&pr, cx).notes.values().map(|x| x.key).collect();
        assert_eq!(keys_before.iter().map(|k| k + 1).collect::<Vec<_>>(), keys_after, "Ctrl+A, Shift+Up");
        keys(cx, "secondary-up");
        assert_eq!(clip(&pr, cx).notes.values().next().unwrap().key, keys_before[0] + 13, "Ctrl+Up: an octave");
        let n = count(&pr, cx);
        keys(cx, "secondary-c secondary-v");
        assert_eq!(count(&pr, cx), n * 2, "copy and paste");
        keys(cx, "delete");
        assert_eq!(count(&pr, cx), n, "the pasted (selected) notes deleted");
        keys(cx, "shift-e");
        assert_eq!(pr.read_with(cx, |pr, _| pr.tool), Tool::Erase);
        keys(cx, "shift-k");
        assert_eq!(pr.read_with(cx, |pr, _| pr.tool), Tool::Knife);
        keys(cx, "escape");
        assert!(pr.read_with(cx, |pr, _| pr.sel.is_empty()), "Esc selects nothing");
    }

    #[gpui::test]
    fn piano_roll_knife_cuts_along_a_line(cx: &mut TestAppContext) {
        let (pr, _core, cx) = piano(cx);
        let c = clip(&pr, cx);
        let x = c.notes.values().min_by_key(|x| x.start).unwrap().clone();
        pr.update(cx, |pr, cx| {
            pr.tool = Tool::Knife;
            cx.notify();
        });
        let before = count(&pr, cx);
        // a vertical line through the middle of the first note
        let mid = (x.start + x.length / 2) as f64;
        let top = at(&pr, cx, mid, x.key as i32 + 1);
        drag_at(cx, top, 0.0, pr.read_with(cx, |pr, _| pr.key_h * 2.0), Modifiers::none());
        assert!(count(&pr, cx) > before, "the note crossed is cut in two");
    }

    #[gpui::test]
    fn piano_roll_quantize_button(cx: &mut TestAppContext) {
        let (pr, _core, cx) = piano(cx);
        click(cx, "pr-quantize");
        let grid = BEAT / 4;
        assert!(clip(&pr, cx).notes.values().all(|x| x.start % grid == 0), "every start on the 1/16 grid");
    }
}
