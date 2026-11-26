//! `lm-sys-coreaudio`: the safe binding to CoreAudio (macOS) under the CoreAudio backend of `lm-io` (interfaces.md,
//! I-10, I-17; Parte 6 U7/U11/U17): device enumeration and properties, an IO proc on one device, property listeners
//! (a device that went away, a processor overload) and private aggregate devices that the user asked for.
//!
//! It is a binding, not a backend: it does not implement `Backend`, knows no `Renderer` and has no barrier (those are
//! `lm-io`'s, I-10.1, S16). **On every system other than macOS the crate is empty**, so the Linux build and the
//! workspace's checks are not affected.
//!
//! # Safety sheet (ficha, I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | **Who owns each handle?** | One type with `Drop` per object, one owner each. [`Io`] owns an `AudioDeviceIOProcID` (stopped and destroyed on drop) and the boxed state its proc reads. [`Watch`] owns a property listener registration and its boxed closure (removed, then freed, on drop). [`Aggregate`] owns a private aggregate device (destroyed on drop). Every Core Foundation object made here (strings, numbers, arrays, the aggregate's description) is held by a private guard that releases it. [`Device`] is a plain `AudioObjectID`: a copyable name, not an owner (a device that went away makes its calls fail with an error, never undefined behaviour). |
//! | **Which thread may call what?** | The property calls are thread-safe in CoreAudio and are free functions or `&self` methods; none may run on the audio thread or on the main thread (they can block for a USB device: the caller runs them in an `Operation`, U17). [`Io`] is `Send`: built on one thread, started and stopped by the thread that owns it. The IO proc runs on CoreAudio's own real-time thread; a [`Watch`] closure runs on a CoreAudio notification thread, so it is `Send + Sync` and must only set flags. |
//! | **Allocates, locks, does I/O?** | **Real-time safe** (inside the IO proc, the closure given to [`Io::new`]): [`Cycle`]'s accessors and [`host_nanos`]: no allocation, no lock, no system call (the time base is read once by `Io::new`). **Not real-time safe**: every property call, `Io::{new, start, stop}` (`stop` blocks until the proc has returned for the last time), `Aggregate::create` (waits for the sub-devices to appear), and everything in enumeration. |
//! | **Creates threads?** | None of ours. CoreAudio runs the IO proc on a real-time thread it owns and the listeners on another of its own. |
//! | **Calls our code back?** | Yes, two trampolines: the IO proc and the property listener. Both catch panics (unwinding across the C frame is undefined behaviour): a panic in the IO proc silences the outputs and is reported by [`Io::panicked`] (the backend then reports `Panicked` and leaves); a panic in a listener is swallowed. The client pointer of each is a `Box` this crate owns and frees only after the registration is gone (`AudioDeviceStop` and `AudioDeviceDestroyIOProcID` for the proc, `AudioObjectRemovePropertyListener` for the listener). |
//! | **Tests** | Written for the Mac with devices (the machine's built-in output, run on the maintainer's Mac): enumeration, properties, an IO proc that counts cycles. Pure parts (four-character codes, struct sizes) are checked at compile time. Nothing here needs a microphone: an [`Io`] built with `inputs: false` turns the input streams off in its stream usage so macOS does not ask for the microphone permission. |
//!
//! # Why hand-written declarations and not `coreaudio-sys`
//!
//! `coreaudio-sys` generates its declarations with `bindgen` at build time, which needs libclang and the macOS SDK
//! headers on every machine that builds (and a different set of headers per SDK version). The CoreAudio calls used
//! are fewer than twenty and stable since 10.5; the structs read through pointers have their sizes checked at compile
//! time. So the declarations are in `src/ffi.rs`; the Core Foundation types come from `core-foundation-sys`
//! (declarations only, no build script). The choice is recorded in `docs/decisoes-autonomas.md`.
//!
//! # Why `AudioDeviceIOProc` and not an AUHAL AudioUnit
//!
//! One IO proc on one device gives input and output in the same call, on the device's own clock, with the host
//! time of both sides: exactly the shape of the `CallbackHandle` (one callback per period, capture in, playback
//! out). An AUHAL unit would add a second layer (render callbacks, a channel map, a format converter) for what the
//! program does itself (the planar float conversion, the edge resampler). Both need the same microphone permission.
#![cfg_attr(not(target_os = "macos"), allow(unused))]

#[cfg(target_os = "macos")]
mod ffi;
#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
pub use mac::*;
