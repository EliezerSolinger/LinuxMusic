//! The mixing desk's fake data (owner: agent A). Channels exist by themselves (spec item 2): normal channels,
//! groups (a channel others send to, P-18), returns (two in a new project, P-19) and the one master. Each channel
//! has the analog desk's rack (legacy F-MIX-030..055: input, gain stage with its models and tubes, filters and mono
//! bass, dynamics with nine compressor models and a gate, the fixed 4-band EQ, transformer, tolerance seeded by the
//! channel's id, P-21), inserts with their point among the rack's sections (P-20), sends to every return, an
//! output to another channel or the master, and a fader link (relative group, P-18). The master has its own
//! section (F-MIX-060..068).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::{Channel, Config, Doc, Edit, Error, Field, NodeId, Result, Session, Track, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Normal,
    Group,
    Return,
    Master,
}

/// Where an insert sits among the rack's sections (P-20, the six points of the legacy)
pub const INSERT_POINTS: [&str; 6] = ["Pre-gain", "Post-gain", "Post-filter", "Post-dynamics", "Post-EQ", "Pre-pan"];

#[derive(Clone, Debug)]
pub struct Insert {
    pub id: NodeId,
    pub plugin: &'static str,
    pub bypass: bool,
    /// index into INSERT_POINTS; Post-gain by default
    pub point: u8,
}

#[derive(Clone, Debug)]
pub struct SendInfo {
    pub id: NodeId,
    pub to: NodeId,
    /// -60 = off
    pub level_db: f64,
    pub pre_fader: bool,
}

/// One parameter of the rack: a permanent key (C5), its label key, range, default, unit, and the choices of a
/// selector (the value is then the choice's index)
#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub unit: &'static str,
    pub choices: &'static [&'static str],
}

const fn p(key: &'static str, label: &'static str, min: f64, max: f64, default: f64, unit: &'static str) -> ParamSpec {
    ParamSpec { key, label, min, max, default, unit, choices: &[] }
}
const fn choice(key: &'static str, label: &'static str, choices: &'static [&'static str], default: usize) -> ParamSpec {
    ParamSpec { key, label, min: 0.0, max: (choices.len() - 1) as f64, default: default as f64, unit: "", choices }
}
const fn switch(key: &'static str, label: &'static str, default: bool) -> ParamSpec {
    ParamSpec { key, label, min: 0.0, max: 1.0, default: if default { 1.0 } else { 0.0 }, unit: "", choices: &["Off", "On"] }
}

pub const GAIN_MODELS: &[&str] = &["FLAT", "4558", "TL072", "5532", "5534", "12AX7", "12AX7 V", "12AX7 H", "12AT7", "12AU7"];
pub const COMP_MODELS: &[&str] =
    &["SSL", "API 2500", "dbx 160", "1176", "LA-2A", "670", "Gates Sta-Level", "Altec 436", "RCA BA6A"];
pub const GATE_MODES: &[&str] = &["STD", "KICK", "SNARE", "TOMS", "VOX", "GTR", "BASS", "NOISE", "EXP"];

/// The channel rack's sections, in signal order. Each section also has `rack.<section>.on`.
pub const RACK: &[(&str, &str, &[ParamSpec])] = &[
    ("input", "Input", &[
        p("rack.input.trim", "Trim", -60.0, 24.0, 0.0, "dB"),
        switch("rack.input.mono", "Mono", false),
        switch("rack.input.phase", "Ø", false),
        switch("rack.input.swap", "L⇄R", false),
        switch("rack.input.lcut", "LCUT", false),
        p("rack.input.width", "Width", 0.0, 200.0, 100.0, "%"),
    ]),
    ("gain", "Gain stage", &[
        p("rack.gain.gain", "GAIN", 0.0, 1.0, 0.25, ""),
        choice("rack.gain.model", "Model", GAIN_MODELS, 1),
        choice("rack.gain.tube", "Tubes", &["LT", "RL"], 0),
    ]),
    ("filter", "Filters", &[
        p("rack.filter.hpf", "HPF", 20.0, 1000.0, 20.0, "Hz"),
        p("rack.filter.lpf", "LPF", 1000.0, 20000.0, 20000.0, "Hz"),
        p("rack.filter.bass", "Mono bass", 0.0, 100.0, 0.0, "%"),
        p("rack.filter.bass_freq", "Freq", 20.0, 300.0, 120.0, "Hz"),
    ]),
    ("dynamics", "Dynamics", &[
        choice("rack.dynamics.model", "Model", COMP_MODELS, 0),
        p("rack.dynamics.comp", "COMP", 0.0, 1.0, 0.0, ""),
        p("rack.dynamics.ratio", "RATIO", 0.0, 1.0, 0.0, ""),
        choice("rack.dynamics.attack", "ATK", &["FAST", "NORM", "SLOW"], 1),
        p("rack.dynamics.release", "REL", 0.0, 1.0, 0.5, ""),
        p("rack.dynamics.gate_thr", "Gate", -30.0, 10.0, -10.0, "dBu"),
        p("rack.dynamics.gate_range", "Range", 0.0, 40.0, 0.0, "dB"),
        choice("rack.dynamics.gate_mode", "Mode", GATE_MODES, 0),
    ]),
    ("eq", "EQ", &[
        p("rack.eq.hi", "HI 12k", -15.0, 15.0, 0.0, "dB"),
        p("rack.eq.hmf", "HMF 2.5k", -15.0, 15.0, 0.0, "dB"),
        p("rack.eq.lmf", "LMF 400", -15.0, 15.0, 0.0, "dB"),
        p("rack.eq.lo", "LO 80", -15.0, 15.0, 0.0, "dB"),
    ]),
    ("transformer", "Transformer", &[p("rack.transformer.color", "Color", 0.0, 1.0, 0.3, "")]),
    ("tolerance", "Tolerance", &[]),
];

/// The master's section (F-MIX-060..067). The monitoring (LEVEL, DIM, CUT, MONO, SIDE) is not here: it is never
/// saved, undone or exported, so it lives in the interface's state, not in the document.
pub const MASTER: &[(&str, &str, &[ParamSpec])] = &[
    ("pre", "PRE", &[
        p("master.pre_l", "PRE L", -60.0, 6.0, 0.0, "dB"),
        p("master.pre_r", "PRE R", -60.0, 6.0, 0.0, "dB"),
        switch("master.link", "LINK", true),
    ]),
    ("drive", "DRIVE", &[
        p("master.drive", "DRIVE", 0.0, 1.0, 0.25, ""),
        choice("master.drive_model", "Model", GAIN_MODELS, 0),
    ]),
    ("stereo", "Stereo", &[
        p("master.width", "WIDTH", 0.0, 200.0, 100.0, "%"),
        p("master.bass", "BASS", 0.0, 100.0, 0.0, "%"),
        p("master.bass_freq", "FREQ", 20.0, 300.0, 120.0, "Hz"),
        p("master.air", "AIR", -6.0, 6.0, 0.0, "dB"),
    ]),
    ("bus", "BUS COMP", &[
        switch("master.bus.in", "IN", false),
        p("master.bus.thr", "THR", -15.0, 15.0, 5.0, ""),
        choice("master.bus.ratio", "RATIO", &["2", "4", "10"], 1),
        choice("master.bus.atk", "ATK", &["0.1", "0.3", "1", "3", "10", "30"], 4),
        choice("master.bus.rel", "REL", &["0.1", "0.3", "0.6", "1.2", "Auto"], 4),
        p("master.bus.make", "MAKE", -5.0, 15.0, 0.0, "dB"),
    ]),
    ("post", "POST", &[switch("master.xfmr", "XFMR", false)]),
];

pub fn spec(key: &str) -> Option<&'static ParamSpec> {
    RACK.iter().chain(MASTER.iter()).flat_map(|(_, _, ps)| ps.iter()).find(|p| p.key == key)
}

/// A rack section's on/off key
pub fn section_key(section: &str) -> &'static str {
    match section {
        "input" => "rack.input.on",
        "gain" => "rack.gain.on",
        "filter" => "rack.filter.on",
        "dynamics" => "rack.dynamics.on",
        "eq" => "rack.eq.on",
        "transformer" => "rack.transformer.on",
        _ => "rack.tolerance.on",
    }
}

/// GAIN's position as dB (F-MIX-040): 0 = muted; cubic up to 0.25 (0 dB); above, linear in dB up to +60
pub fn gain_db(pos: f64) -> f64 {
    if pos <= 0.0 {
        f64::NEG_INFINITY
    } else if pos <= 0.25 {
        60.0 * (pos / 0.25).log10() // cubic in amplitude: 20*log10((pos/0.25)^3)
    } else {
        (pos - 0.25) / 0.75 * 60.0
    }
}

/// The tolerance of the components, seeded by the channel's id (P-21): gain ±0.1 dB, frequencies ±1 %, EQ gains
/// ±2 %, saturation point ±3 %
pub fn tolerance(id: NodeId) -> [f64; 4] {
    let mut x = id.number.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA5A5;
    let mut next = || {
        x ^= x >> 13;
        x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((x >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    };
    [next() * 0.1, next() * 1.0, next() * 2.0, next() * 3.0]
}

/// The parameters of a new channel: every parameter at its default, a little variety between fake channels
pub fn default_params(seed: u64, master: bool) -> BTreeMap<&'static str, f64> {
    let mut m = BTreeMap::new();
    let sections = if master { MASTER } else { RACK };
    for (section, _, ps) in sections {
        if !master {
            m.insert(section_key(section), if *section == "transformer" { 0.0 } else { 1.0 });
        }
        for p in ps.iter() {
            let v = match p.key {
                "rack.eq.hi" | "rack.eq.lo" => ((seed * 7) % 9) as f64 - 4.0,
                "rack.dynamics.comp" => ((seed * 3) % 5) as f64 / 10.0,
                "rack.dynamics.ratio" => if seed % 3 == 0 { 0.4 } else { 0.0 },
                "rack.gain.model" => (seed % GAIN_MODELS.len() as u64) as f64,
                "rack.dynamics.model" => (seed % COMP_MODELS.len() as u64) as f64,
                "rack.filter.hpf" => if seed % 4 == 1 { 80.0 } else { 20.0 },
                _ => p.default,
            };
            m.insert(p.key, v.clamp(p.min, p.max));
        }
    }
    m
}

pub const RETURNS: [&str; 2] = ["Reverb", "Delay"];
pub const PLUGINS: [&str; 6] = ["Compressor", "EQ 8", "De-esser", "Gate", "Saturator", "Limiter"];

fn channel(id: NodeId, name: String, kind: ChannelKind, output: Option<NodeId>, master: bool) -> Channel {
    Channel {
        id,
        name,
        kind,
        volume_db: 0.0,
        pan: 0.0,
        mute: false,
        solo: false,
        output,
        inserts: Vec::new(),
        sends: Vec::new(),
        params: default_params(id.number, master),
        linked: false,
    }
}

/// The fake desk: channel 0 is the drums' group (its four piece channels send to it), then the tracks' channels, the
/// two returns and the master. Normal channels show sends to both returns (P-19); the outputs go to the master.
pub(crate) fn make_channels(config: &Config, tracks: &[Track]) -> Vec<Channel> {
    let master = NodeId { kind: b'm', number: 999 };
    let returns: Vec<NodeId> = (0..RETURNS.len()).map(|r| NodeId { kind: b'm', number: 900 + r as u64 }).collect();
    let group = NodeId { kind: b'm', number: 1 };
    let mut sub = 50_000u64;
    let mut next = || {
        sub += 1;
        NodeId { kind: b'x', number: sub }
    };
    let mut chs = Vec::new();
    for i in 0..config.channels {
        let id = NodeId { kind: b'm', number: i as u64 + 1 };
        let kind = if i == 0 { ChannelKind::Group } else { ChannelKind::Normal };
        let name = match i {
            0 => "Bateria (peças)".to_string(),
            1..=4 => ["Bumbo", "Caixa", "Overheads", "Sala"][i - 1].to_string(),
            _ => tracks.get(i).map_or(format!("Canal {}", i + 1), |t| t.name.clone()),
        };
        let mut ch = channel(id, name, kind, Some(if (1..=4).contains(&i) { group } else { master }), false);
        ch.pan = if (1..=4).contains(&i) { [0.0, 0.1, -0.5, 0.5][i - 1] } else { [0.0, -0.3, 0.3, -0.6, 0.6][i % 5] };
        ch.inserts = (0..(i % 3))
            .map(|k| Insert { id: next(), plugin: PLUGINS[(i + k) % PLUGINS.len()], bypass: k == 1 && i % 5 == 0, point: if k == 0 { 1 } else { 4 } })
            .collect();
        if kind == ChannelKind::Normal {
            ch.sends = returns
                .iter()
                .enumerate()
                .map(|(r, to)| SendInfo {
                    id: next(),
                    to: *to,
                    level_db: if (i + r) % 3 == 0 { -12.0 - (i % 7) as f64 } else { -60.0 },
                    pre_fader: r == 1 && i % 4 == 0,
                })
                .collect();
        }
        chs.push(ch);
    }
    for (r, name) in RETURNS.iter().enumerate() {
        let mut ch = channel(returns[r], name.to_string(), ChannelKind::Return, Some(master), false);
        ch.volume_db = -3.0;
        ch.inserts = vec![Insert { id: next(), plugin: name, bypass: false, point: 4 }];
        chs.push(ch);
    }
    let mut m = channel(master, "Master".into(), ChannelKind::Master, None, true);
    m.inserts = vec![Insert { id: next(), plugin: "Limiter", bypass: false, point: 4 }];
    chs.push(m);
    chs
}

/// The routes the fake engine meters through, in meter indices: (from, to, gain), a channel's sources before it
pub(crate) fn routes(doc: &Doc, channel_meter: impl Fn(usize) -> usize) -> Vec<(usize, usize, f32)> {
    let chs = &doc.channels;
    let index = |id: NodeId| chs.iter().position(|c| c.id == id);
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(o) = chs[i].output.and_then(index) {
            i = o;
            d += 1;
            if d > chs.len() {
                break;
            }
        }
        d
    };
    let mut order: Vec<usize> = (0..chs.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(depth(i)));
    let mut out = Vec::new();
    for i in order {
        for s in &chs[i].sends {
            if s.level_db > -60.0 {
                if let Some(t) = index(s.to) {
                    out.push((channel_meter(i), channel_meter(t), crate::db_gain(s.level_db)));
                }
            }
        }
        if let Some(t) = chs[i].output.and_then(index) {
            out.push((channel_meter(i), channel_meter(t), 0.9));
        }
    }
    out
}

/// The shared route table the fake engine reads (a lock is fine in a fake; the real core installs a program)
pub(crate) type Routes = Arc<Mutex<Vec<(usize, usize, f32)>>>;

/// The mixer's fields of `Edit::set` (lib.rs delegates here)
pub(crate) fn set(doc: &mut Doc, id: NodeId, field: Field, v: Value) -> Result<()> {
    let ch = doc.channels.iter_mut().find(|c| c.id == id).ok_or(Error::NoSuchNode(id))?;
    match (field, v) {
        (Field::InsertPoint(ins), Value::Real(x)) => {
            ch.inserts.iter_mut().find(|i| i.id == ins).ok_or(Error::NoSuchNode(ins))?.point = x.clamp(0.0, 5.0) as u8
        }
        (Field::Bypass(ins), Value::Bool(b)) => {
            ch.inserts.iter_mut().find(|i| i.id == ins).ok_or(Error::NoSuchNode(ins))?.bypass = b
        }
        (Field::FaderLink, Value::Bool(b)) => ch.linked = b,
        _ => return Err(Error::WrongType),
    }
    Ok(())
}

pub(crate) fn get(doc: &Doc, id: NodeId, field: Field) -> Result<Value> {
    let ch = doc.channels.iter().find(|c| c.id == id).ok_or(Error::NoSuchNode(id))?;
    let ins = |x: NodeId| ch.inserts.iter().find(|i| i.id == x).ok_or(Error::NoSuchNode(x));
    Ok(match field {
        Field::InsertPoint(x) => Value::Real(ins(x)?.point as f64),
        Field::Bypass(x) => Value::Bool(ins(x)?.bypass),
        Field::FaderLink => Value::Bool(ch.linked),
        _ => return Err(Error::WrongType),
    })
}

impl Session {
    fn normal_count(&self) -> usize {
        self.doc.channels.iter().filter(|c| matches!(c.kind, ChannelKind::Normal | ChannelKind::Group)).count()
    }

    fn master_id(&self) -> Option<NodeId> {
        self.doc.channels.iter().find(|c| c.kind == ChannelKind::Master).map(|c| c.id)
    }

    /// A new normal channel after the last normal one (the ghost strip's click, Shift+Insert, F-MIX-004)
    pub fn add_channel(&mut self) -> crate::Result<NodeId> {
        let n = self.normal_count();
        let id = NodeId { kind: b'm', number: 2000 + self.next_id };
        self.next_id += 1;
        let master = self.master_id();
        self.transact("Add channel", |e: &mut Edit| {
            e.doc.channels.insert(n, channel(id, format!("Canal {}", n + 1), ChannelKind::Normal, master, false));
            e.diff.changed.push((id, None));
            Ok(())
        })?;
        Ok(id)
    }

    /// "Create channels for the outputs" (I-3.9, F-MIX-012): a group channel for the track, one channel per output
    /// sending to it, and each output routed to its channel. An action, one undo step.
    pub fn create_channels_for_outputs(&mut self, track: usize) -> crate::Result<()> {
        let t = self.doc.tracks.get(track).ok_or(Error::WrongType)?.clone();
        let master = self.master_id();
        let base = 3000 + self.next_id * 10;
        self.next_id += 1;
        let n = self.normal_count();
        self.transact("Create channels for the outputs", |e: &mut Edit| {
            let group = NodeId { kind: b'm', number: base };
            let mut new = vec![channel(group, format!("{} (peças)", t.name), ChannelKind::Group, master, false)];
            for (k, o) in t.outputs.iter().enumerate() {
                let id = NodeId { kind: b'm', number: base + 1 + k as u64 };
                new.push(channel(id, format!("{} {}", t.name, o.port), ChannelKind::Normal, Some(group), false));
                let tr = e.doc.tracks.iter_mut().find(|x| x.id == t.id).unwrap();
                tr.outputs[k].to = id;
            }
            for (k, c) in new.into_iter().enumerate() {
                e.diff.changed.push((c.id, None));
                e.doc.channels.insert(n + k, c);
            }
            e.diff.changed.push((t.id, None));
            Ok(())
        })?;
        Ok(())
    }
}

/// The desk's structural edits (port.rs routes them here): remove a channel (not the master) or an insert, move an
/// insert inside its channel's chain. A channel's removal sends what came to it to the master (`OnRemove::Redirect`).
pub(crate) fn apply_op(e: &mut Edit, op: &crate::port::EditOp, _created: &[(u32, NodeId)]) -> Option<Result<Option<(u32, NodeId)>>> {
    use crate::port::{EditOp, Ref};
    match op {
        EditOp::Remove { id: Ref::Id(id) } if id.kind == b'm' => Some((|| {
            let i = e.doc.channels.iter().position(|c| c.id == *id).ok_or(Error::NoSuchNode(*id))?;
            if e.doc.channels[i].kind == ChannelKind::Master {
                return Err(Error::WrongType);
            }
            let master = e.doc.channels.iter().find(|c| c.kind == ChannelKind::Master).map(|c| c.id);
            e.doc.channels.remove(i);
            for c in e.doc.channels.iter_mut() {
                if c.output == Some(*id) {
                    c.output = master;
                }
                c.sends.retain(|s| s.to != *id);
            }
            for t in e.doc.tracks.iter_mut() {
                for o in t.outputs.iter_mut() {
                    if o.to == *id {
                        o.to = master.unwrap_or(o.to);
                    }
                }
            }
            e.diff.changed.push((*id, None));
            Ok(None)
        })()),
        EditOp::Remove { id: Ref::Id(id) } if e.doc.channels.iter().any(|c| c.inserts.iter().any(|x| x.id == *id)) => {
            let c = e.doc.channels.iter_mut().find(|c| c.inserts.iter().any(|x| x.id == *id)).unwrap();
            c.inserts.retain(|x| x.id != *id);
            let cid = c.id;
            e.diff.changed.push((cid, None));
            Some(Ok(None))
        }
        EditOp::Move { id: Ref::Id(id), parent: Ref::Id(ch), list: "inserts", index } => Some((|| {
            let c = e.doc.channels.iter_mut().find(|c| c.id == *ch).ok_or(Error::NoSuchNode(*ch))?;
            let from = c.inserts.iter().position(|x| x.id == *id).ok_or(Error::NoSuchNode(*id))?;
            let ins = c.inserts.remove(from);
            let to = (*index).min(c.inserts.len());
            c.inserts.insert(to, ins);
            e.diff.changed.push((*ch, None));
            Ok(None)
        })()),
        _ => None,
    }
}
