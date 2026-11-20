//! Every unit keeps its level at 44.1, 48, 96 and 192 kHz with a band-limited input (U36).
//!
//! The contract's own check feeds a naive saw, whose harmonics above half the rate fold back into the band, and fold
//! differently at each rate. Two units boost the top octave hard (the Rangemaster's treble booster, the Screaming
//! Bird) and their level then follows the test signal's aliasing, not the unit (at 96 kHz the saw's real harmonics
//! above 24 kHz are not there for a unit that runs at 48 kHz, where at 48 kHz their aliases were): they are checked
//! here instead, with a saw made of its harmonics below 18 kHz, the same at every rate; and so is every other unit.

use std::cell::RefCell;

use lm_plugin::testing::{Bench, db, mono, rms};
use lm_plugin::{BuildContext, Registry};
use lm_plugins_guitarix::ALL;
use lm_render::ProcessSpec;

const RATES: [u32; 4] = [44_100, 48_000, 96_000, 192_000];
/// The saw's fundamental: a whole number of samples per period at every rate
const HZ: f64 = 100.0;

thread_local! {
    static PERIOD: RefCell<(u32, Vec<f32>)> = const { RefCell::new((0, Vec::new())) };
}

/// A band-limited saw at 100 Hz (harmonics below 18 kHz) and a sine at 300 Hz, the same on both channels
fn input(frame: u64, _ch: usize, rate: u32) -> f32 {
    PERIOD.with(|p| {
        let mut p = p.borrow_mut();
        if p.0 != rate {
            let n = (f64::from(rate) / HZ) as usize;
            let table = (0..n)
                .map(|i| {
                    let t = i as f64 / f64::from(rate);
                    let mut saw = 0.0;
                    let mut k = 1.0;
                    while k * HZ < 18_000.0 {
                        saw += libm::sin(std::f64::consts::TAU * k * HZ * t) / k;
                        k += 1.0;
                    }
                    (0.3 * (2.0 / std::f64::consts::PI) * saw + 0.2 * libm::sin(std::f64::consts::TAU * 3.0 * HZ * t))
                        as f32
                })
                .collect();
            *p = (rate, table);
        }
        p.1[frame as usize % p.1.len()]
    })
}

fn level(d: &'static lm_plugin::Descriptor, rate: u32) -> f64 {
    let registry = Registry::new(&[d]).expect("registry");
    let state = registry.defaults(d.id).expect("defaults");
    let resources = lm_plugin::ResourceView::new();
    let spec = ProcessSpec { sample_rate: rate, max_block: 256 };
    let cx = BuildContext { spec, resources: &resources, channels: 2 };
    let p = registry.build(d.id, &state, &cx).expect("build");
    let total = (1.4 * f64::from(rate)) as usize;
    let out = Bench::effect(p, spec, 2, 0, input).render(total, 256, &[], false);
    let x = mono(&out);
    rms(&x[(0.3 * f64::from(rate)) as usize..(1.3 * f64::from(rate)) as usize])
}

#[test]
fn every_unit_keeps_its_level_at_every_rate() {
    let mut failures = Vec::new();
    for (_, d) in ALL {
        let reference = level(d, 48_000);
        for rate in RATES {
            let diff = db(level(d, rate) / reference);
            if diff.abs() >= 0.5 {
                failures.push(format!("{}: {diff:.3} dB at {rate} Hz", d.id));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
