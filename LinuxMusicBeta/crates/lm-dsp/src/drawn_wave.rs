//! One period of a wave drawn as points (the BitInvader's, the Watsyn's), made into tables that play it at any pitch
//! with none of its harmonics above the cutoff (the legacy `DrawnWave`): a harmonic above half the rate folds back
//! down, between the note's own, as a tone that has nothing to do with the note.
//!
//! Its harmonics are those of the wave as it plays, worked out from the points: each held until the next (a
//! staircase), or joined to it by a straight line. There is a table for each count of harmonics, two semitones
//! apart; a note plays the one with the most harmonics that stay below the cutoff (20 kHz, or a little under half
//! the rate). Each has sixteen samples a harmonic at least (and 256 in all), read with a cubic.
//!
//! Made away from the audio thread (it allocates); the tables are only read after.

use crate::math::{Complex, exp2, f64c::PI, sin};

/// How the points play between them
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// Each held until the next one
    Step,
    /// Joined by straight lines (the last one to the first)
    Linear,
}

/// A period of the wave with `harmonics` harmonics, `length` samples, kept with the last sample before the first and
/// the first two after the last (for the interpolation)
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub harmonics: usize,
    pub length: usize,
    /// `length + 3`
    pub samples: Vec<f32>,
}

/// The most harmonics kept: all of them up to 20 kHz down to notes of about 20 Hz
pub const MOST_HARMONICS: usize = 1024;

/// The tables of a drawn wave
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawnWave {
    /// From the most harmonics to the fewest
    tables: Vec<Table>,
}

fn sub(a: Complex, b: Complex) -> Complex {
    Complex::new(a.re - b.re, a.im - b.im)
}

/// In place, @p data a power of two long: data[m] becomes the sum of data[k] e^(2 pi i k m / size) (not scaled)
fn inverse_fft(data: &mut [Complex]) {
    let size = data.len();
    let mut j = 0usize;
    for i in 1..size {
        let mut bit = size >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            data.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= size {
        let half = length / 2;
        for k in 0..half {
            let w = Complex::polar(1.0, 2.0 * PI * k as f64 / length as f64);
            let mut start = 0;
            while start < size {
                let even = data[start + k];
                let odd = data[start + k + half] * w;
                data[start + k] = even + odd;
                data[start + k + half] = sub(even, odd);
                start += length;
            }
        }
        length <<= 1;
    }
}

impl DrawnWave {
    /// The tables of @p points played with @p hold, of at most @p most_harmonics harmonics
    pub fn new(points: &[f32], hold: Hold, most_harmonics: usize) -> Self {
        let count = points.len();
        if count == 0 {
            return Self::default();
        }
        // The points' discrete Fourier transform, X[k] (it repeats every count harmonics): only the residues needed
        let residues = count.min(most_harmonics + 1);
        let twiddles: Vec<Complex> =
            (0..count).map(|m| Complex::polar(1.0, -2.0 * PI * m as f64 / count as f64)).collect();
        let transform: Vec<Complex> = (0..residues)
            .map(|k| {
                let mut sum = Complex::new(0.0, 0.0);
                let mut index = 0; // (j k) mod count
                for &p in points {
                    sum += f64::from(p) * twiddles[index];
                    index += k;
                    if index >= count {
                        index -= count;
                    }
                }
                sum
            })
            .collect();

        // The harmonics as it plays: each point's step contributes e^(-i pi k/n) sinc(k/n), each point's triangle
        // sinc^2(k/n)
        let mut coefficients = vec![Complex::new(0.0, 0.0); most_harmonics + 1];
        let mut largest = 0.0f64;
        for (k, c) in coefficients.iter_mut().enumerate() {
            let x = k as f64 / count as f64;
            let sinc = if k == 0 { 1.0 } else { sin(PI * x) / (PI * x) };
            let shape =
                if hold == Hold::Step { sinc * Complex::polar(1.0, -PI * x) } else { Complex::new(sinc * sinc, 0.0) };
            let t = transform[k % count];
            *c = Complex::new(t.re / count as f64, t.im / count as f64) * shape;
            if k > 0 {
                largest = largest.max(c.abs());
            }
        }
        // Down to the last harmonic that counts (a smooth wave has few)
        let mut harmonics = most_harmonics;
        while harmonics > 1 && coefficients[harmonics].abs() <= largest * 1e-6 {
            harmonics -= 1;
        }

        // A table for each count of harmonics, two semitones apart, down to one
        let ratio = exp2(-2.0 / 12.0);
        let mut tables = Vec::new();
        loop {
            let length = 256usize.max((16 * harmonics).next_power_of_two());
            let mut spectrum = vec![Complex::new(0.0, 0.0); length];
            spectrum[0] = Complex::new(coefficients[0].re, 0.0);
            for k in 1..=harmonics {
                spectrum[k] = coefficients[k];
                spectrum[length - k] = Complex::new(coefficients[k].re, -coefficients[k].im);
            }
            inverse_fft(&mut spectrum);
            let mut samples = vec![0.0f32; length + 3];
            for m in 0..length {
                samples[m + 1] = spectrum[m].re as f32;
            }
            samples[0] = samples[length];
            samples[length + 1] = samples[1];
            samples[length + 2] = samples[2];
            tables.push(Table { harmonics, length, samples });
            if harmonics == 1 {
                break;
            }
            let fewer = (harmonics as f64 * ratio).floor() as usize;
            harmonics = fewer.clamp(1, harmonics - 1);
        }
        Self { tables }
    }

    /// The highest frequency a note's harmonics reach, played at @p sample_rate
    pub fn cutoff(sample_rate: f32) -> f32 {
        20000.0f32.min(0.475 * sample_rate)
    }

    /// The table for a note at @p frequency played at @p sample_rate; none when even its first harmonic is above the
    /// cutoff (the note is silent)
    pub fn table_for(&self, frequency: f32, sample_rate: f32) -> Option<&Table> {
        if self.tables.is_empty() {
            return None;
        }
        if frequency.is_nan() || frequency <= 0.0 {
            return self.tables.first();
        }
        let allowed = (Self::cutoff(sample_rate) / frequency).floor();
        if allowed < 1.0 {
            return None;
        }
        self.tables.iter().find(|t| t.harmonics as f32 <= allowed).or(self.tables.last())
    }

    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    /// The wave in @p table at @p phase (a fraction of the period, 0 to 1): a cubic (Hermite) between its samples
    #[inline]
    pub fn at(table: &Table, phase: f32) -> f32 {
        let position = phase * table.length as f32;
        let mut index = if position > 0.0 { position as usize } else { 0 };
        if index >= table.length {
            index = table.length - 1;
        }
        let t = position - index as f32;
        let p = &table.samples[index..index + 4]; // p[1] is the sample at index
        let c1 = 0.5 * (p[2] - p[0]);
        let c2 = p[0] - 2.5 * p[1] + 2.0 * p[2] - 0.5 * p[3];
        let c3 = 0.5 * (p[3] - p[0]) + 1.5 * (p[1] - p[2]);
        ((c3 * t + c2) * t + c1) * t + p[1]
    }
}
