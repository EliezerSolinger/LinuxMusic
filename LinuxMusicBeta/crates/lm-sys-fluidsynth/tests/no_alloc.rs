//! What the audio thread calls neither allocates nor frees: under `assert_no_alloc` (Rust's allocator) and the C heap
//! trap (FluidSynth's malloc). `harness = false`: measured on the main thread, with nothing else running

mod common;
use common::{heap, sf2};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_sys_fluidsynth::{Chorus, Generator, Reverb, SynthBuilder, SynthConfig, VoiceRef};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn main() {
    heap::install();
    heap::self_test();
    let font = std::env::temp_dir().join(format!("lm-sys-fluidsynth-{}-noalloc.sf2", std::process::id()));
    sf2::standard(&font).unwrap();
    let config = SynthConfig { outputs: 9, polyphony: 64, ..SynthConfig::default() };
    let mut b = SynthBuilder::new(config).unwrap();
    let id = b.load_soundfont(&font).unwrap();
    std::fs::remove_file(&font).unwrap();
    for channel in 0..16 {
        b.program_select(channel, id, 0, (channel % 2) as u32).unwrap();
    }
    let mut synth = b.build();
    let mut buffers = vec![[0.0_f32; 64]; 18];
    // Warm up: the first render may set up lazily
    {
        let mut refs: Vec<&mut [f32]> = buffers.iter_mut().map(|b| b.as_mut_slice()).collect();
        synth.render(&mut refs).unwrap();
        let mut refs: Vec<&mut [f32]> = buffers.iter_mut().map(|b| b.as_mut_slice()).collect();
        let events = heap::measure(|| {
            assert_no_alloc(|| {
                for block in 0..3_000_usize {
                    let channel = block % 16;
                    let key = 30 + (block * 7 % 70) as u8;
                    // More notes than voices: stealing must not allocate either
                    let _ = synth.note_on(channel, key, 100);
                    if block % 3 == 0 {
                        let _ = synth.note_off(channel, key);
                    }
                    let _ = synth.cc(channel, 7, (block % 128) as u8);
                    let _ = synth.cc(channel, 10, 64);
                    let _ = synth.pitch_bend(channel, (block * 37 % 16_384) as u16);
                    if block % 100 == 0 {
                        let _ = synth.program_change(channel, 1);
                        let _ = synth.bank_select(channel, 0);
                        let _ = synth.program_select(channel, id, 0, 0);
                        let _ = synth.all_notes_off(Some(channel));
                    }
                    if block % 997 == 0 {
                        let _ = synth.all_sounds_off(None);
                    }
                    synth.render(&mut refs).unwrap();
                }
                // Rejected events make an Error: that must not allocate either
                assert!(synth.note_on(200, 60, 100).is_err());
            });
        });
        assert_eq!(events, 0, "the C heap was touched {events} times");
    }
    drop(synth);

    // The stereo instance of the Sf2 Player: effects, gain, voices retuned while they play
    let mut b = SynthBuilder::new(SynthConfig { polyphony: 64, ..SynthConfig::default() }).unwrap();
    let font = std::env::temp_dir().join(format!("lm-sys-fluidsynth-{}-noalloc2.sf2", std::process::id()));
    sf2::standard(&font).unwrap();
    let id = b.load_soundfont(&font).unwrap();
    std::fs::remove_file(&font).unwrap();
    b.program_select(1, id, 0, 0).unwrap();
    let mut synth = b.build();
    let (mut l, mut r) = (vec![0.0_f32; 100], vec![0.0_f32; 100]);
    synth.render_stereo(&mut l, &mut r).unwrap();
    let mut voices = [VoiceRef::default(); 4];
    let events = heap::measure(|| {
        assert_no_alloc(|| {
            for block in 0..3_000_usize {
                let key = 30 + (block * 7 % 70) as u8;
                let _ = synth.note_on(1, key, 100);
                let n = synth.voices_of(1, key, &mut voices);
                for v in &voices[..n] {
                    let c = synth.voice_gen(v, Generator::CoarseTune).unwrap_or(0.0);
                    synth.set_voice_gen(v, Generator::CoarseTune, c + 0.25);
                    synth.set_voice_gen(v, Generator::VolEnvAttack, -1200.0);
                    synth.set_voice_gen(v, Generator::VolEnvSustain, 60.0);
                }
                if block % 3 == 0 {
                    let _ = synth.note_off(1, key);
                }
                if block % 50 == 0 {
                    synth.set_gain((block % 7) as f32 * 0.3);
                    let on = block % 100 == 0;
                    let _ = synth.set_reverb_on(on);
                    let _ = synth.set_chorus_on(!on);
                    let _ = synth.set_reverb(Reverb { room_size: 0.5, damping: 0.3, width: 0.8, level: 0.7 });
                    let _ =
                        synth.set_chorus(Chorus { voices: (block % 11) as u32, level: 1.5, speed: 1.0, depth: 20.0 });
                    let _ = synth.set_bend_range(1, (block % 24) as u8);
                }
                synth.render_stereo(&mut l, &mut r).unwrap();
            }
        });
    });
    assert_eq!(events, 0, "the stereo instance touched the C heap {events} times");
    drop(synth);
}
