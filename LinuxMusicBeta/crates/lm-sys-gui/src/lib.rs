//! `lm-sys-gui`: native top-level windows for the plugin-host helper (interfaces.md, I-22: a third-party plugin opens
//! its own window), and the event pump that keeps them alive.
//!
//! - [`Window`]: an empty native window the helper makes for a plugin view to be attached to (a VST3 `IPlugView`, an
//!   embedded CLAP gui): macOS an `NSWindow` whose content view is the parent (`"cocoa"`); Linux an X11 window made
//!   through `libX11`, loaded at run time (`"x11"`); elsewhere none ([`supported`] is `false`).
//! - [`init`] and [`pump`]: the window system's loop, run by the helper's **main thread** between the control
//!   channel's messages (macOS: `NSApplication` with the accessory policy, no Dock icon, events dispatched with
//!   `nextEventMatchingMask`; X11: the connection polled and its events read). The audio thread is never involved.
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | A [`Window`] owns its native window (closed or destroyed in `Drop`). The `NSApplication`, the `libX11` handle and the X connection are process-wide and live until the process ends. |
//! | Which thread may call each function? | The main thread only (Cocoa requires it; X11 calls share one connection, which is not locked here): every function but [`supported`] and [`api`] checks nothing and documents it. The helper calls them from its main loop only. |
//! | Allocates, locks, does I/O? | Yes: the window system's own. Never on the audio thread. |
//! | Creates threads? | No. |
//! | Calls our code back? | No (events are read by [`pump`] and acted on there). |
//! | Tests | `tests/window.rs` makes, shows, resizes and drops a window where a window system exists (`LM_GUI_TESTS=1`), and checks the pump returns on time with none. |
//!
//! # Error keys
//!
//! `gui.unsupported`, `gui.init`, `gui.window`.

use std::time::Duration;

use lm_base::{Error, Result};

#[cfg(target_os = "macos")]
mod cocoa;
#[cfg(target_os = "macos")]
use cocoa as native;

#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "linux")]
use x11 as native;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod none;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use none as native;

/// Whether this platform has a window system the helper can use
pub fn supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux"))
}

/// The plugin formats' name for the window system: `"cocoa"`, `"x11"`, or `""`
pub fn api() -> &'static str {
    if cfg!(target_os = "macos") {
        "cocoa"
    } else if cfg!(target_os = "linux") {
        "x11"
    } else {
        ""
    }
}

/// Connects to the window system (once; later calls do nothing). Main thread
pub fn init() -> Result<()> {
    native::init()
}

/// Runs the window system's events for up to `timeout`, returning early when some arrived. Without windows it only
/// waits. Main thread
pub fn pump(timeout: Duration) {
    native::pump(timeout);
}

/// A native window, empty, to attach a plugin's view to
#[derive(Debug)]
pub struct Window {
    native: native::Native,
}

impl Window {
    /// Makes a hidden window with a client area of `width` × `height`. Main thread, after [`init`]
    pub fn new(title: &str, width: u32, height: u32) -> Result<Window> {
        if !supported() {
            return Err(Error::new("gui.unsupported"));
        }
        Ok(Window { native: native::Native::new(title, width, height)? })
    }

    /// What a plugin view is attached to: the content `NSView` (macOS) or the X11 window id (Linux)
    pub fn handle(&self) -> usize {
        self.native.handle()
    }

    pub fn show(&mut self) {
        self.native.show();
    }

    pub fn hide(&mut self) {
        self.native.hide();
    }

    /// The client area's new size
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.native.set_size(width, height);
    }

    /// The user closed the window since the last call (the window is hidden, still alive)
    pub fn take_closed(&mut self) -> bool {
        self.native.take_closed()
    }
}

// SAFETY: a `Window` is made, used and dropped on the helper's main thread only; it only travels inside the plugin
// object (which the audio thread locks for blocks, never touching the window)
unsafe impl Send for Window {}
