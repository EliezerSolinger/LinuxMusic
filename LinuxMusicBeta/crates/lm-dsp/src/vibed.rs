//! The legacy program's Vibed: up to nine plucked strings (Karplus-Strong waveguides of two delay lines, oversampled),
//! each on a harmonic of the note, struck with a drawn impulse, panned and mixed.

use crate::math::LegacyRand;

/// The strings of the instrument
pub const STRINGS: usize = 9;
/// The points of a string's drawn impulse
pub const IMPULSE_LENGTH: usize = 128;
/// The harmonics a string can sit on, over the note
pub const OCTAVES: [f32; 9] = [0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
/// The rate the strings' oversampling is set for: above it, less
pub const BASE_SAMPLE_RATE: u32 = 44100;

/// The legacy `cubicInterpolate`
#[inline]
fn cubic(v0: f32, v1: f32, v2: f32, v3: f32, x: f32) -> f32 {
    let frsq = x * x;
    let frcu = frsq * v0;
    let t1 = v1 * 3.0 + v3;
    v1 + (0.5 * frcu + x) * (v2 - frcu * (1.0 / 6.0) - (t1 * (1.0 / 6.0) - v0) * (1.0 / 3.0))
        + frsq * x * (t1 * (1.0 / 6.0) - 0.5 * v2)
        + frsq * (0.5 * v2 - v1)
}

/// A delay line read and written through a moving index
#[derive(Clone, Debug)]
struct DelayLine {
    data: Vec<f32>,
    pointer: usize,
}

impl DelayLine {
    /// "position" samples from the pointer, wrapped
    #[inline]
    fn access(&self, position: i32) -> f32 {
        let len = self.data.len() as i32;
        let mut at = self.pointer as i32 + position;
        while at < 0 {
            at += len;
        }
        while at > len - 1 {
            at -= len;
        }
        self.data[at as usize]
    }
}

/// One string
#[derive(Clone, Debug)]
pub struct VibratingString {
    from_bridge: DelayLine,
    to_bridge: DelayLine,
    pickup: i32,
    oversample: usize,
    string_loss: f32,
    choice: usize,
    state: f32,
    out: Vec<f32>,
}

impl VibratingString {
    /// A string at @p pitch Hz, picked at @p pick and heard at @p pickup (fractions of its length), struck with
    /// @p impulse; @p oversample the length knob (1 to 16), @p randomize the fuzziness, @p stiffness its loss,
    /// @p detune its length's change, @p state the impulse as it is (not stretched over the string); its randomness
    /// from @p rand
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pitch: f32,
        pick: f32,
        pickup: f32,
        impulse: &[f32],
        sample_rate: u32,
        oversample: i32,
        randomize: f32,
        stiffness: f32,
        detune: f32,
        state: bool,
        rand: &mut LegacyRand,
    ) -> Self {
        // Twice the length's oversampling at the base rate, less at the higher ones; never none
        let oversample = (2 * oversample / (sample_rate / BASE_SAMPLE_RATE).max(1) as i32).max(1);
        let choice = rand.below_int(oversample).clamp(0, oversample - 1) as usize;
        let mut length = ((oversample as u32 * sample_rate) as f32 / pitch) as i32 + 1;
        length += (length as f32 * -detune) as i32;
        let length = length.max(1);
        let pick_at = (length as f32 * pick).ceil() as i32;

        let impulse: Vec<f32> = if state {
            impulse.to_vec()
        } else {
            // The impulse stretched over the string
            let src = impulse.len() as i32;
            (0..length)
                .map(|frame| {
                    let at = frame as f32 * src as f32 / length as f32;
                    let frac = at - (at as i32) as f32;
                    let i = (at as i32).clamp(1, src - 3) as usize;
                    cubic(impulse[i - 1], impulse[i], impulse[i + 1], impulse[i + 2], frac)
                })
                .collect()
        };

        let amount = randomize / 2.0 - randomize;
        let line = |rand: &mut LegacyRand| DelayLine {
            data: (0..length).map(|_| rand.inclusive(amount)).collect(),
            pointer: 0,
        };
        let mut to_bridge = line(rand);
        let mut from_bridge = line(rand);
        for dl in [&mut to_bridge, &mut from_bridge] {
            set_delay_line(dl, pick_at, &impulse, IMPULSE_LENGTH as i32, 0.5, state, amount, rand);
        }
        Self {
            from_bridge,
            to_bridge,
            pickup: (pickup * length as f32) as i32,
            oversample: oversample as usize,
            string_loss: 1.0 - stiffness,
            choice,
            state: 0.1,
            out: vec![0.0; oversample as usize],
        }
    }

    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        for i in 0..self.oversample {
            // What the pickup hears
            self.out[i] = self.from_bridge.access(self.pickup) + self.to_bridge.access(self.pickup);
            // Into the bridge, and to the nut
            let ym0 = self.to_bridge.access(1);
            let yp_m = self.from_bridge.access(self.from_bridge.data.len() as i32 - 2);
            // The bridge reflects it, averaged with the last (a low-pass)
            self.state = (f64::from(self.state + ym0) * 0.5) as f32;
            let reflected = -self.state;
            // Back from the bridge: the pointer moves, then the sample
            let len = self.from_bridge.data.len();
            self.from_bridge.pointer =
                if self.from_bridge.pointer == 0 { len - 1 } else { self.from_bridge.pointer - 1 };
            let at = self.from_bridge.pointer;
            self.from_bridge.data[at] = reflected * self.string_loss;
            // To the bridge: the sample, then the pointer moves
            let at = self.to_bridge.pointer;
            self.to_bridge.data[at] = -yp_m * self.string_loss;
            self.to_bridge.pointer = if at + 1 > len - 1 { 0 } else { at + 1 };
        }
        self.out[self.choice]
    }
}

/// The impulse into a delay line at the pick (the legacy `setDelayLine`)
#[allow(clippy::too_many_arguments)]
fn set_delay_line(
    dl: &mut DelayLine,
    pick: i32,
    values: &[f32],
    len: i32,
    scale: f32,
    state: bool,
    amount: f32,
    rand: &mut LegacyRand,
) {
    let length = dl.data.len() as i32;
    let pick = pick.clamp(0, length);
    if !state {
        for i in 0..pick {
            dl.data[i as usize] = scale * values[(length - i - 1) as usize] + rand.inclusive(amount);
        }
        for i in pick..length {
            dl.data[i as usize] = scale * values[(i - pick) as usize] + rand.inclusive(amount);
        }
    } else if len + pick > length {
        for i in pick..length {
            dl.data[i as usize] = scale * values[(i - pick) as usize] + rand.inclusive(amount);
        }
    } else {
        for i in 0..len {
            dl.data[(i + pick) as usize] = scale * values[i as usize] + rand.inclusive(amount);
        }
    }
}

/// One string's knobs
#[derive(Clone, Debug, PartialEq)]
pub struct VibedString {
    pub enabled: bool,
    /// Index into [`OCTAVES`]
    pub harmonic: usize,
    /// 0 to 0.05
    pub pick: f32,
    /// 0 to 0.05
    pub pickup: f32,
    /// 0 to 0.05
    pub stiffness: f32,
    /// 0 to 200
    pub volume: f32,
    /// -1 to 1
    pub pan: f32,
    /// -0.1 to 0.1
    pub detune: f32,
    /// 0 to 0.75
    pub fuzziness: f32,
    /// 1 to 16
    pub length: i32,
    /// The impulse as drawn, not stretched over the string
    pub impulse: bool,
    pub shape: [f32; IMPULSE_LENGTH],
}

impl Default for VibedString {
    fn default() -> Self {
        Self {
            enabled: false,
            harmonic: 2,
            pick: 0.0,
            pickup: 0.05,
            stiffness: 0.0,
            volume: 100.0,
            pan: 0.0,
            detune: 0.0,
            fuzziness: 0.0,
            length: 1,
            impulse: false,
            shape: core::array::from_fn(|i| {
                crate::math::sinf(i as f32 / IMPULSE_LENGTH as f32 * 2.0 * crate::math::f32c::PI)
            }),
        }
    }
}

/// One note of the Vibed: its strings
#[derive(Clone, Debug)]
pub struct VibedVoice {
    strings: Vec<(usize, VibratingString)>,
}

impl VibedVoice {
    /// A note at @p frequency of the enabled @p strings; their randomness from @p seed
    pub fn new(strings: &[VibedString], frequency: f32, sample_rate: u32, seed: u64) -> Self {
        let mut rand = LegacyRand::new(seed);
        let strings = strings
            .iter()
            .enumerate()
            .filter(|(_, s)| s.enabled)
            .map(|(i, s)| {
                let string = VibratingString::new(
                    frequency * OCTAVES[s.harmonic.min(OCTAVES.len() - 1)],
                    s.pick,
                    s.pickup,
                    &s.shape,
                    sample_rate,
                    s.length,
                    s.fuzziness,
                    s.stiffness,
                    s.detune,
                    s.impulse,
                    &mut rand,
                );
                (i, string)
            })
            .collect();
        Self { strings }
    }

    /// The next @p out frames (before the fade-in, which the host's note applies)
    pub fn render(&mut self, out: &mut [[f32; 2]], strings: &[VibedString]) {
        for frame in out.iter_mut() {
            *frame = [0.0; 2];
            for (i, string) in &mut self.strings {
                let s = &strings[*i];
                // 0: left, 1: right
                let pan = (s.pan + 1.0) / 2.0;
                let sample = string.next_sample() * s.volume / 100.0;
                frame[0] += (1.0 - pan) * sample;
                frame[1] += pan * sample;
            }
        }
    }
}
