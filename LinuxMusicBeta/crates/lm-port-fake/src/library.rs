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
        for (id, instrument, _) in demo::PLUGINS.iter().filter(|(id, ..)| !demo::NOTE_EFFECTS.contains(id)) {
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
                    ("category", Value::text(demo::category(id))),
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
        // The core's filters: `type:samples`, `tag:x` / `#x`, `is:fav` (not in the words)
        let mut kinds: Vec<EntryKind> = Vec::new();
        let mut fav = false;
        let mut tags: Vec<String> = Vec::new();
        terms.retain(|t| {
            if let Some(g) = t.strip_prefix("type:") {
                match g {
                    "instruments" | "effects" => kinds.push(EntryKind::Plugin),
                    "samples" => kinds.push(EntryKind::Audio),
                    "presets" => kinds.push(EntryKind::Preset),
                    "midi" => kinds.push(EntryKind::Midi),
                    "projects" => kinds.push(EntryKind::Project),
                    _ => {}
                }
                false
            } else if t == "is:fav" {
                fav = true;
                false
            } else if let Some(g) = t.strip_prefix("tag:").or_else(|| t.strip_prefix('#')) {
                tags.push(g.to_owned());
                false
            } else {
                true
            }
        });
        if terms.is_empty() && kinds.is_empty() && !fav && tags.is_empty() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|(_, e)| {
                !e.hidden
                    && e.kind != EntryKind::Folder
                    && (kinds.is_empty() || kinds.contains(&e.kind))
                    && (!fav || self.with_favorite(e).favorite)
                    && tags.iter().all(|t| fold(&e.name).contains(t.as_str()))
                    && terms.iter().all(|t| fold(&e.name).contains(t.as_str()))
            })
            .map(|(_, e)| self.with_favorite(e))
            .collect()
    }

    /// A user preset saved from a plugin (`plugin.save_preset`): on the Presets page, the user's (not factory)
    pub fn add_preset(&mut self, plugin: &str, name: &str) -> EntryId {
        let id = EntryId(self.entries.iter().map(|(_, e)| e.id.0).max().unwrap_or(0) + 1);
        self.entries.push((
            BrowsePage::Presets,
            BrowseEntry {
                id,
                parent: None,
                name: std::sync::Arc::from(name),
                kind: EntryKind::Preset,
                factory: false,
                hidden: false,
                favorite: false,
                info: info(&[("plugin", Value::text(plugin))]),
            },
        ));
        id
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
                ("manualOffsets", Value::empty_list()),
                // H19: the residual per device pair, rate and edge converter
                (
                    "calibrations",
                    Value::list([o(vec![
                        ("devices", Value::text("hw:0|hw:0")),
                        ("rate", Value::int(48000)),
                        ("edgeConverter", Value::bool(false)),
                        ("residual", Value::int(19)),
                        ("spread", Value::int(1)),
                        ("measured", Value::text("17/11")),
                    ])]),
                ),
            ]),
        ),
        (
            "midi",
            o(vec![
                ("inputs", Value::list([Value::text("Teclado USB")])),
                ("clock", Value::bool(false)),
                // I-21.1: a port is its client and port names, never ALSA's numbers
                (
                    "ports",
                    Value::list([
                        midi_port(
                            "Arturia KeyStep 32",
                            "Arturia KeyStep 32 MIDI 1",
                            "seq",
                            "Teclado",
                            "instrument",
                            true,
                            false,
                            0.0,
                        ),
                        midi_port(
                            "nanoKONTROL2",
                            "nanoKONTROL2 MIDI 1",
                            "seq",
                            "Controlador 1",
                            "controller",
                            true,
                            true,
                            0.0,
                        ),
                        midi_port("X-Touch", "X-Touch MIDI 1", "seq", "Mesa", "controller", true, true, 2.0),
                        midi_port(
                            "Scarlett 18i20 USB",
                            "Scarlett 18i20 USB MIDI 1",
                            "raw",
                            "",
                            "both",
                            false,
                            false,
                            0.0,
                        ),
                    ]),
                ),
            ]),
        ),
        ("midiMap", Value::empty_list()),
        ("osc", o(vec![("on", Value::bool(false)), ("port", Value::int(9000)), ("lan", Value::bool(false))])),
        (
            "controllers",
            Value::list([
                o(vec![("profile", Value::text("mcu")), ("port", Value::text("Mesa")), ("bank", Value::int(0))]),
                o(vec![
                    ("profile", Value::text("generic")),
                    ("port", Value::text("Controlador 1")),
                    ("bank", Value::int(0)),
                ]),
            ]),
        ),
        (
            "interface",
            o(vec![
                ("followCursor", Value::bool(false)),
                ("scale", Value::real(1.0).unwrap_or_default()),
                ("confirmTrackRemoval", Value::bool(true)),
                ("loopEditMode", Value::text("buttons")),
                ("scrollMode", Value::text("page")),
                ("stopBehavior", Value::text("origin")),
                ("channelMeter", Value::text("peak")),
            ]),
        ),
        ("metronome", o(vec![("when", Value::text("recording")), ("countInBars", Value::int(1))])),
        ("recording", o(vec![("format", Value::text("rf64-24")), ("folder", Value::text("audio"))])),
        ("keymaps", Value::empty_object()),
        ("askBeforeClosing", Value::bool(true)),
        (
            "monitor",
            o(vec![
                ("level", Value::real(0.0).unwrap_or_default()),
                ("dim", Value::bool(false)),
                ("cut", Value::bool(false)),
                ("mono", Value::bool(false)),
                ("side", Value::bool(false)),
            ]),
        ),
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
                ("newerVersion", Value::NULL),
                ("migratedFrom", Value::NULL),
                ("unused", o(vec![("files", Value::int(9)), ("mb", Value::real(412.0).unwrap_or_default())])),
            ]),
        ),
        (
            "recent",
            Value::list(
                [
                    ("Ensaio 12-11", "ontem"),
                    ("Trilha do curta", "18/11"),
                    ("Podcast 12", "17/11"),
                    ("Remix do Lucas", "15/11"),
                    ("Show ao vivo", "02/11"),
                ]
                .iter()
                .map(|(n, d)| {
                    o(vec![
                        ("name", Value::text(*n)),
                        ("folder", Value::text(format!("~/Música/LinuxMusic/{n}"))),
                        ("opened", Value::text(*d)),
                    ])
                }),
            ),
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
        (
            "device",
            o(vec![
                ("input", Value::text("hw:0")),
                ("output", Value::text("hw:0")),
                ("rate", Value::int(48000)),
                ("block", Value::int(256)),
                ("latencyIn", Value::int(314)),
                ("latencyOut", Value::int(314)),
                ("edgeConverter", Value::bool(false)),
            ]),
        ),
        (
            "realtime",
            o(vec![
                ("rtkit", Value::bool(false)),
                ("rtprio", Value::int(0)),
                ("memlockKb", Value::int(8192)),
                ("audioGroup", Value::bool(false)),
                ("user", Value::text("eli")),
            ]),
        ),
        (
            "midiPorts",
            Value::list([
                port_state("Arturia KeyStep 32", "Arturia KeyStep 32 MIDI 1", "seq", "Teclado", true, true, false),
                port_state("nanoKONTROL2", "nanoKONTROL2 MIDI 1", "seq", "Controlador 1", true, true, true),
                port_state("X-Touch", "X-Touch MIDI 1", "seq", "Mesa", false, true, true),
                port_state("Scarlett 18i20 USB", "Scarlett 18i20 USB MIDI 1", "raw", "", true, true, true),
                port_state("Midi Through", "Midi Through Port-0", "seq", "", true, true, true),
            ]),
        ),
    ])
}

#[allow(clippy::too_many_arguments)]
fn midi_port(
    client: &str,
    port: &str,
    backend: &str,
    alias: &str,
    role: &str,
    input: bool,
    output: bool,
    latency: f64,
) -> Value {
    Value::object([
        ("client", Value::text(client)),
        ("port", Value::text(port)),
        ("backend", Value::text(backend)),
        ("alias", Value::text(alias)),
        ("role", Value::text(role)),
        ("input", Value::bool(input)),
        ("output", Value::bool(output)),
        ("inputLatencyMs", Value::real(latency).unwrap_or_default()),
        ("outputLatencyMs", Value::real(0.0).unwrap_or_default()),
    ])
}

fn port_state(
    client: &str,
    port: &str,
    backend: &str,
    alias: &str,
    connected: bool,
    readable: bool,
    writable: bool,
) -> Value {
    Value::object([
        ("client", Value::text(client)),
        ("port", Value::text(port)),
        ("backend", Value::text(backend)),
        ("alias", Value::text(alias)),
        ("connected", Value::bool(connected)),
        ("readable", Value::bool(readable)),
        ("writable", Value::bool(writable)),
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
            details: Value::object([("kind", Value::text("audio")), ("reportedFrames", Value::int(32))]),
        },
        DeviceInfo {
            id: Arc::from("hw:1"),
            name: Arc::from("Focusrite Scarlett 18i20"),
            inputs: 18,
            outputs: 20,
            rates: vec![44100, 48000, 96000],
            details: Value::object([("kind", Value::text("audio")), ("reportedFrames", Value::int(58))]),
        },
        DeviceInfo {
            id: Arc::from("bluez_card.WH-1000XM4"),
            name: Arc::from("WH-1000XM4 (Bluetooth, PipeWire)"),
            inputs: 1,
            outputs: 2,
            rates: vec![16000, 48000],
            details: Value::object([
                ("kind", Value::text("audio")),
                ("bluetooth", Value::bool(true)),
                ("reportedFrames", Value::int(7200)),
            ]),
        },
        DeviceInfo {
            id: Arc::from("hw:2"),
            name: Arc::from("HDA NVidia (HDMI 0)"),
            inputs: 0,
            outputs: 8,
            rates: vec![44100, 48000],
            details: Value::object([("kind", Value::text("audio")), ("busyBy", Value::text("PipeWire"))]),
        },
    ]
}
