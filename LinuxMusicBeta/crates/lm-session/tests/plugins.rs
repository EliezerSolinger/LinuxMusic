//! Every plugin crate of the workspace is registered in the session (`internal_plugins`)

#[test]
fn every_plugin_crate_is_registered() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let manifest = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("Cargo.toml");
    let source = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/session.rs"))
        .expect("session.rs");
    let body = source.split("pub fn internal_descriptors").nth(1).expect("internal_descriptors").to_owned();
    let body = body.split("\n}\n").next().unwrap_or_default();
    let mut missing = Vec::new();
    for e in std::fs::read_dir(&crates).expect("crates").flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("lm-plugins-") || !e.path().join("Cargo.toml").exists() {
            continue;
        }
        let ident = name.replace('-', "_");
        if !manifest.contains(&format!("{name}.workspace = true")) || !body.contains(&format!("{ident}::")) {
            missing.push(name);
        }
    }
    assert!(missing.is_empty(), "plugin crates not registered in lm_session::internal_descriptors: {missing:?}");
    // And they register without clashing ids
    let registry = lm_session::internal_plugins().expect("registry");
    assert!(registry.descriptors().count() > 20);
}
