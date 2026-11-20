//! What the generated tests are made of: random valid states from a schema, test resources, test signals, note
//! scripts, and the measures (level, pitch, clicks).

use std::sync::Arc;

use lm_render::{EventKind, NoteId, Rng, SampleData};
use lm_state::{FieldKind, NodeSchema, PARAMS_KEY, ParamSpec, Resource, Scale, SchemaRegistry, Value};

use crate::resources::{Loaded, ResourceView};
use crate::testing::bench::Timed;

/// A random value of a parameter: often an end of its range, otherwise uniform (log-uniform on a log scale), whole
/// on a stepped one
pub fn random_param(p: &ParamSpec, rng: &mut Rng) -> f64 {
    let v = match rng.below(8) {
        0 => p.min,
        1 => p.max,
        2 => p.def,
        _ => {
            let u = rng.next_f64();
            if p.scale == Scale::Log && p.min > 0.0 {
                p.min * (p.max / p.min).powf(u)
            } else {
                p.min + (p.max - p.min) * u
            }
        }
    };
    let v = if p.scale == Scale::Stepped { v.round() } else { v };
    v.clamp(p.min, p.max)
}

/// A parameter's value at `x` (0 to 1) of its range, as a knob turned end to end
pub fn param_at(p: &ParamSpec, x: f64) -> f64 {
    let v = if p.scale == Scale::Log && p.min > 0.0 {
        p.min * (p.max / p.min).powf(x)
    } else {
        p.min + (p.max - p.min) * x
    };
    let v = if p.scale == Scale::Stepped { v.round() } else { v };
    v.clamp(p.min, p.max)
}

/// A resource of `kind` with a random valid key
pub fn random_resource(kind: &str, rng: &mut Rng) -> Resource {
    let mut key = String::with_capacity(64);
    for _ in 0..64 {
        let d = rng.below(16) as u32;
        key.push(char::from_digit(d, 16).unwrap_or('0'));
    }
    Resource::new(kind, &key).unwrap_or_else(|_| unreachable!("a 64-digit hex key is valid"))
}

/// A test resource's content: audio is a short period of a bright wave (a user wave, an LFO shape); anything else is
/// some bytes
pub fn test_loaded(resource: &Resource, rate: u32) -> Loaded {
    match resource.kind() {
        "audio" => {
            let frames = 600;
            let left: Vec<f32> = (0..frames)
                .map(|i| {
                    let x = i as f32 / frames as f32;
                    0.6 * (2.0 * x - 1.0) + 0.3 * libm::sinf(x * 6.0 * std::f32::consts::PI)
                })
                .collect();
            let right: Vec<f32> = left.iter().map(|x| -x).collect();
            let data = SampleData::from_channels(rate, vec![left, right])
                .unwrap_or_else(|_| unreachable!("two channels of the same length"));
            Loaded::Audio(Arc::new(data))
        }
        _ => Loaded::Bytes(Arc::from(vec![1u8, 2, 3, 4].into_boxed_slice())),
    }
}

/// Every resource a state cites, loaded with test content
pub fn test_resources(registry: &SchemaRegistry, type_name: &str, state: &Value, rate: u32) -> ResourceView {
    let mut view = ResourceView::new();
    if let Ok(found) = registry.resources(type_name, state) {
        for (_, r) in found {
            let loaded = test_loaded(&r, rate);
            view.insert(r, loaded);
        }
    }
    view
}

fn random_text(rng: &mut Rng) -> String {
    let n = rng.below(12) as usize;
    (0..n).map(|_| char::from(b'a' + rng.below(26) as u8)).collect()
}

fn random_value(
    registry: &SchemaRegistry,
    kind: &FieldKind,
    nullable: bool,
    rng: &mut Rng,
    depth: usize,
) -> Option<Value> {
    if nullable && rng.below(3) == 0 {
        return Some(Value::NULL);
    }
    Some(match kind {
        FieldKind::Bool => Value::bool(rng.below(2) == 1),
        FieldKind::Int { min, max } => {
            let (lo, hi) = (*min, (*max).min(min.saturating_add(1_000_000)));
            let span = (hi - lo) as u64 + 1;
            Value::int(lo + rng.below(span) as i64)
        }
        FieldKind::Real { min, max } => {
            let u = rng.next_f64();
            Value::real((min + (max - min) * u).clamp(*min, *max)).ok()?
        }
        FieldKind::Text => Value::text(random_text(rng)),
        FieldKind::Choice(keys) => Value::text(keys[rng.below(keys.len() as u64) as usize]),
        FieldKind::Resource(k) => Value::resource(random_resource(k, rng)),
        FieldKind::List(inner) if depth < 4 => {
            let n = rng.below(3);
            Value::list((0..n).filter_map(|_| random_value(registry, inner, false, rng, depth + 1)))
        }
        FieldKind::Object(t) if depth < 4 => random_node(registry, t, rng, depth + 1)?,
        _ => return None,
    })
}

fn random_node(registry: &SchemaRegistry, type_name: &str, rng: &mut Rng, depth: usize) -> Option<Value> {
    let schema: &NodeSchema = registry.find(type_name)?;
    let mut node = registry.defaults(type_name).ok()?;
    for f in &schema.fields {
        if let Some(v) = random_value(registry, &f.kind, f.nullable, rng, depth) {
            node = node.with(f.key, v);
        }
    }
    if !schema.params.is_empty() {
        let mut params = Value::empty_object();
        for p in &schema.params {
            // Sometimes left out (the default then applies)
            if rng.below(6) != 0 {
                params = params.with(p.key, Value::real(random_param(p, rng)).ok()?);
            }
        }
        node = node.with(PARAMS_KEY, params);
    }
    Some(node)
}

/// A random valid state of a plugin type (fields and parameters within the schema)
pub fn random_state(registry: &SchemaRegistry, type_name: &str, rng: &mut Rng) -> Value {
    random_node(registry, type_name, rng, 0).unwrap_or_else(Value::empty_object)
}

/// An effect's test signal: a saw, a sine an octave and a fifth up, and a little noise, different on each channel
pub fn test_input(frame: u64, ch: usize, rate: u32) -> f32 {
    let t = frame as f64 / f64::from(rate);
    let f = 110.0 * (1.0 + ch as f64 * 0.01);
    let saw = 2.0 * (t * f).fract() - 1.0;
    let sine = libm::sin(t * f * 3.0 * std::f64::consts::TAU);
    let mut z = frame.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (ch as u64).rotate_left(17);
    z = (z ^ (z >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let noise = (z >> 40) as f64 / (1u64 << 24) as f64 * 2.0 - 1.0;
    (0.3 * saw + 0.2 * sine + 0.02 * noise) as f32
}

/// Note numbers for scripts
pub fn note_on(at: u64, id: u32, key: u8, velocity: f32) -> Timed {
    Timed::new(at, EventKind::NoteOn { note: NoteId(id), channel: 0, key, velocity })
}

pub fn note_off(at: u64, id: u32, key: u8) -> Timed {
    Timed::new(at, EventKind::NoteOff { note: NoteId(id), channel: 0, key, velocity: 0.0 })
}

/// A chord of `count` notes from `first` (keys spread over three octaves), starting at `at`, staggered by `stagger`
/// samples, held until `until` (none: held)
pub fn chord(at: u64, first_id: u32, count: usize, stagger: u64, until: Option<u64>) -> Vec<Timed> {
    let mut v = Vec::new();
    for i in 0..count {
        let key = 48 + ((i * 7) % 36) as u8;
        let id = first_id + i as u32;
        let velocity = 0.5 + 0.5 * ((i % 5) as f32 / 4.0);
        v.push(note_on(at + i as u64 * stagger, id, key, velocity));
        if let Some(until) = until {
            v.push(note_off(until + i as u64 * stagger / 2, id, key));
        }
    }
    v
}

// ---- measures ----

pub fn mono(chans: &[Vec<f32>]) -> Vec<f32> {
    let n = chans.first().map_or(0, Vec::len);
    (0..n).map(|i| chans.iter().map(|c| c[i]).sum::<f32>() / chans.len().max(1) as f32).collect()
}

pub fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt()
}

pub fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |p, v| p.max(v.abs()))
}

pub fn db(ratio: f64) -> f64 {
    20.0 * ratio.max(1e-30).log10()
}

/// The period of `x` near `expected` samples (within 3 %), by the squared difference function and a parabola through
/// its minimum: a fraction of a sample, whatever the waveform
pub fn period(x: &[f32], expected: f64) -> f64 {
    let lo = ((expected * 0.97).floor() as usize).max(2);
    let hi = (expected * 1.03).ceil() as usize + 1;
    let window = x.len().saturating_sub(hi + 1);
    let d = |lag: usize| -> f64 {
        (0..window)
            .map(|n| {
                let e = f64::from(x[n]) - f64::from(x[n + lag]);
                e * e
            })
            .sum()
    };
    let mut best = lo;
    let mut best_d = f64::MAX;
    for lag in lo..=hi {
        let v = d(lag);
        if v < best_d {
            best_d = v;
            best = lag;
        }
    }
    let (a, b, c) = (d(best - 1), best_d, d(best + 1));
    let denom = a - 2.0 * b + c;
    let shift = if denom > 0.0 { 0.5 * (a - c) / denom } else { 0.0 };
    best as f64 + shift.clamp(-1.0, 1.0)
}

/// Cents between two frequencies (or two periods, inverted)
pub fn cents(a: f64, b: f64) -> f64 {
    1200.0 * (a / b).log2()
}

/// The largest step between two samples in `range`
pub fn largest_step(x: &[f32], range: std::ops::Range<usize>) -> f32 {
    let start = range.start.max(1);
    (start..range.end.min(x.len())).map(|i| (x[i] - x[i - 1]).abs()).fold(0.0, f32::max)
}
