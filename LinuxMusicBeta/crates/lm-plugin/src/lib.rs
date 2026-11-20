//! LinuxMusic: the plugin contract (interfaces.md, I-9; arquitetura-nova.md, pieces 7 and 7a).
//!
//! - [`Descriptor`]: what a plugin is — its id, its schema (`"plugin:<id>"`: fields, parameters, version, upgrades,
//!   parameter aliases), its ports for a state, what it does with velocity (U28), and its build;
//! - [`Registry`]: every plugin, immutable after start-up inside an `Arc`; gives `lm-doc` its `PortsFn` and the
//!   schemas, and builds a processor from a state ([`Registry::build`]);
//! - [`ResourceView`]: the loaded resources a build reads (C11);
//! - [`StateView`]: reading a complete state in a build.
//!
//! A plugin's processor is an `lm_render::Processor`: built in an `Operation` with everything it will ever need
//! (U31), it only reacts to events on the audio thread, never allocates there, and takes over its predecessor's
//! sound in `adopt` (U32).

#![forbid(unsafe_code)]

mod descriptor;
mod registry;
mod resources;
mod state;

pub use descriptor::{
    BuildContext, BuildFn, DEFAULT_TAIL, Descriptor, DisplaySpec, PluginKind, StatePortsFn, TestHints, TuningProbe,
    Velocity, is_valid_plugin_id, plugin_type,
};
pub use lm_doc::{Port, Ports};
pub use registry::{CHUNK_KEY, EntryInfo, External, ExternalBuild, Registry, check_ports};
pub use resources::{Loaded, ResourceView};
pub use state::StateView;
pub mod instrument;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

// The unit tests run under the allocation trap's allocator (the instrument functions' tests check the audio path)
#[cfg(all(test, debug_assertions))]
#[global_allocator]
static TEST_ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;
