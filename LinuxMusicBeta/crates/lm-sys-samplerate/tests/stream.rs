//! The streaming converter against the real libsamplerate

use lm_sys_samplerate::{Quality, Resampler, is_valid_ratio};
use std::f64::consts::TAU;

fn sine(frames: usize, freq: f64, rate: f64) -> Vec<f32> {
    (0..frames).map(|n| (0.5 * (TAU * freq * n as f64 / rate).sin()) as f32).collect()
}

/// Streams `input` through in blocks of the given sizes (cycled), with the output in blocks of `out_block`
fn stream(r: &mut Resampler, input: &[f32], blocks: &[usize], out_block: usize) -> Vec<f32> {
    let ch = r.channels();
    let mut out = Vec::new();
    let mut buf = vec![0.0_f32; out_block * ch];
    let mut pos = 0;
    let mut i = 0;
    while pos < input.len() / ch {
        let end = (pos + blocks[i % blocks.len()]).min(input.len() / ch);
        i += 1;
        let mut chunk = &input[pos * ch..end * ch];
        while !chunk.is_empty() {
            let done = r.process(chunk, &mut buf).unwrap();
            out.extend_from_slice(&buf[..done.output_frames * ch]);
            chunk = &chunk[done.input_frames * ch..];
        }
        pos = end;
    }
    loop {
        let done = r.finish(&mut buf).unwrap();
        if done.output_frames == 0 {
            break;
        }
        out.extend_from_slice(&buf[..done.output_frames * ch]);
    }
    out
}

#[test]
fn block_sizes_do_not_change_the_result() {
    let input = sine(50_000, 997.0, 44100.0);
    let ratio = 48000.0 / 44100.0;
    let mut a = Resampler::new(Quality::Best, 1, ratio).unwrap();
    let one = stream(&mut a, &input, &[50_000], 100_000);
    let mut b = Resampler::new(Quality::Best, 1, ratio).unwrap();
    let many = stream(&mut b, &input, &[1, 63, 64, 128, 17, 1000], 37);
    assert_eq!(one.len(), many.len());
    let worst = one.iter().zip(&many).map(|(x, y)| (x - y).abs()).fold(0.0_f32, f32::max);
    assert!(worst < 1e-6, "differs by {worst}");
}

#[test]
fn the_latency_is_what_the_stream_shows() {
    for quality in [Quality::Best, Quality::Medium, Quality::Fastest, Quality::Linear] {
        for ratio in [48000.0 / 44100.0, 44100.0 / 48000.0, 2.0, 0.5] {
            let mut r = Resampler::new(quality, 2, ratio).unwrap();
            let latency = r.latency();
            assert!((0.0..10_000.0).contains(&latency), "{quality:?} {ratio}: {latency}");
            // An impulse goes in at frame 1000; the output is aligned with the input (no group delay)
            let mut input = vec![0.0_f32; 2 * 20_000];
            input[2 * 1000] = 1.0;
            input[2 * 1000 + 1] = -1.0;
            let mut out = vec![0.0_f32; 2 * 50_000];
            let (mut used, mut made) = (0, 0);
            let mut first_out_at = None;
            while used < 20_000 {
                let end = (used + 64).min(20_000);
                let done = r.process(&input[used * 2..end * 2], &mut out[made * 2..]).unwrap();
                used += done.input_frames;
                made += done.output_frames;
                if first_out_at.is_none() && made > 0 {
                    first_out_at = Some(used);
                }
                let expected = ((used as f64 - latency) * ratio).max(0.0);
                assert!(
                    (made as f64 - expected).abs() <= 64.0 * ratio + 2.0,
                    "{quality:?} {ratio}: {made} vs {expected}"
                );
            }
            if !matches!(quality, Quality::Best | Quality::Medium | Quality::Fastest) {
                // The plain converters are not aligned (they trail by one input frame) and may skip an impulse
                continue;
            }
            let peak = (0..made).max_by(|&a, &b| out[a * 2].abs().total_cmp(&out[b * 2].abs())).unwrap();
            let expected = 1000.0 * ratio;
            assert!((peak as f64 - expected).abs() <= 1.0 + ratio, "{quality:?} {ratio}: peak at {peak}, {expected}");
            assert!(out[peak * 2 + 1] < 0.0, "the channels stay in place");
        }
    }
}

#[test]
fn a_ratio_change_glides_and_a_step_is_immediate() {
    let input = vec![0.1_f32; 4096];
    let mut out = vec![0.0_f32; 16_384];
    let mut r = Resampler::new(Quality::Fastest, 1, 1.0).unwrap();
    // Warm up past the latency
    let mut pos = 0;
    while pos < 4096 {
        pos += r.process(&input[pos..], &mut out).unwrap().input_frames;
    }
    r.set_ratio_now(2.0).unwrap();
    let step = r.process(&input[..1024], &mut out).unwrap();
    let rate = step.output_frames as f64 / step.input_frames as f64;
    assert!((rate - 2.0).abs() < 0.05, "a step gives the new ratio at once ({rate})");
    r.set_ratio(1.0).unwrap();
    // The glide is spread over the output buffer given: about what the block will produce
    let glide = r.process(&input[..1024], &mut out[..1536]).unwrap();
    let rate = glide.output_frames as f64 / glide.input_frames as f64;
    assert!(rate > 1.2 && rate < 1.8, "a glide is between the two ratios ({rate})");
    assert_eq!(r.ratio(), 1.0);
    r.reset().unwrap();
    assert_eq!(r.set_ratio(1000.0).unwrap_err().key(), "samplerate.bad_ratio");
    assert!(!is_valid_ratio(f64::NAN));
}

#[test]
fn the_resampler_moves_to_another_thread() {
    let mut r = Resampler::new(Quality::Medium, 2, 0.5).unwrap();
    let r = std::thread::spawn(move || {
        let mut out = [0.0_f32; 256];
        r.process(&[0.0; 512], &mut out).unwrap();
        r
    })
    .join()
    .unwrap();
    assert_eq!(r.channels(), 2);
    assert_eq!(Resampler::new(Quality::Best, 0, 1.0).unwrap_err().key(), "samplerate.bad_channels");
}
