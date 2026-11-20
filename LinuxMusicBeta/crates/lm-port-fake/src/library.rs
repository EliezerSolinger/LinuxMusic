//! The fake's side of the session that is not the project: the browser's library (the user's and the factory
//! folders in one tree, F-BRW-002; search with words and "quoted phrases", F-BRW-004; favorites), the preferences
//! tree, the session's status and the devices.

use std::collections::BTreeSet;
use std::sync::Arc;

use lm_base::Result;
use lm_port::{BrowseEntry, BrowsePage, DeviceInfo, EntryId, EntryKind};
use lm_state::Value;

use crate::demo;

#[derive(Debug)]
pub struct Library {
    entries: Vec<(BrowsePage, BrowseEntry)>,
    favorites: BTreeSet<EntryId>,
}

fn info(pairs: &[(&str, Value)]) -> Value {
    Value::object(pairs.iter().map(|(k, v)| (*k, v.clone())))
}

impl Library {
    pub fn new() -> Library {
        let mut entries = Vec::new();
        let mut next = 1u64;
        let mut add = |page: BrowsePage,
                       parent: Option<EntryId>,
                       name: &str,
                       kind: EntryKind,
                       factory: bool,
                       hidden: bool,
                       extra: Value| {
            let id = EntryId(next);
            next += 1;
            entries.push((
                page,
                BrowseEntry { id, parent, name: Arc::from(name), kind, factory, hidden, favorite: false, info: extra },
            ));
            id
        };
        // plugins, by category
        for (id, instrument, _) in demo::PLUGINS {
            let page = if *instrument { BrowsePage::Instruments } else { BrowsePage::Effects };
            add(
                page,
                None,
                id,
                EntryKind::Plugin,
                true,
                false,
                info(&[
                    ("plugin", Value::text(*id)),
                    ("category", Value::text(if *instrument { "synth" } else { "dynamics" })),
                    ("third_party", Value::bool(false)),
                ]),
            );
        }
        add(
            BrowsePage::Effects,
            None,
            "Old Tape (LV2)",
            EntryKind::Plugin,
            false,
            false,
            info(&[("third_party", Value::bool(true))]),
        );
        add(
            BrowsePage::Effects,
            None,
            "Crashy Verb (LV2)",
            EntryKind::Plugin,
            false,
            false,
            info(&[("third_party", Value::bool(true)), ("blocked", Value::bool(true))]),
        );
        // samples: the user's folder, then the factory's
        let user =
            add(BrowsePage::Samples, None, "Minhas amostras", EntryKind::Folder, false, false, Value::empty_object());
        let drums =
            add(BrowsePage::Samples, Some(user), "Bateria", EntryKind::Folder, false, false, Value::empty_object());
        for (n, (name, secs, bpm)) in [
            ("kick 01.wav", 0.4, 0),
            ("snare 02.wav", 0.5, 0),
            ("loop funk 94.wav", 8.2, 94),
            ("hat aberto.flac", 0.9, 0),
        ]
        .iter()
        .enumerate()
        {
            add(
                BrowsePage::Samples,
                Some(drums),
                name,
                EntryKind::Audio,
                false,
                false,
                info(&[
                    ("path", Value::text(format!("~/Música/Amostras/Bateria/{name}"))),
                    ("seconds", Value::real(*secs).unwrap_or_default()),
                    ("rate", Value::int(48000)),
                    ("bits", Value::int(24)),
                    ("channels", Value::int(if n == 2 { 2 } else { 1 })),
                    ("bpm", Value::int(*bpm)),
                ]),
            );
        }
        add(BrowsePage::Samples, Some(user), ".DS_Store", EntryKind::Other, false, true, Value::empty_object());
        add(
            BrowsePage::Samples,
            Some(user),
            "voz guia (longa).wav",
            EntryKind::Audio,
            false,
            false,
            info(&[
                ("seconds", Value::real(214.0).unwrap_or_default()),
                ("rate", Value::int(48000)),
                ("bits", Value::int(24)),
                ("channels", Value::int(1)),
            ]),
        );
        let factory = add(BrowsePage::Samples, None, "Fábrica", EntryKind::Folder, true, false, Value::empty_object());
        for name in ["bass C2.wav", "pad Am.ogg", "clap 808.wav"] {
            add(
                BrowsePage::Samples,
                Some(factory),
                name,
                EntryKind::Audio,
                true,
                false,
                info(&[
                    ("seconds", Value::real(1.5).unwrap_or_default()),
                    ("rate", Value::int(44100)),
                    ("bits", Value::int(16)),
                    ("channels", Value::int(2)),
                ]),
            );
        }
        // presets, projects, the project's files
        for (plugin, name) in [
            ("tripleoscillator", "Baixo gordo"),
            ("tripleoscillator", "Pad suave"),
            ("avldrums", "Kit rock"),
            ("compressor", "Voz"),
        ] {
            add(
                BrowsePage::Presets,
                None,
                name,
                EntryKind::Preset,
                true,
                false,
                info(&[("plugin", Value::text(plugin))]),
            );
        }
        for name in ["Ensaio 12-11", "Show ao vivo", "Demo nova"] {
            add(
                BrowsePage::Projects,
                None,
                name,
                EntryKind::Project,
                false,
                false,
                info(&[("path", Value::text(format!("~/Música/LinuxMusic/{name}")))]),
            );
        }
        let audio =
            add(BrowsePage::ProjectFiles, None, "audio", EntryKind::Folder, false, false, Value::empty_object());
        for n in 1..=4 {
            add(
                BrowsePage::ProjectFiles,
                Some(audio),
                &format!("take-{n}.rf64"),
                EntryKind::Audio,
                false,
                false,
                info(&[
                    ("seconds", Value::real(60.0).unwrap_or_default()),
                    ("rate", Value::int(48000)),
                    ("bits", Value::int(32)),
                    ("channels", Value::int(1)),
                ]),
            );
        }
        Library { entries, favorites: BTreeSet::new() }
    }

    fn with_favorite(&self, e: &BrowseEntry) -> BrowseEntry {
        BrowseEntry { favorite: self.favorites.contains(&e.id), ..e.clone() }
    }

    /// An entry's children, folders first, by name without case; the user's before the factory's
    pub fn children(&self, page: BrowsePage, parent: Option<EntryId>, hidden: bool) -> Vec<BrowseEntry> {
        let mut v: Vec<BrowseEntry> = if page == BrowsePage::Favorites {
            self.entries
                .iter()
                .filter(|(_, e)| self.favorites.contains(&e.id))
                .map(|(_, e)| self.with_favorite(e))
                .collect()
        } else {
            self.entries
                .iter()
                .filter(|(p, e)| *p == page && e.parent == parent && (hidden || !e.hidden))
                .map(|(_, e)| self.with_favorite(e))
                .collect()
        };
        v.sort_by_key(|e| (e.factory, e.kind != lm_port::EntryKind::Folder, e.name.to_lowercase()));
        v
    }

    pub fn entry(&self, id: EntryId) -> Option<BrowseEntry> {
        self.entries.iter().find(|(_, e)| e.id == id).map(|(_, e)| self.with_favorite(e))
    }

    /// Words, and "quoted phrases" that must appear whole; without case or accents
    pub fn search(&self, query: &str) -> Vec<BrowseEntry> {
        let fold = |s: &str| {
            s.to_lowercase()
                .chars()
                .map(|c| match c {
                    'á' | 'à' | 'â' | 'ã' => 'a',
                    'é' | 'ê' => 'e',
                    'í' => 'i',
                    'ó' | 'ô' | 'õ' => 'o',
                    'ú' => 'u',
                    'ç' => 'c',
                    c => c,
                })
                .collect::<String>()
        };
        let mut terms = Vec::new();
        for (n, part) in query.split('"').enumerate() {
            if n % 2 == 1 {
                if !part.trim().is_empty() {
                    terms.push(fold(part.trim()));
                }
            } else {
                terms.extend(part.split_whitespace().map(fold));
            }
        }
        if terms.is_empty() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|(_, e)| {
                !e.hidden && e.kind != EntryKind::Folder && terms.iter().all(|t| fold(&e.name).contains(t.as_str()))
            })
            .map(|(_, e)| self.with_favorite(e))
            .collect()
    }

    pub fn set_favorite(&mut self, id: EntryId, on: bool) -> Result<()> {
        if !self.entries.iter().any(|(_, e)| e.id == id) {
            return Err(lm_base::Error::new("port.browse.unknown_entry"));
        }
        if on {
            self.favorites.insert(id);
        } else {
            self.favorites.remove(&id);
        }
        Ok(())
    }
}

/// The preferences tree (I-16): the keys the settings screen reads and writes
pub fn preferences() -> Value {
    let o = |pairs: Vec<(&str, Value)>| Value::object(pairs);
    o(vec![
        ("language", Value::text("pt-BR")),
        ("theme", Value::text("dark")),
        ("projectsFolder", Value::text("~/Música/LinuxMusic")),
        (
            "audio",
            o(vec![
                ("system", Value::text("alsa")),
                ("input", Value::text("hw:0")),
                ("output", Value::text("hw:0")),
                ("rate", Value::int(48000)),
                ("block", Value::int(256)),
                ("driftCorrection", Value::bool(false)),
                ("latencyFrames", Value::int(0)),
            ]),
        ),
        ("midi", o(vec![("inputs", Value::list([Value::text("Teclado USB")])), ("clock", Value::bool(false))])),
        ("interface", o(vec![("followCursor", Value::bool(false)), ("scale", Value::real(1.0).unwrap_or_default())])),
        ("metronome", o(vec![("when", Value::text("recording")), ("countInBars", Value::int(1))])),
        ("recording", o(vec![("format", Value::text("rf64-24")), ("folder", Value::text("audio"))])),
        ("shortcuts", Value::empty_object()),
        ("askBeforeClosing", Value::bool(true)),
    ])
}

/// The session's status: the project's file and access, recent projects, interrupted takes, safe mode
pub fn status() -> Value {
    let o = |pairs: Vec<(&str, Value)>| Value::object(pairs);
    o(vec![
        (
            "project",
            o(vec![
                ("name", Value::text("Demo")),
                ("folder", Value::text("~/Música/LinuxMusic/Demo")),
                ("readOnly", Value::bool(false)),
                ("lockedBy", Value::NULL),
            ]),
        ),
        (
            "recent",
            Value::list(["Ensaio 12-11", "Show ao vivo"].iter().map(|n| {
                o(vec![("name", Value::text(*n)), ("folder", Value::text(format!("~/Música/LinuxMusic/{n}")))])
            })),
        ),
        ("interruptedTakes", Value::empty_list()),
        (
            "templates",
            Value::list([o(vec![
                ("name", Value::text("Banda")),
                ("folder", Value::text("templates/banda")),
                ("factory", Value::bool(true)),
            ])]),
        ),
        ("safeMode", Value::bool(false)),
        ("safeModePlugins", Value::empty_list()),
        ("lastExport", Value::NULL),
        ("autosave", Value::NULL),
    ])
}

pub fn devices() -> Vec<DeviceInfo> {
    vec![
        DeviceInfo {
            id: Arc::from("hw:0"),
            name: Arc::from("HDA Intel PCH (CX20590)"),
            inputs: 2,
            outputs: 2,
            rates: vec![44100, 48000],
            details: Value::NULL,
        },
        DeviceInfo {
            id: Arc::from("hw:1"),
            name: Arc::from("Focusrite Scarlett 18i20"),
            inputs: 18,
            outputs: 20,
            rates: vec![44100, 48000, 96000],
            details: Value::NULL,
        },
    ]
}
