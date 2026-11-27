//! The schemas of the project's node types (interfaces.md, I-3.3, I-3.9; arquitetura-nova.md, piece 2 and the base
//! spec, items 2 and 3).
//!
//! The root of a project is a `"project"`: its `meta` (the id counter and the project's uuid), the settings, the
//! transport, the tempo map, the **tracks** (the timeline: items, automation, the instrument and the outputs; volume,
//! mute and solo, no pan) and the **mixing desk** (channels with their fixed rack, inserts, sends, pan; the master).
//! A track's outputs route freely to the desk's channels (`"to": "m:3"`).
//!
//! Every type is at version 1. Their defaults are frozen by the reference file `tests/data/defaults.json` (U29).
//!
//! The kind letters of the ids: `t` track, `o` track output, `c` item, `l` item lane, `a` automation lane, `p` plugin
//! (instrument or insert), `m` mixer channel (the master is always `m:0`), `s` send, `k` MIDI mapping (I-21.2), `g`
//! marker and `q` region of the ruler (F-REG-011, F-REG-012), `n` modulator and `j` its target (a parameter it moves).
//! The root is `r:0`.

use lm_base::Result;
use lm_state::{
    FieldKind, FieldSpec, MAX_SAFE_INT, NodeId, NodeSchema, OnRemove, ParamSpec, Scale, SchemaRegistry, Value,
};

/// The master channel: always `m:0`, never removed
pub const MASTER: NodeId = NodeId { kind: b'm', number: 0 };

/// The type of the project's root
pub const PROJECT: &str = "project";
/// The longest project notes the core accepts, in bytes (a document is not a place for a book)
pub const NOTES_MAX: usize = 64 * 1024;

pub const KIND_TRACK: u8 = b't';
pub const KIND_OUTPUT: u8 = b'o';
pub const KIND_ITEM: u8 = b'c';
pub const KIND_LANE: u8 = b'l';
pub const KIND_AUTOMATION: u8 = b'a';
pub const KIND_PLUGIN: u8 = b'p';
pub const KIND_CHANNEL: u8 = b'm';
pub const KIND_SEND: u8 = b's';
pub const KIND_MAPPING: u8 = b'k';
pub const KIND_MARKER: u8 = b'g';
pub const KIND_REGION: u8 = b'q';
pub const KIND_MODULATOR: u8 = b'n';
pub const KIND_MOD_TARGET: u8 = b'j';

/// The type names, for code that reads the tree
pub mod types {
    pub const META: &str = "meta";
    pub const SETTINGS: &str = "project.settings";
    pub const TRANSPORT: &str = "transport";
    pub const RANGE: &str = "transport.range";
    pub const PREROLL: &str = "transport.preroll";
    pub const METRONOME: &str = "transport.metronome";
    pub const TEMPO_POINT: &str = "tempo.point";
    pub const TRACK: &str = "track";
    pub const TRACK_INPUT: &str = "track.input";
    pub const OUTPUT: &str = "track.output";
    pub const ITEM: &str = "item";
    pub const LANE: &str = "item.lane";
    pub const LANE_TIMING_POINT: &str = "item.lane.timing";
    pub const LANE_GAP: &str = "item.lane.gap";
    pub const COMP: &str = "item.comp";
    pub const NOTE: &str = "note";
    pub const NOTE_BEND: &str = "note.bend";
    pub const NOTE_PRESSURE: &str = "note.pressure";
    pub const NOTE_TIMBRE: &str = "note.timbre";
    pub const AUTOMATION: &str = "automation";
    pub const AUTOMATION_TARGET: &str = "automation.target";
    pub const AUTOMATION_POINT: &str = "automation.point";
    pub const PLUGIN: &str = "plugin";
    pub const SEND: &str = "send";
    pub const MIXER: &str = "mixer";
    pub const MASTER: &str = "mixer.master";
    pub const CHANNEL: &str = "mixer.channel";
    pub const DEVICE_OUTPUT: &str = "mixer.device_output";
    pub const RACK: &str = "rack";
    pub const RACK_MASTER: &str = "rack.master";
    pub const TUBE: &str = "rack.tube";
    pub const EQ: &str = "rack.eq";
    pub const DYNAMICS: &str = "rack.dynamics";
    pub const TRANSFORMER: &str = "rack.transformer";
    pub const TOLERANCE: &str = "rack.tolerance";
    pub const STEREO: &str = "rack.stereo";
    pub const BUS_COMPRESSOR: &str = "rack.bus_compressor";
    pub const ITEM_CONTROL: &str = "item.control";
    pub const ITEM_CONTROL_POINT: &str = "item.control.point";
    pub const MAPPING: &str = "midi.mapping";
    pub const MIDI_SOURCE: &str = "midi.source";
    pub const MIDI_TARGET: &str = "midi.target";
    pub const MIDI_TRANSFORM: &str = "midi.transform";
    pub const TRACK_MIDI_INPUT: &str = "track.midi_input";
    pub const TRACK_MIDI_OUTPUT: &str = "track.midi_output";
    pub const TRACK_CC_RACK: &str = "track.cc_rack";
    pub const TRACK_CC: &str = "track.cc";
    pub const TRACK_TUNING: &str = "track.tuning";
    pub const TRACK_KEYS: &str = "track.keys";
    pub const RECORD_QUANTIZE: &str = "transport.record_quantize";
    pub const MARKER: &str = "marker";
    pub const REGION: &str = "region";
    pub const MODULATOR: &str = "modulator";
    pub const MODULATOR_TARGET: &str = "modulator.target";
}

/// The kinds of modulator (decisoes-autonomas, 26/11, item 1)
pub const MODULATOR_KINDS: &[&str] = &["lfo", "envelope", "peak", "random", "macro"];
/// What a modulator may move: a track, a channel or a plugin (the nodes that own parameters the engine has)
pub const MODULATABLE_KINDS: &[u8] = &[KIND_TRACK, KIND_CHANNEL, KIND_PLUGIN];

/// The parameters of a modulator that its kind uses (the others are kept and ignored)
pub fn modulator_keys(kind: &str) -> &'static [&'static str] {
    match kind {
        "envelope" => &["attack", "decay", "sustain", "release"],
        "peak" => &["peakAttack", "peakDecay", "threshold", "gain", "abs"],
        "random" => &["rate", "smooth"],
        "macro" => &["value"],
        _ => &["speed", "phase", "wave", "multiplier", "sync", "syncNumerator", "syncDenominator", "depth"],
    }
}

/// The resource kind of a track's tuning files (`.scl` and `.kbm`, read as bytes)
pub const TUNING_KIND: &str = "tuning";

/// The gain stage models of the rack, by their permanent keys (`lm-dsp`, `GainModel::key`)
pub const GAIN_MODELS: &[&str] =
    &["flat", "4558", "tl072", "5532", "5534", "12ax7", "12ax7-vintage", "12ax7-hot", "12at7", "12au7"];
/// The channel compressor's models (`lm-dsp`, `CompressorModel::key`)
pub const COMPRESSOR_MODELS: &[&str] =
    &["ssl", "api2500", "dbx160", "urei1176", "la2a", "fairchild670", "gates-sta-level", "altec436", "rca-ba6a"];
pub const COMPRESSOR_SPEEDS: &[&str] = &["fast", "norm", "slow"];
/// The gate's modes (`lm-dsp`, `GateMode::key`)
pub const GATE_MODES: &[&str] = &["std", "kick", "snare", "toms", "vox", "guitar", "bass", "noise", "expander"];
pub const BUS_RATIOS: &[&str] = &["2", "4", "10"];
pub const ITEM_KINDS: &[&str] = &["midi", "audio"];
pub const TIMEBASES: &[&str] = &["musical", "audio"];
pub const MONITOR_MODES: &[&str] = &["auto", "on", "off"];
/// Where an insert sits in its channel's signal path, against the rack's sections (P-20; `lm-dsp`,
/// `InsertPoint::key`). An insert without `point` is at `"post-eq"`, the end of the rack
pub const INSERT_POINTS: &[&str] = &["pre-gain", "post-gain", "post-filter", "post-dynamics", "post-eq", "pre-pan"];
/// The desk's mode (P-17): in `"digital"` the rack is clean, without the analog modelling
pub const DESK_MODES: &[&str] = &["analog", "digital"];
/// The automation modes of a lane (I-21.5); an automation lane without `mode` is in `"read"`
pub const AUTOMATION_MODES: &[&str] = &["off", "read", "touch", "latch", "write"];
/// The global automation button: `"lanes"` lets each lane use its own mode; absent = `"lanes"`
pub const GLOBAL_AUTOMATION_MODES: &[&str] = &["lanes", "off", "read", "touch", "latch", "write"];
/// The shape of the segment that ends at an automation point; absent = `"curve"` (I-21.5)
pub const POINT_SHAPES: &[&str] = &["curve", "step", "smooth"];
/// The kinds of an item's control lanes (I-21.4)
pub const CONTROL_KINDS: &[&str] = &["cc", "pitchBend", "channelPressure", "polyPressure", "programChange"];
/// How MIDI recording treats what is under it (I-21.4, F-REC-101); absent = `"overdub"`
pub const RECORD_MODES: &[&str] = &["overdub", "replace", "layers"];
/// The message kinds of a mapping's source (lm-midimap, `Message::kind_name`), and `"osc"`: the source is an OSC
/// address (in its `port`), read by the OSC server and skipped by the MIDI map
pub const MIDI_MESSAGES: &[&str] =
    &["cc", "cc14", "nrpn", "rpn", "note", "pitchBend", "channelPressure", "polyPressure", "programChange", "osc"];
pub const MIDI_TARGET_KINDS: &[&str] = &["param", "field", "action"];
/// A selection-following target (lm-midimap, `SelRole`); `"none"` = the fixed `node`
pub const MIDI_FOLLOW: &[&str] = &["none", "track", "channel", "plugin", "trackBank", "channelBank"];
pub const MIDI_MODES: &[&str] = &["absolute", "twos", "signMagnitude", "offset64"];
pub const MIDI_TAKEOVERS: &[&str] = &["jump", "pickup", "scale"];
pub const MIDI_BUTTONS: &[&str] = &["momentary", "toggle", "trigger"];
pub const MIDI_RINGS: &[&str] = &["none", "dot", "boostCut", "wrap", "spread"];
/// What a mapping's target may point at: every node kind with fields or parameters, and the root (transport)
pub const MAPPABLE_KINDS: &[u8] = &[
    KIND_TRACK,
    KIND_OUTPUT,
    KIND_ITEM,
    KIND_LANE,
    KIND_AUTOMATION,
    KIND_PLUGIN,
    KIND_CHANNEL,
    KIND_SEND,
    KIND_MODULATOR,
    b'r',
];

/// The MIDI mapping types (I-21.2), shared with the machine preferences, whose `midiMap` holds the same nodes
pub fn midi_map_schemas() -> Vec<NodeSchema> {
    use types as ty;
    vec![
        NodeSchema::new(ty::MAPPING, 1)
            .field(FieldSpec::id(KIND_MAPPING))
            .field(object("source", ty::MIDI_SOURCE))
            // A motor fader's touch note: on while held, the hand wins over automation (I-21.3)
            .field(object("touch", ty::MIDI_SOURCE).nullable().default(Value::NULL))
            .field(object("target", ty::MIDI_TARGET))
            .field(object("transform", ty::MIDI_TRANSFORM))
            .field(FieldSpec::bool("feedback", true))
            .field(FieldSpec::bool("consume", true))
            // Feedback to another message than the source (a Mackie Control V-pot's LED ring), and its ring style
            .field(FieldSpec::new("feedbackTo", FieldKind::Object(ty::MIDI_SOURCE)).optional())
            .field(FieldSpec::new("ring", FieldKind::Choice(MIDI_RINGS)).optional()),
        // port: an alias of the machine's preferences ("" = any controller port); channel −1 = any
        NodeSchema::new(ty::MIDI_SOURCE, 1)
            .field(FieldSpec::text("port", ""))
            .field(FieldSpec::int("channel", -1, 15, -1))
            .field(FieldSpec::choice("message", MIDI_MESSAGES, "cc"))
            .field(FieldSpec::int("number", 0, 16383, 0)),
        // The target goes with its node (OnRemove::RemoveOwner: the mapping goes with the target and comes back with
        // undo). `node` is null for actions and for selection-following targets
        NodeSchema::new(ty::MIDI_TARGET, 1)
            .field(FieldSpec::choice("kind", MIDI_TARGET_KINDS, "param"))
            .field(reference("node", MAPPABLE_KINDS, OnRemove::RemoveOwner).nullable().default(Value::NULL))
            .field(FieldSpec::choice("follow", MIDI_FOLLOW, "none"))
            .field(FieldSpec::int("offset", 0, 4095, 0))
            .field(FieldSpec::text("param", ""))
            .field(FieldSpec::new("path", FieldKind::List(Box::new(FieldKind::Text))).default(Value::empty_list()))
            .field(FieldSpec::text("action", ""))
            .field(FieldSpec::new("params", FieldKind::Any).default(Value::empty_object()))
            .field(
                FieldSpec::new("valueInto", FieldKind::List(Box::new(FieldKind::Text))).nullable().default(Value::NULL),
            ),
        NodeSchema::new(ty::MIDI_TRANSFORM, 1)
            .field(FieldSpec::real("min", 0.0, 1.0, 0.0))
            .field(FieldSpec::real("max", 0.0, 1.0, 1.0))
            .field(FieldSpec::bool("invert", false))
            .field(FieldSpec::real("curve", -1.0, 1.0, 0.0))
            .field(FieldSpec::choice("mode", MIDI_MODES, "absolute"))
            .field(FieldSpec::real("step", 0.0, 1.0, 1.0 / 128.0))
            .field(FieldSpec::choice("takeover", MIDI_TAKEOVERS, "pickup"))
            .field(FieldSpec::choice("button", MIDI_BUTTONS, "toggle")),
    ]
}

const MAX: i64 = MAX_SAFE_INT;

fn object(key: &'static str, type_name: &'static str) -> FieldSpec {
    FieldSpec::new(key, FieldKind::Object(type_name))
}

fn list_of(key: &'static str, type_name: &'static str) -> FieldSpec {
    FieldSpec::new(key, FieldKind::List(Box::new(FieldKind::Object(type_name))))
}

fn reference(key: &'static str, to: &'static [u8], on_remove: OnRemove) -> FieldSpec {
    FieldSpec::new(key, FieldKind::Ref { to, on_remove })
}

fn ints(key: &'static str, min: i64, max: i64, default: &[i64]) -> FieldSpec {
    FieldSpec::new(key, FieldKind::List(Box::new(FieldKind::Int { min, max })))
        .default(Value::list(default.iter().map(|v| Value::int(*v))))
}

/// Every type of the project, at version 1
pub fn project_schemas() -> Vec<NodeSchema> {
    use types as ty;
    vec![
        NodeSchema::new(PROJECT, 5)
            .field(object("meta", ty::META))
            .field(object("project", ty::SETTINGS))
            .field(object("transport", ty::TRANSPORT))
            .field(list_of("tempoMap", ty::TEMPO_POINT).default(Value::list([Value::object([
                ("tick", Value::int(0)),
                ("bpm", Value::text("120")),
                ("timeSig", Value::list([Value::int(4), Value::int(4)])),
                ("ramp", Value::bool(false)),
            ])])))
            .field(list_of("tracks", ty::TRACK))
            .field(object("mixer", ty::MIXER))
            // The project's MIDI mappings (I-21.2); absent = none
            .field(FieldSpec::new("midiMap", FieldKind::List(Box::new(FieldKind::Object(ty::MAPPING)))).optional())
            // The ruler's named markers and regions (F-REG-011, F-REG-012); absent = none. Version 2 only added them
            .field(list_of("markers", ty::MARKER).optional())
            .field(list_of("regions", ty::REGION).optional())
            // The project's notes (F-PRJ-018), free text up to `NOTES_MAX` bytes; absent = none. Version 5 added them
            .field(FieldSpec::new("notes", FieldKind::Text).optional())
            // Version 4 dropped the project's ten scale and ten keymap slots (tuning is by file, per track)
            .upgrade(1, unchanged)
            .upgrade(2, unchanged)
            .upgrade(3, project_v3)
            .upgrade(4, unchanged),
        // A modulator (decisoes-autonomas, 26/11, item 1): lives with the owner of the parameters it moves. Its `params`
        // are automatable and MIDI-mappable like a plugin's (those `modulator_keys(kind)` names have slots). `input` is
        // what a peak follower measures (a track's or a channel's output); an envelope starts with the notes of its
        // owner's instrument; `userWave` is the LFO's "user" wave (one cycle of an audio file)
        NodeSchema::new(ty::MODULATOR, 1)
            .field(FieldSpec::id(KIND_MODULATOR))
            .field(FieldSpec::text("name", ""))
            .field(FieldSpec::choice("kind", MODULATOR_KINDS, "lfo"))
            .field(reference("input", &[KIND_TRACK, KIND_CHANNEL], OnRemove::Clear).nullable().default(Value::NULL))
            .field(FieldSpec::new("userWave", FieldKind::Resource("audio")).nullable().default(Value::NULL))
            .field(list_of("targets", ty::MODULATOR_TARGET))
            .param(ParamSpec::new("speed", 0.01, 20.0, 2.0))
            .param(ParamSpec::new("phase", 0.0, 360.0, 0.0).unit("deg"))
            .param(ParamSpec::new("wave", 0.0, 7.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("multiplier", 0.0, 2.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("sync", 0.0, 8.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("syncNumerator", 1.0, 32.0, 1.0).scale(Scale::Stepped))
            .param(ParamSpec::new("syncDenominator", 1.0, 32.0, 4.0).scale(Scale::Stepped))
            .param(ParamSpec::new("depth", 0.0, 1.0, 1.0))
            .param(ParamSpec::new("attack", 0.0, 5000.0, 10.0).unit("ms"))
            .param(ParamSpec::new("decay", 0.0, 5000.0, 200.0).unit("ms"))
            .param(ParamSpec::new("sustain", 0.0, 1.0, 0.5))
            .param(ParamSpec::new("release", 0.0, 5000.0, 300.0).unit("ms"))
            .param(ParamSpec::new("peakAttack", 0.0, 0.999, 0.0))
            .param(ParamSpec::new("peakDecay", 0.0, 0.999, 0.0))
            .param(ParamSpec::new("threshold", 0.0, 1.0, 0.0))
            .param(ParamSpec::new("gain", 0.0, 32.0, 1.0))
            .param(ParamSpec::new("abs", 0.0, 1.0, 1.0).scale(Scale::Stepped))
            .param(ParamSpec::new("rate", 0.01, 50.0, 4.0).unit("Hz"))
            .param(ParamSpec::new("smooth", 0.0, 1.0, 0.0))
            .param(ParamSpec::new("value", 0.0, 1.0, 0.0)),
        // One target of a modulator: the parameter `param` of `node`, moved by `amount` of its range (-1 to 1) and kept
        // inside the window `min`..`max` of the range. The target goes with its node
        NodeSchema::new(ty::MODULATOR_TARGET, 1)
            .field(FieldSpec::id(KIND_MOD_TARGET))
            .field(reference("node", MODULATABLE_KINDS, OnRemove::RemoveOwner))
            .field(FieldSpec::new("param", FieldKind::Text))
            .field(FieldSpec::real("amount", -1.0, 1.0, 0.5))
            .field(FieldSpec::real("min", 0.0, 1.0, 0.0))
            .field(FieldSpec::real("max", 0.0, 1.0, 1.0)),
        // A named point of the ruler (F-REG-011): one per tick; color as 0xRRGGBB, absent or 0 = the theme's
        NodeSchema::new(ty::MARKER, 1)
            .field(FieldSpec::id(KIND_MARKER))
            .field(FieldSpec::text("name", ""))
            .field(FieldSpec::int("tick", 0, MAX, 0))
            .field(FieldSpec::int("color", 0, 0xFF_FFFF, 0).optional()),
        // A named range of the ruler (F-REG-012): at least a sixteenth of a 4/4 bar long; regions may overlap
        NodeSchema::new(ty::REGION, 1)
            .field(FieldSpec::id(KIND_REGION))
            .field(FieldSpec::text("name", ""))
            .field(FieldSpec::int("from", 0, MAX, 0))
            .field(FieldSpec::int("to", 0, MAX, 960))
            .field(FieldSpec::int("color", 0, 0xFF_FFFF, 0).optional()),
        // The part of the root that undo never restores (R7)
        NodeSchema::new(ty::META, 1).field(FieldSpec::int("issued", 0, MAX, 0)).field(FieldSpec::text("uuid", "")),
        NodeSchema::new(ty::SETTINGS, 1)
            .field(FieldSpec::int("sampleRate", 8000, 768_000, 48000))
            .field(FieldSpec::int("ppq", 1, 100_000, 960))
            .field(FieldSpec::int("seed", 0, MAX, 0)),
        NodeSchema::new(ty::TRANSPORT, 1)
            .field(object("loop", ty::RANGE))
            .field(object("punch", ty::RANGE))
            .field(object("preRoll", ty::PREROLL))
            .field(object("metronome", ty::METRONOME))
            // The project's transposition in semitones (P-37), for the tracks that follow it; absent: 0
            .field(FieldSpec::new("transpose", FieldKind::Int { min: -12, max: 12 }).optional())
            // The global automation button (I-21.5); absent = "lanes"
            .field(FieldSpec::new("automationMode", FieldKind::Choice(GLOBAL_AUTOMATION_MODES)).optional())
            // MIDI recording (I-21.4): the mode and quantize on record; absent = overdub, no quantize
            .field(FieldSpec::new("recordMode", FieldKind::Choice(RECORD_MODES)).optional())
            .field(FieldSpec::new("recordQuantize", FieldKind::Object(ty::RECORD_QUANTIZE)).optional()),
        // grid in ticks; strength 0..1 (1 = onto the grid); startsOnly keeps the lengths' ends where they were played
        NodeSchema::new(ty::RECORD_QUANTIZE, 1)
            .field(FieldSpec::bool("on", false))
            .field(FieldSpec::int("grid", 1, MAX, 240))
            .field(FieldSpec::real("strength", 0.0, 1.0, 1.0))
            .field(FieldSpec::bool("startsOnly", false)),
        NodeSchema::new(ty::RANGE, 1)
            .field(FieldSpec::bool("on", false))
            .field(FieldSpec::int("from", 0, MAX, 0))
            .field(FieldSpec::int("to", 0, MAX, 0)),
        NodeSchema::new(ty::PREROLL, 1).field(FieldSpec::int("bars", 0, 16, 1)),
        NodeSchema::new(ty::METRONOME, 1)
            .field(FieldSpec::bool("on", false))
            .field(FieldSpec::int("countIn", 0, 8, 0))
            .field(FieldSpec::real("volume", 0.0, 1.0, 0.8)),
        // bpm: an exact decimal in text, 1 to 1000 with up to 6 decimals (lm-time, I-18)
        NodeSchema::new(ty::TEMPO_POINT, 1)
            .field(FieldSpec::int("tick", 0, MAX, 0))
            .field(FieldSpec::text("bpm", "120"))
            .field(ints("timeSig", 1, 64, &[4, 4]))
            .field(FieldSpec::bool("ramp", false)),
        // The timeline: items, automation, the instrument and the outputs. No pan (decision of 19/11)
        // Version 6: `noteEffects`, the note effect plugins before the instrument (optional: absent = none)
        NodeSchema::new(ty::TRACK, 6)
            .field(FieldSpec::id(KIND_TRACK))
            .field(FieldSpec::text("name", ""))
            // 0xRRGGBB; absent: the palette's colour for the track's place (F-TRK-001)
            .field(FieldSpec::int("color", 0, 0xFF_FFFF, 0).optional())
            .field(FieldSpec::bool("folder", false))
            .field(FieldSpec::bool("collapsed", false))
            .field(reference("parent", &[KIND_TRACK], OnRemove::Clear).nullable().default(Value::NULL))
            .field(FieldSpec::real("volume", 0.0, 4.0, 1.0))
            .field(FieldSpec::bool("mute", false))
            .field(FieldSpec::bool("solo", false))
            .field(FieldSpec::int("channels", 1, 64, 2))
            // Whether the instrument follows the project's transposition (P-37); absent: true
            .field(FieldSpec::new("transpose", FieldKind::Bool).optional())
            .field(object("instrument", ty::PLUGIN).nullable().default(Value::NULL))
            // The note effects, in the order the notes go through them (items and live input first, the instrument
            // and the MIDI out last); an effect is a plugin node of a plugin of kind NoteEffect; absent = none
            .field(FieldSpec::new("noteEffects", FieldKind::List(Box::new(FieldKind::Object(ty::PLUGIN)))).optional())
            .field(object("input", ty::TRACK_INPUT).nullable().default(Value::NULL))
            // The live MIDI input (I-21.1; F-REC-103, F-REC-131); absent = every instrument port, every channel
            .field(FieldSpec::new("midiInput", FieldKind::Object(ty::TRACK_MIDI_INPUT)).optional())
            // The track's MIDI output to an external device (F-IJN-017, P-38); absent or null = none
            .field(FieldSpec::new("midiOutput", FieldKind::Object(ty::TRACK_MIDI_OUTPUT)).optional().nullable())
            // The CC rack (F-IJN-025): its switch, and the values of the controllers set ("cc.N" parameters,
            // 0..127, sent to the instrument as control changes); absent = off, none set
            .field(FieldSpec::new("ccRack", FieldKind::Object(ty::TRACK_CC_RACK)).optional())
            .field(FieldSpec::new("cc", FieldKind::Object(ty::TRACK_CC)).optional())
            // The velocity that plays at 100 % (F-IJN-017); absent: 63
            .field(FieldSpec::new("baseVelocity", FieldKind::Int { min: 1, max: 127 }).optional())
            // The tuning by file (F-IJN-019); absent = off (12-TET). Version 4 replaced the old `microtuner`
            .field(FieldSpec::new("tuning", FieldKind::Object(ty::TRACK_TUNING)).optional().nullable())
            // The keys the instrument plays and its base key (F-IJN-021); absent = every key, base A4 (69)
            .field(FieldSpec::new("keys", FieldKind::Object(ty::TRACK_KEYS)).optional().nullable())
            .field(FieldSpec::bool("armed", false))
            .field(FieldSpec::choice("monitor", MONITOR_MODES, "auto"))
            .field(list_of("outputs", ty::OUTPUT))
            .field(list_of("items", ty::ITEM))
            .field(list_of("automation", ty::AUTOMATION))
            // Version 4: the modulators of the track's parameters (absent = none)
            .field(list_of("modulators", ty::MODULATOR).optional())
            .upgrade(1, unchanged)
            .upgrade(2, unchanged)
            .upgrade(3, unchanged)
            .upgrade(4, track_v4)
            .upgrade(5, unchanged),
        // ports: aliases (empty = every port whose role is instrument); channel −1 = all; followSelection: the
        // keyboard plays the selected track
        NodeSchema::new(ty::TRACK_MIDI_INPUT, 2)
            .field(FieldSpec::new("ports", FieldKind::List(Box::new(FieldKind::Text))).default(Value::empty_list()))
            .field(FieldSpec::int("channel", -1, 15, -1))
            .field(FieldSpec::bool("followSelection", false))
            // Every note at this velocity (F-IJN-017); absent = as played
            .field(FieldSpec::new("fixedVelocity", FieldKind::Int { min: 1, max: 127 }).optional())
            .upgrade(1, unchanged),
        // P-38: on (the legacy's checkbox keeps the settings while off); ports: aliases of the machine's preferences
        // (I-21.1); channel 0..15 (shown +1); program and the fixed ones: absent = as played (program: none sent)
        NodeSchema::new(ty::TRACK_MIDI_OUTPUT, 2)
            .field(FieldSpec::bool("on", true))
            .field(FieldSpec::new("ports", FieldKind::List(Box::new(FieldKind::Text))).default(Value::empty_list()))
            .field(FieldSpec::int("channel", 0, 15, 0))
            .field(FieldSpec::new("program", FieldKind::Int { min: 0, max: 127 }).optional())
            .field(FieldSpec::new("fixedVelocity", FieldKind::Int { min: 1, max: 127 }).optional())
            .field(FieldSpec::new("fixedNote", FieldKind::Int { min: 0, max: 127 }).optional())
            // v1 had one `port` ("" = none) and no switch: the same output, on
            .upgrade(1, midi_output_v1),
        NodeSchema::new(ty::TRACK_CC_RACK, 1).field(FieldSpec::bool("on", false)),
        cc_schema(),
        // Tuning by file: `scale` is a Scala `.scl` and `keymap` an optional `.kbm`, resources of kind "tuning"
        // copied into the project; importRanges: the keymap's first and last key replace the track's keys
        NodeSchema::new(ty::TRACK_TUNING, 1)
            .field(FieldSpec::bool("on", false))
            .field(FieldSpec::new("scale", FieldKind::Resource(TUNING_KIND)).optional().nullable())
            .field(FieldSpec::new("keymap", FieldKind::Resource(TUNING_KIND)).optional().nullable())
            .field(FieldSpec::bool("importRanges", true)),
        NodeSchema::new(ty::TRACK_KEYS, 1)
            .field(FieldSpec::int("first", 0, 127, 0))
            .field(FieldSpec::int("last", 0, 127, 127))
            .field(FieldSpec::int("base", 0, 127, 69)),
        NodeSchema::new(ty::TRACK_INPUT, 1).field(FieldSpec::text("device", "in")).field(ints(
            "channels",
            1,
            256,
            &[1],
        )),
        // An output: a port of the instrument (or a split of an audio track's channels) to a desk channel. Removing
        // the channel sends it to the master (I-3.9)
        NodeSchema::new(ty::OUTPUT, 1)
            .field(FieldSpec::id(KIND_OUTPUT))
            .field(FieldSpec::text("port", "main"))
            .field(FieldSpec::int("channels", 1, 64, 2))
            .field(reference("to", &[KIND_CHANNEL], OnRemove::Redirect(MASTER)).default(Value::text("m:0"))),
        // One type for both kinds; the fields of the other kind stay empty (a domain rule)
        NodeSchema::new(ty::ITEM, 4)
            .field(FieldSpec::id(KIND_ITEM))
            .field(FieldSpec::choice("kind", ITEM_KINDS, "midi"))
            .field(FieldSpec::text("name", ""))
            // 0xRRGGBB; absent: the track's (F-CLP-001)
            .field(FieldSpec::int("color", 0, 0xFF_FFFF, 0).optional())
            .field(FieldSpec::int("start", 0, MAX, 0))
            .field(FieldSpec::int("length", 1, MAX, 3840))
            .field(FieldSpec::bool("mute", false))
            .field(FieldSpec::choice("timebase", TIMEBASES, "musical"))
            .field(FieldSpec::new("notes", FieldKind::NoteMap(ty::NOTE)))
            .field(list_of("lanes", ty::LANE))
            .field(list_of("comp", ty::COMP))
            .field(FieldSpec::int("fadeIn", 0, MAX, 0))
            .field(FieldSpec::int("fadeOut", 0, MAX, 0))
            .field(FieldSpec::real("gain", 0.0, 4.0, 1.0))
            // The fades' tension, -1..1 [0, linear]: above 0 the fade starts slow (like an exponential), below it
            // starts fast; the same shape serves a crossfade (a comp segment's `curve`) (F-CLP-041, F-CLP-066)
            .field(FieldSpec::real("fadeInCurve", -1.0, 1.0, 0.0).optional())
            .field(FieldSpec::real("fadeOutCurve", -1.0, 1.0, 0.0).optional())
            // MIDI items (absent = the meaning in brackets, so trees without them keep their meaning, U29):
            // the repeating part of a pattern [no repeat], the swing [0] and the step editor's resolution [240]
            .field(FieldSpec::int("loop", 1, MAX, 3840).optional())
            .field(FieldSpec::real("swing", 0.0, 1.0, 0.0).optional())
            .field(FieldSpec::int("step", 1, MAX, 240).optional())
            // The control lanes of a MIDI item (I-21.4): controllers, pitch bend, aftertouch and program changes
            // recorded or drawn, played back with the notes [none]
            .field(
                FieldSpec::new("controls", FieldKind::List(Box::new(FieldKind::Object(ty::ITEM_CONTROL)))).optional(),
            )
            .upgrade(1, unchanged)
            .upgrade(2, unchanged)
            .upgrade(3, unchanged),
        // kind and number (the controller, the key of a poly aftertouch; 0 otherwise), the channel; points: ticks from
        // the item's start, values 0..127 (pitch bend 0..16383, centre 8192)
        NodeSchema::new(ty::ITEM_CONTROL, 1)
            .field(FieldSpec::choice("kind", CONTROL_KINDS, "cc"))
            .field(FieldSpec::int("number", 0, 127, 0))
            .field(FieldSpec::int("channel", 0, 15, 0))
            .field(list_of("points", ty::ITEM_CONTROL_POINT)),
        NodeSchema::new(ty::ITEM_CONTROL_POINT, 1)
            .field(FieldSpec::int("tick", 0, MAX, 0))
            .field(FieldSpec::int("value", 0, 16383, 0)),
        NodeSchema::new(ty::LANE, 1)
            .field(FieldSpec::id(KIND_LANE))
            .field(FieldSpec::new("source", FieldKind::Resource("audio")))
            .field(FieldSpec::int("sourceOffset", 0, MAX, 0))
            .field(FieldSpec::text("name", ""))
            // The tempo the take was played at (F-REC-046, F-REC-090) [none: the source plays as it is]: the source
            // frame (from `sourceOffset`) at each tick from the item's start; the lane of a musical item follows the
            // tempo map by a stretch rendered from the original
            .field(list_of("timing", ty::LANE_TIMING_POINT).optional())
            // The silence a capture xrun left in the take (F-REC-084) [none]: in source frames, like `sourceOffset`
            .field(list_of("gaps", ty::LANE_GAP).optional()),
        NodeSchema::new(ty::LANE_GAP, 1)
            .field(FieldSpec::int("frame", 0, MAX, 0))
            .field(FieldSpec::int("frames", 1, MAX, 1)),
        NodeSchema::new(ty::LANE_TIMING_POINT, 1)
            .field(FieldSpec::int("tick", -MAX, MAX, 0))
            .field(FieldSpec::real("frame", -1.0e15, 1.0e15, 0.0)),
        // A comp segment: from/to in the item's ticks; its lane by id (U2). A removed lane leaves silence
        NodeSchema::new(ty::COMP, 2)
            .field(FieldSpec::int("from", 0, MAX, 0))
            .field(FieldSpec::int("to", 0, MAX, 0))
            .field(reference("lane", &[KIND_LANE], OnRemove::Clear).nullable().default(Value::NULL))
            .field(FieldSpec::int("fade", 0, MAX, 240))
            // The tension of the crossfade into this segment, -1..1 [0] (F-CLP-041)
            .field(FieldSpec::real("curve", -1.0, 1.0, 0.0).optional())
            .upgrade(1, unchanged),
        // A note, in an item's note map by its number (C1); start is from the item's start
        NodeSchema::new(ty::NOTE, 3)
            .field(FieldSpec::int("key", 0, 127, 60))
            .field(FieldSpec::int("start", 0, MAX, 0))
            .field(FieldSpec::int("length", 1, MAX, 240))
            .field(FieldSpec::real("velocity", 0.0, 1.0, 0.8))
            // Per-note pan (F-NOT-010) [0] and pitch-bend curve (P-24) [none]
            .field(FieldSpec::real("pan", -1.0, 1.0, 0.0).optional())
            .field(list_of("bend", ty::NOTE_BEND).optional())
            // Version 3: the per-note pressure and timbre (CC74) curves of MPE, 0..1 [none]
            .field(list_of("pressure", ty::NOTE_PRESSURE).optional())
            .field(list_of("timbre", ty::NOTE_TIMBRE).optional())
            .upgrade(1, unchanged)
            .upgrade(2, unchanged),
        // A point of a note's bend: ticks from the note's start, and semitones; linear between points
        NodeSchema::new(ty::NOTE_BEND, 1).field(FieldSpec::int("at", 0, MAX, 0)).field(FieldSpec::real(
            "semitones",
            -24.0,
            24.0,
            0.0,
        )),
        // A point of a note's pressure or timbre: ticks from the note's start, and 0..1; linear between points
        NodeSchema::new(ty::NOTE_PRESSURE, 1)
            .field(FieldSpec::int("at", 0, MAX, 0))
            .field(FieldSpec::real("value", 0.0, 1.0, 0.0)),
        NodeSchema::new(ty::NOTE_TIMBRE, 1)
            .field(FieldSpec::int("at", 0, MAX, 0))
            .field(FieldSpec::real("value", 0.0, 1.0, 0.5)),
        // Automation lives with the owner of the parameter (base spec, item 2)
        NodeSchema::new(ty::AUTOMATION, 1)
            .field(FieldSpec::id(KIND_AUTOMATION))
            .field(object("target", ty::AUTOMATION_TARGET))
            .field(FieldSpec::bool("on", true))
            // I-21.5; absent = "read"
            .field(FieldSpec::new("mode", FieldKind::Choice(AUTOMATION_MODES)).optional())
            .field(list_of("points", ty::AUTOMATION_POINT)),
        NodeSchema::new(ty::AUTOMATION_TARGET, 1)
            // A modulator's own parameters can be automated too
            .field(reference("node", &[KIND_TRACK, KIND_CHANNEL, KIND_PLUGIN, KIND_MODULATOR], OnRemove::RemoveOwner))
            .field(FieldSpec::new("param", FieldKind::Text)),
        NodeSchema::new(ty::AUTOMATION_POINT, 1)
            .field(FieldSpec::int("tick", 0, MAX, 0))
            .field(FieldSpec::new("value", FieldKind::Real { min: -1.0e12, max: 1.0e12 }))
            .field(FieldSpec::real("curve", -1.0, 1.0, 0.0).optional())
            // The segment that ends here: "curve" (bent by `curve`), "step" (holds, then jumps here) or "smooth" (a
            // cardinal spline through the neighbours, with `tension`, absent = 0.5); absent = "curve" (I-21.5)
            .field(FieldSpec::new("shape", FieldKind::Choice(POINT_SHAPES)).optional())
            .field(FieldSpec::new("tension", FieldKind::Real { min: 0.0, max: 1.0 }).optional())
            // The value leaving this point when it differs from the arriving one (a jump, P-27)
            .field(FieldSpec::new("out", FieldKind::Real { min: -1.0e12, max: 1.0e12 }).optional()),
        // An instrument or an insert. `state` is the plugin's own tree, validated by the schema "plugin:<plugin>" at
        // `version`. Version 2: the optional `wet` (a node of version 1 has none: all wet)
        NodeSchema::new(ty::PLUGIN, 3)
            .field(FieldSpec::id(KIND_PLUGIN))
            .field(FieldSpec::new("plugin", FieldKind::Text))
            .field(FieldSpec::int("version", 1, MAX, 1))
            .field(FieldSpec::new("state", FieldKind::Any).default(Value::empty_object()))
            .field(FieldSpec::bool("bypass", false))
            // An insert's wet/dry mix, 0 to 1 (the legacy EffectView's W/D; optional: absent is all wet, no mix node)
            .field(FieldSpec::new("wet", FieldKind::Real { min: 0.0, max: 1.0 }).optional())
            // Only for an insert of a channel (P-20); absent: "post-eq"
            .field(FieldSpec::new("point", FieldKind::Choice(INSERT_POINTS)).optional())
            // Version 3: the modulators of the plugin's parameters (absent = none)
            .field(list_of("modulators", ty::MODULATOR).optional())
            .upgrade(1, unchanged)
            .upgrade(2, unchanged),
        // A send to another channel, or to an insert's sidechain port (`port`, then required)
        NodeSchema::new(ty::SEND, 1)
            .field(FieldSpec::id(KIND_SEND))
            .field(reference("to", &[KIND_CHANNEL, KIND_PLUGIN], OnRemove::RemoveOwner))
            .field(FieldSpec::new("port", FieldKind::Text).optional())
            .field(FieldSpec::real("amount", 0.0, 4.0, 1.0))
            .field(FieldSpec::bool("pre", false)),
        NodeSchema::new(ty::MIXER, 1)
            // Absent: "analog"
            .field(FieldSpec::new("mode", FieldKind::Choice(DESK_MODES)).optional())
            .field(object("master", ty::MASTER))
            .field(list_of("channels", ty::CHANNEL)),
        NodeSchema::new(ty::MASTER, 2)
            .field(FieldSpec::id(KIND_CHANNEL))
            .field(FieldSpec::text("name", "Master"))
            .field(FieldSpec::int("channels", 1, 64, 2))
            .field(FieldSpec::real("volume", 0.0, 4.0, 1.0))
            .field(FieldSpec::bool("mute", false))
            // The legacy master's PRE faders, one per side, into its gain stage (linear gain; absent is 1), and LINK,
            // which moves them together (absent is on)
            .field(FieldSpec::new("preLeft", FieldKind::Real { min: 0.0, max: 4.0 }).optional())
            .field(FieldSpec::new("preRight", FieldKind::Real { min: 0.0, max: 4.0 }).optional())
            .field(FieldSpec::new("preLink", FieldKind::Bool).optional())
            .field(FieldSpec::new("color", FieldKind::Text).optional().nullable())
            .field(object("rack", ty::RACK_MASTER))
            .field(list_of("inserts", ty::PLUGIN))
            .field(list_of("automation", ty::AUTOMATION))
            .field(object("output", ty::DEVICE_OUTPUT))
            // Version 2: the modulators of the master's parameters (absent = none)
            .field(list_of("modulators", ty::MODULATOR).optional())
            .upgrade(1, unchanged),
        NodeSchema::new(ty::DEVICE_OUTPUT, 1).field(FieldSpec::text("device", "out")).field(ints(
            "channels",
            1,
            256,
            &[1, 2],
        )),
        NodeSchema::new(ty::CHANNEL, 2)
            .field(FieldSpec::id(KIND_CHANNEL))
            .field(FieldSpec::text("name", ""))
            .field(FieldSpec::int("channels", 1, 64, 2))
            .field(FieldSpec::real("volume", 0.0, 4.0, 1.0))
            .field(FieldSpec::real("pan", -1.0, 1.0, 0.0))
            .field(FieldSpec::bool("mute", false))
            .field(FieldSpec::bool("solo", false))
            // The legacy strip's input controls (optional: absent is off, 0 dB, no color). `trim` is clean gain in dB
            // before the rack (-60 is off); `mono` sums to mono before the processing so that the pan places it;
            // `phaseInvert` inverts both sides; `swap` trades left and right; `color` is "#rrggbb" (null: none)
            .field(FieldSpec::new("trim", FieldKind::Real { min: -60.0, max: 24.0 }).optional())
            .field(FieldSpec::new("mono", FieldKind::Bool).optional())
            .field(FieldSpec::new("phaseInvert", FieldKind::Bool).optional())
            .field(FieldSpec::new("swap", FieldKind::Bool).optional())
            // `width` is the stereo width in percent (mid/side; 0 is mono, 100 is untouched, 200 doubles the side);
            // absent is 100
            .field(FieldSpec::new("width", FieldKind::Real { min: 0.0, max: 200.0 }).optional())
            // `return` marks an effect return (P-19): a channel made to receive sends, shown as one before anything
            // sends to it; absent is false (a channel others send to is a return anyway)
            .field(FieldSpec::new("return", FieldKind::Bool).optional())
            .field(FieldSpec::new("color", FieldKind::Text).optional().nullable())
            .field(object("rack", ty::RACK))
            .field(list_of("inserts", ty::PLUGIN))
            .field(list_of("sends", ty::SEND))
            .field(list_of("automation", ty::AUTOMATION))
            .field(reference("output", &[KIND_CHANNEL], OnRemove::Redirect(MASTER)).default(Value::text("m:0")))
            // Version 2: the modulators of the channel's parameters (absent = none)
            .field(list_of("modulators", ty::MODULATOR).optional())
            .upgrade(1, unchanged),
        // The rack of a channel: fixed sections, no ids; parameters addressed as "rack.tube.drive" (M5)
        NodeSchema::new(ty::RACK, 1)
            .field(object("tube", ty::TUBE))
            .field(object("eq", ty::EQ))
            .field(object("dynamics", ty::DYNAMICS))
            .field(object("transformer", ty::TRANSFORMER))
            .field(object("tolerance", ty::TOLERANCE)),
        NodeSchema::new(ty::RACK_MASTER, 1)
            .field(object("tube", ty::TUBE))
            .field(object("stereo", ty::STEREO))
            .field(object("dynamics", ty::BUS_COMPRESSOR))
            .field(object("transformer", ty::TRANSFORMER)),
        // The gain stage: the GAIN pot (`drive`, unity at 0.25) into an op-amp or a tube
        NodeSchema::new(ty::TUBE, 1)
            .field(FieldSpec::bool("on", true))
            .field(FieldSpec::choice("model", GAIN_MODELS, "4558"))
            .field(FieldSpec::real("drive", 0.0, 1.0, 0.25))
            .field(FieldSpec::bool("realistic", false))
            .field(FieldSpec::bool("lowCut", false)),
        // Filters, mono bass and the 4-band desk EQ (dB)
        NodeSchema::new(ty::EQ, 1)
            .field(FieldSpec::bool("on", true))
            .field(FieldSpec::real("highPass", 20.0, 20000.0, 20.0))
            .field(FieldSpec::real("lowPass", 20.0, 20000.0, 20000.0))
            .field(FieldSpec::real("monoBass", 0.0, 100.0, 0.0))
            .field(FieldSpec::real("monoBassFrequency", 20.0, 500.0, 120.0))
            .field(FieldSpec::real("low", -15.0, 15.0, 0.0))
            .field(FieldSpec::real("lowMid", -15.0, 15.0, 0.0))
            .field(FieldSpec::real("highMid", -15.0, 15.0, 0.0))
            .field(FieldSpec::real("high", -15.0, 15.0, 0.0)),
        // The channel's gate and compressor (SSL E series and the other models)
        NodeSchema::new(ty::DYNAMICS, 1)
            .field(FieldSpec::bool("on", true))
            .field(FieldSpec::choice("compModel", COMPRESSOR_MODELS, "ssl"))
            .field(FieldSpec::real("compAmount", 0.0, 1.0, 0.0))
            .field(FieldSpec::real("compRatio", 0.0, 1.0, 0.0))
            .field(FieldSpec::choice("compSpeed", COMPRESSOR_SPEEDS, "norm"))
            .field(FieldSpec::real("compRelease", 0.0, 1.0, 0.5))
            .field(FieldSpec::real("gateThreshold", -60.0, 20.0, -10.0))
            .field(FieldSpec::real("gateRange", 0.0, 80.0, 0.0))
            .field(FieldSpec::choice("gateMode", GATE_MODES, "std")),
        NodeSchema::new(ty::TRANSFORMER, 1).field(FieldSpec::bool("on", false)),
        // The channel's component tolerances, drawn from its number (lm-dsp, ChannelTolerance)
        NodeSchema::new(ty::TOLERANCE, 1).field(FieldSpec::bool("on", true)),
        NodeSchema::new(ty::STEREO, 1)
            .field(FieldSpec::real("width", 0.0, 200.0, 100.0))
            .field(FieldSpec::real("monoBass", 0.0, 100.0, 0.0))
            .field(FieldSpec::real("bassFrequency", 20.0, 500.0, 120.0))
            .field(FieldSpec::real("air", -12.0, 12.0, 0.0)),
        // The SSL G bus compressor: attack and release are indexes of the panel's steps (release 4 is AUTO)
        NodeSchema::new(ty::BUS_COMPRESSOR, 1)
            .field(FieldSpec::bool("on", false))
            .field(FieldSpec::real("threshold", -15.0, 15.0, 5.0))
            .field(FieldSpec::choice("ratio", BUS_RATIOS, "4"))
            .field(FieldSpec::int("attack", 0, 5, 4))
            .field(FieldSpec::int("release", 0, 4, 4))
            .field(FieldSpec::real("makeup", -5.0, 15.0, 0.0)),
    ]
}

/// Adds every project type to a registry (the plugins add theirs, `"plugin:<id>"`, before or after)
pub fn register_project_schemas(registry: &mut SchemaRegistry) -> Result<()> {
    for schema in project_schemas().into_iter().chain(midi_map_schemas()) {
        registry.add(schema)?;
    }
    Ok(())
}

/// A registry with only the project's types (the plugins' are added by the plugin layer)
pub fn project_registry() -> Result<SchemaRegistry> {
    let mut registry = SchemaRegistry::new();
    register_project_schemas(&mut registry)?;
    registry.check_complete()?;
    Ok(registry)
}

/// The schema type of a plugin's state
pub fn plugin_type(plugin: &str) -> String {
    format!("plugin:{plugin}")
}

/// Version 2 of `item`, `note` and `project`, 3 of `track` and `item` (their `color`), 4 of `item` (the fade curves) and
/// 2 of `item.comp` (its `curve`) only added optional fields whose absence keeps the earlier meaning
fn unchanged(node: Value) -> Result<Value> {
    Ok(node)
}

/// `project` v3 → v4: the ten scale and keymap slots are gone (tuning is a file of the track; no user project had
/// slots yet)
fn project_v3(node: Value) -> Result<Value> {
    Ok(node.without("tuning"))
}

/// `track` v4 → v5: the `microtuner` that cited slots is dropped (tuning is a file now)
fn track_v4(node: Value) -> Result<Value> {
    Ok(node.without("microtuner"))
}

/// `track.midi_output` v1 → v2: `port` becomes `ports`, and the output is on
fn midi_output_v1(node: Value) -> Result<Value> {
    let port = node.get("port").and_then(Value::as_text).unwrap_or("").to_owned();
    let ports = if port.is_empty() { Value::empty_list() } else { Value::list([Value::text(port)]) };
    Ok(node.without("port").with("ports", ports).with("on", Value::bool(true)))
}

/// The field keys of the CC rack's values (`c0`..`c127`): a track's `cc` holds the controllers set; their parameter
/// keys are `cc.0`..`cc.127`
pub const CC_KEYS: [&str; 128] = [
    "c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8", "c9", "c10", "c11", "c12", "c13", "c14", "c15", "c16", "c17",
    "c18", "c19", "c20", "c21", "c22", "c23", "c24", "c25", "c26", "c27", "c28", "c29", "c30", "c31", "c32", "c33",
    "c34", "c35", "c36", "c37", "c38", "c39", "c40", "c41", "c42", "c43", "c44", "c45", "c46", "c47", "c48", "c49",
    "c50", "c51", "c52", "c53", "c54", "c55", "c56", "c57", "c58", "c59", "c60", "c61", "c62", "c63", "c64", "c65",
    "c66", "c67", "c68", "c69", "c70", "c71", "c72", "c73", "c74", "c75", "c76", "c77", "c78", "c79", "c80", "c81",
    "c82", "c83", "c84", "c85", "c86", "c87", "c88", "c89", "c90", "c91", "c92", "c93", "c94", "c95", "c96", "c97",
    "c98", "c99", "c100", "c101", "c102", "c103", "c104", "c105", "c106", "c107", "c108", "c109", "c110", "c111",
    "c112", "c113", "c114", "c115", "c116", "c117", "c118", "c119", "c120", "c121", "c122", "c123", "c124", "c125",
    "c126", "c127",
];

/// `track.cc`: every controller optional (absent = never sent), 0..127
fn cc_schema() -> NodeSchema {
    let mut s = NodeSchema::new(types::TRACK_CC, 1);
    for k in CC_KEYS {
        s = s.field(FieldSpec::new(k, FieldKind::Real { min: 0.0, max: 127.0 }).optional());
    }
    s
}
