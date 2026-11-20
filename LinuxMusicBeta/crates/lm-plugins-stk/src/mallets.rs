//! One note of the legacy Mallets: what `MalletsInstrument::playNoteImpl` does when a note starts (the randomness,
//! the voice's controls, its level) and in each period (the live controls, the pitch, the stereo spread), with the
//! legacy `MalletsSynth` around the STK voice.
//!
//! The three models are three types ([`Model`]): a note's voice is built for its model, and the plugin is an
//! instrument of one model at a time (see the crate's documentation).

use lm_dsp::math::{LegacyRand, dbfs_to_amp};

use crate::stk::Rate;
use crate::stk::banded_wg::BandedWG;
use crate::stk::modal_bar::ModalBar;
use crate::stk::tube_bell::TubeBell;

/// The presets, in the legacy order (the legacy saved the index; the tree keeps these keys, U30)
pub const PRESET_KEYS: [&str; 14] = [
    "marimba",
    "vibraphone",
    "agogo",
    "wood1",
    "reso",
    "wood2",
    "beats",
    "two-fixed",
    "clump",
    "tubular-bells",
    "uniform-bar",
    "tuned-bar",
    "glass",
    "tibetan-bowl",
];

/// The first tubular bells preset (the bars come before it, the banded waveguides after)
pub const TUBE_BELL: usize = 9;
/// The first banded waveguide preset
pub const BANDED: usize = 10;

/// Each preset's output scale (`m_scalers`)
pub const SCALERS: [f32; 14] = [4.0, 4.0, 5.0, 4.0, 2.5, 5.0, 20.0, 5.0, 4.0, 1.8, 25.0, 10.0, 16.0, 7.0];
/// Each preset's level trim when levelled, dB (`playNoteImpl`'s `Trims`: thirteen values for fourteen presets, the
/// last used for the last two, as in the legacy)
pub const TRIMS: [f32; 13] = [-7.0, -8.5, -11.0, -6.0, -5.0, -6.0, -16.5, -12.5, -6.5, -0.5, -3.0, -5.0, -3.0];
/// The bowed bars' gain (`BowedGains`): uniform bar, tuned bar, glass, Tibetan bowl
const BOWED_GAINS: [f64; 4] = [1.0, 2.0, 1.0, 2.4];
/// The legacy `DefaultVolume`
const DEFAULT_VOLUME: f32 = 100.0;

/// The legacy note volume of a velocity: 0 to 200, the default note (100) at velocity 0.5 (see the crate's
/// documentation, U28)
pub fn legacy_volume(velocity: f32) -> f32 {
    velocity * 200.0
}

/// The knobs, as the legacy models hold them
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Knobs {
    pub hardness: f32,
    pub position: f32,
    pub vibrato_gain: f32,
    pub vibrato_freq: f32,
    pub stick_mix: f32,
    pub modulator: f32,
    pub crossfade: f32,
    pub lfo_speed: f32,
    pub lfo_depth: f32,
    pub adsr: f32,
    pub pressure: f32,
    pub speed: f32,
    pub bowed: bool,
    pub spread: f32,
    pub randomness: f32,
}

/// What a note starts with besides the knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Start {
    /// The preset (an index of [`PRESET_KEYS`])
    pub preset: usize,
    /// Hz
    pub freq: f32,
    /// The legacy note volume, 0 to 200 ([`legacy_volume`])
    pub volume: f32,
    /// The instrument's levelling (`m_levelled`)
    pub levelled: bool,
    /// A project from before the presets' version (`m_isOldVersionModel`)
    pub old_version: bool,
}

/// One of the legacy models, and what a note does with it
pub trait Model: Clone + Send + 'static {
    /// A voice as the library builds it
    fn new(rate: Rate) -> Self;
    /// The voice back to `fresh` (a new voice's state) without allocating
    fn reset_from(&mut self, fresh: &Self);
    /// `MalletsSynth`'s constructor for this model, after the randomness: the controls and the note on. Returns the
    /// gain of the voice's output (`m_gain`)
    fn note_on(&mut self, knobs: &Knobs, start: &Start, velocity: f64, rand: &mut LegacyRand) -> f64;
    /// What `playNoteImpl` sends the voice in each period
    fn update(&mut self, knobs: &Knobs);
    fn set_frequency(&mut self, freq: f64);
    fn tick(&mut self) -> f64;
}

/// `fastRandInc(from, to)` in single precision
fn rand_inc(rand: &mut LegacyRand, from: f32, to: f32) -> f32 {
    from + rand.inclusive(to - from)
}

impl Model for ModalBar {
    fn new(rate: Rate) -> Self {
        ModalBar::new(rate)
    }

    fn reset_from(&mut self, fresh: &Self) {
        *self = *fresh;
    }

    fn note_on(&mut self, knobs: &Knobs, start: &Start, velocity: f64, rand: &mut LegacyRand) -> f64 {
        let random = knobs.randomness;
        let mut hardness = knobs.hardness;
        let mut position = knobs.position;
        hardness += random * rand_inc(rand, -64.0, 64.0);
        hardness = hardness.clamp(0.0, 128.0);
        position += random * rand_inc(rand, -32.0, 32.0);
        position = position.clamp(0.0, 64.0);
        self.control_change(16, start.preset as f64);
        self.control_change(1, f64::from(knobs.stick_mix));
        self.control_change(2, f64::from(hardness));
        self.control_change(4, f64::from(position));
        self.control_change(8, f64::from(knobs.vibrato_gain));
        self.control_change(11, f64::from(knobs.vibrato_freq));
        self.control_change(128, 128.0);
        ModalBar::note_on(self, f64::from(start.freq), velocity);
        1.0
    }

    fn update(&mut self, knobs: &Knobs) {
        // (the library swaps controls 8 and 1 in ModalBar: 8 is the vibrato's gain)
        self.control_change(8, f64::from(knobs.vibrato_gain));
        self.control_change(11, f64::from(knobs.vibrato_freq));
    }

    fn set_frequency(&mut self, freq: f64) {
        ModalBar::set_frequency(self, freq);
    }

    fn tick(&mut self) -> f64 {
        ModalBar::tick(self)
    }
}

impl Model for TubeBell {
    fn new(rate: Rate) -> Self {
        TubeBell::new(rate)
    }

    fn reset_from(&mut self, fresh: &Self) {
        *self = *fresh;
    }

    fn note_on(&mut self, knobs: &Knobs, start: &Start, velocity: f64, rand: &mut LegacyRand) -> f64 {
        let random = knobs.randomness;
        let mut modulator = knobs.modulator;
        let mut crossfade = knobs.crossfade;
        modulator += random * rand_inc(rand, -64.0, 64.0);
        modulator = modulator.clamp(0.0, 128.0);
        crossfade += random * rand_inc(rand, -64.0, 64.0);
        crossfade = crossfade.clamp(0.0, 128.0);
        self.control_change(1, f64::from(knobs.lfo_depth));
        self.control_change(2, f64::from(modulator));
        self.control_change(4, f64::from(crossfade));
        self.control_change(11, f64::from(knobs.lfo_speed));
        self.control_change(128, f64::from(knobs.adsr));
        TubeBell::note_on(self, f64::from(start.freq), velocity);
        1.0
    }

    fn update(&mut self, knobs: &Knobs) {
        self.control_change(128, f64::from(knobs.adsr));
        self.control_change(1, f64::from(knobs.lfo_depth));
        self.control_change(11, f64::from(knobs.lfo_speed));
    }

    fn set_frequency(&mut self, freq: f64) {
        TubeBell::set_frequency(self, freq);
    }

    fn tick(&mut self) -> f64 {
        TubeBell::tick(self)
    }
}

impl Model for BandedWG {
    fn new(rate: Rate) -> Self {
        BandedWG::new(rate)
    }

    fn reset_from(&mut self, fresh: &Self) {
        BandedWG::reset_from(self, fresh);
    }

    /// The legacy's bowed bars (see `MalletsSynth`'s BandedWG constructor): the bow's integration at 0.75 (control
    /// 11 = 96), its pressure from 1 to 48 on the knob, its speed the note's velocity scaled by the speed knob up to
    /// 0.55, and a gain per preset; plucked, only what plucking reads
    fn note_on(&mut self, knobs: &Knobs, start: &Start, velocity: f64, rand: &mut LegacyRand) -> f64 {
        const BOW_INTEGRATION: f64 = 96.0;
        const LIGHTEST_PRESSURE: f64 = 1.0;
        const HEAVIEST_PRESSURE: f64 = 48.0;
        const FASTEST_BOW: f64 = 0.55;
        let random = knobs.randomness;
        let mut pressure = knobs.pressure;
        let mut speed = knobs.speed;
        pressure += random * rand_inc(rand, -64.0, 64.0);
        pressure = pressure.clamp(0.0, 128.0);
        speed += random * rand_inc(rand, -64.0, 64.0);
        speed = speed.clamp(0.0, 128.0);
        let control16 = start.preset as i32 - BANDED as i32;
        let pitch = f64::from(start.freq);
        self.control_change(1, 128.0);
        self.control_change(16, f64::from(control16));
        if knobs.bowed {
            let pressure = f64::from(pressure);
            let speed = f64::from(speed);
            self.control_change(
                2,
                LIGHTEST_PRESSURE + pressure.clamp(0.0, 128.0) / 128.0 * (HEAVIEST_PRESSURE - LIGHTEST_PRESSURE),
            );
            self.control_change(11, BOW_INTEGRATION);
            self.control_change(64, 128.0);
            BandedWG::note_on(self, pitch, (velocity * speed / 64.0).clamp(0.0, 1.0) * FASTEST_BOW);
            BOWED_GAINS[control16.clamp(0, 3) as usize]
        } else {
            self.control_change(64, 0.0);
            BandedWG::note_on(self, pitch, velocity);
            1.0
        }
    }

    fn update(&mut self, _: &Knobs) {}

    fn set_frequency(&mut self, freq: f64) {
        BandedWG::set_frequency(self, freq);
    }

    fn tick(&mut self) -> f64 {
        BandedWG::tick(self)
    }
}

/// The stereo spread's ring (the legacy's held 256 samples; here up to 255 samples at 44.1 kHz at any rate up to
/// 192 kHz)
const SPREAD_RING: usize = 2048;
const SPREAD_MASK: u64 = SPREAD_RING as u64 - 1;

/// A note: the voice and what `MalletsSynth` keeps around it
#[derive(Debug)]
pub struct Note<M> {
    model: M,
    rate: Rate,
    preset: usize,
    /// The voice's frequency, last set (`m_frequency`)
    frequency: f64,
    /// `m_gain`
    gain: f64,
    /// `m_level`
    level: f32,
    /// The legacy note volume and the output gain it gives (the legacy track's `volume / 100`, U28's half)
    output: f32,
    /// The right side's delay (samples at this rate)
    spread: u64,
    written: u64,
    ring: Box<[f64; SPREAD_RING]>,
}

impl<M: Model> Clone for Note<M> {
    fn clone(&self) -> Self {
        Self { model: self.model.clone(), ring: self.ring.clone(), ..*self }
    }

    /// Without allocating (the instrument copies voices on the audio thread when it crossfades a change)
    fn clone_from(&mut self, source: &Self) {
        self.model.clone_from(&source.model);
        // (only what the right side may still read)
        let n = source.spread.min(source.written).min(SPREAD_RING as u64);
        for i in 0..n {
            let at = (source.written - 1 - i) & SPREAD_MASK;
            self.ring[at as usize] = source.ring[at as usize];
        }
        self.rate = source.rate;
        self.preset = source.preset;
        self.frequency = source.frequency;
        self.gain = source.gain;
        self.level = source.level;
        self.output = source.output;
        self.spread = source.spread;
        self.written = source.written;
    }
}

impl<M: Model> Note<M> {
    /// A voice (in the build: it allocates)
    pub fn new(fresh: &M, rate: Rate) -> Self {
        Self {
            model: fresh.clone(),
            rate,
            preset: 0,
            frequency: 0.0,
            gain: 1.0,
            level: 1.0,
            output: 1.0,
            spread: 0,
            written: 0,
            ring: Box::new([0.0; SPREAD_RING]),
        }
    }

    /// `playNoteImpl` when the note starts: a new voice (`fresh`), the knobs randomized from `rand`, the controls and
    /// the note on, the level
    pub fn start(&mut self, fresh: &M, knobs: &Knobs, start: &Start, rand: &mut LegacyRand) {
        self.model.reset_from(fresh);
        let p = start.preset;
        // If newer projects, adjust velocity to within STK's limits
        let adjust: f32 = if start.old_version { 100.0 } else { 200.0 };
        let velocity = start.volume / adjust;
        self.gain = self.model.note_on(knobs, start, f64::from(velocity), rand);
        self.frequency = f64::from(start.freq);
        self.preset = p;
        self.level = 1.0;
        if start.levelled {
            let mut level = dbfs_to_amp(TRIMS[p.min(TRIMS.len() - 1)]);
            // Struck (bars, bells): the strike's own law is the square of its velocity, made proportional to it
            if p <= TUBE_BELL {
                level *= DEFAULT_VOLUME / start.volume.max(1.0);
            }
            self.level = level;
        }
        self.output = start.volume * (1.0 / DEFAULT_VOLUME) * 0.5;
        // (the spread is samples at 44.1 kHz: as long at any rate)
        let spread = f64::from(knobs.spread.clamp(0.0, 255.0) as u8);
        self.spread = if self.rate.is_reference() { spread as u64 } else { (spread / self.rate.k).round() as u64 };
        self.written = 0;
    }

    /// The preset the note plays
    pub fn preset(&self) -> usize {
        self.preset
    }

    /// One period of `playNoteImpl` (here a piece of at most 32 frames): the pitch when it changed, the live
    /// controls, then the samples. `left` and `right` have the same length; with `raw`, without the output gain (the
    /// legacy plugin's own output, for the A/B)
    pub fn render(&mut self, knobs: &Knobs, old_version: bool, freq: f32, left: &mut [f32], right: &mut [f32]) {
        self.render_with(knobs, old_version, freq, left, right, false);
    }

    /// [`Note::render`], with `raw` leaving out the output gain
    pub fn render_with(
        &mut self,
        knobs: &Knobs,
        old_version: bool,
        freq: f32,
        left: &mut [f32],
        right: &mut [f32],
        raw: bool,
    ) {
        let pitch = f64::from(freq);
        if pitch != self.frequency {
            self.model.set_frequency(pitch);
            self.frequency = pitch;
        }
        self.model.update(knobs);
        let p = self.preset;
        let mut add_scale = 0.0f32;
        if p == BANDED && old_version {
            add_scale = f32::from(u8::from(knobs.bowed)) * freq * 2.5;
        }
        let scale = SCALERS[p] + add_scale;
        let level = self.level;
        let output = self.output;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            // MalletsSynth::nextSampleLeft and nextSampleRight
            let s = self.model.tick() * self.gain;
            self.ring[(self.written & SPREAD_MASK) as usize] = s;
            let delayed = if self.written >= self.spread {
                self.ring[((self.written - self.spread) & SPREAD_MASK) as usize]
            } else {
                0.0
            };
            self.written += 1;
            let (a, b) = ((s as f32) * scale * level, (delayed as f32) * scale * level);
            if raw {
                *l = a;
                *r = b;
            } else {
                *l = a * output;
                *r = b * output;
            }
        }
    }
}
