//! The preferences of the machine (I-16; the shape is lm-port's `trees::preferences`): never in the project. Read
//! once at start-up (after the first frame, U19), changed by path, written back whole in an operation.
//!
//! Where: `$LINUXMUSIC_CONFIG/preferences.json` when set; beside the executable when a file `portable` is there
//! (P-35); otherwise `$XDG_CONFIG_HOME/linuxmusic` (or `~/.config/linuxmusic`) on Linux and
//! `~/Library/Application Support/LinuxMusic` on macOS. The data folder (untitled projects, peaks, logs) is
//! `$LINUXMUSIC_DATA`, or `data` beside the executable in portable mode, or the system's own (see [`data_dir`]).

use std::path::PathBuf;

use lm_base::{Error, Result};
use lm_state::{PathStep, Value, from_json, path_text};

/// The preferences a new installation starts with
pub fn defaults() -> Value {
    let o = |pairs: Vec<(&str, Value)>| Value::object(pairs);
    let real = |x: f64| Value::real(x).unwrap_or(Value::NULL);
    o(vec![
        ("language", Value::text("pt-BR")),
        ("theme", Value::text("dark")),
        ("projectsFolder", Value::text(default_projects_folder())),
        ("askBeforeClosing", Value::bool(true)),
        (
            "audio",
            o(vec![
                ("system", Value::text(if cfg!(target_os = "macos") { "coreaudio" } else { "alsa" })),
                ("input", Value::text("")),
                ("output", Value::text("")),
                ("rate", Value::int(48_000)),
                ("block", Value::int(256)),
                ("driftCorrection", Value::bool(false)),
                ("latencyFrames", Value::int(0)),
                ("calibrations", Value::empty_list()),
            ]),
        ),
        (
            "midi",
            o(vec![("inputs", Value::empty_list()), ("clock", Value::bool(false)), ("ports", Value::empty_list())]),
        ),
        // The machine's MIDI mappings (I-21.2, the same nodes as `project.midiMap`) and the controller profiles in
        // use (`{profile, port, bank}`, I-21.3)
        ("midiMap", Value::empty_list()),
        // The OSC server (off; localhost unless `lan`): the port it listens at, and whether other machines may reach it
        ("osc", o(vec![("on", Value::bool(false)), ("port", Value::int(9000)), ("lan", Value::bool(false))])),
        ("controllers", Value::empty_list()),
        (
            "interface",
            o(vec![
                ("followCursor", Value::bool(false)),
                ("scale", real(1.0)),
                ("confirmTrackRemoval", Value::bool(true)),
                ("tooltips", Value::bool(true)),
                ("noteNames", Value::bool(true)),
                ("loopEditMode", Value::text("buttons")),
                ("scrollMode", Value::text("page")),
                ("stopBehavior", Value::text("origin")),
                // What a mixer channel's meter reads: peak, rms, vu, k20, k14 or k12
                ("channelMeter", Value::text("peak")),
            ]),
        ),
        ("metronome", o(vec![("when", Value::text("recording")), ("countInBars", Value::int(1))])),
        (
            "recording",
            o(vec![("format", Value::text("rf64-24")), ("folder", Value::text("")), ("lanes", Value::bool(true))]),
        ),
        // The General page (F-CFG-010)
        ("general", o(vec![("reopenLast", Value::bool(false)), ("newProjectRate", Value::int(48_000))])),
        // Folders: the default SoundFont, and the folders outside a project it may read (C9)
        ("folders", o(vec![("soundfont", Value::text("")), ("allowed", Value::empty_list())])),
        // Autosave (minutes, 0 off) and the copies kept of a project
        (
            "backup",
            o(vec![("autosaveMinutes", Value::int(1)), ("kept", Value::int(10)), ("onSave", Value::bool(true))]),
        ),
        // Keymaps (user, 26/11): per context `{preset, keys: {action: keys}, mouse: {action: gesture}}`, the choice of
        // preset and the user's own changes on top of it
        ("keymaps", Value::empty_object()),
        // The speakers' monitoring (never in the project, the undo or an export)
        (
            "monitor",
            o(vec![
                ("level", real(0.0)),
                ("dim", Value::bool(false)),
                ("cut", Value::bool(false)),
                ("mono", Value::bool(false)),
                ("side", Value::bool(false)),
            ]),
        ),
        // Not shown on the settings screen
        ("recent", Value::empty_list()),
        ("sampleFolders", Value::empty_list()),
        ("favorites", Value::empty_list()),
        // The user's tags of browser files: path -> list of tags
        ("tags", Value::empty_object()),
        ("lowLatencyMonitoring", Value::bool(false)),
        // Third-party plugins (I-9.6): search folders added by the user, the last scans, the plugins blocked (they
        // crashed or hung their probe)
        (
            "plugins",
            o(vec![
                ("paths", Value::empty_list()),
                // The folders searched for one format's plugins, besides the system's
                (
                    "folders",
                    o(vec![
                        ("lv2", Value::empty_list()),
                        ("clap", Value::empty_list()),
                        ("vst3", Value::empty_list()),
                        ("ladspa", Value::empty_list()),
                    ]),
                ),
                ("scanned", Value::empty_list()),
                ("blocked", Value::empty_list()),
                ("grouped", Value::empty_list()),
                ("windowsOnTop", Value::bool(false)),
            ]),
        ),
    ])
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn default_projects_folder() -> String {
    home().join("LinuxMusic").display().to_string()
}

/// The folder of the program's executable when a file `portable` is there: everything of the program lives in it
fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join("portable").exists().then(|| dir.to_path_buf())
}

/// An environment variable that names a folder (empty or relative values are ignored)
fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The machine's data folder: where an untitled project lives until it is saved, the peaks cache and the logs.
/// `$LINUXMUSIC_DATA`, else `data` beside a portable executable, else `~/Library/Application Support/LinuxMusicBeta`
/// (macOS) or `$XDG_DATA_HOME/LinuxMusicBeta` (`~/.local/share`)
pub fn data_dir() -> PathBuf {
    if let Some(d) = env_dir("LINUXMUSIC_DATA") {
        return d;
    }
    if let Some(d) = portable_dir() {
        return d.join("data");
    }
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/LinuxMusicBeta")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/share"))
            .join("LinuxMusicBeta")
    }
}

/// The folders a first run creates: the working folder (projects) with its presets, and the data folder
pub fn create_default_folders(prefs: &Preferences) -> std::io::Result<()> {
    let projects = PathBuf::from(prefs.text(&["projectsFolder"]).unwrap_or(""));
    if !projects.as_os_str().is_empty() {
        std::fs::create_dir_all(projects.join("Presets"))?;
    }
    std::fs::create_dir_all(data_dir())
}

/// The file the preferences live in
pub fn location() -> PathBuf {
    if let Some(dir) = env_dir("LINUXMUSIC_CONFIG") {
        return dir.join("preferences.json");
    }
    if let Some(dir) = portable_dir() {
        return dir.join("preferences.json");
    }
    let base = if cfg!(target_os = "macos") {
        home().join("Library/Application Support/LinuxMusic")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config"))
            .join("linuxmusic")
    };
    base.join("preferences.json")
}

/// The preferences, as a tree with every key of [`defaults`]
#[derive(Clone, Debug)]
pub struct Preferences {
    tree: Value,
    file: Option<PathBuf>,
}

/// Every key of the defaults kept, with the file's value where it has one of the same kind
fn merge(defaults: &Value, read: &Value) -> Value {
    if !defaults.is_object() {
        return if read.kind() == defaults.kind() { read.clone() } else { defaults.clone() };
    }
    let mut out = defaults.clone();
    for (key, d) in defaults.entries() {
        if let Some(r) = read.get(key) {
            out = out.with(
                key,
                if d.is_object() && !d.is_empty() {
                    merge(d, r)
                } else if r.kind() == d.kind() {
                    r.clone()
                } else {
                    d.clone()
                },
            );
        }
    }
    // Free maps (the keymaps) keep the file's keys
    if defaults.is_empty() && read.is_object() {
        return read.clone();
    }
    out
}

/// Keys that moved: a file written before keeps its value under the new key
fn migrate(tree: Value, read: &Value) -> Value {
    let mut tree = tree;
    if let Some(old) = read.get("allowedFolders").filter(|l| l.items().next().is_some())
        && tree.get("folders").and_then(|f| f.get("allowed")).is_some_and(|l| l.items().next().is_none())
        && let Some(folders) = tree.get("folders").cloned()
    {
        tree = tree.with("folders", folders.with("allowed", old.clone()));
    }
    if let Some(seconds) = read.get("autosaveSeconds").and_then(Value::as_int)
        && read.get("backup").and_then(|b| b.get("autosaveMinutes")).is_none()
        && let Some(backup) = tree.get("backup").cloned()
    {
        tree = tree.with("backup", backup.with("autosaveMinutes", Value::int((seconds / 60).clamp(0, 30))));
    }
    tree
}

impl Preferences {
    /// The defaults, kept nowhere (tests, the headless renderer)
    pub fn in_memory() -> Self {
        Self { tree: defaults(), file: None }
    }

    /// Read from `file` (the defaults where it is absent or unreadable: a broken file never stops the program)
    pub fn load(file: PathBuf) -> (Self, Option<Error>) {
        let base = defaults();
        let (tree, error) = match std::fs::read_to_string(&file) {
            Ok(text) => match from_json(&text) {
                Ok(v) => (migrate(merge(&base, &v), &v), None),
                Err(e) => (base, Some(e.arg("path", file.display().to_string()))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (base, None),
            Err(e) => (base, Some(crate::folder::io_error("session.preferences.read", &file, &e))),
        };
        (Self { tree, file: Some(file) }, error)
    }

    pub fn tree(&self) -> &Value {
        &self.tree
    }

    pub fn file(&self) -> Option<&PathBuf> {
        self.file.as_ref()
    }

    pub fn get(&self, path: &[&str]) -> Option<&Value> {
        let steps: Vec<PathStep> = path.iter().map(|k| PathStep::key(k)).collect();
        self.tree.at_path(&steps)
    }

    pub fn int(&self, path: &[&str]) -> Option<i64> {
        self.get(path).and_then(Value::as_int)
    }

    pub fn text(&self, path: &[&str]) -> Option<&str> {
        self.get(path).and_then(Value::as_text)
    }

    pub fn bool(&self, path: &[&str]) -> Option<bool> {
        self.get(path).and_then(Value::as_bool)
    }

    /// Changes one preference: the path must exist in the defaults (or be inside a free map, the keymaps), and the
    /// value must be of the same kind. Keys `session.preferences.unknown`, `session.preferences.kind`
    pub fn set(&mut self, path: &[PathStep], value: Value) -> Result<()> {
        let d = defaults();
        let free_map = path.len() >= 2 && matches!(&path[0], PathStep::Key(k) if &**k == "keymaps");
        if !free_map {
            let Some(default) = d.at_path(path) else {
                return Err(Error::new("session.preferences.unknown").arg("path", path_text(path)));
            };
            if default.kind() != value.kind() {
                return Err(Error::new("session.preferences.kind").arg("path", path_text(path)));
            }
        }
        self.tree = self.tree.with_path(path, value)?;
        Ok(())
    }

    /// The project the last session ended on (`general.reopenLast`): the first recent folder that still exists
    pub fn last_project(&self) -> Option<PathBuf> {
        if !self.bool(&["general", "reopenLast"]).unwrap_or(false) {
            return None;
        }
        self.tree
            .get("recent")?
            .items()
            .filter_map(|e| e.get("folder").and_then(Value::as_text))
            .map(PathBuf::from)
            .find(|p| p.is_dir())
    }

    /// Puts a project first in the recent list (at most 10)
    pub fn add_recent(&mut self, name: &str, folder: &str) {
        let now = crate::lock::now_seconds().to_string();
        let entry =
            Value::object([("name", Value::text(name)), ("folder", Value::text(folder)), ("opened", Value::text(now))]);
        let mut list = vec![entry];
        if let Some(old) = self.tree.get("recent") {
            list.extend(old.items().filter(|e| e.get("folder").and_then(Value::as_text) != Some(folder)).cloned());
        }
        list.truncate(10);
        self.tree = self.tree.with("recent", Value::list(list));
    }
}
