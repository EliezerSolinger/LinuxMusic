//! The offline stretch against the real Rubber Band library

mod common;
use common::signal::{channel, frequency, rms, sine};

use lm_base::{Cancel, Progress};
use lm_sys_rubberband::{
    Engine, Formants, Format, Offline, PITCH_SCALE_RANGE, Settings, TIME_RATIO_RANGE, Transients, frames_for_ratio,
    stretch, version,
};

const RATE: u32 = 48_000;

fn run(input: &[f32], channels: usize, output_frames: usize, pitch: f64, settings: &Settings) -> Vec<f32> {
    stretch(input, Format::new(RATE, channels), output_frames, pitch, settings, &Cancel::new(), &Progress::new())
        .unwrap()
}

/// R3 with its defaults; R2 with smooth transients, the setting for sustained material (a sine): with crisp transients
/// R2's onset detector misfires on a steady tone and the pitch wobbles (documented on `Transients`)
fn settings(engine: Engine) -> Settings {
    let transients = if engine == Engine::Faster { Transients::Smooth } else { Transients::Crisp };
    Settings { engine, transients, ..Settings::default() }
}

#[test]
fn the_library_is_linked_and_new_enough() {
    let major: u32 = version().split('.').next().unwrap().parse().unwrap();
    assert!(major >= 3, "{}", version());
    let s = Offline::new(Format::new(RATE, 1), 100, 100, 1.0, &settings(Engine::Finer)).unwrap();
    assert_eq!(s.engine_version(), 3, "the finer engine is R3");
    let s = Offline::new(Format::new(RATE, 1), 100, 100, 1.0, &settings(Engine::Faster)).unwrap();
    assert_eq!(s.engine_version(), 2, "the faster engine is R2");
}

#[test]
fn a_stretched_sine_keeps_its_pitch_and_gets_exactly_the_length_asked() {
    for engine in [Engine::Finer, Engine::Faster] {
        for channels in [1, 2] {
            // 4x only on R3: R2 loses about 10 dB on a sine at that ratio (documented on `Engine::Faster`)
            let cases: &[(usize, usize)] = match engine {
                Engine::Finer => &[(48_000, 72_000), (48_000, 30_011), (44_101, 44_101), (24_000, 96_000)],
                Engine::Faster => &[(48_000, 72_000), (48_000, 30_011), (44_101, 44_101), (32_000, 60_000)],
            };
            for &(input_frames, output_frames) in cases {
                let input = sine(input_frames, channels, 440.0, f64::from(RATE), 0.5);
                let out = run(&input, channels, output_frames, 1.0, &settings(engine));
                assert_eq!(
                    out.len(),
                    output_frames * channels,
                    "{engine:?} {channels}ch {input_frames}->{output_frames}"
                );
                // Away from the edges, every channel still plays 440 Hz at its level
                let (from, to) = (output_frames / 5, output_frames * 4 / 5);
                for c in 0..channels {
                    let x = channel(&out, channels, c);
                    let f = frequency(&x, f64::from(RATE), from, to);
                    assert!(
                        (f / 440.0 - 1.0).abs() < 0.005,
                        "{engine:?} ch{c} {input_frames}->{output_frames}: {f} Hz"
                    );
                    let want = if c == 0 { 0.5 } else { 0.25 } / 2.0_f64.sqrt();
                    let level = rms(&x, from, to);
                    // R3 keeps the level within 0.5 dB; R2 within 1.5 dB (it dips about 1 dB near 2x)
                    let tolerance_db = if engine == Engine::Finer { 0.5 } else { 1.5 };
                    assert!(
                        (20.0 * (level / want).log10()).abs() < tolerance_db,
                        "{engine:?} {channels}ch ch{c} {input_frames}->{output_frames}: rms {level}, want {want}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_library_makes_almost_exactly_the_length_on_its_own() {
    // The trim or the padding of the exact length is a frame or so, never a gap of silence
    for engine in [Engine::Finer, Engine::Faster] {
        let (input_frames, output_frames) = (48_000, 61_234);
        let input = sine(input_frames, 2, 300.0, f64::from(RATE), 0.5);
        let mut s = Offline::new(Format::new(RATE, 2), input_frames, output_frames, 1.0, &settings(engine)).unwrap();
        s.study(&input).unwrap();
        let mut out = Vec::new();
        s.process(&input, &mut out).unwrap();
        s.finish(&mut out).unwrap();
        assert_eq!(out.len(), output_frames * 2);
        let made = s.library_frames();
        assert!(made.abs_diff(output_frames) <= 64, "{engine:?}: the library made {made} for {output_frames}");
        let tail = channel(&out, 2, 0);
        assert!(rms(&tail, output_frames - 2000, output_frames - 200) > 0.2, "{engine:?}: the tail is silent");
    }
}

#[test]
fn a_pitch_shift_moves_the_frequency_by_the_scale() {
    for engine in [Engine::Finer, Engine::Faster] {
        for formants in [Formants::Shifted, Formants::Preserved] {
            for (pitch, length) in [(2.0, 48_000), (0.5, 48_000), (1.5, 48_000), (2.0_f64.powf(-3.0 / 12.0), 60_000)] {
                let input = sine(48_000, 1, 440.0, f64::from(RATE), 0.5);
                let s = Settings { formants, ..settings(engine) };
                let out = run(&input, 1, length, pitch, &s);
                assert_eq!(out.len(), length);
                let f = frequency(&out, f64::from(RATE), length / 5, length * 4 / 5);
                assert!(
                    (f / (440.0 * pitch) - 1.0).abs() < 0.005,
                    "{engine:?} {formants:?} x{pitch}: {f} Hz, want {}",
                    440.0 * pitch
                );
            }
        }
    }
}

#[test]
fn feeding_in_chunks_equals_one_call() {
    let input = sine(30_000, 2, 523.25, f64::from(RATE), 0.4);
    let whole = run(&input, 2, 45_000, 1.0, &Settings::default());
    let mut s = Offline::new(Format::new(RATE, 2), 30_000, 45_000, 1.0, &Settings::default()).unwrap();
    for chunk in input.chunks(2 * 1000 + 2 * 7) {
        s.study(chunk).unwrap();
    }
    let mut out = Vec::new();
    for chunk in input.chunks(2 * 333) {
        s.process(chunk, &mut out).unwrap();
        assert!(out.len() <= 45_000 * 2);
    }
    s.finish(&mut out).unwrap();
    assert_eq!(out, whole, "the chunking does not change the result");
}

#[test]
fn an_empty_item_stays_empty() {
    let out = run(&[], 2, 0, 1.0, &Settings::default());
    assert!(out.is_empty());
    let mut s = Offline::new(Format::new(RATE, 1), 0, 0, 1.0, &Settings::default()).unwrap();
    let mut out = Vec::new();
    s.finish(&mut out).unwrap();
    assert!(out.is_empty());
}

#[test]
fn cancel_and_progress() {
    let input = sine(200_000, 2, 440.0, f64::from(RATE), 0.5);
    let format = Format::new(RATE, 2);

    let cancel = Cancel::new();
    cancel.cancel();
    let err = stretch(&input, format, 300_000, 1.0, &Settings::default(), &cancel, &Progress::new()).unwrap_err();
    assert!(err.is_cancelled());

    // Cancelled while it runs: a thread waits for the progress to move and cancels
    let cancel = Cancel::new();
    let progress = Progress::new();
    let err = std::thread::scope(|scope| {
        let (c, p) = (cancel.clone(), progress.clone());
        scope.spawn(move || {
            while p.fraction().unwrap_or(0.0) < 0.3 {
                std::thread::yield_now();
            }
            c.cancel();
        });
        stretch(&input, format, 300_000, 1.0, &Settings::default(), &cancel, &progress).unwrap_err()
    });
    assert!(err.is_cancelled());
    let reached = progress.fraction().unwrap();
    assert!(reached < 1.0, "cancelled at {reached}");

    let progress = Progress::new();
    stretch(&input, format, 300_000, 1.0, &settings(Engine::Faster), &Cancel::new(), &progress).unwrap();
    assert_eq!(progress.fraction(), Some(1.0));
}

#[test]
fn the_passes_must_come_in_order() {
    let input = vec![0.0_f32; 2000];
    let mut s = Offline::new(Format::new(RATE, 2), 1000, 1500, 1.0, &Settings::default()).unwrap();
    let mut out = Vec::new();
    assert_eq!(s.process(&input, &mut out).unwrap_err().key(), "rubberband.out_of_order");
    assert_eq!(s.finish(&mut out).unwrap_err().key(), "rubberband.out_of_order");
    s.study(&input[..1000]).unwrap();
    assert_eq!(s.process(&input, &mut out).unwrap_err().key(), "rubberband.out_of_order", "half studied");
    assert_eq!(s.study(&input).unwrap_err().key(), "rubberband.too_much_input");
    assert_eq!(s.study(&input[..3]).unwrap_err().key(), "rubberband.bad_length");
    s.study(&input[1000..]).unwrap();
    assert_eq!(s.study(&input[..2]).unwrap_err().key(), "rubberband.out_of_order", "the study is over");
    s.process(&input, &mut out).unwrap();
    s.finish(&mut out).unwrap();
    assert_eq!(out.len(), 3000);
}

#[test]
fn bad_arguments_are_errors_with_keys() {
    let s = Settings::default();
    let key = |r: lm_base::Result<Offline>| r.unwrap_err().key();
    assert_eq!(key(Offline::new(Format::new(0, 1), 10, 10, 1.0, &s)), "rubberband.bad_rate");
    assert_eq!(key(Offline::new(Format::new(RATE, 0), 10, 10, 1.0, &s)), "rubberband.bad_channels");
    assert_eq!(key(Offline::new(Format::new(RATE, 65), 10, 10, 1.0, &s)), "rubberband.bad_channels");
    assert_eq!(key(Offline::new(Format::new(RATE, 1), 10, 10, 0.0, &s)), "rubberband.bad_pitch");
    assert_eq!(key(Offline::new(Format::new(RATE, 1), 10, 10, f64::NAN, &s)), "rubberband.bad_pitch");
    assert_eq!(key(Offline::new(Format::new(RATE, 1), 10, 10, PITCH_SCALE_RANGE.1 * 2.0, &s)), "rubberband.bad_pitch");
    assert_eq!(key(Offline::new(Format::new(RATE, 1), 10, 0, 1.0, &s)), "rubberband.bad_ratio");
    assert_eq!(key(Offline::new(Format::new(RATE, 1), 0, 10, 1.0, &s)), "rubberband.bad_ratio");
    assert_eq!(key(Offline::new(Format::new(RATE, 1), 1000, 1000 * 65, 1.0, &s)), "rubberband.bad_ratio");
    assert_eq!(key(Offline::new(Format::new(RATE, 1), 1 << 33, 1 << 33, 1.0, &s)), "rubberband.too_long");
    let err = stretch(&[0.0; 5], Format::new(RATE, 2), 4, 1.0, &s, &Cancel::new(), &Progress::new()).unwrap_err();
    assert_eq!(err.key(), "rubberband.bad_length");

    assert_eq!(frames_for_ratio(1000, 1.5), 1500);
    assert_eq!(frames_for_ratio(3, 0.5), 2, "half rounds up");
    assert_eq!(frames_for_ratio(0, 2.0), 0);
    assert!(TIME_RATIO_RANGE.0 < 1.0 && TIME_RATIO_RANGE.1 > 1.0);
}

/// Where the loudest sample of `x[from..to]` is
fn peak(x: &[f32], from: usize, to: usize) -> usize {
    (from..to).max_by(|a, b| x[*a].abs().total_cmp(&x[*b].abs())).unwrap()
}

#[test]
fn key_frames_put_each_frame_where_the_map_says() {
    // Clicks every 100 ms; the first half squeezed to a quarter of the output, the second half stretched over the rest
    let frames = 48_000;
    let input: Vec<f32> = (0..frames).map(|n| if n % 4_800 == 2_400 { 1.0 } else { 0.0 }).collect();
    let map_in = |n: usize| if n <= 24_000 { n / 2 } else { 12_000 + (n - 24_000) * 3 / 2 };
    let mut s = Offline::new(Format::new(RATE, 1), frames, frames, 1.0, &Settings::default()).unwrap();
    assert_eq!(s.set_key_frames(&[(24_000, 12_000)]).unwrap_err().key(), "rubberband.out_of_order", "before the study");
    s.study(&input).unwrap();
    assert_eq!(s.set_key_frames(&[(24_000, 12_000), (20_000, 30_000)]).unwrap_err().key(), "rubberband.bad_key_frames");
    assert_eq!(s.set_key_frames(&[(48_000, 12_000)]).unwrap_err().key(), "rubberband.bad_key_frames");
    s.set_key_frames(&[(12_000, 6_000), (24_000, 12_000), (36_000, 30_000)]).unwrap();
    let mut out = Vec::new();
    s.process(&input, &mut out).unwrap();
    s.finish(&mut out).unwrap();
    assert_eq!(out.len(), frames);
    // Each click lands within 3 ms of where the map puts it
    for click in [2_400usize, 16_800, 26_400, 40_800] {
        let at = map_in(click);
        let found = peak(&out, at.saturating_sub(1_000), (at + 1_000).min(frames));
        assert!(found.abs_diff(at) <= 144, "the click at {click} should be at {at}, found at {found}");
    }
    assert_eq!(s.set_key_frames(&[(1, 1)]).unwrap_err().key(), "rubberband.out_of_order", "after the processing");
}
