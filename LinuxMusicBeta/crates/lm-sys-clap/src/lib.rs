//! Safe binding for hosting CLAP plugins (CLAP 1.2, the ABI written by hand in `ffi`, checked against the vendored
//! headers by `tests/layout.rs`).
//!
//! CLAP plugins are third-party plugins: they run **out of the main process**, in the plugin-host helper (interfaces.md
//! I-9.6). This crate is what that helper links.
//!
//! - [`Library`]: a `.clap` file (a shared library, or a macOS bundle) opened, its entry initialised, its plugins
//!   listed ([`Library::descriptors`]).
//! - [`Instance`]: one plugin made and activated with a host this crate provides (log, thread check, params,
//!   latency, state, audio and note ports, GUI), its parameters ([`Instance::params`]), its audio ports, its
//!   latency, one block ([`Instance::process`]: audio, events, transport; the plugin's own parameter changes come
//!   back in [`Instance::changes`]), its state as bytes ([`Instance::save_state`], [`Instance::load_state`]), and
//!   [`Instance::idle`], the main-thread turn (callbacks, restarts, rescans).
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | The `dlopen` handle and the entry: one [`Library`] (an `Arc` shared by its instances, so the library outlives them); `Drop` calls the entry's `deinit` and `dlclose`. A `clap_plugin*`: one [`Instance`]; `Drop` stops processing, deactivates and calls `destroy`, then drops the host structure the plugin held a pointer to (boxed, at a fixed address for the plugin's whole life). The audio buffers, the event lists and their slots: owned by the `Instance`, allocated when it is made, at fixed addresses. Descriptors and parameter infos are copied into owned Rust data: no plugin pointer leaves the crate. |
//! | Which thread may call each function? | CLAP's thread classes: [`Instance::process`] is the **audio thread** (the caller says which thread it is with [`set_thread_role`]; the host's `thread-check` answers from it). Everything else on `Instance` is the **main thread**. `Instance` is `Send`, not `Sync`, and every call takes `&mut self`, so the two never run at once; restarts (deactivate, activate) are done by [`Instance::idle`] only after the audio side has stopped processing at its next block. `Library` is `Send` and `Sync` (CLAP's entry and factory are thread-safe). |
//! | Allocates, locks, does I/O? | [`Instance::process`], [`Instance::set_param`] and [`Instance::changes`] do not allocate, lock or do I/O on the host's side: the buffers, events and outputs are preallocated (`tests/instance.rs` runs `process` under `assert_no_alloc`). Whether the plugin's own `process` is real-time safe is the plugin's promise. Everything else (opening, instantiating, state, parameter infos, `idle`) may allocate and block. The host's `log` writes to stderr (in the helper process). |
//! | Creates threads? | No. A plugin may create its own. |
//! | Calls back into our code? | Yes, from plugin code on any thread, through `extern "C"` functions that never unwind (each body is `catch_unwind`ed): the host's `get_extension`, `request_restart`, `request_process`, `request_callback`, `log`, `thread-check`, params `rescan`/`clear`/`request_flush`, `latency.changed`, `state.mark_dirty`, `audio-ports`/`note-ports` `rescan`, `gui` requests, the event lists and the state streams. They only touch atomics and the instance's own preallocated lists. |
//! | Tests | `tests/plugin/lm_test_clap.c`, two plugins compiled by the tests with the system's C compiler against the vendored headers (a gain with parameters, latency, state and a restart; a synth with note ports, note expressions, transport and parameter output). `tests/layout.rs` (every struct's size and field offsets against the C compiler's), `tests/instance.rs` (listing, parameters, exact audio, latency and restart, notes and expressions, the plugin's own parameter changes, state round trip, thread roles, no allocation in `process`). |

mod ffi;

use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lm_base::{Error, Result};

pub use ffi::{
    CLAP_NOTE_EXPRESSION_BRIGHTNESS, CLAP_NOTE_EXPRESSION_EXPRESSION, CLAP_NOTE_EXPRESSION_PAN,
    CLAP_NOTE_EXPRESSION_PRESSURE, CLAP_NOTE_EXPRESSION_TUNING, CLAP_NOTE_EXPRESSION_VIBRATO,
    CLAP_NOTE_EXPRESSION_VOLUME,
};

#[doc(hidden)]
pub mod layout {
    //! The declarations, for `tests/layout.rs`
    pub use crate::ffi::*;
}

fn err(key: &'static str) -> Error {
    Error::new(key)
}

/// A C string the plugin gave, copied (`""` for null)
fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the plugin gives NUL-terminated strings that live at least as long as the call that returned them
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// A fixed char array the plugin filled, copied up to its NUL
fn text_array(a: &[c_char]) -> String {
    let bytes: Vec<u8> = a.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---------------------------------------------------------------------------------------------------------------
// Threads

/// Which thread this is, for the plugin's `thread-check` (the helper sets it on its threads)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadRole {
    Main,
    Audio,
}

thread_local! {
    static ROLE: Cell<ThreadRole> = const { Cell::new(ThreadRole::Main) };
}

/// Declares the calling thread's role (the audio thread before its first [`Instance::process`])
pub fn set_thread_role(role: ThreadRole) {
    ROLE.with(|r| r.set(role));
}

fn role() -> ThreadRole {
    ROLE.with(Cell::get)
}

// ---------------------------------------------------------------------------------------------------------------
// The library

/// A plugin a `.clap` file offers
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    pub features: Vec<String>,
}

impl Descriptor {
    /// An instrument by its features (`"instrument"`)
    pub fn is_instrument(&self) -> bool {
        self.features.iter().any(|f| f == "instrument")
    }
}

/// An opened `.clap` file
pub struct Library {
    handle: *mut c_void,
    entry: *const ffi::clap_plugin_entry,
    factory: *const ffi::clap_plugin_factory,
    path: PathBuf,
}

// SAFETY: CLAP's entry and plugin factory are thread-safe by the specification (any thread, concurrently); the
// handle is only closed in Drop
unsafe impl Send for Library {}
// SAFETY: as above: every method only calls the entry and factory functions, which CLAP makes thread-safe
unsafe impl Sync for Library {}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library").field("path", &self.path).finish_non_exhaustive()
    }
}

/// The shared library inside a `.clap` path: the file itself, or a macOS bundle's executable
fn binary_of(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    let macos = path.join("Contents").join("MacOS");
    let stem = path.file_stem().map(|s| macos.join(s));
    if let Some(p) = stem.filter(|p| p.is_file()) {
        return Ok(p);
    }
    std::fs::read_dir(&macos)
        .ok()
        .and_then(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.is_file()))
        .ok_or_else(|| err("clap.not_found").arg("path", path.display().to_string()))
}

impl Library {
    /// Opens a `.clap` file and initialises its entry. Runs the plugin's code (`init`): do it in the helper
    pub fn open(path: &Path) -> Result<Arc<Library>> {
        let binary = binary_of(path)?;
        let c_binary = CString::new(binary.as_os_str().to_string_lossy().as_bytes())
            .map_err(|_| err("clap.path").arg("path", path.display().to_string()))?;
        let c_path = CString::new(path.as_os_str().to_string_lossy().as_bytes())
            .map_err(|_| err("clap.path").arg("path", path.display().to_string()))?;
        // SAFETY: a NUL-terminated path; dlopen runs the library's constructors (plugin code, in the helper)
        let handle = unsafe { libc::dlopen(c_binary.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            // SAFETY: dlerror returns null or a NUL-terminated message owned by the C library, read at once
            let why = unsafe { libc::dlerror() };
            return Err(err("clap.open").arg("path", path.display().to_string()).arg("error", text(why)));
        }
        let close = |e: Error| {
            // SAFETY: the handle dlopen just gave, closed once
            unsafe { libc::dlclose(handle) };
            e
        };
        // SAFETY: a valid handle and a NUL-terminated symbol name
        let sym = unsafe { libc::dlsym(handle, c"clap_entry".as_ptr()) };
        if sym.is_null() {
            return Err(close(err("clap.no_entry").arg("path", path.display().to_string())));
        }
        let entry = sym as *const ffi::clap_plugin_entry;
        // SAFETY: `clap_entry` is a `clap_plugin_entry_t` by the ABI; it lives as long as the library stays open
        let e = unsafe { &*entry };
        if e.clap_version.major < 1 {
            return Err(close(err("clap.version").arg("path", path.display().to_string())));
        }
        let (Some(init), Some(get_factory)) = (e.init, e.get_factory) else {
            return Err(close(err("clap.no_entry").arg("path", path.display().to_string())));
        };
        // SAFETY: the entry's init with the plugin's path, called once before anything else (CLAP)
        if !unsafe { init(c_path.as_ptr()) } {
            return Err(close(err("clap.init").arg("path", path.display().to_string())));
        }
        // SAFETY: a NUL-terminated factory id; the result is a factory or null
        let factory =
            unsafe { get_factory(ffi::CLAP_PLUGIN_FACTORY_ID.as_ptr().cast()) } as *const ffi::clap_plugin_factory;
        let lib = Library { handle, entry, factory, path: path.to_path_buf() };
        if factory.is_null() {
            return Err(err("clap.no_factory").arg("path", path.display().to_string()));
        }
        Ok(Arc::new(lib))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn factory(&self) -> &ffi::clap_plugin_factory {
        // SAFETY: checked non-null at open; the factory lives as long as the library is initialised
        unsafe { &*self.factory }
    }

    /// The plugins the file offers
    pub fn descriptors(&self) -> Vec<Descriptor> {
        let f = self.factory();
        let (Some(count), Some(get)) = (f.get_plugin_count, f.get_plugin_descriptor) else { return Vec::new() };
        // SAFETY: the factory's own functions with the factory pointer
        let n = unsafe { count(self.factory) };
        let mut out = Vec::new();
        for i in 0..n.min(4096) {
            // SAFETY: an index below the count; the result is a descriptor or null, valid while the library is open
            let d = unsafe { get(self.factory, i) };
            if d.is_null() {
                continue;
            }
            // SAFETY: non-null descriptor from the factory
            let d = unsafe { &*d };
            let mut features = Vec::new();
            if !d.features.is_null() {
                for k in 0..256 {
                    // SAFETY: a NULL-terminated array of C strings (CLAP); bounded at 256 entries
                    let p = unsafe { *d.features.add(k) };
                    if p.is_null() {
                        break;
                    }
                    features.push(text(p));
                }
            }
            out.push(Descriptor {
                id: text(d.id),
                name: text(d.name),
                vendor: text(d.vendor),
                version: text(d.version),
                description: text(d.description),
                features,
            });
        }
        out
    }
}

/// CLAP asks for `deinit` on the main thread. It runs on the thread that drops the last `Arc`, so the owners must drop
/// their instances and the library on the main thread, never from the audio callback
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: the entry stays valid until dlclose; deinit once, after every instance is gone (they hold the Arc)
        if let Some(deinit) = unsafe { (*self.entry).deinit } {
            // SAFETY: as above
            unsafe { deinit() };
        }
        // SAFETY: the handle from dlopen, closed once
        unsafe { libc::dlclose(self.handle) };
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The host the plugin sees

/// What the plugin asked of the host, from any thread
#[derive(Debug, Default)]
struct Flags {
    restart: AtomicBool,
    callback: AtomicBool,
    latency: AtomicBool,
    params_rescan: AtomicBool,
    state_dirty: AtomicBool,
    ports_rescan: AtomicBool,
    gui_closed: AtomicBool,
    /// The size the plugin's gui asked for (`request_resize`): width << 32 | height, 0 for none
    gui_resize: std::sync::atomic::AtomicU64,
}

#[repr(C)]
struct HostBox {
    host: ffi::clap_host,
    flags: Flags,
}

fn flags<'a>(host: *const ffi::clap_host) -> Option<&'a Flags> {
    if host.is_null() {
        return None;
    }
    // SAFETY: `host` is the first field of a `HostBox` this crate made (repr(C)), alive while its plugin lives
    let b = unsafe { &*(host as *const HostBox) };
    Some(&b.flags)
}

/// Runs a callback body, never unwinding into the plugin. Every host callback below goes through it, except the ones
/// that only return a constant or do nothing (`host_request_process`, `host_params_clear`, `host_ports_supported`,
/// `host_note_dialects`, `host_gui_hints`, `host_gui_show`), which cannot panic
fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(default)
}

macro_rules! set_flag {
    ($name:ident, $field:ident) => {
        unsafe extern "C" fn $name(host: *const ffi::clap_host) {
            guard((), || {
                if let Some(f) = flags(host) {
                    f.$field.store(true, Ordering::Release);
                }
            })
        }
    };
}

set_flag!(host_request_restart, restart);
set_flag!(host_request_callback, callback);
set_flag!(host_latency_changed, latency);
set_flag!(host_mark_dirty, state_dirty);

unsafe extern "C" fn host_request_process(_: *const ffi::clap_host) {}

unsafe extern "C" fn host_log(_: *const ffi::clap_host, severity: i32, msg: *const c_char) {
    guard((), || {
        // Debug and info stay quiet; the rest goes to the helper's stderr
        if severity >= 2 {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("clap plugin: {}", text(msg));
            }
        }
    })
}

unsafe extern "C" fn host_is_main(_: *const ffi::clap_host) -> bool {
    guard(false, || role() == ThreadRole::Main)
}

unsafe extern "C" fn host_is_audio(_: *const ffi::clap_host) -> bool {
    guard(false, || role() == ThreadRole::Audio)
}

unsafe extern "C" fn host_params_rescan(host: *const ffi::clap_host, _flags: u32) {
    guard((), || {
        if let Some(f) = flags(host) {
            f.params_rescan.store(true, Ordering::Release);
        }
    })
}

unsafe extern "C" fn host_params_clear(_: *const ffi::clap_host, _: ffi::clap_id, _: u32) {}

unsafe extern "C" fn host_params_request_flush(host: *const ffi::clap_host) {
    // The next block delivers the plugin's pending changes (it is processing continuously)
    guard((), || {
        if let Some(f) = flags(host) {
            f.callback.store(true, Ordering::Release);
        }
    })
}

unsafe extern "C" fn host_ports_supported(_: *const ffi::clap_host, _: u32) -> bool {
    true
}

unsafe extern "C" fn host_ports_rescan(host: *const ffi::clap_host, _: u32) {
    guard((), || {
        if let Some(f) = flags(host) {
            f.ports_rescan.store(true, Ordering::Release);
            f.restart.store(true, Ordering::Release);
        }
    })
}

unsafe extern "C" fn host_note_dialects(_: *const ffi::clap_host) -> u32 {
    ffi::CLAP_NOTE_DIALECT_CLAP | ffi::CLAP_NOTE_DIALECT_MIDI
}

unsafe extern "C" fn host_gui_hints(_: *const ffi::clap_host) {}

unsafe extern "C" fn host_gui_resize(host: *const ffi::clap_host, w: u32, h: u32) -> bool {
    // A floating window resizes itself; an embedded one is resized by the helper's loop, which takes this
    guard(false, || {
        if let Some(f) = flags(host) {
            f.gui_resize.store((u64::from(w) << 32) | u64::from(h), Ordering::Release);
        }
        true
    })
}

unsafe extern "C" fn host_gui_show(_: *const ffi::clap_host) -> bool {
    false
}

unsafe extern "C" fn host_gui_closed(host: *const ffi::clap_host, _: bool) {
    guard((), || {
        if let Some(f) = flags(host) {
            f.gui_closed.store(true, Ordering::Release);
        }
    })
}

static HOST_LOG: ffi::clap_host_log = ffi::clap_host_log { log: Some(host_log) };
static HOST_THREAD_CHECK: ffi::clap_host_thread_check =
    ffi::clap_host_thread_check { is_main_thread: Some(host_is_main), is_audio_thread: Some(host_is_audio) };
static HOST_PARAMS: ffi::clap_host_params = ffi::clap_host_params {
    rescan: Some(host_params_rescan),
    clear: Some(host_params_clear),
    request_flush: Some(host_params_request_flush),
};
static HOST_LATENCY: ffi::clap_host_latency = ffi::clap_host_latency { changed: Some(host_latency_changed) };
static HOST_STATE: ffi::clap_host_state = ffi::clap_host_state { mark_dirty: Some(host_mark_dirty) };
static HOST_AUDIO_PORTS: ffi::clap_host_audio_ports = ffi::clap_host_audio_ports {
    is_rescan_flag_supported: Some(host_ports_supported),
    rescan: Some(host_ports_rescan),
};
static HOST_NOTE_PORTS: ffi::clap_host_note_ports =
    ffi::clap_host_note_ports { supported_dialects: Some(host_note_dialects), rescan: Some(host_ports_rescan) };
static HOST_GUI: ffi::clap_host_gui = ffi::clap_host_gui {
    resize_hints_changed: Some(host_gui_hints),
    request_resize: Some(host_gui_resize),
    request_show: Some(host_gui_show),
    request_hide: Some(host_gui_show),
    closed: Some(host_gui_closed),
};

unsafe extern "C" fn host_get_extension(_: *const ffi::clap_host, id: *const c_char) -> *const c_void {
    guard(ptr::null(), || {
        if id.is_null() {
            return ptr::null();
        }
        // SAFETY: a NUL-terminated id from the plugin, read during the call
        let id = unsafe { CStr::from_ptr(id) }.to_bytes_with_nul();
        let is = |e: &[u8]| e == id;
        if is(ffi::CLAP_EXT_LOG) {
            (&raw const HOST_LOG).cast()
        } else if is(ffi::CLAP_EXT_THREAD_CHECK) {
            (&raw const HOST_THREAD_CHECK).cast()
        } else if is(ffi::CLAP_EXT_PARAMS) {
            (&raw const HOST_PARAMS).cast()
        } else if is(ffi::CLAP_EXT_LATENCY) {
            (&raw const HOST_LATENCY).cast()
        } else if is(ffi::CLAP_EXT_STATE) {
            (&raw const HOST_STATE).cast()
        } else if is(ffi::CLAP_EXT_AUDIO_PORTS) {
            (&raw const HOST_AUDIO_PORTS).cast()
        } else if is(ffi::CLAP_EXT_NOTE_PORTS) {
            (&raw const HOST_NOTE_PORTS).cast()
        } else if is(ffi::CLAP_EXT_GUI) {
            (&raw const HOST_GUI).cast()
        } else {
            ptr::null()
        }
    })
}

// SAFETY (of the statics' Sync): the host extension structs hold only function pointers; the plugin reads them
struct SyncPtr(*const c_char);
// SAFETY: points to a 'static NUL-terminated literal, never written
unsafe impl Sync for SyncPtr {}
static HOST_NAME: SyncPtr = SyncPtr(c"LinuxMusic".as_ptr());
static HOST_VENDOR: SyncPtr = SyncPtr(c"LinuxMusic".as_ptr());
static HOST_URL: SyncPtr = SyncPtr(c"https://github.com/EliezerSolinger/LinuxMusic".as_ptr());
static HOST_VERSION: SyncPtr = SyncPtr(c"0.1".as_ptr());

// ---------------------------------------------------------------------------------------------------------------
// Events

/// An event for the plugin in one block (`time`: the frame in the block)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    NoteOn {
        time: u32,
        note_id: i32,
        channel: i16,
        key: i16,
        velocity: f64,
    },
    NoteOff {
        time: u32,
        note_id: i32,
        channel: i16,
        key: i16,
        velocity: f64,
    },
    /// Every sound of a note, at once (all sound off)
    Choke {
        time: u32,
        note_id: i32,
        channel: i16,
        key: i16,
    },
    /// `expression`: one of the `CLAP_NOTE_EXPRESSION_*`
    Expression {
        time: u32,
        expression: i32,
        note_id: i32,
        channel: i16,
        key: i16,
        value: f64,
    },
    Param {
        time: u32,
        id: u32,
        value: f64,
    },
    Midi {
        time: u32,
        data: [u8; 3],
    },
}

/// The song's position for one block
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transport {
    pub playing: bool,
    pub recording: bool,
    /// Quarter notes from the start
    pub beats: f64,
    pub seconds: f64,
    pub tempo: f64,
    /// (numerator, denominator), when known
    pub signature: Option<(u16, u16)>,
}

/// One slot of the input list: room for the largest event this host sends
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct Slot([u8; 128]);

const _: () = assert!(std::mem::size_of::<ffi::clap_event_transport>() <= 128);
const _: () = assert!(std::mem::size_of::<ffi::clap_event_param_value>() <= 128);

#[repr(C)]
struct InEvents {
    list: ffi::clap_input_events,
    slots: Vec<Slot>,
    len: usize,
}

#[repr(C)]
struct OutEvents {
    list: ffi::clap_output_events,
    params: Vec<(u32, f64)>,
}

unsafe extern "C" fn in_size(list: *const ffi::clap_input_events) -> u32 {
    guard(0, || {
        // SAFETY: `list` is the first field of an `InEvents` this crate made (repr(C)), alive for the call
        let l = unsafe { &*(list as *const InEvents) };
        l.len as u32
    })
}

unsafe extern "C" fn in_get(list: *const ffi::clap_input_events, index: u32) -> *const ffi::clap_event_header {
    guard(ptr::null(), || {
        // SAFETY: as in `in_size`
        let l = unsafe { &*(list as *const InEvents) };
        match l.slots.get(index as usize).filter(|_| (index as usize) < l.len) {
            Some(s) => (s as *const Slot).cast(),
            None => ptr::null(),
        }
    })
}

unsafe extern "C" fn out_push(list: *const ffi::clap_output_events, event: *const ffi::clap_event_header) -> bool {
    guard(false, || {
        if event.is_null() {
            return false;
        }
        // SAFETY: `list` is the first field of an `OutEvents` this crate made, alive for the call; the plugin only
        // calls it from inside `process` or `flush`, while the instance is borrowed mutably by that call
        let l = unsafe { &mut *(list as *mut OutEvents) };
        // SAFETY: a valid event header from the plugin, for the call
        let h = unsafe { &*event };
        if h.space_id == ffi::CLAP_CORE_EVENT_SPACE_ID && h.type_ == ffi::CLAP_EVENT_PARAM_VALUE {
            if (h.size as usize) < std::mem::size_of::<ffi::clap_event_param_value>() {
                return false;
            }
            // SAFETY: the header says it is a param value event of at least that size
            let p = unsafe { &*(event as *const ffi::clap_event_param_value) };
            if l.params.len() == l.params.capacity() {
                return false;
            }
            l.params.push((p.param_id, p.value));
        }
        true
    })
}

fn header(time: u32, type_: u16, size: usize) -> ffi::clap_event_header {
    ffi::clap_event_header {
        size: size as u32,
        time,
        space_id: ffi::CLAP_CORE_EVENT_SPACE_ID,
        type_,
        flags: ffi::CLAP_EVENT_IS_LIVE,
    }
}

impl InEvents {
    fn push<T: Copy>(&mut self, e: T) -> bool {
        if self.len >= self.slots.len() {
            return false;
        }
        const { assert!(std::mem::size_of::<T>() <= 128 && std::mem::align_of::<T>() <= 8) };
        let slot = &mut self.slots[self.len];
        // SAFETY: the slot is 128 bytes, 8-aligned, and T fits both (checked at compile time)
        unsafe { ptr::write((slot as *mut Slot).cast::<T>(), e) };
        self.len += 1;
        true
    }

    fn push_event(&mut self, e: &Event) -> bool {
        use std::mem::size_of;
        match *e {
            Event::NoteOn { time, note_id, channel, key, velocity }
            | Event::NoteOff { time, note_id, channel, key, velocity } => {
                let t =
                    if matches!(e, Event::NoteOn { .. }) { ffi::CLAP_EVENT_NOTE_ON } else { ffi::CLAP_EVENT_NOTE_OFF };
                self.push(ffi::clap_event_note {
                    header: header(time, t, size_of::<ffi::clap_event_note>()),
                    note_id,
                    port_index: 0,
                    channel,
                    key,
                    velocity,
                })
            }
            Event::Choke { time, note_id, channel, key } => self.push(ffi::clap_event_note {
                header: header(time, ffi::CLAP_EVENT_NOTE_CHOKE, size_of::<ffi::clap_event_note>()),
                note_id,
                port_index: 0,
                channel,
                key,
                velocity: 0.0,
            }),
            Event::Expression { time, expression, note_id, channel, key, value } => {
                self.push(ffi::clap_event_note_expression {
                    header: header(time, ffi::CLAP_EVENT_NOTE_EXPRESSION, size_of::<ffi::clap_event_note_expression>()),
                    expression_id: expression,
                    note_id,
                    port_index: 0,
                    channel,
                    key,
                    value,
                })
            }
            Event::Param { time, id, value } => self.push(ffi::clap_event_param_value {
                header: header(time, ffi::CLAP_EVENT_PARAM_VALUE, size_of::<ffi::clap_event_param_value>()),
                param_id: id,
                cookie: ptr::null_mut(),
                note_id: -1,
                port_index: -1,
                channel: -1,
                key: -1,
                value,
            }),
            Event::Midi { time, data } => self.push(ffi::clap_event_midi {
                header: header(time, ffi::CLAP_EVENT_MIDI, size_of::<ffi::clap_event_midi>()),
                port_index: 0,
                data,
            }),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The instance

/// One parameter of a plugin
#[derive(Clone, Debug, PartialEq)]
pub struct ParamInfo {
    /// The plugin's stable id
    pub id: u32,
    pub name: String,
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub stepped: bool,
    pub hidden: bool,
    pub read_only: bool,
    pub automatable: bool,
    pub bypass: bool,
}

/// One audio port
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioPort {
    pub name: String,
    pub channels: u32,
    pub main: bool,
}

#[derive(Clone, Copy)]
struct Exts {
    params: *const ffi::clap_plugin_params,
    latency: *const ffi::clap_plugin_latency,
    state: *const ffi::clap_plugin_state,
    audio_ports: *const ffi::clap_plugin_audio_ports,
    note_ports: *const ffi::clap_plugin_note_ports,
    gui: *const ffi::clap_plugin_gui,
}

/// The longest the plugin's own parameter change list grows in one block
const MAX_OUT_EVENTS: usize = 1024;
/// The most events in one block
pub const MAX_EVENTS: usize = 4096;

/// One plugin made, activated, and ready to process
pub struct Instance {
    _lib: Arc<Library>,
    host: Box<HostBox>,
    plugin: *const ffi::clap_plugin,
    ext: Exts,
    rate: f64,
    max_block: u32,
    active: bool,
    processing: bool,
    latency: u32,
    params: Vec<ParamInfo>,
    inputs: Vec<AudioPort>,
    outputs: Vec<AudioPort>,
    note_inputs: u32,
    // Preallocated for `process`
    in_events: Box<InEvents>,
    out_events: Box<OutEvents>,
    pending: Vec<(u32, f64)>,
    changes: Vec<(u32, f64)>,
    in_data: Vec<Vec<f32>>,
    out_data: Vec<Vec<f32>>,
    in_ptrs: Vec<*mut f32>,
    out_ptrs: Vec<*mut f32>,
    in_bufs: Vec<ffi::clap_audio_buffer>,
    out_bufs: Vec<ffi::clap_audio_buffer>,
    steady: i64,
}

// SAFETY: a CLAP plugin may be called from different threads as long as its thread classes are kept: `Instance`
// is not `Sync`, every call takes `&mut self`, and the audio-thread call (`process`) and main-thread calls never
// overlap (the caller declares the thread with `set_thread_role`)
unsafe impl Send for Instance {}

impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instance").field("latency", &self.latency).field("params", &self.params.len()).finish()
    }
}

impl Instance {
    /// Makes plugin `id` of `lib`, initialises and activates it at `rate` for blocks up to `max_block` frames.
    /// Main thread
    pub fn new(lib: &Arc<Library>, id: &str, rate: f64, max_block: u32) -> Result<Instance> {
        let f = lib.factory();
        let create = f.create_plugin.ok_or_else(|| err("clap.no_factory"))?;
        let c_id = CString::new(id).map_err(|_| err("clap.not_found").arg("id", id))?;
        let host = Box::new(HostBox {
            host: ffi::clap_host {
                clap_version: ffi::CLAP_VERSION,
                host_data: ptr::null_mut(),
                name: HOST_NAME.0,
                vendor: HOST_VENDOR.0,
                url: HOST_URL.0,
                version: HOST_VERSION.0,
                get_extension: Some(host_get_extension),
                request_restart: Some(host_request_restart),
                request_process: Some(host_request_process),
                request_callback: Some(host_request_callback),
            },
            flags: Flags::default(),
        });
        // SAFETY: the factory's own function; `host` is boxed at a fixed address and outlives the plugin (Drop
        // destroys the plugin first)
        let plugin = unsafe { create(lib.factory, &host.host, c_id.as_ptr()) };
        if plugin.is_null() {
            return Err(err("clap.not_found").arg("id", id));
        }
        let mut inst = Instance {
            _lib: Arc::clone(lib),
            host,
            plugin,
            ext: Exts {
                params: ptr::null(),
                latency: ptr::null(),
                state: ptr::null(),
                audio_ports: ptr::null(),
                note_ports: ptr::null(),
                gui: ptr::null(),
            },
            rate,
            max_block: max_block.max(1),
            active: false,
            processing: false,
            latency: 0,
            params: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            note_inputs: 0,
            in_events: Box::new(InEvents {
                list: ffi::clap_input_events { ctx: ptr::null_mut(), size: Some(in_size), get: Some(in_get) },
                slots: vec![Slot([0; 128]); MAX_EVENTS + 512],
                len: 0,
            }),
            out_events: Box::new(OutEvents {
                list: ffi::clap_output_events { ctx: ptr::null_mut(), try_push: Some(out_push) },
                params: Vec::with_capacity(MAX_OUT_EVENTS),
            }),
            pending: Vec::new(),
            changes: Vec::with_capacity(MAX_OUT_EVENTS),
            in_data: Vec::new(),
            out_data: Vec::new(),
            in_ptrs: Vec::new(),
            out_ptrs: Vec::new(),
            in_bufs: Vec::new(),
            out_bufs: Vec::new(),
            steady: 0,
        };
        // SAFETY: a plugin from create_plugin: `init` first, on the main thread
        let ok = unsafe { (*plugin).init.is_some_and(|init| init(plugin)) };
        if !ok {
            // Drop destroys it (never activated)
            return Err(err("clap.init_plugin").arg("id", id));
        }
        inst.ext = Exts {
            params: inst.extension(ffi::CLAP_EXT_PARAMS).cast(),
            latency: inst.extension(ffi::CLAP_EXT_LATENCY).cast(),
            state: inst.extension(ffi::CLAP_EXT_STATE).cast(),
            audio_ports: inst.extension(ffi::CLAP_EXT_AUDIO_PORTS).cast(),
            note_ports: inst.extension(ffi::CLAP_EXT_NOTE_PORTS).cast(),
            gui: inst.extension(ffi::CLAP_EXT_GUI).cast(),
        };
        inst.read_params();
        inst.read_ports();
        inst.activate()?;
        Ok(inst)
    }

    fn plugin(&self) -> &ffi::clap_plugin {
        // SAFETY: non-null since create, valid until destroy in Drop
        unsafe { &*self.plugin }
    }

    fn extension(&self, id: &[u8]) -> *const c_void {
        match self.plugin().get_extension {
            // SAFETY: the plugin's own function with a NUL-terminated id; the result is null or the extension
            Some(get) => unsafe { get(self.plugin, id.as_ptr().cast()) },
            None => ptr::null(),
        }
    }

    fn read_params(&mut self) {
        self.params.clear();
        // SAFETY: null or the plugin's params extension, valid while it lives
        let Some(p) = (unsafe { self.ext.params.as_ref() }) else { return };
        let (Some(count), Some(get_info)) = (p.count, p.get_info) else { return };
        // SAFETY: main thread, the plugin's own function
        let n = unsafe { count(self.plugin) };
        for i in 0..n.min(65_536) {
            // SAFETY: a zeroed info the plugin fills (all-zero is a valid clap_param_info)
            let mut info: ffi::clap_param_info = unsafe { std::mem::zeroed() };
            // SAFETY: an index below the count and a valid out pointer
            if !unsafe { get_info(self.plugin, i, &mut info) } {
                continue;
            }
            let fl = info.flags;
            self.params.push(ParamInfo {
                id: info.id,
                name: text_array(&info.name),
                module: text_array(&info.module),
                min: info.min_value,
                max: info.max_value,
                default: info.default_value,
                stepped: fl & ffi::CLAP_PARAM_IS_STEPPED != 0,
                hidden: fl & ffi::CLAP_PARAM_IS_HIDDEN != 0,
                read_only: fl & ffi::CLAP_PARAM_IS_READONLY != 0,
                automatable: fl & ffi::CLAP_PARAM_IS_AUTOMATABLE != 0,
                bypass: fl & ffi::CLAP_PARAM_IS_BYPASS != 0,
            });
        }
        let n = self.params.len();
        self.pending = Vec::with_capacity(n * 2 + 16);
    }

    fn read_ports(&mut self) {
        let read = |is_input: bool| -> Vec<AudioPort> {
            // SAFETY: null or the plugin's audio-ports extension
            let Some(a) = (unsafe { self.ext.audio_ports.as_ref() }) else {
                // No extension: no ports
                return Vec::new();
            };
            let (Some(count), Some(get)) = (a.count, a.get) else { return Vec::new() };
            // SAFETY: main thread, the plugin's own function
            let n = unsafe { count(self.plugin, is_input) };
            (0..n.min(64))
                .filter_map(|i| {
                    // SAFETY: zeroed info the plugin fills (all-zero is valid: null port_type)
                    let mut info: ffi::clap_audio_port_info = unsafe { std::mem::zeroed() };
                    // SAFETY: index below the count, valid out pointer
                    unsafe { get(self.plugin, i, is_input, &mut info) }.then(|| AudioPort {
                        name: text_array(&info.name),
                        channels: info.channel_count.min(64),
                        main: info.flags & ffi::CLAP_AUDIO_PORT_IS_MAIN != 0,
                    })
                })
                .collect()
        };
        self.inputs = read(true);
        self.outputs = read(false);
        // SAFETY: null or the plugin's note-ports extension
        self.note_inputs = match unsafe { self.ext.note_ports.as_ref() }.and_then(|n| n.count) {
            // SAFETY: main thread, the plugin's own function
            Some(count) => unsafe { count(self.plugin, true) },
            None => 0,
        };
        // The buffers for every port's channels, at fixed addresses from now on
        let max = self.max_block as usize;
        let ins: u32 = self.inputs.iter().map(|p| p.channels).sum();
        let outs: u32 = self.outputs.iter().map(|p| p.channels).sum();
        self.in_data = vec![vec![0.0; max]; ins as usize];
        self.out_data = vec![vec![0.0; max]; outs as usize];
        self.in_ptrs = self.in_data.iter_mut().map(|b| b.as_mut_ptr()).collect();
        self.out_ptrs = self.out_data.iter_mut().map(|b| b.as_mut_ptr()).collect();
        let bufs = |ports: &[AudioPort], ptrs: &mut Vec<*mut f32>| -> Vec<ffi::clap_audio_buffer> {
            let mut at = 0usize;
            ports
                .iter()
                .map(|p| {
                    let b = ffi::clap_audio_buffer {
                        // SAFETY: `at` stays within `ptrs` (it holds the sum of the ports' channels)
                        data32: unsafe { ptrs.as_mut_ptr().add(at) },
                        data64: ptr::null_mut(),
                        channel_count: p.channels,
                        latency: 0,
                        constant_mask: 0,
                    };
                    at += p.channels as usize;
                    b
                })
                .collect()
        };
        self.in_bufs = bufs(&self.inputs, &mut self.in_ptrs);
        self.out_bufs = bufs(&self.outputs, &mut self.out_ptrs);
    }

    fn read_latency(&mut self) {
        // SAFETY: null or the plugin's latency extension; main thread, active
        self.latency = match unsafe { self.ext.latency.as_ref() }.and_then(|l| l.get) {
            // SAFETY: as above
            Some(get) => unsafe { get(self.plugin) },
            None => 0,
        };
    }

    fn activate(&mut self) -> Result<()> {
        let activate = self.plugin().activate.ok_or_else(|| err("clap.activate"))?;
        // SAFETY: main thread, not active
        if !unsafe { activate(self.plugin, self.rate, 1, self.max_block) } {
            return Err(err("clap.activate"));
        }
        self.active = true;
        self.read_latency();
        Ok(())
    }

    fn deactivate(&mut self) {
        if self.active {
            if let Some(d) = self.plugin().deactivate {
                // SAFETY: main thread, active and not processing
                unsafe { d(self.plugin) };
            }
            self.active = false;
        }
    }

    pub fn params(&self) -> &[ParamInfo] {
        &self.params
    }

    pub fn audio_inputs(&self) -> &[AudioPort] {
        &self.inputs
    }

    pub fn audio_outputs(&self) -> &[AudioPort] {
        &self.outputs
    }

    /// It takes notes (a note input port)
    pub fn takes_notes(&self) -> bool {
        self.note_inputs > 0
    }

    pub fn latency(&self) -> u32 {
        self.latency
    }

    pub fn has_gui(&self) -> bool {
        !self.ext.gui.is_null()
    }

    /// A parameter's value now (main thread)
    pub fn param_value(&self, id: u32) -> Option<f64> {
        // SAFETY: null or the params extension
        let get = unsafe { self.ext.params.as_ref() }?.get_value?;
        let mut v = 0.0;
        // SAFETY: main thread, a valid out pointer
        unsafe { get(self.plugin, id, &mut v) }.then_some(v)
    }

    /// Sets a parameter: delivered at the start of the next block. No allocation
    pub fn set_param(&mut self, id: u32, value: f64) {
        if let Some(p) = self.pending.iter_mut().find(|p| p.0 == id) {
            p.1 = value;
        } else if self.pending.len() < self.pending.capacity() {
            self.pending.push((id, value));
        }
    }

    /// One block: `inputs` and `outputs` are the channels of the audio ports in order (main ports first, as the
    /// plugin lists them); missing input channels are silence, extra output channels are not written. Audio
    /// thread. No allocation on the host's side
    pub fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[Event],
        transport: Option<&Transport>,
    ) -> i32 {
        let frames = frames.min(self.max_block as usize);
        let silence = |outputs: &mut [&mut [f32]]| {
            for o in outputs.iter_mut() {
                let n = frames.min(o.len());
                o[..n].fill(0.0);
            }
        };
        // A restart waits for the audio side to stop processing first (CLAP: stop_processing on the audio thread)
        if self.host.flags.restart.load(Ordering::Acquire) || !self.active {
            if self.processing {
                if let Some(stop) = self.plugin().stop_processing {
                    // SAFETY: audio thread, processing
                    unsafe { stop(self.plugin) };
                }
                self.processing = false;
            }
            silence(outputs);
            return ffi::CLAP_PROCESS_CONTINUE;
        }
        if !self.processing {
            let ok = match self.plugin().start_processing {
                // SAFETY: audio thread, active, not processing
                Some(start) => unsafe { start(self.plugin) },
                None => true,
            };
            if !ok {
                silence(outputs);
                return ffi::CLAP_PROCESS_ERROR;
            }
            self.processing = true;
        }
        // Audio in
        for (k, buf) in self.in_data.iter_mut().enumerate() {
            match inputs.get(k) {
                Some(i) => {
                    let n = frames.min(i.len());
                    buf[..n].copy_from_slice(&i[..n]);
                    buf[n..frames].fill(0.0);
                }
                None => buf[..frames].fill(0.0),
            }
        }
        for b in &mut self.out_data {
            b[..frames].fill(0.0);
        }
        // Events: the pending parameters first, then the block's, in order
        let ev = &mut *self.in_events;
        ev.len = 0;
        for (id, value) in self.pending.drain(..) {
            ev.push_event(&Event::Param { time: 0, id, value });
        }
        for e in events.iter().take(MAX_EVENTS) {
            let mut e = *e;
            let (Event::NoteOn { time, .. }
            | Event::NoteOff { time, .. }
            | Event::Choke { time, .. }
            | Event::Expression { time, .. }
            | Event::Param { time, .. }
            | Event::Midi { time, .. }) = &mut e;
            *time = (*time).min(frames.saturating_sub(1) as u32);
            ev.push_event(&e);
        }
        self.out_events.params.clear();
        let t = transport.map(|t| {
            let mut flags = ffi::CLAP_TRANSPORT_HAS_TEMPO
                | ffi::CLAP_TRANSPORT_HAS_BEATS_TIMELINE
                | ffi::CLAP_TRANSPORT_HAS_SECONDS_TIMELINE;
            if t.playing {
                flags |= ffi::CLAP_TRANSPORT_IS_PLAYING;
            }
            if t.recording {
                flags |= ffi::CLAP_TRANSPORT_IS_RECORDING;
            }
            let (num, den) = t.signature.unwrap_or((4, 4));
            if t.signature.is_some() {
                flags |= ffi::CLAP_TRANSPORT_HAS_TIME_SIGNATURE;
            }
            ffi::clap_event_transport {
                header: header(0, ffi::CLAP_EVENT_TRANSPORT, std::mem::size_of::<ffi::clap_event_transport>()),
                flags,
                song_pos_beats: (t.beats * ffi::CLAP_BEATTIME_FACTOR) as i64,
                song_pos_seconds: (t.seconds * ffi::CLAP_SECTIME_FACTOR) as i64,
                tempo: t.tempo,
                tempo_inc: 0.0,
                loop_start_beats: 0,
                loop_end_beats: 0,
                loop_start_seconds: 0,
                loop_end_seconds: 0,
                bar_start: 0,
                bar_number: 0,
                tsig_num: num,
                tsig_denom: den,
            }
        });
        // The lists the plugin reads and pushes into, by raw pointer (the push callback writes through it)
        let in_list: *const ffi::clap_input_events = &raw const self.in_events.list;
        let out_ptr: *mut OutEvents = &raw mut *self.out_events;
        // SAFETY: a field of the boxed OutEvents, which outlives the call
        let out_list: *const ffi::clap_output_events = unsafe { &raw const (*out_ptr).list };
        let process = ffi::clap_process {
            steady_time: self.steady,
            frames_count: frames as u32,
            transport: t.as_ref().map_or(ptr::null(), |t| t as *const _),
            audio_inputs: self.in_bufs.as_ptr(),
            audio_outputs: self.out_bufs.as_mut_ptr(),
            audio_inputs_count: self.in_bufs.len() as u32,
            audio_outputs_count: self.out_bufs.len() as u32,
            in_events: in_list,
            out_events: out_list,
        };
        let status = match self.plugin().process {
            // SAFETY: audio thread, processing; every pointer in `process` is valid for the call (buffers sized
            // max_block, event lists owned by self)
            Some(p) => unsafe { p(self.plugin, &process) },
            None => ffi::CLAP_PROCESS_ERROR,
        };
        self.steady += frames as i64;
        for (k, o) in outputs.iter_mut().enumerate() {
            let n = frames.min(o.len());
            match self.out_data.get(k) {
                Some(b) => o[..n].copy_from_slice(&b[..n]),
                None => o[..n].fill(0.0),
            }
        }
        self.changes.clear();
        let room = self.changes.capacity();
        self.changes.extend(self.out_events.params.iter().take(room).copied());
        status
    }

    /// The plugin's own parameter changes of the last block (its window, a preset): (id, value)
    pub fn changes(&self) -> &[(u32, f64)] {
        &self.changes
    }

    /// The plugin said its state changed since the last call
    pub fn take_state_dirty(&self) -> bool {
        self.host.flags.state_dirty.swap(false, Ordering::AcqRel)
    }

    /// The plugin's window was closed by the user since the last call
    pub fn take_gui_closed(&self) -> bool {
        self.host.flags.gui_closed.swap(false, Ordering::AcqRel)
    }

    /// The main thread's turn (call it often, ~every 10 ms): the plugin's callback, a restart the audio side has
    /// stopped for (deactivate, activate, read the latency and ports again), a parameter rescan. `true` when the
    /// latency, the parameters or the ports changed
    pub fn idle(&mut self) -> bool {
        let mut changed = false;
        if self.host.flags.callback.swap(false, Ordering::AcqRel)
            && let Some(cb) = self.plugin().on_main_thread
        {
            // SAFETY: main thread, the plugin's own function
            unsafe { cb(self.plugin) };
        }
        if self.host.flags.params_rescan.swap(false, Ordering::AcqRel) {
            let before = self.params.clone();
            self.read_params();
            changed |= before != self.params;
        }
        if self.host.flags.restart.load(Ordering::Acquire) && !self.processing {
            let before = (self.latency, self.inputs.clone(), self.outputs.clone());
            self.deactivate();
            if self.host.flags.ports_rescan.swap(false, Ordering::AcqRel) {
                self.read_ports();
            }
            self.host.flags.restart.store(false, Ordering::Release);
            if self.activate().is_err() {
                return true;
            }
            changed |= before != (self.latency, self.inputs.clone(), self.outputs.clone());
        }
        if self.host.flags.latency.swap(false, Ordering::AcqRel) {
            // CLAP: a latency change is applied by a restart; one the plugin did not ask for is asked here
            self.host.flags.restart.store(true, Ordering::Release);
        }
        changed
    }

    /// Whether a restart waits for the audio side to stop (then [`Instance::idle`] does it)
    pub fn restart_pending(&self) -> bool {
        self.host.flags.restart.load(Ordering::Acquire)
    }

    /// The plugin's state (main thread)
    pub fn save_state(&mut self) -> Result<Option<Vec<u8>>> {
        // SAFETY: null or the plugin's state extension
        let Some(save) = unsafe { self.ext.state.as_ref() }.and_then(|s| s.save) else { return Ok(None) };
        let mut out: Vec<u8> = Vec::new();
        let stream = ffi::clap_ostream { ctx: (&raw mut out).cast(), write: Some(stream_write) };
        // SAFETY: main thread; the stream and its ctx live for the call
        if unsafe { save(self.plugin, &stream) } { Ok(Some(out)) } else { Err(err("clap.state.save")) }
    }

    /// Gives the plugin a state (main thread); the parameters' values follow it
    pub fn load_state(&mut self, chunk: &[u8]) -> Result<()> {
        // SAFETY: null or the plugin's state extension
        let Some(load) = unsafe { self.ext.state.as_ref() }.and_then(|s| s.load) else {
            return Err(err("clap.state.none"));
        };
        let mut cursor = Reader { data: chunk, at: 0 };
        let stream = ffi::clap_istream { ctx: (&raw mut cursor).cast(), read: Some(stream_read) };
        // SAFETY: main thread; the stream and its ctx live for the call
        if unsafe { load(self.plugin, &stream) } { Ok(()) } else { Err(err("clap.state.load")) }
    }

    /// Whether the plugin can show its own floating window with this window system (`"cocoa"`, `"x11"`)
    pub fn gui_supported(&self, api: &str, floating: bool) -> bool {
        let Ok(c) = CString::new(api) else { return false };
        // SAFETY: null or the plugin's gui extension
        match unsafe { self.ext.gui.as_ref() }.and_then(|g| g.is_api_supported) {
            // SAFETY: main thread, a NUL-terminated api name
            Some(f) => unsafe { f(self.plugin, c.as_ptr(), floating) },
            None => false,
        }
    }

    /// Makes and shows the plugin's floating window (main thread, the helper's GUI loop running)
    pub fn gui_open(&mut self, api: &str, title: &str) -> Result<()> {
        // SAFETY: null or the plugin's gui extension
        let g = unsafe { self.ext.gui.as_ref() }.ok_or_else(|| err("clap.gui.none"))?;
        let api = CString::new(api).map_err(|_| err("clap.gui.api"))?;
        let title = CString::new(title).unwrap_or_default();
        let (Some(create), Some(show)) = (g.create, g.show) else { return Err(err("clap.gui.none")) };
        // SAFETY: main thread; a floating window needs no parent
        if !unsafe { create(self.plugin, api.as_ptr(), true) } {
            return Err(err("clap.gui.create"));
        }
        if let Some(t) = g.suggest_title {
            // SAFETY: main thread, NUL-terminated title
            unsafe { t(self.plugin, title.as_ptr()) };
        }
        // SAFETY: main thread, created
        if !unsafe { show(self.plugin) } {
            if let Some(d) = g.destroy {
                // SAFETY: main thread, created
                unsafe { d(self.plugin) };
            }
            return Err(err("clap.gui.show"));
        }
        Ok(())
    }

    /// Makes the plugin's gui embedded in the native window `handle` (an `NSView` with `"cocoa"`, an X11 window id with
    /// `"x11"`) and shows it; returns its size (main thread)
    pub fn gui_open_embedded(&mut self, api: &str, handle: usize) -> Result<(u32, u32)> {
        // SAFETY: null or the plugin's gui extension
        let g = unsafe { self.ext.gui.as_ref() }.ok_or_else(|| err("clap.gui.none"))?;
        let api = CString::new(api).map_err(|_| err("clap.gui.api"))?;
        let (Some(create), Some(show), Some(set_parent)) = (g.create, g.show, g.set_parent) else {
            return Err(err("clap.gui.none"));
        };
        // SAFETY: main thread; an embedded window
        if !unsafe { create(self.plugin, api.as_ptr(), false) } {
            return Err(err("clap.gui.create"));
        }
        let window = ffi::clap_window { api: api.as_ptr(), handle };
        // SAFETY: main thread, created; the window names a live native window of that api
        let attached = unsafe { set_parent(self.plugin, &window) };
        let size = self.gui_size();
        // SAFETY: main thread, created
        if !attached || !unsafe { show(self.plugin) } {
            if let Some(d) = g.destroy {
                // SAFETY: main thread, created
                unsafe { d(self.plugin) };
            }
            return Err(err("clap.gui.show"));
        }
        self.host.flags.gui_resize.store(0, Ordering::Release);
        Ok(size.unwrap_or((640, 480)))
    }

    /// The plugin's gui size now (main thread)
    pub fn gui_size(&self) -> Option<(u32, u32)> {
        // SAFETY: null or the plugin's gui extension
        let g = unsafe { self.ext.gui.as_ref() }?;
        let (mut w, mut h) = (0u32, 0u32);
        // SAFETY: main thread, two out values
        (unsafe { g.get_size?(self.plugin, &mut w, &mut h) }).then_some((w, h))
    }

    /// The window was resized: the gui is told and answers the size it takes (it keeps its own when it cannot resize);
    /// main thread
    pub fn gui_resize(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        // SAFETY: null or the plugin's gui extension
        let g = unsafe { self.ext.gui.as_ref() }?;
        // SAFETY: main thread
        let can = match g.can_resize {
            Some(f) => unsafe { f(self.plugin) },
            None => false,
        };
        if can {
            let (mut w, mut h) = (width, height);
            if let Some(adjust) = g.adjust_size {
                // SAFETY: main thread, in/out sizes
                unsafe { adjust(self.plugin, &mut w, &mut h) };
            }
            if let Some(set) = g.set_size {
                // SAFETY: main thread
                unsafe { set(self.plugin, w, h) };
            }
        }
        self.gui_size()
    }

    /// The size the plugin's gui asked the host for by itself, once (main thread)
    pub fn take_gui_resize(&self) -> Option<(u32, u32)> {
        let v = self.host.flags.gui_resize.swap(0, Ordering::AcqRel);
        (v != 0).then_some(((v >> 32) as u32, v as u32))
    }

    /// Hides and destroys the plugin's window (main thread)
    pub fn gui_close(&mut self) {
        // SAFETY: null or the plugin's gui extension
        if let Some(g) = unsafe { self.ext.gui.as_ref() } {
            if let Some(h) = g.hide {
                // SAFETY: main thread
                unsafe { h(self.plugin) };
            }
            if let Some(d) = g.destroy {
                // SAFETY: main thread
                unsafe { d(self.plugin) };
            }
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

unsafe extern "C" fn stream_write(stream: *const ffi::clap_ostream, buffer: *const c_void, size: u64) -> i64 {
    guard(-1, || {
        if buffer.is_null() {
            return -1;
        }
        // SAFETY: `ctx` is the `Vec<u8>` of `save_state`, alive and not otherwise borrowed during the call
        let out = unsafe { &mut *((*stream).ctx as *mut Vec<u8>) };
        // The plugin decides the sizes: the whole state is bounded, so it cannot grow the host without end
        const MAX_STATE: usize = 64 << 20;
        if out.len() >= MAX_STATE && size > 0 {
            return -1;
        }
        let n = (size.min(1 << 30) as usize).min(MAX_STATE - out.len());
        // SAFETY: the plugin gives `size` readable bytes
        let bytes = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n) };
        out.extend_from_slice(bytes);
        n as i64
    })
}

unsafe extern "C" fn stream_read(stream: *const ffi::clap_istream, buffer: *mut c_void, size: u64) -> i64 {
    guard(-1, || {
        if buffer.is_null() {
            return -1;
        }
        // SAFETY: `ctx` is the `Reader` of `load_state`, alive during the call
        let r = unsafe { &mut *((*stream).ctx as *mut Reader<'_>) };
        let n = (size as usize).min(r.data.len() - r.at);
        // SAFETY: the plugin gives `size` writable bytes; n ≤ size
        unsafe { ptr::copy_nonoverlapping(r.data[r.at..].as_ptr(), buffer.cast::<u8>(), n) };
        r.at += n;
        n as i64
    })
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.processing {
            // CLAP wants stop_processing on the audio thread: the instance is being dropped after the audio side
            // ended, so this thread acts as it
            let was = role();
            set_thread_role(ThreadRole::Audio);
            if let Some(stop) = self.plugin().stop_processing {
                // SAFETY: processing; no other thread uses the instance any more
                unsafe { stop(self.plugin) };
            }
            set_thread_role(was);
            self.processing = false;
        }
        self.deactivate();
        if let Some(destroy) = self.plugin().destroy {
            // SAFETY: main thread, not active; the plugin is not used after this
            unsafe { destroy(self.plugin) };
        }
    }
}

/// Where CLAP plugins are installed: `CLAP_PATH`, then the user's and the system's folders
pub fn search_paths() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> =
        std::env::var_os("CLAP_PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            out.push(h.join("Library/Audio/Plug-Ins/CLAP"));
        }
        out.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
    } else {
        if let Some(h) = &home {
            out.push(h.join(".clap"));
        }
        out.push(PathBuf::from("/usr/lib/clap"));
        out.push(PathBuf::from("/usr/local/lib/clap"));
    }
    out
}

/// The `.clap` files under `dirs` (recursively, bundles not entered); no plugin code runs
pub fn find_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        if depth > 8 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.extension().is_some_and(|e| e == "clap") {
                out.push(p);
            } else if p.is_dir() {
                walk(&p, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        if d.extension().is_some_and(|e| e == "clap") && d.exists() {
            out.push(d.clone());
        } else {
            walk(d, 0, &mut out);
        }
    }
    out.dedup();
    out
}

/// Builds the test plugins (`tests/plugin/lm_test_clap.c`) into `dir` with the system's C compiler and the
/// vendored headers: `dir/lm_test.clap`. For the tests of this crate and of the helper
#[cfg(feature = "testing")]
pub fn build_test_plugin(dir: &Path) -> Result<PathBuf> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = dir.join("lm_test.clap");
    std::fs::create_dir_all(dir).map_err(|e| err("clap.test.build").arg("error", e.to_string()))?;
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = std::process::Command::new(cc);
    cmd.arg("-shared").arg("-fPIC").arg("-O1").arg("-Wall").arg("-Werror");
    cmd.arg("-I").arg(here.join("clap/include"));
    if cfg!(target_os = "macos") {
        cmd.arg("-undefined").arg("dynamic_lookup");
    }
    cmd.arg("-o").arg(&out).arg(here.join("tests/plugin/lm_test_clap.c"));
    let status = cmd.status().map_err(|e| err("clap.test.build").arg("error", e.to_string()))?;
    if !status.success() {
        return Err(err("clap.test.build").arg("status", status.to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    #[test]
    fn a_plugin_cannot_grow_the_saved_state_without_end() {
        let mut out: Vec<u8> = Vec::new();
        let stream = ffi::clap_ostream { ctx: (&raw mut out).cast(), write: Some(stream_write) };
        let chunk = vec![0u8; 1 << 20];
        let mut total = 0i64;
        for _ in 0..80 {
            // SAFETY: a live stream over `out`, a readable buffer of the size given
            let n = unsafe { stream_write(&stream, chunk.as_ptr().cast(), chunk.len() as u64) };
            if n < 0 {
                break;
            }
            total += n;
        }
        assert_eq!(total, 64 << 20);
        assert_eq!(out.len(), 64 << 20);
    }
}
