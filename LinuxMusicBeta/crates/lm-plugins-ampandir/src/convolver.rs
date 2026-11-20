//! The legacy convolver (`plugins/AmpAndIr/Convolver.cpp`, the legacy program's own), ported: one channel convolved
//! with an impulse response by uniformly partitioned overlap-save. The IR is cut in blocks of [`BLOCK`]; each block's
//! spectrum is multiplied with the spectrum of the input as many blocks before, and the products summed in the
//! frequency domain. Its latency is one block.
//!
//! Made off the audio thread (the IR's spectra and every buffer are allocated then); [`Convolver::process`] and
//! [`Convolver::reset`] neither allocate nor lock.

use crate::fft::{Complex, RealFft};

/// The block (and the latency), frames
pub const BLOCK: usize = 256;
const FFT: usize = 2 * BLOCK;
const BINS: usize = FFT / 2 + 1;

/// One channel's convolver
#[derive(Clone, Debug)]
pub struct Convolver {
    fft: RealFft,
    /// Each partition's spectrum
    ir: Vec<[Complex; BINS]>,
    /// The spectra of the input's last blocks, a ring
    input: Vec<[Complex; BINS]>,
    newest: usize,
    /// The last two blocks of input (overlap-save)
    time_in: [f32; FFT],
    time_out: [f32; FFT],
    spectrum: [Complex; BINS],
    accumulator: [Complex; BINS],
    /// The block being gathered
    in_block: [f32; BLOCK],
    /// The last block's output, being handed out
    out_block: [f32; BLOCK],
    position: usize,
}

impl Convolver {
    /// A convolver of `impulse` (an empty one is a single zero, as the legacy one)
    pub fn new(impulse: &[f32]) -> Self {
        let mut fft = RealFft::new(FFT);
        let count = impulse.len().max(1).div_ceil(BLOCK);
        let mut time = [0.0f32; FFT];
        let mut ir = Vec::with_capacity(count);
        for p in 0..count {
            time.fill(0.0);
            for (i, t) in time.iter_mut().take(BLOCK).enumerate() {
                if let Some(&x) = impulse.get(p * BLOCK + i) {
                    *t = x;
                }
            }
            let mut spectrum = [Complex::ZERO; BINS];
            fft.forward(&time, &mut spectrum);
            ir.push(spectrum);
        }
        let input = vec![[Complex::ZERO; BINS]; count];
        Self {
            fft,
            ir,
            input,
            newest: 0,
            time_in: [0.0; FFT],
            time_out: [0.0; FFT],
            spectrum: [Complex::ZERO; BINS],
            accumulator: [Complex::ZERO; BINS],
            in_block: [0.0; BLOCK],
            out_block: [0.0; BLOCK],
            position: 0,
        }
    }

    /// How many partitions the IR has
    pub fn partitions(&self) -> usize {
        self.ir.len()
    }

    /// Convolves `io` in place (any number of frames: they are gathered in blocks)
    pub fn process(&mut self, io: &mut [f32]) {
        for x in io {
            // In, and out what the last block made (a block's latency)
            self.in_block[self.position] = *x;
            *x = self.out_block[self.position];
            self.position += 1;
            if self.position == BLOCK {
                self.block();
                self.position = 0;
            }
        }
    }

    /// Lets go of the input it holds (from rest again)
    pub fn reset(&mut self) {
        for s in &mut self.input {
            s.fill(Complex::ZERO);
        }
        self.time_in.fill(0.0);
        self.time_out.fill(0.0);
        self.in_block.fill(0.0);
        self.out_block.fill(0.0);
        self.position = 0;
    }

    fn block(&mut self) {
        // Overlap-save: the last two blocks of input, transformed
        self.time_in.copy_within(BLOCK.., 0);
        self.time_in[BLOCK..].copy_from_slice(&self.in_block);
        self.fft.forward(&self.time_in, &mut self.spectrum);
        let count = self.input.len();
        self.newest = (self.newest + 1) % count;
        self.input[self.newest] = self.spectrum;

        // Each partition with the input of as many blocks before
        self.accumulator.fill(Complex::ZERO);
        for (p, ir) in self.ir.iter().enumerate() {
            let input = &self.input[(self.newest + count - p) % count];
            for ((acc, x), h) in self.accumulator.iter_mut().zip(input).zip(ir) {
                let product = *x * *h;
                acc.re += product.re;
                acc.im += product.im;
            }
        }

        // Back in time: the second half is this block's output (the first is the circular wrap)
        self.fft.inverse(&self.accumulator, &mut self.time_out);
        let scale = 1.0 / FFT as f32;
        for (out, x) in self.out_block.iter_mut().zip(&self.time_out[BLOCK..]) {
            *out = x * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct(x: &[f32], h: &[f32]) -> Vec<f64> {
        (0..x.len())
            .map(|n| (0..h.len()).filter(|&k| k <= n).map(|k| f64::from(x[n - k]) * f64::from(h[k])).sum())
            .collect()
    }

    #[test]
    fn it_is_the_convolution_one_block_late() {
        let h: Vec<f32> = (0..700).map(|i| ((i * 37 % 61) as f32 / 61.0 - 0.5) * 0.9f32.powi(i / 40)).collect();
        let x: Vec<f32> = (0..3000).map(|i| (i * 7919 % 113) as f32 / 113.0 - 0.5).collect();
        let want = direct(&x, &h);
        let mut c = Convolver::new(&h);
        assert_eq!(c.partitions(), 3);
        let mut io = x.clone();
        for piece in io.chunks_mut(77) {
            c.process(piece);
        }
        for n in BLOCK..x.len() {
            let got = f64::from(io[n]);
            assert!((got - want[n - BLOCK]).abs() < 1.0e-4, "sample {n}: {got} and {}", want[n - BLOCK]);
        }
        assert!(io[..BLOCK].iter().all(|v| *v == 0.0));
    }
}
