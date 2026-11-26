//! The legacy program's Xpressive (Orr Dvori's `ExprSynth`): a note is two expressions, O1 and O2, evaluated every
//! sample of the note, each with its own panning, over three wave tables W1, W2 and W3 that are themselves drawn or
//! written as expressions.
//!
//! The expressions are compiled by `lm-expr` (the exprtk of the legacy, safely), off the audio thread: [`Patch`] holds
//! what a note plays (the two compiled outputs and the three tables), [`wave_table`] renders a W expression and
//! smooths it ([`smooth`]) as the legacy's view did when its text or its smoothness knob changed, and [`ExprVoice`]
//! is one note (`ExprSynth`): its variables (`t`, `f`, `rel`, `trel`, `A1` to `A3`, ...), its `integrate` sums, its
//! `last` history, its noise, none of which allocates once the voice is made.
//!
//! What differs from the legacy, on purpose:
//! - a sample that is NaN or infinite is written as 0 (the legacy let it through);
//! - the noise of `rand`, `randv` and `seed` comes from the note's generator, not from the system's, so a render
//!   repeats;
//! - a W expression's noise is seeded from its text (the legacy seeded it from the clock at every edit, and the
//!   edit saved what it had drawn), so building the same state twice gives the same table;
//! - `bnote` and `tempo`, which the legacy took from the instrument track and the song, are variables the host sets.

use core::f32::consts::SQRT_2;

use lm_expr::xpressive::{self as lang, History, Integrate, Kind, Rand, Vars};
use lm_expr::{CompileError, Expr, Symbols, VarId};

/// The points of a wave table (the legacy's `GRAPH_LENGTH`)
pub const WAVE_LENGTH: usize = 4096;
/// The smoothness knob's largest value
pub const SMOOTH_MAX: f32 = 70.0;
/// The release transition's largest value, ms
pub const REL_TRANSITION_MAX: f32 = 500.0;

/// The variables of an output expression in the slice `eval` takes: the legacy's (see [`Vars`]) then `bnote` and
/// `tempo`
pub const OUTPUT_VARS: usize = Vars::COUNT + 2;

/// The language of an expression: its names, and where its variables are
#[derive(Clone, Debug)]
pub struct Language {
    pub symbols: Symbols,
    pub vars: Vars,
    pub bnote: VarId,
    pub tempo: VarId,
}

/// The functions a W expression has (the legacy registered `W1` to `W3` only for the outputs), in the order of
/// `lm_expr::xpressive`'s table so that its `kind` maps them: name, arguments, side effects
const WAVE_FUNCS: [(&str, u8, bool); 16] = [
    ("rand", 0, true),
    ("randv", 1, false),
    ("randsv", 2, false),
    ("integrate", 1, true),
    ("semitone", 1, false),
    ("cent", 1, false),
    ("last", 1, false),
    ("sinew", 1, false),
    ("squarew", 1, false),
    ("trianglew", 1, false),
    ("saww", 1, false),
    ("sawtoothw", 1, false),
    ("moogsaww", 1, false),
    ("moogw", 1, false),
    ("expw", 1, false),
    ("expnw", 1, false),
];

impl Language {
    /// An output expression's: `t f key rel trel A1 A2 A3 v seed srate bnote tempo` and every function
    pub fn output() -> Self {
        let (mut symbols, vars) = lang::symbols();
        let bnote = symbols.add_var("bnote").unwrap_or(VarId(u32::MAX));
        let tempo = symbols.add_var("tempo").unwrap_or(VarId(u32::MAX));
        Self { symbols, vars, bnote, tempo }
    }

    /// A W expression's: `t`, `seed` and `srate`, and the functions but `W1` to `W3`. (Its [`Vars`] has the three
    /// that exist; the others point nowhere)
    pub fn wave() -> Self {
        let mut symbols = Symbols::new();
        let none = VarId(u32::MAX);
        let t = symbols.add_var("t").unwrap_or(none);
        let seed = symbols.add_var("seed").unwrap_or(none);
        let srate = symbols.add_var("srate").unwrap_or(none);
        for (name, arity, side_effects) in WAVE_FUNCS {
            // The names are distinct and not reserved: registration cannot fail
            let _ = symbols.add_fn(name, arity, side_effects);
        }
        let vars =
            Vars { t, f: none, key: none, rel: none, trel: none, a1: none, a2: none, a3: none, v: none, seed, srate };
        Self { symbols, vars, bnote: none, tempo: none }
    }

    pub fn compile(&self, source: &str) -> Result<Expr, CompileError> {
        Expr::compile(source, &self.symbols)
    }
}

fn set(vars: &mut [f32], id: VarId, x: f32) {
    if let Some(slot) = vars.get_mut(id.index()) {
        *slot = x;
    }
}

/// FNV-1a of a text: the seed of a W expression's noise
fn hash_text(text: &str) -> u32 {
    let mut h = 0x811c_9dc5_u32;
    for b in text.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The legacy's `seed` constant: a random number that a float holds exactly (24 bits)
fn seed_value(r: u32) -> f32 {
    (r & ((1 << 24) - 1)) as f32
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// What the legacy's view did for a W expression: evaluates it at `t = i / length` for the `WAVE_LENGTH` points
/// (`integrate` and `last` over one second of `WAVE_LENGTH` samples, `srate` the length), a point that is NaN or
/// infinite is 0. `Err` when the text does not compile (the legacy then left the graph as it was)
pub fn render_wave_expression(source: &str) -> Result<Vec<f32>, CompileError> {
    let language = Language::wave();
    let expr = language.compile(source)?;
    let length = WAVE_LENGTH as f32;
    let seed = hash_text(source);
    let sites = expr.call_count(lang::fn_id(Kind::Integrate)) as usize;
    let mut integrate = Integrate::new(sites, length);
    let mut history = History::new(WAVE_LENGTH);
    let mut rand = Rand::new(seed);
    let mut stack = vec![0.0f32; expr.stack_len()];
    let mut vars = [0.0f32; 3];
    set(&mut vars, language.vars.seed, seed_value(seed));
    set(&mut vars, language.vars.srate, length);
    let mut out = Vec::with_capacity(WAVE_LENGTH);
    for i in 0..WAVE_LENGTH {
        set(&mut vars, language.vars.t, i as f32 / length);
        let y = {
            let mut host = lang::Voice {
                integrate: &mut integrate,
                history: &history,
                rand: &mut rand,
                random_seed: seed,
                waves: [&[], &[], &[]],
                interpolate: [false; 3],
            };
            expr.eval(&mut stack, &vars, &mut host)
        };
        history.push(y);
        out.push(if y.is_finite() { y } else { 0.0 });
    }
    Ok(out)
}

/// The legacy's `Xpressive::smooth`: the graph convolved cyclically with a gaussian of width `smoothness` (0 to
/// [`SMOOTH_MAX`]; 0 or less leaves it as it is)
pub fn smooth(raw: &[f32], smoothness: f32) -> Vec<f32> {
    let mut out = raw.to_vec();
    if !(smoothness > 0.0) || raw.is_empty() {
        return out;
    }
    let size = ((smoothness * 5.0) as i32 | 1) as usize;
    let center = size / 2;
    let delta = smoothness;
    // std::numbers::inv_sqrtpi_v<float> / sqrt2
    let inv_sqrt2pi = 0.564_189_6_f32 / SQRT_2;
    let a = inv_sqrt2pi / delta;
    let mut gauss = vec![0.0f32; size];
    let mut sum = 0.0f32;
    for (i, g) in gauss.iter_mut().enumerate() {
        let temp = (i as f32 - center as f32) / delta;
        *g = a * libm::expf(-0.5 * temp * temp);
        sum += *g;
    }
    for g in &mut gauss {
        *g /= sum;
    }
    // `graphModel::convolve`: a cyclic convolution, the result written `center` points on
    let length = raw.len();
    for i in 0..length {
        let mut acc = 0.0f32;
        for (j, g) in gauss.iter().enumerate() {
            acc += g * raw.get((i + j) % length).copied().unwrap_or(0.0);
        }
        if let Some(slot) = out.get_mut((i + center) % length) {
            *slot = acc;
        }
    }
    out
}

/// One W table as the instrument plays it: the expression's points when `source` compiles and is not empty, else the
/// drawn ones (`drawn`, padded with zeros or cut to [`WAVE_LENGTH`]), smoothed. The error is the expression's, when
/// it did not compile
pub fn wave_table(source: &str, drawn: &[f32], smoothness: f32) -> (Vec<f32>, Option<CompileError>) {
    let mut error = None;
    let raw = if source.is_empty() {
        None
    } else {
        match render_wave_expression(source) {
            Ok(points) => Some(points),
            Err(e) => {
                error = Some(e);
                None
            }
        }
    };
    let raw = raw.unwrap_or_else(|| {
        let mut points = vec![0.0f32; WAVE_LENGTH];
        for (p, d) in points.iter_mut().zip(drawn) {
            *p = if d.is_finite() { *d } else { 0.0 };
        }
        points
    });
    (smooth(&raw, smoothness), error)
}

/// What a voice needs room for: per output, the stack and the `integrate` sites. A voice made for one patch plays
/// another that [`Needs::fit_in`] it
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    pub stack: [usize; 2],
    pub sites: [usize; 2],
}

impl Needs {
    /// Whether a voice with room for `room` plays these
    pub fn fit_in(&self, room: &Needs) -> bool {
        self.stack.iter().zip(&room.stack).all(|(a, b)| a <= b)
            && self.sites.iter().zip(&room.sites).all(|(a, b)| a <= b)
    }
}

/// What a note plays: the two compiled outputs (`None` for one that is empty or does not compile: the other plays
/// alone) and the three wave tables, made away from the audio thread
#[derive(Clone, Debug)]
pub struct Patch {
    outputs: [Option<Expr>; 2],
    waves: [Vec<f32>; 3],
    vars: Vars,
    bnote: VarId,
    tempo: VarId,
}

impl Patch {
    /// Compiles `outputs` (O1, O2) and takes the three tables (each [`WAVE_LENGTH`] points: a table of another size
    /// is padded or cut). The errors are each output's
    pub fn new(outputs: [&str; 2], waves: [Vec<f32>; 3]) -> (Self, [Option<CompileError>; 2]) {
        let language = Language::output();
        let mut errors = [None, None];
        let mut compiled = [None, None];
        for ((src, slot), error) in outputs.iter().zip(&mut compiled).zip(&mut errors) {
            match language.compile(src) {
                Ok(e) => *slot = Some(e),
                Err(e) => *error = Some(e),
            }
        }
        let waves = waves.map(|mut w| {
            w.resize(WAVE_LENGTH, 0.0);
            w
        });
        (Self { outputs: compiled, waves, vars: language.vars, bnote: language.bnote, tempo: language.tempo }, errors)
    }

    /// Whether output `k` (0 or 1) compiled
    pub fn valid(&self, k: usize) -> bool {
        self.outputs.get(k).is_some_and(Option::is_some)
    }

    pub fn needs(&self) -> Needs {
        let integrate = lang::fn_id(Kind::Integrate);
        let mut n = Needs::default();
        for (k, o) in self.outputs.iter().enumerate() {
            if let Some(e) = o {
                if let Some(s) = n.stack.get_mut(k) {
                    *s = e.stack_len();
                }
                if let Some(s) = n.sites.get_mut(k) {
                    *s = e.call_count(integrate) as usize;
                }
            }
        }
        n
    }

    pub fn wave(&self, i: usize) -> &[f32] {
        self.waves.get(i).map_or(&[], Vec::as_slice)
    }
}

/// The knobs that act on a sounding note
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knobs {
    /// A1, A2, A3, -1 to 1
    pub a: [f32; 3],
    /// O1's and O2's panning, -1 to 1
    pub pan: [f32; 2],
}

impl Default for Knobs {
    fn default() -> Self {
        Self { a: [1.0; 3], pan: [1.0, -1.0] }
    }
}

/// What a note starts with
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteInit {
    /// The key pressed (`key`)
    pub key: f32,
    /// The base note (`bnote`)
    pub base_note: f32,
    /// The note's volume (`v`: the legacy's volume over 255)
    pub volume: f32,
    /// The song's tempo (`tempo`), BPM
    pub tempo: f32,
    /// Hz at the start (`f`)
    pub freq: f32,
    /// The release transition, ms: how long `rel` takes from 0 to 1 once the note is let go
    pub release_ms: f32,
    /// Whether W1, W2 and W3 are read with interpolation
    pub interpolate: [bool; 3],
    /// The note's own noise (`rand`, `randv`, `seed`)
    pub random: u64,
}

/// One output's state in a note
#[derive(Clone, Debug)]
struct Channel {
    integrate: Integrate,
    history: History,
    stack: Vec<f32>,
    vars: [f32; OUTPUT_VARS],
    random_seed: u32,
}

impl Channel {
    fn new(room: usize, sites: usize, rate: u32) -> Self {
        Self {
            integrate: Integrate::new(sites, rate as f32),
            history: History::new(rate as usize),
            stack: vec![0.0; room.max(1)],
            vars: [0.0; OUTPUT_VARS],
            random_seed: 0,
        }
    }

    fn eval(&mut self, expr: &Expr, rand: &mut Rand, patch: &Patch, interpolate: [bool; 3]) -> f32 {
        let mut host = lang::Voice {
            integrate: &mut self.integrate,
            history: &self.history,
            rand,
            random_seed: self.random_seed,
            waves: [patch.wave(0), patch.wave(1), patch.wave(2)],
            interpolate,
        };
        let y = expr.eval(&mut self.stack, &self.vars, &mut host);
        self.history.push(y);
        y
    }
}

/// One note (`ExprSynth`)
#[derive(Clone, Debug)]
pub struct ExprVoice {
    channels: [Channel; 2],
    rand: Rand,
    rate: u32,
    interpolate: [bool; 3],
    note_sample: u32,
    note_rel_sample: u32,
    note_sample_sec: f32,
    note_rel_sec: f32,
    frequency: f32,
    released: f32,
    rel_inc: f32,
    is_released: bool,
}

impl ExprVoice {
    /// A voice with room for `room` (made in the build: allocates, for the pool)
    pub fn new(room: &Needs, rate: u32) -> Self {
        let channel = |k: usize| {
            Channel::new(room.stack.get(k).copied().unwrap_or(1), room.sites.get(k).copied().unwrap_or(0), rate)
        };
        Self {
            channels: [channel(0), channel(1)],
            rand: Rand::new(1),
            rate,
            interpolate: [false; 3],
            note_sample: 0,
            note_rel_sample: 0,
            note_sample_sec: 0.0,
            note_rel_sec: 0.0,
            frequency: 440.0,
            released: 0.0,
            rel_inc: 0.0,
            is_released: false,
        }
    }

    /// Starts a note (never allocates; `patch` must [`Needs::fit_in`] the room the voice was made with, else the
    /// outputs that do not fit play nothing)
    pub fn start(&mut self, patch: &Patch, note: &NoteInit) {
        let mut state = note.random;
        self.rand = Rand::new(splitmix(&mut state) as u32);
        self.interpolate = note.interpolate;
        self.note_sample = 0;
        self.note_rel_sample = 0;
        self.note_sample_sec = 0.0;
        self.note_rel_sec = 0.0;
        self.frequency = note.freq;
        self.released = 0.0;
        self.is_released = false;
        // rel_transition in ms: the increment of each frame
        self.rel_inc = 1000.0 / (self.rate as f32 * note.release_ms);
        let rate = self.rate as f32;
        for ch in &mut self.channels {
            ch.integrate.reset();
            ch.history.clear();
            ch.random_seed = splitmix(&mut state) as u32;
            let seed = seed_value(splitmix(&mut state) as u32);
            let v = &mut ch.vars;
            v.fill(0.0);
            set(v, patch.vars.key, note.key);
            set(v, patch.bnote, note.base_note);
            set(v, patch.vars.v, note.volume);
            set(v, patch.tempo, note.tempo);
            set(v, patch.vars.srate, rate);
            set(v, patch.vars.seed, seed);
            set(v, patch.vars.f, note.freq);
        }
    }

    /// The note is let go: `rel` and `trel` start to move from the next sample
    pub fn release(&mut self) {
        self.is_released = true;
    }

    /// The tempo changed under a sounding note
    pub fn set_tempo(&mut self, patch: &Patch, tempo: f32) {
        for ch in &mut self.channels {
            set(&mut ch.vars, patch.tempo, tempo);
        }
    }

    /// Writes the next `left.len()` samples of the note, at `freq` Hz (the pitch glides to it over the piece)
    pub fn render(&mut self, patch: &Patch, knobs: &Knobs, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        // An output plays when it compiled and the voice has the stack for it
        let needs = patch.needs();
        let fits = |k: usize| {
            patch.valid(k)
                && match (needs.stack.get(k), self.channels.get(k)) {
                    (Some(need), Some(c)) => *need <= c.stack.len(),
                    _ => false,
                }
        };
        let (v1, v2) = (fits(0), fits(1));
        if frames == 0 {
            return;
        }
        left.iter_mut().for_each(|s| *s = 0.0);
        right.iter_mut().for_each(|s| *s = 0.0);
        if !v1 && !v2 {
            return;
        }
        let pn = [knobs.pan[0] * 0.5, knobs.pan[1] * 0.5];
        let freq_inc = (freq - self.frequency) / frames as f32;
        if self.is_released && self.note_rel_sample == 0 {
            self.note_rel_sample = self.note_sample;
        }
        // The outputs that play, with their panning
        let mut playing: [Option<(usize, f32)>; 2] = [None, None];
        if v1 {
            playing[0] = Some((0, pn[0]));
        }
        if v2 {
            playing[1] = Some((1, pn[1]));
        }
        let rate = self.rate as f32;
        for frame in 0..frames {
            if self.is_released && self.released < 1.0 {
                self.released = (self.released + self.rel_inc).min(1.0);
            }
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for (k, slot) in playing.iter().enumerate() {
                let Some((_, pan)) = slot else { continue };
                let Some(expr) = patch.outputs.get(k).and_then(Option::as_ref) else { continue };
                let Some(ch) = self.channels.get_mut(k) else { continue };
                set(&mut ch.vars, patch.vars.t, self.note_sample_sec);
                set(&mut ch.vars, patch.vars.f, self.frequency);
                set(&mut ch.vars, patch.vars.rel, self.released);
                set(&mut ch.vars, patch.vars.trel, self.note_rel_sec);
                set(&mut ch.vars, patch.vars.a1, knobs.a[0]);
                set(&mut ch.vars, patch.vars.a2, knobs.a[1]);
                set(&mut ch.vars, patch.vars.a3, knobs.a[2]);
                let o = ch.eval(expr, &mut self.rand, patch, self.interpolate);
                l += (-pan + 0.5) * o;
                r += (pan + 0.5) * o;
            }
            if let Some(s) = left.get_mut(frame) {
                *s = if l.is_finite() { l } else { 0.0 };
            }
            if let Some(s) = right.get_mut(frame) {
                *s = if r.is_finite() { r } else { 0.0 };
            }
            self.note_sample = self.note_sample.wrapping_add(1);
            self.note_sample_sec = self.note_sample as f32 / rate;
            if self.is_released {
                self.note_rel_sec = self.note_sample.wrapping_sub(self.note_rel_sample) as f32 / rate;
            }
            self.frequency += freq_inc;
        }
        self.frequency = freq;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn patch(o1: &str, o2: &str) -> (Patch, [Option<CompileError>; 2]) {
        let w = || vec![0.0; WAVE_LENGTH];
        Patch::new([o1, o2], [w(), w(), w()])
    }

    fn note(freq: f32) -> NoteInit {
        NoteInit {
            key: 69.0,
            base_note: 69.0,
            volume: 0.4,
            tempo: 120.0,
            freq,
            release_ms: 50.0,
            interpolate: [false; 3],
            random: 7,
        }
    }

    fn play(p: &Patch, knobs: &Knobs, n: &NoteInit, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut v = ExprVoice::new(&p.needs(), RATE);
        v.start(p, n);
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        v.render(p, knobs, n.freq, &mut l, &mut r);
        (l, r)
    }

    #[test]
    fn a_constant_output_is_panned_as_the_legacy() {
        // Full right: left gets (-0.5+0.5), right (0.5+0.5)
        let (p, e) = patch("0.5", "");
        assert!(e[0].is_none() && e[1].is_some());
        let (l, r) = play(&p, &Knobs { a: [1.0; 3], pan: [1.0, -1.0] }, &note(440.0), 16);
        assert!(l.iter().all(|s| *s == 0.0));
        assert!(r.iter().all(|s| *s == 0.5));
        // Centre: half each side
        let (l, r) = play(&p, &Knobs { a: [1.0; 3], pan: [0.0, 0.0] }, &note(440.0), 16);
        assert!(l.iter().all(|s| *s == 0.25) && r.iter().all(|s| *s == 0.25));
    }

    #[test]
    fn two_outputs_sum_with_their_pans() {
        let (p, e) = patch("0.5", "0.25");
        assert!(e.iter().all(Option::is_none));
        let (l, r) = play(&p, &Knobs { a: [1.0; 3], pan: [1.0, -1.0] }, &note(440.0), 4);
        // O1 right, O2 left
        assert!(l.iter().all(|s| *s == 0.25));
        assert!(r.iter().all(|s| *s == 0.5));
    }

    #[test]
    fn neither_valid_is_silence() {
        let (p, e) = patch("", "sinew(");
        assert!(e.iter().all(Option::is_some));
        let (l, r) = play(&p, &Knobs::default(), &note(440.0), 8);
        assert!(l.iter().chain(&r).all(|s| *s == 0.0));
    }

    #[test]
    fn time_and_frequency_variables_move_with_the_note() {
        let (p, _) = patch("t * 48000", "");
        let (_, r) = play(&p, &Knobs { a: [1.0; 3], pan: [0.0, 0.0] }, &note(440.0), 4);
        // pan 0 gives 0.5 of the output on each side; t = sample / rate
        for (i, s) in r.iter().enumerate() {
            assert!((s - 0.5 * i as f32).abs() < 1e-3, "{i}: {s}");
        }
        let (p, _) = patch("f", "");
        let (_, r) = play(&p, &Knobs { a: [1.0; 3], pan: [0.0, 0.0] }, &note(440.0), 4);
        assert!(r.iter().all(|s| (*s - 220.0).abs() < 1e-3));
    }

    #[test]
    fn the_note_constants_reach_the_expression() {
        let (p, _) = patch("key + bnote * 1000 + v * 1e6 + tempo * 1e9", "");
        let mut n = note(440.0);
        n.key = 60.0;
        n.base_note = 57.0;
        n.volume = 0.5;
        n.tempo = 100.0;
        let (_, r) = play(&p, &Knobs { a: [1.0; 3], pan: [0.0, 0.0] }, &n, 1);
        let want = 0.5 * (60.0 + 57_000.0 + 500_000.0 + 1.0e11);
        assert!((r[0] - want).abs() < want * 1e-5, "{} vs {want}", r[0]);
        let (p, _) = patch("srate + A1 + 10 * A2 + 100 * A3", "");
        let (_, r) = play(&p, &Knobs { a: [1.0, 2.0, 3.0], pan: [0.0, 0.0] }, &note(440.0), 1);
        assert_eq!(r[0], 0.5 * (48_000.0 + 1.0 + 20.0 + 300.0));
    }

    #[test]
    fn release_ramps_rel_and_counts_trel() {
        let (p, _) = patch("rel * 100 + trel * 1000", "");
        let n = note(440.0);
        let mut v = ExprVoice::new(&p.needs(), RATE);
        v.start(&p, &n);
        let k = Knobs { a: [1.0; 3], pan: [0.0, 0.0] };
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        v.render(&p, &k, n.freq, &mut l, &mut r);
        assert!(r.iter().all(|s| *s == 0.0));
        v.release();
        v.render(&p, &k, n.freq, &mut l, &mut r);
        // rel climbs by 1000 / (rate * 50 ms) a frame; trel from the first released frame
        assert!(r[63] > r[1] && r[1] > 0.0);
        let rel_inc = 1000.0 / (RATE as f32 * 50.0);
        assert!((r[0] - 0.5 * (100.0 * rel_inc)).abs() < 1e-4);
        // The transition caps at 1: the sound stops rising
        let mut v = ExprVoice::new(&p.needs(), RATE);
        let mut quick = n;
        quick.release_ms = 0.0;
        v.start(&p, &quick);
        v.release();
        v.render(&p, &k, n.freq, &mut l, &mut r);
        assert!((r[0] - 50.0).abs() < 0.1, "rel jumps to 1 without a transition: {}", r[0]);
    }

    #[test]
    fn integrate_follows_the_frequency() {
        let (p, _) = patch("integrate(f)", "");
        let (_, r) = play(&p, &Knobs { a: [1.0; 3], pan: [0.0, 0.0] }, &note(480.0), 3);
        // The value before this sample's addition, over the rate: 0, 480/48000, 960/48000
        assert_eq!(r[0], 0.0);
        assert!((r[1] - 0.5 * 0.01).abs() < 1e-6 && (r[2] - 0.5 * 0.02).abs() < 1e-6);
    }

    #[test]
    fn a_sine_output_has_the_pitch() {
        let (p, _) = patch("sinew(t * f)", "");
        let (_, r) = play(&p, &Knobs { a: [1.0; 3], pan: [0.0, 0.0] }, &note(480.0), 480);
        // 480 Hz at 48 kHz: 100 samples a cycle, so five cycles end near zero crossing
        assert!(r[25] > 0.49 && r[75] < -0.49);
    }

    #[test]
    fn non_finite_output_is_silent() {
        let (p, _) = patch("log(0 - 1)", "");
        let (l, r) = play(&p, &Knobs::default(), &note(440.0), 8);
        assert!(l.iter().chain(&r).all(|s| *s == 0.0));
    }

    #[test]
    fn a_note_is_reproducible_and_notes_differ_by_their_noise() {
        let (p, _) = patch("rand() + randv(3) + seed / 1e7", "");
        let k = Knobs { a: [1.0; 3], pan: [0.0, 0.0] };
        let a = play(&p, &k, &note(440.0), 32).1;
        let b = play(&p, &k, &note(440.0), 32).1;
        assert_eq!(a, b);
        let mut other = note(440.0);
        other.random = 99;
        assert_ne!(a, play(&p, &k, &other, 32).1);
    }

    #[test]
    fn a_restart_forgets_the_last_note() {
        let (p, _) = patch("integrate(1) + last(1)", "");
        let n = note(440.0);
        let k = Knobs { a: [1.0; 3], pan: [0.0, 0.0] };
        let mut v = ExprVoice::new(&p.needs(), RATE);
        v.start(&p, &n);
        let (mut l, mut r) = (vec![0.0; 100], vec![0.0; 100]);
        v.render(&p, &k, n.freq, &mut l, &mut r);
        let first = r.clone();
        v.release();
        v.start(&p, &n);
        v.render(&p, &k, n.freq, &mut l, &mut r);
        assert_eq!(first, r);
    }

    #[test]
    fn the_pitch_glides_over_the_piece() {
        let (p, _) = patch("f", "");
        let n = note(100.0);
        let mut v = ExprVoice::new(&p.needs(), RATE);
        v.start(&p, &n);
        let (mut l, mut r) = (vec![0.0; 4], vec![0.0; 4]);
        v.render(&p, &Knobs { a: [1.0; 3], pan: [0.0, 0.0] }, 200.0, &mut l, &mut r);
        // 100, 125, 150, 175 (each half on a side)
        for (i, s) in r.iter().enumerate() {
            assert!((s - 0.5 * (100.0 + 25.0 * i as f32)).abs() < 1e-3);
        }
        v.render(&p, &Knobs { a: [1.0; 3], pan: [0.0, 0.0] }, 200.0, &mut l, &mut r);
        assert!((r[0] - 100.0).abs() < 1e-3);
    }

    #[test]
    fn waves_are_read_with_and_without_interpolation() {
        let mut w1 = vec![0.0; WAVE_LENGTH];
        for (i, s) in w1.iter_mut().enumerate() {
            *s = i as f32 / WAVE_LENGTH as f32;
        }
        let (p, _) = Patch::new(["W1(0.25 + 0.00006)", ""], [w1, vec![0.0; WAVE_LENGTH], vec![0.0; WAVE_LENGTH]]);
        let k = Knobs { a: [1.0; 3], pan: [0.0, 0.0] };
        let mut n = note(440.0);
        let flat = play(&p, &k, &n, 1).1[0];
        n.interpolate = [true, false, false];
        let smooth = play(&p, &k, &n, 1).1[0];
        // 0.25006 lands a quarter of the way into point 1024: 1024/4096 without, a little more with
        assert!((flat - 0.5 * 0.25).abs() < 1e-6);
        assert!(smooth > flat && smooth < 0.5 * (1025.0 / 4096.0));
    }

    #[test]
    fn wave_expressions_render_over_one_period() {
        let w = render_wave_expression("sinew(t)").unwrap();
        assert_eq!(w.len(), WAVE_LENGTH);
        assert_eq!(w[0], 0.0);
        assert!((w[WAVE_LENGTH / 4] - 1.0).abs() < 1e-6);
        // Integrate counts over the table's length as its rate: the sum of 1 per point, over WAVE_LENGTH
        let w = render_wave_expression("integrate(1)").unwrap();
        assert_eq!(w[0], 0.0);
        assert!((w[WAVE_LENGTH - 1] - (WAVE_LENGTH - 1) as f32 / WAVE_LENGTH as f32).abs() < 1e-5);
        // No f or W1 in a wave expression, as the legacy
        assert!(render_wave_expression("f").is_err());
        assert!(render_wave_expression("W1(t)").is_err());
        assert!(render_wave_expression("srate").is_ok());
        // Not a number is 0
        assert!(render_wave_expression("log(0 - t - 1)").unwrap().iter().all(|s| *s == 0.0));
        // Noise repeats for the same text
        assert_eq!(render_wave_expression("rand()").unwrap(), render_wave_expression("rand()").unwrap());
    }

    #[test]
    fn wave_tables_fall_back_to_the_drawn_points() {
        let drawn: Vec<f32> = (0..WAVE_LENGTH).map(|i| if i < 2048 { 1.0 } else { -1.0 }).collect();
        let (t, e) = wave_table("", &drawn, 0.0);
        assert!(e.is_none());
        assert_eq!(t, drawn);
        let (t, e) = wave_table("sinew(", &drawn, 0.0);
        assert!(e.is_some());
        assert_eq!(t, drawn);
        let (t, e) = wave_table("0.5", &drawn, 0.0);
        assert!(e.is_none() && t.iter().all(|s| *s == 0.5));
        // Short drawn data is padded with silence
        let (t, _) = wave_table("", &[1.0, 1.0], 0.0);
        assert_eq!(t.len(), WAVE_LENGTH);
        assert_eq!(t[1], 1.0);
        assert_eq!(t[2], 0.0);
    }

    #[test]
    fn smoothing_is_a_cyclic_gaussian_that_keeps_the_mean() {
        let mut raw = vec![0.0f32; WAVE_LENGTH];
        raw[100] = 1.0;
        assert_eq!(smooth(&raw, 0.0), raw);
        let s = smooth(&raw, 4.0);
        // The impulse spreads, its sum stays 1, and it is centred where the impulse was
        let sum: f32 = s.iter().sum();
        assert!((sum - 1.0).abs() < 1e-4);
        let peak = s.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).unwrap();
        assert_eq!(peak, 100);
        assert!(s[100] < 0.2 && s[100] > s[104]);
        // Wraps round the end
        let mut raw = vec![0.0f32; WAVE_LENGTH];
        raw[0] = 1.0;
        assert!(smooth(&raw, 4.0)[WAVE_LENGTH - 1] > 0.0);
    }

    #[test]
    fn needs_fit_in_a_larger_room() {
        let (small, _) = patch("t", "");
        let (big, _) = patch("integrate(t) + integrate(t * 2) + (1 + (2 + (3 + t)))", "integrate(1)");
        assert!(small.needs().fit_in(&big.needs()));
        assert!(!big.needs().fit_in(&small.needs()));
        // A voice with the small room plays a big patch's outputs? No: they stay silent, never panic
        let mut v = ExprVoice::new(&small.needs(), RATE);
        v.start(&big, &note(440.0));
        let (mut l, mut r) = (vec![0.0; 8], vec![0.0; 8]);
        v.render(&big, &Knobs::default(), 440.0, &mut l, &mut r);
        assert!(l.iter().chain(&r).all(|s| s.is_finite()));
    }

    #[test]
    fn the_wave_language_registers_the_outputs_functions_in_the_same_order() {
        let wave = Language::wave();
        let out = Language::output();
        for (name, ..) in WAVE_FUNCS {
            assert_eq!(wave.symbols.function(name), out.symbols.function(name), "{name}");
        }
        assert_eq!(wave.symbols.function("w1"), None);
        assert_eq!(out.symbols.var_count(), OUTPUT_VARS);
    }
}
