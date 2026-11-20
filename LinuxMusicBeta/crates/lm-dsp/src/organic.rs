//! The legacy program's Organic: eight oscillators on the harmonics of the note, mixed (each the next one's mix),
//! each panned and detuned between the sides, through a soft waveshaper; band limited.

use crate::oscillator::{ModulationAlgo, NoiseRng, OscParams, Oscillator, WaveShape, update};

/// The oscillators of a note
pub const OSCILLATORS: usize = 8;
/// The harmonics an oscillator can play: its frequency over the note's
pub const HARMONICS: [f64; 18] =
    [0.5, 0.75, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0];
/// The shapes an oscillator can play, in the order of its knob
pub const SHAPES: [WaveShape; 6] = [
    WaveShape::Sine,
    WaveShape::Saw,
    WaveShape::Square,
    WaveShape::Triangle,
    WaveShape::MoogSaw,
    WaveShape::Exponential,
];

/// One oscillator's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrganicOsc {
    pub shape: WaveShape,
    /// Index into [`HARMONICS`]
    pub harmonic: usize,
    /// 0 to 100
    pub volume: f32,
    /// -100 (left) to 100 (right)
    pub pan: f32,
    /// Cents, -1200 to 1200: the left that much higher, the right that much lower
    pub detune: f32,
}

/// The Organic's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrganicSettings {
    pub oscillators: [OrganicOsc; OSCILLATORS],
    /// 0 to 0.99: the waveshaper's amount
    pub distortion: f32,
    /// 0 to 200
    pub volume: f32,
}

impl Default for OrganicSettings {
    fn default() -> Self {
        Self {
            oscillators: core::array::from_fn(|i| OrganicOsc {
                shape: WaveShape::Sine,
                harmonic: i,
                volume: 100.0,
                pan: 0.0,
                detune: 0.0,
            }),
            distortion: 0.0,
            volume: 100.0,
        }
    }
}

/// One oscillator's values for each side: its volume and its detuning over the sample rate
fn side_values(o: &OrganicOsc, sample_rate: f32) -> [(f32, f64); 2] {
    let count = OSCILLATORS as f32;
    let left_volume = (1.0 - o.pan / 100.0) * o.volume / count / 100.0;
    let right_volume = (1.0 + o.pan / 100.0) * o.volume / count / 100.0;
    let ratio = HARMONICS[o.harmonic.min(HARMONICS.len() - 1)];
    let sr = f64::from(sample_rate as i32);
    let detune = f64::from(o.detune) * f64::from(1.0f32 / 1200.0);
    [(left_volume, ratio * crate::math::exp2(detune) / sr), (right_volume, ratio * crate::math::exp2(-detune) / sr)]
}

impl Clone for OrganicVoice {
    fn clone(&self) -> Self {
        Self { sides: self.sides.clone(), phase_offsets: self.phase_offsets, scratch: self.scratch.clone() }
    }

    fn clone_from(&mut self, source: &Self) {
        for (a, b) in self.sides.iter_mut().zip(&source.sides) {
            a.clone_from(b);
        }
        self.phase_offsets = source.phase_offsets;
        self.scratch.clone_from(&source.scratch);
    }
}

/// The legacy Organic's soft waveshaper
#[inline]
pub fn waveshape(x: f32, amount: f32) -> f32 {
    let k = 2.0 * amount / (1.0 - amount);
    (1.0 + k) * x / (1.0 + k * x.abs())
}

/// One note of the Organic. Made once with its room; a note's start ([`OrganicVoice::restart`]), a render and
/// `clone_from` never allocate (for the audio thread)
#[derive(Debug)]
pub struct OrganicVoice {
    sides: [Vec<Oscillator>; 2],
    phase_offsets: [[f32; OSCILLATORS]; 2],
    scratch: Vec<f32>,
}

impl OrganicVoice {
    /// A note at @p sample_rate, for periods of up to @p max_period frames; the oscillators start at random phases
    /// from @p seed
    pub fn new(sample_rate: f32, max_period: usize, seed: u64) -> Self {
        Self::with_phases(sample_rate, max_period, Self::random_phases(seed))
    }

    fn random_phases(seed: u64) -> [[f32; OSCILLATORS]; 2] {
        let mut rng = NoiseRng::new(seed);
        let mut phase_offsets = [[0.0f32; OSCILLATORS]; 2];
        // (the last one first, as the legacy program drew them)
        for i in (0..OSCILLATORS).rev() {
            phase_offsets[0][i] = rng.unit() as f32;
            phase_offsets[1][i] = rng.unit() as f32;
        }
        phase_offsets
    }

    /// A new note in this voice, as [`OrganicVoice::new`] makes one, without allocating
    pub fn restart(&mut self, sample_rate: f32, seed: u64) {
        self.phase_offsets = Self::random_phases(seed);
        for side in 0..2 {
            for (i, osc) in self.sides[side].iter_mut().enumerate() {
                *osc = Oscillator::new(self.phase_offsets[side][i], sample_rate, 0);
            }
        }
    }

    /// A note whose oscillators start at @p phase_offsets (left, right)
    pub fn with_phases(sample_rate: f32, max_period: usize, phase_offsets: [[f32; OSCILLATORS]; 2]) -> Self {
        let sides = core::array::from_fn(|side| {
            (0..OSCILLATORS).map(|i| Oscillator::new(phase_offsets[side][i], sample_rate, 0)).collect()
        });
        Self { sides, phase_offsets, scratch: vec![0.0; max_period] }
    }

    /// The next @p out frames of the note at @p frequency (before the fade-in, which the host's note applies)
    pub fn render(&mut self, out: &mut [[f32; 2]], s: &OrganicSettings, frequency: f32, sample_rate: f32) {
        let values = s.oscillators.map(|o| side_values(&o, sample_rate));
        let n = out.len();
        for side in 0..2 {
            // (an array: a render never allocates)
            let params: [OscParams<'_>; OSCILLATORS] = core::array::from_fn(|i| OscParams {
                shape: s.oscillators[i].shape,
                algo: ModulationAlgo::SignalMix,
                freq: frequency,
                detuning: values[i][side].1,
                phase_offset: self.phase_offsets[side][i],
                volume: values[i][side].0,
                use_wave_table: true,
                user_wave: None,
            });
            let mono = &mut self.scratch[..n];
            update(&mut self.sides[side], &params, mono, false);
            for (frame, v) in out.iter_mut().zip(mono.iter()) {
                frame[side] = waveshape(*v, s.distortion) * s.volume / 100.0;
            }
        }
    }
}
