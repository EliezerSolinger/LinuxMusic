//! The fake's plugins (their schemas and ports) and the demo project: a real lm-doc document with the project's
//! schemas, built by transactions, coherent with the fake engine (the same resources, the same channels).

use std::sync::Arc;

use lm_base::{Label, Result};
use lm_doc::{Document, Options, Port, Ports, register_project_schemas};
use lm_state::{FieldSpec, NodeId, NodeSchema, ParamSpec, PathStep, ROOT, Resource, Scale, SchemaRegistry, Value};

/// A plugin parameter: key, min, max, default
pub type Param = (&'static str, f64, f64, f64);

/// The fake plugins: (id, instrument?, parameters)
pub const PLUGINS: &[(&str, bool, &[Param])] = &[
    (
        "tripleoscillator",
        true,
        &[
            ("vol1", 0.0, 1.0, 0.8),
            ("vol2", 0.0, 1.0, 0.5),
            ("vol3", 0.0, 1.0, 0.0),
            // the filter and the envelopes are the instrument functions' (add_instrument_functions)
            ("coarse2", -24.0, 24.0, -12.0),
            ("fine2", -100.0, 100.0, 0.0),
        ],
    ),
    ("avldrums", true, &[("level", 0.0, 1.0, 1.0)]),
    ("sampler", true, &[("gain", 0.0, 2.0, 1.0), ("start", 0.0, 1.0, 0.0), ("end", 0.0, 1.0, 1.0)]),
    (
        "compressor",
        false,
        &[
            ("threshold", -60.0, 0.0, -18.0),
            ("ratio", 1.0, 20.0, 4.0),
            ("attack", 0.1, 100.0, 10.0),
            ("release", 10.0, 2000.0, 200.0),
            ("makeup", 0.0, 24.0, 0.0),
        ],
    ),
    ("eq8", false, &[("low", -18.0, 18.0, 0.0), ("mid", -18.0, 18.0, 0.0), ("high", -18.0, 18.0, 0.0)]),
    ("reverb", false, &[("size", 0.0, 1.0, 0.6), ("damping", 0.0, 1.0, 0.4), ("mix", 0.0, 1.0, 1.0)]),
    ("delay", false, &[("time", 0.01, 2.0, 0.375), ("feedback", 0.0, 0.95, 0.35), ("mix", 0.0, 1.0, 1.0)]),
    ("limiter", false, &[("ceiling", -12.0, 0.0, -0.3), ("release", 1.0, 1000.0, 50.0)]),
];

pub const KITS: &[&str] = &["acoustic", "big"];

/// The instrument functions every per-note instrument gets, with the keys of lm-plugin's `instrument::schema`
/// (F-IJN-002, 010, 011, 013, 014, 015): the fake cannot depend on lm-plugin, so it writes the same keys
pub fn add_instrument_functions(mut s: NodeSchema) -> NodeSchema {
    const LFO_SHAPES: &[&str] = &["sine", "triangle", "saw", "square", "random", "user"];
    const SYNC: &[&str] = &["off", "2/1", "1/1", "1/2", "1/4", "1/8", "1/16", "1/32"];
    const FILTERS: &[&str] = &["low-pass", "high-pass", "band-pass-csg", "band-pass-czpg", "notch", "all-pass", "moog"];
    const CHORDS: &[&str] = &["octave", "major", "minor", "sus4", "maj7", "m7", "7", "major-scale", "minor-scale"];
    let switch = |k: &'static str| ParamSpec::new(k, 0.0, 1.0, 0.0).scale(Scale::Stepped);
    s = s.param(ParamSpec::new("pitch", -100.0, 100.0, 0.0).unit("cents"));
    s = s.param(ParamSpec::new("bendRange", 1.0, 60.0, 2.0).scale(Scale::Stepped).unit("st").not_automatable());
    for t in ["vol", "cut", "res"] {
        let k = |n: &str| -> &'static str { Box::leak(format!("{t}{n}").into_boxed_str()) };
        for (n, min, max, d) in [
            ("Predelay", 0.0, 2.0, 0.0),
            ("Attack", 0.0, 2.0, 0.0),
            ("Hold", 0.0, 2.0, 0.5),
            ("Decay", 0.0, 2.0, 0.5),
            ("Sustain", 0.0, 1.0, 0.5),
            ("Release", 0.0, 2.0, 0.1),
            ("Amount", -1.0, 1.0, 0.0),
            ("LfoPredelay", 0.0, 1.0, 0.0),
            ("LfoAttack", 0.0, 1.0, 0.0),
        ] {
            s = s.param(ParamSpec::new(k(n), min, max, d));
        }
        s = s.param(ParamSpec::new(k("LfoSpeed"), 0.001, 1.0, 0.1).scale(Scale::Log));
        s = s.param(ParamSpec::new(k("LfoAmount"), -1.0, 1.0, 0.0));
        s = s.param(switch(k("LfoX100")));
        s = s.param(switch(k("LfoEnvAmount")));
        s = s.field(FieldSpec::choice(k("LfoShape"), LFO_SHAPES, "sine"));
        s = s.field(FieldSpec::choice(k("LfoSync"), SYNC, "off"));
    }
    for p in [
        switch("filterOn"),
        ParamSpec::new("cutoff", 1.0, 14000.0, 14000.0).scale(Scale::Log).unit("Hz"),
        ParamSpec::new("resonance", 0.01, 10.0, 0.5),
        switch("chordOn"),
        ParamSpec::new("chordRange", 1.0, 9.0, 1.0).scale(Scale::Stepped),
        switch("arpOn"),
        ParamSpec::new("arpRange", 1.0, 9.0, 1.0).scale(Scale::Stepped),
        ParamSpec::new("arpRepeats", 1.0, 8.0, 1.0).scale(Scale::Stepped),
        ParamSpec::new("arpCycle", 0.0, 6.0, 0.0).scale(Scale::Stepped),
        ParamSpec::new("arpSkip", 0.0, 100.0, 0.0).unit("%"),
        ParamSpec::new("arpMiss", 0.0, 100.0, 0.0).unit("%"),
        ParamSpec::new("arpTime", 25.0, 2000.0, 200.0).unit("ms"),
        ParamSpec::new("arpGate", 1.0, 200.0, 100.0).unit("%"),
        ParamSpec::new("glide", 0.0, 2000.0, 0.0).unit("ms"),
    ] {
        s = s.param(p);
    }
    for (k, c, d) in [
        ("filterType", FILTERS, "low-pass"),
        ("chord", CHORDS, "octave"),
        ("arpChord", CHORDS, "octave"),
        ("arpDirection", &["up", "down", "up-down", "down-up", "random"][..], "up"),
        ("arpMode", &["free", "sort", "sync"][..], "free"),
        ("arpSync", SYNC, "off"),
        ("voiceMode", &["poly", "mono", "legato"][..], "poly"),
    ] {
        s = s.field(FieldSpec::choice(k, c, d));
    }
    s
}

/// A fake plugin's category (F-FX-036), as the browser's entries carry it
pub fn category(id: &str) -> &'static str {
    match id {
        "tripleoscillator" => "synth",
        "avldrums" => "drums",
        "sampler" => "sampler",
        "compressor" | "limiter" => "dynamics",
        "eq8" => "eq",
        "reverb" | "delay" => "delay-reverb",
        _ => "other",
    }
}

/// A fake plugin's latency in samples (the look-ahead of the dynamics)
pub fn latency(id: &str) -> u32 {
    match id {
        "compressor" => 64,
        "limiter" => 240,
        _ => 0,
    }
}

/// The registry: the project's types and the fake plugins
pub fn registry() -> Result<SchemaRegistry> {
    registry_with(Vec::new())
}

/// The registry with real plugin schemas: each replaces the fake plugin of the same type (or is added)
pub fn registry_with(real: Vec<NodeSchema>) -> Result<SchemaRegistry> {
    let mut r = SchemaRegistry::new();
    register_project_schemas(&mut r)?;
    for (id, _, params) in PLUGINS {
        if real.iter().any(|s| s.type_name.strip_prefix("plugin:") == Some(*id)) {
            continue;
        }
        let mut s = NodeSchema::new(Box::leak(format!("plugin:{id}").into_boxed_str()), 1);
        for (k, min, max, d) in params.iter() {
            s = s.param(ParamSpec::new(k, *min, *max, *d));
        }
        if *id == "avldrums" {
            s = s.field(FieldSpec::choice("kit", KITS, "acoustic"));
        }
        if *id == "tripleoscillator" || *id == "sampler" {
            s = add_instrument_functions(s);
        }
        r.add(s)?;
    }
    for s in real {
        r.add(s)?;
    }
    r.check_complete()?;
    Ok(r)
}

/// A saved demo with the nodes of these plugins at their real schema's defaults: the real schemas do not know the
/// fake parameters
pub fn reset_plugins(saved: &Value, ids: &[String], registry: &SchemaRegistry) -> Result<Value> {
    let mut sites = Vec::new();
    saved.walk(&mut |path, v| {
        if v.get("state").is_some()
            && v.get("plugin").and_then(Value::as_text).is_some_and(|p| ids.iter().any(|i| i == p))
        {
            sites.push(path.to_vec());
        }
    });
    let mut out = saved.clone();
    for mut path in sites {
        let plugin = out.at_path(&path).and_then(|v| v.get("plugin")).and_then(Value::as_text).unwrap_or("").to_owned();
        let state = registry.defaults(&format!("plugin:{plugin}"))?;
        path.push(PathStep::key("state"));
        out = out.with_path(&path, state)?;
    }
    Ok(out)
}

/// The options with real plugins: each has one stereo output, like the fake ones
pub fn options_with(real: Vec<String>) -> Options {
    let ports = move |plugin: &str, state: &Value| {
        if real.iter().any(|r| r == plugin) {
            Some(Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![] })
        } else {
            self::ports(plugin, state)
        }
    };
    Options { ports: Some(Arc::new(ports)), ..Options::default() }
}

/// The fake plugins' ports (M1): one stereo output, or a drum kit's pieces
pub fn ports(plugin: &str, state: &Value) -> Option<Ports> {
    match plugin {
        "avldrums" => {
            let big = state.get("kit").and_then(Value::as_text) == Some("big");
            let mut outputs =
                vec![Port::new("kick", 1), Port::new("snare", 1), Port::new("oh", 2), Port::new("room", 2)];
            if big {
                outputs.insert(2, Port::new("tom", 1));
            }
            Some(Ports { outputs, sidechains: vec![] })
        }
        "compressor" => {
            Some(Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![Port::new("sidechain", 2)] })
        }
        p if PLUGINS.iter().any(|(id, ..)| *id == p) => {
            Some(Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![] })
        }
        _ => None,
    }
}

pub fn options() -> Options {
    Options { ports: Some(Arc::new(ports)), ..Options::default() }
}

/// A plugin node with every parameter at its default
pub fn plugin(id: &str) -> Value {
    let params = PLUGINS.iter().find(|(p, ..)| *p == id).map_or(&[][..], |(_, _, ps)| ps);
    let mut state = Value::object([(
        "params",
        Value::object(params.iter().map(|(k, _, _, d)| (*k, Value::real(*d).unwrap_or_default()))),
    )]);
    if id == "avldrums" {
        state = state.with("kit", Value::text("acoustic"));
    }
    Value::object([("plugin", Value::text(id)), ("state", state)])
}

/// A fake recording's resource: an "audio" resource whose key is the hash of a name
pub fn audio(name: &str) -> Resource {
    Resource::new("audio", &blake3::hash(name.as_bytes()).to_hex()).expect("a blake3 hex is a valid key")
}

pub const PPQ: i64 = 960;
/// Ticks per bar (4/4)
pub const BAR: i64 = 4 * PPQ;

const NAMES: [&str; 8] = ["Bateria", "Voz principal", "Backing", "Guitarra", "Baixo", "Teclado", "Sopros", "Percussão"];

fn k(s: &str) -> PathStep {
    PathStep::key(s)
}

fn int(v: i64) -> Value {
    Value::int(v)
}

fn real(v: f64) -> Value {
    Value::real(v).unwrap_or_default()
}

fn idv(id: NodeId) -> Value {
    Value::text(id.to_string())
}

fn ids_in(v: &Value, list: &str) -> Vec<NodeId> {
    v.get(list)
        .map(|l| {
            l.items().filter_map(|n| n.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok())).collect()
        })
        .unwrap_or_default()
}

/// The demo project: folders, a multi-output drum kit with its piece channels in a group, audio tracks with lanes and
/// a comp, MIDI items with notes, automation, two returns with sends, loop, punch and a tempo ramp
pub fn project(doc: &mut Document, tracks: usize) -> Result<()> {
    let l = |key: &'static str| Label::new(key);
    // the desk first: the drums' group, its four pieces, two returns, then a channel per track
    let mut channels = Vec::new();
    doc.transact(l("demo.desk"), |e| {
        let mut n = 0;
        let mut add = |e: &mut lm_doc::Edit<'_>, name: &str| -> Result<NodeId> {
            let id = e.insert(ROOT, "mixer.channels", n, Value::object([("name", Value::text(name))]))?;
            n += 1;
            Ok(id)
        };
        let group = add(e, "Bateria (peças)")?;
        for piece in ["Bumbo", "Caixa", "Overheads", "Sala"] {
            let c = add(e, piece)?;
            e.set(c, &[k("output")], idv(group))?;
            channels.push(c);
        }
        channels.insert(0, group);
        for (i, name) in ["Reverb", "Delay"].iter().enumerate() {
            let c = add(e, name)?;
            let fx = e.insert(c, "inserts", 0, plugin(if i == 0 { "reverb" } else { "delay" }))?;
            let _ = fx;
            channels.push(c);
        }
        for i in 1..tracks {
            let c = add(e, &format!("{} {}", NAMES[i % 8], i / 8 + 1))?;
            if i % 3 == 0 {
                e.insert(c, "inserts", 0, plugin("compressor"))?;
            }
            if i % 4 == 1 {
                e.insert(c, "inserts", 0, plugin("eq8"))?;
            }
            e.set(c, &[k("pan")], real([0.0, -0.3, 0.3, -0.6, 0.6][i % 5]))?;
            channels.push(c);
        }
        e.insert(lm_doc::schemas::MASTER, "inserts", 0, plugin("limiter"))?;
        Ok(())
    })?;
    let (reverb, delay) = (channels[5], channels[6]);
    // sends to the returns
    doc.transact(l("demo.sends"), |e| {
        for (i, c) in channels.iter().enumerate().skip(7) {
            if i % 2 == 0 {
                e.insert(*c, "sends", 0, Value::object([("to", idv(reverb)), ("amount", real(0.25))]))?;
            }
            if i % 3 == 0 {
                e.insert(
                    *c,
                    "sends",
                    0,
                    Value::object([("to", idv(delay)), ("amount", real(0.15)), ("pre", Value::bool(true))]),
                )?;
            }
        }
        Ok(())
    })?;
    // the tracks
    doc.transact(l("demo.tracks"), |e| {
        let mut folder: Option<(NodeId, usize)> = None;
        for i in 0..tracks {
            let is_folder = i % 10 == 0;
            let instrument = i == 1 || (i % 3 == 0 && !is_folder);
            let name = match i {
                0 => "Bateria".to_string(),
                1 => "Bateria AVL".to_string(),
                2 => "Voz principal".to_string(),
                _ if is_folder => format!("Grupo {}", i / 10 + 1),
                _ => format!("{} {}", NAMES[i % 8], i / 8 + 1),
            };
            let mut node = Value::object([("name", Value::text(name)), ("folder", Value::bool(is_folder))]);
            if instrument {
                node = node.with("instrument", plugin(if i == 1 { "avldrums" } else { "tripleoscillator" }));
            }
            if !instrument && !is_folder {
                node = node.with(
                    "input",
                    Value::object([
                        ("device", Value::text("in")),
                        ("channels", Value::list([int((i % 8) as i64 + 1)])),
                    ]),
                );
            }
            if i == 2 {
                node = node.with("armed", Value::bool(true)).with("monitor", Value::text("on"));
            }
            let t = e.insert(ROOT, "tracks", i, node)?;
            match folder {
                Some((f, left)) if left > 0 && !is_folder => {
                    e.set(t, &[k("parent")], idv(f))?;
                    folder = Some((f, left - 1));
                }
                _ => folder = None,
            }
            if is_folder {
                folder = Some((t, 3));
                continue;
            }
            // outputs: the drum pieces to their channels; every other track's main output to its channel
            let outs = ids_in(e.find(t).expect("just inserted"), "outputs");
            if i == 1 {
                for (o, c) in outs.iter().zip(channels[1..5].iter()) {
                    e.set(*o, &[k("to")], idv(*c))?;
                }
            } else if let Some(o) = outs.first() {
                let c = channels.get(6 + i).copied().unwrap_or(lm_doc::schemas::MASTER);
                e.set(*o, &[k("to")], idv(c))?;
            }
            let start = ((i % 5) as i64) * BAR / 2;
            if instrument {
                for part in 0..3 {
                    let it = e.insert(
                        t,
                        "items",
                        part,
                        Value::object([
                            ("kind", Value::text("midi")),
                            (
                                "name",
                                Value::text(if part == 0 { "Padrão A".to_string() } else { format!("Frase {part}") }),
                            ),
                            ("start", int(start + part as i64 * 12 * BAR)),
                            ("length", int(4 * BAR)),
                        ]),
                    )?;
                    let step = BAR / 8;
                    for s in 0..32i64 {
                        let key = match (i + part) % 3 {
                            0 => [36, 38, 42, 42][(s % 4) as usize],
                            1 => [60, 64, 67, 72][(s % 4) as usize],
                            _ => [40, 40, 43, 45, 47, 45, 43, 38][(s % 8) as usize],
                        };
                        e.add_note(
                            it,
                            Value::object([
                                ("key", int(key)),
                                ("start", int(s * step)),
                                ("length", int(step - 60)),
                                ("velocity", real(0.6 + 0.1 * (s % 4) as f64)),
                            ]),
                        )?;
                    }
                }
            } else {
                for part in 0..2 {
                    let length = 24 * BAR;
                    let it = e.insert(
                        t,
                        "items",
                        part,
                        Value::object([
                            ("kind", Value::text("audio")),
                            ("name", Value::text(format!("Take {i}{}", if part == 1 { "b" } else { "" }))),
                            ("start", int(start + part as i64 * (length + 2 * BAR))),
                            ("length", int(length)),
                            ("timebase", Value::text("audio")),
                        ]),
                    )?;
                    let takes = if i == 2 && part == 0 { 3 } else { 1 };
                    let mut lanes = Vec::new();
                    for tk in 0..takes {
                        let lane = e.insert(
                            it,
                            "lanes",
                            tk,
                            Value::object([
                                ("source", Value::resource(audio(&format!("take-{}-{tk}", i % 8 + tk)))),
                                ("name", Value::text(format!("Take {}", tk + 1))),
                            ]),
                        )?;
                        lanes.push(lane);
                    }
                    let cuts: &[(i64, i64, usize)] = if takes == 3 {
                        &[(0, 6 * BAR, 0), (6 * BAR, 14 * BAR, 1), (14 * BAR, 24 * BAR, 2)]
                    } else {
                        &[(0, 24 * BAR, 0)]
                    };
                    // the comp's segments have no id: the list is set whole
                    let comp = Value::list(cuts.iter().map(|(from, to, lane)| {
                        Value::object([("from", int(*from)), ("to", int(*to)), ("lane", idv(lanes[*lane]))])
                    }));
                    e.set(it, &[k("comp")], comp)?;
                }
            }
            if i == 2 || i == 4 {
                let target = Value::object([("node", idv(t)), ("param", Value::text("volume"))]);
                let points = Value::list((0..24).map(|p| {
                    Value::object([
                        ("tick", int(p * BAR)),
                        ("value", real(0.6 + 0.35 * ((p as f64) * 0.7).sin())),
                        ("curve", real(if p % 3 == 0 { 0.5 } else { 0.0 })),
                    ])
                }));
                e.insert(t, "automation", 0, Value::object([("target", target), ("points", points)]))?;
            }
        }
        Ok(())
    })?;
    // the song: loop, punch, pre-roll, metronome, a tempo ramp
    doc.transact(l("demo.song"), |e| {
        let tr = |e: &mut lm_doc::Edit<'_>, path: &[&str], v: Value| {
            e.set(ROOT, &path.iter().map(|s| k(s)).collect::<Vec<_>>(), v)
        };
        tr(e, &["transport", "loop", "from"], int(4 * BAR))?;
        tr(e, &["transport", "loop", "to"], int(12 * BAR))?;
        tr(e, &["transport", "punch", "from"], int(6 * BAR))?;
        tr(e, &["transport", "punch", "to"], int(8 * BAR))?;
        tr(e, &["transport", "metronome", "on"], Value::bool(true))?;
        tr(e, &["transport", "metronome", "countIn"], int(1))?;
        let point = |tick: i64, bpm: &str, ramp: bool| {
            Value::object([
                ("tick", int(tick)),
                ("bpm", Value::text(bpm)),
                ("timeSig", Value::list([int(4), int(4)])),
                ("ramp", Value::bool(ramp)),
            ])
        };
        tr(
            e,
            &["tempoMap"],
            Value::list([
                point(0, "120", false),
                point(16 * BAR, "120", true),
                point(24 * BAR, "132", false),
                point(40 * BAR, "96", false),
            ]),
        )?;
        Ok(())
    })?;
    // the ruler's markers and regions (F-REG-011, F-REG-012)
    doc.transact(l("demo.marks"), |e| {
        for (i, (name, bar)) in [("Intro", 0), ("Verso", 8), ("Refrão", 24), ("Ponte", 40)].iter().enumerate() {
            e.insert(ROOT, "markers", i, Value::object([("name", Value::text(*name)), ("tick", int(bar * BAR))]))?;
        }
        for (i, (name, from, to)) in [("Verso 1", 8, 16), ("Refrão 1", 24, 32)].iter().enumerate() {
            e.insert(
                ROOT,
                "regions",
                i,
                Value::object([("name", Value::text(*name)), ("from", int(from * BAR)), ("to", int(to * BAR))]),
            )?;
        }
        Ok(())
    })?;
    Ok(())
}
