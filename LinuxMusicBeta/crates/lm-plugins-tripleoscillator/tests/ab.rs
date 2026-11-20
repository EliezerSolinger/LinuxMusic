//! A/B of the plugin against the legacy TripleOscillator's note, by composition (U34): `lm-dsp`'s oscillator chain is
//! A/B-tested against the legacy `Oscillator` (`lm-dsp/ab`, bit for bit or below −90 dBFS), so what is left to check
//! here is that the plugin drives it as the legacy `TripleOscillator::playNoteImpl` did: the volume and pan law of
//! `updateVolume`, the detuning of `updateDetuning*` (`exp2((coarse·100 + fine) / 1200) / rate`, in double), the phase
//! offsets of `updatePhaseOffset*` (the stereo detuning on the left only), the chain order (oscillator 1 works with 2,
//! 2 with 3), and the instrument's velocity at level. The reference below is written from the legacy source, not
//! from the plugin; after the fade-in (at most 85 frames at 44.1 kHz) the two agree bit for bit.

use lm_dsp::oscillator::{ModulationAlgo, OscParams, Oscillator, UserWave, WaveShape, update};
use lm_plugin::testing::{Bench, note_on, test_loaded};
use lm_plugin::{BuildContext, Loaded, Registry, ResourceView};
use lm_plugins_tripleoscillator::DESCRIPTOR;
use lm_render::{ProcessSpec, Rng};
use lm_state::{Resource, Value};

const RATE: u32 = 48_000;
const FRAMES: usize = 4800;
const SEED: u64 = 77;

struct Knobs {
    vol: f32,
    pan: f32,
    coarse: f32,
    fine_l: f32,
    fine_r: f32,
    phase: f32,
    stereo: f32,
}

/// The legacy `playNoteImpl` for one note at A4, rendered the way the instrument renders (pieces of 32 frames)
fn legacy(knobs: &[Knobs; 3], shapes: [WaveShape; 3], algos: [ModulationAlgo; 2], wave: &UserWave) -> Vec<f32> {
    let rate = f64::from(RATE);
    let mut rng = Rng::new(SEED);
    let side = |k: &Knobs, left: bool| -> (f32, f64, f32) {
        // updateVolume
        let volume = if k.pan >= 0.0 {
            if left { (1.0 - k.pan / 100.0) * k.vol / 100.0 } else { k.vol / 100.0 }
        } else if left {
            k.vol / 100.0
        } else {
            (1.0 + k.pan / 100.0) * k.vol / 100.0
        };
        // updateDetuningLeft/Right
        let fine = if left { k.fine_l } else { k.fine_r };
        let detuning = libm::exp2((f64::from(k.coarse) * 100.0 + f64::from(fine)) / 1200.0) / rate;
        // updatePhaseOffsetLeft/Right
        let phase = if left { (k.phase + k.stereo) / 360.0 } else { k.phase / 360.0 };
        (volume, detuning, phase)
    };
    let mut left_oscs: Vec<Oscillator> = Vec::new();
    let mut right_oscs: Vec<Oscillator> = Vec::new();
    for k in knobs {
        left_oscs.push(Oscillator::new(side(k, true).2, RATE as f32, rng.next_u64()));
        right_oscs.push(Oscillator::new(side(k, false).2, RATE as f32, rng.next_u64()));
    }
    let params = |left: bool| -> Vec<OscParams<'_>> {
        (0..3)
            .map(|i| {
                let (volume, detuning, phase_offset) = side(&knobs[i], left);
                OscParams {
                    shape: shapes[i],
                    algo: if i < 2 { algos[i] } else { ModulationAlgo::SignalMix },
                    freq: 440.0,
                    detuning,
                    phase_offset,
                    volume,
                    use_wave_table: true,
                    user_wave: Some(wave),
                }
            })
            .collect()
    };
    let (pl, pr) = (params(true), params(false));
    let mut out = vec![0.0f32; FRAMES];
    let mut right = vec![0.0f32; FRAMES];
    for start in (0..FRAMES).step_by(32) {
        update(&mut left_oscs, &pl, &mut out[start..start + 32], false);
        update(&mut right_oscs, &pr, &mut right[start..start + 32], false);
    }
    out
}

#[test]
fn the_plugin_plays_the_legacy_note() {
    let registry = Registry::new(&[&DESCRIPTOR]).expect("registry");
    let resource = Resource::new("audio", &"ab".repeat(32)).expect("resource");
    let loaded = test_loaded(&resource, RATE);
    let Loaded::Audio(data) = &loaded else { unreachable!() };
    let wave = UserWave::new(data.channel(0).to_vec());
    let resources = ResourceView::new().with(resource.clone(), loaded.clone());
    let spec = ProcessSpec { sample_rate: RATE, max_block: 1024 };
    let knobs = [
        Knobs { vol: 80.0, pan: -30.0, coarse: 0.0, fine_l: 3.0, fine_r: -7.0, phase: 30.0, stereo: 90.0 },
        Knobs { vol: 55.0, pan: 60.0, coarse: -12.0, fine_l: -11.0, fine_r: 13.0, phase: 0.0, stereo: 45.0 },
        Knobs { vol: 120.0, pan: 0.0, coarse: 7.0, fine_l: 50.0, fine_r: -50.0, phase: 180.0, stereo: 0.0 },
    ];
    let mut cases = 0;
    for (n, &shape) in WaveShape::ALL.iter().enumerate() {
        for &algo in &ModulationAlgo::ALL {
            let shapes = [shape, WaveShape::ALL[(n + 3) % 8], WaveShape::ALL[(n + 5) % 8]];
            let algos = [algo, ModulationAlgo::ALL[(algo as usize + 2) % 5]];
            let mut params = Value::empty_object();
            for (i, k) in knobs.iter().enumerate() {
                let o = i + 1;
                for (key, v) in [
                    ("vol", k.vol),
                    ("pan", k.pan),
                    ("coarse", k.coarse),
                    ("fineL", k.fine_l),
                    ("fineR", k.fine_r),
                    ("phase", k.phase),
                    ("stereoPhase", k.stereo),
                ] {
                    params = params.with(&format!("{key}{o}"), Value::real(f64::from(v)).expect("real"));
                }
            }
            let mut state = Value::empty_object().with("params", params);
            for i in 0..3 {
                state = state
                    .with(&format!("wave{}", i + 1), Value::text(shapes[i].key()))
                    .with(&format!("userWave{}", i + 1), Value::resource(resource.clone()));
            }
            state =
                state.with("modulation1", Value::text(algos[0].key())).with("modulation2", Value::text(algos[1].key()));
            let cx = BuildContext { spec, resources: &resources, channels: 0 };
            let processor = registry.build("tripleoscillator", &state, &cx).expect("build");
            let out = Bench::instrument(processor, spec, 2, SEED).render(FRAMES, 256, &[note_on(0, 1, 69, 1.0)], false);
            let reference = legacy(&knobs, shapes, algos, &wave);
            for i in 128..FRAMES {
                assert_eq!(
                    out[0][i].to_bits(),
                    reference[i].to_bits(),
                    "{shapes:?} {algos:?}: sample {i}: {} and {}",
                    out[0][i],
                    reference[i]
                );
            }
            cases += 1;
        }
    }
    assert_eq!(cases, 40);
}
