//! `lm-sys-linux`: the Linux system services the audio engine needs (interfaces.md, H1, H2, U8; I-17):
//!
//! - real-time scheduling of the audio threads: `SCHED_FIFO` directly when `RLIMIT_RTPRIO` allows, else through rtkit,
//!   which grants `SCHED_RR` (the same as FIFO for one thread per priority level; both are accepted)
//!   ([`make_current_thread_realtime`]), and what is missing when neither works (`linux.realtime.denied`, with the
//!   limit and the `audio` group) (H1);
//! - memory locking by the rule of revisão V6: [`lock_all`] only when `RLIMIT_MEMLOCK` is unlimited, otherwise
//!   [`LockedBuffer`]s and [`lock_current_stack`]; [`prefault_stack`] (H1);
//! - the systemd-logind sleep inhibitor ([`SystemBus::inhibit_sleep`], a [`SleepInhibitor`] guard) (H2);
//! - ReserveDevice1, to ask PipeWire or PulseAudio to release an ALSA card ([`SystemBus::reserve_audio_device`], a
//!   [`Reservation`] guard) (U8).
//!
//! D-Bus is `zbus` (pure Rust) on a thread of its own (decisoes-autonomas.md). On other platforms the crate is empty.
//!
//! # Safety sheet (ficha, I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | **Who owns each handle?** | One guard with `Drop` per resource: [`RealtimeGuard`] (the thread's previous policy, restored on drop), [`LockAll`] (`munlockall`), [`LockedBuffer`] (its own `mmap`, unmapped and so unlocked on drop), [`StackLock`] (`munlock`), [`SleepInhibitor`] (logind's descriptor, closed on drop), [`Reservation`] (the bus name and object, released by the D-Bus thread), [`SystemBus`] (the thread). The D-Bus connections belong to the D-Bus thread alone. |
//! | **Which thread may call what?** | By the types. `RealtimeGuard` and `StackLock` are not `Send`: made and dropped on the thread they act on. `SystemBus` is `Send + Sync`; its requests return a [`Reply`] at once, which the main thread only polls ([`Reply::try_take`]) and other threads may wait on. [`make_current_thread_realtime`] blocks for the rtkit round trip: called once by an audio thread as it starts, never by the main thread. |
//! | **Allocates, locks, does I/O?** | Nothing here is real-time safe, and nothing here is called by an audio processor: all of it prepares a thread or the process before the real-time loop, or releases after it. The only calls an audio thread makes are at its start and end (`make_current_thread_realtime`, `lock_current_stack`, and the guards' drops: one `sched_setscheduler` or `munlock`). Reading and writing a `LockedBuffer` is plain memory. |
//! | **Creates threads?** | **"lm-dbus"** ([`SystemBus::start`]): owns both bus connections, runs one request at a time inside `zbus::block_on`, and ends when its handle and every `Reservation` are dropped or on [`SystemBus::shutdown`]; it releases every reservation and closes the connections before ending, and it owns everything it uses, so it never outlives anything it touches. **zbus's helper thread** (from `zbus::block_on`, zbus 6): drives the connections between our requests, so the ReserveDevice1 object answers other programs; it ends when the connections are dropped by "lm-dbus". |
//! | **Calls our code back?** | No C callbacks. zbus calls `ReserveDevice::request_release` and its property getters (plain Rust, on its helper thread): they only read fields and bump an atomic counter, and cannot panic. |
//! | **Tests** | On Linux, in CI: limits and groups; memory locking within the limit (a buffer, a stack) and the clear error beyond it; scheduling (a direct `SCHED_FIFO` or rtkit's `SCHED_RR` where allowed, otherwise the `linux.realtime.denied` error with its arguments, and the guard restoring the policy); the D-Bus requests against the real buses when present, and their errors when absent (CI containers have no system bus: those tests say so on stderr and pass). On a real machine the rtkit and logind tests must run in the user's session, e.g. `systemd-run --user --wait --pipe <test binary>`: over plain SSH polkit refuses both. Under ASan, LSan and TSan with the rest of the workspace. |
//!
//! # Error keys
//!
//! `linux.realtime.denied`, `linux.rtkit.unavailable`, `linux.rtkit.refused`, `linux.rtkit.not_applied`,
//! `linux.memlock.limited`, `linux.memlock.denied`, `linux.stack_unknown`, `linux.rlimit_failed`,
//! `linux.groups_failed`, `linux.out_of_memory`, `linux.dbus.unavailable` (with `bus`), `linux.dbus.timeout`,
//! `linux.dbus.thread_gone`, `linux.logind.unavailable`, `linux.reserve.refused` (with `owner` when known),
//! `linux.reserve.failed`, `linux.reserve.already_held`, `linux.thread_failed`, `linux.invalid_argument`.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    bus::{CALL_TIMEOUT, Reply, Reservation, ReserveRequest, RtkitInfo, SleepInhibitor, SystemBus},
    mem::{LockAll, LockedBuffer, StackLock, Zeroable, lock_all, lock_current_stack, memlock_limit, prefault_stack},
    rt::{
        Limit, RealtimeGuard, ResourceLimits, RtMethod, SchedPolicy, ThreadId, current_policy, current_thread_id,
        in_audio_group, in_group, make_current_thread_realtime, resource_limits,
    },
};
