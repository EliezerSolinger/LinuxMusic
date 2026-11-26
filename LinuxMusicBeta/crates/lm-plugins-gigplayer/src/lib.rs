//! The GigPlayer (F-INS; legacy `plugins/GigPlayer`, Garrett Wilson's): a GigaStudio `.gig` file played from memory
//! through libgig (`lm-sys-gig`), one synthesizer per instrument, with the note functions of a single-stream
//! instrument (`lm_plugin::instrument::stream`: chords, arpeggio, mono, legato and glide, the sustain pedal, the
//! pitch knob).
//!
//! Its state, version 1:
//! - `gig`: the `.gig` file, a resource of kind `"gig"` (null: silent, as a legacy instrument before a file was
//!   opened);
//! - `bank` (0–999) and `patch` (0–127), **fields** and not parameters: they choose the instrument of the file,
//!   and loading an instrument (every sample decoded to memory) may take longer than 200 ms, so a change builds a new
//!   synthesizer apart, and the old one finishes its notes as a ghost (U32). A bank and a patch the file lacks play
//!   nothing, as in the legacy plugin;
//! - parameters with the legacy knob's range and default: `gain` (0–5, 1); and the volume envelope the legacy took
//!   from the track's ENV tab (`volPredelay`, `volAttack`, `volHold`, `volDecay`, `volSustain`, `volRelease`,
//!   `volAmount`): with an amount above 0 it takes the place of the file's own envelope on every voice;
//! - the note functions.
//!
//! The voices are the legacy's: a note chooses, for each region of the instrument that covers its key, the
//! dimension region its velocity, the sustain pedal, the release trigger... select (libgig's own selection), and
//! plays that region's sample with libsamplerate at the ratio of the engine's rate and, when the region tracks the
//! pitch, of the note's frequency to the sample's; the file's volume envelope (pre-attack, attack, decay, sustain,
//! release) and the first loop of the region (forward and ping-pong) shape it. A release-trigger dimension plays its
//! release samples at the key up, with the legacy's attenuation by the decay. The legacy ignores the pitch bend
//! (`IsNotBendable`), and so does this.

#![forbid(unsafe_code)]

use lm_base::Result;
use lm_plugin::instrument::{Stream, StreamInstrument, add_note_functions, instrument_params};
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{NoteId, Processor};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Resource, Value};
use lm_sys_gig::{DimensionKind, LoopType, Patch};
use lm_sys_samplerate::{Quality, Resampler};

pub const ID: &str = "gigplayer";
/// Notes followed at once (the legacy's `MostNotes`)
const MOST_NOTES: usize = 128;
/// Voices made in advance, with their resamplers (the legacy's `MostVoices`)
const MOST_VOICES: usize = 256;
/// Voices a note may hold (the legacy's `GigNote::MostSamples`)
const MOST_SAMPLES: usize = 16;
/// Frames of a sample read at a time, before the resampler (the legacy's `DEFAULT_BUFFER_SIZE`)
const SOURCE: usize = 256;
/// Frames the resampler writes in one call
const MIX: usize = 64;
/// The resampler the legacy used (`AudioResampler::Mode::Linear`)
const QUALITY: Quality = Quality::Linear;

/// The parameters, in index order
mod k {
    pub const GAIN: usize = 0;
    /// The volume envelope: predelay, attack, hold, decay, sustain, release, amount
    pub const ENV: usize = 1;
    pub const COUNT: usize = ENV + 7;
}

const ENV_KEYS: [&str; 7] =
    ["volPredelay", "volAttack", "volHold", "volDecay", "volSustain", "volRelease", "volAmount"];

fn own_params() -> Vec<ParamSpec> {
    let mut v = vec![ParamSpec::new("gain", 0.0, 5.0, 1.0)];
    for key in ENV_KEYS {
        if let Some(p) = instrument_params().iter().find(|p| p.key == key) {
            v.push(p.clone());
        }
    }
    v
}

/// The descriptor
pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.gigplayer.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    // The file's velocity layers and the velocity attenuation of each
    velocity: Velocity::Timbre,
    display: None,
    build,
    tests: TestHints {
        tuning: Some(TuningProbe { state: probe_state, key: 69, hz: 440.0 }),
        no_adopt: &[],
        voices: 16,
        max_level: 1.0e3,
        tail: lm_plugin::DEFAULT_TAIL,
    },
};

/// The resource a test plays (any key: the tests' setup gives every `gig` resource the test file)
pub fn test_gig() -> Resource {
    Resource::new("gig", &"6a".repeat(32)).unwrap_or_else(|_| unreachable!("a valid key"))
}

/// The tuning probe: the test file's sine at 440 Hz on key 69
fn probe_state() -> Value {
    Value::empty_object().with("gig", Value::resource(test_gig()))
}

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:gigplayer", 1)
        .field(FieldSpec::new("gig", FieldKind::Resource("gig")).nullable().default(Value::NULL))
        .field(FieldSpec::int("bank", 0, 999, 0))
        .field(FieldSpec::int("patch", 0, 127, 0));
    for p in own_params() {
        s = s.param(p);
    }
    vec![add_note_functions(s)]
}

fn ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut values = [0.0f32; k::COUNT];
    for (i, p) in own_params().iter().enumerate() {
        values[i] = s.param_f32(p.key)?;
    }
    let bank = s.int("bank")?.clamp(0, 999) as i32;
    let program = s.int("patch")?.clamp(0, 127) as i32;
    let patch = match s.resource("gig")? {
        Some(r) => {
            let path = cx.resources.file(r)?;
            match Patch::load(&path, bank, program) {
                Ok(patch) => Some(patch),
                // A bank and a patch the file lacks: silent, as the legacy plugin was
                Err(e) if e.key() == "gig.no_instrument" => None,
                Err(e) => return Err(e),
            }
        }
        None => None,
    };
    let stream = Gig::new(patch, values, cx.spec.sample_rate as f32)?;
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(StreamInstrument::new(stream, &s, params, &cx.spec)?))
}

// ---- the envelope ----

/// The track's volume envelope (the ENV tab), in seconds; `amount` above 0 turns it on
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Shape {
    predelay: f32,
    attack: f32,
    hold: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    amount: f32,
}

/// The amplitude of a voice, from the file's envelope for its region (the legacy `ADSR`): counted in the frames of
/// the sample, each asked for once per frame
#[derive(Clone, Copy, Debug)]
struct Adsr {
    /// The starting amplitude, 0 to 1
    preattack: f32,
    sustain: f32,
    amplitude: f32,
    is_attack: bool,
    is_release: bool,
    is_done: bool,
    attack_position: i64,
    attack_length: i64,
    decay_length: i64,
    release_position: i64,
    release_length: i64,
}

impl Adsr {
    fn idle() -> Self {
        Self {
            preattack: 0.0,
            sustain: 0.0,
            amplitude: 0.0,
            is_attack: true,
            is_release: false,
            is_done: false,
            attack_position: 0,
            attack_length: 0,
            decay_length: 0,
            release_position: 0,
            release_length: 0,
        }
    }

    /// The file's envelope of a region, counting in `rate` frames a second
    fn from_region(r: &lm_sys_gig::Envelope, rate: i64) -> Self {
        let mut a = Self::idle();
        // (EG1PreAttack and EG1Sustain are permille; the second decay is ignored, as the legacy did)
        a.preattack = r.preattack / 1000.0;
        a.sustain = r.sustain / 1000.0;
        a.amplitude = a.preattack;
        a.attack_length = (f64::from(r.attack) * rate as f64) as i64;
        a.decay_length = (f64::from(r.decay1) * rate as f64) as i64;
        a.release_length = (f64::from(r.release) * rate as f64) as i64;
        a.start_level();
        a
    }

    /// With no attack or decay the note starts at the sustain, with no attack at the full amplitude
    fn start_level(&mut self) {
        if self.attack_length == 0 && self.decay_length == 0 {
            self.amplitude = self.sustain;
        } else if self.attack_length == 0 {
            self.amplitude = 1.0;
        }
    }

    /// The track's volume envelope in place of the file's: its pre-delay joins the attack and its hold the decay
    fn use_envelope(&mut self, shape: &Shape, rate: i64) {
        self.preattack = (1.0 - shape.amount).clamp(0.0, 1.0);
        self.sustain = (1.0 - shape.amount + shape.amount * shape.sustain).clamp(0.0, 1.0);
        self.amplitude = self.preattack;
        self.attack_length = (f64::from(shape.predelay + shape.attack) * rate as f64) as i64;
        self.decay_length = (f64::from(shape.hold + shape.decay) * rate as f64) as i64;
        self.release_length = ((f64::from(shape.release) * rate as f64) as i64).max(1);
        self.start_level();
    }

    /// The key went up: the release starts now
    fn keyup(&mut self) {
        self.is_release = true;
    }

    fn done(&self) -> bool {
        self.is_done
    }

    /// The amplitude of the next frame
    fn value(&mut self) -> f32 {
        let current = self.amplitude;
        if self.is_done {
            return 0.0;
        } else if self.is_attack && self.is_release {
            // Released in the attack: it fades from where it is, not from the sustain
            self.sustain = self.amplitude;
            self.is_attack = false;
        }
        if self.is_attack {
            if self.attack_position < self.attack_length {
                self.amplitude =
                    self.preattack + (1.0 - self.preattack) / self.attack_length as f32 * self.attack_position as f32;
            } else if self.attack_position < self.attack_length + self.decay_length {
                self.amplitude = 1.0
                    - (1.0 - self.sustain) / self.decay_length as f32
                        * (self.attack_position - self.attack_length) as f32;
            } else {
                self.is_attack = false;
            }
            self.attack_position += 1;
        } else if self.is_release {
            // The legacy's curve: f(0) = sustain and f(release length) very small
            self.amplitude = (self.sustain + 1.0e-3)
                * (-5.0 / self.release_length as f32 * self.release_position as f32).exp()
                - 1.0e-3;
            if self.amplitude <= 0.0 || self.release_position >= self.release_length {
                self.amplitude = 0.0;
                self.is_done = true;
            }
            self.release_position += 1;
        }
        current
    }
}

// ---- the voices ----

/// One sample playing: the region's data copied at its start, the position, the envelope and its own resampler
struct Voice {
    active: bool,
    sample: usize,
    /// The sample's rate and frames, and its channels (1 or 2)
    sample_rate: u32,
    total: i64,
    channels: usize,
    attenuation: f32,
    pitch_track: bool,
    /// The loop of the region: type, start and length in frames
    looping: Option<(LoopType, i64, i64)>,
    adsr: Adsr,
    /// The position in the sample, in frames
    pos: i64,
    /// The frequency the sample sounds at, and the factor to the note's (a pitch-tracked sample)
    sample_freq: f32,
    freq_factor: f32,
    resampler: Resampler,
    /// The frames read from the sample and not yet resampled: `source[start..start + len]`, interleaved
    source: [f32; SOURCE * 2],
    source_start: usize,
    source_len: usize,
}

impl Voice {
    fn new() -> Result<Self> {
        Ok(Self {
            active: false,
            sample: 0,
            sample_rate: 1,
            total: 0,
            channels: 1,
            attenuation: 1.0,
            pitch_track: false,
            looping: None,
            adsr: Adsr::idle(),
            pos: 0,
            sample_freq: 0.0,
            freq_factor: 1.0,
            resampler: Resampler::new(QUALITY, 2, 1.0)?,
            source: [0.0; SOURCE * 2],
            source_start: 0,
            source_len: 0,
        })
    }

    /// Plays `region`'s sample from its start at `desired` Hz (the legacy `GigSample::start`). Allocates nothing
    fn start(&mut self, patch: &Patch, sample: usize, region: &lm_sys_gig::DimRegion, attenuation: f32, desired: f32) {
        let Some(info) = patch.sample(sample) else { return };
        self.active = true;
        self.sample = sample;
        self.sample_rate = info.rate.max(1);
        self.total = info.frames as i64;
        self.channels = info.channels as usize;
        self.attenuation = attenuation;
        self.pitch_track = region.pitch_track;
        self.looping = region.loop_.map(|l| (l.kind, i64::from(l.start), i64::from(l.length)));
        self.pos = 0;
        self.sample_freq = 0.0;
        self.freq_factor = 1.0;
        let _ = self.resampler.reset();
        self.source_start = 0;
        self.source_len = 0;
        if self.pitch_track {
            // The frequency the provided sample is
            self.sample_freq =
                440.0 * ((f32::from(region.unity_note) - 69.0 - region.fine_tune as f32 * 0.01) / 12.0).exp2();
            self.freq_factor = self.sample_freq / desired.max(1.0e-3);
        }
        // The rate the envelope counts at is affected by how the note is resampled, so that a 1.5 s release is 1.5 s
        let rate = (self.sample_rate as f32 / self.freq_factor) as i64;
        self.adsr = Adsr::from_region(&region.envelope, rate);
    }

    /// The track's envelope instead of the file's (at the rate the sample plays at)
    fn use_envelope(&mut self, shape: &Shape) {
        let rate = (self.sample_rate as f32 / self.freq_factor) as i64;
        self.adsr.use_envelope(shape, rate);
    }

    /// Follows the note to `frequency` (a pitch-tracked sample)
    fn set_frequency(&mut self, frequency: f32) {
        if self.pitch_track && frequency > 0.0 {
            self.freq_factor = self.sample_freq / frequency;
        }
    }

    /// The resampling ratio (output frames per source frame)
    fn ratio(&self, rate: f32) -> f64 {
        let mut factor = f64::from(rate) / f64::from(self.sample_rate);
        if self.pitch_track {
            factor *= f64::from(self.freq_factor);
        }
        factor.clamp(1.0e-3, 1.0e3)
    }

    /// The next `SOURCE` frames of the sample, looped where the region loops (the legacy `loadSample`), with the
    /// attenuation and the envelope applied, in `source`
    fn load(&mut self, patch: &Patch) {
        let data = patch.sample_data(self.sample);
        let n = SOURCE as i64;
        let total = (data.len() / self.channels.max(1)) as i64;
        self.source.fill(0.0);
        let (channels, source) = (self.channels, &mut self.source);
        let mut copy = |to: i64, from: i64, count: i64| -> i64 {
            let count = count.min(total - from).clamp(0, n - to);
            for j in 0..count {
                let (src, dst) = (((from + j) as usize) * channels, ((to + j) as usize) * 2);
                let (l, r) = (data[src], if channels == 2 { data[src + 1] } else { data[src] });
                source[dst] = l;
                source[dst + 1] = r;
            }
            count
        };
        match self.looping {
            Some((kind, start, length)) if length > 0 && (self.pos >= start || self.pos + n > start) => {
                let end = start + length;
                self.pos = match kind {
                    LoopType::Bidirectional => ping_pong_index(self.pos, start, end),
                    _ => looped_index(self.pos, start, end),
                };
                // Round the loop from the position (as libgig's ReadAndLoop)
                let loop_end = end.min(total);
                let (mut position, mut done) = (self.pos, 0i64);
                while done < n && loop_end > start {
                    if position >= loop_end {
                        position = start;
                    }
                    let read = copy(done, position, (n - done).min(loop_end - position));
                    if read <= 0 {
                        break;
                    }
                    done += read;
                    position += read;
                }
            }
            _ => {
                if self.pos < total {
                    copy(0, self.pos, n);
                }
            }
        }
        for frame in self.source.chunks_exact_mut(2) {
            let g = self.attenuation * self.adsr.value();
            frame[0] *= g;
            frame[1] *= g;
        }
        self.pos += n;
        self.source_start = 0;
        self.source_len = SOURCE;
    }

    /// Adds `left.len()` frames of this voice to the outputs
    fn mix(&mut self, patch: &Patch, rate: f32, left: &mut [f32], right: &mut [f32]) {
        if self.resampler.set_ratio_now(self.ratio(rate)).is_err() {
            return;
        }
        let frames = left.len();
        let mut mixed = 0usize;
        let mut tmp = [0.0f32; MIX * 2];
        while mixed < frames {
            if self.source_len == 0 {
                self.load(patch);
            }
            let want = (frames - mixed).min(MIX);
            let input = &self.source[self.source_start * 2..(self.source_start + self.source_len) * 2];
            let Ok(done) = self.resampler.process(input, &mut tmp[..want * 2]) else { break };
            // Nothing more from this voice this block
            if done.input_frames == 0 && done.output_frames == 0 {
                break;
            }
            for i in 0..done.output_frames {
                left[mixed + i] += tmp[i * 2];
                right[mixed + i] += tmp[i * 2 + 1];
            }
            self.source_start += done.input_frames;
            self.source_len -= done.input_frames;
            mixed += done.output_frames;
        }
    }
}

/// The legacy's loop index functions (from its SampleBuffer)
fn looped_index(index: i64, start: i64, end: i64) -> i64 {
    if index < end { index } else { start + (index - start) % (end - start) }
}

fn ping_pong_index(index: i64, start: i64, end: i64) -> i64 {
    if index < end {
        return index;
    }
    let length = end - start;
    let at = (index - end) % (length * 2);
    if at < length { end - at } else { start + (at - length) }
}

// ---- the notes ----

/// What portion of a note we are in (the legacy `GigState`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// The key was just pressed
    KeyDown,
    PlayingKeyDown,
    /// The key was just released
    KeyUp,
    /// Being released: a release sample may play
    PlayingKeyUp,
    Completed,
}

/// A note: one key, with the voices of its samples
#[derive(Clone, Copy, Debug)]
struct Note {
    id: NoteId,
    key: u8,
    velocity: u8,
    /// A release sample plays at the key up
    release: bool,
    /// This is a release sample
    is_release: bool,
    state: State,
    frequency: f32,
    voices: [u16; MOST_SAMPLES],
    voice_count: usize,
}

fn frequency_of(key: f32) -> f32 {
    440.0 * ((key - 69.0) / 12.0).exp2()
}

/// What the notes share: the patch, the voices and the settings they are chosen with
struct Engine {
    patch: Option<Patch>,
    rate: f32,
    voices: Vec<Voice>,
    free: Vec<u16>,
    /// libgig's key dimension fraction (the legacy `m_currentKeyDimension`)
    key_dimension: f32,
    random: u32,
    pedal: bool,
    shape: Shape,
}

impl Engine {
    fn take_voice(&mut self) -> Option<u16> {
        self.free.pop()
    }

    fn give_voice(&mut self, index: u16) {
        self.voices[usize::from(index)].active = false;
        self.free.push(index);
    }

    /// The dimension values for a region (the legacy `getDimensions`), and whether it has a release-trigger dimension
    fn dimensions(&mut self, region: &lm_sys_gig::Region, velocity: u8, release: bool) -> ([u32; 8], bool) {
        let mut values = [0u32; 8];
        let mut has_release = false;
        for i in (0..region.dimensions.len().min(8)).rev() {
            let d = region.dimensions[i];
            values[i] = match d.kind {
                DimensionKind::Velocity => u32::from(velocity),
                DimensionKind::ReleaseTrigger => {
                    has_release = true;
                    u32::from(release)
                }
                DimensionKind::Keyboard => (self.key_dimension * d.zones as f32) as u32,
                // The samples recorded with the pedal down (a piano's strings resonating): a controller's value
                DimensionKind::SustainPedal => {
                    if self.pedal {
                        127
                    } else {
                        0
                    }
                }
                DimensionKind::Random => {
                    // From LinuxSampler, as the legacy had it
                    self.random = self.random.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    (self.random as f32 / 4_294_967_296.0 * d.bits as f32) as u32
                }
                // (layers and round robin: the first, as the legacy did)
                DimensionKind::Layer | DimensionKind::RoundRobin | DimensionKind::Other => 0,
            };
        }
        (values, has_release)
    }

    /// The samples a note plays, normal or release: a voice for each region its key falls in (the legacy
    /// `addSamples`). Allocates nothing
    fn add_samples(&mut self, note: &mut Note, want_release: bool) {
        let Some(patch) = self.patch.take() else { return };
        // The key dimension, from the instrument's range (the legacy chose it for release samples only)
        let (low, high) = patch.key_dimension();
        let key = note.key;
        if want_release && key >= low && key <= high {
            self.key_dimension = f32::from(key - low) / (f32::from(high - low) + 1.0);
        }
        note.is_release = want_release;
        for r in 0..patch.regions().len() {
            let Some(region) = patch.region(r) else { continue };
            let (values, has_release) = self.dimensions(region, note.velocity, want_release);
            if key < region.key_low || key > region.key_high {
                continue;
            }
            // A note with a release dimension plays release samples at its key up
            if has_release && !want_release {
                note.release = true;
            }
            let Some(selected) = patch.select(r, &values) else { continue };
            let Some(dim) = patch.dimregion(selected) else { continue };
            let Some(sample) = dim.sample else { continue };
            let Some(info) = patch.sample(sample) else { continue };
            if info.frames == 0 {
                continue;
            }
            let mut attenuation = dim.velocity_attenuation[usize::from(note.velocity.min(127))];
            let length = info.frames as f32 / self.rate;
            if want_release {
                // From LinuxSampler, as the legacy had it (never below 0: a negative gain would flip the sound)
                let decay = 256u32 >> u32::from(dim.release_trigger_decay.min(8));
                attenuation *= (1.0 - 0.01053 * decay as f32 * length).max(0.0);
            } else {
                attenuation *= dim.sample_attenuation;
            }
            // A voice made in advance (none left, or the note has all it can hold: this sample is not played)
            if note.voice_count >= MOST_SAMPLES {
                continue;
            }
            let Some(index) = self.take_voice() else { continue };
            let voice = &mut self.voices[usize::from(index)];
            voice.start(&patch, sample, dim, attenuation, note.frequency);
            if self.shape.amount > 0.0 {
                voice.use_envelope(&self.shape);
            }
            note.voices[note.voice_count] = index;
            note.voice_count += 1;
        }
        self.patch = Some(patch);
    }
}

/// The synthesizer of a GigPlayer
pub struct Gig {
    engine: Engine,
    values: [f32; k::COUNT],
    notes: Vec<Note>,
}

impl std::fmt::Debug for Gig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gig")
            .field("patch", &self.engine.patch)
            .field("notes", &self.notes.len())
            .finish_non_exhaustive()
    }
}

impl Gig {
    /// A synthesizer for `patch` (none: silent) at `rate`, with every voice and resampler made now (build time: the
    /// audio thread allocates nothing)
    fn new(patch: Option<Patch>, values: [f32; k::COUNT], rate: f32) -> Result<Self> {
        let mut voices = Vec::with_capacity(MOST_VOICES);
        let mut free = Vec::with_capacity(MOST_VOICES);
        if patch.is_some() {
            for i in 0..MOST_VOICES {
                voices.push(Voice::new()?);
                free.push((MOST_VOICES - 1 - i) as u16);
            }
        }
        let mut gig = Self {
            engine: Engine {
                patch,
                rate,
                voices,
                free,
                key_dimension: 0.0,
                random: 0,
                pedal: false,
                shape: Shape::default(),
            },
            values,
            notes: Vec::with_capacity(MOST_NOTES),
        };
        gig.engine.shape = gig.shape();
        Ok(gig)
    }

    fn shape(&self) -> Shape {
        let e = &self.values[k::ENV..k::ENV + 7];
        Shape { predelay: e[0], attack: e[1], hold: e[2], decay: e[3], sustain: e[4], release: e[5], amount: e[6] }
    }

    fn find(&self, id: NoteId) -> Option<usize> {
        self.notes.iter().position(|n| n.id == id)
    }

    /// Every note's voices back to the pool, and no note left
    fn clear_notes(&mut self) {
        for note in &mut self.notes {
            for i in 0..note.voice_count {
                self.engine.give_voice(note.voices[i]);
            }
            note.voice_count = 0;
        }
        self.notes.clear();
    }

    /// The changes of a period's start: keys down and up become samples, ended voices go back to the pool, ended
    /// notes go (the legacy `playImpl`)
    fn begin_block(&mut self) {
        let mut i = 0;
        while i < self.notes.len() {
            let mut note = self.notes[i];
            match note.state {
                State::KeyUp => {
                    if note.voice_count == 0 {
                        note.state = State::Completed;
                    } else {
                        note.state = State::PlayingKeyUp;
                        for v in 0..note.voice_count {
                            self.engine.voices[usize::from(note.voices[v])].adsr.keyup();
                        }
                        if note.release {
                            self.engine.add_samples(&mut note, true);
                        }
                    }
                }
                State::KeyDown => {
                    note.state = State::PlayingKeyDown;
                    self.engine.add_samples(&mut note, false);
                }
                _ => {}
            }
            // Ended voices: for normal notes when the envelope is complete, for a release sample at its end
            let mut v = 0;
            while v < note.voice_count {
                let index = note.voices[v];
                let voice = &self.engine.voices[usize::from(index)];
                if !voice.active || voice.adsr.done() || (note.is_release && voice.pos >= voice.total - 1) {
                    self.engine.give_voice(index);
                    note.voice_count -= 1;
                    note.voices[v] = note.voices[note.voice_count];
                } else {
                    v += 1;
                }
            }
            if note.state == State::Completed || note.voice_count == 0 {
                self.notes.swap_remove(i);
            } else {
                self.notes[i] = note;
                i += 1;
            }
        }
    }
}

impl Stream for Gig {
    const PARAMS: usize = k::COUNT;

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(slot) = self.values.get_mut(index) {
            *slot = value;
        }
        if index >= k::ENV {
            self.engine.shape = self.shape();
        }
    }

    fn note_on(&mut self, note: NoteId, key: u8, velocity: f32, detune: f32) {
        // Out of range (the legacy dropped key 0 too), or no room for it
        if key == 0 || key > 127 || self.notes.len() == self.notes.capacity() || self.engine.patch.is_none() {
            return;
        }
        let velocity = (f64::from(velocity) * 127.0 + 0.5).clamp(1.0, 127.0) as u8;
        self.notes.push(Note {
            id: note,
            key,
            velocity,
            release: false,
            is_release: false,
            state: State::KeyDown,
            frequency: frequency_of(f32::from(key) + detune),
            voices: [0; MOST_SAMPLES],
            voice_count: 0,
        });
    }

    fn detune(&mut self, note: NoteId, semitones: f32) {
        if let Some(i) = self.find(note) {
            self.notes[i].frequency = frequency_of(f32::from(self.notes[i].key) + semitones);
        }
    }

    fn note_off(&mut self, note: NoteId) {
        let Some(i) = self.find(note) else { return };
        // Only a note that was playing or just pressed
        if matches!(self.notes[i].state, State::KeyDown | State::PlayingKeyDown) {
            self.notes[i].state = State::KeyUp;
        }
    }

    // The legacy ignores the pitch bend (`IsNotBendable`)
    fn bend(&mut self, _bend: f32, _range: f32) {}

    fn pedal(&mut self, down: bool) {
        self.engine.pedal = down;
    }

    fn release_all(&mut self) {
        for note in &mut self.notes {
            if matches!(note.state, State::KeyDown | State::PlayingKeyDown) {
                note.state = State::KeyUp;
            }
        }
    }

    fn sound_off(&mut self) {
        self.clear_notes();
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        left.fill(0.0);
        right.fill(0.0);
        // The legacy's instrument not found: silence
        if self.engine.patch.is_none() {
            return;
        }
        self.begin_block();
        let Engine { patch, voices, rate, .. } = &mut self.engine;
        let Some(patch) = patch.as_ref() else { return };
        for note in &self.notes {
            if !matches!(note.state, State::PlayingKeyDown | State::PlayingKeyUp) {
                continue;
            }
            for v in 0..note.voice_count {
                let voice = &mut voices[usize::from(note.voices[v])];
                if !voice.active {
                    continue;
                }
                // The note's pitch now, while it is held
                voice.set_frequency(note.frequency);
                voice.mix(patch, *rate, left, right);
            }
        }
        let gain = self.values[k::GAIN];
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            *l *= gain;
            *r *= gain;
        }
    }

    fn reset(&mut self) {
        self.clear_notes();
        self.engine.pedal = false;
        self.engine.key_dimension = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_index_functions_wrap_as_the_legacy_ones() {
        assert_eq!(looped_index(5, 10, 20), 5);
        assert_eq!(looped_index(20, 10, 20), 10);
        assert_eq!(looped_index(35, 10, 20), 15);
        assert_eq!(ping_pong_index(5, 10, 20), 5);
        // Past the end it goes back: 20 is the end, 21 one frame before it
        assert_eq!(ping_pong_index(21, 10, 20), 19);
        assert_eq!(ping_pong_index(30, 10, 20), 10);
        assert_eq!(ping_pong_index(31, 10, 20), 11);
    }

    #[test]
    fn the_envelope_attacks_sustains_and_releases() {
        let env = lm_sys_gig::Envelope {
            preattack: 0.0,
            attack: 0.01,
            decay1: 0.0,
            decay2: 0.0,
            infinite_sustain: true,
            sustain: 500.0,
            release: 0.01,
        };
        let mut a = Adsr::from_region(&env, 1000);
        // A 10-frame attack from silence to the full amplitude
        let first = a.value();
        assert_eq!(first, 0.0);
        let mut last = first;
        for _ in 0..9 {
            let v = a.value();
            assert!(v >= last);
            last = v;
        }
        // Then the sustain (no decay: the legacy jumps to it)
        for _ in 0..20 {
            a.value();
        }
        a.keyup();
        let mut frames = 0;
        while !a.done() && frames < 1000 {
            a.value();
            frames += 1;
        }
        assert!(a.done(), "the release ends");
        assert!(frames <= 12, "in about the release time: {frames}");
        assert_eq!(a.value(), 0.0);
    }

    #[test]
    fn a_note_plays_the_sample_and_ends_after_its_release() {
        let path = std::env::temp_dir().join(format!("lm-plugins-gigplayer-unit-{}.gig", std::process::id()));
        lm_sys_gig::test_file(&path).expect("the test file");
        let patch = Patch::load(&path, 0, 0).expect("the patch");
        let _ = std::fs::remove_file(&path);
        let mut values = [0.0f32; k::COUNT];
        values[k::GAIN] = 1.0;
        let mut gig = Gig::new(Some(patch), values, 44_100.0).expect("the synthesizer");
        gig.note_on(NoteId(1), 69, 1.0, 0.0);
        let (mut l, mut r) = ([0.0f32; 32], [0.0f32; 32]);
        let mut peak = 0.0f32;
        for _ in 0..64 {
            gig.render(&mut l, &mut r);
            peak = l.iter().chain(r.iter()).fold(peak, |m, v| m.max(v.abs()));
        }
        assert!(peak > 0.05 && peak < 1.0, "the sine plays at a sane level: {peak}");
        gig.note_off(NoteId(1));
        for _ in 0..200 {
            gig.render(&mut l, &mut r);
        }
        assert!(gig.notes.is_empty(), "the note ended after its release");
        assert!(l.iter().chain(r.iter()).all(|v| v.abs() < 1.0e-4), "and the sound with it");
    }

    #[test]
    fn a_synthesizer_without_a_patch_is_silent() {
        let mut gig = Gig::new(None, [1.0; k::COUNT], 44_100.0).expect("the synthesizer");
        gig.note_on(NoteId(1), 69, 1.0, 0.0);
        let (mut l, mut r) = ([1.0f32; 32], [1.0f32; 32]);
        gig.render(&mut l, &mut r);
        assert!(l.iter().chain(r.iter()).all(|v| *v == 0.0));
    }
}
