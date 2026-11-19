//! Test signals and their measurement

#![allow(dead_code)]

use std::f64::consts::TAU;

/// An interleaved sine: channel 0 at `amp`, the others at the same phase and half the level (a stereo image to keep)
pub fn sine(frames: usize, channels: usize, freq: f64, rate: f64, amp: f64) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * channels);
    for n in 0..frames {
        let v = amp * (TAU * freq * n as f64 / rate).sin();
        for c in 0..channels {
            out.push(if c == 0 { v as f32 } else { (v * 0.5) as f32 });
        }
    }
    out
}

/// One channel of an interleaved buffer
pub fn channel(interleaved: &[f32], channels: usize, c: usize) -> Vec<f32> {
    interleaved.iter().skip(c).step_by(channels).copied().collect()
}

/// The frequency of a steady tone, from the upward zero crossings (linearly interpolated) of `x[from..to]`
pub fn frequency(x: &[f32], rate: f64, from: usize, to: usize) -> f64 {
    let mut first = None;
    let mut last = 0.0;
    let mut count = 0_usize;
    for n in from.max(1)..to {
        let (a, b) = (f64::from(x[n - 1]), f64::from(x[n]));
        if a < 0.0 && b >= 0.0 {
            let t = (n - 1) as f64 + a / (a - b);
            if first.is_none() {
                first = Some(t);
            }
            last = t;
            count += 1;
        }
    }
    let first = first.expect("no zero crossing");
    assert!(count > 2, "too few zero crossings ({count})");
    (count - 1) as f64 / (last - first) * rate
}

/// The RMS level of `x[from..to]`
pub fn rms(x: &[f32], from: usize, to: usize) -> f64 {
    let sum: f64 = x[from..to].iter().map(|v| f64::from(*v).powi(2)).sum();
    (sum / (to - from) as f64).sqrt()
}
