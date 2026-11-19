//! LinuxMusic's interface prototype in GPUI, against `fake-core` (interfaces.md, I-15). It grows into the real
//! `lm-gui`: every screen of the program, on fake data with the core's surface.
//!
//! Layout of the code (owners in docs/prototipo-interface.md):
//! - `core`: the `Core` entity that owns the `!Send` `Session` and the 10 ms pump (H2);
//! - `workspace`: the shell (transport bar, page tabs, docks, modals, keys);
//! - `pages`: the two pages, Composition and Mixing, and the transport bar;
//! - `screens`: every other screen, one file each, registered in `screens/mod.rs`;
//! - `ui`: the shared kit: theme as data (`themes/*.txt`), translation (`locales/*/*.txt`), widgets, the loading
//!   component;
//! - `tiles`: the waveform tile cache.

// The kit and the registry have pieces the screens of agents B, C and D will use
#![allow(dead_code)]

mod core;
mod pages;
mod screens;
mod tiles;
mod ui;
mod workspace;

use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, point, px, size};
use gpui_component::{Root, Theme, ThemeMode};
use gpui_platform::application;

fn main() {
    application().run(|cx: &mut App| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        workspace::bind_keys(cx);
        let bounds = Bounds::new(
            point(px(0.), px(0.)),
            size(px(fake_core::view::WINDOW_W), px(fake_core::view::WINDOW_H)),
        );
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |window, cx| {
                let ws = cx.new(|cx| workspace::Workspace::new(window, cx));
                cx.new(|cx| Root::new(ws, window, cx))
            },
        )
        .expect("open window");
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    //! GPUI's headless test harness drives the real views: no window server, a fake clock, simulated input.
    use std::time::Duration;

    use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point};

    use super::*;
    use crate::workspace::Workspace;

    fn open(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            workspace::bind_keys(cx);
        });
        cx.add_window_view(|window, cx| Workspace::new(window, cx))
    }

    fn drag(cx: &mut VisualTestContext, selector: &'static str, dy: f32) {
        let p = cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is drawn")).center();
        cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
        for k in 1..=10 {
            cx.simulate_mouse_move(point(p.x, p.y + px(dy * k as f32 / 10.0)), MouseButton::Left, Modifiers::none());
        }
        cx.simulate_mouse_up(point(p.x, p.y + px(dy)), MouseButton::Left, Modifiers::none());
    }

    /// A real drag on a header knob (track 1: track 0 is a folder, without a knob): begin, updates and commit of one gesture (I-3.7), one undo step, and the
    /// pump timer's diff rebuilding the label through the feed in the next frame.
    #[gpui::test]
    fn knob_drag_is_one_gesture(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx);
        drag(cx, "tknob-1", -50.0);
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        // 50 px up from 0 dB (75 % of travel) is 25 % more: +6 dB
        core.read_with(cx, |core, _| assert!((core.port().root().tracks[1].volume_db - 6.0).abs() < 1e-9));
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        core.update(cx, |core, _| {
            assert!(core.port_mut().undo().ok().is_some());
            assert_eq!(core.port().root().tracks[1].volume_db, 0.0, "the whole drag is one undo step");
        });
    }

    /// The mixing page's fader over the meter: a drag is one gesture, a double click goes back to 0 dB
    #[gpui::test]
    fn fader_drag_and_double_click(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx);
        cx.dispatch_action(workspace::ShowMixing);
        cx.run_until_parked();
        drag(cx, "fader-1", 60.0);
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        let after = core.read_with(cx, |core, _| core.port().root().channels[1].volume_db);
        assert!(after < -10.0, "the fader went down: {after}");
        let p = cx.debug_bounds("fader-1").unwrap().center();
        cx.simulate_event(gpui::MouseDownEvent { position: p, modifiers: Modifiers::none(), button: MouseButton::Left, click_count: 2, first_mouse: false });
        cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
        core.read_with(cx, |core, _| assert_eq!(core.port().root().channels[1].volume_db, 0.0, "double click: 0 dB"));
    }

    /// H2: the pump runs every 10 ms on GPUI's timer whether or not anything is drawn (fake clock)
    #[gpui::test]
    fn pump_runs_without_frames(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx);
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        let before = core.read_with(cx, |core, _| core.port().pumps());
        cx.executor().advance_clock(Duration::from_millis(1000));
        cx.run_until_parked();
        let after = core.read_with(cx, |core, _| core.port().pumps());
        assert!((95..=105).contains(&(after - before)), "{} pumps in 1 s", after - before);
    }

    /// Every screen opens and draws (and closes) without panicking
    #[gpui::test]
    fn every_screen_opens(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx);
        for s in screens::Screen::ALL {
            ws.update_in(cx, |ws, window, cx| ws.open(s, window, cx));
            cx.run_until_parked();
            ws.update(cx, |ws, cx| ws.close(s, cx));
        }
    }
}
