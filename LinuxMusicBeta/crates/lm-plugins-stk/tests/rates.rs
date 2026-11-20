//! The same sound at any rate (U36), for every preset: the plugin at 48, 96 and 192 kHz against itself at 44.1 kHz
//! (where it is the legacy's, `tests/ab.rs`), by the level in windows of 50 ms and the frequency of the fundamental
//! mode. The generated contract tests cannot do it with a period: the bars and bells are inharmonic, and only the
//! bowed waveguides last (see `TestHints` in `src/lib.rs`).

#![allow(clippy::print_stdout)] // (the measurements are the test's report)

use lm_plugin::testing::{Bench, mono, note_on, rms};
use lm_plugin::{BuildContext, Registry, ResourceView};
use lm_plugins_stk::DESCRIPTOR;
use lm_plugins_stk::mallets::{BANDED, PRESET_KEYS};
use lm_render::ProcessSpec;
use lm_state::Value;

fn state(preset: &str, bowed: bool) -> Value {
    let params = Value::empty_object().with("bowed", Value::real(f64::from(u8::from(bowed))).expect("real"));
    Value::empty_object().with("preset", Value::text(preset)).with("params", params)
}

fn render(state: &Value, rate: u32, key: u8, seconds: f64) -> Vec<f32> {
    let registry = Registry::new(&[&DESCRIPTOR]).expect("registry");
    let resources = ResourceView::new();
    let spec = ProcessSpec { sample_rate: rate, max_block: 1024 };
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let state = registry.complete("mallets", state).expect("complete");
    let p = registry.build("mallets", &state, &cx).expect("build");
    let frames = (seconds * f64::from(rate)) as usize;
    mono(&Bench::instrument(p, spec, 2, 77).render(frames, 256, &[note_on(0, 1, key, 0.5)], false))
}

/// The magnitude of the Hann-windowed DTFT of `x` at `hz`
fn dtft(x: &[f32], rate: f64, hz: f64) -> f64 {
    let n = x.len() as f64;
    let w = 2.0 * std::f64::consts::PI * hz / rate;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, v) in x.iter().enumerate() {
        let hann = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n).cos();
        let a = f64::from(*v) * hann;
        re += a * (w * i as f64).cos();
        im += a * (w * i as f64).sin();
    }
    (re * re + im * im).sqrt()
}

/// The strongest frequency of `x` within `width` (a fraction) of `near`
fn peak(x: &[f32], rate: f64, near: f64, width: f64) -> f64 {
    let bin = rate / x.len() as f64 / 4.0;
    let (lo, hi) = (near * (1.0 - width), near * (1.0 + width));
    let mut best = (lo, 0.0);
    let mut f = lo;
    while f <= hi {
        let m = dtft(x, rate, f);
        if m > best.1 {
            best = (f, m);
        }
        f += bin;
    }
    // Golden section around the best step
    let (mut a, mut b) = (best.0 - bin, best.0 + bin);
    let g = 0.618_033_988_75;
    for _ in 0..40 {
        let (c, d) = (b - g * (b - a), a + g * (b - a));
        if dtft(x, rate, c) > dtft(x, rate, d) {
            b = d;
        } else {
            a = c;
        }
    }
    0.5 * (a + b)
}

fn cents(a: f64, b: f64) -> f64 {
    1200.0 * (a / b).log2()
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1.0e-12).log10()
}

/// The level of `x` in windows of 50 ms over the first second, dB
fn envelope(x: &[f32], rate: u32) -> Vec<f64> {
    let w = rate as usize / 20;
    (0..20).map(|i| db(rms(&x[i * w..(i + 1) * w]))).collect()
}

#[test]
fn every_preset_sounds_the_same_at_any_rate() {
    let mut report = String::new();
    let mut failures = Vec::new();
    for (index, preset) in PRESET_KEYS.iter().enumerate() {
        for bowed in [true, false] {
            if !bowed && index < BANDED {
                continue;
            }
            for key in [57u8, 81] {
                let s = state(preset, bowed);
                let hz = 440.0 * 2f64.powf((f64::from(key) - 69.0) / 12.0);
                let reference = render(&s, 44_100, key, 1.0);
                let env_ref = envelope(&reference, 44_100);
                // (the Tibetan bowl's first two modes are 0.8 % apart, as loud: the first alone)
                let (near, width) = if *preset == "tibetan-bowl" { (hz * 0.996108344, 0.003) } else { (hz, 0.03) };
                let f_ref = peak(&reference[..22_050], 44_100.0, near, width);
                for rate in [48_000u32, 96_000, 192_000] {
                    let x = render(&s, rate, key, 1.0);
                    let env = envelope(&x, rate);
                    // (windows more than 60 dB under the loudest are left out: the tail of a struck bar is noise of
                    // rounding at -100 dB and below)
                    let top = env_ref.iter().copied().fold(f64::MIN, f64::max);
                    let level = env
                        .iter()
                        .zip(&env_ref)
                        .filter(|(_, r)| **r > top - 60.0)
                        .map(|(a, r)| (a - r).abs())
                        .fold(0.0, f64::max);
                    let f = peak(&x[..rate as usize / 2], f64::from(rate), near, width);
                    let off = cents(f, f_ref);
                    report +=
                        &format!("{preset} bowed {bowed} key {key} {rate}: level {level:.2} dB, pitch {off:+.2} c\n");
                    // The bowed waveguides: the bow's friction is a nonlinearity in the loop, whose harmonics alias at
                    // 44.1 kHz (in the legacy itself) and less above, and its velocity integrator's response at the
                    // high modes is 44.1 kHz's only near it. At 48 kHz within 0.3 dB; at 96 and 192 kHz, a high note
                    // (A5) up to 3 dB (measured: 2.97 dB, the uniform bar at 192 kHz) and 0.4 cent
                    let (max_level, max_cents) = if index >= BANDED && bowed { (3.5, 0.5) } else { (0.5, 0.25) };
                    if level > max_level || off.abs() > max_cents {
                        failures.push(format!(
                            "{preset} (bowed {bowed}) key {key} at {rate} Hz: {level:.2} dB, {off:.2} c"
                        ));
                    }
                }
            }
        }
    }
    println!("{report}");
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A bowed note held for 30 minutes keeps its pitch (U35; the generated test needs a period, which an inharmonic
/// sound has not): the fundamental mode at the end is the one at the start, within 0.1 cent
#[test]
fn a_bowed_note_keeps_its_pitch_for_thirty_minutes() {
    let registry = Registry::new(&[&DESCRIPTOR]).expect("registry");
    let resources = ResourceView::new();
    let rate = 48_000u32;
    let spec = ProcessSpec { sample_rate: rate, max_block: 1024 };
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let s = registry.complete("mallets", &state("tuned-bar", true)).expect("complete");
    let p = registry.build("mallets", &s, &cx).expect("build");
    let mut bench = Bench::instrument(p, spec, 2, 77);
    let total = 1800 * rate as usize;
    let window = rate as usize / 2;
    let starts = [rate as u64, (total - window) as u64];
    let mut windows: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
    bench.run(total, 1024, &[note_on(0, 1, 57, 0.5)], false, &mut |at, chans| {
        let n = chans[0].len() as u64;
        for (w, s) in starts.iter().enumerate() {
            if at + n > *s && at < s + window as u64 && windows[w].len() < window {
                let from = s.saturating_sub(at) as usize;
                let take = (window - windows[w].len()).min(n as usize - from);
                windows[w].extend((from..from + take).map(|k| 0.5 * (chans[0][k] + chans[1][k])));
            }
        }
    });
    let a = peak(&windows[0], f64::from(rate), 220.0, 0.03);
    let b = peak(&windows[1], f64::from(rate), 220.0, 0.03);
    let drift = cents(b, a);
    let level = db(rms(&windows[1])) - db(rms(&windows[0]));
    println!("after 30 min: {drift:+.4} c, {level:+.3} dB ({a:.4} Hz)");
    assert!(drift.abs() < 0.1, "after 30 minutes the pitch moved {drift:.4} cents");
    assert!(level.abs() < 0.5, "after 30 minutes the level moved {level:.3} dB");
}
