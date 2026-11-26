use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gpui::Keystroke;
use lm_port::Value;

use super::*;
use crate::actions::REGISTRY;

fn prefs_with(ctx: &str, node: Value) -> Value {
    Value::object([("keymaps", Value::object([(ctx, node)]))])
}

fn def(id: &str) -> &'static ActionDef {
    crate::actions::find(id).unwrap_or_else(|| panic!("no action {id}"))
}

// --- the data ---------------------------------------------------------------------------------------------------

#[test]
fn every_action_and_mouse_action_has_a_known_context() {
    let known: BTreeSet<_> = CONTEXTS.iter().map(|c| c.id).collect();
    for d in REGISTRY {
        assert!(known.contains(ctx_of(d)), "{} is in the unknown context {}", d.id, ctx_of(d));
    }
    for m in MOUSE {
        assert!(known.contains(m.ctx), "{} is in the unknown context {}", m.id, m.ctx);
    }
}

#[test]
fn mouse_action_ids_are_unique_in_their_context_and_their_gestures_parse() {
    let mut seen = BTreeSet::new();
    for m in MOUSE {
        assert!(seen.insert((m.ctx, m.id)), "{} twice in {}", m.id, m.ctx);
        for g in m.gestures {
            assert!(Gesture::parse(g).is_some(), "{}: the gesture {g} does not parse", m.id);
        }
    }
}

#[test]
fn the_shipped_presets_are_well_formed() {
    assert_eq!(presets::shipped().len(), presets::SHIPPED.len());
    for p in presets::shipped() {
        let file = format!("{}/{}", p.ctx, p.name);
        assert!(CONTEXTS.iter().any(|c| c.id == p.ctx), "{file}: unknown context");
        assert!(!p.user, "{file}");
        for (id, keys) in &p.keys {
            let d = crate::actions::REGISTRY
                .iter()
                .find(|d| d.id == id)
                .unwrap_or_else(|| panic!("{file}: no action {id}"));
            assert_eq!(ctx_of(d), p.ctx, "{file}: {id} belongs to another context");
            for k in keys {
                // the parser wants a string that lives for the whole run: a test may leak it
                let stroke: &'static str = Box::leak(crate::actions::keys_for_platform(k).into_boxed_str());
                for part in stroke.split(' ') {
                    Keystroke::parse(part).unwrap_or_else(|_| panic!("{file}: {id}: the keys {k} do not parse"));
                }
            }
        }
        for (id, gestures) in &p.mouse {
            let m = MOUSE
                .iter()
                .find(|m| m.id == id && m.ctx == p.ctx)
                .unwrap_or_else(|| panic!("{file}: no mouse action {id}"));
            for g in gestures {
                assert!(Gesture::parse(g).is_some(), "{file}: {}: the gesture {g} does not parse", m.id);
            }
        }
        for id in p.unsure.keys() {
            assert!(
                p.keys.contains_key(id) || p.mouse.contains_key(id),
                "{file}: unsure about {id}, which it does not set"
            );
        }
    }
}

#[test]
fn every_context_ships_lmms_and_reaper() {
    for c in CONTEXTS {
        for name in ["LMMS", "REAPER"] {
            assert!(presets::find(c.id, name, &[]).is_some(), "{} has no {name}", c.id);
        }
    }
}

/// Two actions of one context never share a key, in the defaults and in every shipped preset (a preset that wants a
/// key for two actions would hide one)
#[test]
fn no_key_is_two_actions_in_one_context() {
    let mut sets: Vec<(String, BTreeMap<&str, Vec<String>>)> = Vec::new();
    for c in CONTEXTS {
        let defaults = REGISTRY
            .iter()
            .filter(|d| ctx_of(d) == c.id)
            .map(|d| (d.id, d.keys.iter().map(|k| k.to_string()).collect()))
            .collect();
        sets.push((format!("{} defaults", c.id), defaults));
        for name in presets::names(c.id, &[]) {
            let p = presets::find(c.id, &name, &[]).unwrap();
            let all = REGISTRY
                .iter()
                .filter(|d| ctx_of(d) == c.id)
                .map(|d| {
                    (d.id, p.keys.get(d.id).cloned().unwrap_or_else(|| d.keys.iter().map(|k| k.to_string()).collect()))
                })
                .collect();
            sets.push((format!("{}/{name}", c.id), all));
        }
    }
    for (what, set) in sets {
        let mut owner: BTreeMap<&String, &str> = BTreeMap::new();
        for (id, keys) in &set {
            for k in keys {
                if let Some(other) = owner.insert(k, id) {
                    panic!("{what}: {k} is {other} and {id}");
                }
            }
        }
    }
}

// --- the order of precedence --------------------------------------------------------------------------------------

#[test]
fn the_defaults_then_the_preset_then_the_users_change() {
    let loop_ = def("transport.loop");
    assert_eq!(keys_for(loop_, &Value::NULL, &[]), vec!["l".to_string()]);
    let chosen = prefs_with("Transport", with_preset(&Value::NULL, "Transport", "LMMS"));
    assert_eq!(keys_for(loop_, &chosen, &[]), vec!["c".to_string()], "the preset");
    let changed =
        prefs_with("Transport", with_change(&chosen, "Transport", "keys", "transport.loop", Some(&["f7".to_string()])));
    assert_eq!(keys_for(loop_, &changed, &[]), vec!["f7".to_string()], "the user's change wins");
}

#[test]
fn a_preset_is_chosen_per_context() {
    let prefs = prefs_with("PianoRoll", with_preset(&Value::NULL, "PianoRoll", "REAPER"));
    assert_eq!(chosen(&prefs, "PianoRoll"), "REAPER");
    assert_eq!(chosen(&prefs, "Composition"), DEFAULT);
    assert_eq!(keys_for(def("arrange.split_at_cursor"), &prefs, &[]), vec!["s".to_string()]);
    assert_eq!(
        keys_for(def("notes.transpose_up"), &prefs, &[]),
        vec!["up".to_string()],
        "the Piano Roll's REAPER keys"
    );
}

#[test]
fn none_on_purpose_is_not_the_default() {
    let prefs = prefs_with("Transport", with_change(&Value::NULL, "Transport", "keys", "transport.loop", Some(&[])));
    assert!(keys_for(def("transport.loop"), &prefs, &[]).is_empty());
}

#[test]
fn restoring_drops_the_changes_and_keeps_the_choice() {
    let node = with_preset(&Value::NULL, "Mixing", "REAPER");
    let prefs = prefs_with("Mixing", node);
    let node = with_change(&prefs, "Mixing", "keys", "desk.remove_channel", Some(&["x".to_string()]));
    let prefs = prefs_with("Mixing", node);
    let node = with_change(&prefs, "Mixing", "mouse", "mix.fader.fine", None);
    let prefs = prefs_with("Mixing", node);
    assert!(has_changes(&prefs, "Mixing"));
    let back = prefs_with("Mixing", restored(&prefs, "Mixing"));
    assert!(!has_changes(&back, "Mixing"));
    assert_eq!(chosen(&back, "Mixing"), "REAPER");
}

#[test]
fn a_preset_update_keeps_the_users_changes() {
    let user_change =
        with_change(&Value::NULL, "Composition", "keys", "arrange.split_at_cursor", Some(&["q".to_string()]));
    let prefs = prefs_with("Composition", user_change.with("preset", Value::text("REAPER")));
    let mut p = presets::find("Composition", "REAPER", &[]).unwrap().clone();
    p.keys.insert("arrange.split_at_cursor".into(), vec!["w".into()]);
    p.user = true;
    p.name = "REAPER".into();
    // an updated preset with the same name (here as a user preset) changes nothing the user changed
    assert_eq!(keys_for(def("arrange.split_at_cursor"), &prefs, &[p]), vec!["q".to_string()]);
}

#[test]
fn a_mouse_action_follows_the_same_order() {
    let m = MOUSE.iter().find(|m| m.id == "comp.view.zoom").unwrap();
    assert_eq!(gestures_for(m, &Value::NULL, &[]), vec![Gesture::parse("wheel").unwrap()]);
    let lmms = prefs_with("Composition", with_preset(&Value::NULL, "Composition", "LMMS"));
    assert_eq!(gestures_for(m, &lmms, &[]), vec![Gesture::parse("secondary-wheel").unwrap()]);
    let own = prefs_with(
        "Composition",
        with_change(&lmms, "Composition", "mouse", "comp.view.zoom", Some(&["alt-wheel".to_string()])),
    );
    assert_eq!(gestures_for(m, &own, &[]), vec![Gesture::parse("alt-wheel").unwrap()]);
}

#[test]
fn the_handlers_read_the_installed_table() {
    use gpui::{Modifiers, MouseButton};
    let shift = Modifiers { shift: true, ..Modifiers::default() };
    install(&Value::NULL);
    assert!(holds("Automation", "au.extend", &shift));
    let own = prefs_with(
        "Automation",
        with_change(&Value::NULL, "Automation", "mouse", "au.extend", Some(&["alt-click".to_string()])),
    );
    install(&own);
    assert!(!holds("Automation", "au.extend", &shift));
    assert!(fires("Automation", "au.extend", MouseButton::Left, 1, &Modifiers { alt: true, ..Modifiers::default() }));
    install(&Value::NULL);
}

#[test]
fn plain_typing_keys_are_told_from_commands() {
    for k in ["a", "shift-d", "space", "left", "enter", "delete", "-", "="] {
        assert!(is_typing(k), "{k}");
    }
    for k in ["secondary-s", "alt-left", "escape", "f1", "ctrl-tab", "secondary-shift-z"] {
        assert!(!is_typing(k), "{k}");
    }
}

#[test]
fn a_snapshot_is_the_whole_context() {
    let s = snapshot("Mixing", "Mine", &Value::NULL, &[]);
    assert_eq!(s.keys.len(), REGISTRY.iter().filter(|d| ctx_of(d) == "Mixing").count());
    assert_eq!(s.mouse.len(), MOUSE.iter().filter(|m| m.ctx == "Mixing").count());
    assert!(s.user);
}

// --- the files --------------------------------------------------------------------------------------------------

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lm-keymap-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_user_preset_is_saved_loaded_and_deleted_as_a_file() {
    let dir = temp("user");
    let p = snapshot("PianoRoll", "My roll", &Value::NULL, &[]);
    let file = presets::save_user(&dir, &p).unwrap();
    assert!(file.ends_with("keymaps/PianoRoll/My roll.json"));
    let loaded = presets::load_user(&dir);
    assert_eq!(loaded, vec![p.clone()]);
    presets::delete_user(&dir, &p).unwrap();
    assert!(presets::load_user(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_preset_is_exported_and_imported_with_its_context() {
    let dir = temp("export");
    let mut p = snapshot("Browser", "Mine", &Value::NULL, &[]);
    p.unsure.insert("browser.find".into(), "to check".into());
    let file = dir.join("out.json");
    presets::write_file(&file, &p).unwrap();
    let back = presets::read_file(&file, "Global").unwrap();
    assert_eq!(back.ctx, "Browser", "the file's context wins over the one asked for");
    assert_eq!(back, p);
    assert!(presets::read_file(&dir.join("missing.json"), "Global").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_free_name_never_takes_a_shipped_or_a_user_name() {
    assert_eq!(presets::free_name("Mixing", "Mine", &[]), "Mine");
    assert_eq!(presets::free_name("Mixing", "REAPER", &[]), "REAPER 2");
    let mine = Preset { ctx: "Mixing".into(), name: "Mine".into(), user: true, ..Preset::default() };
    assert_eq!(presets::free_name("Mixing", "Mine", &[mine]), "Mine 2");
    assert_eq!(presets::free_name("Mixing", "  ", &[]), "Mine");
}

// --- the sources: no key and no modifier outside the keymap --------------------------------------------------------

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let p = e.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn gui_sources() -> Vec<(String, Vec<String>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    files
        .into_iter()
        .map(|f| {
            let rel = f.strip_prefix(&root).unwrap().display().to_string();
            let text = std::fs::read_to_string(&f).unwrap();
            // what is under `#[cfg(test)]` at the end of a file is not the program's
            let body = text.split("\n#[cfg(test)]").next().unwrap_or("").to_string();
            (rel, body.lines().map(str::to_string).collect())
        })
        .filter(|(rel, _)| {
            !rel.starts_with("keymap/")
                && !rel.starts_with("tests")
                && rel != "actions.rs"
                && !rel.ends_with("settings/keys.rs")
        })
        .collect()
}

fn marked(lines: &[String], i: usize, what: &[&str]) -> bool {
    let from = i.saturating_sub(6);
    lines[from..=i].iter().any(|l| what.iter().any(|w| l.contains(w)))
}

/// A key is read only by the registry: any other reading of the keyboard says why with `keymap-ok:` (a piano key, a
/// text field)
#[test]
fn no_key_is_handled_outside_the_registry() {
    let patterns = ["keystroke", "on_key_down", "on_key_up", "capture_key_down", "capture_key_up"];
    let mut found = Vec::new();
    for (file, lines) in gui_sources() {
        for (i, l) in lines.iter().enumerate() {
            let code = l.split("//").next().unwrap_or("");
            if patterns.iter().any(|p| code.contains(p))
                && !code.trim_start().starts_with("use ")
                && !marked(&lines, i, &["keymap-ok"])
            {
                found.push(format!("{file}:{}: {}", i + 1, l.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "keys read outside the registry (make them actions, or mark `keymap-ok: why`):\n{}",
        found.join("\n")
    );
}

/// A modifier is read only through the mouse table (`holds`, `fires`, `pick_wheel`, `keymap::`)
#[test]
fn no_modifier_is_read_outside_the_mouse_table() {
    let through = ["holds(", "fires(", "pick_wheel(", "keymap::", "keymap-ok", "fn holds", "fn fires", "snapped("];
    let mut found = Vec::new();
    for (file, lines) in gui_sources() {
        for (i, l) in lines.iter().enumerate() {
            let code = l.split("//").next().unwrap_or("");
            let raw = code.contains(".modifiers")
                || code.contains("modifiers()")
                || code.contains("mods.alt")
                || code.contains("mods.shift");
            if raw && !code.trim_start().starts_with("use ") && !marked(&lines, i, &through) {
                found.push(format!("{file}:{}: {}", i + 1, l.trim()));
            }
        }
    }
    assert!(found.is_empty(), "modifiers read outside the mouse table:\n{}", found.join("\n"));
}

/// Every id the screens ask the table for exists in it
#[test]
fn the_ids_the_screens_use_are_in_the_table() {
    let known: BTreeSet<&str> = MOUSE.iter().map(|m| m.id).collect();
    let prefixes = ["comp.", "mix.", "pr.", "au.", "st.", "param.", "transport."];
    let mut missing = Vec::new();
    for (file, lines) in gui_sources() {
        for l in &lines {
            let mut rest = l.as_str();
            while let Some(i) = rest.find('"') {
                let Some(j) = rest[i + 1..].find('"') else { break };
                let lit = &rest[i + 1..i + 1 + j];
                rest = &rest[i + j + 2..];
                let shaped = lit.chars().all(|c| c.is_ascii_lowercase() || c == '.' || c == '_');
                if shaped && prefixes.iter().any(|p| lit.starts_with(p)) && lit.contains('.') {
                    let is_action = crate::actions::find(lit).is_some() || lm_session::actions::find(lit).is_some();
                    if !known.contains(lit) && !is_action {
                        missing.push(format!("{file}: {lit}"));
                    }
                }
            }
        }
    }
    assert!(missing.is_empty(), "not in the mouse table nor the actions: {missing:?}");
}
