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
    let s: &'static str = lm_port::intern(selector);
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
    let global = Value::object([("keys", Value::object([("view.mixing", Value::text("f2"))]))]);
    let prefs = Value::object([("keymaps", Value::object([("Global", global)]))]);
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
    assert_eq!(ws.read_with(cx, |ws, _| ws.confirm.clone()), Some(Pending::Quit));
    keys(cx, "escape");
    assert_eq!(ws.read_with(cx, |ws, _| ws.confirm.clone()), None);
    // the window's close button asks the same way
    ws.update_in(cx, |ws, window, cx| ws.request(Pending::Close, window, cx));
    assert_eq!(ws.read_with(cx, |ws, _| ws.confirm.clone()), Some(Pending::Close));
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
    let s: &'static str = lm_port::intern(selector);
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
    let wet = |cx: &mut VisualTestContext| {
        let core = core(&ws, cx);
        core.read_with(cx, |c, _| {
            c.port().find(insert).and_then(|n| n.get("wet").and_then(Value::as_real)).unwrap_or(1.0)
        })
    };
    drag(cx, &format!("wet-{}", insert.number), 0.0, 40.0);
    assert!(wet(cx) < 1.0, "W/D down: {}", wet(cx));
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
        open_screen_on(cx, port(), screen, target, make)
    }

    pub(super) fn open_screen_on<V: gpui::Render + 'static>(
        cx: &mut TestAppContext,
        p: Box<dyn CorePort>,
        screen: Screen,
        target: impl FnOnce(&Value) -> Option<NodeId> + 'static,
        make: fn(ScreenCx, &mut gpui::Window, &mut gpui::Context<V>) -> V,
    ) -> (Entity<V>, Entity<Core>, &mut VisualTestContext) {
        let prefs = p.preferences().clone();
        cx.update(|cx| workspace::start(cx, &prefs));
        let mut p = Some(p);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let s2 = slot.clone();
        let (v, vcx) = cx.add_window_view(move |window, cx| {
            let core = cx.new(|cx| Core::new(p.take().unwrap(), cx));
            let t = target(core.read(cx).port().root());
            *s2.borrow_mut() = Some(core.clone());
            make(ScreenCx { core, workspace: WeakEntity::new_invalid(), screen, target: t, binding: None }, window, cx)
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

    // --- the automation editor ---------------------------------------------------------------------------------

    use crate::screens::automation::{AutomationEditor, Lane};

    fn auto(cx: &mut TestAppContext) -> (Entity<AutomationEditor>, Entity<Core>, &mut VisualTestContext) {
        open_screen(cx, Screen::Automation, |root| Lane::ids(root).first().copied(), AutomationEditor::new)
    }

    fn lane(ae: &Entity<AutomationEditor>, cx: &mut VisualTestContext) -> Lane {
        ae.read_with(cx, |ae, cx| ae.lane(cx).expect("a lane is open"))
    }

    #[gpui::test]
    fn automation_click_adds_a_point_and_drag_moves_it(cx: &mut TestAppContext) {
        let (ae, core, cx) = auto(cx);
        let before = lane(&ae, cx).points.len();
        let b = bounds(cx, "ae-curve");
        let x = ae.read_with(cx, |ae, _| ae.tick_to_x((BEAT * 6) as f64 + 10.0));
        let p = point(b.origin.x + px(x), b.origin.y + b.size.height * 0.5);
        drag_at(cx, p, 0.0, -15.0, Modifiers::none());
        let l = lane(&ae, cx);
        assert_eq!(l.points.len(), before + 1, "a click adds a point");
        assert!(l.points.iter().any(|q| q.tick == BEAT * 6), "on the grid");
        settle(cx);
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(lane(&ae, cx).points.len(), before, "adding, then moving: two steps");
    }

    #[gpui::test]
    fn automation_right_drag_erases_in_one_step(cx: &mut TestAppContext) {
        let (ae, core, cx) = auto(cx);
        let l = lane(&ae, cx);
        let before = l.points.len();
        let b = bounds(cx, "ae-curve");
        let p0 = l.points[1];
        let x = ae.read_with(cx, |ae, _| ae.tick_to_x(p0.tick as f64));
        // the point's height on screen: found by trying down its column
        let mut hit = None;
        for k in 0..200 {
            let py = b.origin.y + px(k as f32 * b.size.height.as_f32() / 200.0);
            let at = point(b.origin.x + px(x), py);
            cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
            cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
            cx.run_until_parked();
            if lane(&ae, cx).points.len() < before {
                hit = Some(k);
                break;
            }
        }
        assert!(hit.is_some(), "a right click on a point erases it");
        settle(cx);
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(lane(&ae, cx).points.len(), before, "one step");
    }

    #[gpui::test]
    fn automation_flip_twice_and_mode_and_keys(cx: &mut TestAppContext) {
        let (ae, _core, cx) = auto(cx);
        let before: Vec<(i64, f64)> = lane(&ae, cx).points.iter().map(|p| (p.tick, p.value)).collect();
        click(cx, "ae-flip-x");
        assert_ne!(lane(&ae, cx).points.iter().map(|p| (p.tick, p.value)).collect::<Vec<_>>(), before);
        click(cx, "ae-flip-x");
        let after: Vec<(i64, f64)> = lane(&ae, cx).points.iter().map(|p| (p.tick, p.value)).collect();
        assert_eq!(after.len(), before.len());
        for (a, b) in after.iter().zip(before.iter()) {
            assert_eq!(a.0, b.0);
            assert!((a.1 - b.1).abs() < 1e-5, "flipping twice is the identity");
        }
        // the lane's mode (I-21.5)
        click(cx, "ae-lane-mode");
        click(cx, "ae-menu-2");
        assert_eq!(lane(&ae, cx).mode, "touch");
        // keys: Shift+E erase mode, Ctrl+A and Delete
        let b = bounds(cx, "ae-curve");
        button_at(cx, point(b.origin.x + px(2.), b.origin.y + px(2.)), MouseButton::Right, 1);
        keys(cx, "shift-e");
        assert_eq!(ae.read_with(cx, |ae, _| ae.tool), crate::screens::automation::Tool::Erase);
        keys(cx, "secondary-a delete");
        assert_eq!(lane(&ae, cx).points.len(), 0, "Ctrl+A, Delete");
    }

    // --- the step editor ---------------------------------------------------------------------------------------

    use crate::screens::steps::StepEditor;

    fn steps(cx: &mut TestAppContext) -> (Entity<StepEditor>, Entity<Core>, &mut VisualTestContext) {
        open_screen(cx, Screen::Steps, |_| None, StepEditor::new)
    }

    fn item_clip(core: &Entity<Core>, id: NodeId, cx: &mut VisualTestContext) -> Clip {
        core.read_with(cx, |core, _| Clip::read(core.port().root(), id).expect("the item"))
    }

    /// Where a row's step is on screen
    fn step_at(st: &Entity<StepEditor>, id: NodeId, row: usize, s: i64, cx: &mut VisualTestContext) -> Point<Pixels> {
        let b = bounds(cx, &format!("st-cells-{}", id.number));
        st.read_with(cx, |st, _| {
            b.origin + point(px(s as f32 * st.step_w - st.scroll_x + st.step_w / 2.0), px(row as f32 * 20.0 + 10.0))
        })
    }

    /// A step of the first row that is dark, with the next two dark too
    fn dark_steps(core: &Entity<Core>, id: NodeId, cx: &mut VisualTestContext) -> (u8, i64) {
        let c = item_clip(core, id, cx);
        let key = c.notes.values().map(|x| x.key).min().expect("a note");
        let lit =
            |s: i64| c.notes.values().any(|x| x.key == key && x.start >= s * c.step && x.start < (s + 1) * c.step);
        let s =
            (0..StepEditor::steps(&c) - 3).find(|s| !lit(*s) && !lit(s + 1) && !lit(s + 2)).expect("three dark steps");
        (key, s)
    }

    #[gpui::test]
    fn steps_click_toggles_and_drag_paints_in_one_step(cx: &mut TestAppContext) {
        let (st, core, cx) = steps(cx);
        let id = st.read_with(cx, |st, _| st.items[0]);
        let before = item_clip(&core, id, cx).notes.len();
        let (_, s) = dark_steps(&core, id, cx);
        let p = step_at(&st, id, 0, s, cx);
        cx.simulate_click(p, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(item_clip(&core, id, cx).notes.len(), before + 1, "a click lights a step");
        cx.simulate_click(p, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(item_clip(&core, id, cx).notes.len(), before, "and darkens it");
        let (a, z) = (step_at(&st, id, 0, s, cx), step_at(&st, id, 0, s + 2, cx));
        drag_at(cx, a, (z.x - a.x).as_f32(), 0.0, Modifiers::none());
        assert_eq!(item_clip(&core, id, cx).notes.len(), before + 3, "three steps painted");
        settle(cx);
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(item_clip(&core, id, cx).notes.len(), before, "the painting is one undo step");
    }

    #[gpui::test]
    fn steps_add_remove_and_clone(cx: &mut TestAppContext) {
        let (st, core, cx) = steps(cx);
        let id = st.read_with(cx, |st, _| st.items[0]);
        let c0 = item_clip(&core, id, cx);
        let n0 = StepEditor::steps(&c0);
        let per_bar = crate::screens::piano_roll::music::BAR / c0.step;
        click(cx, "st-add-steps");
        assert_eq!(StepEditor::steps(&item_clip(&core, id, cx)), n0 + per_bar, "a bar more");
        click(cx, "st-remove-steps");
        assert_eq!(StepEditor::steps(&item_clip(&core, id, cx)), n0, "a bar less");
        click(cx, "st-clone-steps");
        let c1 = item_clip(&core, id, cx);
        assert_eq!(StepEditor::steps(&c1), n0 * 2, "cloned");
        let end = c1.loop_len.unwrap_or(c1.length) / 2;
        let first = c1.notes.values().filter(|x| x.start < end).count();
        let second = c1.notes.values().filter(|x| x.start >= end && x.start < 2 * end).count();
        assert_eq!(first, second, "the second half is a copy of the first");
    }
}

// --- the instrument window, the effect window and the browser (agent C) -----------------------------------------

fn open_screen(ws: &Entity<Workspace>, cx: &mut VisualTestContext, s: Screen, target: Option<NodeId>) {
    ws.update_in(cx, |ws, window, cx| ws.open(s, target, window, cx));
    cx.run_until_parked();
}

/// The first track playing a TripleOscillator: (track, instrument node)
fn triple(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> (NodeId, NodeId) {
    let t = model::tracks(&root(ws, cx))
        .into_iter()
        .find(|t| t.instrument.as_ref().is_some_and(|(_, p)| &**p == "tripleoscillator"))
        .expect("a TripleOscillator track");
    (t.id, t.instrument.unwrap().0)
}

fn state(ws: &Entity<Workspace>, cx: &mut VisualTestContext, node: NodeId) -> Value {
    let core = core(ws, cx);
    core.read_with(cx, |core, _| core.port().find(node).and_then(|n| n.get("state").cloned()).unwrap_or_default())
}

fn param(ws: &Entity<Workspace>, cx: &mut VisualTestContext, node: NodeId, key: &str) -> Option<f64> {
    state(ws, cx, node).get("params").and_then(|p| p.get(key)).and_then(Value::as_real)
}

/// A knob of the generic editor: a drag is one gesture and one undo step; the middle click goes back to the default
#[gpui::test]
fn instrument_knob_drag_and_middle_click(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (track, node) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    // the TripleOscillator's own controls are its legacy panel (lm-gui-plugins); the instrument functions' tabs are
    // the generic editor's
    click(cx, "iw-tab-Envelopes");
    let before = param(&ws, cx, node, "volAttack").unwrap_or(0.0);
    drag(cx, &format!("ctl-{node}-volAttack"), 0.0, -50.0);
    let after = param(&ws, cx, node, "volAttack").unwrap();
    assert!((after - (before + 0.5)).abs() < 1e-6, "50 px of 200 is a quarter of 0..2 s: {after}");
    undo(cx);
    assert_eq!(param(&ws, cx, node, "volAttack").unwrap_or(before), before, "one undo step");
    drag(cx, &format!("ctl-{node}-volAttack"), 0.0, -30.0);
    let p = center(cx, &format!("ctl-{node}-volAttack"));
    button_at(cx, p, MouseButton::Middle, 1);
    assert_eq!(param(&ws, cx, node, "volAttack"), Some(0.0), "the middle click: the default");
}

/// A choice by its key (U30) from its list; the envelopes' target buttons; the transposition switch on the track
#[gpui::test]
fn instrument_choice_and_tabs(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (track, node) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    click(cx, "iw-tab-Envelopes");
    click(cx, "iw-target-1");
    click(cx, &format!("ctl-{node}-filterType"));
    click(cx, "popup-item-1");
    assert_eq!(state(&ws, cx, node).get("filterType").and_then(Value::as_text), Some("high-pass"));
    // the note effects: the chords and the arpeggio are plugins of the track's chain now
    click(cx, "iw-tab-NoteEffects");
    assert!(cx.debug_bounds(lm_port::intern(&format!("ctl-{node}-arpOn"))).is_none());
    click(cx, "iw-ne-add-arpeggiator");
    let effects = |ws: &Entity<Workspace>, cx: &mut VisualTestContext| {
        model::list(&root(ws, cx), "tracks")
            .find(|t| model::id_of(t) == Some(track))
            .map(|t| model::list(t, "noteEffects").cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let list = effects(&ws, cx);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].get("plugin").and_then(Value::as_text), Some("arpeggiator"));
    let id = model::id_of(&list[0]).unwrap();
    assert!(
        cx.debug_bounds(lm_port::intern(&format!("ctl-{id}-time"))).is_some(),
        "its controls are the generic editor's"
    );
    click(cx, "iw-ne-add-chords");
    assert_eq!(effects(&ws, cx).len(), 2);
    click(cx, &format!("iw-ne-down-{}", id.number));
    assert_eq!(model::id_of(&effects(&ws, cx)[1]), Some(id), "moved down");
    click(cx, &format!("iw-ne-on-{}", id.number));
    assert_eq!(effects(&ws, cx)[1].get("bypass").and_then(Value::as_bool), Some(true), "bypassed");
    click(cx, &format!("iw-ne-remove-{}", id.number));
    assert_eq!(effects(&ws, cx).len(), 1);
    click(cx, "iw-tab-Tuning");
    click(cx, "iw-transpose");
    let follows = model::list(&root(&ws, cx), "tracks")
        .find(|t| model::id_of(t) == Some(track))
        .and_then(|t| t.get("transpose").and_then(Value::as_bool));
    assert_eq!(follows, Some(false));
    for t in ["Instrument", "Effects", "Midi"] {
        click(cx, &format!("iw-tab-{t}"));
    }
}

/// Swap ▾ replaces the instrument without removing the track, one step; the presets load and save
#[gpui::test]
fn instrument_swap_and_presets(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (track, node) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    click(cx, "iw-preset-next");
    let core = core(&ws, cx);
    assert!(core.read_with(cx, |c, _| c.port().can_undo()), "the preset loaded as an edit");
    click(cx, "iw-save");
    // Swap: the list by category; pick the sampler
    click(cx, "iw-swap");
    let k = (0..20)
        .find(|k| {
            let s: &'static str = lm_port::intern(&format!("popup-item-{k}"));
            cx.debug_bounds(s).is_some()
                && core.read_with(cx, |c, _| {
                    let mut list: Vec<_> = c.port().plugins().into_iter().filter(|p| p.instrument).collect();
                    list.sort_by_key(|p| (p.third_party, p.category.clone(), p.id.clone()));
                    let mut i = 0;
                    let mut last = None;
                    for p in list {
                        if last.as_ref() != Some(&p.category) {
                            i += 1;
                            last = Some(p.category.clone());
                        }
                        if &*p.id == "sampler" {
                            return i == *k;
                        }
                        i += 1;
                    }
                    false
                })
        })
        .expect("the sampler in the list");
    click(cx, &format!("popup-item-{k}"));
    let now = model::tracks(&root(&ws, cx)).into_iter().find(|t| t.id == track).unwrap().instrument.unwrap();
    assert_eq!(&*now.1, "sampler");
    assert_ne!(now.0, node, "a new node, the document's id");
    undo(cx);
    let back = model::tracks(&root(&ws, cx)).into_iter().find(|t| t.id == track).unwrap().instrument.unwrap();
    assert_eq!(&*back.1, "tripleoscillator");
}

/// The window closes when its track is removed (U38)
#[gpui::test]
fn instrument_window_closes_with_its_track(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (track, _) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::Instrument)));
    let core = core(&ws, cx);
    core.update(cx, |c, _| {
        let _ = c
            .port_mut()
            .apply(lm_port::Label::new("Remove"), vec![lm_port::EditOp::Remove { id: lm_port::Ref::Id(track) }]);
    });
    settle(cx);
    assert!(!ws.read_with(cx, |ws, _| ws.is_open(Screen::Instrument)), "closed with its track");
}

/// The effects rack: add, bypass by the LED, move down by the menu, open the controls, remove
#[gpui::test]
fn effect_rack(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let ch = some_channel(&ws, cx);
    open_screen(&ws, cx, Screen::Effect, Some(ch));
    let n = channel(&ws, cx, ch).inserts.len();
    click(cx, "ef-add");
    click(cx, "popup-item-1");
    let ins = channel(&ws, cx, ch).inserts;
    assert_eq!(ins.len(), n + 1);
    let last = ins.len() - 1;
    click(cx, &format!("ef-led-{last}"));
    assert!(channel(&ws, cx, ch).inserts[last].bypass);
    click(cx, &format!("ef-gear-{last}"));
    let added = channel(&ws, cx, ch).inserts[last].clone();
    // its controls are under the card, from its schema
    let some_param = core(&ws, cx).read_with(cx, |c, _| c.port().plugin_schema(&added.plugin).unwrap().params[0].key);
    assert!(cx.debug_bounds(lm_port::intern(&format!("ctl-{}-{some_param}", added.id))).is_some());
    if last > 0 {
        let p = center(cx, &format!("ef-card-{last}"));
        button_at(cx, p, MouseButton::Right, 1);
        click(cx, "popup-item-0");
        assert_eq!(channel(&ws, cx, ch).inserts[last - 1].id, added.id, "moved up");
    }
    let at = channel(&ws, cx, ch).inserts.iter().position(|x| x.id == added.id).unwrap();
    let p = center(cx, &format!("ef-card-{at}"));
    button_at(cx, p, MouseButton::Right, 1);
    let lastk = (0..12).rev().find(|k| cx.debug_bounds(lm_port::intern(&format!("popup-item-{k}"))).is_some()).unwrap();
    click(cx, &format!("popup-item-{lastk}"));
    assert_eq!(channel(&ws, cx, ch).inserts.len(), n, "removed");
}

/// The browser: a folder opens, a sample previews, ↓ walks and previews, the menu favourites, Enter sends a plugin
#[gpui::test]
fn browser_preview_keys_and_send(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_screen(&ws, cx, Screen::Browser, None);
    click(cx, "br-tab-Samples");
    let core = core(&ws, cx);
    let top = core.update(cx, |c, _| c.port_mut().browse(lm_port::BrowsePage::Samples, None, false).ok().unwrap());
    let folder = top.iter().find(|e| e.kind == lm_port::EntryKind::Folder).unwrap().clone();
    click(cx, &format!("br-entry-{}", folder.id.0));
    let kids =
        core.update(cx, |c, _| c.port_mut().browse(lm_port::BrowsePage::Samples, Some(folder.id), false).ok().unwrap());
    let first = kids.first().unwrap().clone();
    click(cx, &format!("br-entry-{}", first.id.0));
    assert!(cx.debug_bounds("br-playing").is_some() || first.kind == lm_port::EntryKind::Folder, "the preview plays");
    keys(cx, "down");
    let p = center(cx, &format!("br-entry-{}", first.id.0));
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "br-menu-item-1");
    let fav = core.update(cx, |c, _| c.port_mut().browse(lm_port::BrowsePage::Favorites, None, false).ok().unwrap());
    assert!(fav.iter().any(|e| e.id == first.id), "a favourite");
    // Instruments: Enter sends the selected instrument to the active track
    click(cx, "br-tab-Instruments");
    let plugins =
        core.update(cx, |c, _| c.port_mut().browse(lm_port::BrowsePage::Instruments, None, false).ok().unwrap());
    let sampler =
        plugins.iter().find(|e| e.info.get("plugin").and_then(Value::as_text) == Some("sampler")).unwrap().clone();
    let track = model::tracks(&root(&ws, cx)).into_iter().find(|t| t.instrument.is_some()).unwrap().id;
    click(cx, &format!("br-entry-{}", sampler.id.0));
    keys(cx, "enter");
    let now = model::tracks(&root(&ws, cx)).into_iter().find(|t| t.id == track).unwrap().instrument.unwrap();
    assert_eq!(&*now.1, "sampler");
}

/// The browser's search: words and "a phrase", the results replacing the tree
#[gpui::test]
fn browser_search(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_screen(&ws, cx, Screen::Browser, None);
    click(cx, "br-search");
    cx.simulate_input("kick");
    settle(cx);
    let core = core(&ws, cx);
    let hits = core.update(cx, |c, _| c.port_mut().search("kick").ok().unwrap());
    assert!(!hits.is_empty());
    assert!(cx.debug_bounds(lm_port::intern(&format!("br-entry-{}", hits[0].id.0))).is_some());

    #[gpui::test]
    fn automation_hand_won_and_rearm(cx: &mut TestAppContext) {
        let mut fake =
            FakePort::new(Config { tracks: 12, slow_delay: None, slow_track: usize::MAX }).expect("the demo");
        let l = crate::screens::automation::Lane::all(&fake).into_iter().next().expect("a lane");
        fake.hand_wins(l.target.expect("the lane's node"), &l.param);
        let (_ae, core, cx) = notes_editors::open_screen_on(
            cx,
            Box::new(fake),
            Screen::Automation,
            move |_| Some(l.id),
            crate::screens::automation::AutomationEditor::new,
        );
        let won = |core: &Entity<Core>, cx: &mut VisualTestContext| {
            core.read_with(cx, |core, _| core.port().telemetry().hand_won.len())
        };
        assert_eq!(won(&core, cx), 1, "the hand won");
        click(cx, "ae-rearm");
        settle(cx);
        assert_eq!(won(&core, cx), 0, "re-armed: back to the curve");
    }
}

// --- agent D: settings, export, project, notices, palette, new project, about ----------------------------------

fn prefs(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Value {
    let core = core(ws, cx);
    core.read_with(cx, |core, _| core.port().preferences().clone())
}

fn status(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Value {
    let core = core(ws, cx);
    core.read_with(cx, |core, _| core.port().status().clone())
}

fn pref_at(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &[&str]) -> Option<Value> {
    let p = prefs(ws, cx);
    let steps: Vec<lm_port::PathStep> = path.iter().map(|k| lm_port::PathStep::key(k)).collect();
    p.at_path(&steps).cloned()
}

/// The fake's operations are threads: real time passes, then the pump applies them
fn wait_until(ws: &Entity<Workspace>, cx: &mut VisualTestContext, done: impl Fn(&Value) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        std::thread::sleep(Duration::from_millis(40));
        settle(cx);
        if done(&status(ws, cx)) || std::time::Instant::now() > deadline {
            break;
        }
    }
}

fn open_part(ws: &Entity<Workspace>, cx: &mut VisualTestContext, s: Screen, part: Option<&'static str>) {
    cx.update(|_, cx| cx.set_global(crate::screens::palette::ScreenArg(part)));
    ws.update_in(cx, |ws, window, cx| ws.open(s, None, window, cx));
    cx.run_until_parked();
}

/// Design 08: an output chosen applies at once, and the device reopens 0.4 s after the last change
#[gpui::test]
fn settings_device_applies_at_once(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Settings, Some("audio"));
    click(cx, "output-dev-1");
    assert_eq!(
        pref_at(&ws, cx, &["audio", "output"]).and_then(|v| v.as_text().map(str::to_string)).as_deref(),
        Some("hw:1")
    );
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    let core = core(&ws, cx);
    let reopening = core.read_with(cx, |c, _| c.port().loading(&lm_port::Target::Device) != lm_port::Loading::Ready);
    assert!(reopening, "the reopen started");
    click(cx, "block-4");
    assert_eq!(pref_at(&ws, cx, &["audio", "block"]).and_then(|v| v.as_int()), Some(512));
}

/// A restart-only setting shows the band; back to the running value, it goes
#[gpui::test]
fn settings_restart_band(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Settings, Some("appearance"));
    click(cx, "theme-1");
    assert!(cx.debug_bounds("restart-band").is_some());
    click(cx, "theme-0");
    assert!(cx.debug_bounds("restart-band").is_none());
}

/// I-21.1: a MIDI port's role and its switches are the machine's preferences
#[gpui::test]
fn settings_midi_port(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Settings, Some("midi"));
    click(cx, "midi-role-0-2");
    let role = pref_at(&ws, cx, &["midi", "ports"])
        .and_then(|l| l.at(0).and_then(|p| p.get("role")).and_then(|r| r.as_text().map(str::to_string)));
    assert_eq!(role.as_deref(), Some("both"));
    click(cx, "midi-add-0");
    let n = pref_at(&ws, cx, &["midi", "ports"]).map(|l| l.len()).unwrap_or(0);
    assert_eq!(n, 5, "the port present and unknown was added");
}

/// A shortcut is changed by pressing it; a conflict waits for a second press, which takes the keys
#[gpui::test]
fn settings_change_a_shortcut(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Settings, Some("keys"));
    let i = REGISTRY.iter().position(|d| d.id == "transport.loop").unwrap();
    click(cx, "kctx-1"); // the Transport context
    click(cx, &format!("key-change-{i}"));
    keys(cx, "alt-x");
    let k = |ws: &Entity<Workspace>, cx: &mut VisualTestContext, id: &str| {
        pref_at(ws, cx, &["keymaps", "Transport", "keys", id]).and_then(|v| v.as_text().map(str::to_string))
    };
    assert_eq!(k(&ws, cx, "transport.loop").as_deref(), Some("alt-x"));
    click(cx, &format!("key-change-{i}"));
    keys(cx, "r");
    assert_eq!(k(&ws, cx, "transport.loop").as_deref(), Some("alt-x"), "a conflict waits");
    keys(cx, "r");
    assert_eq!(k(&ws, cx, "transport.loop").as_deref(), Some("r"));
    assert_eq!(k(&ws, cx, "transport.record").as_deref(), Some(""), "taken from Record");
}

/// The palette: typing finds, Enter runs the action's GPUI action (here, the export window)
#[gpui::test]
fn palette_runs_an_action(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    keys(cx, "secondary-k");
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::Palette)));
    cx.simulate_input("exportaud");
    cx.run_until_parked();
    keys(cx, "enter");
    ws.read_with(cx, |ws, _| {
        assert!(ws.is_open(Screen::Export), "the palette ran Export audio");
        assert!(!ws.is_open(Screen::Palette));
    });
}

/// The keys of agent D's screens
#[gpui::test]
fn file_keys_open_their_screens(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    for (k, s) in [
        ("secondary-e", Screen::Export),
        ("secondary-o", Screen::Project),
        ("secondary-shift-s", Screen::Project),
        ("secondary-n", Screen::NewProject),
        ("f1", Screen::About),
    ] {
        keys(cx, k);
        assert!(ws.read_with(cx, |ws, _| ws.is_open(s)), "{k} opens {s:?}");
        keys(cx, "escape");
    }
}

/// The export screen: one screen, then the export as an operation and its report
#[gpui::test]
fn export_screen(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Export, None);
    click(cx, "what-1");
    click(cx, "fmt-mp3");
    click(cx, "start-export");
    wait_until(&ws, cx, |s| s.get("lastExport").is_some_and(|r| !r.is_null()));
    let r = status(&ws, cx).get("lastExport").cloned().unwrap();
    assert_eq!(r.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(r.get("identical").and_then(|v| v.as_bool()), Some(false), "a fast range is said not identical");
}

/// Opening a project another program holds: read-only (H10); Save as frees it
#[gpui::test]
fn project_locked_and_save_as(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Project, Some("open"));
    click(cx, "recent-2");
    click(cx, "open-recent");
    let ro = |s: &Value| s.get("project").and_then(|p| p.get("readOnly")).and_then(|v| v.as_bool()).unwrap_or(false);
    wait_until(&ws, cx, ro);
    assert!(ro(&status(&ws, cx)), "locked elsewhere: read-only");
    click(cx, "ro-save-as");
    click(cx, "version-up");
    click(cx, "save-as-go");
    wait_until(&ws, cx, |s| !ro(s));
    let folder = status(&ws, cx)
        .get("project")
        .and_then(|p| p.get("folder"))
        .and_then(|v| v.as_text().map(str::to_string))
        .unwrap();
    assert!(folder.ends_with("-01"), "{folder}");
}

/// After a crash while recording, a take is recovered from the dialog (U5)
#[gpui::test]
fn project_recovers_a_take(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Project, Some("open"));
    click(cx, "recent-0");
    click(cx, "open-recent");
    let takes = |s: &Value| s.get("interruptedTakes").map_or(0, |l| l.len());
    wait_until(&ws, cx, |s| takes(s) == 2);
    click(cx, "recover-0");
    assert_eq!(takes(&status(&ws, cx)), 1);
}

/// A new project: its folder, its rate and the template
#[gpui::test]
fn new_project_creates(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::NewProject, None);
    click(cx, "np-rate-3");
    click(cx, "create");
    assert!(!ws.read_with(cx, |ws, _| ws.is_open(Screen::NewProject)));
    let name = |s: &Value| {
        s.get("project").and_then(|p| p.get("name")).and_then(|v| v.as_text().map(str::to_string)).unwrap_or_default()
    };
    wait_until(&ws, cx, |s| name(s) != "Demo");
    assert_ne!(name(&status(&ws, cx)), "Demo");
}

/// About: the environment is copied for a bug report; the manual opens from F1's part
#[gpui::test]
fn about_copies_the_environment(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::About, None);
    click(cx, "copy-env");
    let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
    assert!(text.contains("GPUI"));
    click(cx, "about-Manual");
}

/// Notices: dismissing one takes it from the queue
#[gpui::test]
fn notices_dismiss(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Notices, None);
    let core = core(&ws, cx);
    let first = core.update(cx, |c, _| c.port_mut().notices().first().map(|n| n.id)).expect("the fake has a notice");
    click(cx, &format!("dismiss-{first}"));
    let left = core.update(cx, |c, _| c.port_mut().notices().iter().any(|n| n.id == first));
    assert!(!left);
}

// --- MIDI mapping and automation modes (I-21, agent A) ------------------------------------------------------------

/// The workspace on a fake whose outside world (MIDI, the engine's reports) the test plays
fn open_outside(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext, lm_port_fake::Outside) {
    let fake =
        FakePort::new(Config { tracks: 12, slow_delay: None, slow_track: usize::MAX }).expect("the demo project");
    let outside = fake.outside();
    let prefs = fake.preferences().clone();
    cx.update(|cx| workspace::start(cx, &prefs));
    let mut p: Option<Box<dyn CorePort>> = Some(Box::new(fake));
    let (ws, vcx) = cx.add_window_view(move |window, cx| Workspace::new(p.take().unwrap(), window, cx));
    vcx.run_until_parked();
    (ws, vcx, outside)
}

fn mappings(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<lm_port::Mapping> {
    let core = core(ws, cx);
    core.read_with(cx, |c, _| crate::ui::midi::mappings(c.port(), lm_port::MapScope::Project))
}

const CC7: lm_port::MidiMessage = lm_port::MidiMessage::Cc { number: 7 };

/// "MIDI learn" in a knob's menu: the banner asks for a control, the first one moved makes the mapping (to the
/// knob's parameter); "Remove MIDI mapping" takes it away
#[gpui::test]
fn learning_from_a_knobs_menu(cx: &mut TestAppContext) {
    let (ws, cx, outside) = open_outside(cx);
    show(&ws, cx, Page::Mixing);
    let id = some_channel(&ws, cx);
    select_strip(&ws, cx, id);
    let knob = center(cx, &format!("rk-{}-eq-high", id.number));
    button_at(cx, knob, MouseButton::Right, 1);
    click(cx, "menu-midi-learn");
    settle(cx);
    assert!(drawn(cx, "learning"), "the banner asks for a control");
    outside.midi("Controlador 1", 0, CC7, 64);
    settle(cx);
    assert!(!drawn(cx, "learning"), "learned");
    let maps = mappings(&ws, cx);
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0].target, crate::ui::midi::param(id, "rack.eq.high"));
    button_at(cx, knob, MouseButton::Right, 1);
    click(cx, "menu-midi-unmap");
    assert!(mappings(&ws, cx).is_empty(), "unmapped");
    // a switch maps its field; Cancel ends a learn with nothing made
    let mute = center(cx, &format!("mute-{}", id.number));
    button_at(cx, mute, MouseButton::Right, 1);
    click(cx, "menu-midi-learn");
    settle(cx);
    click(cx, "learning-cancel");
    settle(cx);
    assert!(!drawn(cx, "learning"));
    button_at(cx, mute, MouseButton::Right, 1);
    click(cx, "menu-midi-learn");
    outside.midi("Controlador 1", 0, lm_port::MidiMessage::Note { key: 40 }, 127);
    settle(cx);
    assert_eq!(mappings(&ws, cx)[0].target, crate::ui::midi::field(id, &["mute"]));
}

/// The binding window: learn for this project, change the mode, remove
#[gpui::test]
fn the_binding_window(cx: &mut TestAppContext) {
    let (ws, cx, outside) = open_outside(cx);
    show(&ws, cx, Page::Mixing);
    let id = some_channel(&ws, cx);
    select_strip(&ws, cx, id);
    let fader = center(cx, &format!("fader-{}", id.number));
    button_at(cx, fader, MouseButton::Right, 1);
    click(cx, "menu-midi-binding");
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::MidiBinding)));
    click(cx, "binding-learn");
    outside.midi("Controlador 1", 2, CC7, 10);
    settle(cx);
    assert_eq!(mappings(&ws, cx)[0].target, crate::ui::midi::param(id, "volume"));
    click(cx, "binding-0-mode");
    assert_eq!(map_text(&ws, cx, &["transform", "mode"]), "twos", "absolute → twos");
    click(cx, "binding-0-remove");
    assert!(mappings(&ws, cx).is_empty());
}

/// A text of the project's first mapping, by key path
fn map_text(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &[&str]) -> String {
    let r = root(ws, cx);
    let m = r.get("midiMap").and_then(|l| l.items().next().cloned()).unwrap_or_default();
    model::at(&m, path).and_then(Value::as_text).unwrap_or("").to_string()
}

/// The arrangement's volume knob has the same menu: reset, and the MIDI items
#[gpui::test]
fn the_track_knob_menu(cx: &mut TestAppContext) {
    let (ws, cx, outside) = open_outside(cx);
    let t = model::tracks(&root(&ws, cx))[1].id;
    drag(cx, "tknob-1", 0.0, -30.0);
    let p = center(cx, "tknob-1");
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "knob-menu-reset");
    assert_eq!(track_volume(&ws, cx, 1), 1.0);
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "menu-midi-learn");
    outside.midi("Controlador 1", 0, CC7, 1);
    settle(cx);
    assert_eq!(mappings(&ws, cx)[0].target, crate::ui::midi::param(t, "volume"));
}

/// The MIDI maps page lists the mappings (search, in-place edits, remove) and shows the activity
#[gpui::test]
fn the_midi_maps_page(cx: &mut TestAppContext) {
    let (ws, cx, outside) = open_outside(cx);
    let t = model::tracks(&root(&ws, cx))[1].id;
    let core = core(&ws, cx);
    core.update(cx, |c, _| {
        c.port_mut().midi_learn(crate::ui::midi::param(t, "volume"), lm_port::MapScope::Project).ok()
    });
    outside.midi("Controlador 1", 0, CC7, 5);
    outside.midi("Teclado", 0, lm_port::MidiMessage::Note { key: 60 }, 100);
    settle(cx);
    keys(cx, "secondary-3");
    assert_eq!(ws.read_with(cx, |ws, _| ws.page), Page::MidiMaps);
    assert!(drawn(cx, "map-0"), "the mapping is listed");
    assert!(!drawn(cx, "map-1"));
    let fb = mappings(&ws, cx)[0].feedback;
    click(cx, "map-0-feedback");
    assert_ne!(mappings(&ws, cx)[0].feedback, fb, "feedback toggled in place");
    click(cx, "map-0-takeover");
    assert_eq!(map_text(&ws, cx, &["transform", "takeover"]), "scale", "pickup → scale");
    click(cx, "map-0-remove");
    assert!(!drawn(cx, "map-0"), "removed");
    assert!(drawn(cx, "midi-activity"));
}

/// A mapped interface action arrives through the port's feed and runs as if its key was pressed
#[gpui::test]
fn a_mapped_interface_action_runs(cx: &mut TestAppContext) {
    let (ws, cx, outside) = open_outside(cx);
    let core = core(&ws, cx);
    let target = lm_port::MapTarget::Action { id: "view.mixing".into(), params: Value::NULL, value_into: None };
    core.update(cx, |c, _| c.port_mut().midi_learn(target, lm_port::MapScope::Machine).ok());
    outside.midi("Controlador 1", 0, lm_port::MidiMessage::Note { key: 36 }, 127);
    settle(cx);
    assert_eq!(ws.read_with(cx, |ws, _| ws.page), Page::Composition, "learning took the first one");
    outside.midi("Controlador 1", 0, lm_port::MidiMessage::Note { key: 36 }, 127);
    settle(cx);
    settle(cx);
    assert_eq!(ws.read_with(cx, |ws, _| ws.page), Page::Mixing, "the mapped note shows the mixing page");
}

/// The selection goes to the interface document, which the selection-following MIDI targets read (I-21.2)
#[gpui::test]
fn the_selection_is_in_the_interface_document(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    show(&ws, cx, Page::Mixing);
    let id = some_channel(&ws, cx);
    select_strip(&ws, cx, id);
    settle(cx);
    let core = core(&ws, cx);
    let sel = |cx: &mut VisualTestContext, k: &str| {
        core.read_with(cx, |c, _| c.port().ui().get("selection").and_then(|s| s.get(k)).cloned())
    };
    assert_eq!(sel(cx, "channel"), Some(Value::text(id.to_string())));
    show(&ws, cx, Page::Composition);
    let (item, _) = pick_item(&ws, cx, 2);
    let _ = item;
    settle(cx);
    let t = model::tracks(&root(&ws, cx))[2].id;
    assert_eq!(sel(cx, "track"), Some(Value::text(t.to_string())));
}

/// Lanes: the mode button cycles the lane's mode; "the hand won" shows until re-armed, per lane and globally; the
/// global automation button; a double click opens the lane in the automation editor; the lane's menu thins it
#[gpui::test]
fn lane_modes_hand_won_and_rearm(cx: &mut TestAppContext) {
    let (ws, cx, outside) = open_outside(cx);
    let tr2 = model::tracks(&root(&ws, cx))[2].clone();
    let lane = tr2.automation[0].id;
    click(cx, "lanes-2");
    let sel = format!("lane-mode-{}", lane.number);
    click(cx, &sel);
    assert_eq!(&*model::tracks(&root(&ws, cx))[2].automation[0].mode, "touch");
    outside.hand_wins(tr2.id, "volume");
    settle(cx);
    assert!(drawn(cx, &format!("rearm-{}", lane.number)), "the hand won, on the lane");
    assert!(drawn(cx, "rearm-all"), "and in the transport");
    click(cx, &format!("rearm-{}", lane.number));
    settle(cx);
    assert!(!drawn(cx, "rearm-all"), "re-armed");
    click(cx, "automation-mode");
    assert_eq!(model::at(&root(&ws, cx), &["transport", "automationMode"]).and_then(Value::as_text), Some("off"));
    // the lane's menu thins it; a double click opens the editor on it
    let n = model::tracks(&root(&ws, cx))[2].automation[0].points.len();
    let p = point(center(cx, "timeline").x, center(cx, &sel).y);
    button_at(cx, p, MouseButton::Right, 1);
    click(cx, "lane-menu-thin");
    assert!(model::tracks(&root(&ws, cx))[2].automation[0].points.len() <= n);
    button_at(cx, p, MouseButton::Left, 2);
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::Automation)), "the automation editor on the lane");
}

/// Moving an item moves the automation under it, in one undo step (I-21.5)
#[gpui::test]
fn moving_an_item_takes_its_automation(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    // the first point under the item
    let start = model::tracks(&root(&ws, cx))[2].items[0].start;
    let i = model::tracks(&root(&ws, cx))[2].automation[0].points.iter().position(|p| p.0 >= start).unwrap();
    let first_point = |cx: &mut VisualTestContext| model::tracks(&root(&ws, cx))[2].automation[0].points[i].0;
    let before = first_point(cx);
    let (_, p) = pick_item(&ws, cx, 2);
    drag_at(cx, p, 120.0, 0.0, Modifiers::none());
    assert!(first_point(cx) > before, "the automation moved with the item");
    undo(cx);
    assert_eq!(first_point(cx), before, "one undo step");
}

// --- agent D: the ruler's markers and regions (F-REG-011, F-REG-012) ---------------------------------------------

fn marks_point(cx: &mut VisualTestContext, ws: &Entity<Workspace>, tick: i64, lane: usize) -> Point<Pixels> {
    let b = bounds(cx, "marks");
    let v = ws.read_with(cx, |ws, cx| ws.composition.read(cx).view_for_tests());
    point(b.origin.x + px(v.x(tick as f64)), b.origin.y + px(crate::pages::marks::LANE_H * (lane as f32 + 0.5)))
}

#[gpui::test]
fn markers_keys_and_jumps(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let before = model::markers(&root(&ws, cx)).len();
    click(cx, "ruler");
    settle(cx);
    // the page's keys: a click in the arrangement gives it the focus
    let r = bounds(cx, "ruler");
    cx.simulate_click(point(r.origin.x + px(40.), r.origin.y + px(400.)), Modifiers::none());
    cx.run_until_parked();
    keys(cx, "m");
    assert_eq!(model::markers(&root(&ws, cx)).len(), before + 1, "M adds a marker at the cursor");
    keys(cx, "2");
    settle(cx);
    let second = model::markers(&root(&ws, cx))[1].tick;
    let core = core(&ws, cx);
    let at = core.read_with(cx, |c, _| {
        let p = c.port();
        p.tempo_map().tick_at(p.telemetry().position.samples)
    });
    assert_eq!(at, second, "2 goes to the second marker");
}

#[gpui::test]
fn a_click_on_a_region_sets_the_loop_and_a_drag_moves_a_marker(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let r = model::regions(&root(&ws, cx))[0].clone();
    let p = marks_point(cx, &ws, r.from + 480, 1);
    cx.simulate_click(p, Modifiers::none());
    cx.run_until_parked();
    let song = model::song(&root(&ws, cx));
    assert!(song.loop_on);
    assert_eq!(song.loop_range, (r.from, r.to));
    let m = model::markers(&root(&ws, cx))[1].clone();
    let p = marks_point(cx, &ws, m.tick, 0);
    drag_at(cx, p, 60.0, 0.0, Modifiers::none());
    let moved = model::markers(&root(&ws, cx)).into_iter().find(|x| x.id == m.id).unwrap().tick;
    assert!(moved > m.tick, "the marker moved: {moved}");
    undo(cx);
    let back = model::markers(&root(&ws, cx)).into_iter().find(|x| x.id == m.id).unwrap().tick;
    assert_eq!(back, m.tick, "the drag is one undo step");
}

#[gpui::test]
fn export_between_markers(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    open_part(&ws, cx, Screen::Export, None);
    click(cx, "what-2");
    click(cx, "marker-span-1");
    assert!(cx.debug_bounds("marker-span-0").is_some());
}

/// The docks' splitters (agent B): dragging resizes the editors' dock, and the size is kept in the interface document
#[gpui::test]
fn dock_splitter_resizes_and_is_remembered(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    ws.update_in(cx, |ws, window, cx| ws.open(Screen::PianoRoll, None, window, cx));
    cx.run_until_parked();
    let before = ws.read_with(cx, |ws, _| ws.docks.bottom);
    drag(cx, "splitter-bottom", 0.0, -60.0);
    let after = ws.read_with(cx, |ws, _| ws.docks.bottom);
    assert!((after - (before + 60.0)).abs() < 1.0, "{before} -> {after}");
    let core = core(&ws, cx);
    let saved = core
        .read_with(cx, |core, _| core.port().ui().get("docks").and_then(|d| d.get("bottom")).and_then(Value::as_int));
    assert_eq!(saved, Some(after.round() as i64), "kept in the interface document");
}

fn track_node(ws: &Entity<Workspace>, cx: &mut VisualTestContext, id: NodeId) -> Value {
    model::list(&root(ws, cx), "tracks").find(|t| model::id_of(t) == Some(id)).cloned().unwrap()
}

fn popup_items(cx: &mut VisualTestContext) -> usize {
    (0..40).take_while(|k| cx.debug_bounds(lm_port::intern(&format!("popup-item-{k}"))).is_some()).count()
}

/// The MIDI tab (F-IJN-017): the base velocity, the output to a device and its channel, the input's channel
#[gpui::test]
fn instrument_midi_tab(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (track, _) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    click(cx, "iw-tab-Midi");
    click(cx, "iw-base-velocity-plus");
    assert_eq!(track_node(&ws, cx, track).get("baseVelocity").and_then(Value::as_int), Some(64));
    click(cx, "iw-midi-out-on");
    assert!(track_node(&ws, cx, track).get("midiOutput").is_some_and(|o| !o.is_null()));
    click(cx, "iw-midi-out-channel-plus");
    let ch = track_node(&ws, cx, track).get("midiOutput").and_then(|o| o.get("channel")).and_then(Value::as_int);
    assert_eq!(ch, Some(1));
    click(cx, "iw-midi-in-channel-plus");
    let ch = track_node(&ws, cx, track).get("midiInput").and_then(|o| o.get("channel")).and_then(Value::as_int);
    assert_eq!(ch, Some(0), "from all (-1) to the first channel");
    undo(cx);
    click(cx, "iw-midi-out-on");
    let out = track_node(&ws, cx, track).get("midiOutput").cloned().unwrap();
    assert_eq!(out.get("on").and_then(Value::as_bool), Some(false), "off, the settings kept");
    assert_eq!(out.get("channel").and_then(Value::as_int), Some(1));
}

/// The CC rack (F-IJN-025): its switch, and a controller's knob as one gesture on `track.cc.cN`, a whole number
#[gpui::test]
fn instrument_cc_rack(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (track, _) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    click(cx, "iw-tab-Midi");
    click(cx, "iw-cc-rack-on");
    let on = track_node(&ws, cx, track).get("ccRack").and_then(|r| r.get("on")).and_then(Value::as_bool);
    assert_eq!(on, Some(true));
    drag(cx, &format!("ctl-{track}-cc.7"), 0.0, -100.0);
    let cc = |ws: &Entity<Workspace>, cx: &mut VisualTestContext| {
        track_node(ws, cx, track).get("cc").and_then(|c| c.get("c7")).and_then(Value::as_real)
    };
    assert_eq!(cc(&ws, cx), Some(64.0), "half the travel up from 0, rounded");
    undo(cx);
    assert!(cc(&ws, cx).is_none_or(|v| v == 0.0), "one undo step");
}

/// The Tuning tab (F-IJN-019..021): the tuning on, the keys, and the files cleared
#[gpui::test]
fn instrument_tuning_tab(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (track, _) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    click(cx, "iw-tab-Tuning");
    click(cx, "iw-tuning-on");
    let tuning = track_node(&ws, cx, track).get("tuning").cloned().unwrap();
    assert_eq!(tuning.get("on").and_then(Value::as_bool), Some(true));
    click(cx, "iw-keys-first-plus");
    assert_eq!(track_node(&ws, cx, track).get("keys").and_then(|k| k.get("first")).and_then(Value::as_int), Some(1));
    click(cx, "iw-tuning-clear");
    assert!(track_node(&ws, cx, track).get("tuning").is_none_or(Value::is_null), "cleared");
}

/// MIDI learn from a control's menu (I-21.2): the port starts learning that control
#[gpui::test]
fn instrument_midi_learn(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let (track, node) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    click(cx, "iw-tab-Envelopes");
    let p = center(cx, &format!("ctl-{node}-volAttack"));
    button_at(cx, p, MouseButton::Right, 1);
    let n = popup_items(cx);
    // default, copy, paste, automate, MIDI learn, (remove), binding, ...
    assert!(n >= 6);
    click(cx, "popup-item-4");
    let core = core(&ws, cx);
    assert!(core.read_with(cx, |c, _| c.port().learning().is_some()), "learning the knob");
}

/// The sidechain source of an insert that has the port: a send from another channel, never a loop
#[gpui::test]
fn effect_sidechain_source(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let chs = model::channels(&root(&ws, cx));
    let ch = chs.iter().find(|c| c.inserts.iter().any(|x| &*x.plugin == "compressor")).expect("a compressor").clone();
    let at = ch.inserts.iter().position(|x| &*x.plugin == "compressor").unwrap();
    let ins = ch.inserts[at].id;
    open_screen(&ws, cx, Screen::Effect, Some(ch.id));
    let p = center(cx, &format!("ef-card-{at}"));
    button_at(cx, p, MouseButton::Right, 1);
    // move up, move down, replace with, load preset, save preset, then the sidechain source (the compressor has it)
    click(cx, "popup-item-5");
    assert!(popup_items(cx) >= 2, "None and the sources");
    let before: usize =
        model::channels(&root(&ws, cx)).iter().map(|c| c.sends.iter().filter(|s| s.to == ins).count()).sum();
    // pick the first enabled source
    for k in 1..popup_items(cx) {
        click(cx, &format!("popup-item-{k}"));
        let after: usize =
            model::channels(&root(&ws, cx)).iter().map(|c| c.sends.iter().filter(|s| s.to == ins).count()).sum();
        if after > before {
            assert_eq!(after, 1, "one source");
            return;
        }
    }
    panic!("no source could be chosen");
}

/// "+ Instrument track" adds a TripleOscillator track at the end, "+ Audio track" an audio one; a double click on
/// the instrument track's row makes a MIDI item a bar long there and opens it in the Piano Roll
#[gpui::test]
fn adding_tracks_and_a_clip(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let n = model::tracks(&root(&ws, cx)).len();
    let comp = ws.read_with(cx, |ws, _| ws.composition.clone());
    // the last row is in view
    comp.update(cx, |c, cx| {
        c.track_h = 24.0;
        c.view.scroll_y = 1e6;
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "add-audio-track");
    click(cx, "add-instrument-track");
    let tracks = model::tracks(&root(&ws, cx));
    assert_eq!(tracks.len(), n + 2);
    assert!(tracks[n].instrument.is_none(), "an audio track");
    let t = tracks[n + 1].clone();
    assert_eq!(t.instrument.as_ref().map(|i| i.1.to_string()), Some("tripleoscillator".into()));
    cx.run_until_parked();
    let row = comp.read_with(cx, |c, _| c.track_row(t.id)).expect("the new track is drawn");
    let b = bounds(cx, "timeline");
    let p = point(b.origin.x + px(200.0), b.origin.y + px(row.0 + row.1 / 2.0));
    button_at(cx, p, MouseButton::Left, 2);
    let items = model::tracks(&root(&ws, cx))[n + 1].items.clone();
    assert_eq!(items.len(), 1, "a double click made an item");
    assert!(!items[0].audio && items[0].length > 0);
    assert!(ws.read_with(cx, |ws, _| ws.is_open(Screen::PianoRoll)), "opened in the Piano Roll");
}

/// The W/D knob of an effect card (the legacy EffectView's): a drag is one step on the insert's `wet`
#[gpui::test]
fn effect_wet_knob(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let ch = model::channels(&root(&ws, cx)).into_iter().find(|c| !c.inserts.is_empty()).unwrap();
    let ins = ch.inserts[0].id;
    open_screen(&ws, cx, Screen::Effect, Some(ch.id));
    drag(cx, &format!("ef-wet-{}", ins.number), 0.0, 50.0);
    let core = core(&ws, cx);
    let wet = core.read_with(cx, |c, _| c.port().find(ins).and_then(|n| n.get("wet").and_then(Value::as_real)));
    assert_eq!(wet, Some(0.75), "50 px of 200 down from all wet");
    undo(cx);
    let wet = core.read_with(cx, |c, _| c.port().find(ins).and_then(|n| n.get("wet").and_then(Value::as_real)));
    assert!(wet.is_none_or(|w| w == 1.0), "one undo step");
}

// --- agent D: files in and out (F-ARR-011, F-PRJ-019/020/021) --------------------------------------------------

fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lm-gui-files-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

fn a_midi_file() -> Vec<u8> {
    use lm_smf::{Event, Smf, Timed, Track};
    let t = |tick, event| Timed { tick, event };
    lm_smf::write(&Smf {
        format: 1,
        ppq: 480,
        tracks: vec![Track {
            events: vec![
                t(0, Event::TrackName("Melodia".into())),
                t(0, Event::NoteOn { channel: 0, key: 72, velocity: 100 }),
                t(480, Event::NoteOff { channel: 0, key: 72, velocity: 0 }),
            ],
        }],
    })
}

/// A file dropped on the arrangement (F-ARR-011): an audio file becomes a new audio track at the bar under the pointer
#[gpui::test]
fn dropping_an_audio_file_makes_a_track(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let n = model::tracks(&root(&ws, cx)).len();
    let file = temp_file("voz.wav", b"RIFF....WAVE");
    let r = bounds(cx, "ruler");
    let at = point(r.origin.x + px(200.), r.origin.y + px(300.));
    cx.simulate_event(gpui::FileDropEvent::Entered { position: at, paths: gpui::ExternalPaths(vec![file].into()) });
    cx.simulate_event(gpui::FileDropEvent::Submit { position: at });
    for _ in 0..20 {
        settle(cx);
    }
    let tracks = model::tracks(&root(&ws, cx));
    assert_eq!(tracks.len(), n + 1);
    let t = tracks.last().unwrap();
    assert_eq!(&*t.name, "voz");
    assert!(t.items[0].audio);
}

/// Import and Export MIDI by a path (the dialog of a remote core; locally the system's dialog is used)
#[gpui::test]
fn importing_and_exporting_midi(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let n = model::tracks(&root(&ws, cx)).len();
    let file = temp_file("melodia.mid", &a_midi_file());
    open_part(&ws, cx, Screen::Project, Some("import"));
    cx.simulate_input(&file.display().to_string());
    click(cx, "path-go");
    settle(cx);
    let tracks = model::tracks(&root(&ws, cx));
    assert_eq!(tracks.len(), n + 1);
    assert_eq!(&*tracks.last().unwrap().name, "Melodia");
    let out = file.with_file_name("saida.mid");
    open_part(&ws, cx, Screen::Project, Some("export_midi"));
    cx.simulate_input(&out.display().to_string());
    click(cx, "path-go");
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(20));
        settle(cx);
    }
    assert!(lm_smf::read(&std::fs::read(&out).unwrap()).is_ok());
}

// --- live notes from the interface (P2, I-15) ----------------------------------------------------------------------

/// The workspace on a fake whose live-note log the test reads
fn open_live(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext, lm_port_fake::LiveLog) {
    let fake =
        FakePort::new(Config { tracks: 12, slow_delay: None, slow_track: usize::MAX }).expect("the demo project");
    let log = fake.live_log();
    let prefs = fake.preferences().clone();
    cx.update(|cx| workspace::start(cx, &prefs));
    let mut p: Option<Box<dyn CorePort>> = Some(Box::new(fake));
    let (ws, vcx) = cx.add_window_view(move |window, cx| Workspace::new(p.take().unwrap(), window, cx));
    vcx.run_until_parked();
    (ws, vcx, log)
}

/// The instrument window's test keyboard: a press sounds the key, the release (even outside the keys) ends it
#[gpui::test]
fn instrument_keyboard_plays_and_releases(cx: &mut TestAppContext) {
    let (ws, cx, log) = open_live(cx);
    let (track, _) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    click(cx, "iw-keyboard");
    let p = center(cx, "iw-keys");
    cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(log.held().len(), 1, "one key sounds while the button is down");
    assert_eq!(log.held()[0].0, track);
    cx.simulate_mouse_up(point(p.x, p.y + px(400.)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(log.held().is_empty(), "no note sticks after the release");
}

/// The computer keyboard is a piano in the instrument window (the legacy's layout): Z is the lowest key
#[gpui::test]
fn instrument_computer_keyboard(cx: &mut TestAppContext) {
    let (ws, cx, log) = open_live(cx);
    let (track, _) = triple(&ws, cx);
    open_screen(&ws, cx, Screen::Instrument, Some(track));
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: gpui::Keystroke::parse("z").expect("a keystroke"),
        is_held: false,
        prefer_character_input: false,
    });
    cx.run_until_parked();
    assert_eq!(log.held(), vec![(track, 36)]);
    cx.simulate_event(gpui::KeyUpEvent { keystroke: gpui::Keystroke::parse("z").expect("a keystroke") });
    cx.run_until_parked();
    assert!(log.held().is_empty());
}

/// The kit picture of AVL Drums plays a piece on the click and ends it on the release
#[gpui::test]
fn kit_picture_plays(cx: &mut TestAppContext) {
    let (ws, cx, log) = open_live(cx);
    let t = model::tracks(&root(&ws, cx))
        .into_iter()
        .find(|t| t.instrument.as_ref().is_some_and(|(_, p)| &**p == "avldrums"));
    let Some(t) = t else { return };
    open_screen(&ws, cx, Screen::Instrument, Some(t.id));
    let p = center(cx, "iw-kit");
    cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(log.held().len(), 1);
    cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(log.held().is_empty());
}

/// The piano roll's keys sound the clip's instrument on the press and end it on the release
#[gpui::test]
fn piano_roll_keys_sound(cx: &mut TestAppContext) {
    let (ws, cx, log) = open_live(cx);
    open_screen(&ws, cx, Screen::PianoRoll, None);
    let p = center(cx, "pr-keys");
    cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(log.held().len(), 1, "the key under the mouse sounds");
    cx.simulate_mouse_up(point(p.x + px(300.), p.y), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(log.held().is_empty(), "no note sticks");
}

/// A note drawn in the grid is heard while the button is down
#[gpui::test]
fn piano_roll_draw_auditions(cx: &mut TestAppContext) {
    let (ws, cx, log) = open_live(cx);
    open_screen(&ws, cx, Screen::PianoRoll, None);
    // the grid is to the right of the keys, on the same rows
    let k = center(cx, "pr-keys");
    let p = point(k.x + px(300.), k.y);
    cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(log.held().len(), 1);
    cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(log.held().is_empty());
}

/// A mouse button released anywhere closes the gestures the views left open (a drag released outside its element)
#[gpui::test]
fn a_release_anywhere_closes_the_open_gestures(cx: &mut TestAppContext) {
    use lm_gui_plugins::PortHost;
    let (ws, cx) = open(cx);
    let core = core(&ws, cx);
    let g = core.update(cx, |c, _| c.begin_gesture(lm_port::Label::new("test")));
    assert!(core.update(cx, |c, _| PortHost::gesture_is_open(c, g)));
    cx.simulate_mouse_down(point(px(2.), px(2.)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(point(px(2.), px(2.)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(!core.update(cx, |c, _| PortHost::gesture_is_open(c, g)), "the release closed it");
    // a stale drag writes nothing
    let wrote = core.update(cx, |c, _| c.gesture_apply(g, Vec::new()));
    assert!(!wrote);
}

/// The error of a call to the port is shown as a short notice, not dropped
#[gpui::test]
fn a_failed_call_shows_a_notice(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let core = core(&ws, cx);
    let done = core.update(cx, |c, _| c.report(|pt| pt.run_action("no.such.action".into(), Value::NULL)));
    assert!(done.is_none());
    // the frame collects it as a floating notice
    cx.update(|_, _| {});
    ws.update(cx, |_, _| {});
    settle(cx);
    let toasts = ws.read_with(cx, |ws, _| ws.toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>());
    assert!(toasts.iter().any(|t| t.contains("port.action.unknown")), "the error shows: {toasts:?}");
}

/// The requests nobody takes are not kept
#[gpui::test]
fn a_forgotten_request_leaves_no_answer(cx: &mut TestAppContext) {
    let (ws, cx) = open(cx);
    let core = core(&ws, cx);
    core.update(cx, |c, _| {
        c.expect(4_000_000);
        assert_eq!(c.waiting_requests(), 1);
        c.forget_request(4_000_000);
        assert_eq!(c.waiting_requests(), 0);
    });
}

mod keymap_tests;
