//! Kicker: the contract, and the A/B against the legacy note by composition (U34): `lm-dsp`'s sweep is A/B-tested
//! against the legacy `SweepOsc` (`lm-dsp/ab`, kicker-osc cases); here, that the plugin drives it as the legacy
//! `KickerInstrument::playNoteImpl` did

lm_plugin::contract_tests!(lm_plugins_instruments::kicker::DESCRIPTOR, "kicker");

use lm_dsp::kicker::{KickerVoice, RELEASE_MS, release_fade};
use lm_plugin::testing::{Bench, note_off, note_on, test_resources};
use lm_plugin::{BuildContext, Registry};
use lm_plugins_instruments::kicker::{DESCRIPTOR, settings};
use lm_render::{ProcessSpec, Rng};
use lm_state::Value;

const RATE: u32 = 44_100;
const SEED: u64 = 9;

fn render(params: &[(&str, f64)], frames: usize, release_at: Option<u64>) -> Vec<Vec<f32>> {
    let mut p = Value::empty_object();
    for (k, v) in params {
        p = p.with(k, Value::real(*v).unwrap());
    }
    render_state(&Value::empty_object().with(lm_state::PARAMS_KEY, p), RATE, frames, release_at)
}

fn render_state(state: &Value, rate: u32, frames: usize, release_at: Option<u64>) -> Vec<Vec<f32>> {
    let registry = Registry::new(&[&DESCRIPTOR]).unwrap();
    let state = state.clone();
    let complete = registry.complete("kicker", &state).unwrap();
    let spec = ProcessSpec { sample_rate: rate, max_block: 1024 };
    let resources = test_resources(registry.schemas(), "plugin:kicker", &complete, rate);
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let mut bench = Bench::instrument(registry.build("kicker", &state, &cx).unwrap(), spec, 2, SEED);
    let mut events = vec![note_on(0, 1, 45, 1.0)];
    if let Some(at) = release_at {
        events.push(note_off(at, 1, 45));
    }
    bench.render(frames, 256, &events, false)
}

/// The legacy note: the sweep from the note's pitch (A2, 110 Hz), let go after its length with a 12 ms fade
fn legacy(params: &[(&str, f64)], frames: usize) -> Vec<f32> {
    let defaults = [150.0, 40.0, 440.0, 0.8, 0.8, 1.0, 0.163, 0.0, 0.4, 0.06, 1.0, 1.0, 0.0];
    let keys = [
        "startFrequency",
        "endFrequency",
        "length",
        "startDistortion",
        "endDistortion",
        "gain",
        "envelopeSlope",
        "noise",
        "click",
        "frequencySlope",
        "attack",
        "startFromNote",
        "endToNote",
    ];
    let mut k = defaults;
    for (key, v) in params {
        k[keys.iter().position(|x| x == key).unwrap()] = *v as f32;
    }
    let s = settings(&k);
    let mut rng = Rng::new(SEED);
    let mut voice = KickerVoice::new(&s, 110.0, RATE as f32, rng.next_u64());
    let mut out = vec![[0.0f32; 2]; frames];
    voice.render(&mut out, RATE as f32);
    let length = voice.length() as usize;
    release_fade(&mut out, length, 0, RELEASE_MS * RATE as f32 / 1000.0);
    out.iter().map(|f| f[0]).collect()
}

fn compare(params: &[(&str, f64)]) {
    let frames = 30_000;
    let ours = render(params, frames, None);
    let theirs = legacy(params, frames);
    // After the fade-in (at most 85 frames at 44.1 kHz)
    for (n, (a, b)) in ours[0].iter().zip(&theirs).enumerate().skip(85) {
        assert!((a - b).abs() <= 1.0e-6, "{params:?}: frame {n}: {a} and {b}");
    }
    assert_eq!(ours[0], ours[1], "a mono note on both sides");
}

#[test]
fn ab_against_the_legacy_note_at_the_defaults() {
    compare(&[]);
}

#[test]
fn ab_against_the_legacy_note_with_noise_and_a_moving_distortion() {
    compare(&[("noise", 0.4), ("startDistortion", 30.0), ("endDistortion", 2.0), ("length", 200.0), ("attack", 5.0)]);
}

#[test]
fn ab_against_the_legacy_note_swept_between_fixed_frequencies() {
    compare(&[("startFromNote", 0.0), ("startFrequency", 400.0), ("endFrequency", 30.0), ("frequencySlope", 0.3)]);
}

#[test]
fn the_note_lets_itself_go_and_frees_its_voice() {
    let x = render(&[("length", 100.0)], 44_100, None);
    let end = (0.1 * 44_100.0 + RELEASE_MS * 44.1) as usize + 1;
    assert!(x[0][end..].iter().all(|v| *v == 0.0), "silent after its length and the release");
}

/// A2 (110 Hz) swept from the note to the note: the same pitch at every rate (U36), within a cent, over 3 s
#[test]
fn the_pitch_is_the_same_at_every_rate() {
    for rate in [44_100, 48_000, 96_000, 192_000] {
        let x = render_state(&lm_plugins_instruments::kicker::probe_state(), rate, 3 * rate as usize, None);
        let ups: Vec<usize> = (1..x[0].len()).filter(|&i| x[0][i - 1] < 0.0 && x[0][i] >= 0.0).collect();
        let (a, b) = (ups[1], ups[ups.len() - 1]);
        let hz = (ups.len() - 2) as f64 * f64::from(rate) / (b - a) as f64;
        let cents = 1200.0 * (hz / 110.0).log2();
        assert!(cents.abs() < 1.0, "{rate} Hz: {hz} Hz, {cents:.3} cents");
    }
}
