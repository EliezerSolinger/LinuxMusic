//! The modulators' keys and defaults are the document's (the renderer's table and the schema are two lists that must
//! stay one)

use lm_render::modulation::{MOD_DEFAULTS, MOD_KEYS};

#[test]
fn the_render_table_is_the_schema() {
    let registry = lm_doc::schemas::project_registry().expect("registry");
    let schema = registry.find("modulator").expect("the modulator type");
    let keys: Vec<&str> = schema.params.iter().map(|p| p.key).collect();
    assert_eq!(keys, MOD_KEYS);
    for (p, d) in schema.params.iter().zip(MOD_DEFAULTS) {
        assert_eq!(p.def as f32, d, "{}", p.key);
    }
}

#[test]
fn every_kind_uses_known_keys() {
    for kind in lm_doc::schemas::MODULATOR_KINDS {
        for k in lm_doc::schemas::modulator_keys(kind) {
            assert!(MOD_KEYS.contains(k), "{kind}: {k}");
        }
    }
}
