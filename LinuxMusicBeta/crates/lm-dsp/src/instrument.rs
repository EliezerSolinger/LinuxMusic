//! What every legacy instrument's notes get from `Instrument`: the fade-in that keeps a note from starting with a
//! jump, its length from how often the note crosses zero, and the release fade of the instruments without an
//! envelope of their own.

use crate::math::{cosf, f32c::PI};

/// The most crossings of zero of either side in `buf[start..end]` (from `start - 1`), a crossing being a step from
/// at most zero to above it or from at least zero to below it
fn zero_crossings(buf: &[[f32; 2]], start: usize, end: usize) -> i32 {
    let mut counts = [0i32; 2];
    let mut most = 0;
    for f in start..end {
        for ch in 0..2 {
            let (a, b) = (buf[f - 1][ch], buf[f][ch]);
            if (a <= 0.0 && b > 0.0) || (a >= 0.0 && b < 0.0) {
                counts[ch] += 1;
                most = most.max(counts[ch]);
            }
        }
    }
    most
}

fn fade_in_length(max_length: f32, frames: usize, crossings: i32, block_frames: f32) -> i64 {
    (max_length / (crossings as f32 / (frames as f32 / block_frames) + 1.0)) as i64
}

/// A note's fade-in, carried from one period to the next
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FadeIn {
    /// Frames
    length: i64,
}

impl FadeIn {
    /// Fades in the note's frames of a period: `buf[offset..offset + frames]` in the note's first period (@p played
    /// 0), `buf[..frames]` after; at most 85 frames at 44.1 kHz (about 2 ms), shorter the more often the note crosses
    /// zero (per 128 frames at 44.1 kHz)
    pub fn apply(&mut self, buf: &mut [[f32; 2]], offset: usize, frames: usize, played: u64, sample_rate: f32) {
        let rate_factor = sample_rate / 44100.0;
        let max_length = 85.0 * rate_factor;
        let block_frames = 128.0 * rate_factor;
        if played == 0 {
            // (the first frame almost always crosses: it does not count)
            let crossings = zero_crossings(buf, offset + 1, offset + frames);
            self.length = fade_in_length(max_length, frames, crossings, block_frames);
            let length = (self.length.max(0) as usize).min(frames);
            for f in 0..length {
                let gain = 0.5 - 0.5 * f64::from(cosf(PI * f as f32 / self.length as f32));
                for ch in 0..2 {
                    buf[offset + f][ch] = (f64::from(buf[offset + f][ch]) * gain) as f32;
                }
            }
        } else if (played as i64) < self.length {
            let crossings = zero_crossings(buf, 1, frames);
            let new_length = fade_in_length(max_length, frames, crossings, block_frames);
            for f in 0..frames {
                let at = f as f32 / frames as f32;
                let current = self.length as f32 * (1.0 - at) + new_length as f32 * at;
                if (played + f as u64) as f32 >= current {
                    // Faded in
                    self.length = current as i64;
                    return;
                }
                let gain = 0.5 - 0.5 * cosf(PI * (played + f as u64) as f32 / current);
                buf[f][0] *= gain;
                buf[f][1] *= gain;
            }
            self.length = new_length;
        }
    }
}

/// The legacy `Instrument::applyRelease`: a linear fade over the note's last @p release frames, @p frames_left of it
/// left from the start of @p out
pub fn apply_release(out: &mut [[f32; 2]], frames_left: usize, release: usize) {
    if release == 0 {
        return;
    }
    let start = frames_left - frames_left.min(release);
    for f in start..frames_left.min(out.len()) {
        let fac = (frames_left - f) as f32 / release as f32;
        out[f][0] *= fac;
        out[f][1] *= fac;
    }
}

/// The frames of a release of @p ms at @p sample_rate (the legacy `desiredReleaseFrames`)
pub fn release_frames(ms: f32, sample_rate: f32) -> usize {
    (ms * sample_rate / 1000.0).ceil() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_low_note_fades_in_over_the_longest_time() {
        // No crossing: the whole 85 frames, from silence
        let mut buf = [[1.0f32; 2]; 256];
        let mut fade = FadeIn::default();
        fade.apply(&mut buf, 0, 256, 0, 44100.0);
        assert_eq!(buf[0], [0.0, 0.0]);
        assert!(buf[42][0] > 0.45 && buf[42][0] < 0.55);
        assert_eq!(buf[85], [1.0, 1.0]);
    }

    #[test]
    fn a_busy_note_fades_in_faster() {
        let mut buf: Vec<[f32; 2]> = (0..256).map(|f| if f % 2 == 0 { [1.0; 2] } else { [-1.0; 2] }).collect();
        let mut fade = FadeIn::default();
        fade.apply(&mut buf, 0, 256, 0, 44100.0);
        assert!(fade.length < 10, "{}", fade.length);
    }

    #[test]
    fn release_fades_the_last_frames_to_zero() {
        let mut out = [[1.0; 2]; 8];
        apply_release(&mut out, 6, 4);
        assert_eq!(out.map(|f| f[0]), [1.0, 1.0, 1.0, 0.75, 0.5, 0.25, 1.0, 1.0]);
        assert_eq!(release_frames(1.5, 44100.0), 67);
    }
}
