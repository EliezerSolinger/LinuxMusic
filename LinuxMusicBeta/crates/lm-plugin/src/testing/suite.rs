//! The tests every plugin passes (arquitetura-nova.md, 7a; interfaces.md, Parte 6: U26, U28, U29, U31, U32, U35,
//! U36; V3), made from its descriptor alone. A plugin crate runs them all with [`crate::contract_tests!`].

use std::path::Path;
use std::sync::Arc;

use lm_render::{Adopted, EventKind, ProcessSpec, Processor, Rng};
use lm_state::{FieldKind, PARAMS_KEY, Scale, Value, from_json, to_json};

use crate::descriptor::{BuildContext, Descriptor, PluginKind, Velocity};
use crate::registry::{Registry, check_ports};
use crate::testing::bench::{Bench, Timed};
use crate::testing::material::*;

/// The rates of U36
pub const RATES: [u32; 4] = [44_100, 48_000, 96_000, 192_000];
/// The block sizes of V3
pub const BLOCKS: [usize; 4] = [32, 64, 256, 1024];
const MAX_BLOCK: usize = 1024;
const SEED: u64 = 0x5eed;

/// A plugin under test
struct Plugin {
    d: &'static Descriptor,
    registry: Arc<Registry>,
    type_name: String,
}

fn spec(rate: u32) -> ProcessSpec {
    ProcessSpec { sample_rate: rate, max_block: MAX_BLOCK }
}

impl Plugin {
    fn new(d: &'static Descriptor) -> Self {
        let registry = Registry::new(&[d]).unwrap_or_else(|e| panic!("{}: the descriptor is refused: {e}", d.id));
        Self { d, registry, type_name: d.type_name() }
    }

    fn defaults(&self) -> Value {
        self.registry.defaults(self.d.id).unwrap_or_else(|e| panic!("defaults: {e}"))
    }

    fn complete(&self, state: &Value) -> Value {
        self.registry.complete(self.d.id, state).unwrap_or_else(|e| panic!("complete: {e}"))
    }

    fn channels(&self, state: &Value) -> usize {
        match self.d.kind {
            PluginKind::Effect => 2,
            PluginKind::Instrument => {
                let ports = self.registry.ports(self.d.id, state).unwrap_or_else(|| panic!("no ports for {state:?}"));
                ports.outputs.iter().map(|p| p.channels as usize).sum()
            }
        }
    }

    fn build(&self, state: &Value, rate: u32) -> Box<dyn Processor> {
        let complete = self.complete(state);
        let resources = test_resources(self.registry.schemas(), &self.type_name, &complete, rate);
        let cx = BuildContext { spec: spec(rate), resources: &resources, channels: 2 };
        self.registry
            .build(self.d.id, state, &cx)
            .unwrap_or_else(|e| panic!("{}: the build of {} fails: {e}", self.d.id, to_json(state).unwrap_or_default()))
    }

    fn bench(&self, state: &Value, rate: u32) -> Bench {
        let p = self.build(state, rate);
        match self.d.kind {
            PluginKind::Instrument => Bench::instrument(p, spec(rate), self.channels(state), SEED),
            PluginKind::Effect => Bench::effect(p, spec(rate), 2, SEED, test_input),
        }
    }

    fn voices(&self) -> usize {
        self.d.tests.voices.max(1)
    }
}

/// A state in JSON, for a message
fn short(state: &Value) -> String {
    to_json(state).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every sample finite and within the plugin's largest legitimate level (TestHints::max_level): above it is a blowup
fn finite(chans: &[Vec<f32>], limit: f32, what: &str) {
    for (c, ch) in chans.iter().enumerate() {
        if let Some(i) = ch.iter().position(|x| !x.is_finite() || x.abs() > limit) {
            panic!("{what}: channel {c}, sample {i} is {}", ch[i]);
        }
    }
}

fn same(a: &[Vec<f32>], b: &[Vec<f32>], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: channels");
    for (c, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!(x.len(), y.len(), "{what}: length");
        if let Some(i) = x.iter().zip(y).position(|(p, q)| p.to_bits() != q.to_bits()) {
            panic!("{what}: channel {c} differs first at sample {i}: {} and {}", x[i], y[i]);
        }
    }
}

/// A script of notes for `seconds`: a chord of more notes than voices, single notes coming and going, and the end
/// of every note before the last half second
fn script(p: &Plugin, rate: u32, seconds: f64) -> Vec<Timed> {
    let total = (seconds * f64::from(rate)) as u64;
    let mut v = chord(0, 1, p.voices() + 4, 37, Some(total / 2));
    let mut id = 1000;
    let mut t = rate as u64 / 10;
    while t < total.saturating_sub(rate as u64 / 2) {
        let key = 40 + (id % 40) as u8;
        v.push(note_on(t, id, key, 0.8));
        v.push(note_off(t + rate as u64 / 7, id, key));
        id += 1;
        t += rate as u64 / 11 + 13;
    }
    v
}

/// Parameter changes at odd times through a script (each parameter, a random value)
fn param_changes(p: &Plugin, rate: u32, seconds: f64, rng: &mut Rng) -> Vec<Timed> {
    let schema = p.d.schema();
    let total = (seconds * f64::from(rate)) as u64;
    let mut v = Vec::new();
    for (i, spec) in schema.params.iter().enumerate() {
        if !spec.automatable {
            continue;
        }
        let at = rng.below(total.max(1));
        v.push(Timed::new(at, EventKind::Param { index: i as u32, value: random_param(spec, rng) as f32 }));
    }
    v
}

// ---------------------------------------------------------------------------------------------------------------
// 1, 2: the state round trip

/// Defaults → build → save → open: the tree is the same, bit for bit (7a, test 1)
pub fn defaults_round_trip(d: &'static Descriptor) {
    let p = Plugin::new(d);
    let defaults = p.defaults();
    p.registry.schemas().validate(&p.type_name, &defaults).unwrap_or_else(|e| panic!("defaults invalid: {e}"));
    drop(p.build(&defaults, 48_000));
    let json = to_json(&defaults).unwrap_or_else(|e| panic!("save: {e}"));
    let back = from_json(&json).unwrap_or_else(|e| panic!("open: {e}"));
    assert_eq!(back, defaults, "the defaults after save and open");
    assert_eq!(to_json(&back).ok().as_deref(), Some(json.as_str()), "saving again gives the same bytes");
    let version = p.registry.version(d.id).unwrap_or(1);
    let loaded = p.registry.load(d.id, version, back).unwrap_or_else(|e| panic!("load: {e}"));
    assert_eq!(loaded, defaults, "loading at the current version changes nothing");
}

/// Random valid trees: valid, built, saved and opened bit for bit, with valid ports (7a, test 2)
pub fn random_states_round_trip(d: &'static Descriptor) {
    let p = Plugin::new(d);
    let mut rng = Rng::new(SEED);
    for n in 0..200 {
        let state = random_state(p.registry.schemas(), &p.type_name, &mut rng);
        let complete = p.complete(&state);
        p.registry
            .schemas()
            .validate(&p.type_name, &complete)
            .unwrap_or_else(|e| panic!("random state {n} invalid: {e}: {}", to_json(&state).unwrap_or_default()));
        let json = to_json(&state).unwrap_or_else(|e| panic!("save: {e}"));
        let back = from_json(&json).unwrap_or_else(|e| panic!("open: {e}"));
        assert_eq!(back, state, "state {n} after save and open");
        assert_eq!(to_json(&back).ok().as_deref(), Some(json.as_str()), "state {n}: the same bytes");
        let ports = p.registry.ports(d.id, &state).unwrap_or_else(|| panic!("state {n}: no ports"));
        check_ports(d.id, d.kind, &ports).unwrap_or_else(|e| panic!("state {n}: {e}"));
        if n % 10 == 0 {
            drop(p.build(&state, 48_000));
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 3: no hidden state

/// The same state built twice and played the same notes gives the same audio, bit for bit (7a, test 3)
pub fn no_hidden_state(d: &'static Descriptor) {
    let p = Plugin::new(d);
    let mut rng = Rng::new(SEED + 3);
    let mut states = vec![p.defaults()];
    for _ in 0..4 {
        states.push(random_state(p.registry.schemas(), &p.type_name, &mut rng));
    }
    for (n, state) in states.iter().enumerate() {
        let mut events = script(&p, 48_000, 1.5);
        events.extend(param_changes(&p, 48_000, 1.5, &mut rng));
        let a = p.bench(state, 48_000).render(72_000, 256, &events, false);
        let b = p.bench(state, 48_000).render(72_000, 256, &events, false);
        finite(&a, p.d.tests.max_level, "a random state");
        same(&a, &b, &format!("state {n} built twice"));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 4: adopt

/// Another value for a field that is not a parameter
fn other_value(kind: &FieldKind, current: Option<&Value>) -> Option<Value> {
    match kind {
        FieldKind::Bool => Some(Value::bool(!current.and_then(Value::as_bool).unwrap_or(false))),
        FieldKind::Choice(keys) => {
            let now = current.and_then(Value::as_text).unwrap_or("");
            let i = keys.iter().position(|k| *k == now).unwrap_or(0);
            (keys.len() > 1).then(|| Value::text(keys[(i + 1) % keys.len()]))
        }
        FieldKind::Int { min, max } => {
            let now = current.and_then(Value::as_int).unwrap_or(*min);
            (max > min).then(|| Value::int(if now < *max { now + 1 } else { now - 1 }))
        }
        FieldKind::Real { min, max } => {
            let now = current.and_then(Value::as_real).unwrap_or(*min);
            let mid = (min + max) / 2.0;
            Value::real(if now == mid { *min } else { mid }).ok()
        }
        _ => None,
    }
}

/// A change of a field that is not a parameter, while notes sound: the new processor is built apart, then adopts the
/// old one with no allocation on the audio thread, and the sound goes on without a click; the same tree rebuilt and
/// adopted goes on bit for bit as if nothing happened (7a, test 4; U32)
pub fn adopt_while_playing(d: &'static Descriptor) {
    let p = Plugin::new(d);
    let rate = 48_000;
    let before = 24_000usize;
    let after = 24_000usize;
    let events = chord(0, 1, p.voices().min(6), 101, None);

    // The same tree: seamless
    let defaults = p.defaults();
    let whole = p.bench(&defaults, rate).render(before + after, 256, &events, false);
    let mut bench = p.bench(&defaults, rate);
    let first = bench.render(before, 256, &events, true);
    let new = p.build(&defaults, rate);
    let (adopted, old) = bench.replace(new, true);
    assert_eq!(adopted, Adopted::Yes, "the same tree must be adopted");
    drop(old);
    let second = bench.render(after, 256, &events, true);
    let joined: Vec<Vec<f32>> = first.iter().zip(&second).map(|(a, b)| [a.as_slice(), b].concat()).collect();
    same(&whole, &joined, "the same tree rebuilt and adopted");

    // Every field that is not a parameter
    let schema = d.schema();
    for field in &schema.fields {
        let Some(value) = other_value(&field.kind, defaults.get(field.key)) else { continue };
        let changed = defaults.with(field.key, value.clone());
        if p.registry.schemas().validate(&p.type_name, &changed).is_err() {
            continue;
        }
        let mut bench = p.bench(&defaults, rate);
        let a = bench.render(before, 256, &events, true);
        let new = p.build(&changed, rate);
        let (adopted, old) = bench.replace(new, true);
        drop(old);
        let expected = if d.tests.no_adopt.contains(&field.key) { Adopted::No } else { Adopted::Yes };
        assert_eq!(adopted, expected, "{}: adopting a change of {}", d.id, field.key);
        if adopted == Adopted::No {
            continue;
        }
        let b = bench.render(after, 256, &events, true);
        finite(&b, p.d.tests.max_level, field.key);
        if bench.outputs() != a.len() {
            continue;
        }
        let x = mono(&a.iter().zip(&b).map(|(a, b)| [a.as_slice(), b].concat()).collect::<Vec<_>>());
        let s = before;
        let old_step = largest_step(&x, s - 4000..s);
        let new_step = largest_step(&x, s + 480..s + 4480);
        let boundary = largest_step(&x, s - 8..s + 8);
        assert!(
            boundary <= 2.0 * old_step.max(new_step) + 1.0e-3,
            "{}: a click when {} changes to {:?}: a step of {boundary} at the change, {old_step} before, {new_step} \
             after",
            d.id,
            field.key,
            value
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 5: older versions

/// Every saved fixture of an older version (`<dir>/v<N>-<name>.json`, the state as version N saved it) upgrades to
/// the current version, validates and builds; every version the schema still opens has at least one (7a, test 5)
pub fn fixtures_upgrade(d: &'static Descriptor, dir: &str) {
    let p = Plugin::new(d);
    let schema = d.schema();
    let mut seen = std::collections::BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(Path::new(dir)) {
        let mut files: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for path in files {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let Some(version) =
                name.strip_prefix('v').and_then(|r| r.split('-').next()).and_then(|v| v.parse::<u32>().ok())
            else {
                continue;
            };
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            let state = from_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            let loaded = p.registry.load(d.id, version, state).unwrap_or_else(|e| panic!("{name}: upgrade: {e}"));
            drop(p.build(&loaded, 48_000));
            seen.insert(version);
        }
    }
    for v in schema.oldest_version()..schema.version {
        assert!(seen.contains(&v), "{}: no fixture of version {v} in {dir}", d.id);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 6: sample rates (U36)

/// Measures a probe note: its period (samples) and its level, from 0.3 s to 1.3 s
fn probe(p: &Plugin, state: &Value, key: u8, rate: u32) -> (Option<f64>, f64) {
    let total = (1.4 * f64::from(rate)) as usize;
    let events = match p.d.kind {
        PluginKind::Instrument => vec![note_on(0, 1, key, 1.0)],
        PluginKind::Effect => Vec::new(),
    };
    let out = p.bench(state, rate).render(total, 256, &events, false);
    finite(&out, p.d.tests.max_level, "a probe");
    let x = mono(&out[..out.len().min(2)]);
    let window = &x[(0.3 * f64::from(rate)) as usize..(1.3 * f64::from(rate)) as usize];
    let period = p.d.tests.tuning.map(|t| period(window, f64::from(rate) / t.hz));
    (period, rms(window))
}

/// Rebuilt at 44.1, 48, 96 and 192 kHz, a plugin keeps its tuning (within a cent) and its level (within 0.5 dB)
pub fn sample_rates(d: &'static Descriptor) {
    let p = Plugin::new(d);
    let (state, key, hz) = match d.tests.tuning {
        Some(t) => ((t.state)(), t.key, Some(t.hz)),
        None => (p.defaults(), 69, None),
    };
    let reference = probe(&p, &state, key, 48_000).1;
    assert!(reference > 1.0e-4, "{}: the probe is silent", d.id);
    for rate in RATES {
        let (period, level) = probe(&p, &state, key, rate);
        if let (Some(period), Some(hz)) = (period, hz) {
            let off = cents(f64::from(rate) / period, hz);
            assert!(off.abs() < 1.0, "{}: at {rate} Hz the probe is {off:.3} cents off", d.id);
        }
        let diff = db(level / reference);
        assert!(diff.abs() < 0.5, "{}: at {rate} Hz the level is {diff:.3} dB off the level at 48 kHz", d.id);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 7: U31, every parameter swept and every choice played, under the allocation trap

/// Every parameter swept end to end and back while a chord of more notes than voices plays, with notes coming and
/// going, all under the allocation trap; then all of them at once, at random (U31)
pub fn param_sweep(d: &'static Descriptor) {
    let p = Plugin::new(d);
    let rate = 48_000;
    let schema = d.schema();
    let seconds = 0.4;
    let total = (seconds * f64::from(rate)) as usize;
    let notes = script(&p, rate, seconds + 0.6);
    let mut bench = p.bench(&p.defaults(), rate);
    for (i, spec) in schema.params.iter().enumerate() {
        let mut events = notes.iter().map(|t| Timed::new(t.at + bench.frame(), t.kind)).collect::<Vec<_>>();
        let start = bench.frame();
        let steps = total / 16;
        for s in 0..=steps {
            let x = s as f64 / steps as f64;
            let x = if x <= 0.5 { 2.0 * x } else { 2.0 - 2.0 * x };
            let value = param_at(spec, x) as f32;
            events.push(Timed::new(start + (s * 16) as u64, EventKind::Param { index: i as u32, value }));
        }
        let out = bench.render(total, 256, &events, true);
        finite(&out, p.d.tests.max_level, spec.key);
        // Back to the default before the next
        let reset = [Timed::new(bench.frame(), EventKind::Param { index: i as u32, value: spec.def as f32 })];
        bench.render(64, 64, &reset, true);
    }
    let mut rng = Rng::new(SEED + 7);
    let mut events = Vec::new();
    for s in 0..400u64 {
        let i = rng.below(schema.params.len().max(1) as u64) as usize;
        if let Some(spec) = schema.params.get(i) {
            let start = bench.frame();
            events.push(Timed::new(
                start + s * 60,
                EventKind::Param { index: i as u32, value: random_param(spec, &mut rng) as f32 },
            ));
        }
    }
    let start = bench.frame();
    events.extend(script(&p, rate, 0.5).into_iter().map(|t| Timed::new(t.at + start, t.kind)));
    let out = bench.render(24_000, 128, &events, true);
    finite(&out, p.d.tests.max_level, "every parameter at random");
}

/// A state with every switch (a stepped parameter of 0 to 1) on, so every function of the plugin plays
fn all_on(p: &Plugin, state: &Value) -> Value {
    let schema = p.d.schema();
    let mut params = state.get(PARAMS_KEY).cloned().unwrap_or_else(Value::empty_object);
    for spec in &schema.params {
        if spec.scale == Scale::Stepped && spec.min == 0.0 && spec.max == 1.0 {
            params = params.with(spec.key, Value::real(1.0).unwrap_or(Value::NULL));
        }
    }
    state.with(PARAMS_KEY, params)
}

/// Every value of every choice, built and played (with every switch off and on) under the trap, and adopted from
/// one value to the next under the trap (U31, U30)
pub fn every_choice(d: &'static Descriptor) {
    let p = Plugin::new(d);
    let rate = 48_000;
    let schema = d.schema();
    let defaults = p.defaults();
    let notes = chord(0, 1, p.voices() + 4, 29, Some(9_000));
    for base in [defaults.clone(), all_on(&p, &defaults)] {
        for field in &schema.fields {
            let FieldKind::Choice(keys) = &field.kind else { continue };
            let mut bench: Option<Bench> = None;
            for key in keys.iter() {
                let state = base.with(field.key, Value::text(*key));
                match bench.as_mut() {
                    None => {
                        let mut b = p.bench(&state, rate);
                        let out = b.render(12_000, 256, &notes, true);
                        finite(&out, p.d.tests.max_level, key);
                        bench = Some(b);
                    }
                    Some(b) => {
                        let new = p.build(&state, rate);
                        let (_, old) = b.replace(new, true);
                        drop(old);
                        let start = b.frame();
                        let more: Vec<Timed> = notes.iter().map(|t| Timed::new(t.at + start, t.kind)).collect();
                        let out = b.render(12_000, 256, &more, true);
                        finite(&out, p.d.tests.max_level, key);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// U26: notes of one sample, retriggered at the same sample, more notes than voices

/// Notes of one sample, a note-off and a note-on of the same key at the same sample, a note-on and its note-off at
/// the same sample, and three times more notes than voices at once: under the trap, nothing stuck at the end
pub fn short_notes(d: &'static Descriptor) {
    if d.kind != PluginKind::Instrument {
        return;
    }
    let p = Plugin::new(d);
    let rate = 48_000u32;
    let mut events = Vec::new();
    // One sample long
    for i in 0..8u32 {
        events.push(note_on(1000 + u64::from(i) * 333, 10 + i, 60 + i as u8, 0.9));
        events.push(note_off(1001 + u64::from(i) * 333, 10 + i, 60 + i as u8));
    }
    // Retriggered at the same sample
    events.push(note_on(6000, 30, 64, 0.9));
    events.push(note_off(9000, 30, 64));
    events.push(note_on(9000, 31, 64, 0.9));
    events.push(note_off(20_000, 31, 64));
    // On and off at the same sample
    events.push(note_on(12_000, 40, 67, 0.9));
    events.push(note_off(12_000, 40, 67));
    // Three times more notes than voices
    events.extend(chord(14_000, 100, 3 * p.voices(), 0, Some(16_000)));
    let total = 4 * rate as usize;
    let out = p.bench(&p.defaults(), rate).render(total, 256, &events, true);
    finite(&out, p.d.tests.max_level, "short notes");
    let x = mono(&out);
    assert!(peak(&x[9_200..9_800]) > 1.0e-4, "{}: the retriggered note does not sound", d.id);
    let tail = peak(&x[total - rate as usize / 10..]);
    assert!(tail < 1.0e-5, "{}: something still sounds 3.5 s after the last note-off: {tail}", d.id);
}

// ---------------------------------------------------------------------------------------------------------------
// U35: tuning over a long session

/// A note held for `seconds` of render keeps its pitch: the period at the end is the period at the start (within
/// 0.1 cent) and the probe's (within a cent)
pub fn long_tuning(d: &'static Descriptor, seconds: u64) {
    let Some(t) = d.tests.tuning else { return };
    let p = Plugin::new(d);
    let rate = 48_000u32;
    let state = (t.state)();
    let mut bench = p.bench(&state, rate);
    let total = seconds as usize * rate as usize;
    let window = rate as usize / 2;
    let starts = [rate as u64 / 2, (total - window) as u64];
    let mut windows: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
    let events = [note_on(0, 1, t.key, 1.0)];
    bench.run(total, 1024, &events, false, &mut |at, chans| {
        let n = chans[0].len() as u64;
        for (w, s) in starts.iter().enumerate() {
            if at + n > *s && at < s + window as u64 && windows[w].len() < window {
                let from = s.saturating_sub(at) as usize;
                let take = (window - windows[w].len()).min(n as usize - from);
                for k in from..from + take {
                    let v = chans.iter().take(2).map(|c| c[k]).sum::<f32>() / chans.len().clamp(1, 2) as f32;
                    windows[w].push(v);
                }
            }
        }
    });
    let expected = f64::from(rate) / t.hz;
    let a = period(&windows[0], expected);
    let b = period(&windows[1], expected);
    let drift = cents(a, b);
    assert!(drift.abs() < 0.1, "{}: after {seconds} s the pitch moved {drift:.4} cents", d.id);
    let off = cents(f64::from(rate) / b, t.hz);
    assert!(off.abs() < 1.0, "{}: after {seconds} s the probe is {off:.3} cents off", d.id);
}

// ---------------------------------------------------------------------------------------------------------------
// V3: block sizes

/// The same notes and parameter changes in blocks of 32, 64, 256 and 1024 samples give the same audio, bit for bit
pub fn block_sizes(d: &'static Descriptor) {
    let p = Plugin::new(d);
    let rate = 48_000;
    let mut rng = Rng::new(SEED + 11);
    for state in [p.defaults(), all_on(&p, &p.defaults()), random_state(p.registry.schemas(), &p.type_name, &mut rng)] {
        let mut events = script(&p, rate, 1.0);
        events.extend(param_changes(&p, rate, 1.0, &mut rng));
        let reference = p.bench(&state, rate).render(48_000, BLOCKS[0], &events, false);
        for block in &BLOCKS[1..] {
            let out = p.bench(&state, rate).render(48_000, *block, &events, false);
            let what = format!("{}: blocks of {block} against blocks of {}, state {}", d.id, BLOCKS[0], short(&state));
            same(&reference, &out, &what);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// U28: velocity

/// The response to velocity agrees with the declaration
pub fn velocity(d: &'static Descriptor) {
    if d.kind != PluginKind::Instrument {
        return;
    }
    let p = Plugin::new(d);
    let rate = 48_000;
    let key = d.tests.tuning.map_or(60, |t| t.key);
    let state = d.tests.tuning.map_or_else(|| p.defaults(), |t| (t.state)());
    let level = |v: f32| {
        let out = p.bench(&state, rate).render(36_000, 256, &[note_on(0, 1, key, v)], false);
        let x = mono(&out);
        (rms(&x[12_000..36_000]), out)
    };
    let (full, a) = level(1.0);
    let (half, b) = level(0.5);
    let (quiet, _) = level(0.1);
    match d.velocity {
        Velocity::Ignored => same(&a, &b, "velocity is declared ignored"),
        Velocity::Level => {
            let r = db(half / full);
            assert!((r + 6.0206).abs() < 0.2, "{}: velocity 0.5 is {r:.3} dB from 1 (linear: -6.02)", d.id);
            let q = db(quiet / full);
            assert!((q + 20.0).abs() < 0.2, "{}: velocity 0.1 is {q:.3} dB from 1 (linear: -20)", d.id);
        }
        Velocity::Timbre => {
            assert!(a != b, "{}: velocity is declared to shape the sound, which does not change", d.id);
            assert!(full >= half * 0.944 && half >= quiet * 0.944, "{}: the level falls as velocity grows", d.id);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// U29: frozen defaults

/// The defaults of every published version are kept in `reference`; changing one without a new version fails
pub fn frozen_defaults(d: &'static Descriptor, reference: &str) {
    let p = Plugin::new(d);
    if let Err(m) = lm_state::frozen::check_frozen_defaults(p.registry.schemas(), Path::new(reference)) {
        panic!("{}: {m}", d.id);
    }
}

/// The ports of the defaults and of random states are valid, and a processor's output count follows them
pub fn ports_follow_the_state(d: &'static Descriptor) {
    let p = Plugin::new(d);
    let mut rng = Rng::new(SEED + 13);
    for _ in 0..20 {
        let state = random_state(p.registry.schemas(), &p.type_name, &mut rng);
        let channels = p.channels(&state);
        let out = p.bench(&state, 48_000).render(2048, 256, &chord(0, 1, 3, 0, None), true);
        assert_eq!(out.len(), channels);
        finite(&out, p.d.tests.max_level, "ports");
    }
}
