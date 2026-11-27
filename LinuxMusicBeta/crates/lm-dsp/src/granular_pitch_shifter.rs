//! The legacy program's Granular Pitch Shifter (Lost Robot's): grains read from a ring of the input at another
//! speed, windowed and overlapped, with feedback. It gives the wet signal: dry and wet are the host's.

use crate::math::{exp2, expf, f32c, hermite, tanf};
use crate::oscillator::{NoiseRng, lerp};
use crate::small_effects::Ramp;

/// The prefilter's cutoff, of the Nyquist frequency the pitch leaves
const PREFILTER_BANDWIDTH: f32 = 0.96;
/// Octaves: a gliding pitch this near its target is set to it
const GLIDE_SNAG_RADIUS: f64 = 0.001;
/// Frames a grain keeps behind the writing
const SAFETY_LATENCY: i32 = 3;
/// The ring's length for each range, s
pub const RANGE_SECONDS: [f32; 5] = [5.0, 10.0, 40.0, 40.0, 120.0];
const DC_REMOVAL_HZ: f32 = 7.0;
/// Above it the feedback is saturated
const SATURATION_SAFE: f32 = 16.0;
const SATURATION_STRENGTH: f32 = 0.001;

/// The Granular Pitch Shifter's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GranularSettings {
    /// Semitones, -48 to 24 (to 48 in the longer ranges) (sample-exact)
    pub pitch: Ramp,
    /// Semitones, -24 to 24: the right that much higher than the left (sample-exact)
    pub pitch_spread: Ramp,
    /// Hz: the grains' rate, 2 (4 in the shortest range) to 1000
    pub size: f32,
    /// s, 0 to 0.5 (to 40 in the longer ranges): how far back a grain may start, at random
    pub spray: f32,
    /// 0 to 1 (to 2): random grain speeds
    pub jitter: f32,
    /// 0 to 1: random waits between grains
    pub twitch: f32,
    /// 0 to 1: the right side's spray its own
    pub spray_spread: f32,
    /// 1 (equal gain) to 2 (equal power): the grains' crossfade
    pub shape: f32,
    /// 0.001 to 1
    pub fade_length: f32,
    /// 0 to 1
    pub feedback: f32,
    /// s, 0 to 1
    pub min_latency: f32,
    /// The input low-passed below what the pitch would push past Nyquist
    pub prefilter: bool,
    /// 1 to 16: grains at once
    pub density: f32,
    /// s, 0 to 1
    pub glide: f32,
    /// Index into [`RANGE_SECONDS`]: the ring's length (changing it empties the ring)
    pub range: usize,
}

impl Default for GranularSettings {
    fn default() -> Self {
        Self {
            pitch: Ramp::constant(1.0),
            pitch_spread: Ramp::constant(0.0),
            size: 10.0,
            spray: 0.005,
            jitter: 0.0,
            twitch: 0.0,
            spray_spread: 0.0,
            shape: 2.0,
            fade_length: 1.0,
            feedback: 0.0,
            min_latency: 0.01,
            prefilter: true,
            density: 1.0,
            glide: 0.01,
            range: 0,
        }
    }
}

/// The prefilter: a 2-pole low-pass (state variable, trapezoidal)
#[derive(Clone, Copy, Debug, Default)]
struct Prefilter {
    v0z: f32,
    v1: f32,
    v2: f32,
    g1: f32,
    g2: f32,
    g3: f32,
    g4: f32,
}

impl Prefilter {
    fn clear(&mut self) {
        self.v0z = 0.0;
        self.v1 = 0.0;
        self.v2 = 0.0;
    }

    fn set(&mut self, sample_rate: f32, cutoff: f32) {
        let g = tanf(f32c::PI * cutoff / sample_rate);
        let ginv = g / (1.0 + g * (g + f32c::SQRT_2));
        self.g1 = ginv;
        self.g2 = 2.0 * (g + f32c::SQRT_2) * ginv;
        self.g3 = g * ginv;
        self.g4 = 2.0 * ginv;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let v1z = self.v1;
        let v3 = input + self.v0z - 2.0 * self.v2;
        self.v1 += self.g1 * v3 - self.g2 * v1z;
        self.v2 += self.g3 * v3 + self.g4 * v1z;
        self.v0z = input;
        self.v2
    }
}

#[derive(Clone, Copy, Debug)]
struct Grain {
    read: [f64; 2],
    phase_speed: [f64; 2],
    speed: [f64; 2],
    phase: f64,
}

/// The most grains alive at once, and the room kept for them: a new grain past it is not started, so the list never
/// allocates on the audio thread and the work per sample has a ceiling, whatever the knobs. Inside their ranges at
/// most 2 x density (16) = 32 are alive, 64 at the shortest random waits (twitch 1 halves a wait): never reached
const GRAIN_ROOM: usize = 128;

/// The Granular Pitch Shifter
#[derive(Clone, Debug)]
pub struct GranularPitchShifter {
    sample_rate: f32,
    nyquist: f32,
    range: usize,
    ring: Vec<[f32; 2]>,
    write: i32,
    grains: Vec<Grain>,
    prefilter: [Prefilter; 2],
    speed: [f64; 2],
    true_pitch: [f64; 2],
    dc: [f32; 2],
    dc_coeff: f32,
    next_wait: f32,
    since_last: i32,
    old_glide: f64,
    glide_coef: f64,
    update_pitches: bool,
    rng: NoiseRng,
}

impl GranularPitchShifter {
    /// Prepared for @p sample_rate with the ring of @p range, the pitch at @p pitch and @p pitch_spread
    /// semitones; @p seed is its randomness'
    pub fn new(sample_rate: f32, range: usize, pitch: f32, pitch_spread: f32, seed: u64) -> Self {
        let mut s = Self {
            sample_rate,
            nyquist: sample_rate / 2.0,
            range,
            ring: Vec::new(),
            write: 0,
            grains: Vec::with_capacity(GRAIN_ROOM),
            prefilter: [Prefilter::default(); 2],
            speed: [1.0; 2],
            true_pitch: [0.0; 2],
            dc: [0.0; 2],
            dc_coeff: 0.0,
            next_wait: 1.0,
            since_last: 999_999_999,
            old_glide: -1.0,
            glide_coef: 0.0,
            update_pitches: true,
            rng: NoiseRng::new(seed),
        };
        s.prepare(range, pitch, pitch_spread);
        s
    }

    /// The ring of @p range, empty, and the pitch at once where the knobs are
    fn prepare(&mut self, range: usize, pitch: f32, pitch_spread: f32) {
        let range = range.min(RANGE_SECONDS.len() - 1);
        self.range = range;
        let length = (self.sample_rate * RANGE_SECONDS[range]) as usize;
        self.ring.clear();
        self.ring.resize(length, [0.0; 2]);
        self.write = 0;
        self.old_glide = -1.0;
        self.update_pitches = true;
        self.grains.clear();
        self.dc_coeff = expf(-2.0 * f32c::PI * DC_REMOVAL_HZ / self.sample_rate);
        let pitch = f64::from(pitch) * (1.0 / 12.0);
        let spread = f64::from(pitch_spread) * (1.0 / 24.0);
        self.true_pitch = [pitch - spread, pitch + spread];
        self.speed = self.true_pitch.map(exp2);
    }

    pub fn reset(&mut self) {
        self.ring.fill([0.0; 2]);
        self.write = 0;
        self.grains.clear();
        self.since_last = 999_999_999;
        self.next_wait = 1.0;
        for filter in &mut self.prefilter {
            filter.clear();
        }
        self.dc = [0.0; 2];
    }

    fn ring_sample(&self, index: f64, ch: usize) -> f32 {
        let floor = index as usize;
        let fraction = index - floor as f64;
        let len = self.ring.len();
        let v0 = if floor == 0 { self.ring[len - 1][ch] } else { self.ring[floor - 1][ch] };
        let v1 = self.ring[floor][ch];
        let (v2, v3) = if floor >= len - 2 {
            (self.ring[(floor + 1) % len][ch], self.ring[(floor + 2) % len][ch])
        } else {
            (self.ring[floor + 1][ch], self.ring[floor + 2][ch])
        };
        hermite(v0, v1, v2, v3, fraction as f32)
    }

    /// The process; a change of range takes effect after it (the ring emptied, as the legacy program did)
    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &GranularSettings) {
        let sr = self.sample_rate;
        let ring_len = self.ring.len() as i32;
        let min_latency = (s.min_latency * sr) as i32;
        let density_inv_root = (1.0 / s.density).sqrt();
        let fade_length = 1.0 / s.fade_length;
        if f64::from(s.glide) != self.old_glide {
            self.old_glide = f64::from(s.glide);
            self.glide_coef = if s.glide > 0.0 { f64::from(expf(-1.0 / (s.glide * sr))) } else { 0.0 };
        }
        // Between equal gain (1) and equal power (2)
        let shape_k = -6.0026608 + s.shape * (6.8773512 - 1.5838104 * s.shape);
        let size_samples = ((sr / s.size) as i32).max(1);
        let wait_mult = size_samples as f32 / (s.density * 2.0);
        let n = frames.len();

        for (f, frame) in frames.iter_mut().enumerate() {
            let pitch = f64::from(s.pitch.at(f, n)) * (1.0 / 12.0);
            let spread = f64::from(s.pitch_spread.at(f, n)) * (1.0 / 24.0);
            for i in 0..2 {
                let target = pitch + spread * if i == 1 { 1.0 } else { -1.0 };
                if target == self.true_pitch[i] {
                    continue;
                }
                self.update_pitches = true;
                self.true_pitch[i] = self.glide_coef * self.true_pitch[i] + (1.0 - self.glide_coef) * target;
                if (target - self.true_pitch[i]).abs() < GLIDE_SNAG_RADIUS {
                    self.true_pitch[i] = target;
                }
            }
            if self.update_pitches {
                self.update_pitches = false;
                let speed = self.true_pitch.map(exp2);
                let ratio = [speed[0] / self.speed[0], speed[1] / self.speed[1]];
                for grain in &mut self.grains {
                    for j in 0..2 {
                        grain.speed[j] *= ratio[j];
                        // Faster than the writing: the grain must end before it reaches it
                        if grain.speed[j] > 1.0 {
                            let mut distance = f64::from(self.write) - grain.read[j] - f64::from(SAFETY_LATENCY);
                            if distance <= 0.0 {
                                distance += f64::from(ring_len);
                            }
                            let required = ((grain.speed[j] - 1.0) / distance) * (1.0 - grain.phase);
                            grain.phase_speed[j] = grain.phase_speed[j].max(required);
                        }
                    }
                }
                self.speed = speed;
                // What the pitch would push above Nyquist, low-passed away
                for ch in 0..2 {
                    let cutoff = (self.nyquist / speed[ch] as f32).min(self.nyquist) * PREFILTER_BANDWIDTH;
                    self.prefilter[ch].set(sr, cutoff);
                }
            }

            let mut out = [0.0f32; 2];
            let mut filtered = *frame;

            // A new grain, when it is time
            self.since_last += 1;
            if self.since_last as f32 >= self.next_wait * wait_mult {
                self.since_last = 0;
                let rand = self.rng.unit() * 2.0 - 1.0;
                self.next_wait = exp2(rand * f64::from(s.twitch)) as f32;
                let grain_speed = 1.0 / exp2(rand * f64::from(s.jitter));
                let mut spray = [0.0f32; 2];
                if s.spray > 0.0 {
                    let range = f64::from(s.spray * sr);
                    spray[0] = (self.rng.unit() * range) as f32;
                    spray[1] = lerp(spray[0], (self.rng.unit() * range) as f32, s.spray_spread);
                }
                let farthest =
                    (f64::from(size_samples) * (self.speed[0].max(self.speed[1]) * grain_speed - 1.0)).max(0.0);
                let latency = ((farthest + f64::from(SAFETY_LATENCY)) as i32).max(min_latency);
                let read: [f64; 2] = core::array::from_fn(|i| {
                    // Never further back than the ring holds
                    let point = ((self.write - latency) as f32 - spray[i]).max((self.write - (ring_len - 1)) as f32);
                    let mut at = point as i32;
                    if at < 0 {
                        at += ring_len;
                    }
                    f64::from(at)
                });
                let phase_inc = 1.0 / f64::from(size_samples);
                if self.grains.len() < GRAIN_ROOM {
                    self.grains.push(Grain {
                        read,
                        phase_speed: [phase_inc; 2],
                        speed: [grain_speed * self.speed[0], grain_speed * self.speed[1]],
                        phase: 0.0,
                    });
                }
            }

            let mut i = 0;
            while i < self.grains.len() {
                let grain = &mut self.grains[i];
                grain.phase += grain.phase_speed[0].max(grain.phase_speed[1]);
                if grain.phase >= 1.0 {
                    // Done: the last one takes its place (and is run now)
                    self.grains.swap_remove(i);
                    continue;
                }
                for ch in 0..2 {
                    grain.read[ch] += grain.speed[ch];
                    if grain.read[ch] >= f64::from(ring_len) {
                        grain.read[ch] -= f64::from(ring_len);
                    }
                }
                let fade = ((-(-2.0 * grain.phase as f32 + 1.0).abs() + 0.5) * fade_length + 0.5).clamp(0.0, 1.0);
                // Signalsmith's cheap energy crossfade
                let a = fade * (1.0 - fade);
                let b = a * (1.0 + shape_k * a);
                let c = b + fade;
                let window = c * c;
                let read = grain.read;
                out[0] += self.ring_sample(read[0], 0) * window;
                out[1] += self.ring_sample(read[1], 1) * window;
                i += 1;
            }

            // Uncorrelated grains add in power
            for ch in 0..2 {
                out[ch] *= density_inv_root;
                // A one-pole high-pass, so that the feedback carries no DC
                self.dc[ch] = (1.0 - self.dc_coeff) * out[ch] + self.dc_coeff * self.dc[ch];
                out[ch] -= self.dc[ch];
                if s.feedback > 0.0 {
                    out[ch] = saturate(out[ch]);
                }
            }

            self.write += 1;
            if self.write >= ring_len {
                self.write = 0;
            }
            if s.prefilter {
                for ch in 0..2 {
                    filtered[ch] = self.prefilter[ch].process(filtered[ch]);
                }
            }
            let at = self.write as usize;
            self.ring[at] = [filtered[0] + out[0] * s.feedback, filtered[1] + out[1] * s.feedback];
            *frame = out;
        }

        if s.range.min(RANGE_SECONDS.len() - 1) != self.range {
            self.prepare(s.range, s.pitch.to, s.pitch_spread.to);
        }
    }
}

/// A cheap saturator that costs nothing below 16
#[inline]
fn saturate(x: f32) -> f32 {
    let a = x.abs();
    if a <= SATURATION_SAFE {
        x
    } else {
        ((a - SATURATION_SAFE) / (1.0 + (a - SATURATION_SAFE) * SATURATION_STRENGTH) + SATURATION_SAFE).copysign(x)
    }
}
