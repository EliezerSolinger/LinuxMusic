//! The legacy program's SlicerT (Daniel Kauss Serna's): a sample cut into slices, the base note playing it whole and
//! each key above one slice, in time with the song if synced, faded out at the slice's end (or looped while held).

use crate::math::{cosf, exp2f, f32c::PI};
use crate::sample_player::{Bounds, Interpolator, Loop, PlaybackState, Sample};

/// A sample as the SlicerT holds it
#[derive(Clone, Copy, Debug)]
pub struct SlicerSample<'a> {
    pub frames: &'a [[f32; 2]],
    pub sample_rate: u32,
}

/// The SlicerT's knobs a note reads
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlicerSettings {
    /// ms, 0 to 100: the fade at a slice's end (the release, looped)
    pub fade_out: f32,
    /// The sample's tempo, BPM
    pub original_bpm: i32,
    /// Played at the song's tempo
    pub sync: bool,
    pub looping: Loop,
}

impl Default for SlicerSettings {
    fn default() -> Self {
        Self { fade_out: 10.0, original_bpm: 1, sync: false, looping: Loop::Off }
    }
}

/// The release, ms: the fade out when looped (a slice not looped fades out by itself)
pub fn release_ms(s: &SlicerSettings) -> f32 {
    if s.looping != Loop::Off { s.fade_out } else { 0.0 }
}

/// The slice a note @p index keys above the base one plays, as fractions of the sample (the base note: all of it);
/// none above the last slice
pub fn slice_for(points: &[f32], index: i32) -> Option<(f32, f32)> {
    if index == 0 {
        Some((0.0, 1.0))
    } else if index > 0 && (index as usize) + 1 < points.len() {
        Some((points[index as usize - 1], points[index as usize]))
    } else {
        None
    }
}

/// One note of the SlicerT. Its `clone_from` copies into its own buffer (a host copies a note on the audio thread)
#[derive(Debug)]
pub struct SlicerVoice<I> {
    start: i32,
    end: i32,
    state: PlaybackState<I>,
    /// How far into its slice it went, frames of the sample
    played: f64,
}

impl<I: Clone> Clone for SlicerVoice<I> {
    fn clone(&self) -> Self {
        Self { start: self.start, end: self.end, state: self.state.clone(), played: self.played }
    }

    fn clone_from(&mut self, source: &Self) {
        self.start = source.start;
        self.end = source.end;
        self.state.clone_from(&source.state);
        self.played = source.played;
    }
}

impl<I: Interpolator> SlicerVoice<I> {
    /// Whether the note's slice lies within a sample of @p frames frames (a sample replaced under a sounding note)
    pub fn fits(&self, frames: usize) -> bool {
        self.end as usize <= frames
    }

    /// A new note in this voice, as [`SlicerVoice::new`] makes one, without allocating
    pub fn restart(&mut self, slice: (f32, f32), frames: usize) {
        let size = frames as i32;
        self.start = ((slice.0 * size as f32) as i32).clamp(0, size);
        self.end = ((slice.1 * size as f32) as i32).clamp(self.start, size);
        self.state.restart(self.start);
        self.played = 0.0;
    }

    /// A note on the slice @p slice (fractions of the sample) of a sample of @p frames frames; @p interpolator:
    /// linear, as the legacy program's
    pub fn new(slice: (f32, f32), frames: usize, interpolator: I) -> Self {
        let size = frames as i32;
        let start = ((slice.0 * size as f32) as i32).clamp(0, size);
        let end = ((slice.1 * size as f32) as i32).clamp(start, size);
        Self { start, end, state: PlaybackState::new(interpolator, start), played: 0.0 }
    }

    /// Where the note is in the sample, a fraction (for the view)
    pub fn position(&self, frames: usize) -> f32 {
        self.state.frame_index() as f32 / frames as f32
    }

    /// The next @p out frames of the note at the song's @p bpm and the track's @p pitch (cents), into a stream at
    /// @p output_rate in periods of @p period frames. False once it has ended (@p out is then untouched). Looped
    /// and let go, the host fades it out ([`crate::instrument::apply_release`], [`release_ms`])
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        out: &mut [[f32; 2]],
        sample: &SlicerSample<'_>,
        s: &SlicerSettings,
        bpm: f32,
        pitch: f32,
        output_rate: f32,
        period: usize,
    ) -> bool {
        if sample.frames.len() <= 1 {
            return false;
        }
        let pitch_ratio = 1.0 / exp2f(pitch / 1200.0);
        let mut speed = s.original_bpm as f32 / bpm;
        if !s.sync {
            speed = 1.0;
        }
        speed *= pitch_ratio;
        let output_ratio = f64::from(speed) * f64::from(output_rate as u32) / f64::from(sample.sample_rate);
        let player = Sample {
            frames: sample.frames,
            sample_rate: sample.sample_rate as f32,
            bounds: Bounds { start: self.start, end: self.end, loop_start: self.start, loop_end: self.end },
            amplification: 1.0,
            frequency: crate::sample_player::DEFAULT_BASE_FREQ,
            reversed: false,
            declick: false,
        };
        let frames = out.len();
        if !(self.end > self.start
            && (s.looping != Loop::Off || self.played < f64::from(self.end - self.start))
            && player.play(out, &mut self.state, s.looping, f64::from(speed), output_rate, period))
        {
            return false;
        }
        if s.looping == Loop::Off {
            // Faded out at the slice's end (a cosine)
            let fade = s.fade_out / 1000.0 * output_rate;
            let left_frames = (f64::from(self.end - self.start) - self.played) * output_ratio;
            for (i, frame) in out.iter_mut().enumerate() {
                let left = left_frames as f32 - i as f32;
                let v = if fade > 0.0 {
                    (left / fade).clamp(0.0, 1.0)
                } else if left > 0.0 {
                    1.0
                } else {
                    0.0
                };
                let g = (1.0 - cosf(v * PI)) * 0.5 * (1.0 - 0.0) + 0.0;
                frame[0] *= g;
                frame[1] *= g;
            }
        }
        self.played += frames as f64 / output_ratio;
        true
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The analysis of a sample: its slices and its tempo (away from the audio thread)

/// The spectrum of @p input (a power of two long), bins 0 to n/2, as (real, imaginary): an FFT in double (the
/// legacy program's FFTW in single precision, to its rounding)
fn real_spectrum(input: &[f32]) -> Vec<(f64, f64)> {
    let n = input.len();
    let mut re: Vec<f64> = input.iter().map(|&x| f64::from(x)).collect();
    let mut im = vec![0.0f64; n];
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= n {
        let half = length / 2;
        for k in 0..half {
            let angle = -2.0 * core::f64::consts::PI * k as f64 / length as f64;
            let (wr, wi) = (crate::math::cos(angle), crate::math::sin(angle));
            let mut start = 0;
            while start < n {
                let (a, b) = (start + k, start + k + half);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                start += length;
            }
        }
        length <<= 1;
    }
    (0..=n / 2).map(|k| (re[k], im[k])).collect()
}

/// The slice points of @p frames (fractions, from 0 to 1): onsets by the spectral flux (a window's magnitudes'
/// change over the last one's, above 1 + @p threshold), moved to a zero crossing near them, snapped to
/// 1/2^(@p snap - 1) notes of @p bpm (0: not snapped)
pub fn find_slices(frames: &[[f32; 2]], sample_rate: u32, threshold: f32, bpm: i32, snap: i32) -> Vec<f32> {
    const WINDOW: usize = 512;
    if frames.len() <= 1 {
        return Vec::new();
    }
    let min_dist = (sample_rate as f32 * 0.05) as i64;
    let mono: Vec<f32> = frames.iter().map(|f| (f[0] + f[1]) / 2.0).collect();

    let mut zero_crossings = Vec::new();
    let mut last = 1.0f32;
    for (i, &x) in mono.iter().enumerate() {
        if (last >= 0.0) != (x >= 0.0) {
            zero_crossings.push(i as i64);
            last = x;
        }
    }

    let mut points: Vec<f32> = Vec::new();
    let mut prev_mags = vec![0.0f32; WINDOW / 2];
    let mut last_point = -min_dist - 1;
    let mut flux = 0.0f32;
    let mut prev_flux = 1e-10f32;
    let mut i = 0i64;
    while i < mono.len() as i64 - WINDOW as i64 {
        let spectrum = real_spectrum(&mono[i as usize..i as usize + WINDOW]);
        for j in 0..WINDOW / 2 {
            let (re, im) = (spectrum[j].0 as f32, spectrum[j].1 as f32);
            let magnitude = (re * re + im * im).sqrt();
            flux += (magnitude - prev_mags[j]).abs();
            prev_mags[j] = magnitude;
        }
        if flux / prev_flux > 1.0 + threshold && i - last_point > min_dist {
            points.push(i as f32);
            last_point = i;
            if points.len() > 128 {
                break;
            }
        }
        prev_flux = flux;
        flux = 1e-10;
        i += WINDOW as i64;
    }

    if points.first() != Some(&0.0) {
        points.insert(0, 0.0);
    }
    let size = frames.len() as f32;
    points.push(size);
    for p in &mut points {
        let at = zero_crossings.partition_point(|&z| (z as f32) < *p);
        if let Some(&z) = zero_crossings.get(at)
            && (*p - z as f32).abs() < WINDOW as f32
        {
            *p = z as f32;
        }
    }
    let beats_per_second = bpm as f32 / 60.0;
    let samples_per_beat = sample_rate as f32 / beats_per_second * 4.0;
    let lock = if snap == 0 {
        1
    } else {
        ((f64::from(samples_per_beat) / crate::math::exp2(f64::from(snap + 1))) as i32).max(1)
    };
    for p in &mut points {
        *p += lock as f32 / 2.0;
        *p -= ((*p as i32) % lock) as f32;
        *p = p.min(size);
    }
    points.dedup();
    if points.len() < 2 {
        points = vec![0.0, size];
    }
    for p in &mut points {
        *p /= size;
    }
    points[0] = 0.0;
    let last = points.len() - 1;
    points[last] = 1.0;
    points
}

/// The tempo of @p frames (0: no beat found): onset strength by the rise of the log spectrum, its autocorrelation,
/// the pulse that scores best on and between one to four beats (60 to 200 BPM, weighted towards 120), then its
/// octave by the beats alone
pub fn estimate_tempo(frames: &[[f32; 2]], sample_rate: u32) -> f64 {
    const WINDOW: usize = 1024;
    const HOP: usize = 256;
    const LOWEST: f64 = 60.0;
    const HIGHEST: f64 = 200.0;
    const LIKELIEST: f64 = 120.0;
    const BEATS: i32 = 4;
    let rate = f64::from(sample_rate);
    let n = frames.len().min((60.0 * rate) as usize);
    let hop_rate = rate / HOP as f64;
    let longest_lag = f64::from(BEATS) * 60.0 / LOWEST * hop_rate;
    if n < WINDOW * 4 {
        return 0.0;
    }
    let hops = (n - WINDOW) / HOP + 1;
    if (hops as f64) < 2.0 * 60.0 / LOWEST * hop_rate {
        return 0.0;
    }

    let window: Vec<f32> = (0..WINDOW).map(|i| 0.5 - 0.5 * cosf(2.0 * PI * i as f32 / WINDOW as f32)).collect();
    let mut input = vec![0.0f32; WINDOW];
    let mut previous = vec![0.0f32; WINDOW / 2 + 1];
    let mut onsets = vec![0.0f64; hops];
    for hop in 0..hops {
        for i in 0..WINDOW {
            let f = frames[hop * HOP + i];
            input[i] = (f[0] + f[1]) * 0.5 * window[i];
        }
        let spectrum = real_spectrum(&input);
        let mut rise = 0.0f64;
        for (bin, (re, im)) in spectrum.iter().enumerate() {
            let (re, im) = (*re as f32, *im as f32);
            let magnitude = crate::math::log1pf(100.0 * crate::math::hypotf(re, im));
            if hop > 0 {
                rise += f64::from((magnitude - previous[bin]).max(0.0));
            }
            previous[bin] = magnitude;
        }
        onsets[hop] = rise;
    }

    // Less the local mean (a third of a second each way), above zero
    let reach = (hop_rate / 3.0) as usize;
    let mut sums = vec![0.0f64; hops + 1];
    for i in 0..hops {
        sums[i + 1] = sums[i] + onsets[i];
    }
    let strength: Vec<f64> = (0..hops)
        .map(|i| {
            let from = i.saturating_sub(reach);
            let to = hops.min(i + reach + 1);
            (onsets[i] - (sums[to] - sums[from]) / (to - from) as f64).max(0.0)
        })
        .collect();
    {
        let mut sorted = strength.clone();
        let top = hops * 99 / 100;
        sorted.select_nth_unstable_by(top, |a, b| a.total_cmp(b));
        if sorted[top] < 4.0 * (sums[hops] / hops as f64) {
            return 0.0;
        }
    }

    let lags = (hops - 1).min(longest_lag.ceil() as usize + 2);
    let mut correlation: Vec<f64> = (0..=lags)
        .map(|lag| {
            let mut sum = 0.0;
            let mut i = 0;
            while i + lag < hops {
                sum += strength[i] * strength[i + lag];
                i += 1;
            }
            sum / (hops - lag) as f64
        })
        .collect();
    let energy = correlation[0];
    if energy <= 0.0 {
        return 0.0;
    }
    for v in &mut correlation {
        *v /= energy;
    }
    let at = |lag: f64| {
        let below = lag as usize;
        if below + 1 > lags {
            return 0.0;
        }
        let t = lag - below as f64;
        correlation[below] * (1.0 - t) + correlation[below + 1] * t
    };
    let prior = |bpm: f64| {
        let octaves = crate::math::log2(bpm / LIKELIEST);
        crate::math::exp(-0.5 * octaves * octaves)
    };
    let comb = |bpm: f64, between: bool| {
        let beat = 60.0 / bpm * hop_rate;
        let (mut sum, mut teeth) = (0.0, 0);
        let mut k = 1;
        while k <= BEATS && f64::from(k) * beat <= (lags - 1) as f64 {
            sum += at(f64::from(k) * beat);
            teeth += 1;
            if between {
                sum += at((f64::from(k) - 0.5) * beat);
                teeth += 1;
            }
            k += 1;
        }
        if teeth > 0 { sum / f64::from(teeth) } else { 0.0 }
    };
    let (mut pulse, mut pulse_score) = (0.0, 0.0);
    let mut bpm = LOWEST;
    while bpm <= HIGHEST {
        let score = comb(bpm, true) * prior(bpm);
        if score > pulse_score {
            pulse_score = score;
            pulse = bpm;
        }
        bpm += 0.05;
    }
    if pulse <= 0.0 {
        return 0.0;
    }
    while pulse / 2.0 >= LOWEST {
        pulse /= 2.0;
    }
    let (mut best, mut best_score, mut best_comb) = (0.0, 0.0, 0.0);
    let mut bpm = pulse;
    while bpm <= HIGHEST {
        let beats = comb(bpm, false);
        if beats * prior(bpm) > best_score {
            best_score = beats * prior(bpm);
            best = bpm;
            best_comb = beats;
        }
        bpm *= 2.0;
    }
    if best_comb > 0.3 { best } else { 0.0 }
}

/// The sample's tempo for the "Original bpm" knob (1 to 999): the estimate, fitted to whole bars if it is a loop
/// of them; without a beat, its length taken as a bar, doubled or halved into 100 to 200
pub fn find_bpm(frames: &[[f32; 2]], sample_rate: u32) -> i32 {
    let rate = f64::from(sample_rate);
    let length = frames.len() as f64 / rate;
    let mut bpm = estimate_tempo(frames, sample_rate);
    if bpm <= 0.0 {
        bpm = 240.0 / length;
        while bpm < 100.0 {
            bpm *= 2.0;
        }
        while bpm > 200.0 {
            bpm /= 2.0;
        }
    } else {
        let bars = length * bpm / 240.0;
        let whole = bars.round();
        if whole >= 1.0 && (bars - whole).abs() < 0.005 * whole {
            bpm = 240.0 * whole / length;
        }
    }
    (bpm.round() as i64).clamp(1, 999) as i32
}
