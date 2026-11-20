//! Owner: agent D. The action registry (I-19, spec item 6): every action is data, with an id, a translatable label,
//! a group, its default keys and what it runs. Menus, shortcuts, the command palette and the external API are views
//! of this list. The shortcuts the user changed live in the preferences (`project::Prefs::keys`); `shortcut`
//! answers the one in effect.
//!
//! Keys are written platform-neutral: `primary` is Cmd on macOS and Ctrl elsewhere (`primary-shift-s`, `space`).

use crate::port::CorePort;
use crate::{Field, Session, Value, project};

/// What an action does
#[derive(Clone, Copy)]
pub enum Run {
    /// A change to the core: a transaction, a transport command, or the start of an operation
    Core(fn(&mut Session)),
    /// The same, with the argument a notice gives ("try again" says which operation)
    CoreWith(fn(&mut Session, &str)),
    /// Opens a screen (by its name in the interface's registry), with an optional part of it ("save_as")
    Screen(&'static str, Option<&'static str>),
    /// Shows a page of the shell
    Page(&'static str),
    /// Shows a file in the system's file manager (the argument is the path)
    Reveal,
}

#[derive(Clone, Copy)]
pub struct ActionSpec {
    pub id: &'static str,
    /// English key, translated by the interface
    pub label: &'static str,
    /// English key of the group (the palette and the shortcut list group by it)
    pub group: &'static str,
    pub keys: Option<&'static str>,
    pub run: Run,
    /// Read through the port, so the interface asks it the same way in-process and over the network
    pub enabled: fn(&dyn CorePort) -> bool,
    /// Offered in the palette (the ones that need an argument come only from notices)
    pub palette: bool,
}

fn always(_: &dyn CorePort) -> bool {
    true
}
fn writable(p: &dyn CorePort) -> bool {
    !p.services().project.read_only()
}
fn not_recording(p: &dyn CorePort) -> bool {
    !p.telemetry().position.recording
}

const fn a(id: &'static str, label: &'static str, group: &'static str, keys: Option<&'static str>, run: Run) -> ActionSpec {
    ActionSpec { id, label, group, keys, run, enabled: always, palette: true }
}

const TRANSPORT: &str = "Transport";
const EDIT: &str = "Edit";
const VIEW: &str = "View";
const FILE: &str = "File";
const TRACKS: &str = "Tracks";
const HELP: &str = "Help";
const DEVICE: &str = "Audio device";

fn toggle_play(s: &mut Session) {
    if s.position().playing { s.stop() } else { s.play() }
}

fn set_all_tracks(s: &mut Session, field: Field, v: bool) {
    let ids: Vec<_> = s.tracks().iter().map(|t| t.id).collect();
    let _ = s.transact("Change every track", |e| {
        for id in ids {
            e.set(id, field, Value::Bool(v))?;
        }
        Ok(())
    });
}

/// The whole registry
pub static REGISTRY: &[ActionSpec] = &[
    a("transport.play", "Play or stop", TRANSPORT, Some("space"), Run::Core(toggle_play)),
    a("transport.stop", "Stop", TRANSPORT, None, Run::Core(|s| s.stop())),
    a("transport.record", "Record", TRANSPORT, Some("r"), Run::Core(|s| s.toggle_record())),
    a("transport.rewind", "Go to the start", TRANSPORT, Some("home"), Run::Core(|s| s.locate(0))),
    a("edit.undo", "Undo", EDIT, Some("primary-z"), Run::Core(|s| drop(s.undo()))),
    a("view.composition", "Show the composition page", VIEW, Some("primary-1"), Run::Page("composition")),
    a("view.mixing", "Show the mixing page", VIEW, Some("primary-2"), Run::Page("mixing")),
    a("view.palette", "Command palette", VIEW, Some("primary-k"), Run::Screen("Palette", None)),
    a("view.piano_roll", "Open the Piano Roll", VIEW, None, Run::Screen("PianoRoll", None)),
    a("view.automation", "Open the automation editor", VIEW, None, Run::Screen("Automation", None)),
    a("view.steps", "Open the step editor", VIEW, None, Run::Screen("Steps", None)),
    a("view.instrument", "Open the instrument window", VIEW, None, Run::Screen("Instrument", None)),
    a("view.effect", "Open the effect window", VIEW, None, Run::Screen("Effect", None)),
    a("view.browser", "Open the browser", VIEW, None, Run::Screen("Browser", None)),
    a("view.routing", "Open the routing", VIEW, None, Run::Screen("Routing", None)),
    a("view.notices", "Show the notices and activities", VIEW, None, Run::Screen("Notices", None)),
    a("view.settings", "Settings", VIEW, Some("primary-,"), Run::Screen("Settings", None)),
    a("file.new", "New project", FILE, Some("primary-n"), Run::Screen("NewProject", None)),
    a("file.open", "Open project", FILE, Some("primary-o"), Run::Screen("Project", Some("open"))),
    ActionSpec { enabled: writable, ..a("file.save", "Save", FILE, Some("primary-s"), Run::Core(project::save)) },
    a("file.save_as", "Save as", FILE, Some("primary-shift-s"), Run::Screen("Project", Some("save_as"))),
    a("file.export", "Export audio", FILE, Some("primary-e"), Run::Screen("Export", None)),
    a("file.export_zip", "Export the project as a single file", FILE, None, Run::Core(project::export_zip)),
    a("file.recent", "Recent projects", FILE, None, Run::Screen("Project", Some("open"))),
    a("tracks.unmute_all", "Unmute every track", TRACKS, None, Run::Core(|s| set_all_tracks(s, Field::Mute, false))),
    a("tracks.unsolo_all", "Clear every solo", TRACKS, None, Run::Core(|s| set_all_tracks(s, Field::Solo, false))),
    a("tracks.disarm_all", "Disarm every track", TRACKS, None, Run::Core(|s| set_all_tracks(s, Field::Armed, false))),
    ActionSpec { enabled: not_recording, ..a("device.calibrate", "Calibrate the latency", DEVICE, None, Run::Core(project::calibrate)) },
    ActionSpec { enabled: not_recording, ..a("device.reopen", "Reopen the audio device", DEVICE, None, Run::Core(project::reopen_device)) },
    a("device.scan", "Look for audio and MIDI devices again", DEVICE, None, Run::Core(project::scan_devices)),
    a("notices.clear", "Clear the notices", HELP, None, Run::Core(|s| s.services.notices.clear())),
    a("help.shortcuts", "Keyboard shortcuts", HELP, None, Run::Screen("Settings", Some("keys"))),
    a("help.about", "About LinuxMusic", HELP, None, Run::Screen("About", None)),
    // from notices only (they need an argument)
    ActionSpec { palette: false, ..a("op.retry", "Try again", HELP, None, Run::CoreWith(project::retry)) },
    ActionSpec { palette: false, ..a("plugin.recreate", "Recreate the plugin", HELP, None, Run::CoreWith(project::recreate_plugin)) },
    ActionSpec { palette: false, ..a("resource.locate", "Locate the file", HELP, None, Run::CoreWith(project::locate_resource)) },
    ActionSpec { palette: false, ..a("resource.locate_folder", "Locate the folder", HELP, None, Run::CoreWith(project::locate_resource)) },
    ActionSpec { palette: false, ..a("folder.allow", "Allow this folder", HELP, None, Run::CoreWith(project::allow_folder)) },
    ActionSpec { palette: false, ..a("settings.realtime", "What to configure", HELP, None, Run::Screen("Settings", Some("audio"))) },
    ActionSpec { palette: false, ..a("export.reveal", "Show the files", HELP, None, Run::Reveal) },
    ActionSpec { palette: false, ..a("device.reconnect", "Reconnect", HELP, None, Run::Core(project::reopen_device)) },
    ActionSpec { palette: false, ..a("remote.reconnect", "Reconnect to the server", HELP, None, Run::CoreWith(project::reconnect)) },
    a("remote.connect", "Connect to a server", FILE, None, Run::Screen("Project", Some("connect"))),
    a("remote.disconnect", "Disconnect from the server", FILE, None, Run::Core(project::disconnect)),
    // what the screens change, as data (the parameters are `key=value` lines, `param_lines`)
    ActionSpec { palette: false, ..a("prefs.set", "Change a setting", HELP, None, Run::CoreWith(project::prefs_set)) },
    ActionSpec { palette: false, ..a("palette.used", "Remember a command", HELP, None, Run::CoreWith(project::used_action)) },
    ActionSpec { palette: false, ..a("notices.dismiss", "Dismiss the notice", HELP, None, Run::CoreWith(project::dismiss_notice)) },
    ActionSpec { palette: false, ..a("notices.read", "Mark the notices read", HELP, None, Run::Core(|s| s.services.notices.mark_read())) },
    ActionSpec { palette: false, ..a("project.open_recent", "Open a recent project", FILE, None, Run::CoreWith(project::open_recent_action)) },
    ActionSpec { palette: false, ..a("project.create", "Create the project", FILE, None, Run::CoreWith(project::create_action)) },
    ActionSpec { palette: false, ..a("project.save_to", "Save in another folder", FILE, None, Run::CoreWith(project::save_as_action)) },
    ActionSpec { palette: false, ..a("project.decide_take", "Recover or leave a take", FILE, None, Run::CoreWith(project::decide_take_action)) },
    ActionSpec { palette: false, ..a("project.safe_plugin", "Turn a plugin back on", FILE, None, Run::CoreWith(project::safe_plugin_action)) },
    ActionSpec { palette: false, ..a("export.start", "Start the export", FILE, None, Run::CoreWith(project::export_action)) },
    ActionSpec { palette: false, ..a("project.notes", "Change the project's notes", FILE, None, Run::CoreWith(project::set_notes)) },
    ActionSpec { palette: false, ..a("project.open_path", "Open a folder or a .zip", FILE, None, Run::CoreWith(project::open_path)) },
    a("file.save_version", "Save as a new version", FILE, Some("primary-alt-s"), Run::Screen("Project", Some("version"))),
    ActionSpec { enabled: writable, ..a("file.save_template", "Save as a template", FILE, None, Run::Core(project::save_template)) },
    a("file.clean_unused", "Clean the unused files", FILE, None, Run::Screen("Project", Some("project"))),
    ActionSpec { palette: false, ..a("project.clean_unused", "Move the unused files to the trash", FILE, None, Run::Core(project::clean_unused)) },
    a("file.notes", "Project notes", FILE, None, Run::Screen("Project", Some("notes"))),
    a("help.manual", "The manual", HELP, Some("f1"), Run::Screen("About", Some("manual"))),
    ActionSpec { palette: false, ..a("remote.connect_to", "Connect", FILE, None, Run::CoreWith(project::connect_action)) },
];

/// An action's parameters as the text the fake's actions read: a `Value::Text` as it is (`key=value` lines), a number
/// without a trailing `.0`, an id as `t:17`
pub fn param_text(v: &Value) -> String {
    match v {
        Value::Text(t) => t.clone(),
        Value::Real(x) if x.fract() == 0.0 => format!("{}", *x as i64),
        Value::Real(x) => format!("{x}"),
        Value::Bool(b) => b.to_string(),
        Value::Id(id) => format!("{id:?}"),
        _ => String::new(),
    }
}

/// `key=value` lines into pairs (a value may hold `=`)
pub fn params(text: &str) -> std::collections::BTreeMap<&str, &str> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim(), v)).collect()
}

/// Pairs into `key=value` lines (the inverse of `params`)
pub fn param_lines(pairs: &[(&str, String)]) -> Value {
    Value::Text(pairs.iter().map(|(k, v)| format!("{k}={}", v.replace('\n', " "))).collect::<Vec<_>>().join("\n"))
}

pub fn find(id: &str) -> Option<&'static ActionSpec> {
    REGISTRY.iter().find(|a| a.id == id)
}

/// The shortcut in effect: the user's (from the preferences), or the default; `Some("")` in the preferences means
/// "no shortcut"
pub fn shortcut(prefs: &project::Prefs, id: &str) -> Option<String> {
    match prefs.keys.get(id) {
        Some(k) if k.is_empty() => None,
        Some(k) => Some(k.clone()),
        None => find(id).and_then(|a| a.keys).map(str::to_string),
    }
}

/// The other action that already uses `keys`, if any (the shortcut editor warns before taking it)
pub fn conflict(prefs: &project::Prefs, keys: &str, except: &str) -> Option<&'static ActionSpec> {
    REGISTRY.iter().find(|a| a.id != except && shortcut(prefs, a.id).as_deref() == Some(keys))
}

/// Every (keys, action id) in effect: what the shell binds
pub fn keymap(prefs: &project::Prefs) -> Vec<(String, &'static str)> {
    REGISTRY.iter().filter_map(|a| shortcut(prefs, a.id).map(|k| (k, a.id))).collect()
}

/// Lower case without accents ("Configurações" is found by "configuracoes", F-CFG-001)
pub fn fold(c: char) -> char {
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        c => c,
    }
}

/// Fuzzy match of `query` in `text` (case-insensitive subsequence). The score rewards matches at word starts and
/// runs of consecutive letters, and penalizes gaps; the positions (in chars) are for highlighting.
pub fn fuzzy(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).map(fold).collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let t: Vec<char> = text.chars().collect();
    let lower: Vec<char> = t.iter().map(|c| fold(*c)).collect();
    // greedy from each possible start; keep the best
    let mut best: Option<(i32, Vec<usize>)> = None;
    for start in 0..lower.len() {
        if lower[start] != q[0] {
            continue;
        }
        let mut pos = vec![start];
        let mut i = start + 1;
        for &qc in &q[1..] {
            while i < lower.len() && lower[i] != qc {
                i += 1;
            }
            if i == lower.len() {
                break;
            }
            pos.push(i);
            i += 1;
        }
        if pos.len() < q.len() {
            break; // later starts cannot match more
        }
        let mut score = 0;
        for (k, &p) in pos.iter().enumerate() {
            let word_start = p == 0 || !t[p - 1].is_alphanumeric();
            if word_start {
                score += 10;
            }
            if k > 0 && pos[k - 1] + 1 == p {
                score += 6;
            } else if k > 0 {
                score -= (p - pos[k - 1]) as i32;
            }
        }
        score -= start as i32 / 2;
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, pos));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_defaults_do_not_clash() {
        let mut ids = std::collections::HashSet::new();
        let mut keys = std::collections::HashSet::new();
        for a in REGISTRY {
            assert!(ids.insert(a.id), "duplicate id {}", a.id);
            if let Some(k) = a.keys {
                assert!(keys.insert(k), "two actions on {k}");
            }
        }
    }

    #[test]
    fn fuzzy_prefers_word_starts() {
        assert!(fuzzy("xyz", "Export audio").is_none());
        let (s1, p) = fuzzy("exa", "Export audio").unwrap();
        assert_eq!(p, vec![0, 1, 7]);
        let (s2, _) = fuzzy("exa", "Next page").map_or((i32::MIN, vec![]), |x| x);
        assert!(s1 > s2);
        assert!(fuzzy("sv as", "Save as").is_some());
        assert!(fuzzy("configuracoes", "Configurações").is_some());
    }

    #[test]
    fn user_keys_override_and_clear() {
        let mut p = project::Prefs::default();
        assert_eq!(shortcut(&p, "file.save").as_deref(), Some("primary-s"));
        p.keys.insert("file.save", String::new());
        assert_eq!(shortcut(&p, "file.save"), None);
        p.keys.insert("file.export", "primary-s".into());
        assert_eq!(conflict(&p, "primary-s", "transport.play").map(|a| a.id), Some("file.export"));
    }
}
