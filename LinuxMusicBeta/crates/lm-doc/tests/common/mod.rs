//! What the tests share: a registry with the project's types and fake plugins, a ports callback, a seeded generator
//! and builders

#![allow(dead_code)]

use std::sync::Arc;

use lm_base::Label;
use lm_doc::{Document, Options, Port, Ports, register_project_schemas};
use lm_state::{FieldSpec, NodeId, NodeSchema, ParamSpec, PathStep, SchemaRegistry, Value};

/// SplitMix64 (the workspace takes no test-only crates)
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(std::env::var("LM_TEST_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(seed))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % (n.max(1) as u64)) as usize
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() { None } else { Some(&items[self.below(items.len())]) }
    }
}

pub const KITS: &[&str] = &["small", "big"];

/// The project's types and three fake plugins: a synth (one stereo output), a drum kit (ports by kit) and a
/// compressor with a sidechain input
pub fn registry_with(version_of_synth: u32) -> SchemaRegistry {
    let mut r = SchemaRegistry::new();
    register_project_schemas(&mut r).unwrap();
    let mut synth = NodeSchema::new("plugin:synth", version_of_synth)
        .param(ParamSpec::new("vol1", 0.0, 1.0, 0.8).aliases(&["vol_1"]))
        .param(ParamSpec::new("cutoff", 20.0, 20000.0, 1000.0));
    if version_of_synth == 2 {
        synth = synth.field(FieldSpec::text("wave", "saw")).upgrade(1, |v| Ok(v.with("wave", Value::text("square"))));
    }
    r.add(synth).unwrap();
    r.add(
        NodeSchema::new("plugin:drums", 1)
            .field(FieldSpec::choice("kit", KITS, "small"))
            .param(ParamSpec::new("level", 0.0, 1.0, 1.0)),
    )
    .unwrap();
    r.add(NodeSchema::new("plugin:comp", 1).param(ParamSpec::new("threshold", -60.0, 0.0, -20.0))).unwrap();
    r.check_complete().unwrap();
    r
}

pub fn registry() -> Arc<SchemaRegistry> {
    Arc::new(registry_with(1))
}

pub fn fake_ports(plugin: &str, state: &Value) -> Option<Ports> {
    match plugin {
        "synth" => Some(Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![] }),
        "comp" => Some(Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![Port::new("sidechain", 2)] }),
        "drums" => {
            let kit = state.get("kit").and_then(Value::as_text).unwrap_or("small");
            let outputs = match kit {
                "big" => vec![
                    Port::new("kick", 1),
                    Port::new("snare", 1),
                    Port::new("tom", 1),
                    Port::new("oh", 1),
                    Port::new("room", 2),
                ],
                _ => vec![Port::new("kick", 1), Port::new("snare", 1), Port::new("oh", 2)],
            };
            Some(Ports { outputs, sidechains: vec![] })
        }
        _ => None,
    }
}

pub fn options() -> Options {
    Options { ports: Some(Arc::new(fake_ports)), ..Options::default() }
}

pub fn new_doc() -> Document {
    Document::new_project(registry(), options()).unwrap()
}

pub fn label(key: &'static str) -> Label {
    Label::new(key)
}

pub fn k(key: &str) -> PathStep {
    PathStep::key(key)
}

pub fn real(v: f64) -> Value {
    Value::real(v).unwrap()
}

pub fn text(t: &str) -> Value {
    Value::text(t)
}

pub fn id_text(id: NodeId) -> Value {
    Value::text(id.to_string())
}

pub fn plugin(name: &str, state: Value) -> Value {
    Value::object([("plugin", text(name)), ("state", state)])
}

pub fn synth() -> Value {
    plugin("synth", Value::object([("params", Value::object([("vol1", real(0.8)), ("cutoff", real(1000.0))]))]))
}

pub fn drums(kit: &str) -> Value {
    plugin("drums", Value::object([("kit", text(kit)), ("params", Value::object([("level", real(1.0))]))]))
}

pub fn comp() -> Value {
    plugin("comp", Value::object([("params", Value::object([("threshold", real(-20.0))]))]))
}

/// A track at the end, optionally with an instrument
pub fn add_track(doc: &mut Document, name: &str, instrument: Option<Value>) -> NodeId {
    let mut out = None;
    doc.transact(label("test.add_track"), |e| {
        let n = e.root().get("tracks").map_or(0, Value::len);
        let mut node = Value::object([("name", text(name))]);
        if let Some(i) = instrument {
            node = node.with("instrument", i);
        }
        out = Some(e.insert(lm_state::ROOT, "tracks", n, node)?);
        Ok(())
    })
    .unwrap();
    out.unwrap()
}

/// A desk channel at the end
pub fn add_channel(doc: &mut Document, name: &str) -> NodeId {
    let mut out = None;
    doc.transact(label("test.add_channel"), |e| {
        let n = e.root().get("mixer").and_then(|m| m.get("channels")).map_or(0, Value::len);
        out = Some(e.insert(lm_state::ROOT, "mixer.channels", n, Value::object([("name", text(name))]))?);
        Ok(())
    })
    .unwrap();
    out.unwrap()
}

pub fn add_item(doc: &mut Document, track: NodeId, kind: &str) -> NodeId {
    let mut out = None;
    doc.transact(label("test.add_item"), |e| {
        let n = e.find(track).and_then(|t| t.get("items")).map_or(0, Value::len);
        out = Some(e.insert(track, "items", n, Value::object([("kind", text(kind))]))?);
        Ok(())
    })
    .unwrap();
    out.unwrap()
}

/// The root without `meta` (which undo never restores)
pub fn content(doc: &Document) -> Value {
    doc.root().without("meta")
}

pub fn first_output(doc: &Document, track: NodeId) -> NodeId {
    let t = doc.find(track).unwrap();
    NodeId::parse(t.get("outputs").unwrap().at(0).unwrap().get("id").unwrap().as_text().unwrap()).unwrap()
}

pub fn outputs(doc: &Document, track: NodeId) -> Vec<(String, i64, String)> {
    let t = doc.find(track).unwrap();
    t.get("outputs")
        .unwrap()
        .items()
        .map(|o| {
            (
                o.get("port").unwrap().as_text().unwrap().to_owned(),
                o.get("channels").unwrap().as_int().unwrap(),
                o.get("to").unwrap().as_text().unwrap().to_owned(),
            )
        })
        .collect()
}
