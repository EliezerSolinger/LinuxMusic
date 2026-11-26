//! Safe binding to libgig (GigaStudio `.gig` files), for the GigPlayer instrument (F-INS, legacy `plugins/GigPlayer`).
//! libgig is C++: a small shim (`shim.cpp`, built by `build.rs`) turns what the player needs into a C interface, and
//! this crate wraps that.
//!
//! The life of a file has two steps, both off the audio thread:
//!
//! 1. [`instruments`] lists the instruments of a file (bank, program, name), for the patch chooser;
//! 2. [`Patch::load`] loads the instrument of one (bank, program): its sample regions, their dimensions, their
//!    dimension regions (the envelope, the loop, the pitch tracking and the velocity attenuation of each) and **every
//!    sample it uses decoded to float, in memory** (the legacy loaded them to memory too: the audio thread never reads a
//!    file). The metadata is copied into Rust types; the samples stay in the shim's memory and are borrowed.
//!
//! On the audio thread a [`Patch`] is read only: [`Patch::select`] asks libgig which dimension region plays for a set
//! of dimension values, which is libgig's own selection (the bit positions, the zones, the velocity splits) and works
//! on structures it built at load.
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | `lmgig_patch*`: the one [`Patch`] that [`Patch::load`] made; its `Drop` closes it. The patch owns, in the shim, the opened RIFF file, the libgig `gig::File` and the decoded samples; nothing else points into them but the slices [`Patch::sample_data`] lends (tied to `&self`). |
//! | Which thread may call each function? | `libgig` keeps no global state that the calls here touch, but its objects are not thread-safe (they load lazily and keep iteration state). A [`Patch`] is `Send` and **not `Sync`**: it is made on a worker, moved to the audio thread, used by one thread at a time (every call is `&self` but nothing is shared), and goes back to be dropped. [`instruments`] and [`Patch::load`] open the file again each time, so no object is shared between them. |
//! | Allocates, locks, does I/O? | [`instruments`], [`Patch::load`] and [`Patch`]'s drop allocate and read the file: never on the audio thread. [`Patch::select`], [`Patch::dimregion`], [`Patch::region`], [`Patch::sample`] and [`Patch::sample_data`] do not (libgig's `GetDimensionRegionByValue` only reads the region it is asked about; the rest reads Rust vectors). |
//! | Creates threads? | No. |
//! | Calls back into our code? | No. Every exception libgig throws is caught in the shim and returned as an error text. |
//! | Tests | `tests/gig.rs` with a `.gig` written by [`test_file`]: the instruments are listed, a patch loads with its sample, its loop and envelope data, `select` finds the dimension region, a file that is not a `.gig` is an error. |

use lm_base::{Error, Result};
use std::ffi::{CStr, CString, c_char};
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::NonNull;

mod ffi {
    use super::{c_char, c_float};

    #[repr(C)]
    pub struct Patch {
        _private: [u8; 0],
    }

    /// `lmgig_dimregion` of shim.h
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct DimRegion {
        pub sample: i32,
        pub pitch_track: i32,
        pub unity_note: i32,
        pub fine_tune: i32,
        pub sample_attenuation: c_float,
        pub release_trigger_decay: i32,
        pub eg1_preattack: c_float,
        pub eg1_attack: c_float,
        pub eg1_decay1: c_float,
        pub eg1_decay2: c_float,
        pub eg1_infinite_sustain: i32,
        pub eg1_sustain: c_float,
        pub eg1_release: c_float,
        pub loop_count: i32,
        pub loop_type: i32,
        pub loop_start: u32,
        pub loop_length: u32,
        pub velocity_attenuation: [c_float; 128],
    }

    unsafe extern "C" {
        pub fn lmgig_instrument_count(path: *const c_char, error: *mut c_char, error_len: usize) -> i32;
        pub fn lmgig_instrument(
            path: *const c_char,
            index: i32,
            bank: *mut i32,
            program: *mut i32,
            name: *mut c_char,
            name_len: usize,
            error: *mut c_char,
            error_len: usize,
        ) -> i32;
        pub fn lmgig_load(
            path: *const c_char,
            bank: i32,
            program: i32,
            error: *mut c_char,
            error_len: usize,
        ) -> *mut Patch;
        pub fn lmgig_patch_close(patch: *mut Patch);
        pub fn lmgig_patch_key_dimension(patch: *const Patch, low: *mut i32, high: *mut i32) -> i32;
        pub fn lmgig_patch_sample_count(patch: *const Patch) -> i32;
        pub fn lmgig_patch_sample(
            patch: *const Patch,
            index: i32,
            rate: *mut u32,
            channels: *mut u32,
            frames: *mut u64,
        ) -> i32;
        pub fn lmgig_patch_sample_data(patch: *const Patch, index: i32) -> *const c_float;
        pub fn lmgig_patch_region_count(patch: *const Patch) -> i32;
        pub fn lmgig_patch_region(
            patch: *const Patch,
            region: i32,
            key_low: *mut i32,
            key_high: *mut i32,
            dimensions: *mut i32,
            first_dimregion: *mut i32,
            dimregion_count: *mut i32,
        ) -> i32;
        pub fn lmgig_patch_dimension(
            patch: *const Patch,
            region: i32,
            dimension: i32,
            kind: *mut i32,
            bits: *mut i32,
            zones: *mut i32,
        ) -> i32;
        pub fn lmgig_patch_dimregion_count(patch: *const Patch) -> i32;
        pub fn lmgig_patch_dimregion(patch: *const Patch, index: i32) -> *const DimRegion;
        pub fn lmgig_patch_select(patch: *const Patch, region: i32, values: *const u32) -> i32;
        pub fn lmgig_write_test_file(path: *const c_char, error: *mut c_char, error_len: usize) -> i32;
    }
}

use std::ffi::c_float;

/// The libgig version the crate was built against
pub const LINKED_VERSION: &str = env!("LM_SYS_GIG_LINKED_VERSION");

/// The size of the buffers the shim writes error texts and names into
const TEXT: usize = 512;

fn c_path(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::new("gig.bad_path").arg("path", path.display().to_string()))
}

/// The text a shim call wrote into `buf` (up to its NUL)
fn text(buf: &[c_char]) -> String {
    // SAFETY: `buf` is one of this module's zero-initialised buffers, which the shim fills with a NUL-terminated C string
    // (snprintf) or leaves untouched (all zero), so a NUL always ends it inside the slice
    let c = unsafe { CStr::from_ptr(buf.as_ptr()) };
    c.to_string_lossy().into_owned()
}

fn failed(key: &'static str, path: &Path, buf: &[c_char]) -> Error {
    Error::new(key).arg("path", path.display().to_string()).arg("why", text(buf))
}

/// An instrument of a `.gig` file, as the patch chooser shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentInfo {
    pub bank: i32,
    pub program: i32,
    /// The instrument's name (empty: the legacy showed "<no name>")
    pub name: String,
}

/// The instruments of the file, in the file's order. Reads the file: not on the audio thread. Errors:
/// `gig.open_failed` (not a `.gig` file, unreadable)
pub fn instruments(path: &Path) -> Result<Vec<InstrumentInfo>> {
    let c = c_path(path)?;
    let mut err = [0 as c_char; TEXT];
    // SAFETY: `c` is a NUL-terminated string that outlives the call; `err` is a writable buffer of the length passed
    let n = unsafe { ffi::lmgig_instrument_count(c.as_ptr(), err.as_mut_ptr(), TEXT) };
    if n < 0 {
        return Err(failed("gig.open_failed", path, &err));
    }
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let (mut bank, mut program) = (0i32, 0i32);
        let mut name = [0 as c_char; TEXT];
        let mut err = [0 as c_char; TEXT];
        // SAFETY: as above; `bank` and `program` are valid out pointers and `name` a writable buffer of the length passed
        let rc = unsafe {
            ffi::lmgig_instrument(
                c.as_ptr(),
                i,
                &mut bank,
                &mut program,
                name.as_mut_ptr(),
                TEXT,
                err.as_mut_ptr(),
                TEXT,
            )
        };
        if rc < 0 {
            return Err(failed("gig.open_failed", path, &err));
        }
        out.push(InstrumentInfo { bank, program, name: text(&name) });
    }
    Ok(out)
}

/// What a dimension of a region is about (libgig has more kinds; the player only distinguishes these)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DimensionKind {
    Other,
    Layer,
    Velocity,
    ReleaseTrigger,
    Keyboard,
    RoundRobin,
    SustainPedal,
    Random,
}

impl DimensionKind {
    fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Layer,
            2 => Self::Velocity,
            3 => Self::ReleaseTrigger,
            4 => Self::Keyboard,
            5 => Self::RoundRobin,
            6 => Self::SustainPedal,
            7 => Self::Random,
            _ => Self::Other,
        }
    }
}

/// One dimension of a region: what chooses among its dimension regions
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimension {
    pub kind: DimensionKind,
    /// 1 bit is 2 zones, 2 bits 4...
    pub bits: u32,
    pub zones: u32,
}

/// A sample region: the keys it answers and the dimension regions among which a note chooses
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub key_low: u8,
    pub key_high: u8,
    pub dimensions: Vec<Dimension>,
    /// Its dimension regions are `first..first + count` of [`Patch::dimregion`] (holes included: they have no sample)
    pub first: usize,
    pub count: usize,
}

/// How a loop plays
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopType {
    Forward,
    /// Forward and backward (ping pong)
    Bidirectional,
    Backward,
}

/// A sample loop, in frames of the sample
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loop {
    pub kind: LoopType,
    pub start: u32,
    pub length: u32,
}

/// The volume envelope of a dimension region (the file's EG1). Times in seconds, `preattack` and `sustain` in permille
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    pub preattack: f32,
    pub attack: f32,
    pub decay1: f32,
    pub decay2: f32,
    pub infinite_sustain: bool,
    pub sustain: f32,
    pub release: f32,
}

/// What a note plays when a dimension region is chosen
#[derive(Clone, Debug, PartialEq)]
pub struct DimRegion {
    /// An index of [`Patch::sample`]; `None`: the region has no sample (plays nothing)
    pub sample: Option<usize>,
    /// The sample is pitched by the key
    pub pitch_track: bool,
    pub unity_note: u8,
    /// Cents, as the file has it
    pub fine_tune: i32,
    /// The region's gain, linear
    pub sample_attenuation: f32,
    /// 0 to 8
    pub release_trigger_decay: u8,
    pub envelope: Envelope,
    /// The first loop (the legacy player used at most one)
    pub loop_: Option<Loop>,
    /// The attenuation of each MIDI velocity, linear
    pub velocity_attenuation: [f32; 128],
}

impl DimRegion {
    fn from_raw(r: &ffi::DimRegion) -> Self {
        let loop_ = (r.loop_count > 0).then(|| Loop {
            kind: match r.loop_type {
                1 => LoopType::Bidirectional,
                2 => LoopType::Backward,
                _ => LoopType::Forward,
            },
            start: r.loop_start,
            length: r.loop_length,
        });
        Self {
            sample: usize::try_from(r.sample).ok(),
            pitch_track: r.pitch_track != 0,
            unity_note: r.unity_note.clamp(0, 127) as u8,
            fine_tune: r.fine_tune,
            sample_attenuation: r.sample_attenuation,
            release_trigger_decay: r.release_trigger_decay.clamp(0, 8) as u8,
            envelope: Envelope {
                preattack: r.eg1_preattack,
                attack: r.eg1_attack,
                decay1: r.eg1_decay1,
                decay2: r.eg1_decay2,
                infinite_sustain: r.eg1_infinite_sustain != 0,
                sustain: r.eg1_sustain,
                release: r.eg1_release,
            },
            loop_,
            velocity_attenuation: r.velocity_attenuation,
        }
    }
}

/// A decoded sample of a patch: its rate and shape (the data is [`Patch::sample_data`])
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleInfo {
    pub rate: u32,
    /// 1 or 2
    pub channels: u32,
    pub frames: usize,
}

/// One instrument of a `.gig` file, loaded: see the crate's documentation
pub struct Patch {
    raw: NonNull<ffi::Patch>,
    key_dimension: (u8, u8),
    samples: Vec<SampleInfo>,
    regions: Vec<Region>,
    dimregions: Vec<DimRegion>,
}

// SAFETY: a `lmgig_patch` is plain heap memory with no thread affinity and no thread-local state, and the libgig
// objects in it are only used through `&self` by the thread that holds the `Patch`: it moves between threads whole
// (made on a worker, played on the audio thread, dropped on the main thread) and is never used by two at once.
unsafe impl Send for Patch {}

impl fmt::Debug for Patch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Patch")
            .field("samples", &self.samples.len())
            .field("regions", &self.regions.len())
            .field("dimregions", &self.dimregions.len())
            .finish_non_exhaustive()
    }
}

impl Patch {
    /// Loads the instrument of (`bank`, `program`) of the file, with its samples in memory. Reads the whole instrument:
    /// not on the audio thread. Errors: `gig.open_failed` (not a `.gig`), `gig.no_instrument` (no instrument there)
    pub fn load(path: &Path, bank: i32, program: i32) -> Result<Self> {
        let c = c_path(path)?;
        let mut err = [0 as c_char; TEXT];
        // SAFETY: `c` is a NUL-terminated string that outlives the call; `err` is a writable buffer of the length passed
        let raw = unsafe { ffi::lmgig_load(c.as_ptr(), bank, program, err.as_mut_ptr(), TEXT) };
        let Some(raw) = NonNull::new(raw) else {
            let key = if text(&err).contains("no instrument") { "gig.no_instrument" } else { "gig.open_failed" };
            return Err(failed(key, path, &err).arg("bank", i64::from(bank)).arg("program", i64::from(program)));
        };
        // From here the patch owns the handle: an early return drops it
        let mut patch =
            Self { raw, key_dimension: (0, 127), samples: Vec::new(), regions: Vec::new(), dimregions: Vec::new() };
        patch.read_metadata();
        Ok(patch)
    }

    fn read_metadata(&mut self) {
        let p = self.raw.as_ptr();
        let (mut low, mut high) = (0i32, 127i32);
        // SAFETY: `p` is the patch's valid handle; `low` and `high` are valid out pointers
        if unsafe { ffi::lmgig_patch_key_dimension(p, &mut low, &mut high) } == 0 {
            self.key_dimension = (low.clamp(0, 127) as u8, high.clamp(0, 127) as u8);
        }
        // SAFETY: `p` is valid
        let samples = unsafe { ffi::lmgig_patch_sample_count(p) };
        for i in 0..samples {
            let (mut rate, mut channels, mut frames) = (0u32, 1u32, 0u64);
            // SAFETY: `p` is valid and the three out pointers are valid
            if unsafe { ffi::lmgig_patch_sample(p, i, &mut rate, &mut channels, &mut frames) } == 0 {
                self.samples.push(SampleInfo { rate, channels: channels.clamp(1, 2), frames: frames as usize });
            } else {
                self.samples.push(SampleInfo { rate: 0, channels: 1, frames: 0 });
            }
        }
        // SAFETY: `p` is valid
        let regions = unsafe { ffi::lmgig_patch_region_count(p) };
        for r in 0..regions {
            let (mut low, mut high, mut dims, mut first, mut count) = (0i32, 0i32, 0i32, 0i32, 0i32);
            // SAFETY: `p` is valid and the out pointers are valid
            if unsafe { ffi::lmgig_patch_region(p, r, &mut low, &mut high, &mut dims, &mut first, &mut count) } != 0 {
                continue;
            }
            let mut dimensions = Vec::new();
            for d in 0..dims {
                let (mut kind, mut bits, mut zones) = (0i32, 0i32, 0i32);
                // SAFETY: `p` is valid and the out pointers are valid
                if unsafe { ffi::lmgig_patch_dimension(p, r, d, &mut kind, &mut bits, &mut zones) } == 0 {
                    dimensions.push(Dimension {
                        kind: DimensionKind::from_code(kind),
                        bits: bits.max(0) as u32,
                        zones: zones.max(0) as u32,
                    });
                }
            }
            self.regions.push(Region {
                key_low: low.clamp(0, 127) as u8,
                key_high: high.clamp(0, 127) as u8,
                dimensions,
                first: first.max(0) as usize,
                count: count.max(0) as usize,
            });
        }
        // SAFETY: `p` is valid
        let dimregions = unsafe { ffi::lmgig_patch_dimregion_count(p) };
        for i in 0..dimregions {
            // SAFETY: `p` is valid; the pointer it returns (when not null) points at a struct of the patch, read at once
            let raw = unsafe { ffi::lmgig_patch_dimregion(p, i) };
            // SAFETY: not null was checked, and the struct lives as long as the patch, which this call borrows
            let d = unsafe { raw.as_ref() }.map(DimRegion::from_raw);
            self.dimregions.push(d.unwrap_or_else(|| DimRegion::from_raw(&EMPTY)));
        }
    }

    /// The instrument's key dimension range (libgig's `DimensionKeyRange`): the keys that choose the zones of a
    /// keyboard dimension
    pub fn key_dimension(&self) -> (u8, u8) {
        self.key_dimension
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    pub fn region(&self, index: usize) -> Option<&Region> {
        self.regions.get(index)
    }

    pub fn dimregion(&self, index: usize) -> Option<&DimRegion> {
        self.dimregions.get(index)
    }

    pub fn samples(&self) -> &[SampleInfo] {
        &self.samples
    }

    pub fn sample(&self, index: usize) -> Option<&SampleInfo> {
        self.samples.get(index)
    }

    /// The sample's frames, interleaved (`channels` floats a frame), at half of full scale (as the legacy player read
    /// them). Empty for an index without data
    pub fn sample_data(&self, index: usize) -> &[f32] {
        let Some(info) = self.samples.get(index) else { return &[] };
        // SAFETY: the handle is valid; the call checks the index
        let p = unsafe { ffi::lmgig_patch_sample_data(self.raw.as_ptr(), index as i32) };
        if p.is_null() || info.frames == 0 {
            return &[];
        }
        // SAFETY: the shim keeps `channels * frames` floats at `p` for as long as the patch lives (it decoded them
        // so, and never changes them); the slice borrows `self`, which holds the patch
        unsafe { std::slice::from_raw_parts(p, info.frames * info.channels as usize) }
    }

    /// Which dimension region of `region` plays for these dimension values (one per dimension of the region, the
    /// others 0), as an index for [`Patch::dimregion`]; `None` when no dimension region is there. Does not allocate,
    /// lock or do I/O: for the audio thread
    pub fn select(&self, region: usize, values: &[u32; 8]) -> Option<usize> {
        let region = i32::try_from(region).ok()?;
        // SAFETY: the handle is valid; `values` is eight `u32`s as the shim reads; the call checks the region
        let i = unsafe { ffi::lmgig_patch_select(self.raw.as_ptr(), region, values.as_ptr()) };
        usize::try_from(i).ok()
    }
}

const EMPTY: ffi::DimRegion = ffi::DimRegion {
    sample: -1,
    pitch_track: 0,
    unity_note: 60,
    fine_tune: 0,
    sample_attenuation: 1.0,
    release_trigger_decay: 0,
    eg1_preattack: 0.0,
    eg1_attack: 0.0,
    eg1_decay1: 0.0,
    eg1_decay2: 0.0,
    eg1_infinite_sustain: 1,
    eg1_sustain: 1000.0,
    eg1_release: 0.0,
    loop_count: 0,
    loop_type: 0,
    loop_start: 0,
    loop_length: 0,
    velocity_attenuation: [1.0; 128],
};

impl Drop for Patch {
    fn drop(&mut self) {
        // SAFETY: the handle is valid and owned by this `Patch`, which is being dropped: nothing uses it after this
        unsafe { ffi::lmgig_patch_close(self.raw.as_ptr()) };
    }
}

/// Writes a small `.gig` for tests: bank 0 program 0 is a sine of 440 Hz at key 69 (a looped 44 kHz sample, pitch
/// tracked, the whole keyboard), program 1 the same an octave up. Errors: `gig.write_failed`
pub fn test_file(path: &Path) -> Result<()> {
    let c = c_path(path)?;
    let mut err = [0 as c_char; TEXT];
    // SAFETY: `c` is a NUL-terminated string that outlives the call; `err` is a writable buffer of the length passed
    let rc = unsafe { ffi::lmgig_write_test_file(c.as_ptr(), err.as_mut_ptr(), TEXT) };
    if rc < 0 {
        return Err(failed("gig.write_failed", path, &err));
    }
    Ok(())
}
