//! The oscillator of the legacy program's instruments (legacy `Oscillator`, the heart of the TripleOscillator): eight
//! wave shapes, band-limited tables, and five ways for one oscillator to work with the next.

use std::sync::OnceLock;

use crate::math::{cos, f32c, f64c, sin, sinf};

/// Wavetables cover the audible spectrum only
pub const MAX_FREQ: i32 = 20000;
/// The table's length: enough for all the audible bands of MIDI note 1 (20 kHz / 8.176 Hz)
pub const WAVETABLE_LENGTH: usize = 2446;
/// One table per semitone
pub const SEMITONES_PER_TABLE: i32 = 1;
pub const WAVE_TABLES_PER_WAVEFORM: usize = 128;

/// The wave shapes. Their keys are saved with projects and never change (U30).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WaveShape {
    Sine,
    Triangle,
    Saw,
    Square,
    MoogSaw,
    Exponential,
    WhiteNoise,
    UserDefined,
}

impl WaveShape {
    pub const ALL: [WaveShape; 8] = [
        WaveShape::Sine,
        WaveShape::Triangle,
        WaveShape::Saw,
        WaveShape::Square,
        WaveShape::MoogSaw,
        WaveShape::Exponential,
        WaveShape::WhiteNoise,
        WaveShape::UserDefined,
    ];

    pub fn key(self) -> &'static str {
        match self {
            WaveShape::Sine => "sine",
            WaveShape::Triangle => "triangle",
            WaveShape::Saw => "saw",
            WaveShape::Square => "square",
            WaveShape::MoogSaw => "moog-saw",
            WaveShape::Exponential => "exponential",
            WaveShape::WhiteNoise => "white-noise",
            WaveShape::UserDefined => "user",
        }
    }

    pub fn from_key(key: &str) -> Option<WaveShape> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The index of its band-limited tables, for the shapes that have them
    fn table(self) -> Option<usize> {
        match self {
            WaveShape::Triangle => Some(0),
            WaveShape::Saw => Some(1),
            WaveShape::Square => Some(2),
            WaveShape::MoogSaw => Some(3),
            WaveShape::Exponential => Some(4),
            _ => None,
        }
    }
}

/// How an oscillator works with the next one (its sub-oscillator). Their keys are saved with projects (U30).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModulationAlgo {
    PhaseModulation,
    AmplitudeModulation,
    SignalMix,
    SynchronizedBySubOsc,
    FrequencyModulation,
}

impl ModulationAlgo {
    pub const ALL: [ModulationAlgo; 5] = [
        ModulationAlgo::PhaseModulation,
        ModulationAlgo::AmplitudeModulation,
        ModulationAlgo::SignalMix,
        ModulationAlgo::SynchronizedBySubOsc,
        ModulationAlgo::FrequencyModulation,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ModulationAlgo::PhaseModulation => "pm",
            ModulationAlgo::AmplitudeModulation => "am",
            ModulationAlgo::SignalMix => "mix",
            ModulationAlgo::SynchronizedBySubOsc => "sync",
            ModulationAlgo::FrequencyModulation => "fm",
        }
    }

    pub fn from_key(key: &str) -> Option<ModulationAlgo> {
        Self::ALL.into_iter().find(|a| a.key() == key)
    }
}

// ------------------------------------------------------------------------------------------------------------------
// The wave shapes

/// The fraction above the floor: 2.3 gives 0.3, -2.3 gives 0.7 (a phase made positive)
#[inline]
pub fn abs_fraction(x: f32) -> f32 {
    x - x.floor()
}
#[inline]
fn abs_fraction64(x: f64) -> f64 {
    x - x.floor()
}
/// The fraction toward zero: 2.3 gives 0.3, -2.3 gives -0.3
#[inline]
fn fraction(x: f32) -> f32 {
    x - x.trunc()
}

/// Linear interpolation as libc++'s `std::lerp` (exact at both ends, monotonic), which the legacy program used
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    if (a <= 0.0 && b >= 0.0) || (a >= 0.0 && b <= 0.0) {
        return t * b + (1.0 - t) * a;
    }
    if t == 1.0 {
        return b;
    }
    let x = a + t * (b - a);
    if (t > 1.0) == (b > a) {
        if b < x { x } else { b }
    } else if x < b {
        x
    } else {
        b
    }
}

pub fn sin_sample(sample: f32) -> f32 {
    sinf(sample * 2.0 * f32c::PI)
}

pub fn triangle_sample(sample: f32) -> f32 {
    let ph = abs_fraction(sample);
    if ph <= 0.25 {
        ph * 4.0
    } else if ph <= 0.75 {
        2.0 - ph * 4.0
    } else {
        ph * 4.0 - 4.0
    }
}

pub fn saw_sample(sample: f32) -> f32 {
    -1.0 + abs_fraction(sample) * 2.0
}

pub fn square_sample(sample: f32) -> f32 {
    if abs_fraction(sample) > 0.5 { -1.0 } else { 1.0 }
}

pub fn moog_saw_sample(sample: f32) -> f32 {
    let ph = abs_fraction(sample);
    if ph < 0.5 { -1.0 + ph * 4.0 } else { 1.0 - 2.0 * ph }
}

pub fn exp_sample(sample: f32) -> f32 {
    let mut ph = abs_fraction(sample);
    if ph > 0.5 {
        ph = 1.0 - ph;
    }
    -1.0 + 8.0 * ph * ph
}

/// One sample of a user's wave (its first channel), read around in a loop
pub fn user_wave_sample(wave: &[f32], sample: f32) -> f32 {
    if wave.is_empty() {
        return 0.0;
    }
    let frames = wave.len();
    let frame = abs_fraction(sample) * frames as f32;
    let f1 = frame as usize;
    lerp(wave[f1 % frames], wave[(f1 + 1) % frames], fraction(frame))
}

/// The table band of a pitch: indexed as MIDI keys (440 Hz, A4, key 69: band 69), rounded up so that the table never
/// has harmonics above Nyquist, and kept within the tables
pub fn wave_table_band_from_freq(freq: f32) -> usize {
    let semitones = (12.0 * crate::math::log2f(freq / 440.0)).ceil() as i32;
    let band = 69i32.saturating_add(semitones) / SEMITONES_PER_TABLE;
    band.clamp(1, WAVE_TABLES_PER_WAVEFORM as i32 - 1) as usize
}

pub fn freq_from_wave_table_band(band: usize) -> f32 {
    440.0 * crate::math::exp2f((band as f32 * SEMITONES_PER_TABLE as f32 - 69.0) / 12.0)
}

/// How many harmonics the table of @p band keeps
fn bands_of(table: usize) -> usize {
    (MAX_FREQ as f32 / freq_from_wave_table_band(table)) as usize
}

// ------------------------------------------------------------------------------------------------------------------
// The band-limited tables

/// A whole waveform: one table per band, `WAVE_TABLES_PER_WAVEFORM` of `WAVETABLE_LENGTH` samples
#[derive(Clone, Debug)]
pub struct Waveform {
    samples: Vec<f32>,
}

impl Waveform {
    fn zeroed() -> Self {
        Self { samples: vec![0.0; WAVE_TABLES_PER_WAVEFORM * WAVETABLE_LENGTH] }
    }

    #[inline]
    fn table(&self, band: usize) -> &[f32] {
        &self.samples[band * WAVETABLE_LENGTH..(band + 1) * WAVETABLE_LENGTH]
    }

    fn table_mut(&mut self, band: usize) -> &mut [f32] {
        &mut self.samples[band * WAVETABLE_LENGTH..(band + 1) * WAVETABLE_LENGTH]
    }

    /// The band-limited tables of one period of a wave: for each band, the wave's spectrum kept up to that band's
    /// harmonics (an exact DFT, worked out once)
    pub fn from_period(period: &[f32]) -> Self {
        let n = WAVETABLE_LENGTH;
        let half = n / 2;
        // The spectrum, bins 0 to N/2
        let (cosines, sines) = unit_circle();
        let mut re = vec![0.0f64; half + 1];
        let mut im = vec![0.0f64; half + 1];
        for k in 0..=half {
            let (mut r, mut i) = (0.0, 0.0);
            for (j, &x) in period.iter().enumerate() {
                let index = (k * j) % n;
                r += f64::from(x) * cosines[index];
                i -= f64::from(x) * sines[index];
            }
            re[k] = r;
            im[k] = i;
        }
        // The inverse, as a real inverse FFT of the bins kept (the legacy program used FFTW's), then divided by N:
        // from the table with the fewest harmonics up, each adding the next ones
        let mut waveform = Self::zeroed();
        let mut sum = vec![0.0f64; n];
        let mut kept = 0usize; // bins summed so far: 1..=kept, besides DC
        for (j, s) in sum.iter_mut().enumerate() {
            let _ = j;
            *s = re[0];
        }
        for band in (0..WAVE_TABLES_PER_WAVEFORM).rev() {
            let bands = bands_of(band).min(half);
            for k in kept + 1..=bands {
                // The Nyquist bin counts once, the others twice (their mirror)
                let weight = if k == half { 1.0 } else { 2.0 };
                for (j, s) in sum.iter_mut().enumerate() {
                    let index = (k * j) % n;
                    *s += weight * (re[k] * cosines[index] - im[k] * sines[index]);
                }
            }
            kept = kept.max(bands);
            for (out, s) in waveform.table_mut(band).iter_mut().zip(&sum) {
                *out = (*s / n as f64) as f32;
            }
        }
        waveform
    }

    /// One sample of the table of @p band, interpolated
    #[inline]
    fn sample(&self, band: usize, sample: f32) -> f32 {
        let table = self.table(band);
        let frame = abs_fraction(sample) * WAVETABLE_LENGTH as f32;
        // A phase a hair below a whole period rounds up to the table's length in single precision (the fraction of
        // -1e-9 is 1.0): that is the period's start, not one past the table's end
        let f1 = frame as usize % WAVETABLE_LENGTH;
        let f2 = if f1 < WAVETABLE_LENGTH - 1 { f1 + 1 } else { 0 };
        lerp(table[f1], table[f2], fraction(frame))
    }
}

/// cos and sin of 2 pi j / N for every j
fn unit_circle() -> (Vec<f64>, Vec<f64>) {
    let n = WAVETABLE_LENGTH;
    let angle = |j: usize| 2.0 * f64c::PI * j as f64 / n as f64;
    ((0..n).map(|j| cos(angle(j))).collect(), (0..n).map(|j| sin(angle(j))).collect())
}

/// The built-in waves' tables: the triangle, the saw and the square summed from their harmonics, as the legacy
/// program did (each table adding its harmonics to the next one's, in single precision); the moog saw and the
/// exponential band-limited from their spectra
fn wave_tables() -> &'static [Waveform; 5] {
    static TABLES: OnceLock<[Waveform; 5]> = OnceLock::new();
    TABLES.get_or_init(|| {
        let pi = f32c::PI;
        let length = WAVETABLE_LENGTH as f32;
        let saw = |n: i32, i: usize| {
            // An offset to the position to match the phase of the saw without tables
            let imod = (i as f32 - WAVETABLE_LENGTH as f32 / 2.0) / length;
            (if n % 2 != 0 { 1.0f32 } else { -1.0 }) / n as f32 * sinf(2.0 * pi * n as f32 * imod) / (pi * 0.5)
        };
        let triangle = |n: i32, i: usize| {
            let pi_sqr = pi * pi;
            (if n & 2 != 0 { -1.0f32 } else { 1.0 }) / (n * n) as f32 * sinf(2.0 * pi * n as f32 * i as f32 / length)
                / (pi_sqr / 8.0)
        };
        let square = |n: i32, i: usize| (1.0 / n as f32) * sinf(2.0 * pi * i as f32 * n as f32 / length) / (pi / 4.0);
        let moog: Vec<f32> = (0..WAVETABLE_LENGTH).map(|i| moog_saw_sample(i as f32 / length)).collect();
        let exponential: Vec<f32> = (0..WAVETABLE_LENGTH).map(|i| exp_sample(i as f32 / length)).collect();
        [
            harmonic_waveform(triangle, true),
            harmonic_waveform(saw, false),
            harmonic_waveform(square, true),
            Waveform::from_period(&moog),
            Waveform::from_period(&exponential),
        ]
    })
}

/// Sums a wave's harmonics into its tables, from the table with the fewest up, each starting from the last
fn harmonic_waveform(harmonic: impl Fn(i32, usize) -> f32, odd_only: bool) -> Waveform {
    let mut waveform = Waveform::zeroed();
    let mut accumulated = vec![0.0f32; WAVETABLE_LENGTH];
    let mut last_bands = 0i32;
    for band in (0..WAVE_TABLES_PER_WAVEFORM).rev() {
        let bands = bands_of(band) as i32;
        let first = last_bands + 1;
        for (i, value) in accumulated.iter_mut().enumerate() {
            let mut n = if odd_only { first | 1 } else { first };
            while n <= bands {
                *value += harmonic(n, i);
                n += if odd_only { 2 } else { 1 };
            }
        }
        last_bands = bands;
        waveform.table_mut(band).copy_from_slice(&accumulated);
    }
    waveform
}

/// Makes the built-in tables now (they take a while): call when preparing, never on the audio thread
pub fn prepare_wave_tables() {
    let _ = wave_tables();
}

/// A user's wave: one period, and its band-limited tables
#[derive(Clone, Debug)]
pub struct UserWave {
    period: Vec<f32>,
    tables: Waveform,
}

impl UserWave {
    /// From one period (the first channel of the user's sample). Takes a while: in an operation, never on the audio
    /// thread
    pub fn new(period: Vec<f32>) -> Self {
        let resampled: Vec<f32> =
            (0..WAVETABLE_LENGTH).map(|j| user_wave_sample(&period, j as f32 / WAVETABLE_LENGTH as f32)).collect();
        Self { tables: Waveform::from_period(&resampled), period }
    }

    pub fn period(&self) -> &[f32] {
        &self.period
    }
}

// ------------------------------------------------------------------------------------------------------------------
// The oscillator

/// What an oscillator plays now
#[derive(Clone, Copy, Debug)]
pub struct OscParams<'a> {
    pub shape: WaveShape,
    /// How this oscillator works with the next one (unused by the last)
    pub algo: ModulationAlgo,
    /// Hz, the note's
    pub freq: f32,
    /// The detuning factor over the sample rate (in double, as the phase: a ratio such as 3 / 44100 in a float is a
    /// little off, each its own way, and the oscillators of a note drift apart)
    pub detuning: f64,
    /// In periods
    pub phase_offset: f32,
    pub volume: f32,
    /// Play the band-limited tables, rather than the waves' naive shapes
    pub use_wave_table: bool,
    pub user_wave: Option<&'a UserWave>,
}

/// A small, fast generator for the white noise, seeded (the program's render is deterministic, item 9)
#[derive(Clone, Copy, Debug)]
pub struct NoiseRng(u64);

impl NoiseRng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// SplitMix64's next value
    #[inline]
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in [-1, 1]
    #[inline]
    pub fn sample(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 8_388_607.5 - 1.0
    }

    /// A value in [0, 1]
    #[inline]
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / ((1u64 << 53) - 1) as f64
    }
}

/// One oscillator's running state, for one channel of a note
#[derive(Clone, Debug)]
pub struct Oscillator {
    sample_rate: f32,
    /// The phase offset the phase was last set with
    phase_offset: f32,
    /// In double: summed sample by sample, a float's rounding piled up, each frequency's its own way, and the
    /// oscillators of a long note drifted apart, the sound changing; the waves read it as a float
    phase: f64,
    /// Whether it modulates another's phase or frequency: then it plays its naive shape (the tables' ringing would
    /// show)
    is_modulator: bool,
    /// The band limited table the wave plays now: the one of the wave's pitch, set once a call (with phase or
    /// frequency modulation, of the highest pitch the modulation gives it)
    band: usize,
    /// The modulation's last value (phase modulation: how much it moved the phase since)
    last_modulation: f32,
    noise: NoiseRng,
}

/// For the value in `approximatelyEqual`
const F_EPSILON: f32 = 1.0e-10;

impl Oscillator {
    /// Starts at @p phase_offset; @p seed is the white noise's
    pub fn new(phase_offset: f32, sample_rate: f32, seed: u64) -> Self {
        prepare_wave_tables();
        Self {
            sample_rate,
            phase_offset,
            phase: f64::from(phase_offset),
            is_modulator: false,
            band: 1,
            last_modulation: 0.0,
            noise: NoiseRng::new(seed),
        }
    }

    /// Takes the table for a wave moving @p step periods a sample (either way)
    fn set_band_for_step(&mut self, step: f32) {
        self.band = wave_table_band_from_freq(step.abs().max(1e-6) * self.sample_rate);
    }

    /// Should be called every time the phase offset may have changed
    fn recalc_phase(&mut self, offset: f32) {
        if !(self.phase_offset == offset || (self.phase_offset - offset).abs() < F_EPSILON) {
            self.phase -= f64::from(self.phase_offset);
            self.phase_offset = offset;
            self.phase += f64::from(self.phase_offset);
        }
        self.phase = abs_fraction64(self.phase);
    }

    #[inline]
    fn sample(&mut self, p: &OscParams<'_>, sample: f32) -> f32 {
        let tables = p.use_wave_table && !self.is_modulator;
        match p.shape {
            WaveShape::Sine => {
                let current = (f64::from(p.freq) * p.detuning * f64::from(self.sample_rate)) as f32;
                if !p.use_wave_table || current < MAX_FREQ as f32 { sin_sample(sample) } else { 0.0 }
            }
            WaveShape::WhiteNoise => self.noise.sample(),
            WaveShape::UserDefined => match p.user_wave {
                Some(wave) if tables => wave.tables.sample(self.band, sample),
                Some(wave) => user_wave_sample(&wave.period, sample),
                None => 0.0,
            },
            shape => {
                if tables {
                    wave_tables()[shape.table().unwrap_or(0)].sample(self.band, sample)
                } else {
                    match shape {
                        WaveShape::Triangle => triangle_sample(sample),
                        WaveShape::Saw => saw_sample(sample),
                        WaveShape::Square => square_sample(sample),
                        WaveShape::MoogSaw => moog_saw_sample(sample),
                        _ => exp_sample(sample),
                    }
                }
            }
        }
    }

    fn step(p: &OscParams<'_>) -> f64 {
        f64::from(p.freq) * p.detuning
    }
}

/// Plays a chain of oscillators into @p out: the first one, working with the second as its algorithm says, which
/// works with the third, and so on (the TripleOscillator's three, the last one first). @p oscillators and @p params
/// go together, one each.
pub fn update(oscillators: &mut [Oscillator], params: &[OscParams<'_>], out: &mut [f32], modulator: bool) {
    let Some((osc, subs)) = oscillators.split_first_mut() else {
        out.fill(0.0);
        return;
    };
    let p = &params[0];
    if p.freq >= osc.sample_rate / 2.0 {
        out.fill(0.0);
        return;
    }
    // If it modulates another's phase or frequency, it plays its naive shape: the tables ring
    osc.is_modulator = modulator;
    // The table of the wave's pitch (once a call: the phase and frequency modulations take theirs)
    osc.set_band_for_step(Oscillator::step(p) as f32);
    if subs.is_empty() {
        update_alone(osc, p, out);
        return;
    }
    let sub_params = &params[1..];
    let sub_step = Oscillator::step(&sub_params[0]).abs();
    match p.algo {
        ModulationAlgo::PhaseModulation => {
            update(subs, sub_params, out, true);
            osc.recalc_phase(p.phase_offset);
            let step = Oscillator::step(p);
            if p.use_wave_table {
                // The table for the highest pitch the wave reaches: its own, and how fast the modulation moves its
                // phase (and the modulation's sidebands reach a modulator's pitch past it: Carson's rule)
                let mut fastest = step.abs();
                let mut last = osc.last_modulation;
                for &m in out.iter() {
                    fastest = fastest.max((step + f64::from(m) - f64::from(last)).abs());
                    last = m;
                }
                osc.set_band_for_step((fastest + sub_step) as f32);
            }
            if let Some(&last) = out.last() {
                osc.last_modulation = last;
            }
            for x in out.iter_mut() {
                *x = osc.sample(p, (osc.phase + f64::from(*x)) as f32) * p.volume;
                osc.phase += step;
            }
        }
        ModulationAlgo::AmplitudeModulation => {
            update(subs, sub_params, out, false);
            osc.recalc_phase(p.phase_offset);
            let step = Oscillator::step(p);
            for x in out.iter_mut() {
                *x *= osc.sample(p, osc.phase as f32) * p.volume;
                osc.phase += step;
            }
        }
        ModulationAlgo::SignalMix => {
            update(subs, sub_params, out, false);
            osc.recalc_phase(p.phase_offset);
            let step = Oscillator::step(p);
            for x in out.iter_mut() {
                *x += osc.sample(p, osc.phase as f32) * p.volume;
                osc.phase += step;
            }
        }
        ModulationAlgo::SynchronizedBySubOsc => {
            // As the legacy program: the sub-oscillator does not play, but its own sub-oscillators do (into what this
            // one then writes over), and this one restarts its period with each of the sub's
            let (sub, sub_subs) = subs.split_first_mut().unwrap_or_else(|| unreachable!());
            if !sub_subs.is_empty() {
                update(sub_subs, &sub_params[1..], out, false);
            }
            sub.recalc_phase(sub_params[0].phase_offset);
            let sub_coeff = Oscillator::step(&sub_params[0]);
            osc.recalc_phase(p.phase_offset);
            let step = Oscillator::step(p);
            for x in out.iter_mut() {
                let before = sub.phase;
                sub.phase += sub_coeff;
                if sub.phase.floor() > before.floor() {
                    osc.phase = f64::from(osc.phase_offset);
                }
                *x = osc.sample(p, osc.phase as f32) * p.volume;
                osc.phase += step;
            }
        }
        ModulationAlgo::FrequencyModulation => {
            update(subs, sub_params, out, true);
            osc.recalc_phase(p.phase_offset);
            let step = Oscillator::step(p);
            let correction = 44100.0 / osc.sample_rate;
            if p.use_wave_table {
                // The table for the highest pitch the wave reaches: its own and the modulation's push (and the
                // sidebands, Carson's rule)
                let mut fastest = step.abs();
                for &m in out.iter() {
                    fastest = fastest.max((step + f64::from(m * correction)).abs());
                }
                osc.set_band_for_step((fastest + sub_step) as f32);
            }
            for x in out.iter_mut() {
                osc.phase += f64::from(*x * correction);
                *x = osc.sample(p, osc.phase as f32) * p.volume;
                osc.phase += step;
            }
        }
    }
}

/// With no sub-oscillator there is nothing to work with: the wave alone
fn update_alone(osc: &mut Oscillator, p: &OscParams<'_>, out: &mut [f32]) {
    osc.recalc_phase(p.phase_offset);
    let step = Oscillator::step(p);
    for x in out.iter_mut() {
        *x = osc.sample(p, osc.phase as f32) * p.volume;
        osc.phase += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(shape: WaveShape, freq: f32, tables: bool) -> OscParams<'static> {
        OscParams {
            shape,
            algo: ModulationAlgo::SignalMix,
            freq,
            detuning: 1.0 / 44100.0,
            phase_offset: 0.0,
            volume: 1.0,
            use_wave_table: tables,
            user_wave: None,
        }
    }

    #[test]
    fn bands_follow_midi_keys() {
        assert_eq!(wave_table_band_from_freq(440.0), 69);
        assert_eq!(wave_table_band_from_freq(441.0), 70);
        assert_eq!(wave_table_band_from_freq(1.0), 1);
        assert!((freq_from_wave_table_band(69) - 440.0).abs() < 1e-3);
    }

    #[test]
    fn tables_match_the_naive_waves_at_low_pitches() {
        let tables = wave_tables();
        for (shape, naive) in [
            (WaveShape::Saw, saw_sample as fn(f32) -> f32),
            (WaveShape::Square, square_sample),
            (WaveShape::Triangle, triangle_sample),
            (WaveShape::MoogSaw, moog_saw_sample),
            (WaveShape::Exponential, exp_sample),
        ] {
            let table = &tables[shape.table().unwrap()];
            // Away from the discontinuities, band 1 (8 Hz: all audible harmonics) is close to the naive shape
            let x = 0.1;
            let error = (table.sample(1, x) - naive(x)).abs();
            assert!(error < 0.05, "{shape:?}: {error}");
        }
    }

    #[test]
    fn a_sine_is_a_sine() {
        let mut oscs = [Oscillator::new(0.0, 44100.0, 1)];
        let mut out = vec![0.0f32; 441];
        update(&mut oscs, &[params(WaveShape::Sine, 100.0, true)], &mut out, false);
        assert!(out[0].abs() < 1e-6);
        assert!((out[110] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn noise_is_bounded_and_seeded() {
        let mut a = NoiseRng::new(7);
        let mut b = NoiseRng::new(7);
        for _ in 0..10000 {
            let x = a.sample();
            assert!((-1.0..=1.0).contains(&x));
            assert_eq!(x, b.sample());
        }
    }

    #[test]
    fn a_phase_just_below_a_period_reads_the_table_start() {
        let tables = wave_tables();
        let saw = &tables[WaveShape::Saw.table().unwrap()];
        for x in [-1.0e-9f32, 0.999_999_97, -0.000_000_01] {
            let v = saw.sample(40, x);
            assert!(v.is_finite());
        }
        assert_eq!(saw.sample(40, -1.0e-9).to_bits(), saw.sample(40, 0.0).to_bits());
    }

    #[test]
    fn keys_round_trip() {
        for s in WaveShape::ALL {
            assert_eq!(WaveShape::from_key(s.key()), Some(s));
        }
        for a in ModulationAlgo::ALL {
            assert_eq!(ModulationAlgo::from_key(a.key()), Some(a));
        }
    }
}

#[cfg(test)]
mod band_limit_tests {
    use super::*;

    #[test]
    fn an_infinite_frequency_picks_the_last_band_without_overflow() {
        let last = WAVE_TABLES_PER_WAVEFORM - 1;
        assert_eq!(wave_table_band_from_freq(f32::INFINITY), last);
        assert!(wave_table_band_from_freq(f32::NAN) >= 1);
        assert_eq!(wave_table_band_from_freq(0.0), 1);
    }
}
