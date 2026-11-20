//! STK's generators and filters (`Envelope`, `ADSR`, `SineWave`, `FileWvIn`, `FileLoop`, `BiQuad`, `OnePole`,
//! `TwoZero`, `DelayL`, `BowTable`), each computing what the library computes, in the same order. The sample rate is
//! an argument where the library read its global one. Warnings the library prints and ignores are ignored; where it
//! returns early on a bad argument, so does the port.

use std::sync::LazyLock;

use lm_dsp::math::{cos, pow, sin};

use super::TWO_PI;
use super::rawwaves::Wave;

// ---------------------------------------------------------------------------------------------------------------
// Envelope

/// A linear ramp to a target (`Envelope`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    value: f64,
    target: f64,
    rate: f64,
    state: bool,
}

impl Default for Envelope {
    fn default() -> Self {
        Self { value: 0.0, target: 0.0, rate: 0.001, state: false }
    }
}

impl Envelope {
    pub fn set_rate(&mut self, rate: f64) {
        if rate < 0.0 {
            return;
        }
        self.rate = rate;
    }

    pub fn set_target(&mut self, target: f64) {
        self.target = target;
        if self.value != self.target {
            self.state = true;
        }
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        if self.state {
            if self.target > self.value {
                self.value += self.rate;
                if self.value >= self.target {
                    self.value = self.target;
                    self.state = false;
                }
            } else {
                self.value -= self.rate;
                if self.value <= self.target {
                    self.value = self.target;
                    self.state = false;
                }
            }
        }
        self.value
    }
}

// ---------------------------------------------------------------------------------------------------------------
// ADSR

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdsrState {
    Attack,
    Decay,
    Sustain,
    Release,
    Idle,
}

/// Attack, decay, sustain, release (`ADSR`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adsr {
    state: AdsrState,
    value: f64,
    target: f64,
    attack_rate: f64,
    decay_rate: f64,
    release_rate: f64,
    release_time: f64,
    sustain_level: f64,
}

impl Default for Adsr {
    fn default() -> Self {
        Self {
            state: AdsrState::Idle,
            value: 0.0,
            target: 0.0,
            attack_rate: 0.001,
            decay_rate: 0.001,
            release_rate: 0.005,
            release_time: -1.0,
            sustain_level: 0.5,
        }
    }
}

impl Adsr {
    pub fn key_on(&mut self) {
        if self.target <= 0.0 {
            self.target = 1.0;
        }
        self.state = AdsrState::Attack;
    }

    /// (the legacy Mallets never releases a voice: a note ends when the instrument ends it)
    pub fn key_off(&mut self, rate: f64) {
        self.target = 0.0;
        self.state = AdsrState::Release;
        if self.release_time > 0.0 {
            self.release_rate = self.value / (self.release_time * rate);
        }
    }

    pub fn set_attack_rate(&mut self, rate: f64) {
        if rate < 0.0 {
            return;
        }
        self.attack_rate = rate;
    }

    pub fn set_sustain_level(&mut self, level: f64) {
        if level < 0.0 {
            return;
        }
        self.sustain_level = level;
    }

    fn set_attack_time(&mut self, time: f64, rate: f64) {
        if time <= 0.0 {
            return;
        }
        self.attack_rate = 1.0 / (time * rate);
    }

    fn set_decay_time(&mut self, time: f64, rate: f64) {
        if time <= 0.0 {
            return;
        }
        self.decay_rate = (1.0 - self.sustain_level) / (time * rate);
    }

    fn set_release_time(&mut self, time: f64, rate: f64) {
        if time <= 0.0 {
            return;
        }
        self.release_rate = self.sustain_level / (time * rate);
        self.release_time = time;
    }

    /// `setAllTimes` at `rate` Hz
    pub fn set_all_times(&mut self, a: f64, d: f64, s: f64, r: f64, rate: f64) {
        self.set_attack_time(a, rate);
        self.set_sustain_level(s);
        self.set_decay_time(d, rate);
        self.set_release_time(r, rate);
    }

    pub fn set_target(&mut self, target: f64) {
        if target < 0.0 {
            return;
        }
        self.target = target;
        self.set_sustain_level(self.target);
        if self.value < self.target {
            self.state = AdsrState::Attack;
        }
        if self.value > self.target {
            self.state = AdsrState::Decay;
        }
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        match self.state {
            AdsrState::Attack => {
                self.value += self.attack_rate;
                if self.value >= self.target {
                    self.value = self.target;
                    self.target = self.sustain_level;
                    self.state = AdsrState::Decay;
                }
            }
            AdsrState::Decay => {
                if self.value > self.sustain_level {
                    self.value -= self.decay_rate;
                    if self.value <= self.sustain_level {
                        self.value = self.sustain_level;
                        self.state = AdsrState::Sustain;
                    }
                } else {
                    self.value += self.decay_rate;
                    if self.value >= self.sustain_level {
                        self.value = self.sustain_level;
                        self.state = AdsrState::Sustain;
                    }
                }
            }
            AdsrState::Release => {
                self.value -= self.release_rate;
                if self.value <= 0.0 {
                    self.value = 0.0;
                    self.state = AdsrState::Idle;
                }
            }
            AdsrState::Sustain | AdsrState::Idle => {}
        }
        self.value
    }
}

// ---------------------------------------------------------------------------------------------------------------
// SineWave

/// The length of the sine table (`TABLE_SIZE`)
pub const TABLE_SIZE: usize = 2048;

/// The library's sine table: one period and the first sample again
pub fn sine_table() -> &'static [f64] {
    static T: LazyLock<Box<[f64]>> = LazyLock::new(|| {
        let temp = 1.0 / TABLE_SIZE as f64;
        (0..=TABLE_SIZE).map(|i| sin(TWO_PI * i as f64 * temp)).collect()
    });
    &T
}

/// A sine from a table with linear interpolation (`SineWave`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SineWave {
    time: f64,
    rate: f64,
}

impl Default for SineWave {
    fn default() -> Self {
        Self { time: 0.0, rate: 1.0 }
    }
}

impl SineWave {
    pub fn set_frequency(&mut self, frequency: f64, rate: f64) {
        self.rate = TABLE_SIZE as f64 * frequency / rate;
    }

    #[inline]
    pub fn tick(&mut self, table: &[f64]) -> f64 {
        if !self.time.is_finite() {
            // (the library would loop for ever)
            self.time = 0.0;
        }
        while self.time < 0.0 {
            self.time += TABLE_SIZE as f64;
        }
        while self.time >= TABLE_SIZE as f64 {
            self.time -= TABLE_SIZE as f64;
        }
        let i = self.time as usize;
        let alpha = self.time - i as f64;
        let mut tmp = table[i];
        tmp += alpha * (table[i + 1] - tmp);
        self.time += self.rate;
        tmp
    }
}

/// `StkFrames::interpolate` of one channel
#[inline]
fn interpolate(data: &[f64], frame: f64) -> f64 {
    let i = frame as usize;
    let alpha = frame - i as f64;
    let mut output = data[i];
    if alpha > 0.0 {
        output += alpha * (data[i + 1] - output);
    }
    output
}

// ---------------------------------------------------------------------------------------------------------------
// FileWvIn

/// A wave played once (`FileWvIn`, the whole file in memory)
#[derive(Clone, Copy, Debug)]
pub struct WvIn {
    wave: &'static Wave,
    time: f64,
    rate: f64,
    interpolate: bool,
    finished: bool,
}

impl WvIn {
    /// Opened at `rate` Hz (`openFile`: the file's rate over the output's, then `reset`)
    pub fn open(wave: &'static Wave, rate: f64) -> Self {
        let mut w = Self { wave, time: 0.0, rate: 0.0, interpolate: false, finished: true };
        w.set_rate(super::rawwaves::RAW_RATE / rate);
        w.reset();
        w
    }

    pub fn reset(&mut self) {
        self.time = 0.0;
        self.finished = false;
    }

    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        if self.rate < 0.0 && self.time == 0.0 {
            self.time = self.wave.file_size as f64 - 1.0;
        }
        self.interpolate = self.rate % 1.0 != 0.0;
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        if self.finished {
            return 0.0;
        }
        if self.time < 0.0 || self.time > self.wave.file_size as f64 - 1.0 || self.time.is_nan() {
            self.finished = true;
            return 0.0;
        }
        let out =
            if self.interpolate { interpolate(&self.wave.data, self.time) } else { self.wave.data[self.time as usize] };
        self.time += self.rate;
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// FileLoop

/// A wave played in a loop (`FileLoop`, the whole file in memory)
#[derive(Clone, Copy, Debug)]
pub struct FileLoop {
    wave: &'static Wave,
    time: f64,
    rate: f64,
    interpolate: bool,
    phase_offset: f64,
}

impl FileLoop {
    /// Opened at `rate` Hz
    pub fn open(wave: &'static Wave, rate: f64) -> Self {
        let mut w = Self { wave, time: 0.0, rate: 0.0, interpolate: false, phase_offset: 0.0 };
        w.set_rate(super::rawwaves::RAW_RATE / rate);
        w
    }

    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        self.interpolate = self.rate % 1.0 != 0.0;
    }

    /// `setFrequency` at `rate` Hz
    pub fn set_frequency(&mut self, frequency: f64, rate: f64) {
        self.set_rate(self.wave.file_size as f64 * frequency / rate);
    }

    pub fn add_phase_offset(&mut self, angle: f64) {
        self.phase_offset = self.wave.file_size as f64 * angle;
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        let size = self.wave.file_size as f64;
        if !self.time.is_finite() {
            // (the library would loop for ever)
            self.time = 0.0;
        }
        while self.time < 0.0 {
            self.time += size;
        }
        while self.time >= size {
            self.time -= size;
        }
        let mut tyme = self.time;
        if self.phase_offset != 0.0 {
            if !self.phase_offset.is_finite() {
                self.phase_offset = 0.0;
            }
            tyme += self.phase_offset;
            while tyme < 0.0 {
                tyme += size;
            }
            while tyme >= size {
                tyme -= size;
            }
        }
        let out = if self.interpolate { interpolate(&self.wave.data, tyme) } else { self.wave.data[tyme as usize] };
        self.time += self.rate;
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// BiQuad

/// Two poles, two zeros (`BiQuad`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiQuad {
    b: [f64; 3],
    a: [f64; 3],
    inputs: [f64; 3],
    outputs: [f64; 3],
    gain: f64,
    last: f64,
}

impl Default for BiQuad {
    fn default() -> Self {
        Self { b: [1.0, 0.0, 0.0], a: [1.0, 0.0, 0.0], inputs: [0.0; 3], outputs: [0.0; 3], gain: 1.0, last: 0.0 }
    }
}

impl BiQuad {
    pub fn set_gain(&mut self, gain: f64) {
        self.gain = gain;
    }

    /// `setResonance` at `rate` Hz
    pub fn set_resonance(&mut self, frequency: f64, radius: f64, normalize: bool, rate: f64) {
        self.a[2] = radius * radius;
        self.a[1] = -2.0 * radius * cos(TWO_PI * frequency / rate);
        if normalize {
            self.b[0] = 0.5 - 0.5 * self.a[2];
            self.b[1] = 0.0;
            self.b[2] = -self.b[0];
        } else {
            self.b[0] = 1.0;
            self.b[1] = 0.0;
            self.b[2] = 0.0;
        }
    }

    pub fn set_equal_gain_zeroes(&mut self) {
        self.b[0] = 1.0;
        self.b[1] = 0.0;
        self.b[2] = -1.0;
    }

    pub fn clear(&mut self) {
        self.inputs = [0.0; 3];
        self.outputs = [0.0; 3];
        self.last = 0.0;
    }

    pub fn last_out(&self) -> f64 {
        self.last
    }

    #[inline]
    pub fn tick(&mut self, input: f64) -> f64 {
        self.inputs[0] = self.gain * input;
        let mut out = self.b[0] * self.inputs[0] + self.b[1] * self.inputs[1] + self.b[2] * self.inputs[2];
        out -= self.a[2] * self.outputs[2] + self.a[1] * self.outputs[1];
        self.inputs[2] = self.inputs[1];
        self.inputs[1] = self.inputs[0];
        self.outputs[2] = self.outputs[1];
        self.outputs[1] = out;
        self.last = out;
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// OnePole

/// One pole (`OnePole`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnePole {
    b0: f64,
    a1: f64,
    gain: f64,
    output: f64,
}

impl Default for OnePole {
    fn default() -> Self {
        let mut f = Self { b0: 0.0, a1: 0.0, gain: 1.0, output: 0.0 };
        f.set_pole(0.9);
        f
    }
}

impl OnePole {
    pub fn set_pole(&mut self, pole: f64) {
        if pole.abs() >= 1.0 {
            return;
        }
        self.b0 = if pole > 0.0 { 1.0 - pole } else { 1.0 + pole };
        self.a1 = -pole;
    }

    pub fn clear(&mut self) {
        self.output = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, input: f64) -> f64 {
        let i0 = self.gain * input;
        let out = self.b0 * i0 - self.a1 * self.output;
        self.output = out;
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// TwoZero

/// Two zeros (`TwoZero`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoZero {
    b: [f64; 3],
    inputs: [f64; 3],
    gain: f64,
    last: f64,
}

impl Default for TwoZero {
    fn default() -> Self {
        Self { b: [1.0, 0.0, 0.0], inputs: [0.0; 3], gain: 1.0, last: 0.0 }
    }
}

impl TwoZero {
    pub fn set_b2(&mut self, b2: f64) {
        self.b[2] = b2;
    }

    pub fn set_gain(&mut self, gain: f64) {
        self.gain = gain;
    }

    pub fn last_out(&self) -> f64 {
        self.last
    }

    #[inline]
    pub fn tick(&mut self, input: f64) -> f64 {
        self.inputs[0] = self.gain * input;
        let out = self.b[2] * self.inputs[2] + self.b[1] * self.inputs[1] + self.b[0] * self.inputs[0];
        self.inputs[2] = self.inputs[1];
        self.inputs[1] = self.inputs[0];
        self.last = out;
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// DelayL

/// A delay line with linear interpolation (`DelayL`). The library's ring is 4096 samples long whatever the delay;
/// here it is `delay + 1` samples of a buffer allocated in the build, which plays the same samples (the output is the
/// input `delay` samples before, whatever the ring's length) and is cleared in `O(delay)`. The ring's length changes
/// only together with a clear (all the banded waveguide does), so nothing in it is ever read across the change
#[derive(Debug)]
pub struct DelayL {
    buf: Box<[f64]>,
    /// The ring's length (`inputs_.size()`)
    size: usize,
    in_point: usize,
    out_point: usize,
    delay: f64,
    alpha: f64,
    om_alpha: f64,
    next_output: f64,
    do_next_out: bool,
    last: f64,
}

impl Clone for DelayL {
    fn clone(&self) -> Self {
        Self { buf: self.buf.clone(), ..*self }
    }

    /// Without allocating when the two have the same capacity (the voices of one instrument): only the ring is copied
    fn clone_from(&mut self, source: &Self) {
        if self.buf.len() != source.buf.len() {
            *self = source.clone();
            return;
        }
        self.buf[..source.size].copy_from_slice(&source.buf[..source.size]);
        let buf = std::mem::take(&mut self.buf);
        *self = Self { buf, ..*source };
    }
}

impl DelayL {
    /// A line that can delay up to `capacity - 1` samples, at delay 0 (the library's constructor)
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(2);
        let mut d = Self {
            buf: vec![0.0; capacity].into_boxed_slice(),
            size: capacity,
            in_point: 0,
            out_point: 0,
            delay: 0.0,
            alpha: 0.0,
            om_alpha: 1.0,
            next_output: 0.0,
            do_next_out: true,
            last: 0.0,
        };
        d.set_delay(0.0);
        d
    }

    /// A line with no buffer (a placeholder while a voice is copied)
    pub fn empty() -> Self {
        Self {
            buf: Box::default(),
            size: 0,
            in_point: 0,
            out_point: 0,
            delay: 0.0,
            alpha: 0.0,
            om_alpha: 1.0,
            next_output: 0.0,
            do_next_out: true,
            last: 0.0,
        }
    }

    /// The longest delay it can take
    pub fn max_delay(&self) -> usize {
        self.buf.len() - 1
    }

    pub fn delay(&self) -> f64 {
        self.delay
    }

    /// `setDelay`, keeping the ring
    fn set_delay(&mut self, delay: f64) {
        if delay + 1.0 > self.size as f64 || delay < 0.0 {
            return;
        }
        let mut out_pointer = self.in_point as f64 - delay;
        self.delay = delay;
        while out_pointer < 0.0 {
            out_pointer += self.size as f64;
        }
        self.out_point = out_pointer as usize;
        self.alpha = out_pointer - self.out_point as f64;
        self.om_alpha = 1.0 - self.alpha;
        if self.out_point == self.size {
            self.out_point = 0;
        }
        self.do_next_out = true;
    }

    /// `setDelay(delay)` followed by `clear()`: the ring becomes `delay + 1` samples long and silent. Returns false,
    /// changing nothing, when the delay does not fit (the library's warning)
    pub fn set_delay_and_clear(&mut self, delay: f64) -> bool {
        if delay < 0.0 || delay + 1.0 > self.buf.len() as f64 {
            return false;
        }
        self.size = (delay.ceil() as usize + 1).min(self.buf.len());
        self.in_point = 0;
        self.set_delay(delay);
        self.clear();
        true
    }

    /// `clear` (the ring and the last output; the library leaves the interpolation's pending output, which the next
    /// tick works out again)
    pub fn clear(&mut self) {
        self.buf[..self.size].fill(0.0);
        self.last = 0.0;
    }

    pub fn last_out(&self) -> f64 {
        self.last
    }

    #[inline]
    fn next_out(&mut self) -> f64 {
        if self.do_next_out {
            self.next_output = self.buf[self.out_point] * self.om_alpha;
            if self.out_point + 1 < self.size {
                self.next_output += self.buf[self.out_point + 1] * self.alpha;
            } else {
                self.next_output += self.buf[0] * self.alpha;
            }
            self.do_next_out = false;
        }
        self.next_output
    }

    #[inline]
    pub fn tick(&mut self, input: f64) -> f64 {
        self.buf[self.in_point] = input;
        self.in_point += 1;
        if self.in_point == self.size {
            self.in_point = 0;
        }
        self.last = self.next_out();
        self.do_next_out = true;
        self.out_point += 1;
        if self.out_point == self.size {
            self.out_point = 0;
        }
        self.last
    }
}

// ---------------------------------------------------------------------------------------------------------------
// BowTable

/// The bow's friction (`BowTable`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BowTable {
    offset: f64,
    slope: f64,
    min_output: f64,
    max_output: f64,
}

impl Default for BowTable {
    fn default() -> Self {
        Self { offset: 0.0, slope: 0.1, min_output: 0.01, max_output: 0.98 }
    }
}

impl BowTable {
    pub fn set_slope(&mut self, slope: f64) {
        self.slope = slope;
    }

    #[inline]
    pub fn tick(&self, input: f64) -> f64 {
        let mut sample = input + self.offset;
        sample *= self.slope;
        let mut out = sample.abs() + 0.75;
        out = pow(out, -4.0);
        if out < self.min_output {
            out = self.min_output;
        }
        if out > self.max_output {
            out = self.max_output;
        }
        out
    }
}
