//! AudioFileProcessor: the contract (on a sample of its own, looped), libsamplerate's zero-order-hold and linear
//! converters against their port, and the A/B against the legacy note by composition (U34): `lm-dsp`'s player is the
//! legacy `Sample::play` (its A/B, with libsamplerate); here, that the plugin plays it as the legacy
//! `AudioFileProcessor::playNoteImpl` did: the points from the knobs, the ratio from the note's pitch, the converter
//! of the interpolation chosen, stutter and its magic key, velocity as level

mod common;

use std::sync::Arc;

use lm_plugin::Loaded;
use lm_plugin::testing::Setup;
use lm_render::SampleData;
use lm_state::{Resource, Value};

const KEY: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

/// The sample: 0.625 s of two decaying partials apart on each side, at the test's rate
fn sample_channels(rate: u32) -> [Vec<f32>; 2] {
    let r = rate as f32;
    let frames = rate as usize * 5 / 8;
    let side = |f1: f32, f2: f32| -> Vec<f32> {
        (0..frames)
            .map(|i| {
                let t = i as f32 / r;
                let env = (-t * 3.0).exp();
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

fn playing() -> Value {
    Value::empty_object()
        .with("sample", Value::resource(Resource::new("audio", KEY).unwrap()))
        .with("loop", Value::text("on"))
}

const SETUP: Setup = Setup { resources: Some(test_resource), playing: Some(playing), key_map: None };

lm_plugin::contract_tests!(
    lm_plugins_instruments::audio_file_processor::DESCRIPTOR,
    "audiofileprocessor",
    setup = crate::SETUP
);

mod port {
    //! The port against the library, chunk by chunk, with the ratio gliding and jumping
    use lm_sys_samplerate::{Quality, Resampler};

    fn compare(quality: Quality, kind: lm_plugins_instruments::src_port::Kind) {
        let mut ours = lm_plugins_instruments::src_port::Converter::new(kind);
        let mut theirs = Resampler::new(quality, 2, 1.0).unwrap();
        let input: Vec<[f32; 2]> =
            (0..20_000).map(|i| [((i as f32) * 0.0137).sin(), ((i as f32) * 0.0291).cos() * 0.5]).collect();
        let ratios = [1.0, 0.7311, 0.7311, 1.9, 1.9, 0.25, 3.3, 1.000_000_1, 1.0, 0.5];
        let mut at = 0usize;
        for step in 0..400 {
            let ratio = ratios[step % ratios.len()];
            // (three frames at least: see the port's note on one or two)
            let n = (17 + step * 7) % 300 + 3;
            let input = &input[at.min(input.len())..(at + n).min(input.len())];
            let room = (step * 13) % 400 + 1;
            let mut a = vec![[0.0f32; 2]; room];
            let mut b = vec![[0.0f32; 2]; room];
            let (used_a, made_a) = ours.process(input, &mut a, ratio);
            theirs.set_ratio(ratio).unwrap();
            let done = theirs.process(input.as_flattened(), b.as_flattened_mut()).unwrap();
            assert_eq!((used_a, made_a), (done.input_frames, done.output_frames), "{quality:?} step {step}");
            assert_eq!(a[..made_a], b[..made_a], "{quality:?} step {step}");
            at += used_a;
            if step % 97 == 96 {
                ours.reset();
                theirs.reset().unwrap();
            }
        }
    }

    #[test]
    fn zero_order_hold_is_the_library_s() {
        compare(Quality::ZeroOrderHold, lm_plugins_instruments::src_port::Kind::ZeroOrderHold);
    }

    #[test]
    fn linear_is_the_library_s() {
        compare(Quality::Linear, lm_plugins_instruments::src_port::Kind::Linear);
    }
}

mod ab {
    use super::*;
    use common::{agree, render, state};
    use lm_dsp::sample_player::{DEFAULT_BASE_FREQ, Interpolator, Loop, PlaybackState, Sample};
    use lm_plugin::testing::{Timed, note_off, note_on, use_setup};
    use lm_plugins_instruments::audio_file_processor::{DESCRIPTOR, RELEASE_MS, bounds};
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

    struct Patch {
        amp: f32,
        points: [f32; 3],
        reverse: bool,
        declick: bool,
        looping: Loop,
        quality: Quality,
    }

    impl Patch {
        fn state(&self) -> Value {
            let loop_key = match self.looping {
                Loop::Off => "off",
                Loop::On => "on",
                Loop::PingPong => "pingPong",
            };
            let interpolation = match self.quality {
                Quality::ZeroOrderHold => "none",
                Quality::Linear => "linear",
                _ => "sinc",
            };
            let d = |v: f32| f64::from(v);
            state(
                &[
                    ("amplify", d(self.amp)),
                    ("startPoint", d(self.points[0])),
                    ("endPoint", d(self.points[1])),
                    ("loopPoint", d(self.points[2])),
                    ("reverse", if self.reverse { 1.0 } else { 0.0 }),
                    ("declick", if self.declick { 1.0 } else { 0.0 }),
                ],
                &[
                    ("sample", Value::resource(Resource::new("audio", KEY).unwrap())),
                    ("loop", Value::text(loop_key)),
                    ("interpolation", Value::text(interpolation)),
                ],
            )
        }

        /// The legacy note at `key`, let go at `release` (a multiple of the grid): its frames before the host's
        /// release ramp and fade-in
        fn legacy(&self, key: u8, release: usize) -> [Vec<f32>; 2] {
            let [l, r] = sample_channels(RATE);
            let frames: Vec<[f32; 2]> = l.iter().zip(&r).map(|(a, b)| [*a, *b]).collect();
            let [s, e, lp] = self.points;
            let sample = Sample {
                frames: &frames,
                sample_rate: RATE as f32,
                bounds: bounds(frames.len(), s, e, lp),
                amplification: self.amp / 100.0,
                frequency: DEFAULT_BASE_FREQ,
                reversed: self.reverse,
                declick: self.declick,
            };
            let freq = 440.0 * libm::exp2f((f32::from(key) - 69.0) / 12.0);
            let mut state = PlaybackState::new(Src(Resampler::new(self.quality, 2, 1.0).unwrap()), sample.bounds.start);
            let mut out = vec![[0.0f32; 2]; FRAMES];
            let mut playing = true;
            for chunk in out.chunks_mut(32) {
                if playing {
                    let n = chunk.len();
                    playing = sample.play(
                        chunk,
                        &mut state,
                        self.looping,
                        f64::from(DEFAULT_BASE_FREQ / freq),
                        RATE as f32,
                        n,
                    );
                }
                if !playing {
                    chunk.fill([0.0; 2]);
                }
            }
            // The release (applyRelease): the last 3 ms faded, then nothing
            let desired = (RELEASE_MS * RATE as f32 / 1000.0).ceil() as usize;
            for (i, f) in out.iter_mut().enumerate().skip(release) {
                let left = (release + desired) as i64 - i as i64;
                let fac = if left > 0 { left as f32 / desired as f32 } else { 0.0 };
                f[0] *= fac;
                f[1] *= fac;
            }
            [out.iter().map(|f| f[0]).collect(), out.iter().map(|f| f[1]).collect()]
        }

        fn compare(&self, key: u8) {
            use_setup(SETUP);
            let release = 32_000;
            let theirs = self.legacy(key, release);
            let events: [Timed; 2] = [note_on(0, 1, key, 1.0), note_off(release as u64, 1, key)];
            let ours = render(&DESCRIPTOR, &self.state(), RATE, 7, FRAMES, &events);
            agree(&ours[0], &theirs[0], RATE, 1.0e-6, "left");
            agree(&ours[1], &theirs[1], RATE, 1.0e-6, "right");
            assert!(ours[0].iter().any(|v| *v != 0.0), "it sounds");
        }
    }

    const PLAIN: Patch = Patch {
        amp: 100.0,
        points: [0.0, 1.0, 0.0],
        reverse: false,
        declick: true,
        looping: Loop::Off,
        quality: Quality::Linear,
    };

    #[test]
    fn ab_as_recorded_and_transposed() {
        PLAIN.compare(69);
        PLAIN.compare(76);
        PLAIN.compare(57);
    }

    #[test]
    fn ab_with_every_interpolation() {
        Patch { quality: Quality::ZeroOrderHold, ..PLAIN }.compare(62);
        Patch { quality: Quality::Medium, ..PLAIN }.compare(62);
    }

    #[test]
    fn ab_looped_reversed_with_points() {
        let looped = Patch { amp: 180.0, points: [0.1, 0.6, 0.3], looping: Loop::On, declick: true, ..PLAIN };
        looped.compare(64);
        Patch { looping: Loop::PingPong, reverse: true, declick: false, ..looped }.compare(71);
        // The points crossed: in order anyway
        Patch { points: [0.8, 0.2, 0.9], looping: Loop::On, ..PLAIN }.compare(69);
    }

    #[test]
    fn stutter_goes_on_where_the_last_note_stopped() {
        use_setup(SETUP);
        let s = state(
            &[("stutter", 1.0), ("declick", 0.0)],
            &[("sample", Value::resource(Resource::new("audio", KEY).unwrap()))],
        );
        // Two notes as recorded: the second goes on in the sample from where the first stopped
        let events = [note_on(0, 1, 69, 1.0), note_off(4000, 1, 69), note_on(8000, 2, 69, 1.0)];
        let ours = render(&DESCRIPTOR, &s, RATE, 7, 12_000, &events);
        let [l, _] = sample_channels(RATE);
        let desired = (RELEASE_MS * RATE as f32 / 1000.0).ceil() as usize;
        // The first note went on through its release; the second starts where its reading stopped, as the legacy
        // (the player reads the sample 256 frames at a time ahead of what it plays)
        let from =
            (4000 + desired).div_ceil(lm_dsp::sample_player::BUFFER_FRAMES) * lm_dsp::sample_player::BUFFER_FRAMES;
        for i in 200..3000 {
            assert!((ours[0][8000 + i] - l[from + i]).abs() < 1.0e-6, "frame {i}");
        }
        // The magic key (below 20 Hz) sends the next note back to the start and plays nothing
        let events = [
            note_on(0, 1, 69, 1.0),
            note_off(4000, 1, 69),
            note_on(5000, 2, 0, 1.0),
            note_off(5100, 2, 0),
            note_on(8000, 3, 69, 1.0),
        ];
        let ours = render(&DESCRIPTOR, &s, RATE, 7, 12_000, &events);
        assert!(ours[0][5000..7000].iter().all(|v| *v == 0.0), "the magic key is silent");
        for i in 200..3000 {
            assert!((ours[0][8000 + i] - l[i]).abs() < 1.0e-6, "frame {i}");
        }
    }
}
