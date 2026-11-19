//! One-shot conversion against the real libsamplerate

use lm_base::{Cancel, Progress};
use lm_sys_samplerate::{Quality, convert, output_frames, version};
use std::f64::consts::TAU;

fn sine(frames: usize, channels: usize, freq: f64, rate: f64, amp: f64) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * channels);
    for n in 0..frames {
        let v = (amp * (TAU * freq * n as f64 / rate).sin()) as f32;
        for c in 0..channels {
            out.push(if c == 0 { v } else { -v });
        }
    }
    out
}

fn run(input: &[f32], channels: usize, from: u32, to: u32, quality: Quality) -> Vec<f32> {
    convert(input, channels, from, to, quality, &Cancel::new(), &Progress::new()).unwrap()
}

fn db(x: f64) -> f64 {
    20.0 * x.log10()
}

#[test]
fn the_library_is_linked() {
    assert!(version().contains("libsamplerate"), "{}", version());
}

#[test]
fn the_length_is_exact_for_every_common_pair() {
    let rates = [8000, 22050, 44100, 48000, 88200, 96000, 192000];
    for &from in &rates {
        for &to in &rates {
            for frames in [0, 1, 7, 1000, 44_101] {
                for channels in [1, 2] {
                    let input = vec![0.25_f32; frames * channels];
                    let out = run(&input, channels, from, to, Quality::Fastest);
                    assert_eq!(
                        out.len(),
                        output_frames(frames, from, to) * channels,
                        "{from}->{to} {frames}x{channels}"
                    );
                }
            }
        }
    }
    assert_eq!(output_frames(44_100, 44_100, 48_000), 48_000);
    assert_eq!(output_frames(3, 2, 1), 2, "half rounds up");
    assert_eq!(output_frames(usize::MAX / 4, 1, 4), (usize::MAX / 4) * 4, "no overflow");
}

#[test]
fn identical_rates_are_a_bit_exact_copy() {
    let input = sine(10_000, 2, 1234.5, 48000.0, 0.7);
    assert_eq!(run(&input, 2, 48000, 48000, Quality::Best), input);
}

#[test]
fn a_sine_keeps_its_frequency_level_and_shape_at_the_best_quality() {
    for (from, to) in [(44100, 48000), (48000, 44100), (48000, 96000), (96000, 48000)] {
        let frames = from as usize; // one second
        let input = sine(frames, 2, 1000.0, f64::from(from), 0.5);
        let out = run(&input, 2, from, to, Quality::Best);
        let ideal = sine(output_frames(frames, from, to), 2, 1000.0, f64::from(to), 0.5);
        // Away from the edges (the filter sees silence before the start and after the end)
        let skip = to as usize / 20;
        let (mut err, mut sig) = (0.0_f64, 0.0_f64);
        for i in skip * 2..out.len() - skip * 2 {
            err += (f64::from(out[i]) - f64::from(ideal[i])).powi(2);
            sig += f64::from(ideal[i]).powi(2);
        }
        let residual = db((err / sig).sqrt());
        assert!(residual < -90.0, "{from}->{to}: residual {residual:.1} dB");
    }
}

#[test]
fn the_best_quality_rejects_what_would_alias() {
    // 30 kHz at 96 kHz has no place at 44.1 kHz (Nyquist 22.05 kHz): it must not fold back to 14.1 kHz
    let input = sine(96_000, 1, 30_000.0, 96_000.0, 1.0);
    let out = run(&input, 1, 96_000, 44_100, Quality::Best);
    let skip = 4410;
    let peak = out[skip..out.len() - skip].iter().fold(0.0_f32, |m, v| m.max(v.abs()));
    assert!(db(f64::from(peak)) < -90.0, "alias at {:.1} dB", db(f64::from(peak)));
    // And the linear converter, which is not band-limited, does let it through: the test can tell
    let out = run(&input, 1, 96_000, 44_100, Quality::Linear);
    let peak = out[skip..out.len() - skip].iter().fold(0.0_f32, |m, v| m.max(v.abs()));
    assert!(db(f64::from(peak)) > -40.0);
}

#[test]
fn cancel_and_progress() {
    let input = sine(300_000, 1, 440.0, 44100.0, 0.5);
    let cancel = Cancel::new();
    cancel.cancel();
    let err = convert(&input, 1, 44100, 48000, Quality::Best, &cancel, &Progress::new()).unwrap_err();
    assert!(err.is_cancelled());

    let progress = Progress::new();
    convert(&input, 1, 44100, 48000, Quality::Fastest, &Cancel::new(), &progress).unwrap();
    assert_eq!(progress.fraction(), Some(1.0));
}

#[test]
fn bad_arguments_are_errors_with_keys() {
    let p = Progress::new();
    let c = Cancel::new();
    assert_eq!(
        convert(&[0.0; 4], 0, 44100, 48000, Quality::Best, &c, &p).unwrap_err().key(),
        "samplerate.bad_channels"
    );
    assert_eq!(convert(&[0.0; 5], 2, 44100, 48000, Quality::Best, &c, &p).unwrap_err().key(), "samplerate.bad_length");
    assert_eq!(convert(&[0.0; 4], 2, 0, 48000, Quality::Best, &c, &p).unwrap_err().key(), "samplerate.bad_ratio");
    assert_eq!(
        convert(&[0.0; 4], 2, 1000, 1_000_000, Quality::Best, &c, &p).unwrap_err().key(),
        "samplerate.bad_ratio"
    );
}
