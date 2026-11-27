//! Xpressive (F-INS-022: legacy `plugins/Xpressive`): a note made of two mathematical expressions, O1 and O2, each
//! with its own panning, over three wave tables W1, W2 and W3 that are drawn or written as expressions too. The
//! expressions are the legacy's exprtk language, compiled by `lm-expr`.
//!
//! State, version 1:
//! - fields `o1` and `o2` (the output expressions; the legacy's defaults), `w1Expr`, `w2Expr` and `w3Expr` (a wave's
//!   expression: when it is not empty and compiles it makes the wave, else the drawn points do), `w1Graph`,
//!   `w2Graph` and `w3Graph` (the drawn points, 4096 from −1 to 1; fewer are padded with silence, none is silence),
//!   `smoothW1`, `smoothW2` and `smoothW3` (0–70: the gaussian's width that smooths the wave; a field and not a
//!   parameter because the table is made again when it changes, off the audio thread);
//! - parameters `a1`, `a2`, `a3` (−1–1, 1: the variables `A1` to `A3`), `panning1` (−1–1, 1) and `panning2` (−1–1,
//!   −1), `relTransition` (0–500 ms, 50: how long `rel` takes to go from 0 to 1 after the note is let go),
//!   `interpolateW1` to `interpolateW3` (switches, off: each wave read with linear interpolation, read at the note's
//!   start), and `baseNote` (0–127, 69: the legacy's `bnote`, which it took from the instrument's track);
//! - the instrument functions.
//!
//! `v` is the note's velocity as the legacy's volume over 255 (a velocity of 1 is the legacy's default volume of
//! 100). The knobs `a1` to `a3` and the panning act on the sounding notes; the rest is taken at the note's start.
//!
//! An expression that does not compile is not played (the legacy left the output out, or the wave as drawn): the
//! editor asks [`check`] for the error's position as the text is typed and keeps the last good text with [`edit`].

use std::sync::Arc;

use lm_base::Result;
use lm_dsp::xpressive::{
    ExprVoice, Knobs, Language, Needs, NoteInit, Patch, REL_TRANSITION_MAX, SMOOTH_MAX, WAVE_LENGTH, wave_table,
};
use lm_expr::CompileError;
use lm_plugin::instrument::{Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::stereo_out;

pub const ID: &str = "xpressive";
pub const VOICES: usize = 32;
/// The default output expressions (the legacy's, each at half its old level)
pub const DEFAULT_O1: &str =
    "0.5*sinew(integrate(f*(1+0.05sinew(12t))))*(2^(-(1.1+A2)*t)*(0.4+0.1(1+A3)+0.4sinew((2.5+2A1)t))^2)";
pub const DEFAULT_O2: &str = "0.5*(expw(integrate(f*atan(500t)*2/pi))*0.5+0.12)";
/// The live frame (f32s): one flag for each expression in the order W1, W2, W3, O1, O2 (1 = does not compile), then
/// the curve each output makes (O1, then O2): [`PREVIEW_POINTS`] points from -1 to 1
pub const FLAGS: usize = 5;
pub const PREVIEW_POINTS: usize = 256;
pub const FRAME_FLOATS: usize = FLAGS + 2 * PREVIEW_POINTS;
/// What the preview plays: a note of A4 and this long
const PREVIEW_HZ: f32 = 440.0;
const PREVIEW_SECONDS: f32 = 0.5;
pub const WAVE_EXPR_FIELDS: [&str; 3] = ["w1Expr", "w2Expr", "w3Expr"];
pub const WAVE_GRAPH_FIELDS: [&str; 3] = ["w1Graph", "w2Graph", "w3Graph"];
pub const SMOOTH_FIELDS: [&str; 3] = ["smoothW1", "smoothW2", "smoothW3"];
/// The parameters: key, minimum, maximum, default, unit, stepped
pub const PARAMS: [(&str, f64, f64, f64, &str, bool); 10] = [
    ("a1", -1.0, 1.0, 1.0, "", false),
    ("a2", -1.0, 1.0, 1.0, "", false),
    ("a3", -1.0, 1.0, 1.0, "", false),
    ("panning1", -1.0, 1.0, 1.0, "", false),
    ("panning2", -1.0, 1.0, -1.0, "", false),
    ("relTransition", 0.0, REL_TRANSITION_MAX as f64, 50.0, "ms", false),
    ("interpolateW1", 0.0, 1.0, 0.0, "", true),
    ("interpolateW2", 0.0, 1.0, 0.0, "", true),
    ("interpolateW3", 0.0, 1.0, 0.0, "", true),
    ("baseNote", 0.0, 127.0, 69.0, "", true),
];
const P_A: usize = 0;
const P_PAN: usize = 3;
const P_REL: usize = 5;
const P_INTERP: usize = 6;
const P_BASE: usize = 9;
/// The legacy's note volume of a velocity of 1 (the default volume, 100), over its 255
const VOLUME_PER_VELOCITY: f32 = 100.0 / 255.0;

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.xpressive.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: Some(DisplaySpec { bytes: FRAME_FLOATS * 4 }),
    build,
    // (no tuning probe: the pitch is what the expressions make of `f`)
    tests: TestHints { voices: VOICES, ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:xpressive", 1);
    for (key, min, max, def, unit, stepped) in PARAMS {
        let p = ParamSpec::new(key, min, max, def);
        let p = if unit.is_empty() { p } else { p.unit(unit) };
        s = s.param(if stepped { p.scale(Scale::Stepped) } else { p });
    }
    s = s.field(FieldSpec::text("o1", DEFAULT_O1)).field(FieldSpec::text("o2", DEFAULT_O2));
    for key in WAVE_EXPR_FIELDS {
        s = s.field(FieldSpec::text(key, ""));
    }
    for key in WAVE_GRAPH_FIELDS {
        s = s.field(
            FieldSpec::new(key, FieldKind::List(Box::new(FieldKind::Real { min: -2.0, max: 2.0 })))
                .default(Value::list(std::iter::empty::<Value>())),
        );
    }
    for key in SMOOTH_FIELDS {
        s = s.field(FieldSpec::int(key, 0, SMOOTH_MAX as i64, 0));
    }
    vec![add_instrument_functions(s)]
}

/// Which language an expression is written in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpressionKind {
    /// O1 and O2: `t f key rel trel A1 A2 A3 v seed srate bnote tempo`, the functions, `W1` to `W3`
    Output,
    /// W1 to W3: `t seed srate` and the functions
    Wave,
}

/// Whether `source` is acceptable as an expression of `kind`: it compiles (a wave's text may also be empty: the drawn
/// points play). The error says where the text goes wrong (`position`, a byte offset) and why (`label`, a
/// translatable key). Allocates and works: not for the audio thread
pub fn check(kind: ExpressionKind, source: &str) -> std::result::Result<(), CompileError> {
    if kind == ExpressionKind::Wave && source.is_empty() {
        return Ok(());
    }
    let language = match kind {
        ExpressionKind::Output => Language::output(),
        ExpressionKind::Wave => Language::wave(),
    };
    language.compile(source).map(|_| ())
}

/// The result of an edit of an expression's text
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    /// The text to keep in the state: the candidate when it is acceptable, else the last good one
    pub text: String,
    /// Why the candidate is not (and where): to show in the editor beside the text
    pub error: Option<CompileError>,
}

/// An edit: a candidate text over the last good one. A text that does not compile keeps the previous good one in the
/// state (the sound does not change) and reports its error
pub fn edit(kind: ExpressionKind, last_good: &str, candidate: &str) -> Edit {
    match check(kind, candidate) {
        Ok(()) => Edit { text: candidate.to_owned(), error: None },
        Err(e) => Edit { text: last_good.to_owned(), error: Some(e) },
    }
}

/// What a state's expressions say of themselves, as the editor shows it
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    /// O1 and O2
    pub outputs: [Option<CompileError>; 2],
    /// W1 to W3
    pub waves: [Option<CompileError>; 3],
}

/// The compile error of each expression of `state` (`None`: fine, or empty for a wave). Off the audio thread
pub fn status(state: &Value) -> Status {
    let s = StateView::new(state);
    let err = |kind, key: &str| s.text(key).ok().and_then(|t| check(kind, t).err());
    Status {
        outputs: [err(ExpressionKind::Output, "o1"), err(ExpressionKind::Output, "o2")],
        waves: [
            err(ExpressionKind::Wave, WAVE_EXPR_FIELDS[0]),
            err(ExpressionKind::Wave, WAVE_EXPR_FIELDS[1]),
            err(ExpressionKind::Wave, WAVE_EXPR_FIELDS[2]),
        ],
    }
}

#[derive(Clone, Debug)]
pub struct Xpressive {
    rate: u32,
    patch: Arc<Patch>,
    /// What the pool's voices have room for
    room: Needs,
    values: [f32; PARAMS.len()],
    knobs: Knobs,
    tempo: f64,
    /// What the window shows ([`FRAME_FLOATS`]), made at the build
    frame: Arc<[f32]>,
}

/// The curve output `k` makes alone, as the window draws it: a note of [`PREVIEW_HZ`] with the knobs as they are, in
/// [`PREVIEW_POINTS`] columns, each the sample of the largest size in its stretch. Works and allocates: not for the
/// audio thread
fn preview(outputs: [&str; 2], tables: &[Vec<f32>; 3], k: usize, rate: u32, values: &[f32], knobs: &Knobs) -> Vec<f32> {
    let mut alone = ["", ""];
    alone[k] = outputs[k];
    let (patch, _) = Patch::new(alone, tables.clone());
    if !patch.valid(k) {
        return vec![0.0; PREVIEW_POINTS];
    }
    let room = patch.needs();
    let mut voice = ExprVoice::new(&room, rate);
    let on = |i: usize| values[P_INTERP + i] >= 0.5;
    voice.start(
        &patch,
        &NoteInit {
            key: 69.0,
            base_note: values[P_BASE],
            volume: 1.0 * VOLUME_PER_VELOCITY,
            tempo: 120.0,
            freq: PREVIEW_HZ,
            release_ms: values[P_REL],
            interpolate: [on(0), on(1), on(2)],
            random: 0,
        },
    );
    let total = ((rate as f32 * PREVIEW_SECONDS) as usize).max(PREVIEW_POINTS);
    let (mut l, mut r) = (vec![0.0f32; total], vec![0.0f32; total]);
    // (one block: the expression's state does not depend on how the note is cut)
    let mut centered = *knobs;
    centered.pan = [0.0, 0.0];
    voice.render(&patch, &centered, PREVIEW_HZ, &mut l, &mut r);
    (0..PREVIEW_POINTS)
        .map(|c| {
            let (a, b) = (c * total / PREVIEW_POINTS, (c + 1) * total / PREVIEW_POINTS);
            // (panning 0 gives each side half of the output: the sum is the output itself)
            let mut best = 0.0f32;
            for i in a..b.max(a + 1).min(total) {
                let v = l[i] + r[i];
                if v.abs() > best.abs() {
                    best = v;
                }
            }
            best.clamp(-1.0, 1.0)
        })
        .collect()
}

impl Xpressive {
    fn update(&mut self) {
        let v = &self.values;
        self.knobs = Knobs { a: core::array::from_fn(|i| v[P_A + i]), pan: [v[P_PAN], v[P_PAN + 1]] };
    }
}

/// One note
#[derive(Clone, Debug)]
pub struct Voice {
    sound: ExprVoice,
}

impl Synth for Xpressive {
    type Voice = Voice;
    const PARAMS: usize = PARAMS.len();

    fn voice(&self) -> Voice {
        Voice { sound: ExprVoice::new(&self.room, self.rate) }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(v) = self.values.get_mut(index) {
            *v = value;
            self.update();
        }
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.tempo = bpm;
    }

    fn start(&self, voice: &mut Voice, note: &NoteStart, rng: &mut Rng) {
        let v = &self.values;
        let on = |i: usize| v[P_INTERP + i] >= 0.5;
        let init = NoteInit {
            key: f32::from(note.key),
            base_note: v[P_BASE],
            volume: note.velocity * VOLUME_PER_VELOCITY,
            tempo: self.tempo as f32,
            freq: note.freq,
            release_ms: v[P_REL],
            interpolate: [on(0), on(1), on(2)],
            random: rng.next_u64(),
        };
        voice.sound.start(&self.patch, &init);
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        voice.sound.render(&self.patch, &self.knobs, freq, left, right);
    }

    fn release(&self, voice: &mut Voice) {
        voice.sound.release();
    }

    // (the legacy asks for no release of its own: `rel` and `trel` follow the instrument's envelope release)

    fn adopt(&mut self, old: &mut Self) {
        // The pool is the old one's: its room is what these notes have
        self.room = old.room;
    }

    fn can_adopt(&self, old: &Self) -> bool {
        self.patch.needs().fit_in(&old.room)
    }

    fn display(&self, frame: &mut [u8]) {
        frame.fill(0);
        for (chunk, v) in frame.chunks_exact_mut(4).zip(self.frame.iter()) {
            chunk.copy_from_slice(&v.to_le_bytes());
        }
    }

    fn copies_voices(&self) -> bool {
        // (a voice holds the `last` history and the `integrate` sums of its expressions: copying them allocates)
        false
    }
}

/// What a state's parameters, outputs and tables are
type Read<'a> = ([f32; PARAMS.len()], [&'a str; 2], [Vec<f32>; 3]);

fn read(state: &Value) -> Result<Read<'_>> {
    let s = StateView::new(state);
    let mut values = [0.0f32; PARAMS.len()];
    for (v, (key, ..)) in values.iter_mut().zip(PARAMS) {
        *v = s.param_f32(key)?;
    }
    let outputs = [s.text("o1")?, s.text("o2")?];
    let mut tables: [Vec<f32>; 3] = Default::default();
    for (i, table) in tables.iter_mut().enumerate() {
        let source = s.text(WAVE_EXPR_FIELDS[i])?;
        let drawn: Vec<f32> = state
            .get(WAVE_GRAPH_FIELDS[i])
            .map(|l| l.items().filter_map(Value::as_real).map(|v| v as f32).take(WAVE_LENGTH).collect())
            .unwrap_or_default();
        let smoothness = s.int(SMOOTH_FIELDS[i])? as f32;
        // (an expression that does not compile leaves the drawn points: see `status` for what the editor shows)
        *table = wave_table(source, &drawn, smoothness).0;
    }
    Ok((values, outputs, tables))
}

/// The window's frame ([`FRAME_FLOATS`]) of a state: the flags of the five expressions, then the two outputs' curves.
/// Works and allocates: the build makes it once
fn make_frame(state: &Value, read: &Read<'_>, rate: u32) -> Vec<f32> {
    let (values, outputs, tables) = read;
    let status = status(state);
    let flag = |e: &Option<CompileError>| if e.is_some() { 1.0 } else { 0.0 };
    let mut frame = Vec::with_capacity(FRAME_FLOATS);
    frame.extend(status.waves.iter().map(flag));
    frame.extend(status.outputs.iter().map(flag));
    let knobs = Knobs { a: core::array::from_fn(|i| values[P_A + i]), ..Knobs::default() };
    for k in 0..2 {
        frame.extend(preview(*outputs, tables, k, rate, values, &knobs));
    }
    frame
}

/// The live frame a state's window shows (what the plugin publishes on its display)
pub fn display_frame(state: &Value, rate: u32) -> Result<Vec<f32>> {
    Ok(make_frame(state, &read(state)?, rate))
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let read = read(state)?;
    let frame = make_frame(state, &read, cx.spec.sample_rate);
    let (values, outputs, tables) = read;
    let (patch, _errors) = Patch::new(outputs, tables);
    let room = patch.needs();
    let mut synth = Xpressive {
        rate: cx.spec.sample_rate,
        patch: Arc::new(patch),
        room,
        values,
        knobs: Knobs::default(),
        tempo: 120.0,
        frame: frame.into(),
    };
    synth.update();
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
