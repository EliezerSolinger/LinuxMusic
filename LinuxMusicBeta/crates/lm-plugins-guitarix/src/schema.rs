//! Each unit's plan: its schema made from what it registers, and how its parameters, fields and outputs map to the
//! unit's (made once per unit, the first time it is asked for, and kept for the program's life: the keys are
//! `&'static str`s of the schema).

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use lm_plugin::Ports;
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};
use lm_sys_guitarix::{ParamDesc, UnitDesc, VarType};

/// The plugin window's live frame: the unit's outputs (its meters), in the order of [`outputs`], as little-endian
/// `f32`s, the rest zeros (the most a unit has is 11: the graphic EQ's bands)
pub const DISPLAY_BYTES: usize = 64;

/// A selector: a `Choice` field, its keys the values' Guitarix ids, each key the unit's value at its place
#[derive(Debug)]
pub struct Choice {
    pub key: &'static str,
    /// Its parameter in the unit ([`UnitDesc::params`])
    pub desc: usize,
    pub keys: &'static [&'static str],
    /// The unit's value for each key
    pub values: Vec<f32>,
    pub default: &'static str,
}

/// A setting the unit reads only when it starts, a real field
#[derive(Debug)]
pub struct Real {
    pub key: &'static str,
    pub desc: usize,
}

/// How a unit's state maps onto it
#[derive(Debug)]
pub struct Plan {
    pub unit: &'static UnitDesc,
    pub id: &'static str,
    pub type_name: &'static str,
    /// The schema's parameters, in order (an event's index is a place here)
    pub params: Vec<ParamSpec>,
    /// Each parameter's place in the unit's
    pub param_desc: Vec<usize>,
    pub choices: Vec<Choice>,
    pub reals: Vec<Real>,
    /// The outputs' places in the unit's parameters (the display frame's order)
    pub outputs: Vec<usize>,
    /// The fixed block the unit runs in, if its sound depends on the size of each call ([`fixed_block`])
    pub fixed: Option<u32>,
}

/// Guitarix's id of a parameter made a state key: every character other than an ASCII letter, a digit or `_` made
/// `_` (`ring_modulator.dry/wet` → `ring_modulator_dry_wet`)
pub fn key_of(id: &str) -> String {
    let mut key: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if !key.starts_with(|c: char| c.is_ascii_alphabetic()) {
        key.insert(0, 'p');
    }
    key
}

/// The plugin id of a unit: `gx-` and its Guitarix id in lower case, every run of other characters a `-`
pub fn plugin_id(unit: &str) -> String {
    let mut id = String::from("gx-");
    let mut dash = false;
    for c in unit.chars() {
        if c.is_ascii_alphanumeric() {
            if dash && !id.ends_with('-') {
                id.push('-');
            }
            dash = false;
            id.push(c.to_ascii_lowercase());
        } else {
            dash = true;
        }
    }
    id
}

/// The settings a unit reads only when it starts (the legacy program redid that start on its main thread when one
/// changed: GuitarixUnits.h, `update`): changing one builds the unit anew, and the new one cannot take over the old
/// one's sound (its convolver or its FFT buffers are made for the old setting)
pub fn rebuilds(unit: &str) -> &'static [&'static str] {
    match unit {
        "cab" => &["cab.select"],
        "pre" => &["pre.select"],
        "ampstack" => &["con.Level"],
        "smbPitchShift" => &["smbPitchShift.latency"],
        _ => &[],
    }
}

/// The fixed block of the units whose sound depends on how many frames each call gets: the convolving ones (their
/// zita-convolver partitions are the block: 64 frames, the least it runs without its threads, a latency of 64) and
/// the amp head (its presence is a convolver, and its noise gate measures each call), and the Detune (it works only
/// on calls of exactly its block: 256, the legacy program's period, as its latency settings were made for it)
pub fn fixed_block(unit: &str) -> Option<u32> {
    match unit {
        "cab" | "pre" | "ampstack" => Some(lm_sys_guitarix::MIN_CONVOLVER_BLOCK),
        "smbPitchShift" => Some(256),
        _ => None,
    }
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn param_spec(key: &'static str, p: &ParamDesc) -> ParamSpec {
    let (min, max) = (f64::from(p.low), f64::from(p.up));
    let def = f64::from(p.default).clamp(min, max);
    let whole = |x: f64| x.fract() == 0.0;
    let spec = ParamSpec::new(key, min, max, def);
    match p.var {
        VarType::Int | VarType::Bool => spec.scale(Scale::Stepped),
        VarType::Float if p.is_log() && min > 0.0 => spec.scale(Scale::Log),
        VarType::Float if p.step >= 1.0 && whole(f64::from(p.step)) && whole(min) && whole(max) && whole(def) => {
            spec.scale(Scale::Stepped)
        }
        VarType::Float => spec,
    }
}

fn make_plan(unit: &'static UnitDesc) -> Plan {
    let id = leak(plugin_id(&unit.id));
    let type_name = leak(lm_plugin::plugin_type(id));
    let rebuild = rebuilds(&unit.id);
    let mut plan = Plan {
        unit,
        id,
        type_name,
        params: Vec::new(),
        param_desc: Vec::new(),
        choices: Vec::new(),
        reals: Vec::new(),
        outputs: Vec::new(),
        fixed: fixed_block(&unit.id),
    };
    for (i, p) in unit.params.iter().enumerate() {
        if p.is_output() {
            plan.outputs.push(i);
            continue;
        }
        let key = leak(key_of(&p.id));
        if p.is_selector() {
            let mut keys: Vec<&'static str> = Vec::new();
            for (n, (vid, _)) in p.values.iter().enumerate() {
                let mut k = vid.trim().to_owned();
                if k.is_empty() || k.len() > 64 || keys.contains(&k.as_str()) {
                    k = format!("{n}");
                }
                keys.push(leak(k));
            }
            let values: Vec<f32> = (0..keys.len()).map(|n| p.low + n as f32).collect();
            let at = ((p.default - p.low).round().max(0.0) as usize).min(keys.len() - 1);
            let default = keys[at];
            plan.choices.push(Choice { key, desc: i, keys: Box::leak(keys.into_boxed_slice()), values, default });
        } else if rebuild.contains(&p.id.as_str()) {
            plan.reals.push(Real { key, desc: i });
        } else {
            plan.params.push(param_spec(key, p));
            plan.param_desc.push(i);
        }
    }
    plan
}

/// The plan of the unit with this Guitarix id (made the first time)
pub fn plan(unit: &str) -> &'static Plan {
    static PLANS: LazyLock<Mutex<BTreeMap<String, &'static Plan>>> = LazyLock::new(|| Mutex::new(BTreeMap::new()));
    let mut plans = PLANS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(p) = plans.get(unit) {
        return p;
    }
    let desc = lm_sys_guitarix::unit(unit).unwrap_or_else(|| panic!("Guitarix has no unit {unit}"));
    let made: &'static Plan = Box::leak(Box::new(make_plan(desc)));
    plans.insert(unit.to_owned(), made);
    made
}

/// The unit's schema (version 1)
pub fn schemas(unit: &str) -> Vec<NodeSchema> {
    let plan = plan(unit);
    let mut s = NodeSchema::new(plan.type_name, 1);
    for p in &plan.params {
        s = s.param(p.clone());
    }
    for c in &plan.choices {
        s = s.field(FieldSpec::choice(c.key, c.keys, c.default));
    }
    for r in &plan.reals {
        let p = &plan.unit.params[r.desc];
        let (min, max) = (f64::from(p.low), f64::from(p.up));
        let def = Value::real(f64::from(p.default).clamp(min, max)).unwrap_or(Value::NULL);
        s = s.field(FieldSpec::new(r.key, FieldKind::Real { min, max }).default(def));
    }
    vec![s]
}

/// An effect works in place on its channel's channels: no ports
pub fn no_ports(_: &Value) -> Ports {
    Ports { outputs: Vec::new(), sidechains: Vec::new() }
}

/// The name Guitarix gives the parameter, field or output with this state key (for the window)
pub fn name_of(unit: &str, key: &str) -> Option<&'static str> {
    let plan = plan(unit);
    plan.unit.params.iter().find(|p| key_of(&p.id) == key).map(|p| p.name.as_str())
}

/// The unit's outputs (its meters), in the display frame's order: (Guitarix's id, name, low, up)
pub fn outputs(unit: &str) -> Vec<(&'static str, &'static str, f32, f32)> {
    let plan = plan(unit);
    plan.outputs
        .iter()
        .map(|&i| {
            let p = &plan.unit.params[i];
            (p.id.as_str(), p.name.as_str(), p.low, p.up)
        })
        .collect()
}
