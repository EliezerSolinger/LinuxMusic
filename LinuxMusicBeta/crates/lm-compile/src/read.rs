//! Reading the project tree: small typed accessors over a [`Value`] that the document already validated (I-3.3),
//! with the schema's defaults for what is absent.

use std::sync::Arc;

use lm_base::{Error, Result};
use lm_state::{NodeId, Value};
use lm_time::{Bpm, Ppq, TempoMap, TempoPoint, TimeSignature};

pub(crate) fn id_of(v: &Value) -> Option<NodeId> {
    NodeId::parse(v.get("id")?.as_text()?).ok()
}

pub(crate) fn ref_of(v: &Value, key: &str) -> Option<NodeId> {
    NodeId::parse(v.get(key)?.as_text()?).ok()
}

pub(crate) fn int(v: &Value, key: &str, default: i64) -> i64 {
    v.get(key).and_then(Value::as_int).unwrap_or(default)
}

pub(crate) fn real(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_real).unwrap_or(default)
}

pub(crate) fn flag(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub(crate) fn text<'a>(v: &'a Value, key: &str, default: &'a str) -> &'a str {
    v.get(key).and_then(Value::as_text).unwrap_or(default)
}

pub(crate) fn items<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> + 'a {
    v.get(key).into_iter().flat_map(Value::items)
}

pub(crate) fn channels(v: &Value, default: i64) -> u16 {
    u16::try_from(int(v, "channels", default).clamp(1, 64)).unwrap_or(2)
}

/// A device channel list (`[1, 2]`, numbered from 1) as 0-based channels
pub(crate) fn device_channels(v: Option<&Value>) -> Vec<u16> {
    v.map(|l| {
        l.items().filter_map(Value::as_int).map(|c| u16::try_from((c - 1).clamp(0, 65_535)).unwrap_or(0)).collect()
    })
    .unwrap_or_default()
}

/// A stable 64-bit number for an id (the kind letter in the top byte; numbers stay below 2^53), for the item keys
/// of the renderer (U25) and the seeds of the nodes (C13)
pub fn id_number(id: NodeId) -> u64 {
    (u64::from(id.kind) << 56) ^ id.number
}

/// The project's settings
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub sample_rate: u32,
    pub ppq: u32,
    pub seed: u64,
}

pub(crate) fn settings(root: &Value) -> Settings {
    let p = root.get("project").cloned().unwrap_or_default();
    Settings {
        sample_rate: u32::try_from(int(&p, "sampleRate", 48_000)).unwrap_or(48_000),
        ppq: u32::try_from(int(&p, "ppq", 960)).unwrap_or(960),
        seed: u64::try_from(int(&p, "seed", 0)).unwrap_or(0),
    }
}

/// The tempo map of the tree (`tempoMap`: points `{tick, bpm, timeSig, ramp}`). Keys from `lm-time`, and
/// `compile.tempo.*` for a point the document should not have let in
pub(crate) fn tempo_map(root: &Value, s: Settings) -> Result<TempoMap> {
    let ppq = Ppq::new(s.ppq)?;
    let mut points = Vec::new();
    for p in items(root, "tempoMap") {
        let bpm = Bpm::parse(text(p, "bpm", "120"))?;
        let sig: Vec<i64> = items(p, "timeSig").filter_map(Value::as_int).collect();
        let (num, den) = match sig.as_slice() {
            [n, d] => (*n, *d),
            _ => (4, 4),
        };
        let time_signature = TimeSignature::new(
            u8::try_from(num).map_err(|_| Error::new("compile.tempo.signature").arg("numerator", num))?,
            u8::try_from(den).map_err(|_| Error::new("compile.tempo.signature").arg("denominator", den))?,
        )?;
        let mut point = TempoPoint::new(int(p, "tick", 0), bpm, time_signature);
        if flag(p, "ramp", false) {
            point = point.linear();
        }
        points.push(point);
    }
    if points.is_empty() {
        points.push(TempoPoint::new(0, Bpm::from_integer(120)?, TimeSignature::COMMON));
    }
    TempoMap::new(s.sample_rate, ppq, &points)
}

/// A parameter key shared by the slot table (`"volume"`, `"rack.tube.drive"`, `"vol1"`)
pub(crate) fn key(k: &str) -> Arc<str> {
    Arc::from(k)
}

/// Deep equality of two objects ignoring one key (a plugin's `params`, which only reach the processor through
/// slots), comparing by identity first
pub(crate) fn same_except(a: &Value, b: &Value, ignore: &[&str]) -> bool {
    if a.same_node(b) {
        return true;
    }
    if !a.is_object() || !b.is_object() {
        return a == b;
    }
    let mut ea = a.entries().filter(|(k, _)| !ignore.contains(k));
    let mut eb = b.entries().filter(|(k, _)| !ignore.contains(k));
    loop {
        match (ea.next(), eb.next()) {
            (None, None) => return true,
            (Some((ka, va)), Some((kb, vb))) => {
                if ka != kb || !(va.same_node(vb) || va == vb) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// Equality of two racks ignoring their real fields (the rack's parameters, which go through slots)
pub(crate) fn same_rack_settings(a: &Value, b: &Value) -> bool {
    if a.same_node(b) {
        return true;
    }
    let keys_a: Vec<&str> = a.keys().collect();
    let keys_b: Vec<&str> = b.keys().collect();
    if keys_a != keys_b {
        return false;
    }
    for k in keys_a {
        let (Some(sa), Some(sb)) = (a.get(k), b.get(k)) else { return false };
        if sa.same_node(sb) {
            continue;
        }
        let fa: Vec<(&str, &Value)> = sa.entries().filter(|(_, v)| v.as_real().is_none()).collect();
        let fb: Vec<(&str, &Value)> = sb.entries().filter(|(_, v)| v.as_real().is_none()).collect();
        if fa != fb {
            return false;
        }
    }
    true
}
