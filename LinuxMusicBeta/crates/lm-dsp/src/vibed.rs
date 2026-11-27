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
/// Pitch limits of a string, in Hz: below the lowest note the plugin makes rooms for (about 1.9 Hz) and above any
/// string that fits two samples
const MIN_PITCH: f32 = 1.0;
const MAX_PITCH: f32 = 1.0e6;
/// The longest string, in internal samples (the lowest pitch at the longest length knob is about 0.75 million)
const MAX_LENGTH: usize = 1 << 21;

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
#[derive(Debug)]
struct DelayLine {
    data: Vec<f32>,
    pointer: usize,
}

impl Clone for DelayLine {
    fn clone(&self) -> Self {
        let mut data = Vec::with_capacity(self.data.capacity());
        data.extend_from_slice(&self.data);
        Self { data, pointer: self.pointer }
    }

    /// Into its own room: no allocation when the room holds the source (the voices of one instrument share theirs)
    fn clone_from(&mut self, source: &Self) {
        self.data.clone_from(&source.data);
        self.pointer = source.pointer;
    }
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

/// One string. Made with room for its longest note ([`VibratingString::with_room`]): a note restarts it in that
/// room ([`VibratingString::restart`]) and `clone_from` copies into it, without allocating
#[derive(Debug)]
pub struct VibratingString {
    from_bridge: DelayLine,
    to_bridge: DelayLine,
    pickup: i32,
    string_loss: f32,
    state: f32,
    /// Internal samples per output sample: the string runs at its own rate (see [`VibratingString::restart`])
    steps: f64,
    /// The internal sample the next output sample is read at (fractional between two)
    target: f64,
    /// Internal samples made so far, and the last two (`cur` the newest)
    produced: u64,
    prev: f32,
    cur: f32,
    /// The impulse stretched over the string (scratch of a restart)
    stretched: Vec<f32>,
}

impl Clone for VibratingString {
    fn clone(&self) -> Self {
        let mut stretched = Vec::with_capacity(self.stretched.capacity());
        stretched.extend_from_slice(&self.stretched);
        Self {
            from_bridge: self.from_bridge.clone(),
            to_bridge: self.to_bridge.clone(),
            pickup: self.pickup,
            string_loss: self.string_loss,
            state: self.state,
            steps: self.steps,
            target: self.target,
            produced: self.produced,
            prev: self.prev,
            cur: self.cur,
            stretched,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.from_bridge.clone_from(&source.from_bridge);
        self.to_bridge.clone_from(&source.to_bridge);
        self.pickup = source.pickup;
        self.string_loss = source.string_loss;
        self.state = source.state;
        self.steps = source.steps;
        self.target = source.target;
        self.produced = source.produced;
        self.prev = source.prev;
        self.cur = source.cur;
        self.stretched.clone_from(&source.stretched);
    }
}

impl VibratingString {
    /// The length of a string at @p pitch Hz (its delay lines', in internal samples), as
    /// [`VibratingString::new`] makes it: the same at any sample rate
    pub fn length_for(pitch: f32, sample_rate: u32, oversample: i32, detune: f32) -> usize {
        let _ = sample_rate;
        Self::length_of(pitch, Self::oversample_for(oversample), detune)
    }

    /// The one place a string's length is computed, for `length_for` and `restart`. Whatever the pitch (zero, not
    /// finite, far below the lowest note, which would allocate and loop for megabytes) and the detune, it is
    /// between 1 and [`MAX_LENGTH`] and the arithmetic cannot overflow. Inside the knobs' ranges it is the legacy
    /// length exactly
    fn length_of(pitch: f32, oversample: i32, detune: f32) -> usize {
        let pitch = if pitch.is_finite() { pitch.clamp(MIN_PITCH, MAX_PITCH) } else { MIN_PITCH };
        let detune = if detune.is_finite() { detune.clamp(-1.0, 1.0) } else { 0.0 };
        let mut length = ((oversample as u32 * BASE_SAMPLE_RATE) as f32 / pitch) as i32 + 1;
        length += (length as f32 * -detune) as i32;
        (length.max(1) as usize).min(MAX_LENGTH)
    }

    /// The internal samples per sample of the base rate: twice the length knob (1 to 16 in the schema; limited
    /// here so the product cannot overflow)
    fn oversample_for(oversample: i32) -> i32 {
        oversample.clamp(0, 16).saturating_mul(2).max(1)
    }

    /// A silent string with room for delay lines of @p room samples (allocates: in the build)
    pub fn with_room(room: usize) -> Self {
        let room = room.max(IMPULSE_LENGTH);
        let line = || DelayLine { data: Vec::with_capacity(room), pointer: 0 };
        let mut s = Self {
            from_bridge: line(),
            to_bridge: line(),
            pickup: 0,
            string_loss: 1.0,
            state: 0.1,
            steps: 1.0,
            target: 0.0,
            produced: 0,
            prev: 0.0,
            cur: 0.0,
            stretched: Vec::with_capacity(room),
        };
        s.from_bridge.data.push(0.0);
        s.to_bridge.data.push(0.0);
        s
    }

    /// The samples its delay lines have room for
    pub fn room(&self) -> usize {
        self.to_bridge.data.capacity().min(self.from_bridge.data.capacity())
    }
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
        let mut s = Self::with_room(Self::length_for(pitch, sample_rate, oversample, detune));
        s.restart(pitch, pick, pickup, impulse, sample_rate, oversample, randomize, stiffness, detune, state, rand);
        s
    }

    /// A new note on this string, as [`VibratingString::new`] makes one, in the room it has: never allocates. A
    /// string longer than the room (a pitch below the lowest it was made for) is as long as the room
    #[allow(clippy::too_many_arguments)]
    pub fn restart(
        &mut self,
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
    ) {
        // The string runs at its own rate: twice the length knob times the base rate (44.1 kHz), the same at any
        // project rate, so it sounds the same at any (U36). An output sample is read every `steps` of its samples,
        // between two when that is not whole; at 44.1 kHz it is whole and the string is the legacy one, which ran
        // at that many times the project rate (fewer above 88.2 kHz) and so changed its decay and tone with the rate
        let oversample = Self::oversample_for(oversample);
        let choice = rand.below_int(oversample).clamp(0, oversample - 1);
        let length = Self::length_of(pitch, oversample, detune).min(self.room()) as i32;
        // (fractions of the length: a position outside the string is a position at its end)
        let (pick, pickup) = (finite_or(pick, 0.0).clamp(0.0, 1.0), finite_or(pickup, 0.0).clamp(0.0, 1.0));
        let pick_at = (length as f32 * pick).ceil() as i32;

        self.stretched.clear();
        if state {
            self.stretched.extend_from_slice(impulse);
        } else {
            // The impulse stretched over the string
            let src = impulse.len() as i32;
            self.stretched.extend((0..length).map(|frame| {
                let at = frame as f32 * src as f32 / length as f32;
                let frac = at - (at as i32) as f32;
                let i = (at as i32).clamp(1, src - 3) as usize;
                cubic(impulse[i - 1], impulse[i], impulse[i + 1], impulse[i + 2], frac)
            }));
        }

        let randomize = finite_or(randomize, 0.0).clamp(0.0, 1.0);
        let amount = randomize / 2.0 - randomize;
        for dl in [&mut self.to_bridge, &mut self.from_bridge] {
            dl.data.clear();
            dl.data.extend((0..length).map(|_| rand.inclusive(amount)));
            dl.pointer = 0;
        }
        for dl in [&mut self.to_bridge, &mut self.from_bridge] {
            set_delay_line(dl, pick_at, &self.stretched, IMPULSE_LENGTH as i32, 0.5, state, amount, rand);
        }
        self.pickup = (pickup * length as f32) as i32;
        self.string_loss = 1.0 - finite_or(stiffness, 0.0).clamp(0.0, 1.0);
        self.state = 0.1;
        // (the legacy read the `choice`-th of each output sample's internal samples)
        self.steps = f64::from(oversample) * f64::from(BASE_SAMPLE_RATE) / f64::from(sample_rate);
        self.target = f64::from(choice);
        self.produced = 0;
        self.prev = 0.0;
        self.cur = 0.0;
    }

    /// One internal sample: what the pickup hears, then the waves move on
    #[inline]
    fn step(&mut self) {
        // What the pickup hears
        let heard = self.from_bridge.access(self.pickup) + self.to_bridge.access(self.pickup);
        // Into the bridge, and to the nut
        let ym0 = self.to_bridge.access(1);
        let yp_m = self.from_bridge.access(self.from_bridge.data.len() as i32 - 2);
        // The bridge reflects it, averaged with the last (a low-pass)
        self.state = (f64::from(self.state + ym0) * 0.5) as f32;
        let reflected = -self.state;
        // Back from the bridge: the pointer moves, then the sample
        let len = self.from_bridge.data.len();
        self.from_bridge.pointer = if self.from_bridge.pointer == 0 { len - 1 } else { self.from_bridge.pointer - 1 };
        let at = self.from_bridge.pointer;
        self.from_bridge.data[at] = reflected * self.string_loss;
        // To the bridge: the sample, then the pointer moves
        let at = self.to_bridge.pointer;
        self.to_bridge.data[at] = -yp_m * self.string_loss;
        self.to_bridge.pointer = if at + 1 > len - 1 { 0 } else { at + 1 };
        self.prev = self.cur;
        self.cur = heard;
        self.produced += 1;
    }

    /// The next output sample
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        let floor = self.target.floor();
        let frac = (self.target - floor) as f32;
        // The internal samples up to the target (and the one after it, between two)
        let needed = floor as u64 + if frac > 0.0 { 2 } else { 1 };
        while self.produced < needed {
            self.step();
        }
        self.target += self.steps;
        if frac > 0.0 { self.prev + (self.cur - self.prev) * frac } else { self.cur }
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

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
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

/// One note of the Vibed: its strings, one slot for each, made with their room (see [`VibedVoice::rooms`]); a note
/// restarts them in it and `clone_from` copies into it, without allocating
#[derive(Debug)]
pub struct VibedVoice {
    strings: Vec<VibratingString>,
    playing: [bool; STRINGS],
}

impl Clone for VibedVoice {
    fn clone(&self) -> Self {
        Self { strings: self.strings.clone(), playing: self.playing }
    }

    fn clone_from(&mut self, source: &Self) {
        self.strings.clone_from(&source.strings);
        self.playing = source.playing;
    }
}

impl VibedVoice {
    /// A note at @p frequency of the enabled @p strings; their randomness from @p seed
    pub fn new(strings: &[VibedString], frequency: f32, sample_rate: u32, seed: u64) -> Self {
        let rooms: [usize; STRINGS] = core::array::from_fn(|i| {
            strings.get(i).map_or(1, |s| {
                let pitch = frequency * OCTAVES[s.harmonic.min(OCTAVES.len() - 1)];
                VibratingString::length_for(pitch, sample_rate, s.length, s.detune)
            })
        });
        let mut voice = Self::with_rooms(rooms);
        voice.restart(strings, frequency, sample_rate, seed);
        voice
    }

    /// The room each string needs for every note from @p lowest Hz up, with its harmonic and length, at any detune
    pub fn rooms(strings: &[VibedString], lowest: f32, sample_rate: u32) -> [usize; STRINGS] {
        core::array::from_fn(|i| {
            strings.get(i).map_or(1, |s| {
                let pitch = lowest * OCTAVES[s.harmonic.min(OCTAVES.len() - 1)];
                // (the detune's longest: −0.1 makes the string a tenth longer)
                VibratingString::length_for(pitch, sample_rate, s.length, -0.1)
            })
        })
    }

    /// A silent voice with these rooms (allocates: in the build)
    pub fn with_rooms(rooms: [usize; STRINGS]) -> Self {
        Self { strings: rooms.iter().map(|r| VibratingString::with_room(*r)).collect(), playing: [false; STRINGS] }
    }

    /// The room of each string
    pub fn room(&self) -> [usize; STRINGS] {
        core::array::from_fn(|i| self.strings[i].room())
    }

    /// A new note in this voice, as [`VibedVoice::new`] makes one, in its rooms: never allocates
    pub fn restart(&mut self, strings: &[VibedString], frequency: f32, sample_rate: u32, seed: u64) {
        let mut rand = LegacyRand::new(seed);
        for (i, string) in self.strings.iter_mut().enumerate() {
            let Some(s) = strings.get(i).filter(|s| s.enabled) else {
                self.playing[i] = false;
                continue;
            };
            self.playing[i] = true;
            string.restart(
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
        }
    }

    /// The next @p out frames (before the fade-in, which the host's note applies)
    pub fn render(&mut self, out: &mut [[f32; 2]], strings: &[VibedString]) {
        for frame in out.iter_mut() {
            *frame = [0.0; 2];
            for (i, string) in self.strings.iter_mut().enumerate() {
                if !self.playing[i] {
                    continue;
                }
                let s = &strings[i];
                // 0: left, 1: right
                let pan = (s.pan + 1.0) / 2.0;
                let sample = string.next_sample() * s.volume / 100.0;
                frame[0] += (1.0 - pan) * sample;
                frame[1] += pan * sample;
            }
        }
    }
}
