//! `lm-sys-jack`: the safe binding to JACK (libjack) under the JACK backend of `lm-io` (interfaces.md, I-10, I-17;
//! F-REC-125): a client with audio and MIDI ports in one process callback, connections, latencies, and the server's
//! notifications (xruns, shutdown, ports appearing). It works with JACK2, JACK1 and PipeWire's libjack (`pw-jack`).
//!
//! It is a binding, not a backend: it does not implement `Backend`, knows no `Renderer` and has no barrier (those are
//! `lm-io`'s, I-10.1, S16).
//!
//! # Safety sheet (ficha, I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | **Who owns each handle?** | [`Client`] owns the `jack_client_t` (closed on drop, which unregisters and disconnects its ports); its ports are owned through it (a [`Port`] is an index, valid only with its client). [`ActiveClient`] owns the client and the boxed state the trampolines reach, and on drop deactivates (waiting for the last cycle), closes, then frees the state: no callback can reach freed memory. The port lists of `jack_get_ports` are freed with `jack_free` at once, their names copied. |
//! | **Which thread may call what?** | By the types. `Client` and `ActiveClient` are `Send`, not `Sync`: opened on a worker thread, used by one thread at a time. The [`Process`] handler runs only on JACK's real-time thread, one cycle at a time; the [`Notify`] handler (`Sync`) on JACK's notification threads, concurrently with it. Opening, connecting, listing ports, deactivating and closing block on the server: never on the main thread, never in the process handler. |
//! | **Allocates, locks, does I/O?** | **Real-time safe** (the process trampoline and everything [`Cycle`] does): `jack_port_get_buffer`, the MIDI buffer calls and `jack_get_cycle_times` touch only shared memory; the copies are `memcpy`. No allocation, no lock of ours. **Not real-time safe**: everything of `Client` (open, register, ports, connect, disconnect, latency queries), activation, deactivation and close (server round trips); the notification trampolines copy port names into `String`s (allocation, on JACK's non-real-time threads). |
//! | **Creates threads?** | libjack creates the client's real-time thread and its notification thread (JACK2: one each). Both have stopped calling us when `jack_client_close` returns, before the state they use is freed. |
//! | **Calls our code back?** | Yes: process, xrun, shutdown, buffer size, port registration and port connection, each through an `extern "C"` trampoline that catches panics (`catch_unwind`); a panic in the process handler leaves the client playing silence (and [`ActiveClient::panicked`] true), never unwinds into C. |
//! | **Tests** | Platform independent: none need a server to build. With `jackd` on the `PATH` (Homebrew's `jack`, Debian's `jackd2`), the tests start their own server with the dummy driver under a name of their own: open, register, connect, a cycle that copies audio through and sends MIDI round a loop of connections, latencies, the cycle times, xrun and shutdown notifications, and the allocation trap over the process handler. Without `jackd` they say so on stderr and pass. |
//!
//! # Why hand-written declarations and not `jack-sys`
//!
//! As for ALSA: about forty functions, stable since JACK 0.121 in all three implementations, and no build-time
//! dependency beyond pkg-config (the build script links libjack; with PipeWire it is `pipewire-jack`'s).
//!
//! # The error keys
//!
//! `jack.server_not_running` (open: no server, the backend says how to start one), `jack.version_mismatch`,
//! `jack.open_failed`, `jack.bad_name`, `jack.port_register_failed`, `jack.connect_failed`, `jack.disconnect_failed`,
//! `jack.callback_failed`, `jack.activate_failed`.

mod client;
mod ffi;

pub use client::{
    ActiveClient, Client, Cycle, CycleTimes, Direction, LatencyMode, LatencyRange, Notify, Port, PortFilter, PortKind,
    Process, jack_time,
};
