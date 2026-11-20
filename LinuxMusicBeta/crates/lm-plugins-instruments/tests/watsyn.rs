//! Watsyn: the contract, and the A/B against the legacy note by composition (U34): `lm-dsp`'s voice is the legacy
//! `WatsynObject` (its A/B, waves oversampled by libsamplerate as `srccpy`); here, that the plugin drives it as the
//! legacy `WatsynInstrument::playNoteImpl` did: the drawn waves oversampled when they change, the knobs every period,
//! the modulations of the note's start, the frames played for the mix envelope, the release, velocity as level; and
//! the synced envelope times at the song's tempo

mod common;

lm_plugin::contract_tests!(lm_plugins_instruments::watsyn::DESCRIPTOR, "watsyn");

use common::{agree, render, state};
use lm_dsp::watsyn::{GRAPH_LENGTH, RELEASE_MS, WatsynMod, WatsynVoice};
use lm_plugin::testing::{note_off, note_on};
use lm_plugins_instruments::watsyn::{
    DESCRIPTOR, GRAPH_FIELDS, default_graph, default_values, param_key, settings, waves,
};
use lm_state::Value;

const RATE: u32 = 48_000;
const FRAMES: usize = 30_000;
const BPM: f64 = 120.0;

struct Patch<'a> {
    params: &'a [(&'a str, f64)],
    graphs: [[f32; GRAPH_LENGTH]; 4],
    mods: [WatsynMod; 2],
    sync: [Option<f64>; 3],
    anti_aliasing: bool,
    fields: Vec<(&'a str, Value)>,
}

impl Patch<'_> {
    fn compare(&self, key: u8, velocity: f32) {
        let mut values = default_values();
        for (name, v) in self.params {
            let i = (0..values.len()).position(|i| param_key(i) == *name).unwrap();
            values[i] = *v as f32;
        }
        let s = settings(&values, self.mods, &self.sync, BPM);
        let w = waves(&self.graphs, self.anti_aliasing).unwrap();
        let freq = 440.0 * libm::exp2f((f32::from(key) - 69.0) / 12.0);
        let mut voice = WatsynVoice::new(&s, RATE);
        let release = 20_000;
        let mut out = vec![[0.0f32; 2]; FRAMES];
        for (c, chunk) in out.chunks_mut(32).enumerate() {
            voice.render(chunk, &w, &s, freq, (c * 32) as u64);
        }
        // applyRelease: the last 1.5 ms after the note-off, then nothing
        let desired = (RELEASE_MS * RATE as f32 / 1000.0).ceil() as usize;
        for (i, f) in out.iter_mut().enumerate().skip(release) {
            let left = (release + desired) as i64 - i as i64;
            let fac = if left > 0 { left as f32 / desired as f32 } else { 0.0 };
            f[0] *= fac * velocity;
            f[1] *= fac * velocity;
        }
        for f in out.iter_mut().take(release) {
            f[0] *= velocity;
            f[1] *= velocity;
        }
        let events = [note_on(0, 1, key, velocity), note_off(release as u64, 1, key)];
        let ours = render(&DESCRIPTOR, &state(self.params, &self.fields), RATE, 3, FRAMES, &events);
        for ch in 0..2 {
            let theirs: Vec<f32> = out.iter().map(|f| f[ch]).collect();
            agree(&ours[ch], &theirs, RATE, 1.0e-5, "watsyn");
        }
        assert!(ours[0].iter().any(|v| *v != 0.0), "it sounds");
    }
}

fn graph_value(g: &[f32; GRAPH_LENGTH]) -> Value {
    Value::list(g.iter().map(|v| Value::real(f64::from(*v)).unwrap()))
}

#[test]
fn ab_against_the_legacy_note_at_the_defaults() {
    let p = Patch {
        params: &[],
        graphs: [default_graph(); 4],
        mods: [WatsynMod::Mix; 2],
        sync: [None; 3],
        anti_aliasing: true,
        fields: Vec::new(),
    };
    p.compare(69, 1.0);
    p.compare(45, 0.6);
}

#[test]
fn ab_against_the_legacy_note_with_drawn_waves_modulations_and_the_mix_envelope() {
    // A saw, a square, a triangle and a bent sine, as drawn
    let n = GRAPH_LENGTH as f32;
    let saw: [f32; GRAPH_LENGTH] = core::array::from_fn(|i| 2.0 * i as f32 / n - 1.0);
    let square: [f32; GRAPH_LENGTH] = core::array::from_fn(|i| if i < GRAPH_LENGTH / 3 { 0.9 } else { -0.9 });
    let tri: [f32; GRAPH_LENGTH] = core::array::from_fn(|i| 1.0 - 4.0 * ((i as f32 / n) - 0.5).abs());
    let bent: [f32; GRAPH_LENGTH] = core::array::from_fn(|i| (default_graph()[i] * 1.6).clamp(-1.0, 1.0));
    let graphs = [saw, square, tri, bent];
    let params = [
        ("a1Volume", 80.0),
        ("a2Volume", 60.0),
        ("b1Volume", 70.0),
        ("b2Volume", 50.0),
        ("a1Panning", -40.0),
        ("b2Panning", 60.0),
        ("a2Multiplier", 16.0),
        ("b1Multiplier", 12.0),
        ("a1LeftDetune", -12.0),
        ("b2RightDetune", 25.0),
        ("abMix", -20.0),
        ("envAmount", 150.0),
        ("envAttack", 40.0),
        ("envHold", 30.0),
        ("envDecay", 200.0),
        ("crosstalk", 35.0),
    ];
    for (mods, keys, aa) in
        [([WatsynMod::Pm, WatsynMod::Am], ("pm", "am"), true), ([WatsynMod::Rm, WatsynMod::Pm], ("rm", "pm"), false)]
    {
        let mut fields: Vec<(&str, Value)> =
            GRAPH_FIELDS.iter().zip(&graphs).map(|(k, g)| (*k, graph_value(g))).collect();
        fields.push(("aMod", Value::text(keys.0)));
        fields.push(("bMod", Value::text(keys.1)));
        fields.push(("antiAliasing", Value::bool(aa)));
        Patch { params: &params, graphs, mods, sync: [None; 3], anti_aliasing: aa, fields }.compare(60, 0.9);
    }
}

#[test]
fn synced_envelope_times_follow_the_tempo() {
    // The attack a 1/16 (125 ms at 120 BPM), the decay a 1/8 (250 ms)
    let params = [("abMix", -100.0), ("envAmount", 200.0), ("envAttack", 3.0), ("envDecay", 3.0)];
    let fields = vec![("envAttackSync", Value::text("1/16")), ("envDecaySync", Value::text("1/8"))];
    let p = Patch {
        params: &params,
        graphs: [default_graph(); 4],
        mods: [WatsynMod::Mix; 2],
        sync: [Some(0.25), None, Some(0.5)],
        anti_aliasing: true,
        fields,
    };
    p.compare(64, 1.0);
    let s = settings(&default_values(), [WatsynMod::Mix; 2], &p.sync, BPM);
    assert_eq!((s.env_attack, s.env_hold, s.env_decay), (125.0, 0.0, 250.0));
    assert_eq!(settings(&default_values(), [WatsynMod::Mix; 2], &[Some(8.0); 3], 60.0).env_attack, 2000.0);
}
