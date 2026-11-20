//! The plugin contract (arquitetura-nova.md, 7a) for every unit: the checks of `lm_plugin::contract_tests!`, each
//! run over all the units. (The macro makes one test binary per descriptor; 97 binaries, each linking Guitarix's
//! 5 MB of DSP, would not fit the disk: the same functions, the same allocation trap, one binary.)

use lm_plugin::testing as t;
use lm_plugins_guitarix::ALL;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: t::AllocDisabler = t::AllocDisabler;

fn data(id: &str) -> String {
    format!("{}/tests/data/{id}", env!("CARGO_MANIFEST_DIR"))
}

/// Runs `check` on every unit (only those named in `LM_GX_UNITS`, comma separated, when it is set)
fn each(check: impl Fn(&'static lm_plugin::Descriptor)) {
    let only = std::env::var("LM_GX_UNITS").ok();
    for (unit, d) in ALL {
        if let Some(only) = &only
            && !only.split(',').any(|u| u == *unit || u == d.id)
        {
            continue;
        }
        check(d);
    }
}

#[test]
fn defaults_round_trip() {
    each(t::defaults_round_trip);
}
#[test]
fn random_states_round_trip() {
    each(t::random_states_round_trip);
}
#[test]
fn no_hidden_state() {
    each(t::no_hidden_state);
}
#[test]
fn adopt_while_playing() {
    each(t::adopt_while_playing);
}
#[test]
fn fixtures_upgrade() {
    each(|d| t::fixtures_upgrade(d, &format!("{}/fixtures", data(d.id))));
}
/// (The Rangemaster and the Screaming Bird boost the top octave so hard that their level follows the aliasing of
/// this check's naive saw, which folds differently at each rate: tests/rates.rs checks them, and every unit, with a
/// band-limited input)
const ALIASING_FOLLOWERS: [&str; 2] = ["gx-rangem", "gx-scream"];

#[test]
fn sample_rates() {
    each(|d| {
        if !ALIASING_FOLLOWERS.contains(&d.id) {
            t::sample_rates(d);
        }
    });
}
#[test]
fn param_sweep_without_allocation() {
    each(t::param_sweep);
}
#[test]
fn every_choice_without_allocation() {
    each(t::every_choice);
}
#[test]
fn short_notes() {
    each(t::short_notes);
}
#[test]
fn block_sizes() {
    each(t::block_sizes);
}
#[test]
fn velocity() {
    each(t::velocity);
}
#[test]
fn frozen_defaults() {
    each(|d| t::frozen_defaults(d, &format!("{}/defaults.json", data(d.id))));
}
#[test]
fn ports_follow_the_state() {
    each(t::ports_follow_the_state);
}
