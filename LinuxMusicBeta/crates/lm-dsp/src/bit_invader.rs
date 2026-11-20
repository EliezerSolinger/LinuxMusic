//! The legacy program's BitInvader: a drawn wave of up to 200 points played as a note, stepped or interpolated,
//! band limited (through [`DrawnWave`]) or as drawn; and the release fade every legacy instrument without an
//! envelope of its own gets (`Instrument::applyRelease`).

use crate::drawn_wave::{DrawnWave, Hold, MOST_HARMONICS};
use crate::oscillator::lerp;

/// The most points of the wave
pub const WAVETABLE_SIZE: usize = 200;
/// The release, ms
pub const RELEASE_MS: f32 = 1.5;

/// The wave the notes play, made from the graph when it or its switches change (away from the audio thread)
#[derive(Clone, Debug)]
pub struct BitInvaderWave {
    points: Vec<f32>,
    /// 1 / the largest point (the Normalize switch's factor)
    normalize_factor: f32,
    interpolation: bool,
    /// The band-limited tables, with anti-aliasing on
    drawn: Option<DrawnWave>,
}

impl BitInvaderWave {
    /// From the graph's first @p points (1 to 200 of them)
    pub fn new(points: &[f32], interpolation: bool, anti_aliasing: bool) -> Self {
        let points = &points[..points.len().clamp(1, WAVETABLE_SIZE)];
        let max = points.iter().fold(f32::EPSILON, |m, p| if p.abs() > m { p.abs() } else { m });
        let drawn = anti_aliasing
            .then(|| DrawnWave::new(points, if interpolation { Hold::Linear } else { Hold::Step }, MOST_HARMONICS));
        Self { points: points.to_vec(), normalize_factor: (1.0 / f64::from(max)) as f32, interpolation, drawn }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

/// One note of the BitInvader
#[derive(Clone, Debug)]
pub struct BitInvaderVoice {
    shape: Vec<f32>,
    factor: f32,
    real_index: f32,
}

impl BitInvaderVoice {
    /// A note of @p wave, normalized or not
    pub fn new(wave: &BitInvaderWave, normalize: bool) -> Self {
        let factor = if normalize { wave.normalize_factor } else { 1.0 };
        let shape = wave
            .points
            .iter()
            .map(|p| {
                let v = p * factor;
                // (normalized, nothing past full scale: a rounding over 1 is clipped)
                if factor != 1.0 && v.abs() > 1.0 { if v < 0.0 { -1.0 } else { 1.0 } } else { v }
            })
            .collect();
        Self { shape, factor, real_index: 0.0 }
    }

    /// The next @p out frames of the note at @p frequency (it may bend from one call to the next)
    pub fn render(&mut self, out: &mut [[f32; 2]], wave: &BitInvaderWave, frequency: f32, sample_rate: f32) {
        let length = wave.len() as f32;
        let step = length / (sample_rate / frequency);
        match &wave.drawn {
            Some(drawn) => {
                let table = drawn.table_for(frequency, sample_rate);
                for frame in out.iter_mut() {
                    while self.real_index >= length {
                        self.real_index -= length;
                    }
                    let phase = self.real_index / length;
                    self.real_index += step;
                    let v = table.map_or(0.0, |t| DrawnWave::at(t, phase) * self.factor);
                    *frame = [v, v];
                }
            }
            None => {
                for frame in out.iter_mut() {
                    while self.real_index >= length {
                        self.real_index -= length;
                    }
                    let current = self.real_index;
                    // (the float may round up to the length: the last point)
                    let index = (current as usize).min(wave.len() - 1);
                    self.real_index += step;
                    let v = if wave.interpolation {
                        let next = if (index as f32) < length - 1.0 { index + 1 } else { 0 };
                        lerp(self.shape[index], self.shape[next], current - current.trunc())
                    } else {
                        self.shape[index]
                    };
                    *frame = [v, v];
                }
            }
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
    fn stepped_wave_plays_its_points_at_the_note_rate() {
        // Four points at a note whose period is eight frames: each point held two frames
        let wave = BitInvaderWave::new(&[0.5, -0.25, 1.0, 0.0], false, false);
        let mut voice = BitInvaderVoice::new(&wave, false);
        let mut out = [[0.0; 2]; 8];
        voice.render(&mut out, &wave, 1000.0, 8000.0);
        let left: Vec<f32> = out.iter().map(|f| f[0]).collect();
        assert_eq!(left, [0.5, 0.5, -0.25, -0.25, 1.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn normalized_wave_reaches_full_scale() {
        let wave = BitInvaderWave::new(&[0.25, -0.5], false, false);
        let mut voice = BitInvaderVoice::new(&wave, true);
        let mut out = [[0.0; 2]; 4];
        voice.render(&mut out, &wave, 1000.0, 4000.0);
        assert_eq!(out.map(|f| f[0]), [0.5, 0.5, -1.0, -1.0]);
    }

    #[test]
    fn band_limited_note_above_the_cutoff_is_silent() {
        let wave = BitInvaderWave::new(&[1.0, -1.0, 1.0, -1.0], false, true);
        let mut voice = BitInvaderVoice::new(&wave, false);
        let mut out = [[1.0; 2]; 16];
        voice.render(&mut out, &wave, 21000.0, 44100.0);
        assert!(out.iter().all(|f| f[0] == 0.0));
    }

    #[test]
    fn release_fades_the_last_frames_to_zero() {
        let mut out = [[1.0; 2]; 8];
        apply_release(&mut out, 6, 4);
        assert_eq!(out.map(|f| f[0]), [1.0, 1.0, 1.0, 0.75, 0.5, 0.25, 1.0, 1.0]);
        assert_eq!(release_frames(1.5, 44100.0), 67);
    }
}
