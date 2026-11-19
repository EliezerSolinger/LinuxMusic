//! Safe binding to lilv, for hosting LV2 plugins: the system's library found by pkg-config.
//!
//! LV2 plugins are third-party plugins: they run **out of the main process**, in the plugin-host helper (interfaces.md
//! I-9.6), and the plugin scan runs in a helper too (H15). This crate is what that helper links: it is correct when
//! everything happens on one thread of the helper, and also when the audio part runs on a thread of its own.
//!
//! - [`World`]: the installed plugins, read from their Turtle data ([`World::plugins`] gives [`PluginInfo`] with
//!   the URI, name, class, ports with index, symbol, name, kind, direction, range and default, the required and
//!   optional features, the extension data and the latency port). Discovery never runs plugin code.
//! - [`World::instantiate`] → [`Instance`]: the plugin made with the features a host must give (URID map and unmap,
//!   options with the sample rate and the block lengths, the worker with a host-owned thread, state paths), its
//!   control, CV and atom ports connected to buffers the instance owns, and its audio ports connected to the host's
//!   buffers at every [`run`](Instance::run).
//! - The state as an opaque chunk ([`Instance::save_state`], [`Instance::restore_state`]): the plugin's `"chunk"` in
//!   the project tree (arquitetura-nova.md 7a).
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | `LilvWorld*` and the nodes the discovery compares against: one private `Core`, inside an `Arc` shared by the [`World`] clones and every [`Instance`] (lilv's plugin libraries belong to the world, so the world outlives its instances); freed by `lilv_world_free` when the last one goes. `LilvInstance*`: one [`Instance`], freed by `lilv_instance_free` in its `Drop`, after its worker thread is joined. The feature structs, the worker context and the port buffers the plugin holds pointers to: heap blocks owned by the `Instance`, at fixed addresses, freed **after** `lilv_instance_free`. Every node, node list and string lilv returns as owned is freed where it is read. [`PluginInfo`] is owned Rust data: no lilv pointer leaves the crate. |
//! | Which thread may call each function? | lilv is not thread-safe: every lilv call takes the world's mutex, so [`World`] is `Send` and `Sync`. An [`Instance`] is `Send` and **not `Sync`**, and every call takes `&mut self`: the LV2 threading classes are kept by the compiler (no `run` during `restore`, no two `run`s, no `save` during `restore`). The plugin's `work` runs only on the instance's worker thread (threaded mode) or inside `run` (synchronous mode), never both, never concurrently; before a `restore` (which may call `work` itself) the host waits until the worker is idle. |
//! | Allocates, locks, does I/O? | Host side of the **real-time-safe calls** (no allocation, no lock, no I/O): [`Instance::run`] (it calls the LV2 `connect_port` for the audio ports, `run`, `work_response` for each queued response and `end_run`, all of the LV2 audio class), [`Instance::set_control`], [`Instance::control`], [`Instance::latency`], [`Instance::push_event`], [`Instance::push_midi`], [`Instance::output_events`], and the host's `schedule_work` in threaded mode (a lock-free ring write, two atomics and `Thread::unpark`). Checked by `tests/no_alloc.rs` with `assert_no_alloc` and the C heap trap, with the worker busy. Whether the plugin's own `run` is real-time safe is the plugin's promise (`lv2:hardRTCapable`, in [`PluginInfo::optional_features`]). **Not real-time safe:** everything on [`World`] (Turtle files, the world lock), `instantiate` (dlopen, the plugin's `instantiate`), [`Instance::activate`] and [`Instance::deactivate`] (LV2 instantiation class), `save_state` and `restore_state` (allocate; restore blocks until the worker is idle), `Drop` (joins a thread, takes the world lock), the URID map (a mutex: plugins map their URIs at instantiation, as the spec asks), and `schedule_work` in synchronous mode (the work runs in place). lilv prints its own warnings to stderr (in the helper process). |
//! | Creates threads? | One per [`Instance`] whose plugin has a worker interface, in [`WorkerMode::Threaded`] (named `lv2-worker`): it parks until `schedule_work` wakes it. It is stopped and **joined in `Instance`'s `Drop` before `lilv_instance_free`**, after working every request still queued; its last responses are delivered, then the plugin is deactivated and freed. (The legacy program freed the instance first: lacuna 2 of plano-ciclo-de-vida.md.) A plugin whose `work` never returns therefore blocks the drop: that is a hung plugin, which the helper's deadline (I-9.6) kills with the process. [`WorkerMode::Synchronous`] creates none. |
//! | Calls back into our code? | Yes, from plugin code, through `extern "C"` trampolines that run under `catch_unwind` and return an error code if Rust panics: URID map and unmap (any thread), `schedule_work` (from `run`), the worker's `respond` (from `work`), the state's `store` and `retrieve` (from `save` and `restore`), state:mapPath and freePath (identity, `strdup`/`free`). Not provided: `log:log` (a C variadic function, which stable Rust cannot define): a plugin that requires it is refused with `lilv.missing_feature`, and one that only wants it simply gets none. |
//! | Tests | `tests/plugin/lm_test.c` is a small LV2 bundle of four plugins (a gain with a worker, a state, MIDI in and out, the options reported back, a latency port; one that requires an unknown feature; one whose `instantiate` fails; one with no state), compiled by the tests with the system's C compiler and the LV2 headers. `tests/discovery.rs` (the plugins installed on this machine; the test bundle's ports, features, ranges and latency port), `tests/instance.rs` (gain, options, MIDI round trip, latency, refused and failing plugins, block checks), `tests/worker.rs` (threaded and synchronous work and responses; the worker is joined before the instance is freed, checked by the plugin's own log of `work` and `cleanup`; drop with work queued), `tests/state.rs` (save → restore into a new instance → save gives the identical chunk; the plugin's restore schedules work; restore waits for running work and no older response lands after it; bad chunks; a plugin without state), `tests/installed.rs` (every installed plugin the host can satisfy, probed in a child process with a deadline as the scan helper does: instantiate, run, save, restore, drop), `tests/no_alloc.rs` (`run` and the control and event calls under `assert_no_alloc` and the C heap trap). ASan in CI. |

mod ffi;
mod instance;
mod state;
mod urid;
mod worker;
mod world;

pub use instance::{AtomEvent, AtomEvents, Instance, InstanceConfig};
pub use state::chunk_keys;
pub use urid::{UridMap, Urids, uri};
pub use worker::WorkerMode;
pub use world::{Direction, PluginInfo, PortInfo, PortKind, World};

/// The version of lilv this crate was built against, as pkg-config reported it ("0.28.0")
pub fn version() -> &'static str {
    env!("LM_SYS_LINKED_VERSION")
}
