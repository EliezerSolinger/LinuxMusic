//! The Sampler (decisoes-autonomas.md, 26/11, item 3): one instrument that replaces the Sf2 Player, the GigPlayer,
//! PatMan, the AudioFileProcessor, AvlDrums and the DrumMachines. It plays the engine of `lm-sampler`; the format is
//! a choice of the state, read in the build (an `Operation`, off the audio thread, with the previous sound playing on
//! until the new one is ready).
//!
//! State, version 1:
//! - `format`: `none`, `sf2`, `sfz`, `pat`, `audio`, `kit` or `gig` (a `.gig` file read in pure Rust; `bank` and `program` choose the
//!   instrument; a compressed sample fails the build with `sampler.gig.compressed`);
//! - the source, one nullable resource field for each format: `soundfont` (`.sf2`, `.sf3`), `sfz`, `patch` (`.pat`),
//!   `sample` (an audio file) and `gig`; `kit` is one of the shipped kits (the AVL drum kits and the DrumMachines'),
//!   program data and not a resource;
//! - the preset in a SoundFont: `bank` and `program`;
//! - `outputs`: `main`, or `groups` for a drum kit (kick, snare, hi-hat, tom, floor tom, cymbals, percussion);
//! - `interpolation`: `linear`, `cubic` or `sinc`; for an audio file: `loop` (`off`, `on`, `pingPong`); for a patch:
//!   `patchTuned` and `patchLooped`;
//! - parameters (MIDI-mappable and automatable): `gain`, `pan`, `tune`, `polyphony`, `bendRange`, the volume
//!   envelope that replaces the instrument's own when `volAmount` is above 0 (`volPredelay`, `volAttack`,
//!   `volHold`, `volDecay`, `volSustain`, `volRelease`, `volAmount`), and the audio file's `startPoint`, `endPoint`,
//!   `loopPoint`, `reverse` and `stutter`; the voice functions: `voiceMode` (0 poly, 1 mono, 2 legato) and `glide` (ms).
//!
//! What the old plugins did that the Sampler leaves to the note effects (decisoes-autonomas.md, item 2): chords and
//! the arpeggio. Mono, legato and glide are the Sampler's own (`voiceMode`, `glide`): they belong to the voice.

#![forbid(unsafe_code)]

pub mod kits;
pub mod load;
mod processor;
mod source;

use std::sync::LazyLock;

use lm_base::Result;
use lm_plugin::instrument::instrument_params;
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

pub use processor::Sampler;
pub use source::{Format, Source};

pub const ID: &str = "sampler";

pub const FORMAT_KEYS: [&str; 7] = ["none", "sf2", "sfz", "pat", "audio", "kit", "gig"];
pub const OUTPUT_KEYS: [&str; 2] = ["main", "groups"];
pub const LOOP_KEYS: [&str; 3] = ["off", "on", "pingPong"];
pub const INTERPOLATION_KEYS: [&str; 3] = ["linear", "cubic", "sinc"];

static FORMATS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| FORMAT_KEYS.into());
static OUTPUTS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| OUTPUT_KEYS.into());
static LOOPS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| LOOP_KEYS.into());
static INTERPOLATIONS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| INTERPOLATION_KEYS.into());
static KITS: LazyLock<Box<[&'static str]>> =
    LazyLock::new(|| kits::AVL_KITS.iter().map(|k| k.key).chain(kits::DM_KITS.iter().map(|k| k.key)).collect());

/// The parameters, in index order (the events' `index`)
pub mod p {
    pub const GAIN: usize = 0;
    pub const PAN: usize = 1;
    pub const TUNE: usize = 2;
    pub const POLYPHONY: usize = 3;
    pub const BEND_RANGE: usize = 4;
    /// Predelay, attack, hold, decay, sustain, release, amount
    pub const ENV: usize = 5;
    pub const START: usize = 12;
    pub const END: usize = 13;
    pub const LOOP: usize = 14;
    pub const REVERSE: usize = 15;
    pub const STUTTER: usize = 16;
    /// 0 poly, 1 mono, 2 legato
    pub const VOICE_MODE: usize = 17;
    /// Milliseconds
    pub const GLIDE: usize = 18;
    pub const COUNT: usize = 19;
}

pub const ENV_KEYS: [&str; 7] =
    ["volPredelay", "volAttack", "volHold", "volDecay", "volSustain", "volRelease", "volAmount"];

fn switch(key: &'static str) -> ParamSpec {
    ParamSpec::new(key, 0.0, 1.0, 0.0).scale(Scale::Stepped)
}

fn own_params() -> Vec<ParamSpec> {
    let mut v = vec![
        ParamSpec::new("gain", -60.0, 24.0, 0.0).unit("dB"),
        ParamSpec::new("pan", -1.0, 1.0, 0.0),
        ParamSpec::new("tune", -1200.0, 1200.0, 0.0).unit("cents"),
        ParamSpec::new("polyphony", 1.0, 256.0, 64.0).scale(Scale::Stepped),
        ParamSpec::new("bendRange", 0.0, 60.0, 2.0).scale(Scale::Stepped).unit("st"),
    ];
    for key in ENV_KEYS {
        if let Some(spec) = instrument_params().iter().find(|s| s.key == key) {
            v.push(spec.clone());
        }
    }
    v.extend([
        ParamSpec::new("startPoint", 0.0, 1.0, 0.0),
        ParamSpec::new("endPoint", 0.0, 1.0, 1.0),
        ParamSpec::new("loopPoint", 0.0, 1.0, 0.0),
        switch("reverse"),
        switch("stutter"),
        ParamSpec::new("voiceMode", 0.0, 2.0, 0.0).scale(Scale::Stepped),
        ParamSpec::new("glide", 0.0, 2000.0, 0.0).unit("ms"),
    ]);
    v
}

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.sampler.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    // The velocity layers of a SoundFont, the filter of a zone and the level all follow it
    velocity: Velocity::Timbre,
    display: None,
    build,
    tests: TestHints {
        tuning: None,
        // Another output layout is other ports: the processor is replaced, not adopted
        no_adopt: &["outputs"],
        voices: 64,
        max_level: 1.0e3,
        tail: lm_plugin::DEFAULT_TAIL,
    },
};

fn resource(key: &'static str, kind: &'static str) -> FieldSpec {
    FieldSpec::new(key, FieldKind::Resource(kind)).nullable().default(Value::NULL)
}

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:sampler", 1)
        .field(FieldSpec::choice("format", &FORMATS, "none"))
        .field(resource("soundfont", "soundfont"))
        .field(resource("sfz", "sfz"))
        .field(resource("patch", "pat"))
        .field(resource("sample", "audio"))
        .field(resource("gig", "gig"))
        .field(FieldSpec::choice("kit", &KITS, kits::AVL_KITS[0].key))
        .field(FieldSpec::int("bank", 0, 16383, 0))
        .field(FieldSpec::int("program", 0, 127, 0))
        .field(FieldSpec::choice("outputs", &OUTPUTS, "main"))
        .field(FieldSpec::choice("interpolation", &INTERPOLATIONS, "sinc"))
        .field(FieldSpec::choice("loop", &LOOPS, "off"))
        .field(FieldSpec::bool("patchTuned", true))
        .field(FieldSpec::bool("patchLooped", true));
    for p in own_params() {
        s = s.param(p);
    }
    vec![s]
}

/// Whether a state plays a drum kit with an output per group
pub fn multi(state: &Value) -> bool {
    let text = |k: &str| state.get(k).and_then(Value::as_text);
    if text("format") != Some("kit") || text("outputs") != Some("groups") {
        return false;
    }
    match text("kit") {
        Some(key) => match kits::AvlKit::from_key(key) {
            Some(avl) => avl.multi_output,
            None => kits::DmKit::from_key(key).is_some(),
        },
        None => false,
    }
}

/// The ports of the drum groups: the kick, snare, hi-hat, tom and floor tom mono, the cymbals and percussion stereo
pub fn group_ports() -> Vec<Port> {
    lm_sampler::import::avl::GROUPS.iter().map(|(name, stereo)| Port::new(name, if *stereo { 2 } else { 1 })).collect()
}

fn ports(state: &Value) -> Ports {
    let outputs = if multi(state) { group_ports() } else { vec![Port::new("main", 2)] };
    Ports { outputs, sidechains: Vec::new() }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let built = source::build(state, cx)?;
    Ok(Box::new(processor::Sampler::new(built, state, cx)?))
}
