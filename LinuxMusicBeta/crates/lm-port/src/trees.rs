//! The shapes of the two session trees the port gives as `Value` (`CorePort::status` and `CorePort::preferences`),
//! written down so the LocalPort, the RemotePort, the fake and the interface agree. A key not listed here is ignored
//! by the interface; a listed key that is absent means its default.
//!
//! `status()`:
//! ```text
//! project:          { name: text, folder: text, readOnly: bool,
//!                     lockedBy: null | { host: text, pid: int, since: text },         (H10)
//!                     newerVersion: null | { version: text, format: int },           (H11)
//!                     migratedFrom: null | int,                                       (H11: the .bak kept)
//!                     unused: { files: int, mb: real } }                              (H4: files no root cites)
//! recent:           [ { name: text, folder: text, opened: text } ]                    (most recent first)
//! templates:        [ { name: text, folder: text, factory: bool } ]
//! interruptedTakes: [ { track: text, input: text, bar: int, file: text, seconds: real } ]   (U5)
//! safeMode:         bool                                                              (U21)
//! safeModePlugins:  [ { node: text (NodeId), plugin: text, reason: text } ]           (U21)
//! lastExport:       null | { file: text, files: int, seconds: real, peakDb: real, lufs: real, ok: bool,
//!                            error: text, identical: bool (false: a fast range, U37) }
//! autosave:         null | { at: text, file: text }
//! device:           null | { input: text, output: text, rate: int, block: int (agreed, C7),
//!                            latencyIn: int, latencyOut: int (frames), edgeConverter: bool }   (I-10)
//! realtime:         { rtkit: bool, rtprio: int, memlockKb: int (-1 unlimited), audioGroup: bool, user: text }  (H1)
//! midiPorts:        [ { client: text, port: text, backend: "seq"|"raw", alias: text, connected: bool,
//!                       readable: bool, writable: bool } ]                         (I-21.1: hotplug state)
//! ```
//!
//! `ui()`, the interface document (I-3.8; changed by `set_ui(path, value)`, no undo):
//! ```text
//! selection: { track: text (NodeId) | null, channel: text (NodeId, the master "m:0") | null,
//!              plugin: text (NodeId) | null, tracks: [text] }
//!            the focused track, the selected desk channel, the focused plugin (its window or rack slot); the
//!            selection-following MIDI targets read the first three (I-21.2)
//! ```
//!
//! `preferences()` (I-16; changed by `set_preference(path, value)`):
//! ```text
//! language:          text ("pt-BR", "en")                 applies at the restart
//! theme:             text ("dark", "light")               applies at the restart
//! projectsFolder:    text
//! askBeforeClosing:  bool
//! audio:     { system: text ("alsa", "jack", "coreaudio"), input: text (device id), output: text (device id),
//!              rate: int, block: int, driftCorrection: bool, latencyFrames: int,
//!              calibrations: [ { devices: "<input>|<output>", rate: int, edgeConverter: bool, residual: int,
//!                                spread: int, measured: text } ] }                    (H19)
//! midi:      { inputs: [text], clock: bool,
//!              ports: [ { client: text, port: text, backend: "seq"|"raw", alias: text,
//!                         role: "instrument"|"controller"|"both", input: bool, output: bool,
//!                         inputLatencyMs: real, outputLatencyMs: real } ] }            (I-21.1)
//! midiMap:   [ midi.mapping ]                         the machine's mappings (I-21.2)
//! controllers: [ { profile: "generic"|"mcu"|<controllers/<x>.json stem>, port: <alias>, bank: int } ]  (I-21.3)
//! interface: { followCursor: bool, scale: real }
//! metronome: { when: text ("always", "recording", "never"), countInBars: int }
//! recording: { format: text ("rf64-24", "rf64-32f"), folder: text }
//! shortcuts: { <action id>: text (keys, "secondary-s") }       only the ones the user changed
//! monitor:   { level: real (dB, -60..+12), dim: bool, cut: bool, mono: bool, side: bool }
//!            the master section's monitoring (the legacy MONITOR): what is heard, never mixed, exported or undone
//! general:   { reopenLast: bool, askDeleteTrack: bool, longAudioSeconds: int, previewsToEnd: bool,
//!              analogDesk: bool, newProjectRate: int }                          (F-CFG-010)
//! interface: also { tooltips: bool, compactHeaders: bool, noteNames: bool }   (F-CFG-016)
//! recording: also { lanes: bool (off: the simple recorder, P-11), quantizeMidi: bool }
//! plugins:   { windowsOnTop: bool, oneInstrumentWindow: bool, notLoaded: [text] }   (F-CFG-014)
//! folders:   { soundfont: text, allowed: [text] }                          (F-CFG-015, C9)
//! backup:    { autosaveMinutes: int (0 off), kept: int, onSave: bool }     (F-PRJ-010)
//! advanced:  { sanitize: bool }                                            (F-CFG-018, U20)
//! ```
//!
//! The parameters of the session's actions the settings, export and project screens run (agent D's):
//! ```text
//! project.open, project.new, project.save_as: { folder: text }   (new also: template: text, rate: int)
//! project.export:  { folder: text, name: text, formats: [text "wav"|"flac"|"ogg"|"mp3"], rate: int,
//!                    depth: int (16, 24, 32 = float), dither: bool, oggQuality: int, mp3Kbps: int,
//!                    normalize: { kind: "none"|"peak"|"lufs", target: real }, fadeIn: real, fadeOut: real,
//!                    range: null | { start: int, end: int (bars, 1-based, end exclusive), fast: bool,
//!                                    warmup: real }, asLoop: bool, repeats: int,
//!                    stems: null | { by: "tracks"|"channels", nodes: [text] } }    (I-13, U37, P-40)
//! takes.recover, takes.discard: { index: int } (absent: all)       safe_mode.enable: { node: text }
//! device.reopen, device.calibrate, devices.scan, project.export_zip, project.clean_unused,
//! project.save_template: no parameters        midi.assign_alias: { alias, client, port }
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
    pub const DEVICE: &str = "device";
    pub const REALTIME: &str = "realtime";
    pub const MIDI_PORTS: &str = "midiPorts";
}

/// The keys of `ui()`
pub mod ui {
    pub const SELECTION: &str = "selection";
    pub const TRACK: &str = "track";
    pub const CHANNEL: &str = "channel";
    pub const PLUGIN: &str = "plugin";
    pub const TRACKS: &str = "tracks";
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
    pub const MIDI_MAP: &str = "midiMap";
    pub const CONTROLLERS: &str = "controllers";
    pub const MONITOR: &str = "monitor";
}

/// The desk's live readings, `CorePort::display(channel)` with `kind` "rack" (the session's `lm_session::rack::display`,
/// the same layout): a value the node does not measure is NaN; `None` when the channel has no rack node
pub mod rack_display {
    pub const KIND: &str = "rack";
    /// The gain stage's saturation in the last block, 0 to 1 (the SAT LED)
    pub const SAT: usize = 0;
    /// How many blocks clipped so far (the CLIP LED lights when it changes)
    pub const CLIP: usize = 1;
    /// The channel's gate and compressor gain reductions, dB (≥ 0)
    pub const GATE_GR: usize = 2;
    pub const COMP_GR: usize = 3;
    /// The master's bus compressor gain reduction, dB (≥ 0)
    pub const BUS_GR: usize = 4;
    /// The master's correlation over ~300 ms, -1 to 1
    pub const CORRELATION: usize = 5;
    /// The goniometer: `POINTS` (left, right) pairs of the master's output
    pub const GONIO: usize = 6;
    pub const POINTS: usize = 64;
    pub const FLOATS: usize = GONIO + 2 * POINTS;
}
