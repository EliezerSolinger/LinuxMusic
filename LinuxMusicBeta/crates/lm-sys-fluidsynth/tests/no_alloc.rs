//! What the audio thread calls neither allocates nor frees: under `assert_no_alloc` (Rust's allocator) and the C heap
//! trap (FluidSynth's malloc). `harness = false`: measured on the main thread, with nothing else running

mod common;
use common::{heap, sf2};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_sys_fluidsynth::{SynthBuilder, SynthConfig};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn main() {
    heap::install();
    heap::self_test();
    let font = std::env::temp_dir().join(format!("lm-sys-fluidsynth-{}-noalloc.sf2", std::process::id()));
    sf2::standard(&font);
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
}
