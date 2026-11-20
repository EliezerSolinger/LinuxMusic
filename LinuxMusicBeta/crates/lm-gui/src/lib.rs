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
mod model;
mod pages;
mod screens;
mod tiles;
mod ui;
mod view;
mod workspace;

use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, point, px, size};
use gpui_component::Root;
use lm_port::CorePort;

/// Starts the interface on a port, in a GPUI application, and runs until the user quits
pub fn run(make_port: impl FnOnce() -> Box<dyn CorePort> + 'static) {
    gpui_platform::application().run(move |cx: &mut App| {
        let port = make_port();
        workspace::start(cx, port.preferences());
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(view::WINDOW_W), px(view::WINDOW_H)));
        let opened = cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |window, cx| {
                let ws = cx.new(|cx| workspace::Workspace::new(port, window, cx));
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

#[cfg(test)]
mod tests;
