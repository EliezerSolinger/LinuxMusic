//! Any bytes into `from_json_bytes`, then every validation path of a project-shaped registry. Nothing may panic, and
//! whatever is read must write back and read back to the same tree.
#![no_main]

use libfuzzer_sys::fuzz_target;
use lm_state::{
    FieldKind, FieldSpec, NodeId, NodeSchema, OnRemove, ParamSpec, Scale, SchemaRegistry, Value, from_json,
    from_json_bytes, to_json,
};
use std::sync::OnceLock;

fn registry() -> &'static SchemaRegistry {
    static REGISTRY: OnceLock<SchemaRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut r = SchemaRegistry::new();
        let master = NodeId { kind: b'm', number: 0 };
        let schemas = [
            NodeSchema::new("note", 1)
                .field(FieldSpec::int("key", 0, 127, 60))
                .field(FieldSpec::int("start", 0, lm_state::MAX_SAFE_INT, 0))
                .field(FieldSpec::real("velocity", 0.0, 1.0, 0.8)),
            NodeSchema::new("item", 1)
                .field(FieldSpec::id(b'c'))
                .field(FieldSpec::new("notes", FieldKind::NoteMap("note"))),
            NodeSchema::new("output", 1)
                .field(FieldSpec::id(b'o'))
                .field(FieldSpec::new("to", FieldKind::Ref { to: b"m", on_remove: OnRemove::Redirect(master) })),
            NodeSchema::new("track", 1)
                .field(FieldSpec::id(b't'))
                .field(FieldSpec::text("name", ""))
                .field(FieldSpec::new("parent", FieldKind::Ref { to: b"t", on_remove: OnRemove::Clear }).nullable())
                .field(FieldSpec::real("volume", 0.0, 2.0, 1.0))
                .field(FieldSpec::new("source", FieldKind::Resource("audio")).optional())
                .field(FieldSpec::new("outputs", FieldKind::List(Box::new(FieldKind::Object("output")))))
                .field(FieldSpec::new("items", FieldKind::List(Box::new(FieldKind::Object("item"))))),
            NodeSchema::new("plugin", 1)
                .field(FieldSpec::choice("wave", &["sine", "saw"], "sine"))
                .field(FieldSpec::new("chunk", FieldKind::Any).optional())
                .param(ParamSpec::new("vol1", 0.0, 2.0, 1.0).aliases(&["volume1"]))
                .param(ParamSpec::new("octave", -4.0, 4.0, 0.0).scale(Scale::Stepped)),
        ];
        for s in schemas {
            r.add(s).expect("valid schema");
        }
        r.check_complete().expect("complete registry");
        r
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(v) = from_json_bytes(data) else { return };
    let r = registry();
    for t in ["note", "item", "output", "track", "plugin"] {
        let _ = r.validate(t, &v);
        let _ = r.refs(t, &v);
        let _ = r.ids(t, &v);
        let _ = r.resources(t, &v);
        let _ = r.load(t, 1, v.clone());
    }
    let json = to_json(&v).expect("what was read can be written");
    let back: Value = from_json(&json).expect("what was written can be read");
    assert_eq!(back, v);
});
