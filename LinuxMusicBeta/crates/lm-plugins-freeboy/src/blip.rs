//! Blip_Buffer 0.4 (game-music-emu, by Shay Green, LGPL-2.1 or later), ported: band-limited synthesis of steps
//! (`BlipSynth`), the buffer of deltas they are added to and its reader with the bass high-pass (`BlipBuffer`), as
//! the legacy FreeBoy compiled them (`BLIP_BUFFER_ACCURACY` 16, `BLIP_PHASE_BITS` 6, not `BLIP_BUFFER_FAST`).
//!
//! The arithmetic is the original's, in its types: the deltas and the buffer in wrapping 32-bit integers (C's signed
//! overflow, which the original relies on, wraps on every target it builds for), the kernels made in `double` and
//! `float` as there. The kernel is made with the math functions of `libm` (the original used the C library's); the
//! A/B checks that the result is the same.

use std::f64::consts::PI;

/// Fixed-point bits of a resampled time
pub const ACCURACY: u32 = 16;
pub const PHASE_BITS: u32 = 6;
/// Phases of the kernel
pub const RES: usize = 1 << PHASE_BITS;
pub const WIDEST_IMPULSE: usize = 16;
pub const EXTRA: usize = WIDEST_IMPULSE + 2;
pub const SAMPLE_BITS: u32 = 30;
/// The buffer's length, ms (`blip_default_length`)
pub const DEFAULT_LENGTH_MS: i64 = 250;

/// A treble equalization (`blip_eq_t`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlipEq {
    pub treble: f64,
    pub rolloff_freq: i64,
    pub sample_rate: i64,
    pub cutoff_freq: i64,
}

impl BlipEq {
    /// `blip_eq_t(double treble_db)`: the treble at 44.1 kHz, no rolloff
    pub fn treble(treble: f64) -> Self {
        Self { treble, rolloff_freq: 0, sample_rate: 44_100, cutoff_freq: 0 }
    }

    fn generate(&self, out: &mut [f32]) {
        let count = out.len();
        let mut oversample = RES as f64 * 2.25 / count as f64 + 0.85;
        let half_rate = self.sample_rate as f64 * 0.5;
        if self.cutoff_freq != 0 {
            oversample = half_rate / self.cutoff_freq as f64;
        }
        let cutoff = self.rolloff_freq as f64 * oversample / half_rate;
        gen_sinc(out, RES as f64 * oversample, self.treble, cutoff);
        let to_fraction = PI / (count - 1) as f64;
        for i in (0..count).rev() {
            out[i] *= 0.54f32 - 0.46f32 * libm::cos(i as f64 * to_fraction) as f32;
        }
    }
}

fn gen_sinc(out: &mut [f32], oversample: f64, treble: f64, cutoff: f64) {
    let count = out.len() as i64;
    let cutoff = cutoff.min(0.999);
    let treble = treble.clamp(-300.0, 5.0);
    let maxh = 4096.0;
    let rolloff = libm::pow(10.0, 1.0 / (maxh * 20.0) * treble / (1.0 - cutoff));
    let pow_a_n = libm::pow(rolloff, maxh - maxh * cutoff);
    let to_angle = PI / 2.0 / maxh / oversample;
    for (i, o) in out.iter_mut().enumerate() {
        let angle = (((i as i64 - count) * 2 + 1) as f64) * to_angle;
        let angle_maxh = angle * maxh;
        let angle_maxh_mid = angle_maxh * cutoff;
        let mut y = maxh;
        if angle_maxh_mid != 0.0 {
            y *= libm::sin(angle_maxh_mid) / angle_maxh_mid;
        }
        let cosa = libm::cos(angle);
        let den = 1.0 + rolloff * (rolloff - cosa - cosa);
        if den > 1e-13 {
            let num = (libm::cos(angle_maxh - angle) * rolloff - libm::cos(angle_maxh)) * pow_a_n
                - libm::cos(angle_maxh_mid - angle) * rolloff
                + libm::cos(angle_maxh_mid);
            y = y * cutoff + num / den;
        }
        *o = y as f32;
    }
}

/// The buffer of a channel: deltas at resampled times, read out through an integrator with a bass high-pass
#[derive(Clone, Debug)]
pub struct BlipBuffer {
    pub factor: u32,
    pub offset: u32,
    buffer: Vec<i32>,
    buffer_size: i64,
    reader_accum: i32,
    pub bass_shift: i32,
    sample_rate: i64,
    clock_rate: i64,
    bass_freq: i32,
    modified: bool,
}

impl Default for BlipBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl BlipBuffer {
    pub fn new() -> Self {
        Self {
            factor: u32::MAX / 2,
            offset: 0,
            buffer: Vec::new(),
            buffer_size: 0,
            reader_accum: 0,
            bass_shift: 0,
            sample_rate: 0,
            clock_rate: 0,
            bass_freq: 16,
            modified: false,
        }
    }

    /// Sizes the buffer for `msec` ms at `rate` (allocates: build time only)
    pub fn set_sample_rate(&mut self, rate: i64, msec: i64) {
        let mut new_size = i64::from(u32::MAX >> ACCURACY) - EXTRA as i64 - 64;
        let s = (rate * (msec + 1) + 999) / 1000;
        if s < new_size {
            new_size = s;
        }
        self.buffer = vec![0; new_size as usize + EXTRA];
        self.buffer_size = new_size;
        self.sample_rate = rate;
        if self.clock_rate != 0 {
            self.set_clock_rate(self.clock_rate);
        }
        self.set_bass_freq(self.bass_freq);
        self.clear();
    }

    pub fn set_clock_rate(&mut self, cps: i64) {
        self.clock_rate = cps;
        self.factor = self.clock_rate_factor(cps);
    }

    fn clock_rate_factor(&self, rate: i64) -> u32 {
        let ratio = self.sample_rate as f64 / rate as f64;
        libm::floor(ratio * f64::from(1u32 << ACCURACY) + 0.5) as i32 as u32
    }

    pub fn set_bass_freq(&mut self, freq: i32) {
        self.bass_freq = freq;
        let mut shift = 31;
        if freq > 0 {
            shift = 13;
            let mut f = (i64::from(freq) << 16) / self.sample_rate;
            loop {
                f >>= 1;
                if f == 0 {
                    break;
                }
                shift -= 1;
                if shift == 0 {
                    break;
                }
            }
        }
        self.bass_shift = shift;
    }

    pub fn clear(&mut self) {
        self.offset = 0;
        self.reader_accum = 0;
        self.modified = false;
        self.buffer.fill(0);
    }

    /// Becomes a copy of `other` (the same length: no allocation)
    pub fn copy_from(&mut self, other: &Self) {
        self.factor = other.factor;
        self.offset = other.offset;
        self.buffer.clone_from(&other.buffer);
        self.buffer_size = other.buffer_size;
        self.reader_accum = other.reader_accum;
        self.bass_shift = other.bass_shift;
        self.sample_rate = other.sample_rate;
        self.clock_rate = other.clock_rate;
        self.bass_freq = other.bass_freq;
        self.modified = other.modified;
    }

    pub fn set_modified(&mut self) {
        self.modified = true;
    }

    pub fn clear_modified(&mut self) -> bool {
        std::mem::take(&mut self.modified)
    }

    pub fn end_frame(&mut self, t: i32) {
        self.offset = self.offset.wrapping_add((t as u32).wrapping_mul(self.factor));
    }

    pub fn samples_avail(&self) -> i64 {
        i64::from(self.offset >> ACCURACY)
    }

    pub fn resampled_duration(&self, t: i32) -> u32 {
        (t as u32).wrapping_mul(self.factor)
    }

    pub fn resampled_time(&self, t: i32) -> u32 {
        (t as u32).wrapping_mul(self.factor).wrapping_add(self.offset)
    }

    pub fn remove_silence(&mut self, count: i64) {
        self.offset = self.offset.wrapping_sub((count as u32) << ACCURACY);
    }

    pub fn remove_samples(&mut self, count: i64) {
        if count != 0 {
            self.remove_silence(count);
            let remain = (self.samples_avail() + EXTRA as i64) as usize;
            let count = count as usize;
            self.buffer.copy_within(count..count + remain, 0);
            self.buffer[remain..remain + count].fill(0);
        }
    }

    pub(crate) fn reader(&self) -> (i32, i32) {
        (self.reader_accum, self.bass_shift)
    }

    pub(crate) fn sample_at(&self, i: usize) -> i32 {
        self.buffer[i]
    }

    pub(crate) fn set_reader(&mut self, accum: i32) {
        self.reader_accum = accum;
    }

    fn add(&mut self, at: usize, value: i32) {
        self.buffer[at] = self.buffer[at].wrapping_add(value);
    }
}

/// A band-limited step synthesizer of `QUALITY` points (`Blip_Synth<quality, 1>`)
#[derive(Clone, Debug)]
pub struct BlipSynth<const QUALITY: usize> {
    delta_factor: i32,
    volume_unit: f64,
    kernel_unit: i64,
    impulses: Vec<i16>,
}

impl<const QUALITY: usize> Default for BlipSynth<QUALITY> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const QUALITY: usize> BlipSynth<QUALITY> {
    /// (allocates its kernel: build time only)
    pub fn new() -> Self {
        Self { delta_factor: 0, volume_unit: 0.0, kernel_unit: 0, impulses: vec![0; RES * (QUALITY / 2) + 1] }
    }

    fn impulses_size(&self) -> usize {
        RES / 2 * QUALITY + 1
    }

    /// In place (the same kernel size: no allocation)
    pub fn copy_from(&mut self, other: &Self) {
        self.delta_factor = other.delta_factor;
        self.volume_unit = other.volume_unit;
        self.kernel_unit = other.kernel_unit;
        self.impulses.copy_from_slice(&other.impulses);
    }

    fn adjust_impulse(&mut self) {
        let size = self.impulses_size();
        // (`for (p = blip_res; p-- >= blip_res / 2;)`: p from blip_res − 1 down to blip_res / 2 − 1)
        for p in (RES / 2 - 1..RES).rev() {
            // (−1 for the first: the sample before each row's first)
            let p2 = RES as isize - 2 - p as isize;
            let mut error = self.kernel_unit;
            let mut i = 1;
            while i < size {
                error -= i64::from(self.impulses[i + p]);
                error -= i64::from(self.impulses[(i as isize + p2) as usize]);
                i += RES;
            }
            if p as isize == p2 {
                error /= 2;
            }
            let at = size - RES + p;
            self.impulses[at] = (i32::from(self.impulses[at]) + i32::from(error as i16)) as i16;
        }
    }

    pub fn treble_eq(&mut self, eq: &BlipEq) {
        let mut fimpulse = [0.0f32; RES / 2 * (WIDEST_IMPULSE - 1) + RES * 2];
        let half_size = RES / 2 * (QUALITY - 1);
        eq.generate(&mut fimpulse[RES..RES + half_size]);
        for i in (0..RES).rev() {
            fimpulse[RES + half_size + i] = fimpulse[RES + half_size - 1 - i];
        }
        for f in fimpulse.iter_mut().take(RES) {
            *f = 0.0;
        }
        let mut total = 0.0f64;
        for f in &fimpulse[RES..RES + half_size] {
            total += f64::from(*f);
        }
        let base_unit = 32768.0;
        let rescale = base_unit / 2.0 / total;
        self.kernel_unit = base_unit as i64;
        let mut sum = 0.0f64;
        let mut next = 0.0f64;
        for i in 0..self.impulses_size() {
            self.impulses[i] = libm::floor((next - sum) * rescale + 0.5) as i32 as i16;
            sum += f64::from(fimpulse[i]);
            next += f64::from(fimpulse[i + RES]);
        }
        self.adjust_impulse();
        let vol = self.volume_unit;
        if vol != 0.0 {
            self.volume_unit = 0.0;
            self.set_volume_unit(vol);
        }
    }

    fn set_volume_unit(&mut self, new_unit: f64) {
        if new_unit == self.volume_unit {
            return;
        }
        if self.kernel_unit == 0 {
            self.treble_eq(&BlipEq::treble(-8.0));
        }
        self.volume_unit = new_unit;
        let mut factor = new_unit * f64::from(1u32 << SAMPLE_BITS) / self.kernel_unit as f64;
        if factor > 0.0 {
            let mut shift = 0;
            while factor < 2.0 {
                shift += 1;
                factor *= 2.0;
            }
            if shift != 0 {
                self.kernel_unit >>= shift;
                let offset = 0x8000i64 + (1i64 << (shift - 1));
                let offset2 = 0x8000i64 >> shift;
                for imp in self.impulses.iter_mut().take(RES / 2 * QUALITY + 1).rev() {
                    *imp = (((i64::from(*imp) + offset) >> shift) - offset2) as i16;
                }
                self.adjust_impulse();
            }
        }
        self.delta_factor = libm::floor(factor + 0.5) as i32;
    }

    /// `Blip_Synth::volume` (range 1: `v * (1.0 / 1)` is `v`)
    pub fn volume(&mut self, v: f64) {
        self.set_volume_unit(v);
    }

    /// A step of `delta` at clock `t` of the frame
    pub fn offset(&self, t: i32, delta: i32, buf: &mut BlipBuffer) {
        self.offset_resampled(buf.resampled_time(t), delta, buf);
    }

    /// A step of `delta` at resampled time `time`
    pub fn offset_resampled(&self, time: u32, delta: i32, buf: &mut BlipBuffer) {
        let delta = delta.wrapping_mul(self.delta_factor);
        let base = (time >> ACCURACY) as usize;
        let phase = ((time >> (ACCURACY - PHASE_BITS)) as usize) & (RES - 1);
        let fwd = (WIDEST_IMPULSE - QUALITY) / 2;
        let rev = fwd + QUALITY - 2;
        let mid = QUALITY / 2 - 1;
        let imp = &self.impulses;
        let mut start = RES - phase;
        let mut add = |out: usize, at: usize, buf: &mut BlipBuffer| {
            buf.add(base + out, i32::from(imp[at]).wrapping_mul(delta));
        };
        // (the order of the original's x86 path; any order gives the same wrapping sums)
        let fwd_pair =
            |i: usize, start: usize, add: &mut dyn FnMut(usize, usize, &mut BlipBuffer), buf: &mut BlipBuffer| {
                add(fwd + i, start + RES * i, buf);
                add(fwd + 1 + i, start + RES * (i + 1), buf);
            };
        fwd_pair(0, start, &mut add, buf);
        if QUALITY > 8 {
            fwd_pair(2, start, &mut add, buf);
        }
        if QUALITY > 12 {
            fwd_pair(4, start, &mut add, buf);
        }
        add(fwd + mid - 1, start + RES * (mid - 1), buf);
        add(fwd + mid, start + RES * mid, buf);
        start = phase;
        let rev_pair =
            |r: usize, start: usize, add: &mut dyn FnMut(usize, usize, &mut BlipBuffer), buf: &mut BlipBuffer| {
                add(rev - r, start + RES * (r + 1), buf);
                add(rev + 1 - r, start + RES * r, buf);
            };
        if QUALITY > 12 {
            rev_pair(6, start, &mut add, buf);
        }
        if QUALITY > 8 {
            rev_pair(4, start, &mut add, buf);
        }
        rev_pair(2, start, &mut add, buf);
        add(rev, start + RES, buf);
        add(rev + 1, start, buf);
    }
}

/// `Stereo_Buffer`: the center, left and right buffers, mixed when read
#[derive(Clone, Debug, Default)]
pub struct StereoBuffer {
    /// center, left, right
    pub bufs: [BlipBuffer; 3],
    stereo_added: i32,
    was_stereo: i32,
}

pub const CENTER: usize = 0;
pub const LEFT: usize = 1;
pub const RIGHT: usize = 2;

impl StereoBuffer {
    pub fn set_sample_rate(&mut self, rate: i64, msec: i64) {
        for b in &mut self.bufs {
            b.set_sample_rate(rate, msec);
        }
    }

    pub fn set_clock_rate(&mut self, rate: i64) {
        for b in &mut self.bufs {
            b.set_clock_rate(rate);
        }
    }

    pub fn set_bass_freq(&mut self, bass: i32) {
        for b in &mut self.bufs {
            b.set_bass_freq(bass);
        }
    }

    pub fn clear(&mut self) {
        self.stereo_added = 0;
        self.was_stereo = 0;
        for b in &mut self.bufs {
            b.clear();
        }
    }

    pub fn copy_from(&mut self, other: &Self) {
        for (a, b) in self.bufs.iter_mut().zip(&other.bufs) {
            a.copy_from(b);
        }
        self.stereo_added = other.stereo_added;
        self.was_stereo = other.was_stereo;
    }

    pub fn end_frame(&mut self, clock_count: i32) {
        self.stereo_added = 0;
        for (i, b) in self.bufs.iter_mut().enumerate() {
            self.stereo_added |= i32::from(b.clear_modified()) << i;
            b.end_frame(clock_count);
        }
    }

    /// Frames available
    pub fn frames_avail(&self) -> i64 {
        self.bufs[0].samples_avail()
    }

    /// Reads up to `out.len()` frames of interleaved left/right; returns how many
    pub fn read(&mut self, out: &mut [[i16; 2]]) -> usize {
        let count = (out.len() as i64).min(self.bufs[0].samples_avail());
        if count == 0 {
            return 0;
        }
        let out = &mut out[..count as usize];
        let used = self.stereo_added | self.was_stereo;
        if used <= 1 {
            self.mix_mono(out);
            self.bufs[0].remove_samples(count);
            self.bufs[1].remove_silence(count);
            self.bufs[2].remove_silence(count);
        } else if used & 1 != 0 {
            self.mix_stereo(out);
            for b in &mut self.bufs {
                b.remove_samples(count);
            }
        } else {
            self.mix_stereo_no_center(out);
            self.bufs[0].remove_silence(count);
            self.bufs[1].remove_samples(count);
            self.bufs[2].remove_samples(count);
        }
        if self.bufs[0].samples_avail() == 0 {
            self.was_stereo = self.stereo_added;
            self.stereo_added = 0;
        }
        count as usize
    }

    fn mix_stereo(&mut self, out: &mut [[i16; 2]]) {
        let bass = self.bufs[1].bass_shift;
        let mut readers = [self.bufs[0].reader().0, self.bufs[1].reader().0, self.bufs[2].reader().0];
        for (n, o) in out.iter_mut().enumerate() {
            let c = readers[0] >> (SAMPLE_BITS - 16);
            let mut l = c.wrapping_add(readers[1] >> (SAMPLE_BITS - 16));
            let mut r = c.wrapping_add(readers[2] >> (SAMPLE_BITS - 16));
            l = clamp16(l);
            r = clamp16(r);
            for (k, acc) in readers.iter_mut().enumerate() {
                *acc = next(*acc, self.bufs[k].sample_at(n), bass);
            }
            *o = [l as i16, r as i16];
        }
        for (k, acc) in readers.iter().enumerate() {
            self.bufs[k].set_reader(*acc);
        }
    }

    fn mix_stereo_no_center(&mut self, out: &mut [[i16; 2]]) {
        let bass = self.bufs[1].bass_shift;
        let mut left = self.bufs[1].reader().0;
        let mut right = self.bufs[2].reader().0;
        for (n, o) in out.iter_mut().enumerate() {
            let l = clamp16(left >> (SAMPLE_BITS - 16));
            let r = clamp16(right >> (SAMPLE_BITS - 16));
            left = next(left, self.bufs[1].sample_at(n), bass);
            right = next(right, self.bufs[2].sample_at(n), bass);
            *o = [l as i16, r as i16];
        }
        self.bufs[1].set_reader(left);
        self.bufs[2].set_reader(right);
    }

    fn mix_mono(&mut self, out: &mut [[i16; 2]]) {
        let bass = self.bufs[0].bass_shift;
        let mut center = self.bufs[0].reader().0;
        for (n, o) in out.iter_mut().enumerate() {
            let s = clamp16(center >> (SAMPLE_BITS - 16));
            center = next(center, self.bufs[0].sample_at(n), bass);
            *o = [s as i16, s as i16];
        }
        self.bufs[0].set_reader(center);
    }
}

/// `if ((int16_t) s != s) s = 0x7FFF - (s >> 24)`
fn clamp16(s: i32) -> i32 {
    if i32::from(s as i16) != s { 0x7FFF - (s >> 24) } else { s }
}

/// `BLIP_READER_NEXT`
fn next(accum: i32, sample: i32, bass: i32) -> i32 {
    accum.wrapping_add(sample.wrapping_sub(accum >> bass))
}
