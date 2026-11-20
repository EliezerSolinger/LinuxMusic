//! Monstro: the contract, and the A/B against the legacy note by composition (U34): `lm-dsp`'s voice is the legacy
//! `MonstroSynth` (its A/B); here, that the plugin drives it as the legacy `MonstroInstrument::playNoteImpl` did: the
//! knobs every period, the frames played, the release from the period the note is let go in (its own envelopes, no
//! host release), the host's fade-in, velocity as level; and the tempo-synced knobs at the song's tempo

mod common;

lm_plugin::contract_tests!(lm_plugins_instruments::monstro::DESCRIPTOR, "monstro");

use common::{agree, render, state};
use lm_dsp::monstro::MonstroVoice;
use lm_plugin::testing::{note_off, note_on};
use lm_plugins_instruments::monstro::{
    DESCRIPTOR, LFO_WAVE_KEYS, Menus, O23_KEYS, OSC_WAVE_KEYS, default_values, param_key, settings,
};
use lm_render::Rng;
use lm_state::Value;

const RATE: u32 = 48_000;
const SEED: u64 = 41;
const FRAMES: usize = 48_000;
/// The bench's tempo
const BPM: f64 = 120.0;

/// The legacy note: `release` frames in, let go (a multiple of the grid)
fn legacy(params: &[(&str, f64)], menus: &Menus, key: u8, velocity: f32, release: usize) -> [Vec<f32>; 2] {
    let mut values = default_values();
    for (name, v) in params {
        let i = (0..values.len()).position(|i| param_key(i) == *name).unwrap();
        values[i] = *v as f32;
    }
    let s = settings(&values, menus, BPM);
    let freq = 440.0 * libm::exp2f((f32::from(key) - 69.0) / 12.0);
    let mut rng = Rng::new(SEED);
    let mut voice = MonstroVoice::new(&s, RATE, rng.next_u64());
    let mut out = vec![[0.0f32; 2]; FRAMES];
    for (c, chunk) in out.chunks_mut(32).enumerate() {
        let at = c * 32;
        voice.set(&s);
        voice.render(chunk, freq, at as i64, (at >= release).then_some(0));
    }
    // The voice is kept for its longer release, then freed
    let end = release + (voice.release_ms() * RATE as f32 / 1000.0).ceil() as usize;
    for f in out.iter_mut().skip(end) {
        *f = [0.0; 2];
    }
    [out.iter().map(|f| f[0] * velocity).collect(), out.iter().map(|f| f[1] * velocity).collect()]
}

fn compare(params: &[(&str, f64)], fields: &[(&str, Value)], menus: &Menus, key: u8, velocity: f32) {
    let release = 32_000;
    let theirs = legacy(params, menus, key, velocity, release);
    let events = [note_on(0, 1, key, velocity), note_off(release as u64, 1, key)];
    let ours = render(&DESCRIPTOR, &state(params, fields), RATE, SEED, FRAMES, &events);
    agree(&ours[0], &theirs[0], RATE, 1.0e-5, "left");
    agree(&ours[1], &theirs[1], RATE, 1.0e-5, "right");
    assert!(ours[0].iter().any(|v| *v != 0.0), "it sounds");
}

#[test]
fn ab_against_the_legacy_note_at_the_defaults() {
    compare(&[], &[], &Menus::default(), 69, 1.0);
}

#[test]
fn ab_against_the_legacy_note_with_the_matrix_waves_and_envelopes() {
    let params = [
        ("osc1Volume", 60.0),
        ("osc1Panning", -30.0),
        ("osc1FineLeft", -7.0),
        ("osc1FineRight", 9.0),
        ("osc1StereoPhase", 40.0),
        ("osc1PulseWidth", 30.0),
        ("osc1SyncRise", 1.0),
        ("osc2Volume", 50.0),
        ("osc2Coarse", 7.0),
        ("osc2SyncHard", 1.0),
        ("osc3Volume", 45.0),
        ("osc3Coarse", -12.0),
        ("osc3Sub", 40.0),
        ("osc3SyncReverse", 1.0),
        ("lfo1Rate", 180.0),
        ("lfo1Attack", 50.0),
        ("lfo2Phase", 90.0),
        ("env1Attack", 30.0),
        ("env1Hold", 20.0),
        ("env1Decay", 200.0),
        ("env1Sustain", 0.4),
        ("env1Release", 150.0),
        ("env1Slope", 0.5),
        ("env2PreDelay", 10.0),
        ("env2Release", 300.0),
        ("osc1VolEnv1", 0.8),
        ("osc2PhsLfo1", 0.3),
        ("osc3PitLfo2", -0.2),
        ("osc1PwEnv2", 0.5),
        ("osc3SubLfo1", 0.6),
    ];
    let fields = [
        ("osc2Wave", Value::text("moogSaw")),
        ("osc3Wave1", Value::text("digitalSquare")),
        ("osc3Wave2", Value::text("absSine")),
        ("lfo1Wave", Value::text("triangle")),
        ("lfo2Wave", Value::text("randomSmooth")),
        ("o23Mod", Value::text("fm")),
    ];
    let pos = |keys: &[&str], k: &str| keys.iter().position(|w| *w == k).unwrap();
    let menus = Menus {
        osc2_wave: pos(&OSC_WAVE_KEYS, "moogSaw"),
        osc3_waves: [pos(&OSC_WAVE_KEYS, "digitalSquare"), pos(&OSC_WAVE_KEYS, "absSine")],
        lfo_waves: [pos(&LFO_WAVE_KEYS, "triangle"), pos(&LFO_WAVE_KEYS, "randomSmooth")],
        o23: lm_dsp::monstro::O23Mod::ALL[pos(&O23_KEYS, "fm")],
        ..Menus::default()
    };
    compare(&params, &fields, &menus, 57, 0.8);
}

#[test]
fn synced_knobs_follow_the_tempo() {
    // The LFO's period a 1/8 (250 ms at 120 BPM), the release a 1/4 (500 ms)
    let params = [("osc1VolLfo1", 1.0), ("env1Release", 10.0)];
    let fields = [("lfo1RateSync", Value::text("1/8")), ("env1ReleaseSync", Value::text("1/4"))];
    let mut menus = Menus::default();
    menus.sync[1] = Some(0.5);
    menus.sync[8] = Some(1.0);
    compare(&params, &fields, &menus, 60, 1.0);
    let mut values = default_values();
    values[25] = 1.0;
    let s = settings(&values, &menus, BPM);
    assert_eq!(s.lfos[0].rate, 250.0);
    assert_eq!(s.envelopes[0].release, 500.0);
    // At most the knob's maximum (2/1 at 30 BPM is 16 s; the release goes to 4 s)
    menus.sync[8] = Some(8.0);
    assert_eq!(settings(&values, &menus, 30.0).envelopes[0].release, 4000.0);
}
