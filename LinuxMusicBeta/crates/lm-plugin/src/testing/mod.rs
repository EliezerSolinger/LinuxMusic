//! The generated test suite (arquitetura-nova.md, 7a: no plugin works again without passing it), with the feature
//! `testing`. A plugin crate runs every test with one line in its tests:
//!
//! ```ignore
//! lm_plugin::contract_tests!(my_plugin::DESCRIPTOR);
//! ```
//!
//! which installs the allocation trap's allocator for the test binary and makes one `#[test]` per check. It reads
//! `tests/data/defaults.json` (the frozen defaults, U29: written once with `LM_BLESS_DEFAULTS=1`) and
//! `tests/data/fixtures/v<N>-<name>.json` (states saved by older versions) from the plugin crate. A crate of several
//! plugins gives each its own folder under `tests/data`: `lm_plugin::contract_tests!(my_plugins::a::DESCRIPTOR,
//! "a")` reads `tests/data/a/defaults.json` and `tests/data/a/fixtures`.

mod bench;
mod material;
mod suite;

pub use assert_no_alloc::{AllocDisabler, assert_no_alloc, permit_alloc};
pub use bench::{Bench, InputFn, Timed};
pub use material::*;
pub use suite::*;

use std::cell::Cell;

use lm_state::{Resource, Value};

use crate::Loaded;

/// What a plugin's tests give the suite besides the descriptor (see the module's documentation)
#[derive(Clone, Copy, Debug, Default)]
pub struct Setup {
    /// A test resource of a kind the suite does not make itself (`None`: the suite's own, see [`test_loaded`])
    pub resources: Option<fn(&Resource, u32) -> Option<Loaded>>,
    /// The state played where the suite would play the defaults (it is completed with the defaults)
    pub playing: Option<fn() -> Value>,
    /// Every key the suite plays, mapped (an instrument that sounds on some keys only, as a drum kit: the suite's
    /// probes and scripts land on keys it plays)
    pub key_map: Option<fn(u8) -> u8>,
}

impl Setup {
    pub const NONE: Setup = Setup { resources: None, playing: None, key_map: None };
}

thread_local! {
    /// The setup of the test running on this thread (each generated test sets it first; the suite runs on the test's
    /// thread)
    static SETUP: Cell<Setup> = const { Cell::new(Setup::NONE) };
}

/// Sets the setup for the suite's functions called next on this thread
pub fn use_setup(setup: Setup) {
    SETUP.with(|s| s.set(setup));
}

/// The setup of this thread
pub fn current_setup() -> Setup {
    SETUP.with(Cell::get)
}

/// One `#[test]` for each check of the suite, for the descriptor `$d` (a path to a `static Descriptor`), and the
/// allocation trap's allocator for the test binary (in debug builds: in release the trap does nothing)
#[macro_export]
macro_rules! contract_tests {
    (@with $d:path, $data:expr, $setup:expr) => {
        #[cfg(debug_assertions)]
        #[global_allocator]
        static LM_PLUGIN_CONTRACT_ALLOCATOR: $crate::testing::AllocDisabler = $crate::testing::AllocDisabler;

        /// The plugin under test
        static LM_PLUGIN_UNDER_TEST: &$crate::Descriptor = &$d;
        /// What its tests give the suite
        const LM_PLUGIN_SETUP: $crate::testing::Setup = $setup;

        mod contract {
            use $crate::testing as t;

            #[test]
            fn defaults_round_trip() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::defaults_round_trip(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn random_states_round_trip() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::random_states_round_trip(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn no_hidden_state() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::no_hidden_state(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn adopt_while_playing() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::adopt_while_playing(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn fixtures_upgrade() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::fixtures_upgrade(
                    super::LM_PLUGIN_UNDER_TEST,
                    &[env!("CARGO_MANIFEST_DIR"), $data, "/fixtures"].concat(),
                );
            }
            #[test]
            fn sample_rates() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::sample_rates(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn param_sweep_without_allocation() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::param_sweep(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn every_choice_without_allocation() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::every_choice(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn short_notes() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::short_notes(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn tuning_after_thirty_minutes() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::long_tuning(super::LM_PLUGIN_UNDER_TEST, 1800);
            }
            #[test]
            fn block_sizes() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::block_sizes(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn velocity() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::velocity(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn frozen_defaults() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::frozen_defaults(
                    super::LM_PLUGIN_UNDER_TEST,
                    &[env!("CARGO_MANIFEST_DIR"), $data, "/defaults.json"].concat(),
                );
            }
            #[test]
            fn ports_follow_the_state() {
                t::use_setup(super::LM_PLUGIN_SETUP);
                t::ports_follow_the_state(super::LM_PLUGIN_UNDER_TEST);
            }
        }
    };
    ($d:path) => {
        $crate::contract_tests!(@with $d, "/tests/data", $crate::testing::Setup::NONE);
    };
    ($d:path, "", setup = $setup:expr) => {
        $crate::contract_tests!(@with $d, "/tests/data", $setup);
    };
    ($d:path, $data:literal) => {
        $crate::contract_tests!(@with $d, concat!("/tests/data/", $data), $crate::testing::Setup::NONE);
    };
    ($d:path, $data:literal, setup = $setup:expr) => {
        $crate::contract_tests!(@with $d, concat!("/tests/data/", $data), $setup);
    };
}
