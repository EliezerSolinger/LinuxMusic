//! Schemas: validation names the path and the field, upgrades chain, defaults, references, frozen defaults (U29)

mod common;

use common::{HASH, MASTER, registry, track};
use lm_state::frozen::compare_or_bless;
use lm_state::{
    FieldKind, FieldSpec, NodeId, NodeSchema, OnRemove, ParamSpec, PathStep, Resource, Scale, SchemaRegistry, Value,
    from_json, path_text,
};

fn k(key: &str) -> PathStep {
    PathStep::key(key)
}

fn real(v: f64) -> Value {
    Value::real(v).unwrap()
}

fn error_text(type_name: &str, node: &Value) -> (String, String) {
    let e = registry().validate(type_name, node).unwrap_err();
    (e.key().to_owned(), e.to_string())
}

#[test]
fn a_valid_track_validates() {
    registry().validate("track", &track()).unwrap();
}

#[test]
fn errors_name_the_path_and_the_field() {
    let t = track();
    let cases: Vec<(Vec<PathStep>, Value, &str, &str, &str)> = vec![
        (vec![k("volume")], real(2.5), "state.schema.out_of_range", "/volume", "volume"),
        (vec![k("volume")], Value::int(1), "state.schema.wrong_kind", "/volume", "volume"),
        (
            vec![k("outputs"), PathStep::Index(0), k("to")],
            Value::text("t:3"),
            "state.schema.wrong_ref_kind",
            "/outputs/0/to",
            "to",
        ),
        (
            vec![k("outputs"), PathStep::Index(0), k("to")],
            Value::text("m:03"),
            "state.schema.invalid_id",
            "/outputs/0/to",
            "to",
        ),
        (
            vec![k("items"), PathStep::Index(0), k("notes"), PathStep::Note(812), k("key")],
            Value::int(128),
            "state.schema.out_of_range",
            "/items/0/notes/812/key",
            "key",
        ),
        (
            vec![k("items"), PathStep::Index(0), k("notes"), PathStep::Note(812), k("velocity")],
            real(-0.5),
            "state.schema.out_of_range",
            "/items/0/notes/812/velocity",
            "velocity",
        ),
        (
            vec![k("items"), PathStep::Index(0), k("kind")],
            Value::text("audio"),
            "state.schema.not_a_choice",
            "/items/0/kind",
            "kind",
        ),
        (vec![k("id")], Value::text("c:17"), "state.schema.wrong_id_kind", "/id", "id"),
        (vec![k("mute")], Value::NULL, "state.schema.wrong_kind", "/mute", "mute"),
        (vec![k("surprise")], Value::int(1), "state.schema.unknown_field", "/surprise", "surprise"),
        (vec![k("name")], Value::text("x".repeat(64 * 1024 + 1)), "state.schema.text_too_large", "/name", "name"),
    ];
    for (path, v, key, at, field) in cases {
        let (got_key, text) = error_text("track", &t.with_path(&path, v).unwrap());
        assert_eq!(got_key, key, "{text}");
        assert!(text.contains(&format!("path=\"{at}\"")), "{text}");
        assert!(text.contains(&format!("field=\"{field}\"")), "{text}");
    }
    let (key, text) = error_text("track", &t.without("items"));
    assert_eq!(key, "state.schema.missing_field");
    assert!(text.contains("path=\"/items\""), "{text}");
    let bad_note_key =
        t.with_path(&[k("items"), PathStep::Index(0), k("notes"), k("0812")], Value::empty_object()).unwrap();
    assert_eq!(error_text("track", &bad_note_key).0, "state.schema.note_key");
    assert_eq!(error_text("track", &Value::int(1)).0, "state.schema.not_object");
    assert_eq!(registry().validate("nope", &t).unwrap_err().key(), "state.schema.unknown_type");
}

#[test]
fn validate_at_prefixes_the_path() {
    let t = track().with("volume", real(9.0));
    let e = registry().validate_at("track", &t, &[k("tracks"), PathStep::Index(3)]).unwrap_err();
    assert!(e.to_string().contains("path=\"/tracks/3/volume\""), "{e}");
}

#[test]
fn nullable_and_optional_fields() {
    let r = registry();
    let t = track();
    r.validate("track", &t.with("parent", Value::text("t:1"))).unwrap();
    r.validate("track", &t.without("parent")).unwrap_err();
    r.validate("track", &t.with("follows", Value::list([Value::text("t:2")]))).unwrap();
    // Items of a list are never null
    assert_eq!(
        r.validate("track", &t.with("follows", Value::list([Value::NULL]))).unwrap_err().key(),
        "state.schema.wrong_kind"
    );
}

#[test]
fn resources_are_checked_by_kind_and_never_paths() {
    let r = registry();
    let lane = |source: Value| {
        Value::object([("id", Value::text("l:61")), ("source", source), ("sourceOffset", Value::int(0))])
    };
    r.validate("lane", &lane(Value::resource(Resource::new("audio", HASH).unwrap()))).unwrap();
    assert_eq!(
        r.validate("lane", &lane(Value::resource(Resource::new("state", HASH).unwrap()))).unwrap_err().key(),
        "state.schema.wrong_resource_kind"
    );
    // A project from outside naming a path: the JSON reader gives an ordinary object, refused here (C9)
    let evil =
        from_json(r#"{"id": "l:61", "sourceOffset": 0, "source": {"kind": "audio", "key": "../../x"}}"#).unwrap();
    assert_eq!(r.validate("lane", &evil).unwrap_err().key(), "state.schema.wrong_kind");
    let abs =
        from_json(r#"{"id": "l:61", "sourceOffset": 0, "source": {"kind": "audio", "key": "/etc/passwd"}}"#).unwrap();
    assert_eq!(r.validate("lane", &abs).unwrap_err().key(), "state.schema.wrong_kind");
}

#[test]
fn params_are_checked_and_aliases_resolved() {
    let r = registry();
    let osc = r.defaults("plugin:tripleoscillator").unwrap();
    r.validate("plugin:tripleoscillator", &osc).unwrap();
    let set = |key: &str, v: Value| osc.with_path(&[k("params"), k(key)], v).unwrap();
    r.validate("plugin:tripleoscillator", &set("octave", real(-4.0))).unwrap();
    for (key, v, err) in [
        ("vol1", real(2.1), "state.schema.param_out_of_range"),
        ("octave", real(0.5), "state.schema.param_out_of_range"),
        ("vol1", Value::int(1), "state.schema.wrong_kind"),
        ("nope", real(0.0), "state.schema.unknown_param"),
        ("volume1", real(0.5), "state.schema.param_alias"),
    ] {
        let e = r.validate("plugin:tripleoscillator", &set(key, v)).unwrap_err();
        assert_eq!(e.key(), err);
        assert!(e.to_string().contains(&format!("path=\"/params/{key}\"")), "{e}");
    }
    // Whoever points by the old key resolves to the parameter (C5)
    assert_eq!(r.resolve_param("plugin:tripleoscillator", "volume1").unwrap().key, "vol1");
    assert_eq!(r.resolve_param("plugin:tripleoscillator", "vol1").unwrap().key, "vol1");
    assert!(r.resolve_param("plugin:tripleoscillator", "x").is_none());
    // And the node's own params are rewritten by the upgrade, even at the current version
    let old_key = osc.with("params", Value::object([("volume1", real(0.5))]));
    let up = r.load("plugin:tripleoscillator", 3, old_key).unwrap();
    assert_eq!(up.at_path(&[k("params"), k("vol1")]), Some(&real(0.5)));
    assert!(up.at_path(&[k("params"), k("volume1")]).is_none());
    let both = osc.with("params", Value::object([("volume1", real(0.5)), ("vol1", real(0.4))]));
    assert_eq!(r.upgrade("plugin:tripleoscillator", 3, both).unwrap_err().key(), "state.schema.param_alias_conflict");
}

#[test]
fn upgrades_chain_from_any_supported_version() {
    let r = registry();
    let v1 = from_json(r#"{"wave": 1, "params": {"vol": 0.25}}"#).unwrap();
    let v3 = r.load("plugin:tripleoscillator", 1, v1).unwrap();
    assert_eq!(v3.get("wave"), Some(&Value::text("saw")));
    assert_eq!(v3.at_path(&[k("params"), k("vol1")]), Some(&real(0.25)));
    let v2 = from_json(r#"{"wave": 2, "params": {"vol1": 0.5}}"#).unwrap();
    assert_eq!(r.load("plugin:tripleoscillator", 2, v2).unwrap().get("wave"), Some(&Value::text("square")));
    // An upgrade's own error passes through, with the type and the step
    let e = r.upgrade("plugin:tripleoscillator", 2, from_json(r#"{"wave": 7}"#).unwrap()).unwrap_err();
    assert_eq!(e.key(), "test.osc.unknown_wave");
    assert!(e.to_string().contains("from=2"), "{e}");
    // A newer version opens read-only (H11); an unknown older one is refused
    let e = r.upgrade("plugin:tripleoscillator", 4, Value::empty_object()).unwrap_err();
    assert_eq!(e.key(), "state.schema.newer_version");
    assert_eq!(
        r.upgrade("plugin:tripleoscillator", 0, Value::empty_object()).unwrap_err().key(),
        "state.schema.version_too_old"
    );
    assert_eq!(r.upgrade("track", 1, track()).unwrap(), track());
    // An upgraded node that is still wrong is caught by load
    let e = r.load("plugin:tripleoscillator", 2, from_json(r#"{"wave": 0, "params": {"vol1": 9.0}}"#).unwrap());
    assert_eq!(e.unwrap_err().key(), "state.schema.param_out_of_range");
}

#[test]
fn defaults_fill_every_field_but_the_id() {
    let r = registry();
    let t = r.defaults("track").unwrap();
    assert_eq!(t.get("id"), None);
    assert_eq!(t.get("volume"), Some(&real(1.0)));
    assert_eq!(t.get("parent"), Some(&Value::NULL));
    assert_eq!(t.get("items"), Some(&Value::empty_list()));
    assert_eq!(t.get("lanes"), None, "optional without default");
    let item = r.defaults("item.midi").unwrap();
    assert_eq!(item.get("notes"), Some(&Value::empty_object()));
    let osc = r.defaults("plugin:tripleoscillator").unwrap();
    assert_eq!(osc.at_path(&[k("params"), k("cutoff")]), Some(&real(1000.0)));
    // With an id, a default node is valid
    r.validate("track", &t.with("id", Value::text("t:1"))).unwrap();
}

#[test]
fn refs_ids_and_resources_are_listed_for_the_document() {
    let r = registry();
    let t = track()
        .with("parent", Value::text("t:2"))
        .with("follows", Value::list([Value::text("t:4"), Value::text("t:5")]));
    let refs = r.refs("track", &t).unwrap();
    let summary: Vec<_> = refs.iter().map(|s| (path_text(&s.path), s.field, s.on_remove, s.target)).collect();
    assert_eq!(
        summary,
        [
            ("/parent".to_owned(), "parent", OnRemove::Clear, Some(NodeId { kind: b't', number: 2 })),
            ("/outputs/0/to".to_owned(), "to", OnRemove::Redirect(MASTER), Some(NodeId { kind: b'm', number: 3 })),
            ("/follows/0".to_owned(), "follows", OnRemove::Clear, Some(NodeId { kind: b't', number: 4 })),
            ("/follows/1".to_owned(), "follows", OnRemove::Clear, Some(NodeId { kind: b't', number: 5 })),
        ]
    );
    assert_eq!(refs.len(), 4);
    assert_eq!(r.refs("track", &track()).unwrap().iter().filter(|s| s.target.is_none()).count(), 1);
    let ids: Vec<_> = r.ids("track", &t).unwrap().into_iter().map(|(p, id)| (path_text(&p), id.to_string())).collect();
    assert_eq!(
        ids,
        [
            ("/id".to_owned(), "t:17".to_owned()),
            ("/outputs/0/id".to_owned(), "o:20".to_owned()),
            ("/items/0/id".to_owned(), "c:52".to_owned())
        ]
    );
    let audio = Resource::new("audio", HASH).unwrap();
    let lane = Value::object([
        ("id", Value::text("l:61")),
        ("source", Value::resource(audio.clone())),
        ("sourceOffset", Value::int(0)),
    ]);
    let with_lane = t.with("lanes", Value::list([lane]));
    let res = r.resources("track", &with_lane).unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(path_text(&res[0].0), "/lanes/0/source");
    assert_eq!(res[0].1, audio);
}

#[test]
fn bad_definitions_are_refused() {
    let problem = |s: NodeSchema| {
        let e = SchemaRegistry::new().add(s).unwrap_err();
        assert_eq!(e.key(), "state.schema.definition");
        e.to_string()
    };
    let p = |s: NodeSchema, what: &str| {
        let text = problem(s);
        assert!(text.contains(&format!("problem=\"{what}\"")), "{text}");
    };
    p(NodeSchema::new("Track", 1), "type_name");
    p(NodeSchema::new("t", 0), "version_zero");
    p(NodeSchema::new("t", 1).field(FieldSpec::bool("a", true)).field(FieldSpec::bool("a", true)), "duplicate_field");
    p(NodeSchema::new("t", 1).field(FieldSpec::bool("a-b", true)), "field_key");
    p(NodeSchema::new("t", 1).field(FieldSpec::int("a", 0, 10, 11)), "default");
    p(NodeSchema::new("t", 1).field(FieldSpec::real("a", 0.0, 1.0, f64::NAN)), "default");
    p(NodeSchema::new("t", 1).field(FieldSpec::int("a", 0, 1 << 53, 0)), "int_range");
    p(NodeSchema::new("t", 1).field(FieldSpec::real("a", 1.0, 0.0, 0.5)), "real_range");
    p(NodeSchema::new("t", 1).field(FieldSpec::choice("a", &[], "x")), "choices");
    p(NodeSchema::new("t", 1).field(FieldSpec::choice("a", &["x", "x"], "x")), "choices");
    p(NodeSchema::new("t", 1).field(FieldSpec::choice("a", &["x", "y"], "z")), "default");
    p(NodeSchema::new("t", 1).field(FieldSpec::new("a", FieldKind::Resource("../x"))), "resource_kind");
    p(
        NodeSchema::new("t", 1).field(FieldSpec::new("a", FieldKind::Ref { to: b"m", on_remove: OnRemove::Clear })),
        "clear_needs_nullable",
    );
    p(
        NodeSchema::new("t", 1).field(FieldSpec::new(
            "a",
            FieldKind::Ref { to: b"m", on_remove: OnRemove::Redirect(NodeId { kind: b't', number: 0 }) },
        )),
        "redirect_target",
    );
    p(NodeSchema::new("t", 1).field(FieldSpec::new("a", FieldKind::List(Box::new(FieldKind::Id(b't'))))), "id_in_list");
    p(NodeSchema::new("t", 1).param(ParamSpec::new("a", 0.0, 1.0, 2.0)), "param_default");
    p(NodeSchema::new("t", 1).param(ParamSpec::new("a", 1.0, 1.0, 1.0)), "param_range");
    p(NodeSchema::new("t", 1).param(ParamSpec::new("a", 0.0, 1.0, 0.5).scale(Scale::Log)), "param_log_range");
    p(NodeSchema::new("t", 1).param(ParamSpec::new("a", 0.0, 4.0, 0.5).scale(Scale::Stepped)), "param_steps");
    p(
        NodeSchema::new("t", 1)
            .param(ParamSpec::new("a", 0.0, 1.0, 0.5))
            .param(ParamSpec::new("b", 0.0, 1.0, 0.5).aliases(&["a"])),
        "duplicate_param",
    );
    p(
        NodeSchema::new("t", 1).param(ParamSpec::new("a", 0.0, 1.0, 0.5)).field(FieldSpec::bool("params", true)),
        "params_field",
    );
    fn step(v: lm_state::Value) -> lm_base::Result<lm_state::Value> {
        Ok(v)
    }
    p(NodeSchema::new("t", 3).upgrade(1, step), "upgrade_chain");
    p(NodeSchema::new("t", 3).upgrade(1, step).upgrade(3, step), "upgrade_chain");
    p(NodeSchema::new("t", 2).upgrade(0, step).upgrade(1, step), "upgrade_chain");
    let mut r = SchemaRegistry::new();
    r.add(NodeSchema::new("t", 1)).unwrap();
    assert!(r.add(NodeSchema::new("t", 1)).unwrap_err().to_string().contains("duplicate_type"));
    // Nested types are checked once all are added
    let mut r = SchemaRegistry::new();
    r.add(NodeSchema::new("a", 1).field(FieldSpec::new("b", FieldKind::Object("b")))).unwrap();
    assert!(r.check_complete().unwrap_err().to_string().contains("unknown_nested_type"));
    // A type that requires itself has infinite defaults
    let mut r = SchemaRegistry::new();
    r.add(NodeSchema::new("a", 1).field(FieldSpec::new("a", FieldKind::Object("a")))).unwrap();
    assert!(r.check_complete().unwrap_err().to_string().contains("recursive_defaults"));
    // But an optional one is fine (a folder of folders)
    let mut r = SchemaRegistry::new();
    r.add(NodeSchema::new("a", 1).field(FieldSpec::new("a", FieldKind::Object("a")).optional())).unwrap();
    r.check_complete().unwrap();
}

#[test]
fn opaque_state_over_64_kb_must_be_a_resource() {
    let r = registry();
    let osc = r.defaults("plugin:tripleoscillator").unwrap();
    let small = osc.with("chunk", Value::object([("blob", Value::text("x".repeat(1000)))]));
    r.validate("plugin:tripleoscillator", &small).unwrap();
    let big = osc.with("chunk", Value::object([("blob", Value::text("x".repeat(64 * 1024 + 1)))]));
    let e = r.validate("plugin:tripleoscillator", &big).unwrap_err();
    assert_eq!(e.key(), "state.schema.text_too_large");
    assert!(e.to_string().contains("path=\"/chunk/blob\""), "{e}");
}

/// U29: the reference of this crate's test registry, compared on every run
#[test]
// Reads a file, which miri's isolation refuses; the comparison is plain safe code, run by every other job
#[cfg_attr(miri, ignore)]
fn defaults_are_frozen() {
    let reference = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/defaults.json");
    if let Err(mismatch) = lm_state::frozen::check_frozen_defaults(&registry(), reference.as_ref()) {
        panic!("{mismatch}");
    }
}

/// U29: what the check catches, on a reference in a temporary folder
#[test]
// Writes and removes files, which miri's isolation refuses; plain safe code, run by every other job
#[cfg_attr(miri, ignore)]
fn a_default_changed_without_a_version_bump_fails() {
    let dir = std::env::temp_dir().join(format!("lm-state-frozen-{}", std::process::id()));
    let reference = dir.join("defaults.json");
    let _ = std::fs::remove_file(&reference);
    let make = |version: u32, volume: f64| {
        let mut r = SchemaRegistry::new();
        r.add(NodeSchema::new("track", version).field(FieldSpec::real("volume", 0.0, 2.0, volume))).unwrap();
        r
    };
    // No reference: fails, and blessing writes it
    assert!(compare_or_bless(&make(1, 1.0), &reference, false).is_err());
    compare_or_bless(&make(1, 1.0), &reference, true).unwrap();
    compare_or_bless(&make(1, 1.0), &reference, false).unwrap();
    // The default changed, the version did not: fails, and blessing does not accept it
    let e = compare_or_bless(&make(1, 0.8), &reference, false).unwrap_err();
    assert!(e.problems[0].contains("\"volume\" changed without a version bump"), "{e}");
    assert!(compare_or_bless(&make(1, 0.8), &reference, true).is_err());
    // With a version bump: the reference must be updated, which blessing does
    let e = compare_or_bless(&make(2, 0.8), &reference, false).unwrap_err();
    assert!(e.problems[0].contains("version 1 → 2"), "{e}");
    compare_or_bless(&make(2, 0.8), &reference, true).unwrap();
    compare_or_bless(&make(2, 0.8), &reference, false).unwrap();
    // A type removed or added is also out of date
    assert!(compare_or_bless(&SchemaRegistry::new(), &reference, false).unwrap_err().problems[0].contains("removed"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn registry_is_shareable_across_threads() {
    fn check<T: Send + Sync>() {}
    check::<SchemaRegistry>();
    let r = std::sync::Arc::new(registry());
    let t = track();
    let handle = {
        let r = std::sync::Arc::clone(&r);
        std::thread::spawn(move || r.validate("track", &t).is_ok())
    };
    assert!(handle.join().unwrap());
}
