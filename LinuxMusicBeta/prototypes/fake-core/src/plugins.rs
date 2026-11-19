//! Owner: agent C. Fake data for instrument and effect descriptors with their parameter schemas (the generic editor
//! builds from them), and the plugin nodes in the document: each instrument track's instrument and its instrument
//! functions (envelopes, filter, chords, arpeggio, MIDI, tuning), and each desk channel's inserts.
//!
//! The shape follows the real contract (`arquitetura-nova.md` piece 7/7a, `interfaces.md` I-3.3, I-9, U30, C5): a
//! descriptor declares its parameters by permanent text key, with range, scale (linear, log, stepped), unit and
//! whether it is automatable; named choices are by text key, never by number; the state is the node's tree (here a
//! struct in `Doc`), so undo, presets and copying are the same operation. Building a plugin is an operation with the
//! loading rule (I-2): `build` starts it on the node's target.
//!
//! Ids: instruments are `p:`, inserts are `x:` (the desk's own numbering, `mixer.rs`), outputs `o:`, all from one
//! counter that never repeats in a session.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::arrange::{Output, TrackKind};
use crate::{Doc, Edit, Error, NodeId, Result, Session, Target};

// ---------------------------------------------------------------------------------------------------------------
// The schema

/// How a parameter's range is shown and swept (lm-state's `Scale`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Linear,
    /// frequencies and times: the knob's travel is logarithmic
    Log,
    /// whole numbers only (octaves, semitones, MIDI channels): a stepper, never a knob that lands between steps
    Stepped,
}

/// What kind of control a parameter gets
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Real { min: f64, max: f64, def: f64, scale: Scale },
    Bool { def: bool },
    /// options as (permanent key, label key) (U30)
    Choice { options: &'static [(&'static str, &'static str)], def: &'static str },
}

#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    /// permanent (C5)
    pub key: &'static str,
    /// label key, translated by the GUI
    pub label: &'static str,
    /// the group (a card in the generic editor), label key
    pub group: &'static str,
    pub kind: Kind,
    /// "%", "dB", "Hz", "ms", "st", "ct", "°", "x", ":1", or empty
    pub unit: &'static str,
    pub automatable: bool,
}

impl ParamSpec {
    pub fn default_value(&self) -> f64 {
        match self.kind {
            Kind::Real { def, .. } => def,
            Kind::Bool { def } => f64::from(u8::from(def)),
            Kind::Choice { .. } => 0.0,
        }
    }
    /// The value clamped to the range, and whole when stepped
    pub fn clamp(&self, v: f64) -> f64 {
        match self.kind {
            Kind::Real { min, max, scale, .. } => {
                let v = v.clamp(min, max);
                if scale == Scale::Stepped { v.round() } else { v }
            }
            Kind::Bool { .. } => f64::from(u8::from(v > 0.5)),
            Kind::Choice { .. } => v,
        }
    }
    /// A value as 0..1 of the control's travel (log for log parameters)
    pub fn to_travel(&self, v: f64) -> f64 {
        match self.kind {
            Kind::Real { min, max, scale: Scale::Log, .. } => ((v / min).ln() / (max / min).ln()).clamp(0.0, 1.0),
            Kind::Real { min, max, .. } => ((v - min) / (max - min)).clamp(0.0, 1.0),
            Kind::Bool { .. } => v,
            Kind::Choice { .. } => 0.0,
        }
    }
    pub fn from_travel(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let v = match self.kind {
            Kind::Real { min, max, scale: Scale::Log, .. } => min * (max / min).powf(t),
            Kind::Real { min, max, .. } => min + t * (max - min),
            Kind::Bool { .. } => t,
            Kind::Choice { .. } => 0.0,
        };
        self.clamp(v)
    }
    /// The value as shown, with its unit
    pub fn format(&self, v: f64) -> String {
        match self.unit {
            "Hz" if v >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
            "Hz" if v < 10.0 => format!("{v:.2} Hz"),
            "Hz" => format!("{v:.0} Hz"),
            "ms" if v >= 1000.0 => format!("{:.2} s", v / 1000.0),
            "ms" if v < 10.0 => format!("{v:.1} ms"),
            "dB" => format!("{v:+.1} dB"),
            ":1" => format!("{v:.1}:1"),
            "x" => format!("{v:.2}x"),
            "st" | "ct" if matches!(self.kind, Kind::Real { min, .. } if min < 0.0) && v > 0.0 => {
                format!("+{v:.0} {}", self.unit)
            }
            "" if matches!(self.kind, Kind::Real { scale: Scale::Stepped, .. }) => format!("{v:.0}"),
            "" => format!("{v:.2}"),
            u if matches!(self.kind, Kind::Real { scale: Scale::Stepped, .. }) => format!("{v:.0} {u}"),
            u => format!("{v:.0} {u}"),
        }
    }
    pub fn choice_label(&self, key: &str) -> &'static str {
        match self.kind {
            Kind::Choice { options, .. } => options.iter().find(|(k, _)| *k == key).map_or("?", |(_, l)| l),
            _ => "?",
        }
    }
}

const fn real(key: &'static str, label: &'static str, group: &'static str, min: f64, max: f64, def: f64, unit: &'static str) -> ParamSpec {
    ParamSpec { key, label, group, kind: Kind::Real { min, max, def, scale: Scale::Linear }, unit, automatable: true }
}
const fn log(key: &'static str, label: &'static str, group: &'static str, min: f64, max: f64, def: f64, unit: &'static str) -> ParamSpec {
    ParamSpec { key, label, group, kind: Kind::Real { min, max, def, scale: Scale::Log }, unit, automatable: true }
}
const fn stepped(key: &'static str, label: &'static str, group: &'static str, min: f64, max: f64, def: f64, unit: &'static str) -> ParamSpec {
    ParamSpec { key, label, group, kind: Kind::Real { min, max, def, scale: Scale::Stepped }, unit, automatable: true }
}
const fn switch(key: &'static str, label: &'static str, group: &'static str, def: bool) -> ParamSpec {
    ParamSpec { key, label, group, kind: Kind::Bool { def }, unit: "", automatable: true }
}
const fn choice(key: &'static str, label: &'static str, group: &'static str, options: &'static [(&'static str, &'static str)], def: &'static str) -> ParamSpec {
    // choices are not interpolated (they change at the exact point) but are automatable
    ParamSpec { key, label, group, kind: Kind::Choice { options, def }, unit: "", automatable: true }
}
const fn fixed(mut p: ParamSpec) -> ParamSpec {
    p.automatable = false;
    p
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Instrument,
    Effect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    /// in the render path, in the process (I-9.6: internal plugins stay in)
    Internal,
    /// a third-party LV2 plugin: always out of the process (I-9.6)
    Lv2,
}

#[derive(Clone, Debug)]
pub struct Preset {
    pub name: &'static str,
    pub values: Vec<(&'static str, f64)>,
    pub choices: Vec<(&'static str, &'static str)>,
}

#[derive(Debug)]
pub struct Descriptor {
    /// permanent id; also the label key of its name
    pub id: &'static str,
    pub role: Role,
    /// label key (F-FX-036 categories)
    pub category: &'static str,
    /// a short tag drawn on the plugin's chip (F-FX-036: colour and initials per category)
    pub short: &'static str,
    pub host: Host,
    /// plays each note in a voice of the host (envelopes, filter and microtuner apply); false: a single stream that
    /// manages its own voices (Sf2, AVL, third-party)
    pub per_note: bool,
    /// what it loads, as a label key ("Sample", "SoundFont"); None: nothing
    pub resource: Option<&'static str>,
    /// latency in samples at 48 kHz (reported to the PDC)
    pub latency: u32,
    /// how long building takes in the fake (I-2: past 200 ms the loading shows)
    pub build_ms: u64,
    /// has a sidechain input port (the source is a send, `arquitetura-nova.md` item 3)
    pub sidechain: bool,
    pub params: Vec<ParamSpec>,
    pub presets: Vec<Preset>,
    /// label key
    pub description: &'static str,
}

impl Descriptor {
    pub fn param(&self, key: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.key == key)
    }
    /// The groups in schema order
    pub fn groups(&self) -> Vec<&'static str> {
        let mut g: Vec<&'static str> = Vec::new();
        for p in &self.params {
            if !g.contains(&p.group) {
                g.push(p.group);
            }
        }
        g
    }
}

// --- choices (U30: permanent keys, label keys) ------------------------------------------------------------------

pub const WAVES: &[(&str, &str)] = &[
    ("sine", "Sine"), ("triangle", "Triangle"), ("saw", "Saw"), ("square", "Square"), ("moog_saw", "Moog saw"),
    ("exponential", "Exponential"), ("white_noise", "White noise"), ("user", "User wave"),
];
pub const OSC_MOD: &[(&str, &str)] =
    &[("pm", "Phase modulation"), ("am", "Amplitude modulation"), ("mix", "Mix"), ("sync", "Sync"), ("fm", "Frequency modulation")];
pub const KITS: &[(&str, &str)] = &[
    ("black_pearl", "Black Pearl"), ("red_zeppelin", "Red Zeppelin"), ("blonde_bop", "Blonde Bop"),
    ("blonde_bop_hr", "Blonde Bop Hot Rod"), ("buskmans", "Buskman's Holiday"),
];
pub const LOOP_MODES: &[(&str, &str)] = &[("off", "No loop"), ("loop", "Loop"), ("ping_pong", "Ping-pong")];
pub const INTERP: &[(&str, &str)] = &[("none", "None"), ("linear", "Linear"), ("sinc", "Sinc")];
pub const LB_WAVES: &[(&str, &str)] = &[("saw", "Saw"), ("square", "Square"), ("triangle", "Triangle"), ("sine", "Sine")];
pub const LFO_SHAPES: &[(&str, &str)] =
    &[("sine", "Sine"), ("triangle", "Triangle"), ("saw", "Saw"), ("square", "Square"), ("user", "User wave"), ("random", "Random")];
pub const FILTERS: &[(&str, &str)] = &[
    ("lowpass", "Low-pass"), ("highpass", "High-pass"), ("bandpass_csg", "Band-pass (constant skirt)"),
    ("bandpass_czpg", "Band-pass (constant peak)"), ("notch", "Notch"), ("allpass", "All-pass"), ("moog", "Moog"),
    ("lowpass_2x", "2x Low-pass"), ("rc_lowpass_12", "RC low-pass 12 dB"), ("rc_bandpass_12", "RC band-pass 12 dB"),
    ("rc_highpass_12", "RC high-pass 12 dB"), ("rc_lowpass_24", "RC low-pass 24 dB"), ("rc_bandpass_24", "RC band-pass 24 dB"),
    ("rc_highpass_24", "RC high-pass 24 dB"), ("formant", "Vocal formant"), ("moog_2x", "2x Moog"),
    ("sv_lowpass", "SV low-pass"), ("sv_bandpass", "SV band-pass"), ("sv_highpass", "SV high-pass"), ("sv_notch", "SV notch"),
    ("fast_formant", "Fast formant"), ("tripole", "Tripole"),
];
/// The chord and scale table (F-IJN-016); the fake has the common part of the legacy's 95 entries
pub const CHORDS: &[(&str, &str)] = &[
    ("octave", "Octave"), ("major", "Major"), ("majb5", "Major flat 5"), ("minor", "Minor"), ("minb5", "Minor flat 5"),
    ("sus2", "Sus2"), ("sus4", "Sus4"), ("aug", "Augmented"), ("augsus4", "Augmented sus4"), ("tri", "Triad"),
    ("6", "Sixth"), ("6sus4", "Sixth sus4"), ("6add9", "Sixth add 9"), ("m6", "Minor sixth"), ("m6add9", "Minor sixth add 9"),
    ("7", "Seventh"), ("7sus4", "Seventh sus4"), ("7#5", "Seventh sharp 5"), ("7b5", "Seventh flat 5"), ("7#9", "Seventh sharp 9"),
    ("7b9", "Seventh flat 9"), ("7#11", "Seventh sharp 11"), ("maj7", "Major seventh"), ("maj7b5", "Major seventh flat 5"),
    ("maj7#5", "Major seventh sharp 5"), ("m7", "Minor seventh"), ("m7b5", "Half-diminished"), ("mmaj7", "Minor major seventh"),
    ("dim7", "Diminished seventh"), ("9", "Ninth"), ("9sus4", "Ninth sus4"), ("add9", "Add 9"), ("maj9", "Major ninth"),
    ("m9", "Minor ninth"), ("madd9", "Minor add 9"), ("11", "Eleventh"), ("m11", "Minor eleventh"), ("maj11", "Major eleventh"),
    ("13", "Thirteenth"), ("m13", "Minor thirteenth"), ("maj13", "Major thirteenth"), ("dim", "Diminished"), ("power", "Power chord"),
    ("major_scale", "Major scale"), ("minor_scale", "Natural minor scale"), ("harmonic_minor", "Harmonic minor"),
    ("melodic_minor", "Melodic minor"), ("whole_tone", "Whole tone"), ("diminished_scale", "Diminished scale"),
    ("major_pentatonic", "Major pentatonic"), ("minor_pentatonic", "Minor pentatonic"), ("jap_in_sen", "Jap in sen"),
    ("major_bebop", "Major bebop"), ("dominant_bebop", "Dominant bebop"), ("blues", "Blues scale"), ("arabic", "Arabic"),
    ("enigmatic", "Enigmatic"), ("neopolitan", "Neapolitan"), ("neopolitan_minor", "Neapolitan minor"),
    ("hungarian_minor", "Hungarian minor"), ("dorian", "Dorian"), ("phrygian", "Phrygian"), ("lydian", "Lydian"),
    ("mixolydian", "Mixolydian"), ("aeolian", "Aeolian"), ("locrian", "Locrian"), ("chromatic", "Chromatic"),
    ("half_whole", "Half-whole diminished"), ("persian", "Persian"),
];
pub const ARP_DIRECTIONS: &[(&str, &str)] =
    &[("up", "Up"), ("down", "Down"), ("up_down", "Up and down"), ("down_up", "Down and up"), ("random", "Random")];
pub const ARP_MODES: &[(&str, &str)] = &[("free", "Free"), ("sort", "Sort"), ("sync", "Sync")];
/// The project's scales and keymaps: 10 slots each (F-IJN-019/020); the microtuner picks a slot
pub const TUNING_SLOTS: usize = 10;

/// A scale (Scala `.scl`): a description and the intervals, one per line, in cents ("701.955") or as a ratio ("3/2")
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleDef {
    pub description: String,
    pub intervals: String,
}

/// A keymap (Scala `.kbm`): the scale degree of each key ("x" = silent), the range and the reference
#[derive(Clone, Debug, PartialEq)]
pub struct KeymapDef {
    pub description: String,
    pub degrees: String,
    pub first: u8,
    pub last: u8,
    pub middle: u8,
    pub base_key: u8,
    pub base_freq: f64,
}

/// Parses the intervals of a scale: every line a number with a point (cents) or a ratio `a/b` or a whole number
/// (a ratio over 1). Returns the cents, or the first bad line (the editor refuses to apply it)
pub fn parse_intervals(text: &str) -> std::result::Result<Vec<f64>, usize> {
    let mut v = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let l = line.split_whitespace().next().unwrap_or("");
        if l.is_empty() || l.starts_with('!') {
            continue;
        }
        let c = if l.contains('.') {
            l.parse::<f64>().map_err(|_| i + 1)?
        } else if let Some((a, b)) = l.split_once('/') {
            let (a, b) = (a.parse::<f64>().map_err(|_| i + 1)?, b.parse::<f64>().map_err(|_| i + 1)?);
            if a <= 0.0 || b <= 0.0 {
                return Err(i + 1);
            }
            1200.0 * (a / b).log2()
        } else {
            let a = l.parse::<f64>().map_err(|_| i + 1)?;
            if a <= 0.0 {
                return Err(i + 1);
            }
            1200.0 * a.log2()
        };
        v.push(c);
    }
    if v.is_empty() { Err(1) } else { Ok(v) }
}

fn default_scales() -> Vec<ScaleDef> {
    let mut v = vec![
        ScaleDef { description: "12-TET".into(), intervals: (1..=12).map(|k| format!("{}.0", k * 100)).collect::<Vec<_>>().join("\n") },
        ScaleDef { description: "Just intonation (5-limit)".into(), intervals: "16/15\n9/8\n6/5\n5/4\n4/3\n45/32\n3/2\n8/5\n5/3\n9/5\n15/8\n2/1".into() },
        ScaleDef { description: "19-TET".into(), intervals: (1..=19).map(|k| format!("{:.3}", k as f64 * 1200.0 / 19.0)).collect::<Vec<_>>().join("\n") },
    ];
    while v.len() < TUNING_SLOTS {
        v.push(ScaleDef { description: String::new(), intervals: "1200.0".into() });
    }
    v
}

fn default_keymaps() -> Vec<KeymapDef> {
    let mut v = vec![KeymapDef { description: "Every key".into(), degrees: "0 1 2 3 4 5 6 7 8 9 10 11".into(), first: 0, last: 127, middle: 60, base_key: 69, base_freq: 440.0 }];
    v.push(KeymapDef { description: "White keys only".into(), degrees: "0 x 1 x 2 3 x 4 x 5 x 6".into(), first: 0, last: 127, middle: 60, base_key: 69, base_freq: 440.0 });
    while v.len() < TUNING_SLOTS {
        v.push(KeymapDef { description: String::new(), degrees: "0".into(), first: 0, last: 127, middle: 60, base_key: 69, base_freq: 440.0 });
    }
    v
}
pub const SATURATION: &[(&str, &str)] = &[("tape", "Tape"), ("tube", "Tube"), ("transistor", "Transistor"), ("fold", "Wave folder")];
pub const EQ_SHAPES: &[(&str, &str)] = &[("bell", "Bell"), ("low_shelf", "Low shelf"), ("high_shelf", "High shelf"), ("notch", "Notch")];
pub const SYNC_NOTES: &[(&str, &str)] =
    &[("free", "Free (ms)"), ("1_1", "1/1"), ("1_2", "1/2"), ("1_4", "1/4"), ("1_8", "1/8"), ("1_16", "1/16"), ("1_8d", "1/8 dotted"), ("1_4t", "1/4 triplet")];

/// Where an insert sits relative to the analog rack's sections (legacy `AnalogChannelStrip::InsertPoint`; P-20: each
/// insert has its own point)
pub const INSERT_POINTS: &[(&str, &str)] = &[
    ("pre_tube", "Before the tube"), ("post_tube", "After the tube"), ("post_eq", "After the EQ"),
    ("post_dynamics", "After the dynamics"), ("post_rack", "At the end of the rack"), ("post_fader", "After the fader"),
];
pub const DEFAULT_INSERT_POINT: &str = "post_tube";

/// The MIDI devices the fake knows (the real list comes from the device scan, I-10.2)
pub const MIDI_DEVICES: &[&str] = &["Teclado USB (MIDI 1)", "Controlador de pads", "Porta virtual"];

/// The id of the instrument functions' schema: every instrument track has them (envelopes, filter, chords,
/// arpeggio, MIDI, tuning, voices), whichever instrument it plays
pub const FUNCTIONS: &str = "instrument-functions";
/// The common controls of every effect (F-FX-001): wet/dry
pub const COMMON_FX: &[ParamSpec] = &[real("wet", "Wet/Dry", "Common", -100.0, 100.0, 100.0, "%")];

// --- the descriptors --------------------------------------------------------------------------------------------

fn osc(n: u8) -> Vec<ParamSpec> {
    let k = |s: &str| -> &'static str { Box::leak(format!("osc{n}.{s}").into_boxed_str()) };
    let g: &'static str = ["Oscillator 1", "Oscillator 2", "Oscillator 3"][n as usize - 1];
    vec![
        choice(k("wave"), "Wave", g, WAVES, "saw"),
        real(k("vol"), "Volume", g, 0.0, 200.0, 33.0, "%"),
        real(k("pan"), "Pan", g, -100.0, 100.0, 0.0, "%"),
        stepped(k("coarse"), "Coarse", g, -24.0, 24.0, [0.0, -12.0, -24.0][n as usize - 1], "st"),
        real(k("fine_l"), "Fine left", g, -100.0, 100.0, 0.0, "ct"),
        real(k("fine_r"), "Fine right", g, -100.0, 100.0, 0.0, "ct"),
        real(k("phase"), "Phase", g, 0.0, 360.0, 0.0, "°"),
        real(k("stereo_phase"), "Stereo phase", g, 0.0, 360.0, 0.0, "°"),
        switch(k("antialias"), "Anti-aliasing", g, true),
    ]
}

fn env_target(t: &'static str, group: &'static str, amount_def: f64) -> Vec<ParamSpec> {
    let k = |s: &str| -> &'static str { Box::leak(format!("env.{t}.{s}").into_boxed_str()) };
    let l = |s: &str| -> &'static str { Box::leak(format!("lfo.{t}.{s}").into_boxed_str()) };
    vec![
        log(k("predelay"), "Pre-delay", group, 0.1, 2000.0, 0.1, "ms"),
        log(k("attack"), "Attack", group, 0.1, 5000.0, 0.1, "ms"),
        log(k("hold"), "Hold", group, 0.1, 5000.0, 500.0, "ms"),
        log(k("decay"), "Decay", group, 0.1, 5000.0, 500.0, "ms"),
        real(k("sustain"), "Sustain", group, 0.0, 100.0, 50.0, "%"),
        log(k("release"), "Release", group, 0.1, 5000.0, 100.0, "ms"),
        real(k("amount"), "Amount", group, -100.0, 100.0, amount_def, "%"),
        log(l("predelay"), "LFO pre-delay", group, 0.1, 2000.0, 0.1, "ms"),
        log(l("attack"), "LFO attack", group, 0.1, 2000.0, 0.1, "ms"),
        log(l("speed"), "LFO speed", group, 0.01, 20.0, 2.0, "Hz"),
        real(l("amount"), "LFO amount", group, -100.0, 100.0, 0.0, "%"),
        choice(l("shape"), "LFO shape", group, LFO_SHAPES, "sine"),
        switch(l("x100"), "Speed x 100", group, false),
        switch(l("mod_env"), "LFO controls the envelope amount", group, false),
    ]
}

/// The instrument functions' schema (one per instrument track, kept when the instrument is swapped)
fn functions() -> Descriptor {
    let mut p = Vec::new();
    p.extend(env_target("vol", "Volume", 0.0));
    p.extend(env_target("cut", "Cutoff", 0.0));
    p.extend(env_target("res", "Resonance", 0.0));
    p.extend([
        switch("filter.on", "Filter", "Filter", false),
        choice("filter.type", "Type", "Filter", FILTERS, "lowpass"),
        log("filter.cutoff", "Cutoff", "Filter", 1.0, 14000.0, 14000.0, "Hz"),
        real("filter.res", "Resonance", "Filter", 0.01, 10.0, 0.5, ""),
        switch("chord.on", "Chord stacking", "Chords", false),
        choice("chord.type", "Chord", "Chords", CHORDS, "major"),
        stepped("chord.range", "Range", "Chords", 1.0, 9.0, 1.0, "oct"),
        switch("arp.on", "Arpeggio", "Arpeggio", false),
        choice("arp.type", "Chord", "Arpeggio", CHORDS, "major"),
        stepped("arp.range", "Range", "Arpeggio", 1.0, 9.0, 1.0, "oct"),
        stepped("arp.repeats", "Repeats", "Arpeggio", 1.0, 8.0, 1.0, ""),
        stepped("arp.cycle", "Cycle", "Arpeggio", 0.0, 6.0, 0.0, ""),
        real("arp.skip", "Skip", "Arpeggio", 0.0, 100.0, 0.0, "%"),
        real("arp.miss", "Miss", "Arpeggio", 0.0, 100.0, 0.0, "%"),
        log("arp.time", "Time", "Arpeggio", 25.0, 2000.0, 200.0, "ms"),
        choice("arp.sync", "Follow the tempo", "Arpeggio", SYNC_NOTES, "free"),
        real("arp.gate", "Gate", "Arpeggio", 1.0, 200.0, 100.0, "%"),
        choice("arp.direction", "Direction", "Arpeggio", ARP_DIRECTIONS, "up"),
        choice("arp.mode", "Mode", "Arpeggio", ARP_MODES, "free"),
        fixed(switch("voice.mono", "One note at a time", "Voices", false)),
        fixed(switch("voice.legato", "Legato", "Voices", false)),
        log("voice.glide", "Glide", "Voices", 1.0, 2000.0, 1.0, "ms"),
        fixed(stepped("voice.max", "Voices", "Voices", 1.0, 64.0, 32.0, "")),
        fixed(switch("midi.in.on", "Receive MIDI", "MIDI input", true)),
        fixed(stepped("midi.in.channel", "Channel (0 = all)", "MIDI input", 0.0, 16.0, 0.0, "")),
        fixed(stepped("midi.in.velocity", "Fixed velocity (0 = as played)", "MIDI input", 0.0, 127.0, 0.0, "")),
        fixed(switch("midi.out.on", "Send MIDI", "MIDI output", false)),
        fixed(stepped("midi.out.channel", "Channel", "MIDI output", 1.0, 16.0, 1.0, "")),
        fixed(stepped("midi.out.velocity", "Fixed velocity (0 = as played)", "MIDI output", 0.0, 127.0, 0.0, "")),
        fixed(stepped("midi.out.program", "Program", "MIDI output", 1.0, 128.0, 1.0, "")),
        fixed(stepped("midi.out.note", "Fixed note (0 = as played)", "MIDI output", 0.0, 127.0, 0.0, "")),
        fixed(stepped("midi.base_velocity", "Base velocity", "Velocity", 1.0, 127.0, 63.0, "")),
        real("tune.pitch", "Pitch", "Transposition", -100.0, 100.0, 0.0, "ct"),
        fixed(stepped("tune.range", "Pitch bend range", "Transposition", 1.0, 60.0, 2.0, "st")),
        fixed(switch("tune.global", "Follow the global transposition", "Transposition", true)),
        fixed(switch("micro.on", "Microtuner", "Microtuner", false)),
        fixed(stepped("micro.scale", "Scale slot", "Microtuner", 1.0, 10.0, 1.0, "")),
        fixed(stepped("micro.keymap", "Keymap slot", "Microtuner", 1.0, 10.0, 1.0, "")),
        fixed(switch("micro.import", "Note range from the keymap", "Microtuner", false)),
        fixed(stepped("keys.first", "First key", "Keys", 0.0, 127.0, 24.0, "")),
        fixed(stepped("keys.last", "Last key", "Keys", 0.0, 127.0, 108.0, "")),
        fixed(stepped("keys.base", "Base key", "Keys", 0.0, 127.0, 69.0, "")),
    ]);
    Descriptor {
        id: FUNCTIONS, role: Role::Instrument, category: "", short: "", host: Host::Internal, per_note: true,
        resource: None, latency: 0, build_ms: 0, sidechain: false, params: p, presets: Vec::new(), description: "",
    }
}

fn instruments() -> Vec<Descriptor> {
    let mut v = Vec::new();
    let mut triple = Vec::new();
    triple.extend(osc(1));
    triple.extend(osc(2));
    triple.extend(osc(3));
    triple.push(choice("mod12", "Oscillator 1 by 2", "Modulation", OSC_MOD, "mix"));
    triple.push(choice("mod23", "Oscillator 2 by 3", "Modulation", OSC_MOD, "mix"));
    v.push(Descriptor {
        id: "TripleOscillator", role: Role::Instrument, category: "Synthesizers", short: "3O", host: Host::Internal,
        per_note: true, resource: None, latency: 0, build_ms: 40, sidechain: false, params: triple,
        presets: vec![
            Preset { name: "Fat Bass", values: vec![("osc1.vol", 60.0), ("osc2.vol", 45.0), ("osc2.fine_l", -7.0), ("osc2.fine_r", 7.0)], choices: vec![("osc1.wave", "saw"), ("osc2.wave", "square")] },
            Preset { name: "Soft Pad", values: vec![("osc1.vol", 30.0), ("osc2.vol", 30.0), ("osc3.vol", 20.0)], choices: vec![("osc1.wave", "sine"), ("osc2.wave", "triangle"), ("mod12", "am")] },
            Preset { name: "Sync Lead", values: vec![("osc1.vol", 70.0), ("osc2.coarse", 7.0)], choices: vec![("mod12", "sync"), ("osc1.wave", "saw")] },
            Preset { name: "FM Bell", values: vec![("osc1.vol", 50.0), ("osc2.coarse", 12.0), ("osc2.vol", 80.0)], choices: vec![("mod12", "fm"), ("osc1.wave", "sine"), ("osc2.wave", "sine")] },
        ],
        description: "Three stereo oscillators, the first modulated by the second and the second by the third.",
    });
    v.push(Descriptor {
        id: "AVL Drums", role: Role::Instrument, category: "Drums", short: "AV", host: Host::Internal, per_note: false,
        resource: Some("Kit"), latency: 0, build_ms: 1400, sidechain: false,
        params: vec![
            fixed(choice("kit", "Kit", "Kit", KITS, "black_pearl")),
            fixed(switch("per_piece", "One output per piece", "Kit", false)),
            stepped("velocity_floor", "Lowest velocity", "Kit", 1.0, 127.0, 1.0, ""),
        ],
        presets: vec![
            Preset { name: "Rock (Red Zeppelin)", values: vec![], choices: vec![("kit", "red_zeppelin")] },
            Preset { name: "Jazz (Blonde Bop)", values: vec![], choices: vec![("kit", "blonde_bop")] },
            Preset { name: "Percussion (Buskman's)", values: vec![], choices: vec![("kit", "buskmans")] },
        ],
        description: "The AVL acoustic drum kits, with an output per piece of the kit.",
    });
    v.push(Descriptor {
        id: "AudioFileProcessor", role: Role::Instrument, category: "Samplers", short: "AF", host: Host::Internal,
        per_note: true, resource: Some("Sample"), latency: 0, build_ms: 600, sidechain: false,
        params: vec![
            real("amp", "Amplify", "Sample", 0.0, 500.0, 100.0, "%"),
            real("start", "Start", "Sample", 0.0, 100.0, 0.0, "%"),
            real("end", "End", "Sample", 0.0, 100.0, 100.0, "%"),
            real("loop_start", "Loop start", "Loop", 0.0, 100.0, 0.0, "%"),
            choice("loop", "Loop", "Loop", LOOP_MODES, "off"),
            switch("reverse", "Reverse", "Playback", false),
            switch("stutter", "Continue where the last note stopped", "Playback", false),
            choice("interp", "Interpolation", "Playback", INTERP, "sinc"),
        ],
        presets: vec![Preset { name: "One-shot", values: vec![], choices: vec![("loop", "off")] }, Preset { name: "Looped pad", values: vec![("loop_start", 20.0)], choices: vec![("loop", "ping_pong")] }],
        description: "Plays a sample at the pitch of each note, with start, end and loop points.",
    });
    v.push(Descriptor {
        id: "Sf2 Player", role: Role::Instrument, category: "Samplers", short: "SF", host: Host::Internal, per_note: false,
        resource: Some("SoundFont"), latency: 0, build_ms: 2600, sidechain: false,
        params: vec![
            fixed(stepped("bank", "Bank", "Patch", 0.0, 999.0, 0.0, "")),
            fixed(stepped("patch", "Patch", "Patch", 0.0, 127.0, 0.0, "")),
            real("gain", "Gain", "Patch", 0.0, 5.0, 1.0, "x"),
            switch("reverb.on", "Reverb", "Reverb", false),
            real("reverb.room", "Room size", "Reverb", 0.0, 1.0, 0.2, ""),
            real("reverb.damp", "Damping", "Reverb", 0.0, 1.0, 0.0, ""),
            real("reverb.width", "Width", "Reverb", 0.0, 100.0, 0.5, ""),
            real("reverb.level", "Level", "Reverb", 0.0, 1.0, 0.9, ""),
            switch("chorus.on", "Chorus", "Chorus", false),
            stepped("chorus.voices", "Voices", "Chorus", 0.0, 10.0, 3.0, ""),
            real("chorus.level", "Level", "Chorus", 0.0, 10.0, 2.0, ""),
            log("chorus.speed", "Speed", "Chorus", 0.29, 5.0, 0.3, "Hz"),
            real("chorus.depth", "Depth", "Chorus", 0.0, 46.0, 8.0, "ms"),
        ],
        presets: vec![Preset { name: "Grand Piano", values: vec![("patch", 0.0)], choices: vec![] }, Preset { name: "Strings", values: vec![("patch", 48.0)], choices: vec![] }],
        description: "Plays a SoundFont: one patch from a bank.",
    });
    v.push(Descriptor {
        id: "Kicker", role: Role::Instrument, category: "Drums", short: "KI", host: Host::Internal, per_note: true,
        resource: None, latency: 0, build_ms: 20, sidechain: false,
        params: vec![
            log("start_freq", "Start frequency", "Frequency", 5.0, 1000.0, 150.0, "Hz"),
            log("end_freq", "End frequency", "Frequency", 5.0, 1000.0, 40.0, "Hz"),
            real("freq_slope", "Frequency slope", "Frequency", 0.001, 1.0, 0.06, ""),
            log("decay", "Length", "Envelope", 5.0, 5000.0, 440.0, "ms"),
            real("env_slope", "Envelope slope", "Envelope", 0.01, 1.0, 0.163, ""),
            log("attack", "Attack", "Envelope", 0.1, 20.0, 1.0, "ms"),
            real("dist_start", "Start distortion", "Distortion", 0.0, 100.0, 0.8, ""),
            real("dist_end", "End distortion", "Distortion", 0.0, 100.0, 0.8, ""),
            real("gain", "Gain", "Output", 0.1, 5.0, 1.0, "x"),
            real("noise", "Noise", "Output", 0.0, 1.0, 0.0, ""),
            real("click", "Click", "Output", 0.0, 1.0, 0.4, ""),
            fixed(switch("start_from_note", "Start at the note's pitch", "Frequency", false)),
        ],
        presets: vec![Preset { name: "808 Boom", values: vec![("decay", 1200.0), ("end_freq", 45.0)], choices: vec![] }],
        description: "A bass drum synthesizer: a sine sweeping down, with distortion, noise and click.",
    });
    v.push(Descriptor {
        id: "Lb302", role: Role::Instrument, category: "Synthesizers", short: "LB", host: Host::Internal, per_note: false,
        resource: None, latency: 0, build_ms: 20, sidechain: false,
        params: vec![
            log("cutoff", "Cutoff", "Filter", 20.0, 8000.0, 800.0, "Hz"),
            real("reso", "Resonance", "Filter", 0.0, 1.25, 0.75, ""),
            real("env_mod", "Envelope mod", "Filter", 0.0, 1.0, 0.1, ""),
            real("decay", "Decay", "Filter", 0.0, 1.0, 0.1, ""),
            real("dist", "Distortion", "Output", 0.0, 1.0, 0.0, ""),
            choice("wave", "Wave", "Output", LB_WAVES, "saw"),
            switch("slide", "Slide", "Notes", false),
            switch("accent", "Accent", "Notes", false),
            switch("dead", "Dead notes", "Notes", false),
            switch("db24", "24 dB filter", "Filter", false),
        ],
        presets: vec![Preset { name: "Acid Line", values: vec![("reso", 1.1), ("env_mod", 0.7)], choices: vec![] }],
        description: "A monophonic TB-303 imitation.",
    });
    v.push(Descriptor {
        id: "Dexed (LV2)", role: Role::Instrument, category: "LV2", short: "LV", host: Host::Lv2, per_note: false,
        resource: None, latency: 0, build_ms: 900, sidechain: false,
        params: (1..=6)
            .flat_map(|n| {
                let k = |s: &str| -> &'static str { Box::leak(format!("op{n}.{s}").into_boxed_str()) };
                let g: &'static str = ["Operator 1", "Operator 2", "Operator 3", "Operator 4", "Operator 5", "Operator 6"][n - 1];
                [real(k("level"), "Level", g, 0.0, 99.0, 80.0, ""), real(k("ratio"), "Ratio", g, 0.5, 31.0, 1.0, "x"), real(k("detune"), "Detune", g, -7.0, 7.0, 0.0, "")]
            })
            .chain([stepped("algorithm", "Algorithm", "Global", 1.0, 32.0, 1.0, ""), real("feedback", "Feedback", "Global", 0.0, 7.0, 0.0, "")])
            .collect(),
        presets: vec![Preset { name: "E.Piano 1", values: vec![("algorithm", 5.0)], choices: vec![] }],
        description: "A third-party LV2 instrument: it runs outside the program, and its controls come from its ports.",
    });
    v
}

fn effects() -> Vec<Descriptor> {
    let mut v = Vec::new();
    let mut fx = |id, category, short, latency, sidechain, params: Vec<ParamSpec>, presets: Vec<Preset>, description| {
        v.push(Descriptor { id, role: Role::Effect, category, short, host: Host::Internal, per_note: false, resource: None, latency, build_ms: 30, sidechain, params, presets, description });
    };
    fx("Compressor", "Dynamics", "CO", 64, true, vec![
        real("threshold", "Threshold", "Compression", -60.0, 0.0, -12.0, "dB"),
        log("ratio", "Ratio", "Compression", 1.0, 20.0, 4.0, ":1"),
        log("attack", "Attack", "Compression", 0.1, 100.0, 5.0, "ms"),
        log("release", "Release", "Compression", 5.0, 2000.0, 120.0, "ms"),
        real("knee", "Knee", "Compression", 0.0, 24.0, 6.0, "dB"),
        real("makeup", "Make-up gain", "Output", -12.0, 24.0, 0.0, "dB"),
        switch("auto_makeup", "Automatic make-up", "Output", false),
        switch("lookahead", "Look-ahead", "Compression", true),
        switch("sidechain.on", "Sidechain", "Sidechain", false),
        log("sidechain.hpf", "Sidechain high-pass", "Sidechain", 20.0, 2000.0, 20.0, "Hz"),
    ], vec![Preset { name: "Vocal leveler", values: vec![("threshold", -18.0), ("ratio", 3.0)], choices: vec![] }, Preset { name: "Drum smash", values: vec![("threshold", -30.0), ("ratio", 10.0), ("attack", 0.5)], choices: vec![] }],
        "Compresses the dynamics; with look-ahead it reports its latency.");
    let mut eq = Vec::new();
    for (n, f) in [(1, 80.0), (2, 250.0), (3, 1000.0), (4, 2500.0), (5, 5000.0), (6, 10000.0)] {
        let k = |s: &str| -> &'static str { Box::leak(format!("band{n}.{s}").into_boxed_str()) };
        let g: &'static str = ["Band 1", "Band 2", "Band 3", "Band 4", "Band 5", "Band 6"][n - 1];
        eq.extend([switch(k("on"), "On", g, n != 6), choice(k("shape"), "Shape", g, EQ_SHAPES, if n == 1 { "low_shelf" } else if n == 6 { "high_shelf" } else { "bell" }),
            log(k("freq"), "Frequency", g, 20.0, 20000.0, f, "Hz"), real(k("gain"), "Gain", g, -18.0, 18.0, 0.0, "dB"), log(k("q"), "Q", g, 0.1, 10.0, 0.7, "")]);
    }
    eq.extend([switch("hp.on", "High-pass", "Filters", false), log("hp.freq", "High-pass frequency", "Filters", 10.0, 1000.0, 30.0, "Hz"),
        switch("lp.on", "Low-pass", "Filters", false), log("lp.freq", "Low-pass frequency", "Filters", 1000.0, 20000.0, 18000.0, "Hz"),
        real("output", "Output", "Filters", -18.0, 18.0, 0.0, "dB")]);
    fx("EQ 8", "EQ and filters", "EQ", 0, false, eq, vec![Preset { name: "Vocal presence", values: vec![("band4.gain", 3.0), ("hp.on", 1.0)], choices: vec![] }], "Eight bands: six shapeable bands, a high-pass and a low-pass.");
    fx("De-esser", "Dynamics", "DS", 32, false, vec![log("freq", "Frequency", "De-esser", 2000.0, 12000.0, 6500.0, "Hz"), real("threshold", "Threshold", "De-esser", -60.0, 0.0, -24.0, "dB"), real("range", "Range", "De-esser", 0.0, 24.0, 8.0, "dB"), switch("listen", "Listen to the band", "De-esser", false)], vec![], "Tames the sibilance of a voice.");
    fx("Gate", "Dynamics", "GA", 0, true, vec![real("threshold", "Threshold", "Gate", -80.0, 0.0, -40.0, "dB"), real("range", "Range", "Gate", -80.0, 0.0, -60.0, "dB"), log("attack", "Attack", "Gate", 0.1, 50.0, 1.0, "ms"), log("hold", "Hold", "Gate", 1.0, 500.0, 20.0, "ms"), log("release", "Release", "Gate", 5.0, 2000.0, 100.0, "ms"), switch("sidechain.on", "Sidechain", "Sidechain", false)], vec![], "Closes on silence between the hits.");
    fx("Saturator", "Distortion", "SA", 0, false, vec![choice("mode", "Mode", "Saturation", SATURATION, "tape"), real("drive", "Drive", "Saturation", 0.0, 36.0, 6.0, "dB"), real("tone", "Tone", "Saturation", -100.0, 100.0, 0.0, "%"), real("output", "Output", "Saturation", -24.0, 6.0, 0.0, "dB")], vec![], "Warms the sound with tape, tube or transistor saturation.");
    fx("Limiter", "Dynamics", "LI", 240, false, vec![real("ceiling", "Ceiling", "Limiter", -12.0, 0.0, -0.3, "dB"), real("gain", "Input gain", "Limiter", 0.0, 24.0, 0.0, "dB"), log("release", "Release", "Limiter", 1.0, 1000.0, 60.0, "ms"), switch("true_peak", "True peak", "Limiter", true)], vec![], "Keeps the peaks under the ceiling; reports its look-ahead latency.");
    fx("Reverb", "Delay and reverb", "RV", 0, false, vec![real("size", "Size", "Room", 0.0, 1.0, 0.7, ""), log("cutoff", "Cutoff", "Room", 200.0, 20000.0, 10000.0, "Hz"), log("predelay", "Pre-delay", "Room", 0.1, 200.0, 12.0, "ms"), real("input", "Input gain", "Levels", -24.0, 12.0, 0.0, "dB"), real("output", "Output gain", "Levels", -24.0, 12.0, 0.0, "dB")], vec![Preset { name: "Big hall", values: vec![("size", 0.92)], choices: vec![] }], "A feedback-delay-network reverb (ReverbSC).");
    fx("Delay", "Delay and reverb", "DL", 0, false, vec![log("time", "Time", "Delay", 1.0, 5000.0, 375.0, "ms"), choice("sync", "Follow the tempo", "Delay", SYNC_NOTES, "1_8d"), real("feedback", "Feedback", "Delay", 0.0, 100.0, 35.0, "%"), log("lfo_rate", "LFO rate", "Modulation", 0.01, 10.0, 0.3, "Hz"), real("lfo_amount", "LFO amount", "Modulation", 0.0, 20.0, 0.0, "ms"), real("output", "Output", "Delay", -24.0, 12.0, 0.0, "dB")], vec![], "An echo with feedback and LFO, synced to the tempo when asked.");
    fx("Plate", "Delay and reverb", "PL", 0, false, vec![real("decay", "Decay", "Plate", 0.1, 10.0, 2.2, ""), real("damping", "Damping", "Plate", 0.0, 1.0, 0.4, ""), log("predelay", "Pre-delay", "Plate", 0.1, 200.0, 20.0, "ms")], vec![], "A plate reverb.");
    fx("Parallel comp", "Dynamics", "PC", 64, false, vec![real("threshold", "Threshold", "Compression", -60.0, 0.0, -30.0, "dB"), log("ratio", "Ratio", "Compression", 1.0, 20.0, 8.0, ":1"), log("attack", "Attack", "Compression", 0.1, 100.0, 1.0, "ms"), log("release", "Release", "Compression", 5.0, 2000.0, 80.0, "ms")], vec![], "A hard compressor meant for a parallel return.");
    fx("Bitcrush", "Distortion", "BC", 0, false, vec![log("rate", "Sample rate", "Crush", 20.0, 48000.0, 8000.0, "Hz"), stepped("bits", "Bits", "Crush", 1.0, 24.0, 8.0, ""), real("in_gain", "Input gain", "Levels", -24.0, 24.0, 0.0, "dB"), real("out_gain", "Output gain", "Levels", -24.0, 24.0, 0.0, "dB"), real("noise", "Noise", "Crush", 0.0, 100.0, 0.0, "%"), switch("clip", "Clip", "Levels", true)], vec![], "Reduces the sample rate and the bit depth.");
    fx("Flanger", "Modulation", "FL", 0, false, vec![real("delay", "Delay", "Flanger", 0.1, 10.0, 1.0, "ms"), log("rate", "LFO rate", "Flanger", 0.01, 10.0, 0.25, "Hz"), real("amount", "Amount", "Flanger", 0.0, 10.0, 2.0, "ms"), real("feedback", "Feedback", "Flanger", -100.0, 100.0, 0.0, "%"), real("noise", "Noise", "Flanger", 0.0, 100.0, 0.0, "%"), switch("invert", "Invert", "Flanger", false)], vec![], "A modulated short delay.");
    fx("Stereo Enhancer", "Stereo and utility", "SE", 0, false, vec![real("width", "Width", "Stereo", 0.0, 180.0, 0.0, "")], vec![], "Widens the stereo image.");
    fx("Bass Booster", "EQ and filters", "BB", 0, false, vec![log("freq", "Frequency", "Boost", 10.0, 200.0, 100.0, "Hz"), real("gain", "Gain", "Boost", 0.1, 5.0, 1.0, "x"), real("ratio", "Ratio", "Boost", 0.1, 10.0, 2.0, "")], vec![], "Boosts the low end.");
    fx("Spectrum Analyzer", "Analysis", "SP", 0, false, vec![fixed(switch("stereo", "Stereo", "Display", true)), fixed(switch("peak_hold", "Peak hold", "Display", false)), fixed(switch("log_freq", "Logarithmic frequency", "Display", true)), fixed(stepped("block", "Block size", "Display", 256.0, 16384.0, 2048.0, ""))], vec![], "Shows the spectrum of the signal.");
    fx("Amp and IR", "Distortion", "AI", 128, false, vec![real("input", "Input gain", "Amp", -24.0, 24.0, 0.0, "dB"), switch("model.on", "Amp model", "Amp", true), switch("ir.on", "Cabinet impulse", "Cabinet", true), log("ir.hp", "High-pass", "Cabinet", 20.0, 500.0, 80.0, "Hz"), log("ir.lp", "Low-pass", "Cabinet", 2000.0, 20000.0, 9000.0, "Hz"), real("output", "Output", "Amp", -24.0, 12.0, 0.0, "dB")], vec![], "A Neural Amp Modeler amplifier and a cabinet impulse response.");
    v.push(Descriptor {
        id: "Calf Vintage Delay (LV2)", role: Role::Effect, category: "LV2", short: "LV", host: Host::Lv2, per_note: false,
        resource: None, latency: 256, build_ms: 800, sidechain: false,
        params: vec![real("bpm", "Tempo", "Delay", 30.0, 300.0, 120.0, ""), stepped("subdiv", "Subdivision", "Delay", 1.0, 16.0, 4.0, ""), real("feedback", "Feedback", "Delay", 0.0, 100.0, 50.0, "%"), real("amount", "Amount", "Delay", 0.0, 100.0, 25.0, "%"), switch("ping_pong", "Ping-pong", "Delay", false)],
        presets: vec![], description: "A third-party LV2 effect: it runs outside the program, and its controls come from its ports.",
    });
    v
}

static DESCRIPTORS: LazyLock<Vec<Descriptor>> = LazyLock::new(|| {
    let mut v = instruments();
    v.extend(effects());
    v.push(functions());
    v
});

/// Every descriptor the program has (the plugins not yet rewritten simply do not exist, piece 7a)
pub fn all() -> impl Iterator<Item = &'static Descriptor> {
    DESCRIPTORS.iter().filter(|d| d.id != FUNCTIONS)
}

pub fn descriptor(id: &str) -> Option<&'static Descriptor> {
    DESCRIPTORS.iter().find(|d| d.id == id)
}

/// A parameter of a plugin: its own, or the common effect controls (F-FX-001)
pub fn spec(plugin: &str, key: &str) -> Option<&'static ParamSpec> {
    let d = descriptor(plugin)?;
    d.param(key).or_else(|| if d.role == Role::Effect { COMMON_FX.iter().find(|p| p.key == key) } else { None })
}

/// The output ports of an instrument in its current state (I-9.1 `ports(state)`): AVL Drums with "one output per
/// piece" declares one per piece of the kit
pub fn ports(node: &PluginNode) -> Vec<(&'static str, u8)> {
    if node.plugin == "AVL Drums" && node.real("per_piece") > 0.5 {
        match node.choice("kit") {
            "buskmans" => vec![("main", 2)],
            "blonde_bop" | "blonde_bop_hr" => vec![("kick", 1), ("snare", 1), ("hihat", 1), ("tom", 1), ("oh", 2)],
            _ => vec![("kick", 1), ("snare", 1), ("hihat", 1), ("tom", 1), ("floor", 1), ("oh", 2), ("perc", 2)],
        }
    } else {
        vec![("main", 2)]
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The nodes in the document

/// One plugin's state: the tree of piece 7a, as a struct
#[derive(Clone, Debug)]
pub struct PluginNode {
    pub id: NodeId,
    /// the descriptor's id; a plugin the program lacks keeps its state untouched (F-INS-024)
    pub plugin: &'static str,
    /// the track (instrument, functions) or the desk channel (insert)
    pub owner: NodeId,
    pub reals: BTreeMap<&'static str, f64>,
    pub choices: BTreeMap<&'static str, &'static str>,
    pub preset: Option<String>,
    /// changed since the preset was applied
    pub modified: bool,
    /// the loaded file (sample, SoundFont), by the name the project keeps
    pub resource: Option<String>,
    /// the resource is not where the project says (H17)
    pub missing: bool,
    /// parameters with an automation lane
    pub automated: BTreeSet<&'static str>,
    /// parameters bound to a MIDI controller (CC number)
    pub learned: BTreeMap<&'static str, u8>,
    /// the effect wrote inf/NaN: muted, with the reason (F-FX-001)
    pub fault: Option<&'static str>,
    /// the instrument functions' MIDI devices
    pub devices_in: BTreeSet<&'static str>,
    pub devices_out: BTreeSet<&'static str>,
    /// an insert's sidechain source (a desk channel)
    pub sidechain: Option<NodeId>,
    /// an insert's point relative to the rack's sections (P-20; `INSERT_POINTS`)
    pub point: &'static str,
}

impl PluginNode {
    fn new(id: NodeId, plugin: &'static str, owner: NodeId) -> PluginNode {
        let mut n = PluginNode {
            id, plugin, owner, reals: BTreeMap::new(), choices: BTreeMap::new(), preset: None, modified: false,
            resource: None, missing: false, automated: BTreeSet::new(), learned: BTreeMap::new(), fault: None,
            devices_in: BTreeSet::new(), devices_out: BTreeSet::new(), sidechain: None,
            point: DEFAULT_INSERT_POINT,
        };
        n.reset_to_defaults();
        n
    }
    fn reset_to_defaults(&mut self) {
        self.reals.clear();
        self.choices.clear();
        if let Some(d) = descriptor(self.plugin) {
            let common = if d.role == Role::Effect { COMMON_FX } else { &[] };
            for p in d.params.iter().chain(common) {
                match p.kind {
                    Kind::Choice { def, .. } => {
                        self.choices.insert(p.key, def);
                    }
                    _ => {
                        self.reals.insert(p.key, p.default_value());
                    }
                }
            }
        }
    }
    pub fn real(&self, key: &str) -> f64 {
        self.reals.get(key).copied().unwrap_or(0.0)
    }
    pub fn choice(&self, key: &str) -> &'static str {
        self.choices.get(key).copied().unwrap_or("")
    }
    pub fn descriptor(&self) -> Option<&'static Descriptor> {
        descriptor(self.plugin)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Plugins {
    /// instruments (`p:`) and inserts (`x:`)
    pub nodes: BTreeMap<NodeId, PluginNode>,
    /// an instrument track → its instrument
    pub instrument: BTreeMap<NodeId, NodeId>,
    /// an instrument track → its instrument functions (kept when the instrument is swapped)
    pub functions: BTreeMap<NodeId, PluginNode>,
    /// the project's scales and keymaps (10 slots each; empty until the session fills them)
    pub scales: Vec<ScaleDef>,
    pub keymaps: Vec<KeymapDef>,
}

impl Plugins {
    pub fn node(&self, id: NodeId) -> Option<&PluginNode> {
        self.nodes.get(&id)
    }
    pub fn instrument_of(&self, track: NodeId) -> Option<&PluginNode> {
        self.instrument.get(&track).and_then(|p| self.nodes.get(p))
    }
    pub fn functions_of(&self, track: NodeId) -> Option<&PluginNode> {
        self.functions.get(&track)
    }
    fn any_mut(&mut self, id: NodeId) -> Option<&mut PluginNode> {
        if id.kind == b't' { self.functions.get_mut(&id) } else { self.nodes.get_mut(&id) }
    }
    fn any(&self, id: NodeId) -> Option<&PluginNode> {
        if id.kind == b't' { self.functions.get(&id) } else { self.nodes.get(&id) }
    }
}

static NEXT: AtomicU64 = AtomicU64::new(60_000);
fn next(kind: u8) -> NodeId {
    NodeId { kind, number: NEXT.fetch_add(1, Ordering::Relaxed) }
}

/// The latency of an insert in samples (a plugin the program lacks reports none)
pub fn latency(plugin: &str) -> u32 {
    descriptor(plugin).map_or(0, |d| d.latency)
}

const INSTRUMENT_ROTATION: [&str; 6] = ["TripleOscillator", "AudioFileProcessor", "Sf2 Player", "Lb302", "Kicker", "Dexed (LV2)"];

/// Fills the plugin nodes of a new fake session: an instrument and its functions for every instrument track (the
/// AVL kit on "Bateria AVL", one output per piece, with the outputs reconciled), and a node for every insert. Some
/// variety for the screens: automated parameters, a MIDI-learned knob, a sample that is missing (H17), an effect
/// that wrote NaN (F-FX-001), an insert of a plugin the program lacks (F-INS-024), a third-party insert.
pub fn populate(doc: &mut Doc) {
    let mut r = 0usize;
    let tracks: Vec<(usize, NodeId, TrackKind, String)> =
        doc.tracks.iter().enumerate().map(|(i, t)| (i, t.id, t.kind, t.name.clone())).collect();
    for (i, tid, kind, name) in tracks {
        if kind != TrackKind::Instrument {
            continue;
        }
        let avl = name.contains("AVL");
        let plugin = if avl { "AVL Drums" } else { INSTRUMENT_ROTATION[r % INSTRUMENT_ROTATION.len()] };
        let id = next(b'p');
        let mut n = PluginNode::new(id, plugin, tid);
        match plugin {
            "AVL Drums" => {
                n.reals.insert("per_piece", 1.0);
                n.resource = Some("black_pearl.sf2".into());
            }
            "TripleOscillator" => {
                n.preset = Some("Fat Bass".into());
                apply_values(&mut n, "Fat Bass");
                n.automated.insert("osc1.vol");
                n.learned.insert("osc2.fine_l", 74);
            }
            "AudioFileProcessor" => {
                // every second sampler lost its file: the missing state (H17)
                n.missing = r % 4 == 1;
                n.resource = Some(if n.missing { "Amostras/antigas/kick-velho.wav".into() } else { "Amostras/Baterias/kick-acustico.wav".into() });
            }
            "Sf2 Player" => n.resource = Some("FluidR3_GM.sf2".into()),
            _ => {}
        }
        if !avl {
            r += 1;
        }
        doc.plugins.nodes.insert(id, n);
        doc.plugins.instrument.insert(tid, id);
        let mut f = PluginNode::new(tid, FUNCTIONS, tid);
        if i % 4 == 1 {
            f.reals.insert("filter.on", 1.0);
            f.reals.insert("filter.cutoff", 2400.0);
            f.reals.insert("env.cut.amount", 60.0);
            f.reals.insert("env.cut.decay", 320.0);
            f.reals.insert("env.cut.sustain", 20.0);
            f.automated.insert("filter.cutoff");
        }
        if i % 6 == 3 {
            f.reals.insert("arp.on", 1.0);
            f.choices.insert("arp.type", "minor");
            f.choices.insert("arp.direction", "up_down");
        }
        f.devices_in.insert(MIDI_DEVICES[0]);
        doc.plugins.functions.insert(tid, f);
        reconcile(doc, tid);
    }
    let channels: Vec<(usize, NodeId)> = doc.channels.iter().enumerate().map(|(i, c)| (i, c.id)).collect();
    for (ci, cid) in channels {
        let inserts: Vec<(NodeId, &'static str)> = doc.channels[ci].inserts.iter().map(|x| (x.id, x.plugin)).collect();
        for (j, (xid, plugin)) in inserts.into_iter().enumerate() {
            let mut n = PluginNode::new(xid, plugin, cid);
            if ci % 4 == 1 && j == 0 {
                n.automated.insert(descriptor(plugin).and_then(|d| d.params.first()).map_or("wet", |p| p.key));
            }
            if ci == 5 && j == 0 {
                n.fault = Some("The effect wrote invalid samples (NaN); the channel is muted until it is reset.");
            }
            doc.plugins.nodes.insert(xid, n);
        }
        if ci == 3 {
            // a plugin the program lacks: its state is kept and saved back (F-INS-024)
            let id = next(b'x');
            let mut n = PluginNode::new(id, "Old Tape (LV2)", cid);
            n.reals.insert("saturation", 0.42);
            n.reals.insert("wow", 0.1);
            doc.channels[ci].inserts.push(crate::mixer::Insert { id, plugin: "Old Tape (LV2)", bypass: false });
            doc.plugins.nodes.insert(id, n);
        }
        if ci == 6 {
            let id = next(b'x');
            doc.channels[ci].inserts.push(crate::mixer::Insert { id, plugin: "Calf Vintage Delay (LV2)", bypass: false });
            doc.plugins.nodes.insert(id, PluginNode::new(id, "Calf Vintage Delay (LV2)", cid));
        }
    }
    // variety: some inserts after the EQ, one after the fader
    for (k, n) in doc.plugins.nodes.values_mut().filter(|n| n.id.kind == b'x').enumerate() {
        n.point = match k % 7 { 2 => "post_eq", 5 => "post_fader", _ => DEFAULT_INSERT_POINT };
    }
    doc.plugins.scales = default_scales();
    doc.plugins.keymaps = default_keymaps();
}

fn apply_values(n: &mut PluginNode, preset: &str) -> bool {
    let Some(p) = n.descriptor().and_then(|d| d.presets.iter().find(|p| p.name == preset)).cloned() else { return false };
    n.reset_to_defaults();
    for (k, v) in p.values {
        if let Some(s) = spec(n.plugin, k) {
            n.reals.insert(s.key, s.clamp(v));
        }
    }
    for (k, c) in p.choices {
        n.choices.insert(k, c);
    }
    true
}

/// I-3.9: the track's outputs follow the instrument's ports, in the same transaction. A port that stays keeps its
/// output; a new port gets an output to the main output's destination; a port that went away loses its output.
/// Returns the ports whose outputs went away (the notice says which).
fn reconcile(doc: &mut Doc, track: NodeId) -> Vec<&'static str> {
    let Some(node) = doc.plugins.instrument_of(track) else { return Vec::new() };
    let ports = ports(node);
    let Some(t) = doc.tracks.iter_mut().find(|t| t.id == track) else { return Vec::new() };
    let main_to = t.outputs.first().map(|o| o.to).unwrap_or(NodeId { kind: b'm', number: 999 });
    let removed: Vec<&'static str> = t.outputs.iter().filter(|o| !ports.iter().any(|(p, _)| *p == o.port)).map(|o| o.port).collect();
    let mut outs: Vec<Output> = Vec::new();
    for (port, channels) in ports {
        match t.outputs.iter().find(|o| o.port == port) {
            Some(o) => outs.push(Output { channels, ..o.clone() }),
            None => outs.push(Output { id: next(b'o'), port, channels, to: main_to }),
        }
    }
    t.outputs = outs;
    removed
}

// --- reading and writing parameters (the `Edit::set` arm for `Field::Param` on tracks, instruments, inserts) ------

pub(crate) fn set_param(doc: &mut Doc, id: NodeId, key: &'static str, v: f64) -> Result<()> {
    let n = doc.plugins.any_mut(id).ok_or(Error::NoSuchNode(id))?;
    let s = spec(n.plugin, key).ok_or(Error::WrongType)?;
    if matches!(s.kind, Kind::Choice { .. }) {
        return Err(Error::WrongType);
    }
    n.reals.insert(s.key, s.clamp(v));
    n.modified = true;
    if key == "per_piece" {
        let owner = n.owner;
        reconcile(doc, owner);
    }
    Ok(())
}

pub(crate) fn get_param(doc: &Doc, id: NodeId, key: &'static str) -> Result<f64> {
    let n = doc.plugins.any(id).ok_or(Error::NoSuchNode(id))?;
    n.reals.get(key).copied().ok_or(Error::WrongType)
}

/// The edits on plugins that are not one value: choices, swapping, the insert chain. Each pushes the node and its
/// owner into the diff (the windows' feeds are on the owner).
impl Edit<'_> {
    fn touched_node(&mut self, id: NodeId, owner: NodeId) {
        self.diff.changed.push((id, None));
        self.diff.changed.push((owner, None));
    }

    /// A choice by its key (U30). On a track id: the instrument functions'
    pub fn set_choice(&mut self, id: NodeId, key: &'static str, value: &'static str) -> Result<()> {
        let n = self.doc.plugins.any_mut(id).ok_or(Error::NoSuchNode(id))?;
        let s = spec(n.plugin, key).ok_or(Error::WrongType)?;
        let Kind::Choice { options, .. } = s.kind else { return Err(Error::WrongType) };
        if !options.iter().any(|(k, _)| *k == value) {
            return Err(Error::WrongType);
        }
        n.choices.insert(s.key, value);
        n.modified = true;
        let owner = n.owner;
        if n.plugin == "AVL Drums" && key == "kit" {
            n.resource = Some(format!("{value}.sf2"));
            reconcile(self.doc, owner);
        }
        self.touched_node(id, owner);
        Ok(())
    }

    /// Swaps a track's instrument without removing the track (design 05): a new node, built in an operation; the
    /// functions stay; the outputs follow the new ports (I-3.9)
    pub fn swap_instrument(&mut self, track: NodeId, plugin: &'static str) -> Result<NodeId> {
        let d = descriptor(plugin).filter(|d| d.role == Role::Instrument && d.id != FUNCTIONS).ok_or(Error::WrongType)?;
        if !self.doc.tracks.iter().any(|t| t.id == track) {
            return Err(Error::NoSuchNode(track));
        }
        if let Some(old) = self.doc.plugins.instrument.remove(&track) {
            self.doc.plugins.nodes.remove(&old);
            self.diff.changed.push((old, None));
        }
        let id = next(b'p');
        let mut n = PluginNode::new(id, d.id, track);
        if let Some(r) = d.resource {
            n.resource = Some(match r {
                "Kit" => "black_pearl.sf2".into(),
                "SoundFont" => "FluidR3_GM.sf2".into(),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty());
        }
        self.doc.plugins.nodes.insert(id, n);
        self.doc.plugins.instrument.insert(track, id);
        self.doc.plugins.functions.entry(track).or_insert_with(|| PluginNode::new(track, FUNCTIONS, track));
        reconcile(self.doc, track);
        self.touched_node(id, track);
        Ok(id)
    }

    /// Applies a preset: the whole state at once, one undo step
    pub fn apply_preset(&mut self, id: NodeId, preset: &str) -> Result<()> {
        let n = self.doc.plugins.any_mut(id).ok_or(Error::NoSuchNode(id))?;
        let found = apply_values(n, preset) || apply_user_preset(n, preset);
        if !found {
            return Err(Error::WrongType);
        }
        n.preset = Some(preset.to_string());
        n.modified = false;
        let owner = n.owner;
        reconcile(self.doc, owner);
        self.touched_node(id, owner);
        Ok(())
    }

    /// Loads a file into the node (a sample into the sampler, a SoundFont); clears the missing state
    pub fn set_resource(&mut self, id: NodeId, file: &str) -> Result<()> {
        let n = self.doc.plugins.any_mut(id).ok_or(Error::NoSuchNode(id))?;
        n.resource = Some(file.to_string());
        n.missing = false;
        n.modified = true;
        let owner = n.owner;
        self.touched_node(id, owner);
        Ok(())
    }

    pub fn set_automated(&mut self, id: NodeId, key: &'static str, on: bool) -> Result<()> {
        let n = self.doc.plugins.any_mut(id).ok_or(Error::NoSuchNode(id))?;
        let s = spec(n.plugin, key).ok_or(Error::WrongType)?;
        if !s.automatable {
            return Err(Error::WrongType);
        }
        if on { n.automated.insert(s.key) } else { n.automated.remove(s.key) };
        let owner = n.owner;
        self.touched_node(id, owner);
        Ok(())
    }

    /// Binds a parameter to the next free MIDI CC (the fake's "learn": the real one waits for a moved control)
    pub fn set_learned(&mut self, id: NodeId, key: &'static str, on: bool) -> Result<()> {
        let n = self.doc.plugins.any_mut(id).ok_or(Error::NoSuchNode(id))?;
        let s = spec(n.plugin, key).ok_or(Error::WrongType)?;
        if on {
            let cc = (20u8..120).find(|c| !n.learned.values().any(|x| x == c)).unwrap_or(1);
            n.learned.insert(s.key, cc);
        } else {
            n.learned.remove(s.key);
        }
        let owner = n.owner;
        self.touched_node(id, owner);
        Ok(())
    }

    pub fn set_device(&mut self, track: NodeId, output: bool, device: &'static str, on: bool) -> Result<()> {
        let n = self.doc.plugins.functions.get_mut(&track).ok_or(Error::NoSuchNode(track))?;
        let set = if output { &mut n.devices_out } else { &mut n.devices_in };
        if on { set.insert(device) } else { set.remove(device) };
        self.touched_node(track, track);
        Ok(())
    }

    /// Resets a faulted effect (it wrote inf/NaN): it plays again
    pub fn clear_fault(&mut self, id: NodeId) -> Result<()> {
        let n = self.doc.plugins.nodes.get_mut(&id).ok_or(Error::NoSuchNode(id))?;
        n.fault = None;
        let owner = n.owner;
        self.touched_node(id, owner);
        Ok(())
    }

    // --- the insert chain of a desk channel (F-FX-002) -----------------------------------------------------------

    fn channel_mut(&mut self, ch: NodeId) -> Result<&mut crate::Channel> {
        self.doc.channels.iter_mut().find(|c| c.id == ch).ok_or(Error::NoSuchNode(ch))
    }
    fn chain_of(&self, insert: NodeId) -> Result<(usize, usize)> {
        for (ci, c) in self.doc.channels.iter().enumerate() {
            if let Some(k) = c.inserts.iter().position(|x| x.id == insert) {
                return Ok((ci, k));
            }
        }
        Err(Error::NoSuchNode(insert))
    }

    /// Adds an effect at `index` (clamped) of the channel's chain
    pub fn add_insert(&mut self, ch: NodeId, index: usize, plugin: &'static str) -> Result<NodeId> {
        let d = descriptor(plugin).filter(|d| d.role == Role::Effect).ok_or(Error::WrongType)?;
        let id = next(b'x');
        let c = self.channel_mut(ch)?;
        let at = index.min(c.inserts.len());
        c.inserts.insert(at, crate::mixer::Insert { id, plugin: d.id, bypass: false });
        self.doc.plugins.nodes.insert(id, PluginNode::new(id, d.id, ch));
        self.touched_node(id, ch);
        Ok(id)
    }

    /// Removes an insert; a sidechain that pointed at it goes with it (`OnRemove::RemoveOwner`)
    pub fn remove_insert(&mut self, insert: NodeId) -> Result<()> {
        let (ci, k) = self.chain_of(insert)?;
        let ch = self.doc.channels[ci].id;
        self.doc.channels[ci].inserts.remove(k);
        self.doc.plugins.nodes.remove(&insert);
        self.touched_node(insert, ch);
        Ok(())
    }

    /// Moves an insert to `to` within its chain
    pub fn move_insert(&mut self, insert: NodeId, to: usize) -> Result<()> {
        let (ci, k) = self.chain_of(insert)?;
        let c = &mut self.doc.channels[ci];
        let x = c.inserts.remove(k);
        let to = to.min(c.inserts.len());
        c.inserts.insert(to, x);
        let ch = c.id;
        self.touched_node(insert, ch);
        Ok(())
    }

    /// "Replace with…": another effect in the same place, a new node
    pub fn replace_insert(&mut self, insert: NodeId, plugin: &'static str) -> Result<NodeId> {
        let (ci, k) = self.chain_of(insert)?;
        let ch = self.doc.channels[ci].id;
        self.remove_insert(insert)?;
        let _ = k;
        self.add_insert(ch, k, plugin)
    }

    pub fn set_bypass(&mut self, insert: NodeId, bypass: bool) -> Result<()> {
        let (ci, k) = self.chain_of(insert)?;
        self.doc.channels[ci].inserts[k].bypass = bypass;
        let ch = self.doc.channels[ci].id;
        self.touched_node(insert, ch);
        Ok(())
    }

    /// The source of an insert's sidechain input: another channel, never one that would make a cycle
    pub fn set_sidechain(&mut self, insert: NodeId, source: Option<NodeId>) -> Result<()> {
        let (ci, _) = self.chain_of(insert)?;
        let ch = self.doc.channels[ci].id;
        if let Some(src) = source {
            // the source must not be fed by this channel (it would be a cycle)
            let mut at = Some(ch);
            while let Some(n) = at {
                if n == src {
                    return Err(Error::WrongType);
                }
                at = self.doc.channels.iter().find(|c| c.id == n).and_then(|c| c.output);
            }
        }
        let n = self.doc.plugins.nodes.get_mut(&insert).ok_or(Error::NoSuchNode(insert))?;
        n.sidechain = source;
        self.touched_node(insert, ch);
        Ok(())
    }

    /// An insert's point relative to the rack (P-20)
    pub fn set_insert_point(&mut self, insert: NodeId, point: &'static str) -> Result<()> {
        let key = INSERT_POINTS.iter().find(|(k, _)| *k == point).ok_or(Error::WrongType)?.0;
        let (ci, _) = self.chain_of(insert)?;
        let ch = self.doc.channels[ci].id;
        self.doc.plugins.nodes.get_mut(&insert).ok_or(Error::NoSuchNode(insert))?.point = key;
        self.touched_node(insert, ch);
        Ok(())
    }

    /// Replaces a scale slot; refused whole when a line is not an interval (F-IJN-020)
    pub fn set_scale(&mut self, slot: usize, def: ScaleDef) -> Result<()> {
        parse_intervals(&def.intervals).map_err(|_| Error::WrongType)?;
        *self.doc.plugins.scales.get_mut(slot).ok_or(Error::WrongType)? = def;
        self.diff.changed.push((crate::ROOT, None));
        Ok(())
    }

    pub fn set_keymap(&mut self, slot: usize, def: KeymapDef) -> Result<()> {
        if def.first > def.last || !(0.1..=9999.999).contains(&def.base_freq) {
            return Err(Error::WrongType);
        }
        for d in def.degrees.split_whitespace() {
            if d != "x" && d.parse::<u32>().is_err() {
                return Err(Error::WrongType);
            }
        }
        *self.doc.plugins.keymaps.get_mut(slot).ok_or(Error::WrongType)? = def;
        self.diff.changed.push((crate::ROOT, None));
        Ok(())
    }
}

// --- user presets: a fake of the preset files on disk ------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct UserPreset {
    pub plugin: &'static str,
    pub name: String,
    pub reals: BTreeMap<&'static str, f64>,
    pub choices: BTreeMap<&'static str, &'static str>,
}

thread_local! {
    static USER_PRESETS: RefCell<Vec<UserPreset>> = const { RefCell::new(Vec::new()) };
}

/// Saves the node's state as a user preset (a file in the user's presets folder); returns its name
pub fn save_user_preset(n: &PluginNode) -> String {
    USER_PRESETS.with(|u| {
        let mut u = u.borrow_mut();
        let count = u.iter().filter(|p| p.plugin == n.plugin).count();
        let name = format!("{} {}", n.preset.as_deref().unwrap_or("Preset"), count + 1);
        u.push(UserPreset { plugin: n.plugin, name: name.clone(), reals: n.reals.clone(), choices: n.choices.clone() });
        name
    })
}

pub fn user_presets(plugin: &str) -> Vec<String> {
    USER_PRESETS.with(|u| u.borrow().iter().filter(|p| p.plugin == plugin).map(|p| p.name.clone()).collect())
}

fn apply_user_preset(n: &mut PluginNode, name: &str) -> bool {
    USER_PRESETS.with(|u| {
        let u = u.borrow();
        let Some(p) = u.iter().find(|p| p.plugin == n.plugin && p.name == name) else { return false };
        n.reals = p.reals.clone();
        n.choices = p.choices.clone();
        true
    })
}

/// Every preset of a plugin: the factory ones, then the user's
pub fn presets_of(plugin: &str) -> Vec<String> {
    let mut v: Vec<String> = descriptor(plugin).map(|d| d.presets.iter().map(|p| p.name.to_string()).collect()).unwrap_or_default();
    v.extend(user_presets(plugin));
    v
}

// --- operations (I-2) ---------------------------------------------------------------------------------------------

/// Builds a plugin's processor on a worker (I-9.1 `build`): the node is already in its place; past 200 ms the
/// window shows the loading in place of the controls (I-2.3)
pub fn build(s: &mut Session, node: NodeId, plugin: &str) {
    let ms = descriptor(plugin).map_or(10, |d| d.build_ms);
    let stage = if descriptor(plugin).is_some_and(|d| d.resource.is_some()) { "Loading the file" } else { "Building the plugin" };
    let _ = s.start(stage, Target::Node(node), true, move |cancel, progress| {
        let steps = 20u32;
        for i in 0..steps {
            if cancel.is_cancelled() {
                return Err("cancelled".to_string());
            }
            std::thread::sleep(Duration::from_millis(ms) / steps);
            progress.set((i + 1) as f32 / steps as f32);
        }
        Ok(())
    });
}

/// Checks a file the user located for a missing resource (H17: comparing the hash is an operation)
pub fn locate(s: &mut Session, node: NodeId, file: String) {
    let _ = s.start("Checking the file", Target::Node(node), true, move |_, progress| {
        for i in 0..10 {
            std::thread::sleep(Duration::from_millis(30));
            progress.set((i + 1) as f32 / 10.0);
        }
        Ok(())
    });
    let _ = s.transact("Locate the file", |e| e.set_resource(node, &file));
}

/// "Locate folder" (H17): resolves every missing resource of a library that moved, at once
pub fn locate_folder(s: &mut Session, folder: &str) -> usize {
    let missing: Vec<(NodeId, String)> = s
        .doc()
        .plugins
        .nodes
        .values()
        .filter(|n| n.missing)
        .map(|n| (n.id, n.resource.clone().unwrap_or_default()))
        .collect();
    let count = missing.len();
    let _ = s.transact("Locate the folder", |e| {
        for (id, file) in &missing {
            let name = file.rsplit('/').next().unwrap_or(file);
            e.set_resource(*id, &format!("{folder}/{name}"))?;
        }
        Ok(())
    });
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Field, Value};

    fn session() -> Session {
        Session::create(Config { tracks: 12, channels: 8, slow_delay: Duration::from_millis(10), slow_track: 2, item_seconds: 5.0 })
    }

    #[test]
    fn every_schema_is_coherent() {
        for d in DESCRIPTORS.iter() {
            let mut keys = BTreeSet::new();
            for p in &d.params {
                assert!(keys.insert(p.key), "{}: duplicate key {}", d.id, p.key);
                match p.kind {
                    Kind::Real { min, max, def, scale } => {
                        assert!(min < max && def >= min && def <= max, "{}: {} range", d.id, p.key);
                        if scale == Scale::Log {
                            assert!(min > 0.0, "{}: {} log needs min > 0", d.id, p.key);
                        }
                        if scale == Scale::Stepped {
                            assert!(min.fract() == 0.0 && max.fract() == 0.0 && def.fract() == 0.0, "{}: {}", d.id, p.key);
                        }
                    }
                    Kind::Choice { options, def } => assert!(options.iter().any(|(k, _)| *k == def), "{}: {}", d.id, p.key),
                    Kind::Bool { .. } => {}
                }
            }
            for pr in &d.presets {
                for (k, _) in &pr.values {
                    assert!(d.param(k).is_some(), "{}: preset {} has unknown {k}", d.id, pr.name);
                }
            }
        }
    }

    #[test]
    fn travel_round_trips_on_log_and_stepped() {
        let d = descriptor("Kicker").unwrap();
        let p = d.param("start_freq").unwrap();
        for v in [5.0, 40.0, 150.0, 1000.0] {
            assert!((p.from_travel(p.to_travel(v)) - v).abs() < 1e-6);
        }
        let s = descriptor("TripleOscillator").unwrap().param("osc2.coarse").unwrap();
        assert_eq!(s.from_travel(0.52), 1.0, "stepped values land on whole numbers");
    }

    #[test]
    fn params_edit_and_undo_through_the_document() {
        let mut s = session();
        let track = s.tracks().iter().find(|t| s.doc().plugins.instrument_of(t.id).is_some_and(|n| n.plugin == "TripleOscillator")).unwrap().id;
        let p = s.doc().plugins.instrument.get(&track).copied().unwrap();
        s.transact("Knob", |e| e.set(p, Field::Param("osc1.vol"), Value::Real(150.0))).unwrap();
        assert_eq!(s.doc().plugins.node(p).unwrap().real("osc1.vol"), 150.0);
        s.transact("Choice", |e| e.set_choice(p, "osc1.wave", "square")).unwrap();
        s.undo().unwrap();
        s.undo().unwrap();
        assert_eq!(s.doc().plugins.node(p).unwrap().real("osc1.vol"), 60.0, "back to the preset's value");
        // the functions are on the track id
        s.transact("Filter", |e| e.set(track, Field::Param("filter.cutoff"), Value::Real(99_999.0))).unwrap();
        assert_eq!(s.doc().plugins.functions_of(track).unwrap().real("filter.cutoff"), 14000.0, "clamped");
    }

    #[test]
    fn swapping_the_kit_reconciles_the_outputs() {
        let mut s = session();
        let track = s.tracks().iter().find(|t| t.name.contains("AVL")).unwrap().id;
        let p = s.doc().plugins.instrument.get(&track).copied().unwrap();
        let outs = |s: &Session| s.tracks().iter().find(|t| t.id == track).unwrap().outputs.iter().map(|o| o.port).collect::<Vec<_>>();
        assert_eq!(outs(&s), ["kick", "snare", "hihat", "tom", "floor", "oh", "perc"]);
        let kick = s.tracks().iter().find(|t| t.id == track).unwrap().outputs[0].clone();
        s.transact("Kit", |e| e.set_choice(p, "kit", "blonde_bop")).unwrap();
        assert_eq!(outs(&s), ["kick", "snare", "hihat", "tom", "oh"]);
        let kick2 = s.tracks().iter().find(|t| t.id == track).unwrap().outputs[0].clone();
        assert_eq!((kick.id, kick.to), (kick2.id, kick2.to), "a port that stays keeps its output");
        s.transact("Swap", |e| e.swap_instrument(track, "TripleOscillator").map(|_| ())).unwrap();
        assert_eq!(outs(&s), ["main"]);
        assert!(s.doc().plugins.functions_of(track).is_some(), "the functions stay");
    }

    #[test]
    fn the_insert_chain() {
        let mut s = session();
        let ch = s.channels()[0].id;
        let a = { let mut id = None; s.transact("Add", |e| { id = Some(e.add_insert(ch, 99, "Compressor")?); Ok(()) }).unwrap(); id.unwrap() };
        let b = { let mut id = None; s.transact("Add", |e| { id = Some(e.add_insert(ch, 0, "EQ 8")?); Ok(()) }).unwrap(); id.unwrap() };
        let order = |s: &Session| s.channels()[0].inserts.iter().map(|x| x.id).collect::<Vec<_>>();
        assert_eq!(order(&s), [b, a]);
        s.transact("Move", |e| e.move_insert(b, 1)).unwrap();
        assert_eq!(order(&s), [a, b]);
        s.transact("Bypass", |e| e.set_bypass(a, true)).unwrap();
        assert!(s.channels()[0].inserts[0].bypass);
        s.transact("Remove", |e| e.remove_insert(a)).unwrap();
        assert_eq!(order(&s), [b]);
        assert!(s.doc().plugins.node(a).is_none());
        s.undo().unwrap();
        assert_eq!(order(&s), [a, b]);
        assert!(s.transact("Instrument as insert", |e| e.add_insert(ch, 0, "TripleOscillator").map(|_| ())).is_err());
    }
}

#[cfg(test)]
mod tuning_tests {
    use super::*;

    #[test]
    fn scale_intervals_parse_cents_and_ratios() {
        let v = parse_intervals("! a comment\n701.955\n3/2\n2").unwrap();
        assert!((v[0] - 701.955).abs() < 1e-9 && (v[1] - 701.955).abs() < 0.01 && (v[2] - 1200.0).abs() < 1e-9);
        assert_eq!(parse_intervals("100.0\nabc"), Err(2), "the first bad line");
        assert_eq!(parse_intervals("0/2"), Err(1));
    }
}
