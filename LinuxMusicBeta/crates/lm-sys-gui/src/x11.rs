//! X11 through `libX11`, loaded at run time (`dlopen`: the helper starts on a machine without X11 and says
//! `gui.unsupported` only when a window is asked for). Main thread only.
use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;

use lm_base::{Error, Result};

type Display = *mut c_void;
type Window = c_ulong;
type Atom = c_ulong;

/// `XEvent` is a union of 24 longs; only its type and a client message's first datum are read
#[repr(C)]
struct XEvent {
    words: [c_long; 24],
}

const CLIENT_MESSAGE: c_int = 33;
const STRUCTURE_NOTIFY: c_long = 1 << 17;

/// The libX11 functions used
struct Xlib {
    open: unsafe extern "C" fn(*const c_char) -> Display,
    root: unsafe extern "C" fn(Display) -> Window,
    create: unsafe extern "C" fn(Display, Window, c_int, c_int, c_uint, c_uint, c_uint, c_ulong, c_ulong) -> Window,
    store_name: unsafe extern "C" fn(Display, Window, *const c_char) -> c_int,
    map: unsafe extern "C" fn(Display, Window) -> c_int,
    unmap: unsafe extern "C" fn(Display, Window) -> c_int,
    resize: unsafe extern "C" fn(Display, Window, c_uint, c_uint) -> c_int,
    intern: unsafe extern "C" fn(Display, *const c_char, c_int) -> Atom,
    set_protocols: unsafe extern "C" fn(Display, Window, *mut Atom, c_int) -> c_int,
    select_input: unsafe extern "C" fn(Display, Window, c_long) -> c_int,
    pending: unsafe extern "C" fn(Display) -> c_int,
    next: unsafe extern "C" fn(Display, *mut XEvent) -> c_int,
    flush: unsafe extern "C" fn(Display) -> c_int,
    destroy: unsafe extern "C" fn(Display, Window) -> c_int,
    connection: unsafe extern "C" fn(Display) -> c_int,
}

struct Connection {
    x: Xlib,
    display: Display,
    delete: Atom,
}

// SAFETY: the statics only need to hold it; it is used from the thread that made it alone (`connection` refuses
// any other thread)
unsafe impl Send for Connection {}
// SAFETY: as above: a `&Connection` is handed out to the owner thread alone, so nothing is called concurrently
unsafe impl Sync for Connection {}

static CONNECTION: OnceLock<std::result::Result<Connection, &'static str>> = OnceLock::new();
/// The thread that made the connection (the first to ask: the main thread, by the module's contract)
static OWNER: OnceLock<std::thread::ThreadId> = OnceLock::new();
/// The windows the user asked to close since they were last asked about
static CLOSED: Mutex<Vec<Window>> = Mutex::new(Vec::new());

fn symbol<T: Copy>(lib: *mut c_void, name: &str) -> Option<T> {
    let c = CString::new(name).ok()?;
    // SAFETY: a handle from `dlopen` and a NUL-terminated symbol name
    let p = unsafe { libc::dlsym(lib, c.as_ptr()) };
    // SAFETY: `T` is the function pointer type of the symbol (pointer-sized, checked below)
    (!p.is_null() && std::mem::size_of::<T>() == std::mem::size_of::<*mut c_void>())
        .then(|| unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
}

fn load() -> std::result::Result<Connection, &'static str> {
    let name = c"libX11.so.6";
    // SAFETY: a NUL-terminated library name
    let lib = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if lib.is_null() {
        return Err("gui.unsupported");
    }
    macro_rules! sym {
        ($n:literal) => {
            symbol(lib, $n).ok_or("gui.init")?
        };
    }
    let x = Xlib {
        open: sym!("XOpenDisplay"),
        root: sym!("XDefaultRootWindow"),
        create: sym!("XCreateSimpleWindow"),
        store_name: sym!("XStoreName"),
        map: sym!("XMapWindow"),
        unmap: sym!("XUnmapWindow"),
        resize: sym!("XResizeWindow"),
        intern: sym!("XInternAtom"),
        set_protocols: sym!("XSetWMProtocols"),
        select_input: sym!("XSelectInput"),
        pending: sym!("XPending"),
        next: sym!("XNextEvent"),
        flush: sym!("XFlush"),
        destroy: sym!("XDestroyWindow"),
        connection: sym!("XConnectionNumber"),
    };
    // SAFETY: the default display (the DISPLAY variable)
    let display = unsafe { (x.open)(std::ptr::null()) };
    if display.is_null() {
        return Err("gui.init");
    }
    // SAFETY: a NUL-terminated atom name on an open display
    let delete = unsafe { (x.intern)(display, c"WM_DELETE_WINDOW".as_ptr(), 0) };
    Ok(Connection { x, display, delete })
}

/// The connection, only on the thread that made it: Xlib is not thread safe (`gui.wrong_thread` elsewhere)
fn connection() -> Result<&'static Connection> {
    let me = std::thread::current().id();
    let owner = *OWNER.get_or_init(|| me);
    if owner != me {
        return Err(Error::new("gui.wrong_thread"));
    }
    CONNECTION.get_or_init(load).as_ref().map_err(|k| Error::new(k))
}

pub fn init() -> Result<()> {
    connection().map(|_| ())
}

pub fn pump(timeout: Duration) {
    let Ok(c) = connection() else {
        std::thread::sleep(timeout);
        return;
    };
    // SAFETY: an open display
    let fd = unsafe { (c.x.connection)(c.display) };
    // SAFETY: an open display
    if unsafe { (c.x.pending)(c.display) } == 0 {
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let ms = c_int::try_from(timeout.as_millis()).unwrap_or(c_int::MAX);
        // SAFETY: one valid pollfd
        unsafe { libc::poll(&mut p, 1, ms) };
    }
    // SAFETY: an open display; events are read into a buffer of the union's size
    while unsafe { (c.x.pending)(c.display) } > 0 {
        let mut ev = XEvent { words: [0; 24] };
        // SAFETY: as above
        unsafe { (c.x.next)(c.display, &mut ev) };
        // The `type` is the first int; for a client message the window is word 4 and its first datum word 7
        let kind = ev.words[0] as c_int;
        if kind == CLIENT_MESSAGE && ev.words[7] as Atom == c.delete {
            let window = ev.words[4] as Window;
            // SAFETY: an open display and a window of ours
            unsafe { (c.x.unmap)(c.display, window) };
            if let Ok(mut closed) = CLOSED.lock() {
                if !closed.contains(&window) {
                    closed.push(window);
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Native {
    window: Window,
}

impl Native {
    pub fn new(title: &str, width: u32, height: u32) -> Result<Native> {
        let c = connection()?;
        let name = CString::new(title.replace('\0', "")).unwrap_or_default();
        // SAFETY: an open display; a simple window under the root; the title is NUL-terminated; the delete atom lives
        // for the call
        let window = unsafe {
            let w = (c.x.create)(c.display, (c.x.root)(c.display), 0, 0, width.max(1), height.max(1), 0, 0, 0);
            if w == 0 {
                return Err(Error::new("gui.window"));
            }
            (c.x.store_name)(c.display, w, name.as_ptr());
            (c.x.select_input)(c.display, w, STRUCTURE_NOTIFY);
            let mut delete = c.delete;
            (c.x.set_protocols)(c.display, w, &mut delete, 1);
            w
        };
        Ok(Native { window })
    }

    pub fn handle(&self) -> usize {
        self.window as usize
    }

    pub fn show(&mut self) {
        if let Ok(c) = connection() {
            // SAFETY: an open display and a window of ours
            unsafe {
                (c.x.map)(c.display, self.window);
                (c.x.flush)(c.display);
            }
        }
    }

    pub fn hide(&mut self) {
        if let Ok(c) = connection() {
            // SAFETY: an open display and a window of ours
            unsafe {
                (c.x.unmap)(c.display, self.window);
                (c.x.flush)(c.display);
            }
        }
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        if let Ok(c) = connection() {
            // SAFETY: an open display and a window of ours
            unsafe {
                (c.x.resize)(c.display, self.window, width.max(1), height.max(1));
                (c.x.flush)(c.display);
            }
        }
    }

    pub fn take_closed(&mut self) -> bool {
        let Ok(mut closed) = CLOSED.lock() else { return false };
        match closed.iter().position(|w| *w == self.window) {
            Some(i) => {
                closed.swap_remove(i);
                true
            }
            None => false,
        }
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        // A close nobody asked about must not outlive the window: the server reuses window ids, and the next window
        // with this id would start out "closed"
        if let Ok(mut closed) = CLOSED.lock() {
            closed.retain(|w| *w != self.window);
        }
        if let Ok(c) = connection() {
            // SAFETY: an open display and a window of ours, destroyed once
            unsafe {
                (c.x.destroy)(c.display, self.window);
                (c.x.flush)(c.display);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_thread_is_refused_the_connection() {
        // Whether or not libX11 is there, the first thread to ask owns the connection
        let _ = init();
        let other = std::thread::spawn(|| init().err().map(|e| e.key().to_owned())).join().unwrap();
        assert_eq!(other.as_deref(), Some("gui.wrong_thread"));
    }
}
