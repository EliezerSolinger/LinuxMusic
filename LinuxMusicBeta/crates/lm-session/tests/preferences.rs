//! The preferences' tree: every key the interface writes exists, and files from before keep their values.

use lm_session::prefs::Preferences;
use lm_state::{PathStep, Value};

fn key(p: &[&str]) -> Vec<PathStep> {
    p.iter().map(|k| PathStep::key(k)).collect()
}

#[test]
fn every_key_the_settings_screen_writes_is_in_the_defaults() {
    let mut prefs = Preferences::in_memory();
    for (path, value) in [
        (&["general", "reopenLast"][..], Value::bool(true)),
        (&["interface", "tooltips"], Value::bool(false)),
        (&["recording", "lanes"], Value::bool(false)),
        (&["folders", "soundfont"], Value::text("/x.sf2")),
        (&["backup", "autosaveMinutes"], Value::int(0)),
        (&["plugins", "windowsOnTop"], Value::bool(true)),
    ] {
        prefs.set(&key(path), value).unwrap();
    }
    assert_eq!(prefs.bool(&["general", "reopenLast"]), Some(true));
    assert_eq!(prefs.int(&["backup", "autosaveMinutes"]), Some(0));
}

#[test]
fn a_file_from_before_keeps_the_moved_values() {
    let dir = std::env::temp_dir().join(format!("lm-prefs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("preferences.json");
    std::fs::write(&file, r#"{"allowedFolders": ["/a"], "autosaveSeconds": 300}"#).unwrap();
    let (prefs, error) = Preferences::load(file);
    assert!(error.is_none());
    let allowed: Vec<_> = prefs.get(&["folders", "allowed"]).unwrap().items().filter_map(Value::as_text).collect();
    assert_eq!(allowed, ["/a"]);
    assert_eq!(prefs.int(&["backup", "autosaveMinutes"]), Some(5));
    std::fs::remove_dir_all(dir).ok();
}
