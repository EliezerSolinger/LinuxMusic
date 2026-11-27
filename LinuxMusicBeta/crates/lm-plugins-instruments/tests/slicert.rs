//! SlicerT: the contract (on a sample of its own cut in four), and the A/B against the legacy note by composition
//! (U34): `lm-dsp`'s `SlicerVoice` is the legacy `SlicerT::playNoteImpl` (its A/B, with libsamplerate); here, that
//! the plugin plays it as the legacy did: the slice of the key above the base note (the whole sample at the base
//! note, nothing past the last slice), looped or not, the fade out, velocity as level; and the analysis of a sample
//! is `lm-dsp`'s

mod common;

use std::sync::Arc;

use lm_plugin::Loaded;
use lm_plugin::testing::Setup;
use lm_render::SampleData;
use lm_state::{Resource, Value};

const KEY: &str = "6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c";
const POINTS: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// The sample: 0.625 s of decaying partials, different on each side, at the test's rate
fn sample_channels(rate: u32) -> [Vec<f32>; 2] {
    let r = rate as f32;
    let frames = rate as usize * 5 / 8;
    let side = |f1: f32, f2: f32| -> Vec<f32> {
        (0..frames)
            .map(|i| {
                let t = i as f32 / r;
                let env = (-t * 2.0).exp();
                env * (0.6 * (std::f32::consts::TAU * f1 * t).sin() + 0.3 * (std::f32::consts::TAU * f2 * t).sin())
            })
            .collect()
    };
    [side(220.0, 661.0), side(330.0, 997.0)]
}

fn test_resource(resource: &Resource, rate: u32) -> Option<Loaded> {
    (resource.key() == KEY).then(|| {
        let [l, r] = sample_channels(rate);
        Loaded::Audio(Arc::new(SampleData::from_channels(rate, vec![l, r]).unwrap()))
    })
}

fn slices() -> Value {
    Value::list(POINTS.iter().map(|p| Value::real(*p).unwrap()))
}

fn playing() -> Value {
    Value::empty_object().with("sample", Value::resource(Resource::new("audio", KEY).unwrap())).with("slices", slices())
}

/// The suite's keys land on the base note and the three slices
fn on_the_slices(key: u8) -> u8 {
    69 + key % 4
}

const SETUP: Setup = Setup { resources: Some(test_resource), playing: Some(playing), key_map: Some(on_the_slices) };

lm_plugin::contract_tests!(lm_plugins_instruments::slicert::DESCRIPTOR, "slicert", setup = crate::SETUP);

mod ab {
    use super::*;
    use common::{agree, render, state};
    use lm_dsp::sample_player::{Interpolator, Loop};
    use lm_dsp::slicert::{SlicerSample, SlicerSettings, SlicerVoice, release_ms, slice_for};
    use lm_plugin::instrument::GRID;
    use lm_plugin::testing::{note_off, note_on, use_setup};
    use lm_plugins_instruments::audio::Audio;
    use lm_plugins_instruments::slicert::{BASE_KEY, DESCRIPTOR, analyze};
    use lm_sys_samplerate::{Quality, Resampler};

    const RATE: u32 = 48_000;
    const FRAMES: usize = 40_000;

    struct Src(Resampler);
    impl Interpolator for Src {
        fn process(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]], ratio: f64) -> (usize, usize) {
            self.0.set_ratio(ratio).unwrap();
            let done = self.0.process(input.as_flattened(), output.as_flattened_mut()).unwrap();
            (done.input_frames, done.output_frames)
        }
        fn reset(&mut self) {
            self.0.reset().unwrap();
        }
    }

    fn frames(rate: u32) -> Vec<[f32; 2]> {
        let [l, r] = sample_channels(rate);
        l.iter().zip(&r).map(|(l, r)| [*l, *r]).collect()
    }

    fn compare(key: u8, looping: Loop, fade_out: f32, velocity: f32) {
        use_setup(Setup { resources: Some(test_resource), playing: None, key_map: None });
        let sample_frames = frames(RATE);
        let points: Vec<f32> = POINTS.iter().map(|p| *p as f32).collect();
        let settings = SlicerSettings { fade_out, original_bpm: 1, sync: false, looping };
        let release = 20_000usize;
        let mut out = vec![[0.0f32; 2]; FRAMES];
        if let Some(slice) = slice_for(&points, i32::from(key) - BASE_KEY) {
            let sample = SlicerSample { frames: &sample_frames, sample_rate: RATE };
            let mut voice =
                SlicerVoice::new(slice, sample_frames.len(), Src(Resampler::new(Quality::Linear, 2, 1.0).unwrap()));
            let mut playing = true;
            for chunk in out.chunks_mut(GRID) {
                if playing {
                    let n = chunk.len();
                    playing = voice.render(chunk, &sample, &settings, 120.0, 0.0, RATE as f32, n);
                } else {
                    chunk.fill([0.0; 2]);
                }
            }
        }
        // (looped, the note is let go and fades out over the fade out; not looped, it plays to its end)
        let desired = (release_ms(&settings) * RATE as f32 / 1000.0).ceil() as usize;
        for (i, f) in out.iter_mut().enumerate() {
            let fac = if looping == Loop::Off || i < release {
                1.0
            } else {
                let left = (release + desired) as i64 - i as i64;
                if left > 0 { left as f32 / desired as f32 } else { 0.0 }
            };
            f[0] *= fac * velocity;
            f[1] *= fac * velocity;
        }
        let loop_key = match looping {
            Loop::Off => "off",
            Loop::On => "on",
            Loop::PingPong => "pingPong",
        };
        let s = state(
            &[("fadeOut", f64::from(fade_out))],
            &[
                ("sample", Value::resource(Resource::new("audio", KEY).unwrap())),
                ("slices", slices()),
                ("loop", Value::text(loop_key)),
            ],
        );
        let mut events = vec![note_on(0, 1, key, velocity)];
        if looping != Loop::Off {
            events.push(note_off(release as u64, 1, key));
        }
        let ours = render(&DESCRIPTOR, &s, RATE, 5, FRAMES, &events);
        for ch in 0..2 {
            let theirs: Vec<f32> = out.iter().map(|f| f[ch]).collect();
            agree(&ours[ch], &theirs, RATE, 1.0e-6, &format!("key {key} {loop_key}"));
        }
        if slice_for(&points, i32::from(key) - BASE_KEY).is_some() {
            assert!(ours[0].iter().any(|v| *v != 0.0), "key {key} sounds");
        } else {
            assert!(ours[0].iter().all(|v| *v == 0.0), "key {key} has no slice");
        }
    }

    #[test]
    fn ab_the_base_note_plays_the_whole_sample() {
        compare(69, Loop::Off, 10.0, 1.0);
        compare(69, Loop::Off, 0.0, 0.6);
    }

    #[test]
    fn ab_each_key_above_plays_a_slice() {
        for key in [70, 71, 72] {
            compare(key, Loop::Off, 10.0, 0.9);
        }
        compare(71, Loop::Off, 50.0, 1.0);
    }

    #[test]
    fn ab_a_key_past_the_last_slice_or_below_the_base_plays_nothing() {
        compare(73, Loop::Off, 10.0, 1.0);
        compare(60, Loop::Off, 10.0, 1.0);
    }

    #[test]
    fn ab_looped_slices_go_on_while_held_and_fade_out_after() {
        compare(70, Loop::On, 10.0, 1.0);
        compare(72, Loop::PingPong, 30.0, 0.8);
        compare(69, Loop::On, 100.0, 1.0);
    }

    #[test]
    fn the_analysis_is_the_dsp_s() {
        let audio = Audio { frames: frames(RATE), rate: RATE as f32 };
        let a = analyze(&audio, 0.6, 0);
        assert_eq!(a.bpm, lm_dsp::slicert::find_bpm(&audio.frames, RATE));
        assert_eq!(a.slices, lm_dsp::slicert::find_slices(&audio.frames, RATE, 0.6, a.bpm, 0));
        assert!(a.slices.iter().all(|p| (0.0..=1.0).contains(p)));
    }
}
