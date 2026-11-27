//! The browser entries the core has given out (I-15), and the actions that name one. An entry is an id the interface
//! (local or remote) holds; the file behind it is the session's: `plugin.load_preset {node, entry}`,
//! `project.open_entry <entry>` and `browser.reveal <entry>` become the same actions with the file, in
//! `Session::run_action`, so a remote client, the headless runner and the `LocalPort` all get it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lm_base::{Error, Result};
use lm_port::{BrowseEntry, BrowsePage, EntryId, EntryKind};
use lm_state::Value;

/// One entry of the browser the core has given out, to find its children and its file again
#[derive(Clone, Debug)]
pub(crate) struct Known {
    pub(crate) page: BrowsePage,
    pub(crate) path: Option<PathBuf>,
    /// Other folders listed together with `path` (the factory's folder of the same name, F-BRW-002)
    pub(crate) extra: Vec<PathBuf>,
    /// The children of a folder that is not on the disk (a plugin category, F-BRW-009/010)
    pub(crate) kids: Option<Vec<BrowseEntry>>,
    /// The folder is the factory's (what it lists is)
    pub(crate) factory: bool,
    /// When it was last given out (`BrowserEntries::stamp`): the oldest go when the cache is over its cap
    pub(crate) stamp: u64,
}

/// Entries kept; past this the oldest quarter is forgotten (M9). A forgotten id answers `port.browse.unknown_entry`,
/// and the interface lists its parent again to get fresh ones
pub(crate) const MAX_KNOWN: usize = 40_000;

/// Every entry given out, by id
#[derive(Debug, Default)]
pub struct BrowserEntries {
    pub(crate) known: BTreeMap<EntryId, Known>,
    /// The index the background scan makes (the last complete one); the scan replaces it when it ends
    pub(crate) index: std::sync::Arc<std::sync::Mutex<std::sync::Arc<crate::index::Index>>>,
    /// A scan is running (the worker clears it)
    pub(crate) indexing: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) indexed_at: Option<std::time::Instant>,
    clock: u64,
}

impl BrowserEntries {
    fn put(&mut self, id: EntryId, mut known: Known) {
        self.clock += 1;
        known.stamp = self.clock;
        self.known.insert(id, known);
        if self.known.len() > MAX_KNOWN {
            let mut stamps: Vec<u64> = self.known.values().map(|k| k.stamp).collect();
            let cut = stamps.len() / 4;
            let (_, limit, _) = stamps.select_nth_unstable(cut);
            let limit = *limit;
            self.known.retain(|_, k| k.stamp > limit);
        }
    }

    pub(crate) fn remember(&mut self, id: EntryId, page: BrowsePage, path: Option<PathBuf>) {
        self.put(id, Known { page, path, extra: Vec::new(), kids: None, factory: false, stamp: 0 });
    }

    /// A folder made of several (the user's and the factory's of the same name) or of entries given by the core
    pub(crate) fn remember_folder(
        &mut self,
        id: EntryId,
        page: BrowsePage,
        path: Option<PathBuf>,
        extra: Vec<PathBuf>,
        kids: Option<Vec<BrowseEntry>>,
        factory: bool,
    ) {
        self.put(id, Known { page, path, extra, kids, factory, stamp: 0 });
    }

    /// The file of an entry, when it has one
    pub fn path(&self, id: EntryId) -> Option<PathBuf> {
        self.known.get(&id).and_then(|k| k.path.clone())
    }

    /// The actions that name a browser entry with the entry's file in its place; the others, and the ones that
    /// already have the file, as they are. `port.browse.unknown_entry` for an id never given out
    pub fn resolve(&self, action: &str, params: Value) -> Result<Value> {
        let entry = match action {
            "plugin.load_preset" => params.get("entry").and_then(Value::as_int),
            "project.open_entry" | "browser.reveal" | "midi.import" | "resources.import" => {
                params.as_int().or_else(|| params.get("entry").and_then(Value::as_int))
            }
            _ => return Ok(params),
        };
        let Some(entry) = entry else { return Ok(params) };
        let path = self
            .path(EntryId(u64::try_from(entry).unwrap_or(0)))
            .ok_or_else(|| Error::new("port.browse.unknown_entry"))?;
        let text = Value::text(path.display().to_string());
        Ok(match action {
            "plugin.load_preset" => params.with("path", text).without("entry"),
            "project.open_entry" => Value::object([("folder", text)]),
            // The others keep what else they carry (`at`, `track`): only the entry becomes the file
            "midi.import" | "resources.import" if params.get("entry").is_some() => {
                params.with("path", text).without("entry")
            }
            _ => Value::object([("path", text)]),
        })
    }
}

// ---- recognised files (F-BRW-003) ----

/// The audio the program reads
pub const AUDIO_EXTENSIONS: [&str; 7] = ["wav", "flac", "ogg", "mp3", "aif", "aiff", "rf64"];

/// What a file is for the browser: its kind and its type's name (`info.type`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileType {
    pub kind: EntryKind,
    pub name: &'static str,
    /// Listed on the Samples page (what a sampler or a sound bank can open)
    pub sample: bool,
}

/// The type of a file by its extension (lower case or not); `None` for what the browser does not know
pub fn file_type(path: &Path) -> Option<FileType> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    let t = |kind, name, sample| Some(FileType { kind, name, sample });
    match ext.as_str() {
        e if AUDIO_EXTENSIONS.contains(&e) => t(EntryKind::Audio, "audio", true),
        "mid" | "midi" | "rmi" => t(EntryKind::Midi, "midi", false),
        "sf2" | "sf3" => t(EntryKind::Other, "soundfont", true),
        "sfz" => t(EntryKind::Other, "sfz", true),
        "pat" => t(EntryKind::Other, "patch", true),
        "gig" => t(EntryKind::Other, "gig", true),
        "nam" => t(EntryKind::Other, "nam", true),
        "ds" => t(EntryKind::Other, "drumsynth", true),
        "voc" => t(EntryKind::Other, "voc", true),
        "lv2" => t(EntryKind::Other, "lv2", false),
        "vst3" => t(EntryKind::Other, "vst3", false),
        "clap" => t(EntryKind::Other, "clap", false),
        "dll" | "vst" => t(EntryKind::Other, "vst", false),
        _ => None,
    }
}

// ---- the instrument a file goes to (item 3 of 26/11: one Sampler plugin) ----

/// The plugin that opens a type of file: "send to the instrument" for these loads the file there. The one table
pub const OPENED_BY: &[(&str, &str)] = &[
    ("soundfont", "sampler"),
    ("gig", "sampler"),
    ("patch", "sampler"),
    ("sfz", "sampler"),
    ("audio", "sampler"),
    ("drumsynth", "sampler"),
    ("nam", "ampandir"),
];

/// The plugin id that opens files of this type (`info.type`)
pub fn opened_by(file_type: &str) -> Option<&'static str> {
    OPENED_BY.iter().find(|(t, _)| *t == file_type).map(|(_, p)| *p)
}

// ---- plugin categories (F-BRW-009/010, F-FX-036) ----

/// A plugin's category (a key) and the name the browser shows for it
pub fn plugin_category(id: &str, instrument: bool) -> (&'static str, &'static str) {
    if instrument {
        match id {
            "tripleoscillator" | "monstro" | "watsyn" | "bitinvader" | "vibed" | "xpressive" | "lb302" | "organic"
            | "mallets" => ("synth", "Synths"),
            "sid" | "freeboy" | "nes" | "opulenz" | "sfxr" => ("chip", "Chip and retro"),
            "sampler" | "slicert" | "kicker" => ("sampler", "Samplers and drums"),
            _ => ("other", "Other"),
        }
    } else {
        match id {
            "compressor" | "dynamicsprocessor" | "lomm" | "amplifier" => ("dynamics", "Dynamics"),
            "eq" | "crossovereq" | "dualfilter" | "bassbooster" => ("filter", "Filters and EQ"),
            "waveshaper" | "slewdistortion" | "bitcrush" | "ampandir" => ("distortion", "Distortion"),
            "delay" | "multitapecho" | "reverbsc" | "flanger" | "dispersion" => {
                ("time", "Delay, reverb and modulation")
            }
            "stereomatrix" | "stereoenhancer" => ("stereo", "Stereo"),
            "granularpitchshifter" | "frequencyshifter" => ("pitch", "Pitch and frequency"),
            _ => ("other", "Other"),
        }
    }
}

/// The category of a third-party plugin
pub const THIRD_PARTY_CATEGORY: (&str, &str) = ("third_party", "Third party");

// ---- the factory content (F-BRW-002, F-BRW-012) ----

/// The folders of the factory content: samples, presets (`<plugin>/<name>.json`), projects, in this order of
/// search: `LINUXMUSIC_FACTORY`, `factory` beside the program, `../share/linuxmusic/factory`
pub fn factory_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = std::env::var_os("LINUXMUSIC_FACTORY") {
        out.push(PathBuf::from(d));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        out.push(dir.join("factory"));
        out.push(dir.join("../share/linuxmusic/factory"));
    }
    out.retain(|d| d.is_dir());
    out
}

/// A part of the factory content (`"samples"`, `"presets"`, `"projects"`) in the given factory folders
pub fn factory_part(factories: &[PathBuf], part: &str) -> Vec<PathBuf> {
    factories.iter().map(|f| f.join(part)).filter(|p| p.is_dir()).collect()
}

/// The entries of a folder and its factory twins merged (F-BRW-002): the folders of the same name become one (its
/// children are listed from all of them), a file of the user's hides the factory's of the same name, the user's come
/// first. Each entry is with the other folders of its name
pub fn merge_listing(
    user: Vec<(BrowseEntry, PathBuf)>,
    factory: Vec<(BrowseEntry, PathBuf)>,
) -> Vec<(BrowseEntry, PathBuf, Vec<PathBuf>)> {
    let mut out: Vec<(BrowseEntry, PathBuf, Vec<PathBuf>)> =
        user.into_iter().map(|(e, p)| (e, p, Vec::new())).collect();
    for (mut e, p) in factory {
        e.factory = true;
        if let Some(same) = out.iter_mut().find(|(x, _, _)| x.name == e.name && x.kind == e.kind) {
            if e.kind == EntryKind::Folder {
                same.2.push(p);
            }
            continue;
        }
        out.push((e, p, Vec::new()));
    }
    out
}

impl crate::session::Session {
    /// The SoundFont a MIDI file is played on, in a preview and on import: the preferences' (`defaultSoundFont`), else
    /// the first of the index
    pub fn default_soundfont(&self) -> Option<std::path::PathBuf> {
        if let Some(p) = self.preferences().text(&["defaultSoundFont"]).filter(|p| !p.is_empty()) {
            return Some(std::path::PathBuf::from(p));
        }
        let index = self.browser.index.lock().ok()?.clone();
        index.items.iter().find(|i| i.ty == "soundfont").map(|i| std::path::PathBuf::from(&i.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> BrowserEntries {
        let mut b = BrowserEntries::default();
        b.remember(EntryId(7), BrowsePage::Presets, Some(PathBuf::from("/tmp/a.json")));
        b.remember(EntryId(8), BrowsePage::Presets, None);
        b
    }

    #[test]
    fn an_entry_becomes_its_file() {
        let b = entries();
        let p =
            b.resolve("plugin.load_preset", Value::object([("node", Value::text("n:1")), ("entry", Value::int(7))]));
        let p = p.unwrap();
        assert_eq!(p.get("path").and_then(Value::as_text), Some("/tmp/a.json"));
        assert!(p.get("entry").is_none());
        let o = b.resolve("project.open_entry", Value::int(7)).unwrap();
        assert_eq!(o.get("folder").and_then(Value::as_text), Some("/tmp/a.json"));
        let r = b.resolve("browser.reveal", Value::object([("entry", Value::int(7))])).unwrap();
        assert_eq!(r.get("path").and_then(Value::as_text), Some("/tmp/a.json"));
    }

    /// M9: the cache is bounded; the newest entries survive
    #[test]
    fn the_known_entries_are_capped() {
        let mut b = BrowserEntries::default();
        let total = MAX_KNOWN as u64 + 10;
        for i in 0..total {
            b.remember(EntryId(i), BrowsePage::Samples, Some(PathBuf::from(format!("/x/{i}"))));
        }
        assert!(b.known.len() <= MAX_KNOWN);
        assert!(b.known.len() > MAX_KNOWN / 2);
        assert!(b.path(EntryId(total - 1)).is_some());
        assert!(b.path(EntryId(0)).is_none());
    }

    #[test]
    fn unknown_entries_and_other_actions() {
        let b = entries();
        assert_eq!(b.resolve("browser.reveal", Value::int(99)).unwrap_err().key(), "port.browse.unknown_entry");
        assert_eq!(b.resolve("browser.reveal", Value::int(8)).unwrap_err().key(), "port.browse.unknown_entry");
        let params = Value::object([("path", Value::text("/x"))]);
        assert_eq!(b.resolve("browser.reveal", params.clone()).unwrap(), params);
        assert_eq!(b.resolve("transport.play", Value::NULL).unwrap(), Value::NULL);
    }

    #[test]
    fn recognised_types() {
        let t = |p: &str| file_type(Path::new(p)).map(|f| (f.kind, f.name));
        assert_eq!(t("/x/a.WAV"), Some((EntryKind::Audio, "audio")));
        assert_eq!(t("/x/a.mid"), Some((EntryKind::Midi, "midi")));
        assert_eq!(t("/x/a.sf3"), Some((EntryKind::Other, "soundfont")));
        assert_eq!(t("/x/a.gig"), Some((EntryKind::Other, "gig")));
        assert_eq!(t("/x/a.nam"), Some((EntryKind::Other, "nam")));
        assert_eq!(t("/x/a.txt"), None);
        assert_eq!(t("/x/noext"), None);
        assert!(!file_type(Path::new("a.lv2")).unwrap().sample);
    }

    #[test]
    fn the_sampler_opens_the_banks() {
        assert_eq!(opened_by("soundfont"), Some("sampler"));
        assert_eq!(opened_by("gig"), Some("sampler"));
        assert_eq!(opened_by("midi"), None);
    }

    #[test]
    fn categories_cover_the_lists() {
        assert_eq!(plugin_category("sid", true).0, "chip");
        assert_eq!(plugin_category("compressor", false).0, "dynamics");
        assert_eq!(plugin_category("nothing", false).0, "other");
    }

    fn file(name: &str, kind: EntryKind) -> BrowseEntry {
        BrowseEntry {
            id: EntryId(1),
            parent: None,
            name: name.into(),
            kind,
            factory: false,
            hidden: false,
            favorite: false,
            info: Value::NULL,
        }
    }

    #[test]
    fn the_factory_merges_into_the_user_s() {
        let user = vec![
            (file("drums", EntryKind::Folder), PathBuf::from("/u/drums")),
            (file("a.wav", EntryKind::Audio), PathBuf::from("/u/a.wav")),
        ];
        let factory = vec![
            (file("drums", EntryKind::Folder), PathBuf::from("/f/drums")),
            (file("a.wav", EntryKind::Audio), PathBuf::from("/f/a.wav")),
            (file("b.wav", EntryKind::Audio), PathBuf::from("/f/b.wav")),
        ];
        let m = merge_listing(user, factory);
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].2, vec![PathBuf::from("/f/drums")]);
        assert!(!m[0].0.factory && !m[1].0.factory);
        assert!(m[2].0.factory && &*m[2].0.name == "b.wav");
    }

    #[test]
    fn entries_resolve_for_the_imports() {
        let b = entries();
        let r = b.resolve("midi.import", Value::object([("entry", Value::int(7)), ("at", Value::int(960))])).unwrap();
        assert_eq!(r.get("path").and_then(Value::as_text), Some("/tmp/a.json"));
        assert_eq!(r.get("at").and_then(Value::as_int), Some(960));
        // a call that already has its path goes through as it is
        let p = Value::object([("path", Value::text("/x.mid"))]);
        assert_eq!(b.resolve("midi.import", p.clone()).unwrap(), p);
    }
}
