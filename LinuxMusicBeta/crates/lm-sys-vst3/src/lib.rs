//! Safe binding for hosting VST3 plugins, on the `vst3` crate's bindings (generated from the SDK's headers, MIT OR
//! Apache-2.0; the VST 3 SDK itself is MIT since 2025).
//!
//! VST3 plugins are third-party plugins: they run **out of the main process**, in the plugin-host helper (interfaces.md
//! I-9.6). This crate is what that helper links.
//!
//! - [`Module`]: a `.vst3` bundle (or its shared library) opened, its entry called (`ModuleEntry` on Linux,
//!   `bundleEntry` with a `CFBundle` on macOS), its audio module classes listed ([`Module::classes`]).
//! - [`Instance`]: one plugin's component and controller made, initialised with a host application of this crate,
//!   connected to each other, the controller synchronised with the component's state, the buses activated, the
//!   processing set up and the component activated. Its parameters ([`Instance::params`], normalized 0..1), buses,
//!   latency, one block ([`Instance::process`]: audio, notes, note expressions, parameter changes, MIDI controllers
//!   through the controller's MIDI mapping, the transport), the plugin's own parameter changes back
//!   ([`Instance::changes`]: the processor's output changes and the controller's edits), its state as bytes (the
//!   component's and the controller's), and [`Instance::idle`], the main-thread turn (restarts, the controller kept in
//!   step with the values the host set).
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | The `dlopen` handle and the module entry: one [`Module`] (an `Arc` shared by its instances); `Drop` releases the factory, calls the module's exit and `dlclose`s (and releases the `CFBundle` on macOS). Each COM object is held by a `ComPtr`, released in `Drop` in the SDK's order: stop processing, deactivate, disconnect the two halves, terminate the controller then the component, release. The host's objects (`host.rs`) are reference-counted COM objects of ours; the plugin may keep references to the host application and the handler, so they live as long as it does. Strings and infos are copied into owned Rust data. |
//! | Which thread may call each function? | [`Instance::process`] is the audio thread; everything else on `Instance` is the "UI" (main) thread of VST3. `Instance` is `Send`, not `Sync`, and every call takes `&mut self`: the two never run at once. Restarts are done by `idle` after the audio side stopped processing at its next block. `Module` is `Send` and `Sync` (a factory is thread-safe by the SDK). |
//! | Allocates, locks, does I/O? | `process` does not allocate or lock on the host's side: the change lists, queues, event lists and buffers are preallocated with fixed capacity (`tests/instance.rs` runs it under `assert_no_alloc`); the controller's edits come through a lock-free ring (`rtrb`). Everything else may allocate and block. |
//! | Creates threads? | No. A plugin may create its own. |
//! | Calls back into our code? | Yes, from plugin code, through the COM classes of `host.rs` (any thread for the handler's `performEdit` and `restartComponent`; the process thread for the change lists and the event list; the main thread for the streams, messages and attribute lists). They never unwind across the boundary: their bodies only touch their own fields. |
//! | Tests | `examples/test_vst3.rs` builds the test plugins of [`testing`] (a gain with a separate controller connected through messages, parameters, latency and a restart, state; a synth with an event bus, note expressions, the transport, its own output parameter and a MIDI mapping) as a shared library. `tests/instance.rs` drives them through the binding. |

mod host;
#[cfg(feature = "testing")]
pub mod testing;

use std::ffi::{CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use lm_base::{Error, Result};
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{ComPtr, ComWrapper, Interface};

use host::{Changes, Edit, Events, Frame, Handler, HostApp, Requests, Stream, narrow, wide};

fn err(key: &'static str) -> Error {
    Error::new(key)
}

fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: a NUL-terminated string from the C library, read at once
    unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// A class id as 32 hex digits (the plugin's identity, its `uri`)
pub fn cid_hex(cid: &TUID) -> String {
    cid.iter().map(|b| format!("{:02X}", *b as u8)).collect()
}

fn cid_parse(hex: &str) -> Option<TUID> {
    if hex.len() != 32 {
        return None;
    }
    let mut out: TUID = [0; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()? as c_char;
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------------------------
// The module

/// An audio module class of a `.vst3`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassInfo {
    /// The class id, 32 hex digits
    pub cid: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// `"Fx|Delay"`, `"Instrument|Synth"`... (empty when the factory has no `IPluginFactory2`)
    pub subcategories: String,
}

impl ClassInfo {
    pub fn is_instrument(&self) -> bool {
        self.subcategories.split('|').any(|c| c == "Instrument")
    }
}

type EntryFn = unsafe extern "system" fn(*mut c_void) -> bool;
type ExitFn = unsafe extern "system" fn() -> bool;
type FactoryFn = unsafe extern "system" fn() -> *mut IPluginFactory;

/// An opened `.vst3`
pub struct Module {
    handle: *mut c_void,
    factory: Option<ComPtr<IPluginFactory>>,
    exit: Option<ExitFn>,
    bundle: *const c_void,
    path: PathBuf,
}

// SAFETY: a VST3 factory may be used from any thread (the SDK); the handle is only closed in Drop
unsafe impl Send for Module {}
// SAFETY: as above: the methods only call the factory
unsafe impl Sync for Module {}

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Module").field("path", &self.path).finish_non_exhaustive()
    }
}

/// The shared library of a `.vst3` path: the file itself, or the bundle's binary for this system
fn binary_of(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![path.join("Contents/MacOS").join(&stem)]
    } else {
        let arch = if cfg!(target_arch = "aarch64") { "aarch64-linux" } else { "x86_64-linux" };
        vec![path.join("Contents").join(arch).join(format!("{stem}.so"))]
    };
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| err("vst3.not_found").arg("path", path.display().to_string()))
}

#[cfg(target_os = "macos")]
mod cf {
    use std::ffi::c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFURLCreateFromFileSystemRepresentation(
            alloc: *const c_void,
            buffer: *const u8,
            len: isize,
            is_directory: u8,
        ) -> *const c_void;
        pub fn CFBundleCreate(alloc: *const c_void, url: *const c_void) -> *const c_void;
        pub fn CFRelease(cf: *const c_void);
    }
}

fn sym(handle: *mut c_void, name: &std::ffi::CStr) -> *mut c_void {
    // SAFETY: a valid handle and a NUL-terminated name
    unsafe { libc::dlsym(handle, name.as_ptr()) }
}

impl Module {
    /// Opens a `.vst3` and calls its entry. Runs the plugin's code: do it in the helper
    pub fn open(path: &Path) -> Result<Arc<Module>> {
        let binary = binary_of(path)?;
        let c = CString::new(binary.as_os_str().to_string_lossy().as_bytes())
            .map_err(|_| err("vst3.path").arg("path", path.display().to_string()))?;
        // SAFETY: a NUL-terminated path; dlopen runs the library's constructors (plugin code, in the helper)
        let handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            // SAFETY: dlerror's message, read at once
            let why = unsafe { libc::dlerror() };
            return Err(err("vst3.open").arg("path", path.display().to_string()).arg("error", text(why)));
        }
        let mut module = Module { handle, factory: None, exit: None, bundle: ptr::null(), path: path.to_path_buf() };
        // The entry: ModuleEntry(handle) on Linux; bundleEntry(CFBundleRef) on macOS
        let (entry, exit) = if cfg!(target_os = "macos") {
            let e = Some(sym(handle, c"bundleEntry"))
                .filter(|p| !p.is_null())
                .or_else(|| Some(sym(handle, c"BundleEntry")));
            let x =
                Some(sym(handle, c"bundleExit")).filter(|p| !p.is_null()).or_else(|| Some(sym(handle, c"BundleExit")));
            (e.unwrap_or(ptr::null_mut()), x.unwrap_or(ptr::null_mut()))
        } else {
            (sym(handle, c"ModuleEntry"), sym(handle, c"ModuleExit"))
        };
        let arg: *mut c_void = if cfg!(target_os = "macos") { module.make_bundle(path) as *mut c_void } else { handle };
        if !entry.is_null() {
            // SAFETY: the module's entry by the VST3 ABI, called once before the factory
            let f: EntryFn = unsafe { std::mem::transmute::<*mut c_void, EntryFn>(entry) };
            // SAFETY: as above
            if !unsafe { f(arg) } {
                return Err(err("vst3.entry").arg("path", path.display().to_string()));
            }
        }
        if !exit.is_null() {
            // SAFETY: the module's exit by the VST3 ABI
            module.exit = Some(unsafe { std::mem::transmute::<*mut c_void, ExitFn>(exit) });
        }
        let get = sym(handle, c"GetPluginFactory");
        if get.is_null() {
            return Err(err("vst3.no_factory").arg("path", path.display().to_string()));
        }
        // SAFETY: GetPluginFactory by the VST3 ABI
        let get: FactoryFn = unsafe { std::mem::transmute::<*mut c_void, FactoryFn>(get) };
        // SAFETY: as above; the factory comes with one reference, now ours
        let factory = unsafe { ComPtr::from_raw(get()) };
        module.factory = Some(factory.ok_or_else(|| err("vst3.no_factory").arg("path", path.display().to_string()))?);
        Ok(Arc::new(module))
    }

    #[cfg(target_os = "macos")]
    fn make_bundle(&mut self, path: &Path) -> *const c_void {
        if !path.is_dir() {
            return ptr::null();
        }
        let bytes = path.as_os_str().to_string_lossy().into_owned();
        // SAFETY: a path buffer of the given length; CoreFoundation copies it
        let url = unsafe {
            cf::CFURLCreateFromFileSystemRepresentation(ptr::null(), bytes.as_ptr(), bytes.len() as isize, 1)
        };
        if url.is_null() {
            return ptr::null();
        }
        // SAFETY: a valid URL; the bundle is released in Drop
        let b = unsafe { cf::CFBundleCreate(ptr::null(), url) };
        // SAFETY: the URL made above, released once
        unsafe { cf::CFRelease(url) };
        self.bundle = b;
        b
    }

    #[cfg(not(target_os = "macos"))]
    fn make_bundle(&mut self, _path: &Path) -> *const c_void {
        ptr::null()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn factory(&self) -> &ComPtr<IPluginFactory> {
        self.factory.as_ref().expect("set at open")
    }

    /// The audio module classes (the plugins)
    pub fn classes(&self) -> Vec<ClassInfo> {
        let f = self.factory();
        let f2 = f.cast::<IPluginFactory2>();
        // SAFETY: the factory's own methods
        let n = unsafe { f.countClasses() };
        let mut out = Vec::new();
        for i in 0..n.clamp(0, 4096) {
            // SAFETY: zeroed info (valid: plain arrays), filled by the factory
            let mut info: PClassInfo = unsafe { std::mem::zeroed() };
            // SAFETY: index below the count, valid out pointer
            if unsafe { f.getClassInfo(i, &mut info) } != kResultOk {
                continue;
            }
            if narrow(&info.category) != "Audio Module Class" {
                continue;
            }
            let mut c = ClassInfo {
                cid: cid_hex(&info.cid),
                name: narrow(&info.name),
                vendor: String::new(),
                version: String::new(),
                subcategories: String::new(),
            };
            if let Some(f2) = &f2 {
                // SAFETY: zeroed info, filled by the factory
                let mut i2: PClassInfo2 = unsafe { std::mem::zeroed() };
                // SAFETY: as above
                if unsafe { f2.getClassInfo2(i, &mut i2) } == kResultOk {
                    c.vendor = narrow(&i2.vendor);
                    c.version = narrow(&i2.version);
                    c.subcategories = narrow(&i2.subCategories);
                }
            }
            out.push(c);
        }
        out
    }

    fn create<I: Interface>(&self, cid: &TUID) -> Option<ComPtr<I>> {
        let mut obj: *mut c_void = ptr::null_mut();
        let iid = I::IID;
        // SAFETY: the factory's createInstance with a class id and an interface id; the result has one reference
        let r = unsafe { self.factory().createInstance(cid.as_ptr(), iid.as_ptr() as FIDString, &mut obj) };
        if r != kResultOk || obj.is_null() {
            return None;
        }
        // SAFETY: an object of interface I with one reference, now ours
        unsafe { ComPtr::from_raw(obj as *mut I) }
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        self.factory = None;
        if let Some(exit) = self.exit {
            // SAFETY: the module's exit, once, after every instance (they hold the Arc) and the factory are gone
            unsafe { exit() };
        }
        #[cfg(target_os = "macos")]
        if !self.bundle.is_null() {
            // SAFETY: the bundle made at open, released once
            unsafe { cf::CFRelease(self.bundle) };
        }
        // SAFETY: the handle from dlopen, closed once
        unsafe { libc::dlclose(self.handle) };
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Events and the transport

/// A note expression, in VST3's normalized values
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expression {
    /// 0..1, linear gain ×4 at 1 (0.25 is unity)
    Volume,
    /// 0 left, 0.5 center, 1 right
    Pan,
    /// 0.5 none; ±120 semitones at 0 and 1
    Tuning,
    /// 0..1: the note's pressure (VST3 has no pressure type: the generic expression type)
    Pressure,
    /// 0..1: the note's brightness (the timbre of MPE, CC 74)
    Brightness,
}

/// An event for the plugin in one block (`time`: the frame in the block)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    NoteOn {
        time: u32,
        note_id: i32,
        channel: i16,
        key: i16,
        velocity: f32,
    },
    NoteOff {
        time: u32,
        note_id: i32,
        channel: i16,
        key: i16,
        velocity: f32,
    },
    /// Every held note ends (the host keeps them)
    ReleaseAll {
        time: u32,
    },
    Expression {
        time: u32,
        kind: Expression,
        note_id: i32,
        value: f64,
    },
    /// A normalized value
    Param {
        time: u32,
        id: u32,
        value: f64,
    },
    /// A channel message: controllers, pitch bend and channel pressure go through the controller's MIDI mapping
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
    pub samples: i64,
    /// Quarter notes from the start
    pub beats: f64,
    pub tempo: f64,
    pub signature: Option<(i32, i32)>,
}

// ---------------------------------------------------------------------------------------------------------------
// The instance

/// One parameter (values are normalized 0..1)
#[derive(Clone, Debug, PartialEq)]
pub struct ParamInfo {
    pub id: u32,
    pub title: String,
    pub units: String,
    /// 0: continuous; n: n + 1 discrete values
    pub steps: i32,
    pub default: f64,
    pub hidden: bool,
    pub read_only: bool,
    pub automatable: bool,
    pub bypass: bool,
}

/// One audio bus
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bus {
    pub name: String,
    pub channels: u32,
    pub main: bool,
}

/// The most events in one block
pub const MAX_EVENTS: usize = 4096;
const MAX_HELD: usize = 512;
const MIDI_CONTROLLERS: usize = 130;

/// One plugin, set up and active
pub struct Instance {
    _module: Arc<Module>,
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    /// The controller is a separate object (terminated on its own)
    separate: bool,
    points: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    _host: ComWrapper<HostApp>,
    _handler: ComWrapper<Handler>,
    requests: Arc<Requests>,
    edits: rtrb::Consumer<Edit>,
    /// The window's gesture edges of the last block: (parameter id, `true` for begin, `false` for end)
    gestures: Vec<(u32, bool)>,
    editor: Option<Editor>,
    rate: f64,
    max_block: u32,
    active: bool,
    processing: bool,
    latency: u32,
    params: Vec<ParamInfo>,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
    event_inputs: i32,
    /// channel × controller → parameter (the controller's MIDI mapping), `u32::MAX` for none
    midi_map: Vec<u32>,
    // Preallocated for `process`
    in_changes: ComWrapper<Changes>,
    out_changes: ComWrapper<Changes>,
    in_events: ComWrapper<Events>,
    out_events: ComWrapper<Events>,
    pending: Vec<(u32, f64)>,
    /// What the host set, for the controller at the next `idle`
    to_controller: Vec<(u32, f64)>,
    changes: Vec<(u32, f64)>,
    held: Vec<(i32, i16, i16)>,
    in_data: Vec<Vec<f32>>,
    out_data: Vec<Vec<f32>>,
    in_ptrs: Vec<*mut f32>,
    out_ptrs: Vec<*mut f32>,
    in_bufs: Vec<AudioBusBuffers>,
    out_bufs: Vec<AudioBusBuffers>,
    context: Box<ProcessContext>,
}

// SAFETY: VST3 objects may be called from different threads as long as the threading rules are kept: `Instance` is
// not `Sync` and every call takes `&mut self`, so the audio-thread call (`process`) and the main-thread calls never
// overlap
unsafe impl Send for Instance {}

impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instance").field("latency", &self.latency).field("params", &self.params.len()).finish()
    }
}

fn ok(r: tresult) -> bool {
    r == kResultOk
}

impl Instance {
    /// Makes the class `cid` (32 hex digits) of `module`, set up for `rate` and blocks up to `max_block` frames, and
    /// activates it. Main thread
    pub fn new(module: &Arc<Module>, cid: &str, rate: f64, max_block: u32) -> Result<Instance> {
        let id = cid_parse(cid).ok_or_else(|| err("vst3.not_found").arg("cid", cid))?;
        let component: ComPtr<IComponent> = module.create(&id).ok_or_else(|| err("vst3.not_found").arg("cid", cid))?;
        let host = ComWrapper::new(HostApp);
        let host_unknown = host.to_com_ptr::<FUnknown>().ok_or_else(|| err("vst3.host"))?;
        // SAFETY: a new component, initialised once with our host application
        if !ok(unsafe { component.initialize(host_unknown.as_ptr()) }) {
            return Err(err("vst3.initialize").arg("cid", cid));
        }
        let terminate = |c: &ComPtr<IComponent>| {
            // SAFETY: an initialised component
            unsafe { c.terminate() };
        };
        let processor = match component.cast::<IAudioProcessor>() {
            Some(p) => p,
            None => {
                terminate(&component);
                return Err(err("vst3.not_audio").arg("cid", cid));
            }
        };
        // The controller: the component itself, or its own class
        let (controller, separate) = match component.cast::<IEditController>() {
            Some(c) => (Some(c), false),
            None => {
                let mut ccid: TUID = [0; 16];
                // SAFETY: the component's method with an out TUID
                let c = if ok(unsafe { component.getControllerClassId(&mut ccid) }) {
                    module.create::<IEditController>(&ccid)
                } else {
                    None
                };
                // SAFETY: a new controller, initialised once with our host application
                let c = c.filter(|c| ok(unsafe { c.initialize(host_unknown.as_ptr()) }));
                (c, true)
            }
        };
        let requests = Arc::new(Requests::default());
        let (producer, consumer) = rtrb::RingBuffer::new(1024);
        let handler =
            ComWrapper::new(Handler { requests: Arc::clone(&requests), edits: std::sync::Mutex::new(producer) });
        let mut points = None;
        if let Some(c) = &controller {
            if separate && let (Some(a), Some(b)) = (component.cast::<IConnectionPoint>(), c.cast::<IConnectionPoint>())
            {
                // SAFETY: the two halves' connection points, connected both ways (disconnected in Drop)
                unsafe {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                }
                points = Some((a, b));
            }
            if let Some(h) = handler.to_com_ptr::<IComponentHandler>() {
                // SAFETY: the controller keeps a reference to our handler
                unsafe { c.setComponentHandler(h.as_ptr()) };
            }
            // The controller learns the component's state
            let s = Stream::new(Vec::new());
            if let Some(sp) = s.to_com_ptr::<IBStream>() {
                // SAFETY: main thread, our stream
                if ok(unsafe { component.getState(sp.as_ptr()) }) {
                    s.at.set(0);
                    // SAFETY: as above
                    unsafe { c.setComponentState(sp.as_ptr()) };
                }
            }
        }
        let mut inst = Instance {
            _module: Arc::clone(module),
            component,
            processor,
            controller,
            separate,
            points,
            _host: host,
            _handler: handler,
            requests,
            edits: consumer,
            gestures: Vec::with_capacity(256),
            editor: None,
            rate,
            max_block: max_block.max(1),
            active: false,
            processing: false,
            latency: 0,
            params: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            event_inputs: 0,
            midi_map: Vec::new(),
            in_changes: ComWrapper::new(Changes::new(1)),
            out_changes: ComWrapper::new(Changes::new(1)),
            in_events: ComWrapper::new(Events { list: std::cell::RefCell::new(Vec::with_capacity(MAX_EVENTS)) }),
            out_events: ComWrapper::new(Events { list: std::cell::RefCell::new(Vec::with_capacity(1024)) }),
            pending: Vec::new(),
            to_controller: Vec::new(),
            changes: Vec::with_capacity(1024),
            held: Vec::with_capacity(MAX_HELD),
            in_data: Vec::new(),
            out_data: Vec::new(),
            in_ptrs: Vec::new(),
            out_ptrs: Vec::new(),
            in_bufs: Vec::new(),
            out_bufs: Vec::new(),
            // SAFETY: zeroed ProcessContext is valid (plain numbers and a plain Chord struct)
            context: Box::new(unsafe { std::mem::zeroed() }),
        };
        inst.read_params();
        inst.setup_buses();
        inst.read_midi_map();
        inst.activate()?;
        Ok(inst)
    }

    fn read_params(&mut self) {
        self.params.clear();
        let Some(c) = &self.controller else { return };
        // SAFETY: main thread, the controller's own methods
        let n = unsafe { c.getParameterCount() };
        for i in 0..n.clamp(0, 65_536) {
            // SAFETY: zeroed info (valid: plain data), filled by the controller
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            // SAFETY: an index below the count, a valid out pointer
            if !ok(unsafe { c.getParameterInfo(i, &mut info) }) {
                continue;
            }
            let fl = info.flags as u32;
            let has = |f: u32| fl & f != 0;
            self.params.push(ParamInfo {
                id: info.id,
                title: wide(&info.title),
                units: wide(&info.units),
                steps: info.stepCount,
                default: info.defaultNormalizedValue,
                hidden: has(ParameterInfo_::ParameterFlags_::kIsHidden as u32),
                read_only: has(ParameterInfo_::ParameterFlags_::kIsReadOnly as u32),
                automatable: has(ParameterInfo_::ParameterFlags_::kCanAutomate as u32),
                bypass: has(ParameterInfo_::ParameterFlags_::kIsBypass as u32),
            });
        }
        let cap = self.params.len() + 64;
        self.in_changes = ComWrapper::new(Changes::new(cap));
        self.out_changes = ComWrapper::new(Changes::new(cap));
        self.pending = Vec::with_capacity(cap);
        self.to_controller = Vec::with_capacity(cap * 4);
    }

    fn read_midi_map(&mut self) {
        self.midi_map = vec![u32::MAX; 16 * MIDI_CONTROLLERS];
        let Some(m) = self.controller.as_ref().and_then(|c| c.cast::<IMidiMapping>()) else { return };
        for ch in 0..16i16 {
            for cc in 0..MIDI_CONTROLLERS as i16 {
                let mut id: ParamID = 0;
                // SAFETY: main thread, the controller's mapping with an out id
                if ok(unsafe { m.getMidiControllerAssignment(0, ch, cc, &mut id) }) {
                    self.midi_map[ch as usize * MIDI_CONTROLLERS + cc as usize] = id;
                }
            }
        }
    }

    fn buses(&self, dir: BusDirection) -> Vec<Bus> {
        // SAFETY: main thread, the component's own methods
        let n = unsafe { self.component.getBusCount(MediaTypes_::kAudio as MediaType, dir) };
        (0..n.clamp(0, 64))
            .filter_map(|i| {
                // SAFETY: zeroed info filled by the component
                let mut b: BusInfo = unsafe { std::mem::zeroed() };
                // SAFETY: index below the count, valid out pointer
                ok(unsafe { self.component.getBusInfo(MediaTypes_::kAudio as MediaType, dir, i, &mut b) }).then(|| {
                    Bus {
                        name: wide(&b.name),
                        channels: b.channelCount.clamp(0, 64) as u32,
                        main: b.busType == BusTypes_::kMain as BusType,
                    }
                })
            })
            .collect()
    }

    fn setup_buses(&mut self) {
        let input = BusDirections_::kInput as BusDirection;
        let output = BusDirections_::kOutput as BusDirection;
        self.inputs = self.buses(input);
        self.outputs = self.buses(output);
        // The plugin's own arrangements, given back to it; every audio bus active
        let arr = |dir: BusDirection, n: usize| -> Vec<SpeakerArrangement> {
            (0..n as i32)
                .map(|i| {
                    let mut a: SpeakerArrangement = 0;
                    // SAFETY: main thread, out pointer
                    unsafe { self.processor.getBusArrangement(dir, i, &mut a) };
                    a
                })
                .collect()
        };
        let mut ins = arr(input, self.inputs.len());
        let mut outs = arr(output, self.outputs.len());
        // SAFETY: main thread, not active; arrays of the given lengths
        unsafe {
            self.processor.setBusArrangements(ins.as_mut_ptr(), ins.len() as i32, outs.as_mut_ptr(), outs.len() as i32);
        }
        for (dir, n) in [(input, self.inputs.len()), (output, self.outputs.len())] {
            for i in 0..n as i32 {
                // SAFETY: main thread, not active
                unsafe { self.component.activateBus(MediaTypes_::kAudio as MediaType, dir, i, 1) };
            }
        }
        // SAFETY: main thread
        self.event_inputs = unsafe { self.component.getBusCount(MediaTypes_::kEvent as MediaType, input) };
        if self.event_inputs > 0 {
            // SAFETY: main thread, not active
            unsafe { self.component.activateBus(MediaTypes_::kEvent as MediaType, input, 0, 1) };
        }
        // The buffers for every bus's channels, at fixed addresses from now on
        let max = self.max_block as usize;
        let ni: u32 = self.inputs.iter().map(|b| b.channels).sum();
        let no: u32 = self.outputs.iter().map(|b| b.channels).sum();
        self.in_data = vec![vec![0.0; max]; ni as usize];
        self.out_data = vec![vec![0.0; max]; no as usize];
        self.in_ptrs = self.in_data.iter_mut().map(|b| b.as_mut_ptr()).collect();
        self.out_ptrs = self.out_data.iter_mut().map(|b| b.as_mut_ptr()).collect();
        let bufs = |buses: &[Bus], ptrs: &mut Vec<*mut f32>| -> Vec<AudioBusBuffers> {
            let mut at = 0usize;
            buses
                .iter()
                .map(|b| {
                    // SAFETY: zeroed AudioBusBuffers is valid (numbers and a null pointer)
                    let mut x: AudioBusBuffers = unsafe { std::mem::zeroed() };
                    x.numChannels = b.channels as i32;
                    // SAFETY: `at` stays within `ptrs` (the sum of the buses' channels)
                    x.__field0.channelBuffers32 = unsafe { ptrs.as_mut_ptr().add(at) };
                    at += b.channels as usize;
                    x
                })
                .collect()
        };
        self.in_bufs = bufs(&self.inputs, &mut self.in_ptrs);
        self.out_bufs = bufs(&self.outputs, &mut self.out_ptrs);
    }

    fn activate(&mut self) -> Result<()> {
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            maxSamplesPerBlock: self.max_block as i32,
            sampleRate: self.rate,
        };
        // SAFETY: main thread, not active
        if !ok(unsafe { self.processor.setupProcessing(&mut setup) }) {
            return Err(err("vst3.setup"));
        }
        // SAFETY: main thread
        if !ok(unsafe { self.component.setActive(1) }) {
            return Err(err("vst3.activate"));
        }
        self.active = true;
        // SAFETY: main thread, active
        self.latency = unsafe { self.processor.getLatencySamples() };
        Ok(())
    }

    fn deactivate(&mut self) {
        if self.active {
            // SAFETY: main thread, not processing
            unsafe { self.component.setActive(0) };
            self.active = false;
        }
    }

    pub fn params(&self) -> &[ParamInfo] {
        &self.params
    }

    pub fn audio_inputs(&self) -> &[Bus] {
        &self.inputs
    }

    pub fn audio_outputs(&self) -> &[Bus] {
        &self.outputs
    }

    /// It has an event input (notes)
    pub fn takes_notes(&self) -> bool {
        self.event_inputs > 0
    }

    pub fn latency(&self) -> u32 {
        self.latency
    }

    /// It offers an editor view
    pub fn has_editor(&self) -> bool {
        self.controller.is_some()
    }

    /// A parameter's normalized value now, from the controller (main thread)
    pub fn param_value(&self, id: u32) -> Option<f64> {
        let c = self.controller.as_ref()?;
        // SAFETY: main thread
        self.params.iter().any(|p| p.id == id).then(|| unsafe { c.getParamNormalized(id) })
    }

    /// Sets a parameter (normalized): delivered at the start of the next block. No allocation
    pub fn set_param(&mut self, id: u32, value: f64) {
        if let Some(p) = self.pending.iter_mut().find(|p| p.0 == id) {
            p.1 = value;
        } else if self.pending.len() < self.pending.capacity() {
            self.pending.push((id, value));
        }
    }

    fn remember(&mut self, id: u32, value: f64) {
        if self.to_controller.len() < self.to_controller.capacity() {
            self.to_controller.push((id, value));
        }
    }

    fn push_event(&self, e: vst3::Steinberg::Vst::Event) {
        let mut l = self.in_events.list.borrow_mut();
        if l.len() < l.capacity() {
            l.push(e);
        }
    }

    fn note_event(time: u32, kind: u16) -> vst3::Steinberg::Vst::Event {
        // SAFETY: a zeroed Event is valid (numbers and a union of plain structs)
        let mut e: vst3::Steinberg::Vst::Event = unsafe { std::mem::zeroed() };
        e.sampleOffset = time as i32;
        e.r#type = kind;
        e
    }

    /// One block: `inputs`/`outputs` are the channels of the audio buses in order. Audio thread. No allocation on
    /// the host's side
    pub fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[Event],
        transport: Option<&Transport>,
    ) -> bool {
        let frames = frames.min(self.max_block as usize);
        let silence = |outputs: &mut [&mut [f32]]| {
            for o in outputs.iter_mut() {
                let n = frames.min(o.len());
                o[..n].fill(0.0);
            }
        };
        self.changes.clear();
        self.gestures.clear();
        if self.requests.restart.load(Ordering::Acquire) || !self.active {
            if self.processing {
                // SAFETY: audio thread, processing
                unsafe { self.processor.setProcessing(0) };
                self.processing = false;
            }
            silence(outputs);
            return true;
        }
        if !self.processing {
            // SAFETY: audio thread, active (kNotImplemented is fine)
            unsafe { self.processor.setProcessing(1) };
            self.processing = true;
        }
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
        // Parameters: what the host set, the controller's own edits, then the block's events
        self.in_changes.clear();
        self.out_changes.clear();
        self.in_events.list.borrow_mut().clear();
        self.out_events.list.borrow_mut().clear();
        for k in 0..self.pending.len() {
            let (id, v) = self.pending[k];
            self.in_changes.add(id, 0, v);
            self.remember(id, v);
        }
        self.pending.clear();
        while let Ok(edit) = self.edits.pop() {
            match edit {
                Edit::Set(id, v) => {
                    self.in_changes.add(id, 0, v);
                    if self.changes.len() < self.changes.capacity() {
                        self.changes.push((id, v));
                    }
                }
                Edit::Begin(id) | Edit::End(id) => {
                    if self.gestures.len() < self.gestures.capacity() {
                        self.gestures.push((id, matches!(edit, Edit::Begin(_))));
                    }
                }
            }
        }
        for e in events.iter().take(MAX_EVENTS) {
            match *e {
                Event::NoteOn { time, note_id, channel, key, velocity } => {
                    let mut x = Self::note_event(time, Event_::EventTypes_::kNoteOnEvent as u16);
                    x.__field0.noteOn =
                        NoteOnEvent { channel, pitch: key, tuning: 0.0, velocity, length: 0, noteId: note_id };
                    self.push_event(x);
                    if self.held.len() < self.held.capacity() {
                        self.held.push((note_id, channel, key));
                    }
                }
                Event::NoteOff { time, note_id, channel, key, velocity } => {
                    let mut x = Self::note_event(time, Event_::EventTypes_::kNoteOffEvent as u16);
                    x.__field0.noteOff = NoteOffEvent { channel, pitch: key, velocity, noteId: note_id, tuning: 0.0 };
                    self.push_event(x);
                    self.held.retain(|h| h.0 != note_id);
                }
                Event::ReleaseAll { time } => {
                    for k in 0..self.held.len() {
                        let (note_id, channel, key) = self.held[k];
                        let mut x = Self::note_event(time, Event_::EventTypes_::kNoteOffEvent as u16);
                        x.__field0.noteOff =
                            NoteOffEvent { channel, pitch: key, velocity: 0.0, noteId: note_id, tuning: 0.0 };
                        self.push_event(x);
                    }
                    self.held.clear();
                }
                Event::Expression { time, kind, note_id, value } => {
                    let mut x = Self::note_event(time, Event_::EventTypes_::kNoteExpressionValueEvent as u16);
                    let type_id = match kind {
                        Expression::Volume => NoteExpressionTypeIDs_::kVolumeTypeID,
                        Expression::Pan => NoteExpressionTypeIDs_::kPanTypeID,
                        Expression::Tuning => NoteExpressionTypeIDs_::kTuningTypeID,
                        Expression::Pressure => NoteExpressionTypeIDs_::kExpressionTypeID,
                        Expression::Brightness => NoteExpressionTypeIDs_::kBrightnessTypeID,
                    };
                    x.__field0.noteExpressionValue =
                        NoteExpressionValueEvent { typeId: type_id as NoteExpressionTypeID, noteId: note_id, value };
                    self.push_event(x);
                }
                Event::Param { time, id, value } => {
                    self.in_changes.add(id, time as i32, value);
                    self.remember(id, value);
                }
                Event::Midi { time, data } => {
                    let ch = (data[0] & 0x0F) as usize;
                    let (cc, v) = match data[0] & 0xF0 {
                        // (a data byte past 127 is malformed: it would index the next channel's controllers)
                        0xB0 if data[1] < 128 => (data[1] as usize, f64::from(data[2]) / 127.0),
                        0xD0 => (ControllerNumbers_::kAfterTouch as usize, f64::from(data[1]) / 127.0),
                        0xE0 => (
                            ControllerNumbers_::kPitchBend as usize,
                            f64::from(u16::from(data[1]) | (u16::from(data[2]) << 7)) / 16383.0,
                        ),
                        _ => continue,
                    };
                    let id = self.midi_map.get(ch * MIDI_CONTROLLERS + cc).copied().unwrap_or(u32::MAX);
                    if id != u32::MAX {
                        self.in_changes.add(id, time as i32, v);
                        self.remember(id, v);
                    }
                }
            }
        }
        // The transport
        let c = &mut *self.context;
        c.sampleRate = self.rate;
        c.state = 0;
        if let Some(t) = transport {
            use ProcessContext_::StatesAndFlags_ as F;
            let mut state = F::kTempoValid | F::kProjectTimeMusicValid;
            if t.playing {
                state |= F::kPlaying;
            }
            if t.recording {
                state |= F::kRecording;
            }
            if let Some((n, d)) = t.signature {
                state |= F::kTimeSigValid;
                c.timeSigNumerator = n;
                c.timeSigDenominator = d;
            }
            c.state = state;
            c.projectTimeSamples = t.samples;
            c.projectTimeMusic = t.beats;
            c.tempo = t.tempo;
        }
        let in_changes = self.in_changes.as_com_ref::<IParameterChanges>().map_or(ptr::null_mut(), |r| r.as_ptr());
        let out_changes = self.out_changes.as_com_ref::<IParameterChanges>().map_or(ptr::null_mut(), |r| r.as_ptr());
        let in_events = self.in_events.as_com_ref::<IEventList>().map_or(ptr::null_mut(), |r| r.as_ptr());
        let out_events = self.out_events.as_com_ref::<IEventList>().map_or(ptr::null_mut(), |r| r.as_ptr());
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: frames as i32,
            numInputs: self.in_bufs.len() as i32,
            numOutputs: self.out_bufs.len() as i32,
            inputs: self.in_bufs.as_mut_ptr(),
            outputs: self.out_bufs.as_mut_ptr(),
            inputParameterChanges: in_changes,
            outputParameterChanges: out_changes,
            inputEvents: if self.event_inputs > 0 { in_events } else { ptr::null_mut() },
            outputEvents: out_events,
            processContext: transport.map_or(ptr::null_mut(), |_| &mut *self.context as *mut ProcessContext),
        };
        // SAFETY: audio thread, processing; every pointer in `data` is valid for the call (buffers sized max_block,
        // lists owned by self)
        let r = unsafe { self.processor.process(&mut data) };
        for (k, o) in outputs.iter_mut().enumerate() {
            let n = frames.min(o.len());
            match self.out_data.get(k) {
                Some(b) => o[..n].copy_from_slice(&b[..n]),
                None => o[..n].fill(0.0),
            }
        }
        let changes = &mut self.changes;
        self.out_changes.last_points(|id, v| {
            if changes.len() < changes.capacity() {
                changes.push((id, v));
            }
        });
        r == kResultOk
    }

    /// The plugin's own parameter changes of the last block (its processor's output, its window's edits): (id,
    /// normalized value)
    pub fn changes(&self) -> &[(u32, f64)] {
        &self.changes
    }

    /// The gesture edges the plugin's window made in the last block: (parameter id, `true` begin / `false` end), in
    /// order (a drag is begin, its changes in [`Instance::changes`], end)
    pub fn gestures(&self) -> &[(u32, bool)] {
        &self.gestures
    }

    /// The plugin said its state changed since the last call
    pub fn take_state_dirty(&self) -> bool {
        self.requests.dirty.swap(false, Ordering::AcqRel)
    }

    /// Whether a restart waits for the audio side to stop
    pub fn restart_pending(&self) -> bool {
        self.requests.restart.load(Ordering::Acquire)
    }

    /// The main thread's turn: the controller learns the values the host set; a restart the audio side has stopped
    /// for (deactivate, activate, the latency read again). `true` when the latency changed
    pub fn idle(&mut self) -> bool {
        if let Some(c) = &self.controller {
            for (id, v) in self.to_controller.drain(..) {
                // SAFETY: main thread
                unsafe { c.setParamNormalized(id, v) };
            }
        } else {
            self.to_controller.clear();
        }
        let mut changed = false;
        if self.requests.restart.load(Ordering::Acquire) && !self.processing {
            let before = self.latency;
            self.deactivate();
            self.requests.restart.store(false, Ordering::Release);
            self.requests.latency.store(false, Ordering::Release);
            if self.activate().is_ok() {
                changed = before != self.latency;
            }
        }
        changed
    }

    /// The plugin's state: the component's and the controller's, in one chunk (main thread)
    pub fn save_state(&mut self) -> Result<Vec<u8>> {
        let a = Stream::new(Vec::new());
        let ap = a.to_com_ptr::<IBStream>().ok_or_else(|| err("vst3.state"))?;
        // SAFETY: main thread, our stream
        if !ok(unsafe { self.component.getState(ap.as_ptr()) }) {
            return Err(err("vst3.state.save"));
        }
        let b = Stream::new(Vec::new());
        if let (Some(c), Some(bp)) = (&self.controller, b.to_com_ptr::<IBStream>()) {
            // SAFETY: main thread, our stream (a controller without state writes nothing)
            unsafe { c.getState(bp.as_ptr()) };
        }
        let a = a.data.borrow();
        let b = b.data.borrow();
        let mut out = Vec::with_capacity(8 + a.len() + b.len());
        out.extend_from_slice(b"LMV3");
        out.extend_from_slice(&(a.len() as u32).to_le_bytes());
        out.extend_from_slice(&a);
        out.extend_from_slice(&b);
        Ok(out)
    }

    /// Gives the plugin a state saved by [`Instance::save_state`] (main thread)
    pub fn load_state(&mut self, chunk: &[u8]) -> Result<()> {
        let bad = || err("vst3.state.load");
        if chunk.len() < 8 || &chunk[..4] != b"LMV3" {
            return Err(bad());
        }
        let n = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as usize;
        let comp = chunk.get(8..8 + n).ok_or_else(bad)?;
        let ctrl = &chunk[8 + n..];
        let a = Stream::new(comp.to_vec());
        let ap = a.to_com_ptr::<IBStream>().ok_or_else(bad)?;
        // SAFETY: main thread, our stream
        if !ok(unsafe { self.component.setState(ap.as_ptr()) }) {
            return Err(bad());
        }
        if let Some(c) = &self.controller {
            a.at.set(0);
            // SAFETY: main thread, our streams
            unsafe { c.setComponentState(ap.as_ptr()) };
            if !ctrl.is_empty() {
                let b = Stream::new(ctrl.to_vec());
                if let Some(bp) = b.to_com_ptr::<IBStream>() {
                    // SAFETY: as above
                    unsafe { c.setState(bp.as_ptr()) };
                }
            }
        }
        // A state with another latency: applied by a restart, as if the plugin had asked (many do not)
        // SAFETY: main thread, active
        if self.active && unsafe { self.processor.getLatencySamples() } != self.latency {
            self.requests.restart.store(true, Ordering::Release);
        }
        Ok(())
    }
}

/// The editor view attached to a window, and the frame it was given
struct Editor {
    view: ComPtr<IPlugView>,
    frame: ComWrapper<Frame>,
    /// Keeps the frame's interface alive as long as the view may hold it
    _frame_ptr: ComPtr<IPlugFrame>,
}

fn rect_size(r: &ViewRect) -> (u32, u32) {
    (r.right.saturating_sub(r.left).max(1) as u32, r.bottom.saturating_sub(r.top).max(1) as u32)
}

impl Instance {
    /// Makes the plugin's editor view and attaches it to a native window (`parent`: the content `NSView` with
    /// `api` `"cocoa"`, an X11 window id with `"x11"`); returns the view's size. Main thread. An editor already
    /// open is closed first
    pub fn editor_open(&mut self, parent: usize, api: &str) -> Result<(u32, u32)> {
        self.editor_close();
        let c = self.controller.as_ref().ok_or_else(|| err("vst3.editor.none"))?;
        let kind: FIDString = match api {
            "cocoa" => kPlatformTypeNSView,
            "x11" => kPlatformTypeX11EmbedWindowID,
            _ => return Err(err("vst3.editor.platform")),
        };
        // SAFETY: main thread; `createView` returns a new reference (or null)
        let raw = unsafe { c.createView(c"editor".as_ptr()) };
        // SAFETY: the new reference `createView` gave, owned from here
        let view = unsafe { ComPtr::from_raw(raw) }.ok_or_else(|| err("vst3.editor.none"))?;
        // SAFETY: main thread, a platform type constant
        if !ok(unsafe { view.isPlatformTypeSupported(kind) }) {
            return Err(err("vst3.editor.platform"));
        }
        let frame = ComWrapper::new(Frame::default());
        let frame_ptr = frame.to_com_ptr::<IPlugFrame>().ok_or_else(|| err("vst3.editor.frame"))?;
        // SAFETY: main thread; the frame lives in `Editor` until the view is removed
        unsafe { view.setFrame(frame_ptr.as_ptr()) };
        // SAFETY: main thread; `parent` is a live native window of the right kind
        if !ok(unsafe { view.attached(parent as *mut c_void, kind) }) {
            // SAFETY: the frame is dropped right after
            unsafe { view.setFrame(ptr::null_mut()) };
            return Err(err("vst3.editor.attach"));
        }
        let mut rect = ViewRect { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: main thread, attached; a rect to fill
        let size = if ok(unsafe { view.getSize(&mut rect) }) { rect_size(&rect) } else { (640, 480) };
        self.editor = Some(Editor { view, frame, _frame_ptr: frame_ptr });
        Ok(size)
    }

    /// Detaches and drops the editor view (main thread); nothing when none is open
    pub fn editor_close(&mut self) {
        if let Some(e) = self.editor.take() {
            // SAFETY: main thread; the view is detached, then forgets its frame
            unsafe {
                e.view.removed();
                e.view.setFrame(ptr::null_mut());
            }
        }
    }

    pub fn editor_is_open(&self) -> bool {
        self.editor.is_some()
    }

    /// The window was resized by the user or the helper: the view is told, and answers the size it takes (main
    /// thread). A view that cannot resize keeps its own size, which is returned
    pub fn editor_resize(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let e = self.editor.as_ref()?;
        let mut rect = ViewRect { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        // SAFETY: main thread, attached
        unsafe {
            if e.view.canResize() == kResultTrue {
                e.view.checkSizeConstraint(&mut rect);
                e.view.onSize(&mut rect);
            } else {
                e.view.getSize(&mut rect);
            }
        }
        Some(rect_size(&rect))
    }

    /// The size the plugin's view asked for by itself (`resizeView`), once (main thread)
    pub fn take_editor_resize(&mut self) -> Option<(u32, u32)> {
        self.editor.as_ref()?.frame.resize.take()
    }

    /// The view's own event loop (Linux plugins ask the host for timers and file descriptors): run it from the main
    /// thread's loop, every few milliseconds
    pub fn editor_tick(&mut self) {
        if let Some(e) = &self.editor {
            e.frame.tick();
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.editor_close();
        if self.processing {
            // SAFETY: no other thread uses the instance any more
            unsafe { self.processor.setProcessing(0) };
            self.processing = false;
        }
        self.deactivate();
        if let Some((a, b)) = self.points.take() {
            // SAFETY: connected at creation
            unsafe {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
        }
        if let Some(c) = self.controller.take() {
            // SAFETY: main thread; the handler reference is dropped by the controller
            unsafe { c.setComponentHandler(ptr::null_mut()) };
            if self.separate {
                // SAFETY: initialised at creation, terminated once
                unsafe { c.terminate() };
            }
        }
        // SAFETY: initialised at creation, terminated once
        unsafe { self.component.terminate() };
    }
}

/// Where VST3 plugins are installed: `VST3_PATH`, then the user's and the system's folders
pub fn search_paths() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> =
        std::env::var_os("VST3_PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            out.push(h.join("Library/Audio/Plug-Ins/VST3"));
        }
        out.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
    } else {
        if let Some(h) = &home {
            out.push(h.join(".vst3"));
        }
        out.push(PathBuf::from("/usr/lib/vst3"));
        out.push(PathBuf::from("/usr/local/lib/vst3"));
    }
    out
}

/// The `.vst3` files and bundles under `dirs` (bundles not entered); no plugin code runs
pub fn find_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        if depth > 8 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.extension().is_some_and(|e| e == "vst3") {
                out.push(p);
            } else if p.is_dir() {
                walk(&p, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        if d.extension().is_some_and(|e| e == "vst3") && d.exists() {
            out.push(d.clone());
        } else {
            walk(d, 0, &mut out);
        }
    }
    out.dedup();
    out
}
