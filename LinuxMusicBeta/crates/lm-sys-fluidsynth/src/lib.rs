//! Safe binding to FluidSynth, for the AvlDrums and Sf2 Player instruments (interfaces.md U33): one synthesizer that only the audio
//! thread calls, with several stereo outputs (one per drum group), no threads of its own and no audio driver.
//!
//! The life of an instance has two types:
//!
//! 1. [`SynthBuilder`]: made while the plugin is **built**, off the audio thread. It is the only type that can load a
//!    SoundFont, so a SoundFont is only ever loaded into a fresh instance that has never played (U33; a kit change
//!    builds a new instance and the old one plays its tail, U32). It also sets up the channels (preset, drum type,
//!    controllers).
//! 2. [`Synth`]: what [`SynthBuilder::build`] returns, moved to the audio thread. It plays (notes, controllers,
//!    programs) and renders into the outputs. It cannot load anything.
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | `fluid_settings_t*` and `fluid_synth_t*`: one private `Instance`, inside the `SynthBuilder` and then the `Synth` (moved, never copied). Its `Drop` deletes the synth, then the settings. SoundFonts belong to the synth (freed with it); a [`SoundFontId`] is only a number. A [`VoiceRef`] is plain data (the instance's serial number, the voice's address, its id): the `Synth` dereferences it only when the serial is its own, and touches the voice only when its id is still the one recorded and it plays (voices live as long as the instance: the voice table is allocated once and the polyphony never changes). |
//! | Which thread may call each function? | `synth.threadsafe-api` is **off**: FluidSynth takes no lock, so the instance must never be used by two threads at once. Both types are `Send` and **not `Sync`**, and every call takes `&mut self`: the compiler forbids concurrent use. The `Synth` is moved to the audio thread and stays there; it goes back to the main thread to be dropped (return ring, rule 5). |
//! | Allocates, locks, does I/O? | `SynthBuilder::new` and `load_soundfont` allocate and do file I/O (the samples are read whole: no dynamic sample loading), and take this crate's log lock: build time only. `presets` allocates the list. On `Synth`: `note_on`, `note_off`, `cc`, `pitch_bend`, `program_change`, `bank_select`, `program_select`, `all_notes_off`, `all_sounds_off`, `render`, `render_stereo`, `set_gain`, `set_reverb_on`, `set_reverb`, `set_chorus_on`, `set_chorus`, `set_bend_range`, `set_interpolation`, `voices_of`, `voice_gen` and `set_voice_gen` neither allocate nor lock nor do I/O (voices are preallocated by `synth.polyphony`; checked by `tests/no_alloc.rs`, which counts the C heap) and are for the audio thread. `Drop` frees: never on the audio thread. |
//! | Creates threads? | No: `synth.cpu-cores = 1` (no render threads), no audio or MIDI driver is ever created, and a FluidSynth built with OpenMP (Debian, Ubuntu) loads SoundFonts with OpenMP limited to one thread on the loading thread (`OpenMpSingleThread`), so libgomp never starts its worker pool. `tests/synth.rs` checks the thread count of the process does not grow (on macOS, the first `new_fluid_settings` of the process registers FluidSynth's drivers, CoreAudio and PortAudio, which makes the system start two idle libdispatch workers once per process: not FluidSynth's, never per instance, and never on the audio thread, since settings are made at build time). |
//! | Calls back into our code? | Only the log function, set once for the whole library: errors and panics go to `log_trampoline`, which runs under `catch_unwind` and only records text when the calling thread is the one loading (it compares `pthread_self`, so on the audio thread it neither allocates nor locks); warnings, info and debug are switched off (FluidSynth would print them to stderr from the audio thread). |
//! | Tests | `tests/synth.rs` with a SoundFont made by the test (`tests/common/sf2.rs`): a note renders a non-silent tone at the right pitch, note off and all-sounds-off silence it, two MIDI channels render into two separate outputs, presets are listed, a broken SoundFont is an error with FluidSynth's message, no thread is created. `tests/no_alloc.rs`: the audio-thread calls under `assert_no_alloc` and the C heap trap. ASan in CI. |

use lm_base::{Error, Result};
use std::ffi::{CStr, CString, c_char, c_double, c_float, c_int, c_void};
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Once, PoisonError};

mod ffi {
    use super::{c_char, c_double, c_float, c_int, c_void};

    #[repr(C)]
    pub struct Settings {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct Synth {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct Sfont {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct Preset {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct Voice {
        _private: [u8; 0],
    }

    pub const FLUID_OK: c_int = 0;
    pub const FLUID_FAILED: c_int = -1;

    pub const FLUID_PANIC: c_int = 0;
    pub const FLUID_ERR: c_int = 1;
    pub const FLUID_WARN: c_int = 2;
    pub const FLUID_INFO: c_int = 3;
    pub const FLUID_DBG: c_int = 4;

    pub type LogFunction = Option<unsafe extern "C" fn(level: c_int, message: *const c_char, data: *mut c_void)>;

    unsafe extern "C" {
        pub safe fn new_fluid_settings() -> *mut Settings;
        pub fn delete_fluid_settings(settings: *mut Settings);
        pub fn fluid_settings_setint(settings: *mut Settings, name: *const c_char, val: c_int) -> c_int;
        pub fn fluid_settings_setnum(settings: *mut Settings, name: *const c_char, val: c_double) -> c_int;

        pub fn new_fluid_synth(settings: *mut Settings) -> *mut Synth;
        pub fn delete_fluid_synth(synth: *mut Synth);
        pub fn fluid_synth_sfload(synth: *mut Synth, filename: *const c_char, reset_presets: c_int) -> c_int;
        pub fn fluid_synth_get_sfont_by_id(synth: *mut Synth, id: c_int) -> *mut Sfont;
        pub fn fluid_sfont_iteration_start(sfont: *mut Sfont);
        pub fn fluid_sfont_iteration_next(sfont: *mut Sfont) -> *mut Preset;
        pub fn fluid_preset_get_name(preset: *mut Preset) -> *const c_char;
        pub fn fluid_preset_get_banknum(preset: *mut Preset) -> c_int;
        pub fn fluid_preset_get_num(preset: *mut Preset) -> c_int;

        pub fn fluid_synth_noteon(synth: *mut Synth, chan: c_int, key: c_int, vel: c_int) -> c_int;
        pub fn fluid_synth_noteoff(synth: *mut Synth, chan: c_int, key: c_int) -> c_int;
        pub fn fluid_synth_cc(synth: *mut Synth, chan: c_int, ctrl: c_int, val: c_int) -> c_int;
        pub fn fluid_synth_pitch_bend(synth: *mut Synth, chan: c_int, val: c_int) -> c_int;
        pub fn fluid_synth_program_change(synth: *mut Synth, chan: c_int, program: c_int) -> c_int;
        pub fn fluid_synth_bank_select(synth: *mut Synth, chan: c_int, bank: c_int) -> c_int;
        pub fn fluid_synth_program_select(
            synth: *mut Synth,
            chan: c_int,
            sfont_id: c_int,
            bank_num: c_int,
            preset_num: c_int,
        ) -> c_int;
        pub fn fluid_synth_set_channel_type(synth: *mut Synth, chan: c_int, kind: c_int) -> c_int;
        pub fn fluid_synth_all_notes_off(synth: *mut Synth, chan: c_int) -> c_int;
        pub fn fluid_synth_all_sounds_off(synth: *mut Synth, chan: c_int) -> c_int;
        pub fn fluid_synth_count_audio_channels(synth: *mut Synth) -> c_int;
        pub fn fluid_synth_count_audio_groups(synth: *mut Synth) -> c_int;
        pub fn fluid_synth_count_midi_channels(synth: *mut Synth) -> c_int;
        pub fn fluid_synth_process(
            synth: *mut Synth,
            len: c_int,
            nfx: c_int,
            fx: *mut *mut c_float,
            nout: c_int,
            out: *mut *mut c_float,
        ) -> c_int;

        pub fn fluid_synth_write_float(
            synth: *mut Synth,
            len: c_int,
            lout: *mut c_void,
            loff: c_int,
            lincr: c_int,
            rout: *mut c_void,
            roff: c_int,
            rincr: c_int,
        ) -> c_int;
        pub fn fluid_synth_set_gain(synth: *mut Synth, gain: c_float);
        pub fn fluid_synth_get_gain(synth: *mut Synth) -> c_float;
        pub fn fluid_synth_reverb_on(synth: *mut Synth, fx_group: c_int, on: c_int) -> c_int;
        pub fn fluid_synth_set_reverb_group_roomsize(synth: *mut Synth, fx_group: c_int, v: c_double) -> c_int;
        pub fn fluid_synth_set_reverb_group_damp(synth: *mut Synth, fx_group: c_int, v: c_double) -> c_int;
        pub fn fluid_synth_set_reverb_group_width(synth: *mut Synth, fx_group: c_int, v: c_double) -> c_int;
        pub fn fluid_synth_set_reverb_group_level(synth: *mut Synth, fx_group: c_int, v: c_double) -> c_int;
        pub fn fluid_synth_chorus_on(synth: *mut Synth, fx_group: c_int, on: c_int) -> c_int;
        pub fn fluid_synth_set_chorus_group_nr(synth: *mut Synth, fx_group: c_int, nr: c_int) -> c_int;
        pub fn fluid_synth_set_chorus_group_level(synth: *mut Synth, fx_group: c_int, v: c_double) -> c_int;
        pub fn fluid_synth_set_chorus_group_speed(synth: *mut Synth, fx_group: c_int, v: c_double) -> c_int;
        pub fn fluid_synth_set_chorus_group_depth(synth: *mut Synth, fx_group: c_int, v: c_double) -> c_int;
        pub fn fluid_synth_set_chorus_group_type(synth: *mut Synth, fx_group: c_int, kind: c_int) -> c_int;
        pub fn fluid_synth_pitch_wheel_sens(synth: *mut Synth, chan: c_int, val: c_int) -> c_int;
        pub fn fluid_synth_set_interp_method(synth: *mut Synth, chan: c_int, method: c_int) -> c_int;
        pub fn fluid_synth_get_internal_bufsize(synth: *mut Synth) -> c_int;
        pub fn fluid_synth_get_polyphony(synth: *mut Synth) -> c_int;
        pub fn fluid_synth_get_voicelist(synth: *mut Synth, buf: *mut *mut Voice, bufsize: c_int, id: c_int);
        pub fn fluid_voice_get_id(voice: *const Voice) -> std::ffi::c_uint;
        pub fn fluid_voice_get_channel(voice: *const Voice) -> c_int;
        pub fn fluid_voice_get_key(voice: *const Voice) -> c_int;
        pub fn fluid_voice_is_on(voice: *const Voice) -> c_int;
        pub fn fluid_voice_is_playing(voice: *const Voice) -> c_int;
        pub fn fluid_voice_gen_get(voice: *mut Voice, generator: c_int) -> c_float;
        pub fn fluid_voice_gen_set(voice: *mut Voice, generator: c_int, value: c_float);
        pub fn fluid_voice_update_param(voice: *mut Voice, generator: c_int);

        pub fn fluid_set_log_function(level: c_int, fun: LogFunction, data: *mut c_void) -> LogFunction;
        pub fn fluid_version(major: *mut c_int, minor: *mut c_int, micro: *mut c_int);

        /// From the C library: an id of the calling thread that costs nothing (no allocation, no lock)
        pub safe fn pthread_self() -> usize;

        /// From the C library: a symbol of the loaded libraries, or NULL
        pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
}

/// The most stereo outputs an instance may have (AvlDrums needs 9; the render keeps its pointer table on the stack)
pub const MAX_OUTPUTS: usize = 32;

#[cfg(feature = "testing")]
pub mod test_font;

/// The version of the linked FluidSynth, (major, minor, micro)
pub fn version() -> (i32, i32, i32) {
    let (mut major, mut minor, mut micro) = (0, 0, 0);
    // SAFETY: three valid out pointers
    unsafe { ffi::fluid_version(&mut major, &mut minor, &mut micro) };
    (major, minor, micro)
}

// ---- The log -----------------------------------------------------------------------------------------------------

/// The thread whose FluidSynth errors are being recorded (`pthread_self`), or 0
static CAPTURING_THREAD: AtomicUsize = AtomicUsize::new(0);
/// What was recorded; only the capturing thread touches it
static CAPTURED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// One capture at a time (builds and loads take turns: they are rare and slow anyway)
static CAPTURE_TURN: Mutex<()> = Mutex::new(());
static LOG_SETUP: Once = Once::new();

/// FluidSynth's log function for errors and panics. Never unwinds into C; on any thread but the one capturing it
/// returns at once, touching only an atomic (the audio thread may log "out of voices" and the like)
unsafe extern "C" fn log_trampoline(_level: c_int, message: *const c_char, _data: *mut c_void) {
    let _ = std::panic::catch_unwind(|| {
        let me = ffi::pthread_self();
        if message.is_null() || CAPTURING_THREAD.load(Ordering::Acquire) != me {
            return;
        }
        // SAFETY: FluidSynth passes a NUL-terminated message that lives for the duration of the call
        let text = unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned();
        CAPTURED.lock().unwrap_or_else(PoisonError::into_inner).push(text);
    });
}

fn setup_log() {
    LOG_SETUP.call_once(|| {
        // SAFETY: setting the log functions before any other FluidSynth call of this program (Once), with functions
        // that are valid for the life of the program
        unsafe {
            ffi::fluid_set_log_function(ffi::FLUID_PANIC, Some(log_trampoline), std::ptr::null_mut());
            ffi::fluid_set_log_function(ffi::FLUID_ERR, Some(log_trampoline), std::ptr::null_mut());
            ffi::fluid_set_log_function(ffi::FLUID_WARN, None, std::ptr::null_mut());
            ffi::fluid_set_log_function(ffi::FLUID_INFO, None, std::ptr::null_mut());
            ffi::fluid_set_log_function(ffi::FLUID_DBG, None, std::ptr::null_mut());
        }
    });
}

/// Runs `f` with FluidSynth's errors on this thread recorded; returns them with f's result
fn capturing<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    setup_log();
    let _turn = CAPTURE_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    CAPTURED.lock().unwrap_or_else(PoisonError::into_inner).clear();
    CAPTURING_THREAD.store(ffi::pthread_self(), Ordering::Release);
    let value = f();
    CAPTURING_THREAD.store(0, Ordering::Release);
    let messages = std::mem::take(&mut *CAPTURED.lock().unwrap_or_else(PoisonError::into_inner));
    (value, messages)
}

fn with_messages(error: Error, messages: &[String]) -> Error {
    if messages.is_empty() { error } else { error.arg("message", messages.join("; ")) }
}

// ---- OpenMP ------------------------------------------------------------------------------------------------------

/// FluidSynth built with OpenMP (Debian and Ubuntu's `libfluidsynth3`, linked to libgomp) loads a SoundFont's samples
/// in an `omp parallel` region: libgomp then starts its worker pool, one thread per core, and keeps it for the life of
/// the process. U33 wants no threads of FluidSynth's, so the load runs with OpenMP limited to one thread, which makes
/// libgomp run the region on the calling thread alone and start nothing.
///
/// `omp_set_num_threads` sets the `nthreads-var` of the calling thread only (each thread outside a parallel region
/// has its own), so no other thread is affected, and the guard puts the old value back. The two functions are looked
/// up at run time: a FluidSynth without OpenMP (Homebrew's) does not load libgomp, and then there is nothing to do.
struct OpenMpSingleThread {
    restore: Option<(unsafe extern "C" fn(c_int), c_int)>,
}

/// `RTLD_DEFAULT`: search every library loaded in the process
#[cfg(target_os = "linux")]
const RTLD_DEFAULT: *mut c_void = std::ptr::null_mut();
#[cfg(target_os = "macos")]
const RTLD_DEFAULT: *mut c_void = -2_isize as *mut c_void;

impl OpenMpSingleThread {
    fn enter() -> Self {
        // SAFETY: dlsym with RTLD_DEFAULT and NUL-terminated names only looks symbols up
        let (get, set) = unsafe {
            (
                ffi::dlsym(RTLD_DEFAULT, c"omp_get_max_threads".as_ptr()),
                ffi::dlsym(RTLD_DEFAULT, c"omp_set_num_threads".as_ptr()),
            )
        };
        if get.is_null() || set.is_null() {
            return Self { restore: None };
        }
        // SAFETY: these are the OpenMP runtime's `int omp_get_max_threads(void)` and `void omp_set_num_threads(int)`
        // (OpenMP API, every version), so the pointers have exactly these signatures
        let (get, set) = unsafe {
            (
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> c_int>(get),
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn(c_int)>(set),
            )
        };
        // SAFETY: plain calls into the OpenMP runtime from a thread that is not in a parallel region
        let before = unsafe { get() };
        // SAFETY: as above
        unsafe { set(1) };
        Self { restore: Some((set, before)) }
    }
}

impl Drop for OpenMpSingleThread {
    fn drop(&mut self) {
        if let Some((set, before)) = self.restore {
            // SAFETY: the function found in enter(), called on the same thread (the guard never leaves it)
            unsafe { set(before.max(1)) };
        }
    }
}

// ---- The instance ------------------------------------------------------------------------------------------------

/// How an instance is made
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SynthConfig {
    /// The project rate (FluidSynth accepts 8 kHz to 96 kHz)
    pub sample_rate: f64,
    /// Stereo outputs, 1 to [`MAX_OUTPUTS`]. MIDI channel `c` plays into output `c % outputs`
    pub outputs: usize,
    /// MIDI channels, a multiple of 16
    pub midi_channels: usize,
    /// Voices, all preallocated: a note beyond them steals the oldest, it never allocates
    pub polyphony: usize,
    /// FluidSynth's master gain (its default is 0.2; a drum kit wants 1.0)
    pub gain: f32,
}

impl Default for SynthConfig {
    fn default() -> Self {
        Self { sample_rate: 48_000.0, outputs: 1, midi_channels: 16, polyphony: 256, gain: 1.0 }
    }
}

/// The two handles; deleted synth first, settings after
struct Instance {
    settings: NonNull<ffi::Settings>,
    synth: NonNull<ffi::Synth>,
    config: SynthConfig,
    /// A number no other instance of the process ever has: a [`VoiceRef`] is only used with the instance it came from
    serial: u64,
}

/// The serial numbers of the instances
static SERIALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

// SAFETY: FluidSynth instances have no thread affinity (no thread-local state; with threadsafe-api off, no mutex
// owned by a thread). With threadsafe-api off it must not be used by two threads at once: every use goes through
// &mut self of SynthBuilder or Synth, and the raw pointers keep both !Sync. Moving it between threads is fine.
unsafe impl Send for Instance {}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: both handles were made by new_fluid_* in SynthBuilder::new, are owned by this value alone and are
        // freed once, the synth (which refers to the settings) first. No thread of FluidSynth is running (none is
        // ever made: cpu-cores = 1, no driver)
        unsafe {
            ffi::delete_fluid_synth(self.synth.as_ptr());
            ffi::delete_fluid_settings(self.settings.as_ptr());
        }
    }
}

/// A SoundFont loaded into an instance (FluidSynth's id; valid only for the instance that loaded it)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SoundFontId(i32);

/// A preset of a SoundFont
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub bank: u32,
    pub program: u32,
    pub name: String,
}

/// What a MIDI channel plays as
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelType {
    Melodic,
    /// The GM percussion behaviour (bank 128, no note-off needed for most drums)
    Drum,
}

fn rejected() -> Error {
    // No arguments: this is made on the audio thread and must not allocate
    Error::new("fluidsynth.event_rejected")
}

fn ok(code: c_int) -> Result<()> {
    if code == ffi::FLUID_FAILED { Err(rejected()) } else { Ok(()) }
}

fn int(value: usize) -> c_int {
    c_int::try_from(value).unwrap_or(c_int::MAX)
}

impl Instance {
    fn ptr(&mut self) -> *mut ffi::Synth {
        self.synth.as_ptr()
    }

    fn program_select(&mut self, channel: usize, font: SoundFontId, bank: u32, program: u32) -> Result<()> {
        // SAFETY: the synth is valid and ours (&mut); FluidSynth checks the channel, font, bank and preset
        ok(unsafe {
            ffi::fluid_synth_program_select(self.ptr(), int(channel), font.0, int(bank as usize), int(program as usize))
        })
    }

    fn cc(&mut self, channel: usize, controller: u8, value: u8) -> Result<()> {
        // SAFETY: the synth is valid and ours; FluidSynth checks the ranges
        ok(unsafe { ffi::fluid_synth_cc(self.ptr(), int(channel), c_int::from(controller), c_int::from(value)) })
    }
}

/// An instance being prepared, off the audio thread: the only place a SoundFont can be loaded. `Send`, not `Sync`
pub struct SynthBuilder {
    instance: Instance,
}

impl SynthBuilder {
    /// Makes a silent instance: threadsafe API off, one core (no threads), reverb and chorus off (the kit is dry; the
    /// mixer has the reverb), `config.outputs` stereo outputs. Allocates: build time only.
    ///
    /// Errors: `fluidsynth.bad_config` (with the setting), `fluidsynth.new_failed` (with FluidSynth's message)
    pub fn new(config: SynthConfig) -> Result<Self> {
        if !(1..=MAX_OUTPUTS).contains(&config.outputs) {
            return Err(Error::new("fluidsynth.bad_config")
                .arg("setting", "outputs")
                .arg("value", config.outputs as i64));
        }
        if config.midi_channels == 0 || !config.midi_channels.is_multiple_of(16) || config.midi_channels > 256 {
            return Err(Error::new("fluidsynth.bad_config")
                .arg("setting", "midi_channels")
                .arg("value", config.midi_channels as i64));
        }
        let (made, messages) = capturing(|| Self::make(config));
        made.map_err(|e| with_messages(e, &messages))
    }

    fn make(config: SynthConfig) -> Result<Self> {
        let settings = NonNull::new(ffi::new_fluid_settings()).ok_or_else(|| Error::new("fluidsynth.new_failed"))?;
        // Owns the settings until the synth exists
        struct SettingsGuard(Option<NonNull<ffi::Settings>>);
        impl Drop for SettingsGuard {
            fn drop(&mut self) {
                if let Some(s) = self.0 {
                    // SAFETY: made by new_fluid_settings just above, not yet given to an Instance
                    unsafe { ffi::delete_fluid_settings(s.as_ptr()) };
                }
            }
        }
        let mut guard = SettingsGuard(Some(settings));

        let set_int = |name: &CStr, value: c_int| -> Result<()> {
            // SAFETY: valid settings and a NUL-terminated name
            let code = unsafe { ffi::fluid_settings_setint(settings.as_ptr(), name.as_ptr(), value) };
            if code != ffi::FLUID_OK {
                return Err(Error::new("fluidsynth.bad_config")
                    .arg("setting", name.to_string_lossy().into_owned())
                    .arg("value", i64::from(value)));
            }
            Ok(())
        };
        let set_num = |name: &CStr, value: f64| -> Result<()> {
            // SAFETY: valid settings and a NUL-terminated name
            let code = unsafe { ffi::fluid_settings_setnum(settings.as_ptr(), name.as_ptr(), value) };
            if code != ffi::FLUID_OK {
                return Err(Error::new("fluidsynth.bad_config")
                    .arg("setting", name.to_string_lossy().into_owned())
                    .arg("value", value));
            }
            Ok(())
        };
        // U33: only the audio thread calls the instance; no threads of its own
        set_int(c"synth.threadsafe-api", 0)?;
        set_int(c"synth.cpu-cores", 1)?;
        set_int(c"synth.reverb.active", 0)?;
        set_int(c"synth.chorus.active", 0)?;
        set_int(c"synth.dynamic-sample-loading", 0)?;
        set_int(c"synth.audio-channels", int(config.outputs))?;
        set_int(c"synth.audio-groups", int(config.outputs))?;
        set_int(c"synth.midi-channels", int(config.midi_channels))?;
        set_int(c"synth.polyphony", int(config.polyphony))?;
        set_num(c"synth.sample-rate", config.sample_rate)?;
        set_num(c"synth.gain", f64::from(config.gain))?;

        // SAFETY: valid settings; the synth keeps a pointer to them, so they are deleted after it (Instance::drop)
        let synth = unsafe { ffi::new_fluid_synth(settings.as_ptr()) };
        let synth = NonNull::new(synth).ok_or_else(|| Error::new("fluidsynth.new_failed"))?;
        guard.0 = None;
        let serial = SERIALS.fetch_add(1, Ordering::Relaxed);
        let mut instance = Instance { settings, synth, config, serial };
        // SAFETY: the synth is valid and ours
        let (channels, groups) = unsafe {
            (ffi::fluid_synth_count_audio_channels(instance.ptr()), ffi::fluid_synth_count_audio_groups(instance.ptr()))
        };
        if channels != int(config.outputs) || groups != int(config.outputs) {
            return Err(Error::new("fluidsynth.bad_config")
                .arg("setting", "outputs")
                .arg("value", i64::from(channels)));
        }
        Ok(Self { instance })
    }

    /// Loads a SoundFont (SF2, or SF3 if FluidSynth was built with Vorbis), samples and all. Reads the file and
    /// allocates: build time only. Errors: `fluidsynth.load_failed` (with the path and FluidSynth's message)
    pub fn load_soundfont(&mut self, path: &Path) -> Result<SoundFontId> {
        let load_error = || Error::new("fluidsynth.load_failed").arg("path", path.display().to_string());
        let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| load_error())?;
        let synth = self.instance.ptr();
        let (id, messages) = capturing(|| {
            let _single = OpenMpSingleThread::enter();
            // SAFETY: the synth is valid and ours (&mut self), the name is NUL-terminated; reset_presets = 1 points
            // every channel at the new font's presets
            unsafe { ffi::fluid_synth_sfload(synth, name.as_ptr(), 1) }
        });
        if id == ffi::FLUID_FAILED {
            return Err(with_messages(load_error(), &messages));
        }
        Ok(SoundFontId(id))
    }

    /// The presets of a loaded SoundFont, in FluidSynth's order (bank, then program)
    pub fn presets(&mut self, font: SoundFontId) -> Result<Vec<Preset>> {
        // SAFETY: the synth is valid and ours; an unknown id gives NULL
        let sfont = unsafe { ffi::fluid_synth_get_sfont_by_id(self.instance.ptr(), font.0) };
        if sfont.is_null() {
            return Err(Error::new("fluidsynth.no_such_soundfont").arg("id", i64::from(font.0)));
        }
        let mut presets = Vec::new();
        // SAFETY: `sfont` belongs to the synth and lives while we hold &mut self; the iteration functions and the
        // preset getters are called on the presets it returns, which live as long as the font; names are
        // NUL-terminated strings owned by the presets, copied at once
        unsafe {
            ffi::fluid_sfont_iteration_start(sfont);
            loop {
                let preset = ffi::fluid_sfont_iteration_next(sfont);
                if preset.is_null() {
                    break;
                }
                let name = ffi::fluid_preset_get_name(preset);
                let name =
                    if name.is_null() { String::new() } else { CStr::from_ptr(name).to_string_lossy().into_owned() };
                presets.push(Preset {
                    bank: u32::try_from(ffi::fluid_preset_get_banknum(preset)).unwrap_or(0),
                    program: u32::try_from(ffi::fluid_preset_get_num(preset)).unwrap_or(0),
                    name,
                });
            }
        }
        Ok(presets)
    }

    /// Points `channel` at a preset of a loaded font
    pub fn program_select(&mut self, channel: usize, font: SoundFontId, bank: u32, program: u32) -> Result<()> {
        self.instance.program_select(channel, font, bank, program)
    }

    /// Makes `channel` melodic or percussion
    pub fn set_channel_type(&mut self, channel: usize, kind: ChannelType) -> Result<()> {
        let kind = match kind {
            ChannelType::Melodic => 0,
            ChannelType::Drum => 1,
        };
        // SAFETY: the synth is valid and ours; FluidSynth checks the channel
        ok(unsafe { ffi::fluid_synth_set_channel_type(self.instance.ptr(), int(channel), kind) })
    }

    /// Sets a controller before playing (AvlDrums pans its mono pieces with CC 10)
    pub fn cc(&mut self, channel: usize, controller: u8, value: u8) -> Result<()> {
        self.instance.cc(channel, controller, value)
    }

    /// The instance, ready to move to the audio thread (with room to list every voice, for
    /// [`Synth::voices_of`])
    pub fn build(mut self) -> Synth {
        // SAFETY: the synth is valid and ours
        let polyphony = unsafe { ffi::fluid_synth_get_polyphony(self.instance.ptr()) };
        let room = usize::try_from(polyphony).unwrap_or(0).max(self.instance.config.polyphony);
        Synth { instance: self.instance, voice_list: VoiceList(vec![std::ptr::null_mut(); room].into_boxed_slice()) }
    }
}

impl fmt::Debug for SynthBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SynthBuilder").field("config", &self.instance.config).finish()
    }
}

/// A playing instance, for the audio thread: notes, controllers, programs and render, none of which allocates, locks
/// or does I/O. It cannot load a SoundFont. `Send`, not `Sync`; dropped on the main thread (rule 5)
pub struct Synth {
    instance: Instance,
    voice_list: VoiceList,
}

/// Room for FluidSynth's list of the voices playing (one pointer per voice of the polyphony)
struct VoiceList(Box<[*mut ffi::Voice]>);

// SAFETY: the pointers are only scratch written by fluid_synth_get_voicelist and read right after, by the owner of the
// `Synth` (&mut self); they are never dereferenced from another thread
unsafe impl Send for VoiceList {}

/// A voice that a note started, to change its generators while it plays (the per-note pitch of the Sf2 Player).
/// Plain data: it names the voice of one instance, and the [`Synth`] checks it is still that voice before touching it
/// (The default names no voice: serial 0 is never an instance's)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoiceRef {
    serial: u64,
    /// The voice's address in the instance's voice table (allocated with the instance, never moved: the polyphony is
    /// never changed after it is made)
    address: usize,
    /// FluidSynth's id of the voice when the note started: a voice reused by another note has another id
    id: u32,
}

/// The SoundFont generators a voice may be given while it plays (the values are in the SoundFont's units)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Generator {
    /// Semitones, fractional allowed (FluidSynth keeps generators as floats)
    CoarseTune,
    /// Timecents
    VolEnvDelay,
    VolEnvAttack,
    VolEnvHold,
    VolEnvDecay,
    /// Centibels of attenuation
    VolEnvSustain,
    VolEnvRelease,
    /// FluidSynth's own balance (centibels of attenuation of one side: negative attenuates the right, positive the
    /// left; 1440 silences it)
    CustomBalance,
}

impl Generator {
    /// FluidSynth's number (`enum fluid_gen_type`, the SoundFont 2.04 order)
    fn number(self) -> c_int {
        match self {
            Generator::VolEnvDelay => 33,
            Generator::VolEnvAttack => 34,
            Generator::VolEnvHold => 35,
            Generator::VolEnvDecay => 36,
            Generator::VolEnvSustain => 37,
            Generator::VolEnvRelease => 38,
            Generator::CoarseTune => 51,
            Generator::CustomBalance => 60,
        }
    }
}

/// How FluidSynth interpolates the samples
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    None,
    Linear,
    /// FluidSynth's default
    FourthOrder,
    SeventhOrder,
}

/// The reverb's settings (FluidSynth's units: room size, damping, width and level)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reverb {
    pub room_size: f64,
    pub damping: f64,
    pub width: f64,
    pub level: f64,
}

/// The chorus's settings: voices, level, speed (Hz), depth (ms); the modulation is a sine
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chorus {
    pub voices: u32,
    pub level: f64,
    pub speed: f64,
    pub depth: f64,
}

impl Synth {
    pub fn config(&self) -> &SynthConfig {
        &self.instance.config
    }

    /// The stereo outputs [`render`](Self::render) fills
    pub fn outputs(&self) -> usize {
        self.instance.config.outputs
    }

    /// The MIDI channels (from FluidSynth)
    pub fn midi_channels(&mut self) -> usize {
        // SAFETY: the synth is valid and ours
        usize::try_from(unsafe { ffi::fluid_synth_count_midi_channels(self.instance.ptr()) }).unwrap_or(0)
    }

    /// Starts a note (`velocity` 1 to 127; 0 is a note off, as in MIDI). Error `fluidsynth.event_rejected` when out
    /// of range or the channel has no preset
    pub fn note_on(&mut self, channel: usize, key: u8, velocity: u8) -> Result<()> {
        // SAFETY: the synth is valid and ours; FluidSynth checks the ranges
        ok(unsafe {
            ffi::fluid_synth_noteon(self.instance.ptr(), int(channel), c_int::from(key), c_int::from(velocity))
        })
    }

    /// Releases a note. `fluidsynth.event_rejected` also when the note was not playing (usually ignored)
    pub fn note_off(&mut self, channel: usize, key: u8) -> Result<()> {
        // SAFETY: as in note_on
        ok(unsafe { ffi::fluid_synth_noteoff(self.instance.ptr(), int(channel), c_int::from(key)) })
    }

    pub fn cc(&mut self, channel: usize, controller: u8, value: u8) -> Result<()> {
        self.instance.cc(channel, controller, value)
    }

    /// Pitch bend, 0 to 16383 (8192 is centre)
    pub fn pitch_bend(&mut self, channel: usize, value: u16) -> Result<()> {
        // SAFETY: as in note_on
        ok(unsafe { ffi::fluid_synth_pitch_bend(self.instance.ptr(), int(channel), c_int::from(value)) })
    }

    pub fn program_change(&mut self, channel: usize, program: u8) -> Result<()> {
        // SAFETY: as in note_on
        ok(unsafe { ffi::fluid_synth_program_change(self.instance.ptr(), int(channel), c_int::from(program)) })
    }

    pub fn bank_select(&mut self, channel: usize, bank: u16) -> Result<()> {
        // SAFETY: as in note_on
        ok(unsafe { ffi::fluid_synth_bank_select(self.instance.ptr(), int(channel), c_int::from(bank)) })
    }

    /// Points `channel` at a preset of a font this instance loaded while it was a builder
    pub fn program_select(&mut self, channel: usize, font: SoundFontId, bank: u32, program: u32) -> Result<()> {
        self.instance.program_select(channel, font, bank, program)
    }

    /// Releases every note of `channel` (`None`: of every channel); they end with their release
    pub fn all_notes_off(&mut self, channel: Option<usize>) -> Result<()> {
        let chan = channel.map_or(-1, int);
        // SAFETY: as in note_on (-1 means every channel)
        ok(unsafe { ffi::fluid_synth_all_notes_off(self.instance.ptr(), chan) })
    }

    /// Silences every voice of `channel` (`None`: of every channel) at once, without release
    pub fn all_sounds_off(&mut self, channel: Option<usize>) -> Result<()> {
        let chan = channel.map_or(-1, int);
        // SAFETY: as in note_on (-1 means every channel)
        ok(unsafe { ffi::fluid_synth_all_sounds_off(self.instance.ptr(), chan) })
    }

    /// Renders into the outputs, **replacing** what they held: `out` has `2 × outputs()` slices (left and right of
    /// output 0, then of output 1...), all of the same length, the block's frame count. Neither allocates nor locks.
    /// Error `fluidsynth.bad_buffers` when the slices do not match
    pub fn render(&mut self, out: &mut [&mut [f32]]) -> Result<()> {
        let outputs = self.instance.config.outputs;
        let Some(frames) = out.first().map(|b| b.len()) else {
            return Err(Error::new("fluidsynth.bad_buffers"));
        };
        if out.len() != outputs * 2 || out.iter().any(|b| b.len() != frames) {
            return Err(Error::new("fluidsynth.bad_buffers"));
        }
        let Ok(len) = c_int::try_from(frames) else {
            return Err(Error::new("fluidsynth.bad_buffers"));
        };
        // FluidSynth adds into the buffers
        let mut pointers = [std::ptr::null_mut::<c_float>(); MAX_OUTPUTS * 2];
        for (slot, buffer) in pointers.iter_mut().zip(out.iter_mut()) {
            buffer.fill(0.0);
            *slot = buffer.as_mut_ptr();
        }
        // SAFETY: the synth is valid and ours (&mut self). `pointers[..2 * outputs]` are distinct mutable buffers of
        // exactly `len` floats each (checked above), borrowed for the call; FluidSynth writes `len` frames into each
        // of the `nout` buffers and nothing else (no effects buffers: nfx = 0). nout = 2 × outputs = 2 × the synth's
        // audio channels, so every dry output has its buffer
        let code = unsafe {
            ffi::fluid_synth_process(
                self.instance.ptr(),
                len,
                0,
                std::ptr::null_mut(),
                int(outputs * 2),
                pointers.as_mut_ptr(),
            )
        };
        if code != ffi::FLUID_OK {
            return Err(Error::new("fluidsynth.render_failed"));
        }
        Ok(())
    }
}

impl Synth {
    /// Renders one stereo output with the reverb and chorus mixed in (FluidSynth's `write_float`), **replacing** what
    /// `left` and `right` held; for an instance of one output. Voice changes made since the last render take effect
    /// first. Neither allocates nor locks. Error `fluidsynth.bad_buffers` when the lengths differ or the instance has
    /// more than one output
    pub fn render_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<()> {
        if left.len() != right.len() || self.instance.config.outputs != 1 {
            return Err(Error::new("fluidsynth.bad_buffers"));
        }
        let Ok(len) = c_int::try_from(left.len()) else {
            return Err(Error::new("fluidsynth.bad_buffers"));
        };
        if len == 0 {
            return Ok(());
        }
        let synth = self.instance.ptr();
        // SAFETY: the synth is valid and ours (&mut self); `left` and `right` are distinct buffers of `len` floats,
        // written with offset 0 and stride 1, `len` frames each. `fluid_synth_get_gain` is called first only for its
        // side effect: as a public call it flushes the voice changes queued by `set_voice_gen` (the legacy Sf2 Player
        // does the same)
        let code = unsafe {
            ffi::fluid_synth_get_gain(synth);
            ffi::fluid_synth_write_float(synth, len, left.as_mut_ptr().cast(), 0, 1, right.as_mut_ptr().cast(), 0, 1)
        };
        if code != ffi::FLUID_OK {
            return Err(Error::new("fluidsynth.render_failed"));
        }
        Ok(())
    }

    /// The frames FluidSynth renders at once (64): an event takes effect at the start of its next block of the
    /// instance's own frame count
    pub fn internal_block(&mut self) -> usize {
        // SAFETY: the synth is valid and ours
        usize::try_from(unsafe { ffi::fluid_synth_get_internal_bufsize(self.instance.ptr()) }).unwrap_or(64)
    }

    /// The master gain (FluidSynth's `synth.gain`, 0 to 10)
    pub fn set_gain(&mut self, gain: f32) {
        // SAFETY: the synth is valid and ours; FluidSynth clamps the value
        unsafe { ffi::fluid_synth_set_gain(self.instance.ptr(), gain) };
    }

    /// Turns the reverb on or off (every effects group)
    pub fn set_reverb_on(&mut self, on: bool) -> Result<()> {
        // SAFETY: the synth is valid and ours; -1 is every group
        ok(unsafe { ffi::fluid_synth_reverb_on(self.instance.ptr(), -1, c_int::from(on)) })
    }

    /// The reverb's settings (every effects group)
    pub fn set_reverb(&mut self, r: Reverb) -> Result<()> {
        let synth = self.instance.ptr();
        // SAFETY: the synth is valid and ours; FluidSynth checks the ranges; -1 is every group
        unsafe {
            ok(ffi::fluid_synth_set_reverb_group_roomsize(synth, -1, r.room_size))?;
            ok(ffi::fluid_synth_set_reverb_group_damp(synth, -1, r.damping))?;
            ok(ffi::fluid_synth_set_reverb_group_width(synth, -1, r.width))?;
            ok(ffi::fluid_synth_set_reverb_group_level(synth, -1, r.level))
        }
    }

    /// Turns the chorus on or off (every effects group)
    pub fn set_chorus_on(&mut self, on: bool) -> Result<()> {
        // SAFETY: the synth is valid and ours; -1 is every group
        ok(unsafe { ffi::fluid_synth_chorus_on(self.instance.ptr(), -1, c_int::from(on)) })
    }

    /// The chorus's settings (every effects group), with a sine modulation
    pub fn set_chorus(&mut self, c: Chorus) -> Result<()> {
        let synth = self.instance.ptr();
        let voices = c_int::try_from(c.voices).unwrap_or(c_int::MAX);
        // SAFETY: the synth is valid and ours; FluidSynth checks the ranges (at most 99 voices, all preallocated);
        // -1 is every group; type 0 is FLUID_CHORUS_MOD_SINE
        unsafe {
            ok(ffi::fluid_synth_set_chorus_group_nr(synth, -1, voices))?;
            ok(ffi::fluid_synth_set_chorus_group_level(synth, -1, c.level))?;
            ok(ffi::fluid_synth_set_chorus_group_speed(synth, -1, c.speed))?;
            ok(ffi::fluid_synth_set_chorus_group_depth(synth, -1, c.depth))?;
            ok(ffi::fluid_synth_set_chorus_group_type(synth, -1, 0))
        }
    }

    /// The pitch bend's range of `channel`, semitones
    pub fn set_bend_range(&mut self, channel: usize, semitones: u8) -> Result<()> {
        // SAFETY: the synth is valid and ours; FluidSynth checks the ranges
        ok(unsafe { ffi::fluid_synth_pitch_wheel_sens(self.instance.ptr(), int(channel), c_int::from(semitones)) })
    }

    /// How every channel interpolates its samples
    pub fn set_interpolation(&mut self, method: Interpolation) -> Result<()> {
        let m = match method {
            Interpolation::None => 0,
            Interpolation::Linear => 1,
            Interpolation::FourthOrder => 4,
            Interpolation::SeventhOrder => 7,
        };
        // SAFETY: the synth is valid and ours; -1 is every channel
        ok(unsafe { ffi::fluid_synth_set_interp_method(self.instance.ptr(), -1, m) })
    }

    /// The voices playing `key` on `channel` that are on (not released), written to `out` (as many as fit); returns
    /// how many. After a `note_on`, these are the voices it started: FluidSynth stops the voices of the same channel
    /// and key at a note-on. Neither allocates nor locks
    pub fn voices_of(&mut self, channel: usize, key: u8, out: &mut [VoiceRef]) -> usize {
        let synth = self.instance.ptr();
        let list = &mut self.voice_list.0;
        list.fill(std::ptr::null_mut());
        // SAFETY: the synth is valid and ours; `list` has room for `list.len()` pointers, which FluidSynth fills with
        // the playing voices and ends with NULL when there are fewer (id -1: every voice)
        unsafe { ffi::fluid_synth_get_voicelist(synth, list.as_mut_ptr(), int(list.len()), -1) };
        let mut found = 0;
        for &voice in list.iter() {
            if voice.is_null() || found == out.len() {
                break;
            }
            // SAFETY: `voice` is one of the instance's voices, just listed by FluidSynth; it lives as long as the
            // instance (the voice table is never reallocated: the polyphony is never changed)
            let (chan, k, on, id) = unsafe {
                (
                    ffi::fluid_voice_get_channel(voice),
                    ffi::fluid_voice_get_key(voice),
                    ffi::fluid_voice_is_on(voice),
                    ffi::fluid_voice_get_id(voice),
                )
            };
            if chan == int(channel) && k == c_int::from(key) && on != 0 {
                out[found] = VoiceRef { serial: self.instance.serial, address: voice as usize, id };
                found += 1;
            }
        }
        found
    }

    /// The voice, if it is still the one `v` names and it plays
    fn live(&mut self, v: &VoiceRef) -> Option<*mut ffi::Voice> {
        if v.serial != self.instance.serial {
            return None;
        }
        let voice = v.address as *mut ffi::Voice;
        // SAFETY: a VoiceRef with this instance's serial was made by `voices_of` from this instance's voice table,
        // whose voices live as long as the instance (never reallocated)
        let (id, playing) = unsafe { (ffi::fluid_voice_get_id(voice), ffi::fluid_voice_is_playing(voice)) };
        (id == v.id && playing != 0).then_some(voice)
    }

    /// A generator of a voice, or `None` when the voice has ended or plays another note
    pub fn voice_gen(&mut self, v: &VoiceRef, generator: Generator) -> Option<f32> {
        let voice = self.live(v)?;
        // SAFETY: a live voice of this instance (see `live`)
        Some(unsafe { ffi::fluid_voice_gen_get(voice, generator.number()) })
    }

    /// Sets a generator of a voice while it plays; it takes effect from the next render. `false` when the voice has
    /// ended or plays another note (nothing is done). Neither allocates nor locks
    pub fn set_voice_gen(&mut self, v: &VoiceRef, generator: Generator, value: f32) -> bool {
        let Some(voice) = self.live(v) else { return false };
        // SAFETY: a live voice of this instance (see `live`), on the thread that owns the instance (&mut self)
        unsafe {
            ffi::fluid_voice_gen_set(voice, generator.number(), value);
            ffi::fluid_voice_update_param(voice, generator.number());
        }
        true
    }
}

impl fmt::Debug for Synth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Synth").field("config", &self.instance.config).finish()
    }
}
