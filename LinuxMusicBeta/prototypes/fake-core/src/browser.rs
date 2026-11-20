//! Owner: agent C. Fake data for the browser: the user's and the factory folders (samples, presets, projects, the
//! current project's files) merged in one tree each (F-BRW-002), the search with words and "quoted phrases"
//! (F-BRW-004), and the fake costs of searching, preparing a preview (I-7.3) and rescanning (H15).
//! The plugins (instruments and effects) come from `plugins.rs`; the presets of each plugin too, so the browser and the
//! instrument window show the same lists.

use std::sync::{Arc, LazyLock};

use crate::peaks::Peaks;
use crate::{SAMPLE_RATE, plugins};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Instruments,
    Effects,
    Samples,
    Presets,
    Projects,
    /// the current project's folder
    ProjectFiles,
    Favourites,
}

impl Page {
    pub const ALL: [Page; 7] =
        [Page::Instruments, Page::Effects, Page::Samples, Page::Presets, Page::Projects, Page::ProjectFiles, Page::Favourites];
    /// label key
    pub fn title(self) -> &'static str {
        match self {
            Page::Instruments => "Instruments",
            Page::Effects => "Effects",
            Page::Samples => "Samples",
            Page::Presets => "Presets",
            Page::Projects => "Projects",
            Page::ProjectFiles => "Project files",
            Page::Favourites => "Favourites",
        }
    }
}

/// The recognised file types (F-BRW-003)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Audio,
    Midi,
    Project,
    Preset,
    SoundFont,
    /// a GUS patch
    Patch,
    /// a Neural Amp Modeler model
    Nam,
    Other,
}

impl Kind {
    /// label key
    pub fn label(self) -> &'static str {
        match self {
            Kind::Folder => "Folder",
            Kind::Audio => "Audio",
            Kind::Midi => "MIDI",
            Kind::Project => "Project",
            Kind::Preset => "Preset",
            Kind::SoundFont => "SoundFont",
            Kind::Patch => "Patch",
            Kind::Nam => "Amp model",
            Kind::Other => "File",
        }
    }
    /// a file of this kind plays a preview (samples play the file; the rest a long note at the default key)
    pub fn previews(self) -> bool {
        matches!(self, Kind::Audio | Kind::Preset | Kind::SoundFont | Kind::Patch)
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u32,
    pub parent: Option<u32>,
    pub page: Page,
    pub name: String,
    pub kind: Kind,
    /// from the factory folders (shown after the user's, under a "Factory files" line)
    pub factory: bool,
    /// the full path, for "show in the file manager"
    pub path: String,
    pub size_kb: u32,
    // audio
    pub seconds: f64,
    pub rate: u32,
    pub channels: u8,
    pub bits: u8,
    pub bpm: Option<u32>,
    /// a preset's plugin
    pub plugin: Option<&'static str>,
    pub hidden: bool,
}

impl Entry {
    pub fn is_folder(&self) -> bool {
        self.kind == Kind::Folder
    }
    /// The format line of the detail pane
    pub fn format(&self) -> String {
        match self.kind {
            Kind::Audio => format!(
                "{:.1} s · {} kHz · {} bit · {}",
                self.seconds,
                self.rate as f64 / 1000.0,
                self.bits,
                if self.channels == 1 { "mono" } else { "stereo" }
            ),
            _ if self.size_kb >= 1024 => format!("{:.1} MB", self.size_kb as f64 / 1024.0),
            _ => format!("{} KB", self.size_kb),
        }
    }
}

/// F-ARR-011: dropped audio longer than this becomes an audio track; shorter, a sampler on an instrument track
pub const LONG_AUDIO_SECONDS: f64 = 10.0;

const USER: &str = "/home/ana/LinuxMusic";
const FACTORY: &str = "/usr/share/linuxmusic";

struct Builder {
    v: Vec<Entry>,
}

impl Builder {
    fn add(&mut self, parent: Option<u32>, page: Page, name: &str, kind: Kind, factory: bool) -> u32 {
        let id = self.v.len() as u32 + 1;
        let base = parent.map(|p| self.v[p as usize - 1].path.clone()).unwrap_or_else(|| {
            let root = if factory { FACTORY } else { USER };
            let sub = match page {
                Page::Samples => "samples",
                Page::Presets => "presets",
                Page::Projects => "projects",
                Page::ProjectFiles => "projects/Minha música",
                _ => "",
            };
            format!("{root}/{sub}")
        });
        let seed = id as u64 * 2654435761 >> 7;
        let looped = name.to_lowercase().contains("loop");
        let seconds = match kind {
            Kind::Audio if looped => 4.0 + (seed % 5) as f64 * 2.0,
            Kind::Audio if name.contains("Take") || name.contains("Voz") || name.contains("referência") => 30.0 + (seed % 90) as f64,
            Kind::Audio => 0.2 + (seed % 30) as f64 / 10.0,
            _ => 0.0,
        };
        let size_kb = match kind {
            Kind::Audio => (seconds * 48.0 * 2.0 * 3.0) as u32 + 1,
            Kind::SoundFont => 140_000 + (seed % 9000) as u32,
            Kind::Project => 40 + (seed % 300) as u32,
            Kind::Folder => 0,
            _ => 2 + (seed % 60) as u32,
        };
        self.v.push(Entry {
            id,
            parent,
            page,
            name: name.to_string(),
            kind,
            factory,
            path: format!("{base}/{name}"),
            size_kb,
            seconds,
            rate: if seed % 3 == 0 { 44_100 } else { 48_000 },
            channels: if name.contains("kick") || name.contains("snare") || name.contains("Voz") { 1 } else { 2 },
            bits: if seed % 2 == 0 { 24 } else { 16 },
            bpm: looped.then_some(90 + (seed % 5) as u32 * 10),
            plugin: None,
            hidden: name.starts_with('.'),
        });
        id
    }
    fn files(&mut self, parent: u32, page: Page, kind: Kind, factory: bool, names: &[&str]) {
        for n in names {
            self.add(Some(parent), page, n, kind, factory);
        }
    }
}

static LIBRARY: LazyLock<Vec<Entry>> = LazyLock::new(|| {
    let mut b = Builder { v: Vec::new() };
    use Kind::*;
    use Page::*;
    // samples: the user's and the factory's, merged (same subfolders join)
    let drums = b.add(None, Samples, "Baterias", Folder, false);
    b.files(drums, Samples, Audio, false, &["kick-acustico.wav", "snare-brush.wav", "hihat-aberto.flac", "Loop funk 95.wav"]);
    let vozes = b.add(None, Samples, "Vozes", Folder, false);
    b.files(vozes, Samples, Audio, false, &["Voz guia (Take 3).wav", "coro-aah.wav", "respiração.wav"]);
    b.add(None, Samples, ".cache-picos", Folder, false);
    b.add(None, Samples, "ruído-de-fita.ogg", Audio, false);
    for (folder, files) in [
        ("basses", &["bass01.ogg", "bass_slap.ogg", "synth_bass.ogg"][..]),
        ("beats", &["beat01_120bpm loop.ogg", "breakbeat loop.ogg"][..]),
        ("drums", &["kick01.ogg", "kick02.ogg", "snare01.ogg", "clap01.ogg", "hihat_closed.ogg", "crash01.ogg", "tom_low.ogg"][..]),
        ("drumsynth", &["tr808 kick.ds", "tr909 snare.ds", "tr909 kick.ds"][..]),
        ("effects", &["riser.ogg", "laser.ogg", "vinyl crackle loop.ogg"][..]),
        ("instruments", &["piano_c4.ogg", "string_pad.ogg", "harp_c5.ogg"][..]),
        ("ir", &["sala pequena.wav", "plate.wav", "gabinete 2x12.wav"][..]),
        ("waveforms", &["saw.wav", "square.wav"][..]),
    ] {
        let f = b.add(None, Samples, folder, Folder, true);
        b.files(f, Samples, Audio, true, files);
    }
    let sf = b.add(None, Samples, "soundfonts", Folder, true);
    b.files(sf, Samples, SoundFont, true, &["FluidR3Mono_GM.sf3"]);
    let nam = b.add(None, Samples, "amp models", Folder, true);
    b.files(nam, Samples, Nam, true, &["Clean Twin.nam", "Plexi crunch.nam"]);
    // presets: a folder per plugin, the factory presets from the descriptors, and the user's
    for d in plugins::all() {
        if d.presets.is_empty() {
            continue;
        }
        let f = b.add(None, Presets, d.id, Folder, true);
        for p in &d.presets {
            let id = b.add(Some(f), Presets, p.name, Preset, true);
            b.v[id as usize - 1].plugin = Some(d.id);
        }
    }
    let mine = b.add(None, Presets, "TripleOscillator", Folder, false);
    for n in ["Meu baixo", "Lead do refrão"] {
        let id = b.add(Some(mine), Presets, n, Preset, false);
        b.v[id as usize - 1].plugin = Some("TripleOscillator");
    }
    // projects: the user's, then the factory's demos, templates and tutorial
    for n in ["Minha música.lmp", "Ensaio 14-11.lmp", "Gravação ao vivo.lmp"] {
        b.add(None, Projects, n, Project, false);
    }
    let demos = b.add(None, Projects, "demos", Folder, true);
    b.files(demos, Projects, Project, true, &["Shorty - Fat Bass.lmp", "Shorty - Breakbeat.lmp", "Shorty - Pads.lmp", "Shorty - Acid.lmp"]);
    let tpl = b.add(None, Projects, "templates", Folder, true);
    b.files(tpl, Projects, Project, true, &["Banda (bateria, baixo, guitarra, voz).lmp", "Podcast.lmp", "Eletrônico.lmp", "Orquestra.lmp", "Vazio.lmp"]);
    b.add(None, Projects, "Tutorial.lmp", Project, true);
    b.add(None, Projects, "arranjo.mid", Midi, false);
    // the current project's folder
    let takes = b.add(None, ProjectFiles, "takes", Folder, false);
    b.files(takes, ProjectFiles, Audio, false, &["Voz principal - Take 1.wav", "Voz principal - Take 2.wav", "Voz principal - Take 3.wav", "Guitarra - Take 1.wav"]);
    b.add(None, ProjectFiles, "Minha música.lmp", Project, false);
    b.add(None, ProjectFiles, "referência.mp3", Audio, false);
    b.v
});

pub fn library() -> &'static [Entry] {
    &LIBRARY
}

pub fn entry(id: u32) -> Option<&'static Entry> {
    LIBRARY.get((id as usize).checked_sub(1)?).filter(|e| e.id == id)
}

/// A folder's (or a page's top) children: the user's before the factory's, folders first, names without case
/// (F-BRW-002). Hidden files only when asked.
pub fn children(page: Page, parent: Option<u32>, hidden: bool) -> Vec<&'static Entry> {
    let mut v: Vec<&Entry> =
        LIBRARY.iter().filter(|e| e.page == page && e.parent == parent && (hidden || !e.hidden)).collect();
    v.sort_by_key(|e| (e.factory, !e.is_folder(), e.name.to_lowercase()));
    v
}

/// The query's terms: words, and "phrases in quotes" kept whole (F-BRW-004)
pub fn terms(query: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut phrases = Vec::new();
    for (i, part) in query.split('"').enumerate() {
        if i % 2 == 1 {
            let p = part.trim().to_lowercase();
            if !p.is_empty() {
                phrases.push(p);
            }
        } else {
            words.extend(part.split_whitespace().map(str::to_lowercase));
        }
    }
    words.extend(phrases);
    words
}

/// Every term in the name
pub fn matches(name: &str, terms: &[String]) -> bool {
    let n = name.to_lowercase();
    !terms.is_empty() && terms.iter().all(|t| n.contains(t.as_str()))
}

/// The search over the library (F-BRW-004): the ids of the entries whose names have every term, files and folders
/// (the plugins are searched by the browser from `plugins::all()`). The browser runs it in the background and a new
/// query cancels the old one.
pub fn search(terms: &[String]) -> Vec<u32> {
    LIBRARY.iter().filter(|e| !e.hidden && matches(&e.name, terms)).map(|e| e.id).collect()
}

/// How long the fake takes to search (a slow disk), to prepare an entry's preview (decode the sample, or build the
/// preset's processor, I-7.3), and to rescan the plugins (H15): big files pass 200 ms, so the loading shows.
pub const SEARCH_MS: u64 = 350;
pub const RESCAN_MS: u64 = 1600;
pub fn preview_ms(id: u32) -> u64 {
    entry(id).map_or(20, |e| (u64::from(e.size_kb) / 20).clamp(20, 1500))
}

/// The preview's peaks (the fake decodes nothing: it draws a deterministic waveform)
pub fn preview_peaks(id: u32) -> Arc<Peaks> {
    let seconds = entry(id).map_or(2.0, |e| if e.kind == Kind::Audio { e.seconds.max(0.2) } else { 2.0 });
    Arc::new(Peaks::synthesize(u64::from(id) + 3, seconds, SAMPLE_RATE))
}

/// What a rescan finds: a third-party plugin that crashes the scan is blocked until the user asks again (H15)
pub const BLOCKED_BY_SCAN: &[&str] = &["Broken Synth (LV2)"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_keep_quoted_phrases() {
        assert_eq!(terms(r#"kick "tr808 kick" ds"#), ["kick", "ds", "tr808 kick"]);
        assert!(matches("tr808 kick.ds", &terms(r#""tr808 kick""#)));
        assert!(!matches("tr909 kick.ds", &terms(r#""tr808 kick""#)));
        assert!(!matches("anything", &[]));
        assert!(search(&terms("kick")).iter().all(|id| entry(*id).unwrap().name.to_lowercase().contains("kick")));
    }

    #[test]
    fn user_before_factory_folders_first() {
        let top = children(Page::Samples, None, false);
        let first_factory = top.iter().position(|e| e.factory).unwrap();
        assert!(top[..first_factory].iter().all(|e| !e.factory));
        assert!(top[0].is_folder());
        assert!(!top.iter().any(|e| e.hidden));
        assert!(children(Page::Samples, None, true).iter().any(|e| e.hidden));
    }

    #[test]
    fn every_entry_finds_itself() {
        for e in library() {
            assert_eq!(entry(e.id).unwrap().id, e.id);
            if let Some(p) = e.parent {
                assert!(entry(p).unwrap().is_folder());
            }
        }
        assert!(library().iter().any(|e| e.kind == Kind::Audio && e.seconds > LONG_AUDIO_SECONDS));
    }
}
