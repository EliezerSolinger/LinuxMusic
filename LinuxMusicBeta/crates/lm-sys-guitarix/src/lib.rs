//! Guitarix's units — its pedals, amps, cabinets, modulations and reverbs — compiled in from Guitarix's own code
//! (Faust-generated and hand-written C++, with zita-resampler and zita-convolver, `vendor/gx`), with the legacy
//! program's glue (`glue/`: the units list, the amp head, the convolving units, the Detune, the Vibe, the module
//! selectors), behind a safe binding.
//!
//! - [`units`]: every unit, in the legacy browser's order, with what it registers (its parameters: id, name, kind,
//!   range, default, a selector's values), found once by making each unit.
//! - [`Instance`]: one running unit. Made, its parameters set and started ([`Instance::start`]) off the audio thread;
//!   on the audio thread it only processes, takes parameter values and reports its outputs (meters).
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | A unit's `PluginDef*` (a C++ object that owns its modules, buffers, FFTW plans and convolver): one private pointer inside one [`Instance`], never copied; its `Drop` deactivates and deletes it (`activate_plugin(false)` then `delete_instance`). The pointers to its parameters' variables point inside it and live in the same `Instance`. |
//! | Which thread may call each function? | An `Instance` is `Send` and not `Sync` (every call takes `&mut self`): one thread at a time. Units keep no shared mutable state: Guitarix's Vibe kept its LFOs in its file, here each Vibe has its own (`vendor/gx/plugins/vibe.cc`, `vendor/gx/faust/vibe_*_lfo_sine.cc`), and the sine tables some Faust units fill when started are filled once (`std::call_once`), so instances on different threads never touch the same memory. The Detune's FFTW planning is under a mutex (FFTW's planner is not thread safe). [`units`] is computed once, under a `OnceLock`. |
//! | Allocates, locks, does I/O? | [`Instance::new`], [`Instance::start`] and [`Instance::update`] allocate (delay lines, the convolvers' partitions, impulse responses resampled, FFTW plans under their mutex) and are for the build only; `Drop` frees (never on the audio thread). [`Instance::process_mono`], [`Instance::process_stereo`], [`Instance::set`], [`Instance::output`], [`Instance::take_peak`] and [`Instance::clear`] neither allocate nor lock nor do I/O (`tests/no_alloc.rs`: every unit, under `assert_no_alloc` and the C heap trap). No file is read: the impulse responses are compiled in. |
//! | Creates threads? | No. The convolvers (zita-convolver) run with one partition level whose size is the block ([`MIN_CONVOLVER_BLOCK`] or more), which zita-convolver processes in the calling thread without starting its worker threads; `tests/units.rs` checks the process's thread count does not grow. |
//! | Calls back into our code? | Only while registering parameters ([`Instance::new`], [`units`]): the shim calls `register_param`, which runs under `catch_unwind` and only records the registration. |
//! | Tests | `tests/units.rs`: every unit is listed, made, started at 44.1 to 192 kHz, processes a signal to finite output, its parameters reach its variables, no thread is created. `tests/no_alloc.rs`: every unit's process path under `assert_no_alloc` and the C heap trap. ASan in CI. |

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::OnceLock;

use lm_base::{Error, Result};

mod ffi {
    use super::{c_char, c_int, c_void};

    #[repr(C)]
    pub struct PluginDef {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct ValuePair {
        pub value_id: *const c_char,
        pub value_label: *const c_char,
    }

    #[repr(C)]
    pub struct Param {
        pub id: *const c_char,
        pub name: *const c_char,
        pub kind: *const c_char,
        pub r#type: c_int,
        pub var: *mut c_void,
        pub value: f32,
        pub low: f32,
        pub up: f32,
        pub step: f32,
        pub values: *const ValuePair,
    }

    pub type RegisterFn = unsafe extern "C" fn(context: *mut c_void, param: *const Param);

    unsafe extern "C" {
        pub safe fn lmgx_unit_count() -> c_int;
        pub fn lmgx_unit_id(unit: c_int) -> *const c_char;
        pub fn lmgx_unit_has_update(unit: c_int) -> c_int;
        pub fn lmgx_make(unit: c_int) -> *mut PluginDef;
        pub fn lmgx_describe(
            def: *mut PluginDef,
            name: *mut *const c_char,
            category: *mut *const c_char,
            stereo: *mut c_int,
        );
        pub fn lmgx_register(def: *mut PluginDef, context: *mut c_void, f: RegisterFn);
        pub fn lmgx_start(def: *mut PluginDef, rate: u32, block: u32);
        pub fn lmgx_update(unit: c_int, def: *mut PluginDef, block: u32);
        pub fn lmgx_mono(def: *mut PluginDef, count: c_int, input: *mut f32, output: *mut f32);
        pub fn lmgx_stereo(
            def: *mut PluginDef,
            count: c_int,
            input0: *mut f32,
            input1: *mut f32,
            output0: *mut f32,
            output1: *mut f32,
        );
        pub fn lmgx_clear(def: *mut PluginDef) -> c_int;
        pub fn lmgx_delete(def: *mut PluginDef);
    }
}

/// The smallest block a convolving unit runs in without zita-convolver's worker threads (its `MINPART`: one partition
/// level of the block's size, processed in the calling thread)
pub const MIN_CONVOLVER_BLOCK: u32 = 64;
/// The largest block a unit takes at a time (the legacy plugin's chunk: some units size their buffers for it)
pub const MAX_BLOCK: usize = 256;

/// How a unit's parameter variable is stored
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarType {
    Float,
    Int,
    Bool,
}

/// A parameter as the unit registers it (the first time: a unit may register an id again, for more variables)
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDesc {
    /// Guitarix's id (`"ts9sim.drive"`), which names it in Guitarix's presets
    pub id: String,
    /// Its name, as the legacy plugin named it (Guitarix's, or the id's last part capitalised)
    pub name: String,
    /// Guitarix's kind: `S` a slider, `B` a switch, then options (`L` log, `O`/`o` an output, `M` a peak, `s` not
    /// saved, `A` aliased, `N` no MIDI)
    pub kind: String,
    pub var: VarType,
    pub default: f32,
    /// The range as the legacy plugin made its model (a selector's: its values', a step each)
    pub low: f32,
    pub up: f32,
    pub step: f32,
    /// A selector's values: (id, label)
    pub values: Vec<(String, String)>,
}

impl ParamDesc {
    fn options(&self) -> &str {
        self.kind.get(1..).unwrap_or("")
    }

    /// What the unit measures (a meter's level): not a setting
    pub fn is_output(&self) -> bool {
        self.options().contains(['O', 'o'])
    }

    /// An output that is a peak: the highest since it was last read
    pub fn is_peak(&self) -> bool {
        self.options().contains('M')
    }

    /// A logarithmic slider
    pub fn is_log(&self) -> bool {
        self.kind.contains('L')
    }

    /// One of a list of named values
    pub fn is_selector(&self) -> bool {
        !self.values.is_empty()
    }
}

/// A unit of Guitarix
#[derive(Clone, Debug, PartialEq)]
pub struct UnitDesc {
    /// Its place in the list ([`Instance::new`] takes it)
    pub index: usize,
    /// Guitarix's id (`"ts9sim"`, `"amp.tonestack"`, `"GCB_95"`)
    pub id: String,
    pub name: String,
    pub category: String,
    /// One instance takes both channels; a mono unit runs one instance per channel
    pub stereo: bool,
    /// The unit does work on the main thread when some parameters change ([`Instance::update`])
    pub has_update: bool,
    /// Its parameters, in the order it registers them
    pub params: Vec<ParamDesc>,
}

impl UnitDesc {
    /// The parameter with this Guitarix id
    pub fn param(&self, id: &str) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }
}

/// Every unit, in the legacy browser's order (each made once, the first time, to ask it)
pub fn units() -> &'static [UnitDesc] {
    static UNITS: OnceLock<Vec<UnitDesc>> = OnceLock::new();
    UNITS.get_or_init(|| {
        (0..ffi::lmgx_unit_count().max(0) as usize)
            .map(|index| {
                let probe = Instance::make(index);
                // SAFETY: `index` is below the count; the id is a static C string of the units list
                let id = unsafe { text(ffi::lmgx_unit_id(index as c_int)) };
                // SAFETY: as above
                let has_update = unsafe { ffi::lmgx_unit_has_update(index as c_int) } != 0;
                let (mut name, mut category, mut stereo) = (std::ptr::null(), std::ptr::null(), 0);
                // SAFETY: the instance is alive; the pointers written are the unit's static strings
                unsafe { ffi::lmgx_describe(probe.def.as_ptr(), &mut name, &mut category, &mut stereo) };
                // SAFETY: as above
                let (name, category) = unsafe { (text(name), text(category)) };
                let params = probe.registered.iter().filter(|r| r.first).map(|r| r.desc.clone()).collect();
                UnitDesc { index, id, name, category, stereo: stereo != 0, has_update, params }
            })
            .collect()
    })
}

/// The unit with this Guitarix id
pub fn unit(id: &str) -> Option<&'static UnitDesc> {
    units().iter().find(|u| u.id == id)
}

/// # Safety
/// `p` is null or a NUL-terminated string
unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the caller's
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

#[derive(Clone, Copy, Debug)]
enum Var {
    Float(NonNull<f32>),
    Int(NonNull<c_int>),
    Bool(NonNull<bool>),
}

/// One registration: the parameter (in [`UnitDesc::params`]'s order) and its variable
#[derive(Debug)]
struct Registered {
    desc: ParamDesc,
    /// The first registration of its id
    first: bool,
    var: Var,
}

/// What the registration callback fills
struct Registering {
    registered: Vec<Registered>,
}

unsafe extern "C" fn register_param(context: *mut c_void, param: *const ffi::Param) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the shim passes the `Registering` given to lmgx_register and a parameter valid for this call
        let (reg, p) = unsafe { (&mut *context.cast::<Registering>(), &*param) };
        // SAFETY: the unit's strings, NUL-terminated (or null)
        let (id, name, kind) = unsafe { (text(p.id), text(p.name), text(p.kind)) };
        let mut values = Vec::new();
        let mut v = p.values;
        // SAFETY: a selector's list ends with a null id
        while !v.is_null() && unsafe { !(*v).value_id.is_null() } {
            // SAFETY: as above
            let pair = unsafe { &*v };
            // SAFETY: NUL-terminated strings of the unit
            let (vid, label) = unsafe { (text(pair.value_id), text(pair.value_label)) };
            let label = if label.is_empty() { vid.clone() } else { label };
            values.push((vid, label));
            // SAFETY: still inside the list (its end is the null id)
            v = unsafe { v.add(1) };
        }
        let Some(ptr) = NonNull::new(p.var) else { return };
        let (var, var_type) = match p.r#type {
            0 => (Var::Float(ptr.cast()), VarType::Float),
            1 => (Var::Int(ptr.cast()), VarType::Int),
            _ => (Var::Bool(ptr.cast()), VarType::Bool),
        };
        let (mut low, mut up, mut step) = (p.low, p.up, p.step);
        // A selector's range is its values', a step each (Guitarix's EnumParameter from 0, FloatEnumParameter from
        // its lower bound), as the legacy plugin made its model
        if !values.is_empty() {
            if var_type == VarType::Int {
                low = 0.0;
            }
            up = low + (values.len().max(1) - 1) as f32;
            step = 1.0;
        }
        // (as the legacy plugin named one registered without a name: the id's last part, capitalised)
        let name = if name.is_empty() {
            let last = id.rsplit('.').next().unwrap_or(&id);
            let mut c = last.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        } else {
            name
        };
        let first = !reg.registered.iter().any(|r| r.desc.id == id);
        let desc = ParamDesc { id, name, kind, var: var_type, default: p.value, low, up, step, values };
        reg.registered.push(Registered { desc, first, var });
    }));
}

/// A running unit
pub struct Instance {
    def: NonNull<ffi::PluginDef>,
    unit: usize,
    registered: Vec<Registered>,
    /// For each of the unit's parameters ([`UnitDesc::params`]), its registrations (indices in `registered`)
    vars: Vec<Vec<usize>>,
    stereo: bool,
    block: u32,
}

// SAFETY: a unit is a plain C++ object with no thread affinity: it may be made on one thread and used on another
// (one at a time: `Instance` is not `Sync` and every call takes `&mut self`). Units share no mutable state (see the
// safety sheet)
unsafe impl Send for Instance {}

impl fmt::Debug for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Instance").field("unit", &self.unit).field("stereo", &self.stereo).finish_non_exhaustive()
    }
}

impl Instance {
    fn make(unit: usize) -> Self {
        // SAFETY: the caller checked the index (below lmgx_unit_count); the factory returns a new object
        let def = unsafe { ffi::lmgx_make(unit as c_int) };
        let def = NonNull::new(def).unwrap_or_else(|| panic!("Guitarix's unit {unit} could not be made"));
        let mut reg = Registering { registered: Vec::new() };
        // SAFETY: the instance is alive; the callback only records into `reg`, which outlives the call
        unsafe { ffi::lmgx_register(def.as_ptr(), (&raw mut reg).cast(), register_param) };
        let (mut name, mut category, mut stereo) = (std::ptr::null(), std::ptr::null(), 0);
        // SAFETY: the instance is alive
        unsafe { ffi::lmgx_describe(def.as_ptr(), &mut name, &mut category, &mut stereo) };
        let mut ids: Vec<&str> = Vec::new();
        let mut vars: Vec<Vec<usize>> = Vec::new();
        for (i, r) in reg.registered.iter().enumerate() {
            match ids.iter().position(|id| *id == r.desc.id) {
                Some(p) => vars[p].push(i),
                None => {
                    ids.push(&r.desc.id);
                    vars.push(vec![i]);
                }
            }
        }
        Self { def, unit, registered: reg.registered, vars, stereo: stereo != 0, block: 0 }
    }

    /// A new instance of the unit at `unit` (its place in [`units`]); its parameters at their defaults. Allocates
    pub fn new(unit: usize) -> Result<Self> {
        let desc = units().get(unit).ok_or_else(|| Error::new("guitarix.unit.unknown").arg("unit", unit as i64))?;
        let made = Self::make(unit);
        if made.vars.len() != desc.params.len() {
            return Err(Error::new("guitarix.unit.parameters").arg("unit", desc.id.as_str()));
        }
        Ok(made)
    }

    /// Its unit's place in [`units`]
    pub fn unit(&self) -> usize {
        self.unit
    }

    /// One instance for both channels
    pub fn is_stereo(&self) -> bool {
        self.stereo
    }

    /// Starts it at `rate`, to process `block` frames at a time (what Guitarix's JACK period was; a convolving unit
    /// needs a power of two of at least [`MIN_CONVOLVER_BLOCK`], the Detune exactly that many each call): its
    /// rate (which clears its state), then its activation, with the parameters as they are set. Allocates
    pub fn start(&mut self, rate: u32, block: u32) {
        self.block = block.clamp(1, MAX_BLOCK as u32);
        // SAFETY: the instance is alive and used by this thread only
        unsafe { ffi::lmgx_start(self.def.as_ptr(), rate, self.block) };
    }

    /// The unit's work after a change of the parameters it reads at the start (a convolver's impulse response, the
    /// Detune's latency setting): what the legacy program did on its main thread. Allocates
    pub fn update(&mut self) {
        // SAFETY: the instance is alive and used by this thread only
        unsafe { ffi::lmgx_update(self.unit as c_int, self.def.as_ptr(), self.block) };
    }

    /// Sets parameter `param` (its place in [`UnitDesc::params`]) to `value`, in each of its variables: a float as
    /// it is, an int rounded, a switch on from 0.5 (as the legacy plugin wrote its models' values). An index past
    /// the parameters is ignored. No allocation
    pub fn set(&mut self, param: usize, value: f32) {
        let Some(vars) = self.vars.get(param) else { return };
        for &i in vars {
            // SAFETY: the variables are inside the live unit, which only this thread uses; it reads them only when
            // it processes, which is not now
            unsafe {
                match self.registered[i].var {
                    Var::Float(p) => *p.as_ptr() = value,
                    Var::Int(p) => *p.as_ptr() = value.round() as c_int,
                    Var::Bool(p) => *p.as_ptr() = value >= 0.5,
                }
            }
        }
    }

    /// An output's value (a meter's level): the highest of its variables
    pub fn output(&self, param: usize) -> f32 {
        let Some(vars) = self.vars.get(param) else { return 0.0 };
        vars.iter()
            .map(|&i| {
                // SAFETY: as in `set`
                unsafe {
                    match self.registered[i].var {
                        Var::Float(p) => *p.as_ptr(),
                        Var::Int(p) => *p.as_ptr() as f32,
                        Var::Bool(p) => f32::from(u8::from(*p.as_ptr())),
                    }
                }
            })
            .fold(f32::MIN, f32::max)
    }

    /// A peak output's value, which starts again from 0 (Guitarix's `M` outputs: the highest since read)
    pub fn take_peak(&mut self, param: usize) -> f32 {
        let v = self.output(param);
        self.set(param, 0.0);
        v
    }

    /// Processes a mono unit: `input` into `output` (the same length, at most the block it was started with). The
    /// input may be changed (some units work in place on it). No allocation
    pub fn process_mono(&mut self, input: &mut [f32], output: &mut [f32]) {
        assert!(!self.stereo && input.len() == output.len() && input.len() <= MAX_BLOCK);
        // SAFETY: a started mono unit, two buffers of `len` samples, used by this thread only
        unsafe {
            ffi::lmgx_mono(self.def.as_ptr(), input.len() as c_int, input.as_mut_ptr(), output.as_mut_ptr());
        }
    }

    /// Processes a stereo unit (all four buffers of the same length, at most the block). No allocation
    pub fn process_stereo(&mut self, input: [&mut [f32]; 2], output: [&mut [f32]; 2]) {
        let n = input[0].len();
        assert!(self.stereo && n <= MAX_BLOCK && [input[1].len(), output[0].len(), output[1].len()] == [n, n, n]);
        let [i0, i1] = input;
        let [o0, o1] = output;
        // SAFETY: a started stereo unit, four buffers of `n` samples, used by this thread only
        unsafe {
            ffi::lmgx_stereo(
                self.def.as_ptr(),
                n as c_int,
                i0.as_mut_ptr(),
                i1.as_mut_ptr(),
                o0.as_mut_ptr(),
                o1.as_mut_ptr(),
            );
        }
    }

    /// Clears its sound (filters, delay lines) where the unit can (its `clear_state`); `false` if it cannot. No
    /// allocation
    pub fn clear(&mut self) -> bool {
        // SAFETY: the instance is alive and used by this thread only
        unsafe { ffi::lmgx_clear(self.def.as_ptr()) != 0 }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the instance is alive, owned here only, and never used again
        unsafe { ffi::lmgx_delete(self.def.as_ptr()) };
    }
}
