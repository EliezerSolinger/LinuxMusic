//! The generated test suite (arquitetura-nova.md, 7a: no plugin works again without passing it), with the feature
//! `testing`. A plugin crate runs every test with one line in its tests:
//!
//! ```ignore
//! lm_plugin::contract_tests!(my_plugin::DESCRIPTOR);
//! ```
//!
//! which installs the allocation trap's allocator for the test binary and makes one `#[test]` per check. It reads
//! `tests/data/defaults.json` (the frozen defaults, U29: written once with `LM_BLESS_DEFAULTS=1`) and
//! `tests/data/fixtures/v<N>-<name>.json` (states saved by older versions) from the plugin crate.

mod bench;
mod material;
mod suite;

pub use assert_no_alloc::{AllocDisabler, assert_no_alloc, permit_alloc};
pub use bench::{Bench, InputFn, Timed};
pub use material::*;
pub use suite::*;

/// One `#[test]` for each check of the suite, for the descriptor `$d` (a path to a `static Descriptor`), and the
/// allocation trap's allocator for the test binary (in debug builds: in release the trap does nothing)
#[macro_export]
macro_rules! contract_tests {
    ($d:path) => {
        #[cfg(debug_assertions)]
        #[global_allocator]
        static LM_PLUGIN_CONTRACT_ALLOCATOR: $crate::testing::AllocDisabler = $crate::testing::AllocDisabler;

        /// The plugin under test
        static LM_PLUGIN_UNDER_TEST: &$crate::Descriptor = &$d;

        mod contract {
            use $crate::testing as t;

            #[test]
            fn defaults_round_trip() {
                t::defaults_round_trip(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn random_states_round_trip() {
                t::random_states_round_trip(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn no_hidden_state() {
                t::no_hidden_state(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn adopt_while_playing() {
                t::adopt_while_playing(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn fixtures_upgrade() {
                t::fixtures_upgrade(
                    super::LM_PLUGIN_UNDER_TEST,
                    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/fixtures"),
                );
            }
            #[test]
            fn sample_rates() {
                t::sample_rates(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn param_sweep_without_allocation() {
                t::param_sweep(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn every_choice_without_allocation() {
                t::every_choice(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn short_notes() {
                t::short_notes(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn tuning_after_thirty_minutes() {
                t::long_tuning(super::LM_PLUGIN_UNDER_TEST, 1800);
            }
            #[test]
            fn block_sizes() {
                t::block_sizes(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn velocity() {
                t::velocity(super::LM_PLUGIN_UNDER_TEST);
            }
            #[test]
            fn frozen_defaults() {
                t::frozen_defaults(
                    super::LM_PLUGIN_UNDER_TEST,
                    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/defaults.json"),
                );
            }
            #[test]
            fn ports_follow_the_state() {
                t::ports_follow_the_state(super::LM_PLUGIN_UNDER_TEST);
            }
        }
    };
}
