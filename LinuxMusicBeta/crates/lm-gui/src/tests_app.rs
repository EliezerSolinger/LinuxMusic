//! The real program, driven without a window: the interface on lm-session's `LocalPort` (the real document, the
//! real engine on the Dummy device, the real TripleOscillator), through the interface's own gestures and keys.
//! A track is added, a clip made and notes drawn in the Piano Roll; playing moves the meters; the project is saved,
//! opened again from its folder, and the tree compared.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    Entity, Modifiers, MouseButton, MouseDownEvent, Pixels, Point, TestAppContext, VisualTestContext, point, px,
};
use lm_port::{CorePort, Loading, Target, Value};
use lm_session::{LocalPort, Session, SessionConfig};

use crate::actions::keys_for_platform;
use crate::core::Core;
use crate::model;
use crate::screens::Screen;
use crate::screens::piano_roll::PianoRoll;
use crate::workspace::{self, Workspace};

fn folder(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lm-gui-app-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// The interface on a real session, with a project made in `song`
fn open_real<'a>(cx: &'a mut TestAppContext, song: &std::path::Path) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let config = SessionConfig::headless(song.with_file_name("untitled")).expect("config");
    let mut session = Session::new(config).expect("a session");
    session.create(song.to_path_buf()).expect("a new project");
    let port = LocalPort::new(session);
    let prefs = port.preferences().clone();
    cx.update(|cx| workspace::start(cx, &prefs));
    let mut p: Option<Box<dyn CorePort>> = Some(Box::new(port));
    let (ws, vcx) = cx.add_window_view(move |window, cx| Workspace::new(p.take().unwrap(), window, cx));
    vcx.run_until_parked();
    (ws, vcx)
}

fn core(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<Core> {
    ws.read_with(cx, |ws, _| ws.core.clone())
}

fn root(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Value {
    let core = core(ws, cx);
    core.read_with(cx, |c, _| c.port().root().clone())
}

/// Lets real time pass for the engine while GPUI's clock runs the pump
fn run_for(cx: &mut VisualTestContext, wall: Duration) {
    let end = Instant::now() + wall;
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(10));
        cx.run_until_parked();
    }
}

fn center(cx: &mut VisualTestContext, selector: &'static str) -> Point<Pixels> {
    cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is drawn")).center()
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let p = center(cx, selector);
    cx.simulate_click(p, Modifiers::none());
    cx.run_until_parked();
}

fn keys(cx: &mut VisualTestContext, k: &str) {
    cx.simulate_keystrokes(&keys_for_platform(k));
    cx.run_until_parked();
}

fn the_project(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Value {
    root(ws, cx).without("meta")
}

#[gpui::test]
fn the_real_program_makes_music_saves_and_reopens(cx: &mut TestAppContext) {
    let song = folder("song");
    let (ws, cx) = open_real(cx, &song);

    // a TripleOscillator track, at the end of the arrangement
    let comp = ws.read_with(cx, |ws, _| ws.composition.clone());
    click(cx, "add-instrument-track");
    let tracks = model::tracks(&root(&ws, cx));
    let t = tracks.last().expect("a track").clone();
    assert_eq!(t.instrument.as_ref().map(|i| i.1.to_string()), Some("tripleoscillator".into()), "{tracks:?}");

    // a double click on its row: a clip, opened in the Piano Roll
    cx.run_until_parked();
    let row = comp.read_with(cx, |c, _| c.track_row(t.id)).expect("the track's row is drawn");
    let tl = cx.debug_bounds("timeline").expect("the timeline");
    let p = point(tl.origin.x + px(20.0), tl.origin.y + px(row.0 + row.1 / 2.0));
    cx.simulate_event(MouseDownEvent {
        position: p,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    let item = model::tracks(&root(&ws, cx)).last().unwrap().items.first().expect("a clip").clone();
    assert!(!item.audio);
    let pr: Entity<PianoRoll> = ws
        .read_with(cx, |ws, _| ws.screen_view(Screen::PianoRoll))
        .expect("the Piano Roll is open")
        .downcast()
        .expect("a Piano Roll");

    // four notes drawn with clicks, on the beats of the bar
    let grid_bounds = cx.debug_bounds("pr-grid").expect("the grid");
    let grid = grid_bounds.origin;
    let beat = model::ppq(&root(&ws, cx)) as f64;
    // four keys the grid shows, a few apart
    let shown: Vec<i32> = pr.read_with(cx, |pr, _| {
        (0..128)
            .rev()
            .filter(|k| {
                let y = pr.key_to_y(*k);
                y > 4.0 && y < grid_bounds.size.height.as_f32() - 20.0
            })
            .collect()
    });
    assert!(shown.len() >= 12, "the grid shows keys");
    let chord = [shown[2], shown[5], shown[8], shown[11]];
    for (k, key) in chord.into_iter().enumerate() {
        let at = pr.read_with(cx, |pr, _| {
            grid + point(px(pr.tick_to_x(beat * k as f64 + 10.0) + 1.0), px(pr.key_to_y(key) + pr.key_h / 2.0))
        });
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
    }
    let notes = pr.read_with(cx, |pr, cx| pr.clip(cx).map(|c| c.notes.len()).unwrap_or(0));
    assert_eq!(notes, 4, "four notes drawn");

    // playing through the Dummy device moves the meters: the Piano Roll closed, the arrangement in focus
    ws.update(cx, |ws, cx| ws.close(Screen::PianoRoll, cx));
    cx.run_until_parked();
    let ruler = cx.debug_bounds("ruler").expect("the ruler");
    cx.simulate_click(ruler.origin + point(px(1.0), px(4.0)), Modifiers::none());
    cx.run_until_parked();
    keys(cx, "space");
    let core = core(&ws, cx);
    let mut loudest = 0.0f32;
    let end = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < end {
        run_for(cx, Duration::from_millis(20));
        let peak = core.read_with(cx, |c, _| {
            c.port().telemetry().meters.values().flat_map(|m| m.peak.iter().copied()).fold(0.0f32, f32::max)
        });
        loudest = loudest.max(peak);
    }
    let playing = core.read_with(cx, |c, _| c.port().telemetry().position.playing);
    keys(cx, "shift-space");
    run_for(cx, Duration::from_millis(100));
    assert!(playing, "the transport ran");
    assert!(loudest > 0.0, "the meters moved");

    // saved, then opened again from its folder: the same project
    keys(cx, "secondary-s");
    run_for(cx, Duration::from_millis(300));
    assert!(!core.read_with(cx, |c, _| c.port().modified()), "saved");
    let before = the_project(&ws, cx);
    core.update(cx, |c, _| {
        let _ = c
            .port_mut()
            .run_action("project.open".into(), Value::object([("folder", Value::text(song.to_string_lossy()))]));
    });
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        run_for(cx, Duration::from_millis(50));
        if core.read_with(cx, |c, _| c.port().loading(&Target::Project)) == Loading::Ready {
            break;
        }
        assert!(Instant::now() < end, "the project opened");
    }
    let after = the_project(&ws, cx);
    assert_eq!(after, before, "the reopened project is the saved one");
    let _ = std::fs::remove_dir_all(song.parent().unwrap().join(format!("lm-gui-app-{}-untitled", std::process::id())));
    let _ = std::fs::remove_dir_all(&song);
}
