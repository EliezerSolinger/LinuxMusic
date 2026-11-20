//! The browser service (addition of 20/11 to I-15): the files, samples, presets, projects and plugins the browser
//! shows are the core's — the server's, when the interface is a client — so listing, searching and preparing a
//! preview go through the port, as data (F-BRW-001..012).

use std::sync::Arc;

use lm_state::{Resource, Value};

/// The browser's pages
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BrowsePage {
    Instruments,
    Effects,
    Samples,
    Presets,
    Projects,
    ProjectFiles,
    Favorites,
}

/// An entry's id, stable while the library does not change
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    Folder,
    Audio,
    Midi,
    Preset,
    Project,
    Plugin,
    Other,
}

/// One line of the browser
#[derive(Clone, Debug, PartialEq)]
pub struct BrowseEntry {
    pub id: EntryId,
    pub parent: Option<EntryId>,
    pub name: Arc<str>,
    pub kind: EntryKind,
    /// From the factory folders (shown after the user's, in the same tree)
    pub factory: bool,
    pub hidden: bool,
    pub favorite: bool,
    /// What the detail panel shows, as data: `"path"`, `"seconds"`, `"rate"`, `"bits"`, `"channels"`, `"bpm"`,
    /// a plugin's `"category"`, `"third_party"`, `"blocked"` (it brought the scan down, H15)...
    pub info: Value,
}

/// A plugin the program has (`CorePort::plugins`): the browser's Instruments and Effects pages carry the same id in
/// an entry's `info.plugin`, the instrument window's "Swap" and the effect chain's "add" list them
#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    pub id: Arc<str>,
    /// The translation key of its name (rule 7)
    pub name: Arc<str>,
    pub instrument: bool,
    /// Its category's key (F-FX-036): `"synth"`, `"dynamics"`...
    pub category: Arc<str>,
    /// Hosted outside the program (I-9.6)
    pub third_party: bool,
    /// Its latency in samples at the project's rate, as the PDC counts it
    pub latency: u32,
}

/// A prepared preview: its sound is the engine's (I-7.3); its waveform comes by `CorePort::peaks` on `resource`
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub entry: EntryId,
    pub resource: Resource,
    pub length: i64,
    pub sample_rate: u32,
}
