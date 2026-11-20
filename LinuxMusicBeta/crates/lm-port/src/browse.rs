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

/// A prepared preview: its sound is the engine's (I-7.3); its waveform comes by `CorePort::peaks` on `resource`
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub entry: EntryId,
    pub resource: Resource,
    pub length: i64,
    pub sample_rate: u32,
}
