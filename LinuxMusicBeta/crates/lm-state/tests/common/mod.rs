//! A registry shaped like the project of arquitetura-nova.md (piece 2), for the tests

#![allow(dead_code)]

pub mod rng;

use lm_base::{Error, Result};
use lm_state::{FieldKind, FieldSpec, NodeId, NodeSchema, OnRemove, ParamSpec, Rate, Scale, SchemaRegistry, Value};

pub const MASTER: NodeId = NodeId { kind: b'm', number: 0 };
pub const HASH: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

/// tripleoscillator v1 → v2: "vol" was split into "vol1" (the old value) — written by the migration
fn osc_v1_to_v2(node: Value) -> Result<Value> {
    let params = node.get("params").cloned().unwrap_or_else(Value::empty_object);
    let vol = params.get("vol").cloned().unwrap_or(Value::real(1.0)?);
    Ok(node.with("params", params.without("vol").with("vol1", vol)))
}

/// tripleoscillator v2 → v3: the waveform was a number (refused by U30); now a choice key
fn osc_v2_to_v3(node: Value) -> Result<Value> {
    let wave = match node.get("wave").and_then(Value::as_int) {
        None | Some(0) => "sine",
        Some(1) => "saw",
        Some(2) => "square",
        Some(n) => return Err(Error::new("test.osc.unknown_wave").arg("wave", n)),
    };
    Ok(node.with("wave", Value::text(wave)))
}

pub fn registry() -> SchemaRegistry {
    let mut r = SchemaRegistry::new();
    let add = |r: &mut SchemaRegistry, s| r.add(s).unwrap();
    add(
        &mut r,
        NodeSchema::new("project", 1)
            .field(FieldSpec::int("sampleRate", 8000, 384_000, 48000))
            .field(FieldSpec::int("ppq", 1, 100_000, 960))
            .field(FieldSpec::int("seed", 0, lm_state::MAX_SAFE_INT, 271_828)),
    );
    add(
        &mut r,
        NodeSchema::new("note", 1)
            .field(FieldSpec::int("key", 0, 127, 60))
            .field(FieldSpec::int("start", 0, lm_state::MAX_SAFE_INT, 0))
            .field(FieldSpec::int("length", 1, lm_state::MAX_SAFE_INT, 480))
            .field(FieldSpec::real("velocity", 0.0, 1.0, 0.8)),
    );
    add(
        &mut r,
        NodeSchema::new("item.midi", 1)
            .field(FieldSpec::id(b'c'))
            .field(FieldSpec::choice("kind", &["midi"], "midi"))
            .field(FieldSpec::int("start", 0, lm_state::MAX_SAFE_INT, 0))
            .field(FieldSpec::int("length", 1, lm_state::MAX_SAFE_INT, 3840))
            .field(FieldSpec::new("notes", FieldKind::NoteMap("note"))),
    );
    add(
        &mut r,
        NodeSchema::new("lane", 1)
            .field(FieldSpec::id(b'l'))
            .field(FieldSpec::new("source", FieldKind::Resource("audio")))
            .field(FieldSpec::int("sourceOffset", 0, lm_state::MAX_SAFE_INT, 0)),
    );
    add(
        &mut r,
        NodeSchema::new("output", 1)
            .field(FieldSpec::id(b'o'))
            .field(FieldSpec::text("port", "main"))
            .field(FieldSpec::int("channels", 1, 64, 2))
            .field(FieldSpec::new("to", FieldKind::Ref { to: b"m", on_remove: OnRemove::Redirect(MASTER) })),
    );
    add(
        &mut r,
        NodeSchema::new("plugin:tripleoscillator", 3)
            .field(FieldSpec::choice("wave", &["sine", "saw", "square"], "sine"))
            .field(FieldSpec::new("chunk", FieldKind::Any).optional())
            .param(ParamSpec::new("vol1", 0.0, 2.0, 1.0).aliases(&["volume1"]).rate(Rate::Sample))
            .param(ParamSpec::new("cutoff", 20.0, 20000.0, 1000.0).scale(Scale::Log).unit("Hz"))
            .param(ParamSpec::new("octave", -4.0, 4.0, 0.0).scale(Scale::Stepped).not_automatable())
            .upgrade(1, osc_v1_to_v2)
            .upgrade(2, osc_v2_to_v3),
    );
    add(
        &mut r,
        NodeSchema::new("track", 1)
            .field(FieldSpec::id(b't'))
            .field(FieldSpec::text("name", ""))
            .field(
                FieldSpec::new("parent", FieldKind::Ref { to: b"t", on_remove: OnRemove::Clear })
                    .nullable()
                    .default(Value::NULL),
            )
            .field(FieldSpec::real("volume", 0.0, 2.0, 1.0))
            .field(FieldSpec::bool("mute", false))
            .field(FieldSpec::bool("solo", false))
            .field(FieldSpec::new("outputs", FieldKind::List(Box::new(FieldKind::Object("output")))))
            .field(FieldSpec::new("items", FieldKind::List(Box::new(FieldKind::Object("item.midi")))))
            .field(FieldSpec::new("lanes", FieldKind::List(Box::new(FieldKind::Object("lane")))).optional())
            .field(
                FieldSpec::new(
                    "follows",
                    FieldKind::List(Box::new(FieldKind::Ref { to: b"t", on_remove: OnRemove::Clear })),
                )
                .optional(),
            ),
    );
    add(
        &mut r,
        NodeSchema::new("send", 1)
            .field(FieldSpec::id(b's'))
            .field(FieldSpec::new("to", FieldKind::Ref { to: b"m", on_remove: OnRemove::RemoveOwner }))
            .field(FieldSpec::real("amount", 0.0, 1.0, 0.0))
            .field(FieldSpec::bool("pre", false)),
    );
    add(
        &mut r,
        NodeSchema::new("channel", 1)
            .field(FieldSpec::id(b'm'))
            .field(FieldSpec::real("pan", -1.0, 1.0, 0.0))
            .field(FieldSpec::new("sends", FieldKind::List(Box::new(FieldKind::Object("send")))))
            .field(
                FieldSpec::new("output", FieldKind::Ref { to: b"m", on_remove: OnRemove::Refuse })
                    .default(Value::text("m:0")),
            ),
    );
    r.check_complete().unwrap();
    r
}

pub fn track() -> Value {
    lm_state::from_json(
        r#"{
          "id": "t:17", "name": "Baixo", "parent": null, "volume": 0.8, "mute": false, "solo": false,
          "outputs": [ { "id": "o:20", "port": "main", "channels": 2, "to": "m:3" } ],
          "items": [ { "id": "c:52", "kind": "midi", "start": 0, "length": 3840,
                       "notes": { "812": { "key": 57, "start": 0, "length": 480, "velocity": 0.79 } } } ]
        }"#,
    )
    .unwrap()
}
