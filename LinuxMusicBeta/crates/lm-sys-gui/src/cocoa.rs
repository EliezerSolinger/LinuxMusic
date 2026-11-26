//! Cocoa through the Objective-C runtime (no crate: a few messages are all it takes). Main thread only.
use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lm_base::{Error, Result};

type Id = *mut c_void;

#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    static NSDefaultRunLoopMode: Id;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    w: f64,
    h: f64,
}

static STARTED: AtomicBool = AtomicBool::new(false);

fn class(name: &str) -> Id {
    let c = CString::new(name).unwrap_or_default();
    // SAFETY: a NUL-terminated class name
    unsafe { objc_getClass(c.as_ptr()) }
}

fn sel(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap_or_default();
    // SAFETY: a NUL-terminated selector name
    unsafe { sel_registerName(c.as_ptr()) }
}

// `objc_msgSend` is called through a function pointer of the exact type each message has (the Objective-C ABI on
// arm64 and x86_64 for these argument kinds: pointers, integers, doubles and small structs of doubles)

/// `[obj name]` returning an object
fn send0(obj: Id, name: &str) -> Id {
    // SAFETY: obj is an Objective-C object (or nil) and `name` a message taking no argument and returning an object
    unsafe {
        let f: unsafe extern "C" fn(Id, *mut c_void) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        f(obj, sel(name))
    }
}

/// `[obj name: object]`
fn send1(obj: Id, name: &str, a: Id) -> Id {
    // SAFETY: as `send0`, the message taking one object argument
    unsafe {
        let f: unsafe extern "C" fn(Id, *mut c_void, Id) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        f(obj, sel(name), a)
    }
}

/// `[obj name: integer]`
fn send_int(obj: Id, name: &str, a: isize) -> Id {
    // SAFETY: as `send0`, the message taking one integer argument
    unsafe {
        let f: unsafe extern "C" fn(Id, *mut c_void, isize) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        f(obj, sel(name), a)
    }
}

/// `[obj name: BOOL]`
fn send_bool(obj: Id, name: &str, a: bool) -> Id {
    // SAFETY: as `send0`, the message taking one BOOL argument
    unsafe {
        let f: unsafe extern "C" fn(Id, *mut c_void, u8) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        f(obj, sel(name), u8::from(a))
    }
}

/// A message returning a BOOL
fn ask(obj: Id, name: &str) -> bool {
    // SAFETY: as `send0`, the message returning a BOOL (read as its low byte)
    unsafe {
        let f: unsafe extern "C" fn(Id, *mut c_void) -> u8 = std::mem::transmute(objc_msgSend as *const c_void);
        f(obj, sel(name)) != 0
    }
}

fn string(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).unwrap_or_default();
    // SAFETY: `stringWithUTF8String:` takes a NUL-terminated C string and returns an autoreleased NSString
    unsafe {
        let f: unsafe extern "C" fn(Id, *mut c_void, *const c_char) -> Id =
            std::mem::transmute(objc_msgSend as *const c_void);
        f(class("NSString"), sel("stringWithUTF8String:"), c.as_ptr())
    }
}

fn app() -> Id {
    send0(class("NSApplication"), "sharedApplication")
}

pub fn init() -> Result<()> {
    if STARTED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let app = app();
    if app.is_null() {
        STARTED.store(false, Ordering::Release);
        return Err(Error::new("gui.init"));
    }
    // Accessory: windows and keyboard focus, no Dock icon and no menu bar
    send_int(app, "setActivationPolicy:", 1);
    send0(app, "finishLaunching");
    Ok(())
}

pub fn pump(timeout: Duration) {
    if !STARTED.load(Ordering::Acquire) {
        std::thread::sleep(timeout);
        return;
    }
    let app = app();
    let mut wait = timeout;
    loop {
        // SAFETY: `+[NSDate dateWithTimeIntervalSinceNow:]` takes a double and returns an autoreleased date
        let date = unsafe {
            let f: unsafe extern "C" fn(Id, *mut c_void, f64) -> Id =
                std::mem::transmute(objc_msgSend as *const c_void);
            f(class("NSDate"), sel("dateWithTimeIntervalSinceNow:"), wait.as_secs_f64())
        };
        // SAFETY: `nextEventMatchingMask:untilDate:inMode:dequeue:` (mask, date, mode string, BOOL), on the main thread
        let event = unsafe {
            let f: unsafe extern "C" fn(Id, *mut c_void, u64, Id, Id, u8) -> Id =
                std::mem::transmute(objc_msgSend as *const c_void);
            f(app, sel("nextEventMatchingMask:untilDate:inMode:dequeue:"), u64::MAX, date, NSDefaultRunLoopMode, 1)
        };
        if event.is_null() {
            break;
        }
        send1(app, "sendEvent:", event);
        // The rest already queued is taken without waiting
        wait = Duration::ZERO;
    }
    send0(app, "updateWindows");
}

#[derive(Debug)]
pub struct Native {
    window: Id,
    view: Id,
    shown: bool,
}

impl Native {
    pub fn new(title: &str, width: u32, height: u32) -> Result<Native> {
        init()?;
        let rect = Rect { x: 200.0, y: 200.0, w: f64::from(width), h: f64::from(height) };
        // titled | closable | miniaturizable | resizable; buffered backing; not deferred
        // SAFETY: `initWithContentRect:styleMask:backing:defer:` on a freshly allocated NSWindow (rect, mask, backing, BOOL)
        let window = unsafe {
            let f: unsafe extern "C" fn(Id, *mut c_void, Rect, usize, usize, u8) -> Id =
                std::mem::transmute(objc_msgSend as *const c_void);
            let alloc = send0(class("NSWindow"), "alloc");
            f(alloc, sel("initWithContentRect:styleMask:backing:defer:"), rect, 15, 2, 0)
        };
        if window.is_null() {
            return Err(Error::new("gui.window"));
        }
        send_bool(window, "setReleasedWhenClosed:", false);
        send1(window, "setTitle:", string(title));
        let view = send0(window, "contentView");
        if view.is_null() {
            send0(window, "close");
            return Err(Error::new("gui.window"));
        }
        Ok(Native { window, view, shown: false })
    }

    pub fn handle(&self) -> usize {
        self.view as usize
    }

    pub fn show(&mut self) {
        send1(self.window, "makeKeyAndOrderFront:", std::ptr::null_mut());
        send_bool(app(), "activateIgnoringOtherApps:", true);
        self.shown = true;
    }

    pub fn hide(&mut self) {
        send1(self.window, "orderOut:", std::ptr::null_mut());
        self.shown = false;
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        let size = Size { w: f64::from(width), h: f64::from(height) };
        // SAFETY: `setContentSize:` takes an NSSize (two doubles)
        unsafe {
            let f: unsafe extern "C" fn(Id, *mut c_void, Size) = std::mem::transmute(objc_msgSend as *const c_void);
            f(self.window, sel("setContentSize:"), size);
        }
    }

    pub fn take_closed(&mut self) -> bool {
        if self.shown && !ask(self.window, "isVisible") && !ask(self.window, "isMiniaturized") {
            self.shown = false;
            return true;
        }
        false
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        send0(self.window, "close");
        send0(self.window, "release");
    }
}
