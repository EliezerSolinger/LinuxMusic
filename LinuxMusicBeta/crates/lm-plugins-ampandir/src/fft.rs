//! The FFT of the convolver: a real signal of a fixed power-of-two length to its spectrum (bins 0 to N/2) and back,
//! unnormalised as FFTW's `r2c` and `c2r` (the legacy convolver's), in single precision.
//!
//! An iterative radix-2 transform of the complex signal of half the length (the real signal's even and odd samples
//! as real and imaginary parts), then the split that gives the real signal's spectrum; the inverse undoes the split.
//! The twiddle factors are worked out in double precision when it is made. Made off the audio thread (it allocates
//! its tables); [`RealFft::forward`] and [`RealFft::inverse`] neither allocate nor lock.

use std::f64::consts::PI;

/// A complex number in single precision
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };

    #[inline]
    pub fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    #[inline]
    fn add(self, o: Self) -> Self {
        Self::new(self.re + o.re, self.im + o.im)
    }

    #[inline]
    fn sub(self, o: Self) -> Self {
        Self::new(self.re - o.re, self.im - o.im)
    }

    #[inline]
    fn conj(self) -> Self {
        Self::new(self.re, -self.im)
    }
}

impl std::ops::Mul for Complex {
    type Output = Self;

    #[inline]
    fn mul(self, o: Self) -> Self {
        Self::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}

/// A real FFT of `len` samples (a power of two, at least 4)
#[derive(Clone, Debug)]
pub struct RealFft {
    len: usize,
    /// exp(-2 pi i k / (len / 2)), k < len / 4: the half-length complex transform's twiddles
    twiddles: Vec<Complex>,
    /// exp(-2 pi i k / len), k <= len / 2: the split's
    split: Vec<Complex>,
    /// The bit-reversed index of each of the half-length transform's points
    reversed: Vec<u32>,
    /// The half-length complex signal being transformed
    work: Vec<Complex>,
}

impl RealFft {
    pub fn new(len: usize) -> Self {
        assert!(len.is_power_of_two() && len >= 4, "the FFT's length must be a power of two, at least 4");
        let half = len / 2;
        let angle = |k: usize, n: usize| -2.0 * PI * k as f64 / n as f64;
        let twiddles =
            (0..half / 2).map(|k| Complex::new(angle(k, half).cos() as f32, angle(k, half).sin() as f32)).collect();
        let split = (0..=half).map(|k| Complex::new(angle(k, len).cos() as f32, angle(k, len).sin() as f32)).collect();
        let bits = half.trailing_zeros();
        let reversed = (0..half as u32).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (32 - bits) }).collect();
        Self { len, twiddles, split, reversed, work: vec![Complex::ZERO; half] }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// The number of bins of a spectrum: `len / 2 + 1`
    pub fn bins(&self) -> usize {
        self.len / 2 + 1
    }

    /// The half-length complex transform of `work`, in place (forward: exp(-i...); inverse: the conjugate)
    fn transform(&mut self, inverse: bool) {
        let n = self.work.len();
        for i in 0..n {
            let j = self.reversed[i] as usize;
            if i < j {
                self.work.swap(i, j);
            }
        }
        let mut size = 2;
        while size <= n {
            let step = n / size;
            for start in (0..n).step_by(size) {
                for k in 0..size / 2 {
                    let w = self.twiddles[k * step];
                    let w = if inverse { w.conj() } else { w };
                    let a = self.work[start + k];
                    let b = self.work[start + k + size / 2] * w;
                    self.work[start + k] = a.add(b);
                    self.work[start + k + size / 2] = a.sub(b);
                }
            }
            size *= 2;
        }
    }

    /// The spectrum of `input` (`len` samples) into `output` (`len / 2 + 1` bins), unnormalised
    pub fn forward(&mut self, input: &[f32], output: &mut [Complex]) {
        let half = self.len / 2;
        debug_assert!(input.len() == self.len && output.len() == half + 1);
        for (k, w) in self.work.iter_mut().enumerate() {
            *w = Complex::new(input[2 * k], input[2 * k + 1]);
        }
        self.transform(false);
        // X[k] = (Z[k] + conj Z[n-k]) / 2 + w^k (Z[k] - conj Z[n-k]) / (2i)
        for (k, out) in output.iter_mut().enumerate().take(half + 1) {
            let z = self.work[k % half];
            let zc = self.work[(half - k) % half].conj();
            let even = Complex::new(0.5 * (z.re + zc.re), 0.5 * (z.im + zc.im));
            let d = z.sub(zc);
            let odd = Complex::new(0.5 * d.im, -0.5 * d.re);
            *out = even.add(self.split[k] * odd);
        }
    }

    /// The real signal (`len` samples) of `input` (`len / 2 + 1` bins), unnormalised (`len` times the signal whose
    /// spectrum it is, as FFTW's `c2r`); the imaginary parts of bins 0 and `len / 2` are ignored
    pub fn inverse(&mut self, input: &[Complex], output: &mut [f32]) {
        let half = self.len / 2;
        debug_assert!(output.len() == self.len && input.len() == half + 1);
        // The split undone: Z[k] = E[k] + i O[k], E[k] = X[k] + conj X[n-k], O[k] = (X[k] - conj X[n-k]) conj w^k
        for k in 0..half {
            let x = if k == 0 { Complex::new(input[0].re, 0.0) } else { input[k] };
            let y = if k == 0 { Complex::new(input[half].re, 0.0) } else { input[half - k] }.conj();
            let even = x.add(y);
            let odd = x.sub(y) * self.split[k].conj();
            self.work[k] = Complex::new(even.re - odd.im, even.im + odd.re);
        }
        self.transform(true);
        for (k, z) in self.work.iter().enumerate() {
            output[2 * k] = z.re;
            output[2 * k + 1] = z.im;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The DFT by its definition, in double precision
    fn dft(x: &[f32]) -> Vec<(f64, f64)> {
        let n = x.len();
        (0..=n / 2)
            .map(|k| {
                x.iter().enumerate().fold((0.0, 0.0), |(re, im), (j, &v)| {
                    let a = -2.0 * PI * (k * j) as f64 / n as f64;
                    (re + f64::from(v) * a.cos(), im + f64::from(v) * a.sin())
                })
            })
            .collect()
    }

    fn signal(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 101) as f32 / 50.0 - 1.0).collect()
    }

    #[test]
    fn forward_is_the_dft() {
        for n in [4, 8, 64, 512] {
            let x = signal(n);
            let mut fft = RealFft::new(n);
            let mut spectrum = vec![Complex::ZERO; fft.bins()];
            fft.forward(&x, &mut spectrum);
            for (k, (re, im)) in dft(&x).into_iter().enumerate() {
                let tolerance = 1.0e-5 * n as f64;
                assert!((f64::from(spectrum[k].re) - re).abs() < tolerance, "n {n} bin {k}");
                assert!((f64::from(spectrum[k].im) - im).abs() < tolerance, "n {n} bin {k}");
            }
        }
    }

    #[test]
    fn inverse_gives_the_signal_times_its_length() {
        for n in [4, 16, 512] {
            let x = signal(n);
            let mut fft = RealFft::new(n);
            let mut spectrum = vec![Complex::ZERO; fft.bins()];
            fft.forward(&x, &mut spectrum);
            let mut back = vec![0.0f32; n];
            fft.inverse(&spectrum, &mut back);
            for (a, b) in x.iter().zip(&back) {
                assert!((a * n as f32 - b).abs() < 1.0e-4 * n as f32, "n {n}: {a} and {}", b / n as f32);
            }
        }
    }
}
