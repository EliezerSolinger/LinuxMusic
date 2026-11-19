//! `lm-sys-alsa`: the safe binding to ALSA (libasound) under the F1 audio backend of `lm-io` (interfaces.md, I-10,
//! I-17; Parte 6 U7/U8/U11/U17; Parte 7 C2/C6/C7): PCM playback and capture, the sequencer (MIDI in and out) and
//! device enumeration.
//!
//! It is a binding, not a backend: it does not implement `Backend`, knows no `Renderer` and has no barrier (those are
//! `lm-io`'s, I-10.1, S16). On platforms other than Linux it has only the platform-independent parts (sample formats
//! and their conversion, the configuration and status types, the error keys, MIDI translation), so the workspace
//! builds and tests everywhere.
//!
//! # Safety sheet (ficha, I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | **Who owns each handle?** | One type with `Drop` per C object, one owner each. [`Pcm`] owns an `snd_pcm_t` (unlinked and closed on drop) and its `snd_pcm_status_t`. [`Seq`] owns an `snd_seq_t` (closing it removes its ports, connections and queue). [`MidiInput`] owns its `Seq` and its queue status object. Private guards free every temporary object (`hw_params`, `sw_params`, `pcm_info`, `ctl` handle, card info, port/client info, subscriptions, the hint array, the strings of hints). |
//! | **Which thread may call what?** | By the types. `Pcm`, `Seq`, `MidiInput` and `PcmPoll` are `Send` and not `Sync`: built on a worker thread, moved to the thread that uses them, never shared. [`Waker`] and [`MidiStopper`] are `Send + Sync`: the only things another thread touches. Enumeration functions are free functions for an `Operation` (U17). Nothing here may run on the main thread: opening, configuring and enumerating can block. |
//! | **Allocates, locks, does I/O?** | **Real-time safe** (the audio thread's calls; no allocation, no lock of ours, at most one system call): `Pcm::{write_interleaved, write_planar, write_silence, read_interleaved, read_planar, avail_update, avail, delay, htimestamp, status, state, xrun_info, trigger_time, start, prepare, stop_now, recover(Xrun)}`, `PcmPoll::wait`, `Waker::wake`, `MidiStopper::stop`, and the pure functions (`SampleFormat::{encode, decode}`, `lost_capture_frames`). Under them, libasound's `snd_pcm_mmap_begin/commit`, `avail_update`, `htimestamp`, `state` touch only the shared status page for `hw` devices; `snd_pcm_avail`, `delay`, `status`, `start`, `prepare`, `drop`, `readi/writei/readn/writen` and `poll` are one syscall each, no allocation. **Not real-time safe**: `Pcm::{open, configure, link, unlink, drain (blocks a whole buffer)}`, `recover(Suspended)` (may need retries), `PcmPoll::new`, `Waker::new`, everything in `enumerate`, everything of `Seq` and `MidiInput` (the MIDI thread is not real-time: it blocks in `poll` and hands events over by a ring). Libasound reads its configuration files (and allocates a global cache) on the first open of the process. |
//! | **Creates threads?** | None. libasound creates none for the `hw`, `plughw`, `null` and sequencer paths used here. (The `pipewire`/`pulse` ALSA plugins do create threads; the program opens `hw` devices, U7.) |
//! | **Calls our code back?** | No callbacks are registered with libasound (no async handlers, no `snd_lib_error_set_handler`), so there is no trampoline. The `sink` of [`MidiInput::read`] is a plain Rust closure called by Rust code. |
//! | **Tests** | Platform independent: the conversions, the error keys, the MIDI translation, the C2 arithmetic (every platform). Linux, in CI: the ALSA `null` PCM (open, configure every access type, start, write, read, status with monotonic time stamps, recover from an xrun, poll and wake), the `snd-aloop` loopback when the runner has it (link, real xruns and the lost-frame count), the sequencer with two of our own ports (routing, stamps converted to host time, the stopper), enumeration with or without cards. The CI runs them under ASan, LSan and TSan too. Tests that need something missing (no sequencer, no loopback) say so on stderr and pass. |
//!
//! # Why hand-written declarations and not `alsa-sys`
//!
//! `alsa-sys`'s build script asks `pkg-config` for `alsa`, which fails when checking the Linux code from another
//! platform (`cargo check --target x86_64-unknown-linux-gnu` on the Mac) and needs a cross pkg-config setup for every
//! target. The C API used is small (about 150 functions), stable for 15 years, and the two structs read by layout
//! are checked by size at compile time. So the declarations are in `src/linux/ffi.rs`, with `#[link(name =
//! "asound")]`: checking needs nothing, linking needs `libasound2-dev` (Debian/Ubuntu) or `alsa-lib-devel`.
//!
//! # The error keys
//!
//! In [`error`]: `alsa.device_busy` and `alsa.permission_denied` are distinct (U8); `alsa.device_disconnected`
//! after open (U11); the configuration failures carry the device's ranges.

pub mod error;
mod format;
mod midi;
mod pcm_types;

pub use error::{PcmIoError, error_from_code};
pub use format::SampleFormat;
pub use midi::{Addr, MidiMessage};
pub use pcm_types::{
    Access, AccessChoice, HwConfig, HwRequest, PcmState, PcmStatus, Stream, XrunInfo, lost_capture_frames,
};

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    enumerate,
    pcm::{Pcm, strerror},
    poll::{PcmPoll, PollOutcome, Waker},
    seq::{InputStats, MidiInput, MidiInputEvent, MidiStopper, ReadOutcome, Seq, SeqInput, SeqPortInfo, Stamp},
};
