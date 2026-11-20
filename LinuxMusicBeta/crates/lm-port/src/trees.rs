//! The shapes of the two session trees the port gives as `Value` (`CorePort::status` and `CorePort::preferences`),
//! written down so the LocalPort, the RemotePort, the fake and the interface agree. A key not listed here is ignored
//! by the interface; a listed key that is absent means its default.
//!
//! `status()`:
//! ```text
//! project:          { name: text, folder: text, readOnly: bool,
//!                     lockedBy: null | { host: text, pid: int, since: text } }        (H10)
//! recent:           [ { name: text, folder: text, opened: text } ]                    (most recent first)
//! templates:        [ { name: text, folder: text, factory: bool } ]
//! interruptedTakes: [ { track: text (NodeId), file: text, seconds: real } ]           (U5)
//! safeMode:         bool                                                              (U21)
//! safeModePlugins:  [ { node: text (NodeId), plugin: text, reason: text } ]           (U21)
//! lastExport:       null | { file: text, seconds: real, peakDb: real, lufs: real, ok: bool, error: text }
//! autosave:         null | { at: text, file: text }
//! ```
//!
//! `preferences()` (I-16; changed by `set_preference(path, value)`):
//! ```text
//! language:          text ("pt-BR", "en")                 applies at the restart
//! theme:             text ("dark", "light")               applies at the restart
//! projectsFolder:    text
//! askBeforeClosing:  bool
//! audio:     { system: text ("alsa", "jack", "coreaudio"), input: text (device id), output: text (device id),
//!              rate: int, block: int, driftCorrection: bool, latencyFrames: int }
//! midi:      { inputs: [text], clock: bool }
//! interface: { followCursor: bool, scale: real }
//! metronome: { when: text ("always", "recording", "never"), countInBars: int }
//! recording: { format: text ("rf64-24", "rf64-32f"), folder: text }
//! shortcuts: { <action id>: text (keys, "secondary-s") }       only the ones the user changed
//! ```

/// The keys of `status()`
pub mod status {
    pub const PROJECT: &str = "project";
    pub const RECENT: &str = "recent";
    pub const TEMPLATES: &str = "templates";
    pub const INTERRUPTED_TAKES: &str = "interruptedTakes";
    pub const SAFE_MODE: &str = "safeMode";
    pub const SAFE_MODE_PLUGINS: &str = "safeModePlugins";
    pub const LAST_EXPORT: &str = "lastExport";
    pub const AUTOSAVE: &str = "autosave";
}

/// The keys of `preferences()`
pub mod preferences {
    pub const LANGUAGE: &str = "language";
    pub const THEME: &str = "theme";
    pub const PROJECTS_FOLDER: &str = "projectsFolder";
    pub const ASK_BEFORE_CLOSING: &str = "askBeforeClosing";
    pub const AUDIO: &str = "audio";
    pub const MIDI: &str = "midi";
    pub const INTERFACE: &str = "interface";
    pub const METRONOME: &str = "metronome";
    pub const RECORDING: &str = "recording";
    pub const SHORTCUTS: &str = "shortcuts";
}
