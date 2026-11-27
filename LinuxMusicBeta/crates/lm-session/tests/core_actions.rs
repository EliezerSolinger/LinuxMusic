//! The actions the interface sends that the session answers: presets, projects (new at a rate, the zip, the
//! template), takes, the safe mode, locating files

mod common;

use common::*;
use lm_state::{NodeId, PathStep, Value};

fn text(s: &str) -> Value {
    Value::text(s)
}

fn params(pairs: Vec<(&'static str, Value)>) -> Value {
    Value::object(pairs)
}

#[test]
fn a_preset_is_saved_and_loaded_back_in_one_undo_step() {
    let dir = TempDir::new("preset");
    let mut s = session(&dir);
    s.set_preference(&[PathStep::key("projectsFolder")], text(dir.0.to_str().unwrap())).unwrap();
    let track = add_synth_track(&mut s, "Lead");
    let before = s.document().find(track).unwrap().get("instrument").unwrap().get("state").unwrap().clone();
    s.run_action("plugin.save_preset", &params(vec![("node", text(&track.to_string())), ("name", text("Mine"))]))
        .unwrap();
    let file = dir.0.join("Presets").join("tripleoscillator").join("Mine.json");
    assert!(file.is_file(), "the preset is a file of the user's presets folder");
    // Change the state, then load the preset: the state is the saved one
    let changed = before.with("params", Value::object([("master", Value::real(0.25).unwrap())]));
    s.document_mut()
        .transact(lm_base::Label::new("test.state"), |e| {
            e.set(track, &[PathStep::key("instrument"), PathStep::key("state")], changed.clone())
        })
        .unwrap();
    s.run_action(
        "plugin.load_preset",
        &params(vec![("node", text(&track.to_string())), ("path", text(file.to_str().unwrap()))]),
    )
    .unwrap();
    let after = s.document().find(track).unwrap().get("instrument").unwrap().get("state").unwrap().clone();
    assert_eq!(after, before);
    s.document_mut().undo().unwrap();
    let undone = s.document().find(track).unwrap().get("instrument").unwrap().get("state").unwrap().clone();
    assert_eq!(undone, changed, "one undo step");
}

#[test]
fn a_preset_of_another_plugin_is_refused() {
    let dir = TempDir::new("preset-other");
    let mut s = session(&dir);
    let track = add_synth_track(&mut s, "Lead");
    let file = dir.0.join("other.json");
    std::fs::write(&file, r#"{"plugin":"opulenz","state":{}}"#).unwrap();
    let e = s
        .run_action(
            "plugin.load_preset",
            &params(vec![("node", text(&track.to_string())), ("path", text(file.to_str().unwrap()))]),
        )
        .unwrap_err();
    assert_eq!(e.key(), "plugin.preset.other_plugin");
}

#[test]
fn a_new_project_takes_the_rate() {
    let dir = TempDir::new("new-rate");
    let mut s = session(&dir);
    let folder = dir.join("song");
    s.run_action(
        "project.new",
        &params(vec![("folder", text(folder.to_str().unwrap())), ("rate", Value::int(44_100))]),
    )
    .unwrap();
    settle(&mut s);
    let rate = s.document().root().get("project").and_then(|p| p.get("sampleRate")).and_then(Value::as_int);
    assert_eq!(rate, Some(44_100));
    assert!(folder.join("projeto.json").is_file());
}

#[test]
fn a_new_project_from_a_template_that_is_not_there_is_refused() {
    let dir = TempDir::new("new-template");
    let mut s = session(&dir);
    let e = s
        .run_action(
            "project.new",
            &params(vec![("folder", text(dir.join("song").to_str().unwrap())), ("template", text("Nope"))]),
        )
        .unwrap_err();
    assert!(e.key().starts_with("session.template."), "{}", e.key());
}

#[test]
fn the_zip_holds_the_project_file() {
    let dir = TempDir::new("zip");
    let mut s = session(&dir);
    let folder = dir.join("song");
    s.run_action("project.new", &params(vec![("folder", text(folder.to_str().unwrap()))])).unwrap();
    settle(&mut s);
    s.run_action("project.export_zip", &Value::NULL).unwrap();
    settle(&mut s);
    let zip = std::fs::read(dir.join("song.zip")).expect("the archive is beside the project");
    assert_eq!(&zip[..4], b"PK\x03\x04");
    let name = b"song/projeto.json";
    assert!(zip.windows(name.len()).any(|w| w == name), "the project file is in it");
    // The end record is there too
    assert!(zip.windows(4).any(|w| w == b"PK\x05\x06"));
}

#[test]
fn takes_discard_with_nothing_interrupted_is_quiet_and_a_bad_index_is_refused() {
    let dir = TempDir::new("discard");
    let mut s = session(&dir);
    s.run_action("takes.discard", &Value::NULL).unwrap();
    let e = s.run_action("takes.discard", &params(vec![("index", Value::int(3))])).unwrap_err();
    assert_eq!(e.key(), "action.params");
}

#[test]
fn the_safe_mode_of_a_node_that_is_not_there_is_refused() {
    let dir = TempDir::new("safe");
    let mut s = session(&dir);
    let ghost = NodeId::parse("t:999999").unwrap();
    let e = s.run_action("safe_mode.enable", &params(vec![("node", text(&ghost.to_string()))])).unwrap_err();
    assert_eq!(e.key(), "action.node.missing");
}

#[test]
fn locating_a_file_that_is_not_the_resource_is_refused_when_the_operation_ends() {
    let dir = TempDir::new("locate");
    let mut s = session(&dir);
    let file = dir.join("x.wav");
    std::fs::write(&file, b"not the one").unwrap();
    let key = "0".repeat(64);
    s.run_action("resource.locate", &params(vec![("key", text(&key)), ("path", text(file.to_str().unwrap()))]))
        .unwrap();
    settle(&mut s);
    assert!(s.notices_keys().contains(&"session.locate.not_the_file"));
}

#[test]
fn allowing_a_folder_keeps_it_in_the_preferences() {
    let dir = TempDir::new("allow");
    let mut s = session(&dir);
    s.run_action("folder.allow", &params(vec![("folder", text(dir.0.to_str().unwrap()))])).unwrap();
    let allowed: Vec<String> = s
        .preferences()
        .get(&["folders", "allowed"])
        .map(|l| l.items().filter_map(Value::as_text).map(str::to_owned).collect())
        .unwrap_or_default();
    assert_eq!(allowed, vec![dir.0.display().to_string()]);
    // Twice is once
    s.run_action("folder.allow", &params(vec![("folder", text(dir.0.to_str().unwrap()))])).unwrap();
    assert_eq!(s.preferences().get(&["folders", "allowed"]).map(Value::len), Some(1));
}

#[test]
fn the_save_template_needs_a_saved_project() {
    let dir = TempDir::new("template");
    let mut s = session(&dir);
    let e = s.run_action("project.save_template", &Value::NULL).unwrap_err();
    assert_eq!(e.key(), "session.project.untitled");
}
