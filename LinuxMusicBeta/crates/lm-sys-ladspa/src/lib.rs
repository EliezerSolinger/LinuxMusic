//! Safe binding for hosting LADSPA plugins (the ABI is tiny and frozen: `dlopen`, `ladspa_descriptor`, the
//! descriptor's function pointers; written by hand in `ffi`).
//!
//! LADSPA plugins are third-party plugins: they run **out of the main process**, in the plugin-host helper
//! (interfaces.md I-9.6, P-03). This crate is what that helper links.
//!
//! - [`Library`]: a shared library opened, its descriptors read into owned data ([`Library::descriptors`]).
//! - [`Instance`]: one plugin made from a descriptor, its control ports (the inputs are parameters, with the hint
//!   ranges and defaults of the spec), its audio ports, the latency (an output control port named `latency`, the
//!   convention every host follows), and one block ([`Instance::process`]).
//!
//! LADSPA has no state beyond the control values, no events and no threads: the helper splits a block at a
//! parameter change to make it sample accurate.
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | The `dlopen` handle: one [`Library`] (an `Arc` shared by its instances, so the library outlives them); `Drop` calls `dlclose`. A `LADSPA_Handle`: one [`Instance`]; `Drop` deactivates and calls `cleanup`. The control values live in one array the instance allocated, at a fixed address for its whole life (the plugin holds pointers into it); freed after `cleanup`. Descriptors are copied into owned Rust data: no plugin pointer leaves the crate. |
//! | Which thread may call each function? | LADSPA says one thread at a time per instance and nothing more. `Instance` is `Send`, not `Sync`, and every call takes `&mut self` or `&self` on plain reads of the control array. `Library` is `Send` and `Sync` (it is read only after opening). |
//! | Allocates, locks, does I/O? | [`Instance::process`] and [`Instance::set_param`] do not allocate, lock or do I/O on the host's side (the scratch buffers are made with the instance). Whether the plugin's `run` is real-time safe is the plugin's promise (its `HARD_RT_CAPABLE` property is listed as [`Descriptor::hard_realtime`]). Everything else may allocate and block. |
//! | Creates threads? | No. |
//! | Calls back into our code? | No. LADSPA has no host callbacks. |
//! | Tests | `tests/plugin/lm_test_ladspa.c`, two plugins compiled by the tests with the system's C compiler (a mono gain with a toggle and an integer; a stereo gain with a latency port); `tests/instance.rs` (listing, hint ranges and defaults, exact audio, the latency port, no allocation in `process`). |

mod ffi;

use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;

use lm_base::{Error, Result};

use ffi::{Descriptor as CDescriptor, hint, port, prop};

fn err(key: &'static str) -> Error {
    Error::new(key)
}

fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the plugin gives NUL-terminated strings that live as long as its library is loaded
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// One port of a descriptor
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    /// The port's index in the descriptor
    pub index: u32,
    pub name: String,
    pub input: bool,
    pub audio: bool,
    /// The raw `LADSPA_PortRangeHintDescriptor`
    pub hints: i32,
    pub lower: f32,
    pub upper: f32,
}

impl Port {
    pub fn is_control_input(&self) -> bool {
        self.input && !self.audio
    }

    pub fn toggled(&self) -> bool {
        self.hints & hint::TOGGLED != 0
    }

    pub fn integer(&self) -> bool {
        self.hints & hint::INTEGER != 0
    }

    pub fn logarithmic(&self) -> bool {
        self.hints & hint::LOGARITHMIC != 0
    }

    pub fn sample_rate_relative(&self) -> bool {
        self.hints & hint::SAMPLE_RATE != 0
    }

    /// The range at a sample rate: an unbounded side is open (`None`); `SAMPLE_RATE` multiplies the bounds
    pub fn range(&self, rate: f64) -> (Option<f64>, Option<f64>) {
        let k = if self.sample_rate_relative() { rate } else { 1.0 };
        let lo = (self.hints & hint::BOUNDED_BELOW != 0).then(|| f64::from(self.lower) * k);
        let hi = (self.hints & hint::BOUNDED_ABOVE != 0).then(|| f64::from(self.upper) * k);
        (lo, hi)
    }

    /// The default of the spec's hint: from the bounds (low, middle, high are interpolated, on a log scale for
    /// a logarithmic port), the fixed 0, 1, 100 and 440, or, with no hint, 0 clamped to the range
    pub fn default(&self, rate: f64) -> f64 {
        let (lo, hi) = self.range(rate);
        let blend = |wl: f64, wu: f64| -> Option<f64> {
            let (l, u) = (lo?, hi?);
            Some(if self.logarithmic() && l > 0.0 && u > 0.0 {
                (l.ln() * wl + u.ln() * wu).exp()
            } else {
                l * wl + u * wu
            })
        };
        let chosen = match self.hints & hint::DEFAULT_MASK {
            hint::DEFAULT_MINIMUM => lo,
            hint::DEFAULT_LOW => blend(0.75, 0.25),
            hint::DEFAULT_MIDDLE => blend(0.5, 0.5),
            hint::DEFAULT_HIGH => blend(0.25, 0.75),
            hint::DEFAULT_MAXIMUM => hi,
            hint::DEFAULT_0 => Some(0.0),
            hint::DEFAULT_1 => Some(1.0),
            hint::DEFAULT_100 => Some(100.0),
            hint::DEFAULT_440 => Some(440.0),
            _ => None,
        };
        let v = chosen.unwrap_or(0.0);
        let v = lo.map_or(v, |l| v.max(l));
        let v = hi.map_or(v, |h| v.min(h));
        if self.integer() || self.toggled() { v.round() } else { v }
    }
}

/// A plugin a library offers, copied out of its descriptor
#[derive(Clone, Debug, PartialEq)]
pub struct Descriptor {
    /// Its place in the library (`ladspa_descriptor(index)`)
    pub index: u32,
    pub unique_id: u64,
    /// Unique within its file: what identifies it
    pub label: String,
    pub name: String,
    pub maker: String,
    pub ports: Vec<Port>,
    /// `LADSPA_PROPERTY_HARD_RT_CAPABLE`
    pub hard_realtime: bool,
    /// `LADSPA_PROPERTY_INPLACE_BROKEN`: the input and output buffers must be different (the helper's always are)
    pub inplace_broken: bool,
}

impl Descriptor {
    pub fn audio_inputs(&self) -> usize {
        self.ports.iter().filter(|p| p.audio && p.input).count()
    }

    pub fn audio_outputs(&self) -> usize {
        self.ports.iter().filter(|p| p.audio && !p.input).count()
    }

    /// The control input ports, in port order: the parameters
    pub fn controls(&self) -> impl Iterator<Item = &Port> {
        self.ports.iter().filter(|p| p.is_control_input())
    }

    /// The output control port named `latency` (case-insensitive), the convention for reporting it
    pub fn latency_port(&self) -> Option<&Port> {
        self.ports.iter().find(|p| !p.input && !p.audio && p.name.eq_ignore_ascii_case("latency"))
    }
}

/// An opened shared library with LADSPA plugins in it
pub struct Library {
    handle: *mut libc::c_void,
    entry: ffi::DescriptorFn,
    descriptors: Vec<Descriptor>,
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library").field("plugins", &self.descriptors.len()).finish()
    }
}

// SAFETY: the library is only read after `open` (the descriptors are immutable data in the plugin's static
// storage, `dlclose` happens once, in `Drop`, when no instance holds the `Arc`)
unsafe impl Send for Library {}
// SAFETY: same: nothing here mutates after `open`
unsafe impl Sync for Library {}

/// The most plugins read from one file (a broken library that never ends its list)
const MAX_PLUGINS: u32 = 4096;
/// The most ports of one plugin
const MAX_PORTS: u64 = 4096;

fn read_descriptor(index: u32, d: &CDescriptor) -> Descriptor {
    let count = d.port_count.min(MAX_PORTS) as usize;
    let mut ports = Vec::with_capacity(count);
    if !d.port_descriptors.is_null() && !d.port_names.is_null() && !d.port_range_hints.is_null() {
        for i in 0..count {
            // SAFETY: the three arrays have `port_count` entries each (the LADSPA contract), alive as long as the library
            let (pd, name, h) =
                unsafe { (*d.port_descriptors.add(i), *d.port_names.add(i), *d.port_range_hints.add(i)) };
            ports.push(Port {
                index: i as u32,
                name: text(name),
                input: pd & port::INPUT != 0,
                audio: pd & port::AUDIO != 0,
                hints: h.hint_descriptor,
                lower: h.lower_bound,
                upper: h.upper_bound,
            });
        }
    }
    Descriptor {
        index,
        unique_id: u64::from(d.unique_id as u32),
        label: text(d.label),
        name: text(d.name),
        maker: text(d.maker),
        ports,
        hard_realtime: d.properties & prop::HARD_RT_CAPABLE != 0,
        inplace_broken: d.properties & prop::INPLACE_BROKEN != 0,
    }
}

impl Library {
    /// Opens a file (runs the library's constructors: plugin code) and lists its plugins. A file with no
    /// `ladspa_descriptor` is `ladspa.not_a_plugin`
    pub fn open(path: &Path) -> Result<Arc<Library>> {
        let c = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| err("ladspa.path"))?;
        // SAFETY: a NUL-terminated path; dlopen runs the library's constructors (plugin code, in the helper)
        let handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(err("ladspa.open").arg("path", path.display().to_string()));
        }
        // SAFETY: the handle dlopen just gave; a NUL-terminated name
        let sym = unsafe { libc::dlsym(handle, c"ladspa_descriptor".as_ptr()) };
        if sym.is_null() {
            // SAFETY: the handle from dlopen, closed once
            unsafe { libc::dlclose(handle) };
            return Err(err("ladspa.not_a_plugin").arg("path", path.display().to_string()));
        }
        // SAFETY: the symbol has the type `ladspa_descriptor` has in the LADSPA header
        let entry: ffi::DescriptorFn = unsafe { std::mem::transmute::<*mut libc::c_void, ffi::DescriptorFn>(sym) };
        let mut descriptors = Vec::new();
        for i in 0..MAX_PLUGINS {
            // SAFETY: the entry point, called with consecutive indices until it answers null (the LADSPA contract)
            let d = unsafe { entry(u64::from(i)) };
            if d.is_null() {
                break;
            }
            // SAFETY: a non-null descriptor is valid for as long as the library is loaded
            descriptors.push(read_descriptor(i, unsafe { &*d }));
        }
        Ok(Arc::new(Library { handle, entry, descriptors }))
    }

    pub fn descriptors(&self) -> &[Descriptor] {
        &self.descriptors
    }

    fn raw(&self, index: u32) -> *const CDescriptor {
        // SAFETY: the entry point, with an index `open` saw answered
        unsafe { (self.entry)(u64::from(index)) }
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: the handle from dlopen, closed once; every instance (which holds an Arc) is gone
        unsafe { libc::dlclose(self.handle) };
    }
}

/// One plugin made from a descriptor
pub struct Instance {
    lib: Arc<Library>,
    desc: *const CDescriptor,
    handle: *mut libc::c_void,
    info: Descriptor,
    /// One value per port (only the control ports are used): the plugin holds pointers into this array
    controls: *mut f32,
    ports: usize,
    /// The audio port indexes
    audio_in: Vec<u32>,
    audio_out: Vec<u32>,
    /// The control input port indexes, in order (the parameters)
    params: Vec<u32>,
    /// Silence for an input the caller gave no channel for, and a place for an output it took no channel of
    silence: Vec<f32>,
    sink: Vec<f32>,
    max_block: usize,
    rate: f32,
    active: bool,
}

impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instance").field("label", &self.info.label).finish()
    }
}

// SAFETY: a LADSPA handle may be used from any one thread at a time; every call takes `&mut self` (or only reads the
// control array), so two never run at once. The pointers belong to this instance (and its library, kept by the Arc)
unsafe impl Send for Instance {}

impl Instance {
    /// Makes the plugin `label` of the library at `rate`, with `max_block` frames the most per block, its controls at
    /// their defaults, activated
    pub fn new(lib: &Arc<Library>, label: &str, rate: f64, max_block: usize) -> Result<Instance> {
        let info = lib
            .descriptors
            .iter()
            .find(|d| d.label == label)
            .ok_or_else(|| err("ladspa.not_found").arg("label", label))?
            .clone();
        let desc = lib.raw(info.index);
        if desc.is_null() {
            return Err(err("ladspa.not_found").arg("label", label));
        }
        // SAFETY: a valid descriptor of the loaded library
        let d = unsafe { &*desc };
        let (Some(instantiate), Some(connect)) = (d.instantiate, d.connect_port) else {
            return Err(err("ladspa.incomplete").arg("label", label));
        };
        let ports = info.ports.len();
        let mut values = vec![0.0f32; ports.max(1)].into_boxed_slice();
        for p in info.controls() {
            values[p.index as usize] = p.default(rate) as f32;
        }
        let controls = Box::into_raw(values).cast::<f32>();
        // SAFETY: the descriptor's own function, with its descriptor and a rate (LADSPA's is an unsigned long)
        let handle = unsafe { instantiate(desc, rate.round().max(1.0) as u64) };
        if handle.is_null() {
            // SAFETY: the array Box::into_raw made, with its length, freed once
            drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(controls, ports.max(1))) });
            return Err(err("ladspa.instantiate").arg("label", label));
        }
        let mut audio_in = Vec::new();
        let mut audio_out = Vec::new();
        let mut params = Vec::new();
        for p in &info.ports {
            if p.audio {
                if p.input { audio_in.push(p.index) } else { audio_out.push(p.index) }
            } else {
                if p.input {
                    params.push(p.index);
                }
                // SAFETY: as above: index < ports, the array has `ports` floats
                unsafe { connect(handle, u64::from(p.index), controls.add(p.index as usize)) };
            }
        }
        let max_block = max_block.max(1);
        let mut inst = Instance {
            lib: Arc::clone(lib),
            desc,
            handle,
            info,
            controls,
            ports,
            audio_in,
            audio_out,
            params,
            silence: vec![0.0; max_block],
            sink: vec![0.0; max_block],
            max_block,
            rate: rate as f32,
            active: false,
        };
        // Audio ports start on the scratch buffers: a plugin may be activated with its ports connected
        let (silence, sink) = (inst.silence.as_mut_ptr(), inst.sink.as_mut_ptr());
        for p in &inst.info.ports {
            if p.audio {
                // SAFETY: a port of this plugin, a buffer of `max_block` floats that lives as long as the instance
                unsafe { connect(handle, u64::from(p.index), if p.input { silence } else { sink }) };
            }
        }
        if let Some(activate) = d.activate {
            // SAFETY: the handle, once, before the first `run`
            unsafe { activate(handle) };
        }
        inst.active = true;
        Ok(inst)
    }

    pub fn descriptor(&self) -> &Descriptor {
        &self.info
    }

    /// The control input ports, in order: parameter `i` is `params()[i]`
    pub fn params(&self) -> Vec<&Port> {
        self.params.iter().map(|i| &self.info.ports[*i as usize]).collect()
    }

    pub fn param_count(&self) -> usize {
        self.params.len()
    }

    pub fn audio_inputs(&self) -> usize {
        self.audio_in.len()
    }

    pub fn audio_outputs(&self) -> usize {
        self.audio_out.len()
    }

    /// Parameter `index`'s value now
    pub fn param(&self, index: usize) -> Option<f32> {
        let port = *self.params.get(index)? as usize;
        // SAFETY: port < ports, inside the array; a plain read (the plugin does not write its input ports)
        Some(unsafe { self.controls.add(port).read() })
    }

    /// Sets parameter `index` (clamped to its bounds; for an integer or toggle port, rounded)
    pub fn set_param(&mut self, index: usize, value: f32) {
        let Some(&port) = self.params.get(index) else { return };
        let p = &self.info.ports[port as usize];
        let (lo, hi) = p.range(f64::from(self.rate));
        let mut v = f64::from(if value.is_finite() { value } else { 0.0 });
        if p.integer() || p.toggled() {
            v = v.round();
        }
        if let Some(l) = lo {
            v = v.max(l);
        }
        if let Some(h) = hi {
            v = v.min(h);
        }
        // SAFETY: port < ports, inside the array, written between blocks (this thread has the instance)
        unsafe { self.controls.add(port as usize).write(v as f32) };
    }

    /// The latency in frames the plugin reports on its `latency` output port (0 when it has none), read after a block
    pub fn latency(&self) -> u32 {
        let Some(p) = self.info.latency_port() else { return 0 };
        // SAFETY: the port's slot is inside the array
        let v = unsafe { self.controls.add(p.index as usize).read() };
        if v.is_finite() && v > 0.0 { v.round().min(1.0e7) as u32 } else { 0 }
    }

    /// One block of `frames` (at most the `max_block` of [`Instance::new`]): `inputs[i]` is the i-th audio input
    /// (a missing one is silence), `outputs[i]` the i-th audio output (a missing one is dropped). The input and output
    /// buffers are different, so `INPLACE_BROKEN` plugins are safe
    pub fn process(&mut self, frames: usize, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let frames = frames.min(self.max_block);
        if frames == 0 {
            return;
        }
        // SAFETY: a valid descriptor; both pointers are the plugin's own
        let d = unsafe { &*self.desc };
        let (Some(connect), Some(run)) = (d.connect_port, d.run) else { return };
        for (k, &port) in self.audio_in.iter().enumerate() {
            let p = inputs.get(k).filter(|b| b.len() >= frames).map_or(self.silence.as_ptr(), |b| b.as_ptr());
            // SAFETY: an input port of this plugin; the buffer has `frames` floats and the plugin only reads it (LADSPA
            // takes a mutable pointer for both directions); it is valid until `run` returns
            unsafe { connect(self.handle, u64::from(port), p.cast_mut()) };
        }
        for (k, &port) in self.audio_out.iter().enumerate() {
            let p = match outputs.get_mut(k) {
                Some(b) if b.len() >= frames => b.as_mut_ptr(),
                _ => self.sink.as_mut_ptr(),
            };
            // SAFETY: an output port of this plugin; `frames` floats, exclusively ours until `run` returns
            unsafe { connect(self.handle, u64::from(port), p) };
        }
        // SAFETY: every port is connected; `frames` does not exceed what the buffers hold
        unsafe { run(self.handle, frames as u64) };
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: a valid descriptor; the handle is ours and not used after `cleanup`
        let d = unsafe { &*self.desc };
        if self.active
            && let Some(deactivate) = d.deactivate
        {
            // SAFETY: activated once, deactivated once
            unsafe { deactivate(self.handle) };
        }
        if let Some(cleanup) = d.cleanup {
            // SAFETY: the handle is freed here and never used again
            unsafe { cleanup(self.handle) };
        }
        // SAFETY: the array Box::into_raw made, with its length; the plugin is gone, nothing points into it
        drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(self.controls, self.ports.max(1))) });
        let _ = &self.lib;
    }
}

/// Where LADSPA plugins are installed: `LADSPA_PATH` (colon separated), the folders the LinuxMusic install puts the
/// bundled sets in (`LINUXMUSIC_LADSPA_PATH`, then `<program dir>/ladspa` and `<program dir>/../lib/linuxmusic/ladspa`),
/// then the user's and the system's
pub fn search_paths() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for var in ["LADSPA_PATH", "LINUXMUSIC_LADSPA_PATH"] {
        if let Some(p) = std::env::var_os(var) {
            out.extend(std::env::split_paths(&p).filter(|p| !p.as_os_str().is_empty()));
        }
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        out.push(dir.join("ladspa"));
        out.push(dir.join("../lib/linuxmusic/ladspa"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            out.push(h.join("Library/Audio/Plug-Ins/LADSPA"));
        }
        out.push(PathBuf::from("/Library/Audio/Plug-Ins/LADSPA"));
        out.push(PathBuf::from("/opt/homebrew/lib/ladspa"));
        out.push(PathBuf::from("/usr/local/lib/ladspa"));
    } else {
        if let Some(h) = &home {
            out.push(h.join(".ladspa"));
        }
        for d in ["/usr/local/lib/ladspa", "/usr/lib/ladspa", "/usr/lib64/ladspa", "/usr/lib/x86_64-linux-gnu/ladspa"] {
            out.push(PathBuf::from(d));
        }
    }
    let mut seen = Vec::new();
    out.retain(|p| {
        let keep = !seen.contains(p);
        seen.push(p.clone());
        keep
    });
    out
}

/// The shared libraries under `dirs` (recursively); no plugin code runs. A file path is taken as it is
pub fn find_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    fn is_library(p: &Path) -> bool {
        p.extension().is_some_and(|e| e == "so" || e == "dylib")
    }
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        if depth > 6 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, depth + 1, out);
            } else if is_library(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        if d.is_file() {
            out.push(d.clone());
        } else {
            walk(d, 0, &mut out);
        }
    }
    out.dedup();
    out
}

/// Builds the test plugins (`tests/plugin/lm_test_ladspa.c`) into `dir` with the system's C compiler:
/// `dir/lm_test_ladspa.so`. For the tests of this crate and of the helper
#[cfg(feature = "testing")]
pub fn build_test_plugin(dir: &Path) -> Result<PathBuf> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = dir.join("lm_test_ladspa.so");
    std::fs::create_dir_all(dir).map_err(|e| err("ladspa.test.build").arg("error", e.to_string()))?;
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = std::process::Command::new(cc);
    cmd.arg("-shared").arg("-fPIC").arg("-O1").arg("-Wall").arg("-Werror");
    cmd.arg("-o").arg(&out).arg(here.join("tests/plugin/lm_test_ladspa.c"));
    let status = cmd.status().map_err(|e| err("ladspa.test.build").arg("error", e.to_string()))?;
    if !status.success() {
        return Err(err("ladspa.test.build").arg("status", status.to_string()));
    }
    Ok(out)
}
