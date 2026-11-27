//! LinuxMusic's graphical interface, in GPUI, on the `CorePort` only (interfaces.md, I-15): it never names the
//! session, the document or the engine; whoever starts it hands it a port (the session's LocalPort, a client's
//! RemotePort, or the fake in the tests and the demo: `cargo run -p lm-gui --example demo`).
//!
//! Layout of the code (docs/prototipo-interface.md, section 8):
//! - `core`: the `Core` entity that owns the port and the 10 ms pump (H2);
//! - `workspace`: the shell (transport bar, page tabs, docks, modals, keys, asking before losing changes);
//! - `pages`: Composition and Mixing, separate pages, and the transport bar;
//! - `screens`: every other screen, one file each, registered in `screens/mod.rs`;
//! - `ui`: theme as data (`themes/*.txt`, dark and light), translation as data (`locales/*/*.txt`, `LM_LANG`),
//!   widgets, the loading component;
//! - `actions`: the action registry and the keybindings built from it;
//! - `model`: typed readings of the project tree; `view`: layout and number formats; `tiles`: the waveform cache.

#![forbid(unsafe_code)]

mod actions;
mod bench;
mod core;
mod keymap;
mod menus;
mod model;
mod pages;
mod screens;
mod tap;
mod tiles;
mod ui;
mod view;
mod workspace;

use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, point, px, size};
use gpui_component::Root;
use lm_port::CorePort;

/// How the program starts (the application binary's choices)
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The window's title
    pub title: Option<String>,
    /// Show the recent projects at the start (no project was given)
    pub recents: bool,
    /// The first run on this machine (no preferences file yet): Settings opens on its own (F-JAN-003)
    pub first_run: bool,
}

/// Starts the interface on a port, in a GPUI application, and runs until the user quits
pub fn run(make_port: impl FnOnce() -> Box<dyn CorePort> + 'static) {
    run_with(make_port, Options::default())
}

/// `run`, with the start's options
pub fn run_with(make_port: impl FnOnce() -> Box<dyn CorePort> + 'static, options: Options) {
    gpui_platform::application().run(move |cx: &mut App| {
        let port = make_port();
        if let Some(dir) = port.config_dir() {
            keymap::set_user_presets(keymap::presets::load_user(&dir));
        }
        workspace::start(cx, port.preferences());
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(view::WINDOW_W), px(view::WINDOW_H)));
        let titlebar =
            options.title.clone().map(|t| gpui::TitlebarOptions { title: Some(t.into()), ..Default::default() });
        let recents = options.recents;
        let first_run = options.first_run;
        let opened = cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), titlebar, ..Default::default() },
            |window, cx| {
                let ws = cx.new(|cx| workspace::Workspace::new(port, window, cx));
                if first_run {
                    ws.update(cx, |ws, cx| ws.open(screens::Screen::Settings, None, window, cx));
                } else if recents {
                    // no project was given: the recent projects, to open one (the Project screen's Open)
                    ws.update(cx, |ws, cx| ws.open(screens::Screen::Project, None, window, cx));
                }
                // the window's close button asks before losing changes, like Quit
                let weak = ws.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    weak.update(cx, |ws, cx| {
                        ws.request(workspace::Pending::Close, window, cx);
                        false
                    })
                    .unwrap_or(true)
                });
                cx.new(|cx| Root::new(ws, window, cx))
            },
        );
        if opened.is_ok() {
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            cx.activate(true);
        }
    });
}

// The tests drive the views by GPUI's debug selectors, which GPUI records only in builds with debug assertions
// (InteractiveElement::debug_selector): they run in the debug test builds, and a release test build leaves them
// out rather than fail on selectors it cannot see
#[cfg(all(test, debug_assertions))]
mod tests;
#[cfg(all(test, debug_assertions))]
mod tests_app;
