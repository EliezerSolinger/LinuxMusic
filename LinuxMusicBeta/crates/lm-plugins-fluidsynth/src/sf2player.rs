//! The Sf2 Player (F-INS-014; legacy `plugins/Sf2Player`): a SoundFont (`.sf2`, `.sf3`) played by FluidSynth, one
//! synthesizer per instrument, with the note functions of a single-stream instrument (`lm_plugin::instrument::
//! stream`: chords, arpeggio, mono, legato and glide, the sustain pedal, the pitch knob and the bend).
//!
//! Its state, version 1:
//! - `soundfont`: the SoundFont, a resource of kind `"soundfont"` (null: silent, as a legacy instrument before a
//!   file was opened; the session gives a new instrument the default SoundFont of the preferences);
//! - parameters with the legacy knobs' ranges and defaults: `bank` (0–999), `patch` (0–127), `gain` (0–5, 1),
//!   `reverbOn`, `reverbRoomSize` (0.2), `reverbDamping` (0), `reverbWidth` (0.5), `reverbLevel` (0.9),
//!   `chorusOn`, `chorusNum` (0–10, 3), `chorusLevel` (0–10, 2), `chorusSpeed` (0.29–5 Hz, 0.3), `chorusDepth`
//!   (0–46 ms, 8) — the reverb's and chorus's defaults are FluidSynth's, as the legacy read them;
//! - the volume envelope the legacy took from the track's ENV tab (`volPredelay`, `volAttack`, `volHold`,
//!   `volDecay`, `volSustain`, `volRelease`, `volAmount`, the instrument functions' keys and ranges): with an amount
//!   above 0 it takes the place of the SoundFont's own on every voice;
//! - the note functions.
//!
//! As in the legacy instrument: the notes play on MIDI channel 1; a key outside 0–127 is dropped; two notes of one
//! key end the sound with the last one; a note's detune (the pitch knob, a glide, legato) and its pan (a note
//! expression) are set on its voices while they play (FluidSynth's coarse tune and balance); the bend is the
//! channel's, with its range. A bank and a patch the font lacks play nothing (FluidSynth keeps the channel silent).

use lm_base::{Error, Result};
use lm_plugin::instrument::{Stream, StreamInstrument, add_note_functions, instrument_params};
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{NoteId, Processor};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Resource, Scale, Value};
use lm_sys_fluidsynth::{Chorus, Generator, Reverb, SoundFontId, Synth, SynthBuilder, SynthConfig, VoiceRef};

use crate::avldrums::midi_velocity;
use crate::half_rate::{Renderer, synth_rate};

pub const ID: &str = "sf2player";
/// The MIDI channel the notes play on (the legacy's)
const CHANNEL: usize = 1;
/// FluidSynth's default polyphony, the legacy's
const POLYPHONY: usize = 256;
/// Voices a note may start (the legacy kept four; the fonts it checked used at most two)
const VOICES_PER_NOTE: usize = 4;
/// Notes followed at once (sounding notes of the note functions)
const NOTES: usize = 1024;

/// The parameters, in index order
mod k {
    pub const BANK: usize = 0;
    pub const PATCH: usize = 1;
    pub const GAIN: usize = 2;
    pub const REVERB_ON: usize = 3;
    pub const REVERB_ROOM: usize = 4;
    pub const REVERB_DAMP: usize = 5;
    pub const REVERB_WIDTH: usize = 6;
    pub const REVERB_LEVEL: usize = 7;
    pub const CHORUS_ON: usize = 8;
    pub const CHORUS_NUM: usize = 9;
    pub const CHORUS_LEVEL: usize = 10;
    pub const CHORUS_SPEED: usize = 11;
    pub const CHORUS_DEPTH: usize = 12;
    /// The volume envelope: predelay, attack, hold, decay, sustain, release, amount
    pub const ENV: usize = 13;
    pub const COUNT: usize = ENV + 7;
}

const ENV_KEYS: [&str; 7] =
    ["volPredelay", "volAttack", "volHold", "volDecay", "volSustain", "volRelease", "volAmount"];

fn switch(key: &'static str) -> ParamSpec {
    ParamSpec::new(key, 0.0, 1.0, 0.0).scale(Scale::Stepped)
}

fn own_params() -> Vec<ParamSpec> {
    let mut v = vec![
        ParamSpec::new("bank", 0.0, 999.0, 0.0).scale(Scale::Stepped),
        ParamSpec::new("patch", 0.0, 127.0, 0.0).scale(Scale::Stepped),
        ParamSpec::new("gain", 0.0, 5.0, 1.0),
        switch("reverbOn"),
        ParamSpec::new("reverbRoomSize", 0.0, 1.0, 0.2),
        ParamSpec::new("reverbDamping", 0.0, 1.0, 0.0),
        ParamSpec::new("reverbWidth", 0.0, 1.0, 0.5),
        ParamSpec::new("reverbLevel", 0.0, 1.0, 0.9),
        switch("chorusOn"),
        ParamSpec::new("chorusNum", 0.0, 10.0, 3.0).scale(Scale::Stepped),
        ParamSpec::new("chorusLevel", 0.0, 10.0, 2.0),
        ParamSpec::new("chorusSpeed", 0.29, 5.0, 0.3).unit("Hz"),
        ParamSpec::new("chorusDepth", 0.0, 46.0, 8.0).unit("ms"),
    ];
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
    name: "plugin.sf2player.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    // FluidSynth's velocity curve and the font's velocity layers
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

/// The resource a test plays (any key: the tests' setup gives every SoundFont resource the test font)
pub fn test_soundfont() -> Resource {
    Resource::new("soundfont", &"5f".repeat(32)).unwrap_or_else(|_| unreachable!("a valid key"))
}

/// The tuning probe: the test font's sine at 440 Hz on key 69
fn probe_state() -> Value {
    Value::empty_object().with("soundfont", Value::resource(test_soundfont()))
}

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:sf2player", 1)
        .field(FieldSpec::new("soundfont", FieldKind::Resource("soundfont")).nullable().default(Value::NULL));
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
    let rate = cx.spec.sample_rate;
    let (synth_rate, _) =
        synth_rate(rate).ok_or_else(|| Error::new("plugin.sf2player.rate").arg("rate", i64::from(rate)))?;
    let mut values = [0.0f32; k::COUNT];
    for (i, p) in own_params().iter().enumerate() {
        values[i] = s.param_f32(p.key)?;
    }
    let config = SynthConfig {
        sample_rate: f64::from(synth_rate),
        outputs: 1,
        midi_channels: 16,
        polyphony: POLYPHONY,
        gain: values[k::GAIN],
    };
    let mut b = SynthBuilder::new(config)?;
    let font = match s.resource("soundfont")? {
        Some(r) => {
            let path = cx.resources.file(r)?;
            Some(b.load_soundfont(&path)?)
        }
        None => None,
    };
    let synth = b.build();
    let mut stream = Sf2 {
        synth,
        font,
        values,
        renderer: Renderer::new(rate, 2, cx.spec.max_block),
        notes: Vec::with_capacity(NOTES),
        running: [0; 128],
        frames_rendered: 0,
        sent_bend: None,
    };
    stream.apply_all();
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(StreamInstrument::new(stream, &s, params, &cx.spec)?))
}

/// A note the synthesizer plays
#[derive(Clone, Copy, Debug)]
struct Note {
    id: NoteId,
    key: u8,
    voices: [VoiceRef; VOICES_PER_NOTE],
    /// The coarse tune each voice started with (the font's)
    coarse: [f32; VOICES_PER_NOTE],
    count: usize,
}

/// The synthesizer of an Sf2 Player
pub struct Sf2 {
    synth: Synth,
    font: Option<SoundFontId>,
    values: [f32; k::COUNT],
    renderer: Renderer,
    notes: Vec<Note>,
    /// Notes sounding per key: FluidSynth ends a key at once, so it is told only when the last one ends (the legacy
    /// `m_notesRunning`)
    running: [u16; 128],
    frames_rendered: u64,
    sent_bend: Option<(u16, u8)>,
}

impl std::fmt::Debug for Sf2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sf2").field("font", &self.font).field("notes", &self.notes.len()).finish_non_exhaustive()
    }
}

/// The legacy's conversion of the volume envelope's seconds to timecents
fn timecents(seconds: f32) -> f32 {
    if seconds <= 0.001 { -12_000.0 } else { (1200.0 * seconds.log2()).max(-12_000.0) }
}

/// The legacy's conversion of a pan (−1 to 1) to FluidSynth's balance
fn balance(pan: f32) -> f32 {
    // The smallest balance that silences one side (FluidSynth's FLUID_CB_AMP_SIZE)
    const MAX: f32 = 1441.0;
    let factor = 1.0 - pan.abs().min(1.0);
    let b = if factor <= 0.0 { MAX } else { (-200.0 * factor.log10()).min(MAX) };
    b.copysign(pan)
}

impl Sf2 {
    fn on(v: f32) -> bool {
        v >= 0.5
    }

    fn select(&mut self) {
        if let Some(font) = self.font {
            let (bank, patch) = (self.values[k::BANK].max(0.0) as u32, self.values[k::PATCH].max(0.0) as u32);
            let _ = self.synth.program_select(CHANNEL, font, bank, patch);
        }
    }

    fn reverb(&mut self) {
        let v = &self.values;
        let r = Reverb {
            room_size: f64::from(v[k::REVERB_ROOM]),
            damping: f64::from(v[k::REVERB_DAMP]),
            width: f64::from(v[k::REVERB_WIDTH]),
            level: f64::from(v[k::REVERB_LEVEL]),
        };
        let _ = self.synth.set_reverb(r);
    }

    fn chorus(&mut self) {
        let v = &self.values;
        let c = Chorus {
            voices: v[k::CHORUS_NUM].max(0.0) as u32,
            level: f64::from(v[k::CHORUS_LEVEL]),
            speed: f64::from(v[k::CHORUS_SPEED]),
            depth: f64::from(v[k::CHORUS_DEPTH]),
        };
        let _ = self.synth.set_chorus(c);
    }

    /// Every setting, as the legacy `applySettings` (build time)
    fn apply_all(&mut self) {
        self.select();
        self.synth.set_gain(self.values[k::GAIN]);
        let _ = self.synth.set_reverb_on(Self::on(self.values[k::REVERB_ON]));
        self.reverb();
        let _ = self.synth.set_chorus_on(Self::on(self.values[k::CHORUS_ON]));
        self.chorus();
    }

    fn find(&self, id: NoteId) -> Option<usize> {
        self.notes.iter().position(|n| n.id == id)
    }
}

impl Stream for Sf2 {
    const PARAMS: usize = k::COUNT;

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(slot) = self.values.get_mut(index) else { return };
        if *slot == value {
            return;
        }
        *slot = value;
        match index {
            k::BANK | k::PATCH => self.select(),
            k::GAIN => self.synth.set_gain(value),
            k::REVERB_ON => {
                let _ = self.synth.set_reverb_on(Self::on(value));
            }
            k::REVERB_ROOM..=k::REVERB_LEVEL => self.reverb(),
            k::CHORUS_ON => {
                let _ = self.synth.set_chorus_on(Self::on(value));
            }
            k::CHORUS_NUM..=k::CHORUS_DEPTH => self.chorus(),
            // The envelope is given to the voices at their start
            _ => {}
        }
    }

    fn note_on(&mut self, note: NoteId, key: u8, velocity: f32, detune: f32) {
        if key > 127 || self.notes.len() == self.notes.capacity() {
            return;
        }
        let _ = self.synth.note_on(CHANNEL, key, midi_velocity(velocity));
        let mut n = Note { id: note, key, voices: [VoiceRef::default(); VOICES_PER_NOTE], coarse: [0.0; 4], count: 0 };
        n.count = self.synth.voices_of(CHANNEL, key, &mut n.voices);
        for i in 0..n.count {
            n.coarse[i] = self.synth.voice_gen(&n.voices[i], Generator::CoarseTune).unwrap_or(0.0);
            if detune != 0.0 {
                self.synth.set_voice_gen(&n.voices[i], Generator::CoarseTune, n.coarse[i] + detune);
            }
        }
        // The volume envelope, when it is on, in place of the font's (the legacy `noteOn`)
        let e = &self.values[k::ENV..k::ENV + 7];
        let (predelay, attack, hold, decay, sustain, release, amount) = (e[0], e[1], e[2], e[3], e[4], e[5], e[6]);
        if amount > 0.0 {
            let level = (1.0 - amount + amount * sustain).clamp(0.0, 1.0);
            let sustain_cb = if level <= 0.0 { 1440.0 } else { (-200.0 * level.log10()).clamp(0.0, 1440.0) };
            let gens = [
                (Generator::VolEnvDelay, timecents(predelay)),
                (Generator::VolEnvAttack, timecents(attack)),
                (Generator::VolEnvHold, timecents(hold)),
                (Generator::VolEnvDecay, timecents(decay)),
                (Generator::VolEnvSustain, sustain_cb),
                (Generator::VolEnvRelease, timecents(release)),
            ];
            for v in &n.voices[..n.count] {
                for (g, value) in gens {
                    self.synth.set_voice_gen(v, g, value);
                }
            }
        }
        self.running[usize::from(key)] = self.running[usize::from(key)].saturating_add(1);
        self.notes.push(n);
    }

    fn detune(&mut self, note: NoteId, semitones: f32) {
        let Some(i) = self.find(note) else { return };
        let n = self.notes[i];
        for j in 0..n.count {
            self.synth.set_voice_gen(&n.voices[j], Generator::CoarseTune, n.coarse[j] + semitones);
        }
    }

    fn pan(&mut self, note: NoteId, pan: f32) {
        let Some(i) = self.find(note) else { return };
        let n = self.notes[i];
        let b = balance(pan);
        for v in &n.voices[..n.count] {
            self.synth.set_voice_gen(v, Generator::CustomBalance, b);
        }
    }

    fn note_off(&mut self, note: NoteId) {
        let Some(i) = self.find(note) else { return };
        let n = self.notes.swap_remove(i);
        let running = &mut self.running[usize::from(n.key)];
        *running = running.saturating_sub(1);
        if *running == 0 {
            let _ = self.synth.note_off(CHANNEL, n.key);
        }
    }

    fn bend(&mut self, bend: f32, range: f32) {
        let value = (bend * 8192.0 + 8192.0).round().clamp(0.0, 16_383.0) as u16;
        let range = range.round().clamp(0.0, 127.0) as u8;
        if self.sent_bend == Some((value, range)) {
            return;
        }
        self.sent_bend = Some((value, range));
        let _ = self.synth.set_bend_range(CHANNEL, range);
        let _ = self.synth.pitch_bend(CHANNEL, value);
    }

    fn midi(&mut self, bytes: [u8; 3]) {
        // Controllers reach the channel (modulation, expression...), but not the bank select: the bank is a parameter
        if bytes[0] & 0xF0 == 0xB0 && !matches!(bytes[1], 0 | 32) {
            let _ = self.synth.cc(CHANNEL, bytes[1] & 0x7F, bytes[2] & 0x7F);
        }
    }

    fn release_all(&mut self) {
        let _ = self.synth.all_notes_off(Some(CHANNEL));
        self.notes.clear();
        self.running = [0; 128];
    }

    fn sound_off(&mut self) {
        let _ = self.synth.all_sounds_off(Some(CHANNEL));
        self.notes.clear();
        self.running = [0; 128];
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let Self { synth, renderer, frames_rendered, .. } = self;
        let mut outs: [&mut [f32]; 2] = [left, right];
        renderer.render(&mut outs, |bufs| {
            *frames_rendered += bufs.first().map_or(0, |b| b.len()) as u64;
            let (l, r) = bufs.split_at_mut(1);
            let _ = synth.render_stereo(l[0], r[0]);
        });
    }

    /// Back to rest (the legacy `reset`): every voice off at once, channel by channel, and what FluidSynth had
    /// rendered ahead of the last block dropped, so that the next render starts on a block of its own
    fn reset(&mut self) {
        for channel in 0..16 {
            let _ = self.synth.all_sounds_off(Some(channel));
        }
        self.notes.clear();
        self.running = [0; 128];
        let block = self.synth.internal_block() as u64;
        let mut rest = ((block - self.frames_rendered % block) % block) as usize;
        let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
        while rest > 0 {
            let n = rest.min(64);
            let _ = self.synth.render_stereo(&mut l[..n], &mut r[..n]);
            rest -= n;
        }
        self.frames_rendered = 0;
        self.renderer.reset();
        self.sent_bend = None;
    }
}
