//! PatMan: the contract (on a patch of the `lm-dsp` A/B), and the A/B against the legacy note by composition (U34):
//! `lm-dsp`'s PatMan is the legacy one (its A/B, bit for bit, on these patches); here, that the plugin plays it as
//! the legacy `PatmanInstrument::playNoteImpl` did: the sample nearest the note, tuned or not, looped or not, the
//! release, velocity as level; and a file that is not a patch fails the build with the legacy's reason

mod common;

use lm_plugin::testing::Setup;
use lm_state::{Resource, Value};

const KEY: &str = "7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b7b";

fn playing() -> Value {
    Value::empty_object().with("patch", Value::resource(Resource::new("pat", KEY).unwrap()))
}

const SETUP: Setup = Setup { resources: None, playing: Some(playing), key_map: None };

lm_plugin::contract_tests!(lm_plugins_instruments::patman::DESCRIPTOR, "patman", setup = crate::SETUP);

mod ab {
    use std::sync::Arc;

    use super::*;
    use common::{agree, render, state};
    use lm_dsp::patman::{PatmanVoice, RELEASE_MS};
    use lm_dsp::sample_player::Interpolator;
    use lm_plugin::testing::{note_off, note_on, use_setup};
    use lm_plugin::Loaded;
    use lm_plugins_instruments::patman::{DESCRIPTOR, read_patch};
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

    fn patch(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../lm-dsp/ab/data").join(name)).unwrap()
    }

    thread_local! {
        static PATCH: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn the_patch(resource: &Resource, _: u32) -> Option<Loaded> {
        (resource.key() == KEY).then(|| Loaded::Bytes(Arc::from(PATCH.with(|p| p.borrow().clone()))))
    }

    fn compare(name: &str, key: u8, looped: bool, tuned: bool, velocity: f32) {
        PATCH.with(|p| *p.borrow_mut() = patch(name));
        use_setup(Setup { resources: Some(the_patch), playing: None, key_map: None });
        let samples = read_patch(&patch(name)).unwrap();
        let freq = 440.0 * libm::exp2f((f32::from(key) - 69.0) / 12.0);
        let mut voice = PatmanVoice::new(&samples, freq, tuned, Src(Resampler::new(Quality::Linear, 2, 1.0).unwrap()));
        let release = 30_000;
        let mut out = vec![[0.0f32; 2]; FRAMES];
        let mut playing = true;
        for chunk in out.chunks_mut(32) {
            if playing {
                let n = chunk.len();
                playing = voice.render(chunk, &samples, freq, looped, RATE as f32, n);
            } else {
                chunk.fill([0.0; 2]);
            }
        }
        let desired = (RELEASE_MS * RATE as f32 / 1000.0).ceil() as usize;
        for (i, f) in out.iter_mut().enumerate() {
            let fac = if i < release {
                1.0
            } else {
                let left = (release + desired) as i64 - i as i64;
                if left > 0 { left as f32 / desired as f32 } else { 0.0 }
            };
            f[0] *= fac * velocity;
            f[1] *= fac * velocity;
        }
        let s = state(
            &[("looped", if looped { 1.0 } else { 0.0 }), ("tuned", if tuned { 1.0 } else { 0.0 })],
            &[("patch", Value::resource(Resource::new("pat", KEY).unwrap()))],
        );
        let events = [note_on(0, 1, key, velocity), note_off(release as u64, 1, key)];
        let ours = render(&DESCRIPTOR, &s, RATE, 5, FRAMES, &events);
        for ch in 0..2 {
            let theirs: Vec<f32> = out.iter().map(|f| f[ch]).collect();
            agree(&ours[ch], &theirs, RATE, 1.0e-6, name);
        }
        assert!(ours[0].iter().any(|v| *v != 0.0), "it sounds");
    }

    #[test]
    fn ab_against_the_legacy_note() {
        compare("mixed.pat", 60, true, true, 1.0);
        compare("mixed.pat", 72, false, true, 0.7);
        compare("mixed.pat", 48, true, false, 1.0);
        compare("eight-bit.pat", 64, true, true, 0.9);
        compare("eight-bit.pat", 55, false, false, 1.0);
        // Three samples: each note plays the nearest
        for key in [36, 60, 84] {
            compare("three-layers.pat", key, true, true, 1.0);
        }
    }

    #[test]
    fn a_file_that_is_not_a_patch_fails_the_build_with_the_reason() {
        assert_eq!(read_patch(b"not a patch at all, really").unwrap_err().key(), "patman.not_gus");
        let cut = &patch("mixed.pat")[..400];
        assert_eq!(read_patch(cut).unwrap_err().key(), "patman.truncated");
    }
}
