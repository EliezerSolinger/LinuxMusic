//! The instrument model every importer produces and the engine plays: a list of zones (what plays for which key and
//! velocity, and how), the output groups, and nothing else. The model is plain data: immutable once built, shared
//! between engines in an `Arc`, and read on the audio thread without a lock.

use std::sync::Arc;

use lm_base::{Error, Result};

use crate::pcm::Pcm;

/// How a zone's sample loops
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LoopMode {
    /// Plays once to its end; a note-off starts the envelope's release
    #[default]
    Off,
    /// Plays to its end whatever the note-off does (a drum hit)
    OneShot,
    /// Loops from the loop start to the loop end until the sound ends
    Forward,
    /// Loops back and forth between the loop points
    PingPong,
    /// Loops while the key is down; after the note-off it plays on past the loop to the sample's end (SF2 mode 3,
    /// SFZ `loop_sustain`)
    UntilRelease,
}

impl LoopMode {
    pub fn is_looping(self) -> bool {
        matches!(self, LoopMode::Forward | LoopMode::PingPong | LoopMode::UntilRelease)
    }
}

/// When a zone starts
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// On a note-on
    #[default]
    Attack,
    /// On a note-off (the key's release noise), at the velocity of the note that ends
    Release,
    /// On a note-on while no other note is held (the first of a phrase)
    First,
    /// On a note-on while another note is held (legato)
    Legato,
}

/// How a note's velocity shapes a zone's level
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VelCurve {
    /// The level is the velocity (the legacy sample instruments)
    Linear,
    /// The level is the velocity squared: 40 dB of range, SoundFont's concave curve
    #[default]
    Concave,
}

/// The shape of an AHDSR amplitude or filter envelope. Times in seconds
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ahdsr {
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    /// 0 to 1, linear (for the amplitude envelope: the amplitude it falls to)
    pub sustain: f32,
    pub release: f32,
}

impl Ahdsr {
    /// A plain organ-like envelope with the short release that keeps a note-off from clicking
    pub const SHORT: Ahdsr = Ahdsr { delay: 0.0, attack: 0.0, hold: 0.0, decay: 0.0, sustain: 1.0, release: 0.003 };
}

impl Default for Ahdsr {
    fn default() -> Self {
        Self::SHORT
    }
}

/// The filter's response
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FilterKind {
    #[default]
    LowPass,
    HighPass,
    BandPass,
}

/// A zone's filter, with its envelope. The cutoff moves from `cutoff` by the key, the velocity and the envelope
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    pub kind: FilterKind,
    /// Hz
    pub cutoff: f32,
    /// Resonance, dB
    pub resonance: f32,
    /// Cents of cutoff per key from 60 (SFZ `fil_keytrack`)
    pub key_track: f32,
    /// Cents of cutoff added at velocity 0 (negative: a soft note is darker), falling to 0 at velocity 1
    pub vel_cents: f32,
    /// The envelope and how far it moves the cutoff, in cents, at its peak
    pub env: Ahdsr,
    pub env_depth: f32,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            kind: FilterKind::LowPass,
            cutoff: 20_000.0,
            resonance: 0.0,
            key_track: 0.0,
            vel_cents: 0.0,
            env: Ahdsr { delay: 0.0, attack: 0.0, hold: 0.0, decay: 0.0, sustain: 1.0, release: 0.0 },
            env_depth: 0.0,
        }
    }
}

/// Round-robin: of the zones of one `group` whose keys and velocities match, the one at `position` (1 to `length`) of
/// the turn plays; every note of the group moves the turn on
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    pub group: u16,
    pub length: u16,
    pub position: u16,
}

/// A zone's window on its sample, set by fractions of the sample's length (the AudioFileProcessor's knobs): the engine
/// reads the parameters of the plugin through this instead of the zone's fixed bounds
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub start: f32,
    pub end: f32,
    pub loop_start: f32,
}

impl Default for Window {
    fn default() -> Self {
        Self { start: 0.0, end: 1.0, loop_start: 0.0 }
    }
}

/// What plays for a range of keys and velocities
#[derive(Clone, Debug)]
pub struct Zone {
    /// Inclusive
    pub keys: (u8, u8),
    /// Inclusive, 1 to 127
    pub vels: (u8, u8),
    pub sample: Arc<Pcm>,
    /// The key the sample sounds at its own pitch
    pub root_key: u8,
    /// Cents added to the pitch (fine tuning)
    pub tune: f32,
    /// Cents per key: 100 follows the keyboard, 0 plays every key at the same pitch
    pub key_track: f32,
    /// The part of the sample played, in frames (`end` 0: to the sample's end)
    pub start: u32,
    pub end: u32,
    pub loop_mode: LoopMode,
    pub loop_start: u32,
    pub loop_end: u32,
    pub reverse: bool,
    /// Each note starts where the last one of the zone stopped (the AudioFileProcessor's stutter)
    pub stutter: bool,
    /// The window knobs apply to this zone
    pub windowed: bool,
    /// A short fade in and out against clicks
    pub declick: bool,
    /// dB
    pub gain: f32,
    /// −1 left to 1 right
    pub pan: f32,
    /// 0 to 1: how much of the level the velocity decides
    pub vel_track: f32,
    pub vel_curve: VelCurve,
    pub amp: Ahdsr,
    pub filter: Option<Filter>,
    /// The exclusive group (0: none): a note of the zone ends the sounding notes of every zone whose `off_by` it is
    pub group: u32,
    pub off_by: u32,
    /// The ended notes fall in a few milliseconds instead of their release
    pub off_fast: bool,
    pub trigger: Trigger,
    /// dB per second a release trigger fades with the time the key was held
    pub release_decay: f32,
    pub sequence: Option<Sequence>,
    /// The zone plays when the note's random number (0 to 1, one per note) is in `[lo, hi)`
    pub random: Option<(f32, f32)>,
    /// The index of its output group in [`Instrument::outputs`]
    pub output: u8,
    /// The zone plays only while the sustain pedal is down (`Some(true)`) or up (`Some(false)`): GIG's pedal dimension
    pub pedal: Option<bool>,
    /// The level for each MIDI velocity, replacing `vel_track` and `vel_curve` (GIG's velocity response curves)
    pub vel_table: Option<Arc<[f32; 128]>>,
}

impl Zone {
    /// A zone of the whole keyboard playing `sample` from its start at its own pitch at key 60, the defaults of every
    /// importer
    pub fn new(sample: Arc<Pcm>) -> Self {
        Self {
            keys: (0, 127),
            vels: (1, 127),
            sample,
            root_key: 60,
            tune: 0.0,
            key_track: 100.0,
            start: 0,
            end: 0,
            loop_mode: LoopMode::Off,
            loop_start: 0,
            loop_end: 0,
            reverse: false,
            stutter: false,
            windowed: false,
            declick: false,
            gain: 0.0,
            pan: 0.0,
            vel_track: 1.0,
            vel_curve: VelCurve::Concave,
            amp: Ahdsr::SHORT,
            filter: None,
            group: 0,
            off_by: 0,
            off_fast: false,
            trigger: Trigger::Attack,
            release_decay: 0.0,
            sequence: None,
            random: None,
            output: 0,
            pedal: None,
            vel_table: None,
        }
    }

    /// The zone's end in frames, inside the sample
    pub fn end_frame(&self) -> u32 {
        let n = self.sample.frames() as u32;
        if self.end == 0 { n } else { self.end.min(n) }
    }

    pub fn matches_key(&self, key: u8, velocity: u8) -> bool {
        (self.keys.0..=self.keys.1).contains(&key) && (self.vels.0..=self.vels.1).contains(&velocity)
    }
}

/// An output of the instrument: a group of zones that has its own channel pair (a drum kit's kick, snare...)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputGroup {
    /// Its port key (lowercase letters)
    pub name: String,
    /// Two channels (otherwise one)
    pub stereo: bool,
}

/// What an importer returns
#[derive(Clone, Debug)]
pub struct Instrument {
    pub name: String,
    pub zones: Vec<Zone>,
    /// At least one. A zone whose `output` is past the end plays on the first
    pub outputs: Vec<OutputGroup>,
}

/// The most zones one instrument may have (a bound against a corrupt file, not a design limit)
pub const MAX_ZONES: usize = 200_000;

impl Instrument {
    /// An empty instrument with the one stereo output
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), zones: Vec::new(), outputs: vec![OutputGroup { name: "main".into(), stereo: true }] }
    }

    /// Checks what the engine relies on: the ranges, the loop points and the outputs. Keys: `sampler.model.*`
    pub fn validate(&self) -> Result<()> {
        if self.outputs.is_empty() {
            return Err(Error::new("sampler.model.no_outputs"));
        }
        if self.zones.len() > MAX_ZONES {
            return Err(Error::new("sampler.model.too_many_zones").arg("zones", self.zones.len() as i64));
        }
        for (i, z) in self.zones.iter().enumerate() {
            let bad = |what: &'static str| Error::new("sampler.model.zone").arg("zone", i as i64).arg("what", what);
            if z.keys.0 > z.keys.1 || z.keys.1 > 127 {
                return Err(bad("keys"));
            }
            if z.vels.0 > z.vels.1 || z.vels.1 > 127 {
                return Err(bad("velocities"));
            }
            if z.sample.is_empty() {
                return Err(bad("empty sample"));
            }
            if z.start >= z.end_frame() {
                return Err(bad("start"));
            }
            if z.loop_mode.is_looping() && z.loop_end != 0 && z.loop_start >= z.loop_end {
                return Err(bad("loop"));
            }
            for v in [z.gain, z.pan, z.tune, z.key_track, z.vel_track, z.release_decay] {
                if !v.is_finite() {
                    return Err(bad("not finite"));
                }
            }
        }
        Ok(())
    }

    /// How many round-robin groups the zones name (the engine keeps a counter for each)
    pub fn sequence_groups(&self) -> usize {
        self.zones.iter().filter_map(|z| z.sequence.map(|s| usize::from(s.group) + 1)).max().unwrap_or(0)
    }

    /// The largest exclusive group number
    pub fn max_group(&self) -> u32 {
        self.zones.iter().map(|z| z.group.max(z.off_by)).max().unwrap_or(0)
    }

    /// Frames of sample data the zones hold (shared samples counted once, by address)
    pub fn sample_frames(&self) -> usize {
        let mut seen: std::collections::HashSet<*const Pcm> = std::collections::HashSet::new();
        let mut total = 0;
        for z in &self.zones {
            let p = Arc::as_ptr(&z.sample);
            if seen.insert(p) {
                total += z.sample.frames();
            }
        }
        total
    }
}

/// A preset a file offers (the choice within a SoundFont or a GIG)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetInfo {
    pub bank: u16,
    pub program: u16,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm() -> Arc<Pcm> {
        Arc::new(Pcm::mono(44100, vec![0.0; 100]))
    }

    #[test]
    fn a_default_zone_is_valid() {
        let mut i = Instrument::new("t");
        i.zones.push(Zone::new(pcm()));
        assert!(i.validate().is_ok());
    }

    #[test]
    fn bad_ranges_are_refused() {
        let mut i = Instrument::new("t");
        let mut z = Zone::new(pcm());
        z.keys = (70, 60);
        i.zones.push(z);
        assert!(i.validate().is_err());
        i.zones[0].keys = (0, 127);
        i.zones[0].start = 100;
        assert!(i.validate().is_err());
    }

    #[test]
    fn shared_samples_count_once() {
        let p = pcm();
        let mut i = Instrument::new("t");
        i.zones.push(Zone::new(p.clone()));
        i.zones.push(Zone::new(p));
        i.zones.push(Zone::new(pcm()));
        assert_eq!(i.sample_frames(), 200);
    }
}
