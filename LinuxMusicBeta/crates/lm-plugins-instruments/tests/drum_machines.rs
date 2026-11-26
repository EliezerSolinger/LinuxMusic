//! DrumMachines: the contract (on the keys the TR-808 plays), and the A/B against the legacy instrument by
//! composition (U34): `lm-dsp`'s `DrumMachine` is the legacy `DrumMachinesInstrument`'s `noteOn` and `render` (its
//! unit tests); here, that the plugin loads each kit as the legacy `loadKit` did (DrumSynth pieces rendered at the
//! engine's rate, recordings converted to it) and plays it that way: the piece of the note at the velocity, to its
//! end, the hi-hats choking one another.

mod common;

use lm_plugin::testing::Setup;

/// The keys the TR-808 has a piece on (the suite's keys land on them)
fn on_the_kit(key: u8) -> u8 {
    [36, 38, 42, 46, 45, 47, 50, 51][usize::from(key) % 8]
}

const SETUP: Setup = Setup { resources: None, playing: None, key_map: Some(on_the_kit) };

lm_plugin::contract_tests!(lm_plugins_instruments::drum_machines::DESCRIPTOR, "drummachines", setup = crate::SETUP);

mod ab {
    use super::*;
    use common::{agree, render, state};
    use lm_dsp::drum_machines::{DrumKit, DrumMachine, DrumSound};
    use lm_dsp::drumsynth;
    use lm_dsp::math::LegacyRand;
    use lm_plugin::testing::{note_on, use_setup};
    use lm_plugins_instruments::drum_machines::{DESCRIPTOR, KITS, Kit, samples_dir};
    use lm_state::Value;

    const RATE: u32 = 44_100;
    const FRAMES: usize = 30_000;

    /// The legacy `loadKit` of a kit made of DrumSynth pieces, written out here from the files
    fn legacy_kit(kit: &Kit, rate: u32) -> DrumKit {
        let dir = samples_dir().expect("the samples folder");
        let mut rand = LegacyRand::new(1);
        let sounds = kit
            .pieces
            .iter()
            .map(|p| {
                let text = std::fs::read(dir.join(kit.folder).join(p.file)).unwrap();
                let frames =
                    drumsynth::render(&text, rate, &mut rand).iter().map(|s| [f32::from(*s) / 32768.0; 2]).collect();
                DrumSound { frames, note: usize::from(p.note), choke: p.choke }
            })
            .collect();
        DrumKit::new(sounds)
    }

    fn kit_state(key: &str) -> Value {
        state(&[], &[("kit", Value::text(key))])
    }

    fn theirs(kit: &DrumKit, hits: &[(usize, i32, f32)]) -> Vec<[f32; 2]> {
        let mut machine = DrumMachine::new(RATE);
        let mut out = vec![[0.0f32; 2]; FRAMES];
        let mut done = 0;
        let mut hits = hits.to_vec();
        hits.sort_by_key(|h| h.0);
        for (at, note, velocity) in hits {
            machine.render(kit, &mut out[done..at]);
            done = at;
            machine.note_on(kit, note, velocity);
        }
        machine.render(kit, &mut out[done..]);
        out
    }

    fn compare(key: &str, hits: &[(usize, i32, f32)]) {
        use_setup(Setup::NONE);
        let kit = KITS.iter().find(|k| k.key == key).unwrap();
        if kit.folder.starts_with("drummachines/") {
            // (recordings: converted by libsamplerate, tested by the sound it makes below)
            return;
        }
        let legacy = legacy_kit(kit, RATE);
        let expected = theirs(&legacy, hits);
        let events: Vec<_> = hits
            .iter()
            .enumerate()
            .map(|(i, (at, note, v))| note_on(*at as u64, i as u32 + 1, *note as u8, *v))
            .collect();
        let ours = render(&DESCRIPTOR, &kit_state(key), RATE, 3, FRAMES, &events);
        for ch in 0..2 {
            let want: Vec<f32> = expected.iter().map(|f| f[ch]).collect();
            agree(&ours[ch], &want, RATE, 1.0e-6, key);
        }
        assert!(ours[0].iter().any(|v| *v != 0.0), "{key} sounds");
    }

    #[test]
    fn ab_the_tr808_pieces_at_their_velocities() {
        compare("tr808", &[(0, 36, 1.0)]);
        compare("tr808", &[(0, 38, 0.5), (4_000, 36, 0.8), (9_000, 51, 1.0)]);
        compare("tr808", &[(0, 56, 0.7), (100, 75, 0.7), (200, 82, 1.0)]);
    }

    #[test]
    fn ab_a_hi_hat_chokes_the_other() {
        compare("tr808", &[(0, 46, 1.0), (6_000, 42, 1.0)]);
        compare("tr909", &[(0, 46, 1.0), (3_000, 44, 0.9), (9_000, 42, 1.0)]);
    }

    #[test]
    fn ab_every_synthesized_kit() {
        for key in ["cr8000", "tr606", "tr77", "tr909", "linn", "electro", "rnb", "dmx", "tr707"] {
            let kit = KITS.iter().find(|k| k.key == key).unwrap();
            let notes: Vec<i32> = kit.pieces.iter().map(|p| i32::from(p.note)).collect();
            let hits: Vec<_> = notes.iter().take(6).enumerate().map(|(i, n)| (i * 3_000, *n, 0.9)).collect();
            compare(key, &hits);
        }
    }

    #[test]
    fn a_recorded_kit_sounds_and_a_note_without_a_piece_does_not() {
        use_setup(Setup::NONE);
        let hit = |key: u8| note_on(0, 1, key, 1.0);
        let ours = render(&DESCRIPTOR, &kit_state("sp1200"), RATE, 3, 20_000, &[hit(36)]);
        assert!(ours[0].iter().any(|v| *v != 0.0), "the kick sounds");
        let none = render(&DESCRIPTOR, &kit_state("sp1200"), RATE, 3, 20_000, &[hit(100)]);
        assert!(none[0].iter().all(|v| *v == 0.0), "a note with no piece is silent");
    }

    #[test]
    fn a_note_off_does_not_stop_a_drum() {
        use_setup(Setup::NONE);
        let held = render(&DESCRIPTOR, &kit_state("tr808"), RATE, 3, 20_000, &[note_on(0, 1, 36, 1.0)]);
        let cut = render(
            &DESCRIPTOR,
            &kit_state("tr808"),
            RATE,
            3,
            20_000,
            &[note_on(0, 1, 36, 1.0), lm_plugin::testing::note_off(500, 1, 36)],
        );
        assert_eq!(held, cut);
    }
}
