//! The mixing desk's fake data (owner: the shell/mixing prototype). Channels exist by themselves (spec item 2):
//! normal channels, return channels and the one master. Each has the analog desk's rack (tube, EQ, dynamics,
//! transformer, tolerance), inserts, sends (pre or post fader) and an output to another channel or the master.

use std::collections::BTreeMap;

use crate::NodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Normal,
    Return,
    Master,
}

#[derive(Clone, Debug)]
pub struct Insert {
    pub id: NodeId,
    pub plugin: &'static str,
    pub bypass: bool,
}

#[derive(Clone, Debug)]
pub struct SendInfo {
    pub id: NodeId,
    pub to: NodeId,
    pub level_db: f64,
    pub pre_fader: bool,
}

/// One parameter of the rack: a permanent key (C5), its label key, range, default and unit
#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub unit: &'static str,
}

/// The rack's sections, in signal order, with their parameters. Each section also has `<section>.on`.
pub const RACK: &[(&str, &str, &[ParamSpec])] = &[
    ("tube", "Tube", &[
        ParamSpec { key: "rack.tube.drive", label: "Drive", min: 0.0, max: 10.0, default: 2.0, unit: "" },
        ParamSpec { key: "rack.tube.bias", label: "Bias", min: -1.0, max: 1.0, default: 0.0, unit: "" },
    ]),
    ("eq", "EQ", &[
        ParamSpec { key: "rack.eq.low", label: "Low", min: -15.0, max: 15.0, default: 0.0, unit: "dB" },
        ParamSpec { key: "rack.eq.mid", label: "Mid", min: -15.0, max: 15.0, default: 0.0, unit: "dB" },
        ParamSpec { key: "rack.eq.mid_freq", label: "Mid freq", min: 100.0, max: 8000.0, default: 1000.0, unit: "Hz" },
        ParamSpec { key: "rack.eq.high", label: "High", min: -15.0, max: 15.0, default: 0.0, unit: "dB" },
    ]),
    ("dynamics", "Dynamics", &[
        ParamSpec { key: "rack.dynamics.threshold", label: "Threshold", min: -40.0, max: 0.0, default: -10.0, unit: "dB" },
        ParamSpec { key: "rack.dynamics.ratio", label: "Ratio", min: 1.0, max: 20.0, default: 4.0, unit: ":1" },
        ParamSpec { key: "rack.dynamics.attack", label: "Attack", min: 0.1, max: 30.0, default: 3.0, unit: "ms" },
        ParamSpec { key: "rack.dynamics.release", label: "Release", min: 50.0, max: 1200.0, default: 300.0, unit: "ms" },
    ]),
    ("transformer", "Transformer", &[
        ParamSpec { key: "rack.transformer.color", label: "Color", min: 0.0, max: 1.0, default: 0.3, unit: "" },
    ]),
    ("tolerance", "Tolerance", &[
        ParamSpec { key: "rack.tolerance.amount", label: "Amount", min: 0.0, max: 1.0, default: 0.5, unit: "" },
    ]),
];

pub fn spec(key: &str) -> Option<&'static ParamSpec> {
    RACK.iter().flat_map(|(_, _, ps)| ps.iter()).find(|p| p.key == key)
}

/// The default parameters of a channel: every rack parameter, every section on except the transformer
pub fn default_params(seed: u64) -> BTreeMap<&'static str, f64> {
    let mut m = BTreeMap::new();
    for (section, _, ps) in RACK {
        let on: &'static str = Box::leak(format!("rack.{section}.on").into_boxed_str());
        m.insert(on, if *section == "transformer" { 0.0 } else { 1.0 });
        for p in ps.iter() {
            // a little variety between channels, deterministic
            let jitter = ((seed.wrapping_mul(2654435761) >> (p.key.len() % 13)) % 100) as f64 / 100.0;
            let v = if p.unit == "dB" && p.min < 0.0 && p.max > 0.0 { (jitter - 0.5) * 6.0 } else { p.default };
            m.insert(p.key, v.clamp(p.min, p.max));
        }
    }
    m
}

pub const RETURNS: [&str; 4] = ["Reverb", "Delay", "Plate", "Parallel comp"];
pub const PLUGINS: [&str; 6] = ["Compressor", "EQ 8", "De-esser", "Gate", "Saturator", "Limiter"];
