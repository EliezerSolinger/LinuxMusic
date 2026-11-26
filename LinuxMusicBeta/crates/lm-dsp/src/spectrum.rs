//! A spectrum analyser for a plugin's window (the Eq's input and output curves, the legacy `EqAnalyser`): the mono mix
//! of what goes through, a Blackman-Harris window over the last [`FFT_LEN`] samples, the shared [`RealFft`], and
//! the magnitudes compressed into [`BANDS`] logarithmically spaced bands in dBFS.
//!
//! Everything is allocated when it is made (off the audio thread). [`Spectrum::push`] only fills a ring;
//! [`Spectrum::update`] runs the transform when [`HOP`] new samples have come, and neither allocates nor locks, so
//! the audio thread may call them (the legacy analysed on the audio thread too, once per 2048 samples). A sine at full
//! scale reads 0 dB.

use crate::fft::{Complex, RealFft, Window};
use crate::math::log10f;

/// The transform's length (a power of two): the legacy `FFT_BUFFER_SIZE`
pub const FFT_LEN: usize = 2048;
/// How many new samples make a new analysis (half the window)
pub const HOP: usize = FFT_LEN / 2;
/// The bands of the result
pub const BANDS: usize = 256;
/// The lowest band's frequency, in Hz
pub const LOWEST_HZ: f32 = 20.0;
/// The highest band's frequency, in Hz (or just under the Nyquist frequency)
pub const HIGHEST_HZ: f32 = 20_000.0;
/// What a silent band reads, in dB
pub const FLOOR_DB: f32 = -120.0;

#[derive(Clone, Copy, Debug)]
struct Band {
    lo: u16,
    hi: u16,
    /// Between bin `lo` and `lo + 1`, when the band is narrower than a bin (`lo == hi`)
    frac: f32,
}

/// The analyser of one signal
#[derive(Clone, Debug)]
pub struct Spectrum {
    ring: Box<[f32]>,
    pos: usize,
    fresh: usize,
    window: Box<[f32]>,
    fft: RealFft,
    input: Box<[f32]>,
    spectrum: Box<[Complex]>,
    bands: Box<[Band]>,
    db: Box<[f32]>,
    scale: f32,
    first_hz: f32,
    last_hz: f32,
}

impl Spectrum {
    pub fn new(sample_rate: f32) -> Self {
        let n = FFT_LEN;
        // (normalised by its mean: a full-scale sine's peak bin is then half the length)
        let window: Box<[f32]> = Window::BlackmanHarris.coefficients(n).into_boxed_slice();
        let nyquist = sample_rate * 0.5;
        let first_hz = LOWEST_HZ.min(nyquist * 0.5);
        let last_hz = HIGHEST_HZ.min(nyquist * 0.98).max(first_hz * 2.0);
        let bin = |hz: f32| hz * n as f32 / sample_rate;
        let edge = |k: usize| first_hz * libm::powf(last_hz / first_hz, k as f32 / BANDS as f32);
        let bands = (0..BANDS)
            .map(|k| {
                let (lo, hi) = (bin(edge(k)), bin(edge(k + 1)));
                let top = (n / 2 - 1) as f32;
                if hi - lo < 1.0 {
                    let c = (0.5 * (lo + hi)).clamp(0.0, top);
                    Band { lo: c as u16, hi: c as u16, frac: c - c.floor() }
                } else {
                    Band { lo: lo.clamp(0.0, top) as u16, hi: hi.clamp(0.0, top).ceil() as u16, frac: 0.0 }
                }
            })
            .collect();
        Self {
            ring: vec![0.0; n].into_boxed_slice(),
            pos: 0,
            fresh: 0,
            window,
            fft: RealFft::new(n),
            input: vec![0.0; n].into_boxed_slice(),
            spectrum: vec![Complex::ZERO; n / 2 + 1].into_boxed_slice(),
            bands,
            db: vec![FLOOR_DB; BANDS].into_boxed_slice(),
            scale: 2.0 / n as f32,
            first_hz,
            last_hz,
        }
    }

    /// The frequency of the first and the last band's edge, in Hz
    pub fn range_hz(&self) -> (f32, f32) {
        (self.first_hz, self.last_hz)
    }

    /// Forgets what was heard
    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.fresh = 0;
        self.db.fill(FLOOR_DB);
    }

    /// Takes the mono mix of @p frames
    pub fn push(&mut self, frames: &[[f32; 2]]) {
        for f in frames {
            self.ring[self.pos] = (f[0] + f[1]) * 0.5;
            self.pos = (self.pos + 1) % FFT_LEN;
        }
        self.fresh = (self.fresh + frames.len()).min(FFT_LEN);
    }

    /// Analyses the last window when [`HOP`] samples have come since the last time; whether it did
    pub fn update(&mut self) -> bool {
        if self.fresh < HOP {
            return false;
        }
        self.fresh = 0;
        for i in 0..FFT_LEN {
            self.input[i] = self.ring[(self.pos + i) % FFT_LEN] * self.window[i];
        }
        self.fft.forward(&self.input, &mut self.spectrum);
        let spectrum = &self.spectrum;
        let mag = |k: usize| {
            let c = spectrum[k];
            libm::sqrtf(c.re * c.re + c.im * c.im) * self.scale
        };
        for (out, b) in self.db.iter_mut().zip(self.bands.iter()) {
            let m = if b.lo == b.hi {
                let k = b.lo as usize;
                mag(k) * (1.0 - b.frac) + mag(k + 1) * b.frac
            } else {
                (b.lo..=b.hi).map(|k| mag(k as usize)).fold(0.0, f32::max)
            };
            *out = if m > 1e-6 { 20.0 * log10f(m) } else { FLOOR_DB };
        }
        true
    }

    /// The bands' levels in dBFS, from [`LOWEST_HZ`] up, logarithmically spaced
    pub fn db(&self) -> &[f32] {
        &self.db
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::f32c::PI;

    fn sine(hz: f32, rate: f32, n: usize) -> Vec<[f32; 2]> {
        (0..n)
            .map(|i| {
                let v = libm::sinf(2.0 * PI * hz * i as f32 / rate);
                [v, v]
            })
            .collect()
    }

    #[test]
    fn a_full_scale_sine_reads_about_zero_db_at_its_band() {
        let rate = 48_000.0;
        let mut s = Spectrum::new(rate);
        s.push(&sine(1000.0, rate, FFT_LEN));
        assert!(s.update());
        let (peak, loudest) =
            s.db().iter().enumerate().fold((0, FLOOR_DB), |(bi, bv), (i, &v)| if v > bv { (i, v) } else { (bi, bv) });
        assert!(loudest > -3.0 && loudest < 1.0, "{loudest}");
        let (lo, hi) = s.range_hz();
        let hz = lo * libm::powf(hi / lo, (peak as f32 + 0.5) / BANDS as f32);
        assert!((hz / 1000.0).log2().abs() < 0.2, "{hz}");
    }

    #[test]
    fn silence_is_the_floor_and_nothing_runs_before_a_hop() {
        let mut s = Spectrum::new(44_100.0);
        s.push(&[[0.0; 2]; HOP - 1]);
        assert!(!s.update());
        s.push(&[[0.0; 2]; 1]);
        assert!(s.update());
        assert!(s.db().iter().all(|&v| v == FLOOR_DB));
    }
}
