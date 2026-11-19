//! Owner: agent D. The fake core's services beyond the document: the project folder (open, save, save as, the
//! read-only lock H10/H11, take recovery U5, safe mode U21, "export as a single file"), the preferences tree
//! (spec item 4: a third tree, beside the project and the interface), the devices (U7, U17, H1, H19, I-10), the
//! export (I-13, U37, V18, H18), the notice queue (I-14) and the templates of a new project.
//!
//! Everything that may pass 200 ms is an operation (`Session::start`, I-2) on a worker; its result is applied in the
//! pump (`Services::pump`), never by the worker. Nothing here touches the disk: the folders and files are fake.
//! The session owns one `Services` (`session.services`).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::notices::{NoticeAction, Queue, Severity};
use crate::{NodeId, OperationHandle, Session, Target};

// ---------------------------------------------------------------------------------------------------------------
// The project

/// The schema version this program writes (H11)
pub const SCHEMA: u32 = 3;

#[derive(Clone, Debug, PartialEq)]
pub enum Access {
    ReadWrite,
    /// Saving, the journal and autosave are off; "Save as" is the way out (H10, H11)
    ReadOnly(ReadOnly),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReadOnly {
    /// `projeto.lock` of a live process, or of another machine (H10)
    Locked { machine: String, pid: u32, since: String, this_machine: bool },
    /// A node with a schema newer than this program's (H11)
    Newer { schema: u32, version: String },
}

/// A take that was being recorded when the program closed (U5): its `recording.json` says where it goes
#[derive(Clone, Debug, PartialEq)]
pub struct InterruptedTake {
    pub track: String,
    pub input: String,
    pub start_bar: u32,
    pub seconds: f64,
    pub loop_cuts: u32,
    /// None: not decided yet; Some(true): recovered; Some(false): discarded (the file stays in `audio/`)
    pub decision: Option<bool>,
}

/// A plugin turned off by safe mode (U21): it was running when the program stopped
#[derive(Clone, Debug, PartialEq)]
pub struct SafeModePlugin {
    pub plugin: String,
    pub track: String,
    /// English key: "stopped answering" (the watchdog) or "crashed" (the crash handler)
    pub why: &'static str,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct ProjectInfo {
    pub name: String,
    /// The project folder: `projeto.json`, `interface.json`, `audio/`, `backup/`
    pub folder: String,
    /// Fixed per project (spec item 8)
    pub rate: u32,
    pub template: &'static str,
    pub saved: Option<Instant>,
    pub modified: bool,
    pub access: Access,
    pub recovery: Vec<InterruptedTake>,
    pub safe_mode: Vec<SafeModePlugin>,
    /// Lines in `backup/diario.jsonl` since the last save (spec item 5)
    pub journal: u32,
    pub audio_files: u32,
    pub audio_mb: f64,
    /// Opened from an older schema: the original is kept as `projeto.json.v<N>.bak` before the first save (H11)
    pub migrated_from: Option<u32>,
}

impl ProjectInfo {
    pub fn read_only(&self) -> bool {
        matches!(self.access, Access::ReadOnly(_))
    }
    /// The folder's entries, as the save dialog previews them
    pub fn layout(&self) -> Vec<(&'static str, String)> {
        vec![
            ("projeto.json", "the project's tree".into()),
            ("interface.json", "windows, zoom, scrolling".into()),
            ("audio/", format!("{} | {:.0}", self.audio_files, self.audio_mb)),
            ("backup/", "autosaves and the journal".into()),
        ]
    }
}

fn demo_project() -> ProjectInfo {
    ProjectInfo {
        name: "Minha Música".into(),
        folder: "~/Música/LinuxMusic/Minha Música".into(),
        rate: crate::SAMPLE_RATE,
        template: "Band recording",
        saved: Some(Instant::now() - Duration::from_secs(12 * 60)),
        modified: true,
        access: Access::ReadWrite,
        recovery: Vec::new(),
        safe_mode: Vec::new(),
        journal: 37,
        audio_files: 64,
        audio_mb: 2110.0,
        migrated_from: None,
    }
}

/// What opening a recent project of the prototype runs into (each case of the program, to see the screens)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    Normal,
    /// another LinuxMusic has it open (H10)
    LockedElsewhere,
    /// a lock left by a process that died on this machine: removed, with a notice (H10)
    StaleLock,
    /// saved by a newer version (H11)
    Newer,
    /// the program closed while recording (U5)
    CrashedRecording,
    /// the program closed inside a plugin (U21)
    CrashedPlugin,
    /// saved by an older version: migrated, with the copy kept (H11)
    Older,
    /// the folder is gone
    Missing,
}

#[derive(Clone, Debug)]
pub struct RecentProject {
    pub name: String,
    pub folder: String,
    pub rate: u32,
    pub tracks: u32,
    pub minutes: f64,
    pub modified_ago: Duration,
    pub scenario: Scenario,
}

fn recent_examples() -> Vec<RecentProject> {
    let r = |name: &str, rate, tracks, minutes, h: u64, scenario| RecentProject {
        name: name.into(),
        folder: format!("~/Música/LinuxMusic/{name}"),
        rate,
        tracks,
        minutes,
        modified_ago: Duration::from_secs(h * 3600),
        scenario,
    };
    vec![
        r("Minha Música", 48_000, 64, 3.2, 0, Scenario::Normal),
        r("Ensaio de sábado", 48_000, 16, 41.0, 20, Scenario::CrashedRecording),
        r("Trilha do curta", 96_000, 38, 12.5, 30, Scenario::CrashedPlugin),
        r("Podcast 12", 44_100, 4, 58.0, 50, Scenario::LockedElsewhere),
        r("Remix do Lucas", 48_000, 22, 4.1, 80, Scenario::Newer),
        r("Demo antiga", 44_100, 9, 2.9, 900, Scenario::Older),
        r("Jam de terça", 48_000, 12, 18.0, 170, Scenario::StaleLock),
        r("Pendrive/Show ao vivo", 48_000, 24, 75.0, 2000, Scenario::Missing),
    ]
}

/// A template of a new project
#[derive(Clone, Debug)]
pub struct Template {
    /// English key: name
    pub name: &'static str,
    /// English key: what it has
    pub about: &'static str,
    pub tracks: &'static [&'static str],
}

pub const TEMPLATES: &[Template] = &[
    Template { name: "Empty", about: "No tracks; the mixing desk with its master", tracks: &[] },
    Template {
        name: "Band recording",
        about: "Eight audio tracks armed to the first inputs, a channel each, and two returns (reverb, delay)",
        tracks: &["Bumbo", "Caixa", "Overhead", "Baixo", "Guitarra", "Teclado", "Voz", "Voz 2"],
    },
    Template {
        name: "Beat",
        about: "AVL drums, a bass and a keyboard on TripleOscillator, and a step pattern of 16 steps",
        tracks: &["Bateria (AvlDrums)", "Baixo (TripleOscillator)", "Teclado (TripleOscillator)"],
    },
    Template {
        name: "Podcast",
        about: "Two voices with compressor and EQ on their channels, and a music track",
        tracks: &["Voz 1", "Voz 2", "Trilha"],
    },
    Template {
        name: "Mixing session",
        about: "Twenty-four empty audio tracks to import stems into, with groups for drums, instruments and voices",
        tracks: &["Bateria", "Instrumentos", "Vozes"],
    },
];

pub const RATES: [u32; 5] = [44_100, 48_000, 88_200, 96_000, 192_000];

// ---------------------------------------------------------------------------------------------------------------
// The preferences (a tree of their own, saved in the configuration file)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioSystem {
    /// F1: ALSA directly, `hw` exclusive, capture and playback linked
    Alsa,
    /// F2 (and PipeWire through the JACK API)
    Jack,
    /// No device: plays nothing, records nothing (tests, a machine without a card)
    Dummy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiMode {
    /// The notes go to the selected track, whichever it is
    SelectedTrack,
    /// Only controller changes, to map to parameters
    Controllers,
}

#[derive(Clone, Debug)]
pub struct MidiPort {
    /// ALSA sequencer client:port
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub mode: MidiMode,
    /// 0 = every channel
    pub channel: u8,
    /// The port's own input latency, subtracted from what it records (P-15), in ms
    pub latency_ms: f64,
}

/// When a setting takes effect (design 08: every option says it before you change it)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Apply {
    Now,
    /// at once, by reopening the device on its own (an operation on the device)
    ReopensDevice,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Follow {
    Off,
    /// a page at a time, when the cursor leaves the view
    Page,
    /// the view scrolls, the cursor kept in the middle
    Continuous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detached {
    /// closing a detached window docks it again and shows it
    DockAndShow,
    DockAndHide,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiBackend {
    /// ALSA sequencer (F1)
    Seq,
    /// ALSA raw MIDI, the second ALSA backend (P-16)
    Raw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetronomeWhen {
    Off,
    Recording,
    PlayingAndRecording,
}

/// What a take becomes when it closes (P-12): it is always recorded as RF64 f32, which survives a crash
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeFormat {
    KeepFloat,
    Flac,
    Int24,
}

#[derive(Clone, Debug)]
pub struct Prefs {
    // general
    pub reopen_last: bool,
    pub ask_delete_track: bool,
    pub ask_delete_channel: bool,
    /// Longer than this (s), an audio file dropped on the project becomes an audio track; shorter, a sample
    pub long_audio_seconds: u32,
    pub previews_to_end: bool,
    /// New projects start with the desk's analog modelling on (P-17); off: the clean digital desk
    pub analog_desk: bool,
    /// The rate new projects start with (each project keeps its own, spec item 8)
    pub new_project_rate: u32,
    /// The configuration beside the program (P-35)
    pub portable: bool,
    // audio
    pub audio_system: AudioSystem,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// U7: input and output on different cards only with this on (an adaptive resampler on the input)
    pub drift_correction: bool,
    /// None: the project's rate (spec item 8); a rate the device takes otherwise, resampled at the edge
    pub device_rate: Option<u32>,
    pub block: u32,
    /// Added to the calibrated latency, in frames, -48000..=48000 (F-REC-082)
    pub latency_offset: i32,
    // MIDI (the connections are the machine's, never the project's: P-36)
    pub midi_backend: MidiBackend,
    pub quantize_midi: bool,
    // recording
    pub metronome_when: MetronomeWhen,
    pub metronome_db: f64,
    pub metronome_accent: bool,
    pub click_in_preroll: bool,
    /// off: the simple recorder, a new pass replaces what it covers (P-11)
    pub takes_in_lanes: bool,
    pub take_format: TakeFormat,
    // plugins
    pub plugin_windows_on_top: bool,
    pub one_instrument_window: bool,
    /// Plugins that were running when the program fell (F-CFG-014): not loaded until "load again"
    pub not_loaded: Vec<String>,
    // folders
    pub projects_dir: String,
    pub samples_dir: String,
    pub presets_dir: String,
    pub soundfonts_dir: String,
    pub default_soundfont: String,
    /// Third-party plugin folders (P-01, P-03), looked through by the scan
    pub plugin_dirs: Vec<String>,
    /// C9: the only places outside a project its external resources may come from
    pub allowed: Vec<String>,
    // appearance
    /// a theme file (`themes/*.txt`), by name
    pub theme: String,
    pub language: &'static str,
    pub tooltips: bool,
    pub compact_headers: bool,
    pub browser_right: bool,
    pub follow: Follow,
    pub smooth_scroll: bool,
    pub note_names: bool,
    pub fader_ticks: bool,
    pub detached: Detached,
    // shortcuts: action id → keys ("" = none); absent = the registry's default
    pub keys: BTreeMap<&'static str, String>,
    // autosave and backup
    /// 0 = off
    pub autosave_minutes: u32,
    pub backups_kept: u32,
    pub backup_on_save: bool,
    // advanced
    /// NaN, infinities and denormals cleaned from a plugin's output before it reaches the mix (F-CFG-018, U20)
    pub sanitize: bool,
    /// The settings changed that wait for a restart (design 08's band), by English label
    pub pending_restart: Vec<&'static str>,
    /// Most recent first: the palette shows them before typing
    pub recent_actions: Vec<&'static str>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            reopen_last: false,
            ask_delete_track: true,
            ask_delete_channel: true,
            long_audio_seconds: 30,
            previews_to_end: false,
            analog_desk: true,
            new_project_rate: 48_000,
            portable: false,
            audio_system: AudioSystem::Alsa,
            input_device: Some("hw:1".into()),
            output_device: Some("hw:1".into()),
            drift_correction: false,
            device_rate: None,
            block: 256,
            latency_offset: 0,
            midi_backend: MidiBackend::Seq,
            quantize_midi: false,
            metronome_when: MetronomeWhen::PlayingAndRecording,
            metronome_db: -6.0,
            metronome_accent: true,
            click_in_preroll: true,
            takes_in_lanes: true,
            take_format: TakeFormat::KeepFloat,
            plugin_windows_on_top: false,
            one_instrument_window: false,
            not_loaded: vec!["Vital (VST3)".into()],
            projects_dir: "~/Música/LinuxMusic".into(),
            samples_dir: "~/Música/LinuxMusic/Amostras".into(),
            presets_dir: "~/.local/share/linuxmusic/presets".into(),
            soundfonts_dir: "~/Música/LinuxMusic/SoundFonts".into(),
            default_soundfont: "/usr/share/sounds/sf2/FluidR3_GM.sf2".into(),
            plugin_dirs: vec!["~/.vst3".into(), "~/.lv2".into(), "/usr/lib/ladspa".into()],
            allowed: vec!["/usr/share/sounds/sf2".into(), "~/SoundFonts".into()],
            theme: "dark".into(),
            language: "pt-BR",
            tooltips: true,
            compact_headers: false,
            browser_right: false,
            follow: Follow::Page,
            smooth_scroll: true,
            note_names: true,
            fader_ticks: true,
            detached: Detached::DockAndShow,
            keys: BTreeMap::new(),
            autosave_minutes: 2,
            backups_kept: 10,
            backup_on_save: true,
            sanitize: true,
            pending_restart: Vec::new(),
            recent_actions: vec!["file.export", "view.mixing", "file.save"],
        }
    }
}

impl Prefs {
    /// Notes a restart-only change (once per setting)
    pub fn needs_restart(&mut self, label: &'static str) {
        if !self.pending_restart.contains(&label) {
            self.pending_restart.push(label);
        }
    }
    pub fn used_action(&mut self, id: &'static str) {
        self.recent_actions.retain(|a| *a != id);
        self.recent_actions.insert(0, id);
        self.recent_actions.truncate(8);
    }
    /// C9: is this path inside an allowed folder?
    pub fn allows(&self, path: &str) -> bool {
        self.allowed.iter().any(|f| path == f || path.starts_with(&format!("{}/", f.trim_end_matches('/'))))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Devices

#[derive(Clone, Debug, PartialEq)]
pub struct AudioDevice {
    /// ALSA: `hw:N`, or a PipeWire node name
    pub id: String,
    pub name: String,
    pub inputs: u32,
    pub outputs: u32,
    pub rates: Vec<u32>,
    /// Bluetooth: using its microphone drops the output to the call profile (U7)
    pub bluetooth: bool,
    /// Held by another program; asked for by ReserveDevice1 (U8)
    pub busy_by: Option<&'static str>,
    /// What the device reports as its own latency, in frames, on top of the buffer
    pub reported_extra: u32,
}

/// What a calibration keeps (H19): the residual, by device pair, rate and edge resampler, so it stays valid when
/// the block size changes
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    pub devices: String,
    pub rate: u32,
    pub edge_resampler: bool,
    /// measured minus reported, in frames
    pub residual: i32,
    /// the spread of the five measurements, in frames
    pub spread: u32,
    pub at: Instant,
}

/// Real-time permission on Linux (H1)
#[derive(Clone, Debug)]
pub struct Realtime {
    pub rtkit: bool,
    /// RLIMIT_RTPRIO (0: none)
    pub rtprio: u32,
    /// RLIMIT_MEMLOCK in KB (None: unlimited)
    pub memlock_kb: Option<u64>,
    pub audio_group: bool,
    pub user: String,
}

impl Realtime {
    pub fn priority_ok(&self) -> bool {
        self.rtkit || self.rtprio >= 70
    }
    pub fn memlock_ok(&self) -> bool {
        self.memlock_kb.is_none_or(|k| k >= 512 * 1024)
    }
}

/// The device as it opened
#[derive(Clone, Debug, PartialEq)]
pub struct Opened {
    pub input: String,
    pub output: String,
    pub rate: u32,
    /// C7: the period the device agreed to, not the one asked for
    pub block: u32,
    pub latency_in: u32,
    pub latency_out: u32,
    pub edge_resampler: bool,
}

#[derive(Clone, Debug)]
pub struct Devices {
    pub audio: Vec<AudioDevice>,
    pub midi: Vec<MidiPort>,
    pub realtime: Realtime,
    pub calibrations: Vec<Calibration>,
    pub opened: Option<Opened>,
    pub scanned: Instant,
}

fn audio_examples() -> Vec<AudioDevice> {
    let d = |id: &str, name: &str, i, o, rates: &[u32], bluetooth, busy_by, extra| AudioDevice {
        id: id.into(),
        name: name.into(),
        inputs: i,
        outputs: o,
        rates: rates.to_vec(),
        bluetooth,
        busy_by,
        reported_extra: extra,
    };
    vec![
        d("hw:0", "HDA Intel PCH (ALC892 Analog)", 2, 2, &[44_100, 48_000, 96_000, 192_000], false, None, 32),
        d("hw:1", "Scarlett 2i2 USB", 2, 2, &[44_100, 48_000, 88_200, 96_000, 192_000], false, None, 58),
        d("hw:2", "UMC404HD 192k", 4, 4, &[44_100, 48_000, 88_200, 96_000, 192_000], false, None, 64),
        d("hw:3", "HDA NVidia (HDMI 0)", 0, 8, &[44_100, 48_000], false, Some("PipeWire"), 24),
        d("bluez_card.WH-1000XM4", "WH-1000XM4 (Bluetooth, PipeWire)", 1, 2, &[16_000, 48_000], true, None, 7200),
    ]
}

fn midi_examples() -> Vec<MidiPort> {
    let p = |id: &str, name: &str, enabled, mode| MidiPort { id: id.into(), name: name.into(), enabled, mode, channel: 0, latency_ms: 0.0 };
    vec![
        p("14:0", "Midi Through Port-0", false, MidiMode::SelectedTrack),
        p("20:0", "Arturia KeyStep 32", true, MidiMode::SelectedTrack),
        p("24:0", "nanoKONTROL2", true, MidiMode::Controllers),
        p("28:0", "Scarlett 2i2 USB MIDI 1", false, MidiMode::SelectedTrack),
    ]
}

impl Devices {
    pub fn find(&self, id: &str) -> Option<&AudioDevice> {
        self.audio.iter().find(|d| d.id == id)
    }
    /// The calibration of this combination, if measured
    pub fn calibration(&self, devices: &str, rate: u32, edge: bool) -> Option<&Calibration> {
        self.calibrations.iter().find(|c| c.devices == devices && c.rate == rate && c.edge_resampler == edge)
    }
}

/// The name of an input/output pair, as the calibrations are kept
pub fn pair_name(input: &str, output: &str) -> String {
    if input == output { input.to_string() } else { format!("{input} → {output}") }
}

// ---------------------------------------------------------------------------------------------------------------
// Export (I-13)

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Wav,
    Flac,
    Ogg,
    Mp3,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Wav, Format::Flac, Format::Ogg, Format::Mp3];
    pub fn ext(self) -> &'static str {
        match self {
            Format::Wav => "wav",
            Format::Flac => "flac",
            Format::Ogg => "ogg",
            Format::Mp3 => "mp3",
        }
    }
    /// Lossy formats have no bit depth
    pub fn lossless(self) -> bool {
        matches!(self, Format::Wav | Format::Flac)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeMode {
    /// From the start of the song, dropping what comes before: identical to the range inside the whole song
    Exact,
    /// Starts `warmup` seconds before and drops them: almost identical, and said so
    Fast { warmup: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scope {
    /// Always exact (V18)
    Song,
    /// Bars, start inclusive, end exclusive (1-based, as the ruler)
    Range { start: u32, end: u32, mode: RangeMode },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    I16,
    I24,
    F32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Normalize {
    None,
    Peak(f64),
    Lufs(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mp3Mode {
    Stereo,
    Joint,
    Mono,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stems {
    None,
    Tracks(Vec<NodeId>),
    Channels(Vec<NodeId>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportRequest {
    pub scope: Scope,
    pub formats: Vec<Format>,
    pub rate: u32,
    pub depth: Depth,
    pub dither: bool,
    pub mp3_kbps: u32,
    pub mp3_mode: Mp3Mode,
    /// 0..=10
    pub ogg_quality: u32,
    /// 0..=8
    pub flac_level: u32,
    pub normalize: Normalize,
    /// seconds, 0..=30
    pub fade_in: f64,
    pub fade_out: f64,
    /// Keep the tail after the end (reverbs, releases); off: cut as a loop
    pub tail: bool,
    /// As a loop: the scope rendered this many times in a row (1..=64)
    pub repeats: u32,
    /// Export one track: the others muted (F-PRJ-023)
    pub only_track: Option<NodeId>,
    pub stems: Stems,
    pub folder: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportReport {
    pub files: Vec<String>,
    pub seconds: f64,
    /// False for a fast range: said in the report
    pub identical: bool,
    pub folder: String,
}

/// The song as the export sees it, read from the document (the fake's bars are 2 s: 4/4 at 120 BPM)
#[derive(Clone, Debug)]
pub struct Song {
    pub bars: u32,
    pub bpm: f64,
    pub loop_bars: (u32, u32),
    /// The fake renderer's speed, in times real time
    pub render_speed: f64,
}

/// The song's length (the end of the last item, in whole bars), its loop range and the renderer's speed
pub fn song(s: &Session) -> Song {
    let bar = crate::arrange::BAR;
    let end = s.tracks().iter().flat_map(|t| t.items.iter()).map(|i| i.start + i.length).max().unwrap_or(0);
    let lr = &s.doc().song.loop_range;
    Song {
        bars: ((end + bar - 1) / bar).max(1) as u32,
        bpm: 60.0 * 4.0 * crate::SAMPLE_RATE as f64 / bar as f64,
        loop_bars: ((lr.start / bar) as u32 + 1, (lr.end / bar) as u32 + 1),
        render_speed: s.services.render_speed,
    }
}

impl Song {
    pub fn bar_seconds(&self) -> f64 {
        4.0 * 60.0 / self.bpm
    }
    pub fn seconds(&self) -> f64 {
        self.bars as f64 * self.bar_seconds()
    }
}

/// A plugin used in the project (H18: third-party ones do not promise the same output twice)
#[derive(Clone, Debug)]
pub struct PluginUse {
    pub name: String,
    pub track: String,
    pub third_party: bool,
}

/// What an export renders and costs: (seconds of audio rendered, estimated seconds of work)
pub fn export_cost(song: &Song, req: &ExportRequest) -> (f64, f64) {
    let bar = song.bar_seconds();
    let tail = if req.tail { 2.0 } else { 0.0 };
    let once = match req.scope {
        Scope::Song => song.seconds(),
        Scope::Range { start, end, .. } => (end - start) as f64 * bar,
    };
    let rendered = (req.repeats.max(1) - 1) as f64 * once + match req.scope {
        Scope::Song => song.seconds() + tail,
        Scope::Range { end, mode: RangeMode::Exact, .. } => (end - 1) as f64 * bar + tail,
        Scope::Range { start, end, mode: RangeMode::Fast { warmup } } => {
            (end - start) as f64 * bar + warmup.min((start - 1) as f64 * bar) + tail
        }
    };
    let stems = match &req.stems {
        Stems::None => 0,
        Stems::Tracks(v) | Stems::Channels(v) => v.len(),
    };
    let files = req.formats.len().max(1) * (1 + stems);
    (rendered, rendered / song.render_speed * (1.0 + 0.04 * files as f64))
}

/// The files an export writes, by name
pub fn export_files(req: &ExportRequest, names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for f in &req.formats {
        out.push(format!("{}.{}", req.name, f.ext()));
        for n in names {
            out.push(format!("{} - {}.{}", req.name, n, f.ext()));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// The services

/// An operation of this module waiting for its result (applied in `Services::pump`)
pub enum Pending {
    Open(OperationHandle<ProjectInfo>, Scenario, String),
    Create(OperationHandle<ProjectInfo>),
    Save(OperationHandle<()>, Option<(String, String)>),
    Zip(OperationHandle<String>),
    Export(OperationHandle<ExportReport>, ExportRequest, u64),
    Calibrate(OperationHandle<Calibration>),
    Reopen(OperationHandle<Opened>),
    Scan(OperationHandle<(Vec<AudioDevice>, Vec<MidiPort>)>),
    Recreate(OperationHandle<()>, String),
}

/// What "try again" repeats
#[derive(Clone, Debug)]
enum Retry {
    Export(ExportRequest),
    Open(String),
    Calibrate,
    Reopen,
}

pub struct Services {
    pub project: ProjectInfo,
    pub recent: Vec<RecentProject>,
    pub prefs: Prefs,
    pub devices: Devices,
    pub notices: Queue,
    /// The fake renderer's speed, in times real time (the export's estimate)
    pub render_speed: f64,
    pub plugins: Vec<PluginUse>,
    /// The last export's report, and the one running (the export screen shows them)
    pub last_export: Option<ExportReport>,
    /// The export running: (its id, as in `Target::Export`, and its operation's id)
    pub exporting: Option<(u64, u64)>,
    export_seq: u64,
    /// A screen asked to open on one of its parts ("save_as", "keys"): taken by the screen when it opens
    pub screen_arg: Option<&'static str>,
    /// Operation durations (the prototype's injected delays, I-2.4)
    pub delays: Delays,
    pending: Vec<Pending>,
    retries: BTreeMap<u64, Retry>,
    /// Bumped on every change a screen shows
    pub serial: u64,
}

#[derive(Clone, Debug)]
pub struct Delays {
    pub open: Duration,
    pub save: Duration,
    pub zip: Duration,
    pub calibrate: Duration,
    pub reopen: Duration,
    pub scan: Duration,
}

impl Default for Delays {
    fn default() -> Self {
        Delays {
            open: Duration::from_millis(1200),
            save: Duration::from_millis(120),
            zip: Duration::from_millis(2600),
            calibrate: Duration::from_millis(3000),
            reopen: Duration::from_millis(450),
            scan: Duration::from_millis(700),
        }
    }
}

impl Default for Services {
    fn default() -> Self {
        let now = Instant::now();
        Services {
            project: demo_project(),
            recent: recent_examples(),
            prefs: Prefs::default(),
            devices: Devices {
                audio: audio_examples(),
                midi: midi_examples(),
                realtime: Realtime { rtkit: false, rtprio: 0, memlock_kb: Some(8 * 1024), audio_group: false, user: "eli".into() },
                calibrations: vec![
                    Calibration { devices: "hw:1".into(), rate: 48_000, edge_resampler: false, residual: 23, spread: 1, at: now - Duration::from_secs(3 * 86_400) },
                    Calibration { devices: "hw:1".into(), rate: 96_000, edge_resampler: false, residual: 41, spread: 2, at: now - Duration::from_secs(3 * 86_400) },
                    Calibration { devices: "hw:0".into(), rate: 48_000, edge_resampler: false, residual: 12, spread: 0, at: now - Duration::from_secs(40 * 86_400) },
                ],
                opened: Some(Opened { input: "hw:1".into(), output: "hw:1".into(), rate: 48_000, block: 256, latency_in: 314, latency_out: 314, edge_resampler: false }),
                scanned: now,
            },
            notices: Queue::with_examples(),
            render_speed: 38.0,
            plugins: vec![
                PluginUse { name: "TripleOscillator".into(), track: "Teclado 1".into(), third_party: false },
                PluginUse { name: "AvlDrums".into(), track: "Bateria 1".into(), third_party: false },
                PluginUse { name: "Vital (VST3)".into(), track: "Teclado 2".into(), third_party: true },
                PluginUse { name: "Dragonfly Hall Reverb (LV2)".into(), track: "Retorno A".into(), third_party: true },
                PluginUse { name: "Compressor".into(), track: "Voz principal 1".into(), third_party: false },
            ],
            last_export: None,
            exporting: None,
            export_seq: 0,
            screen_arg: None,
            delays: Delays::default(),
            pending: Vec::new(),
            retries: BTreeMap::new(),
            serial: 0,
        }
    }
}

impl Services {
    /// Applies the results of the finished operations; true when something shown changed. From the session's pump.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.pending.len() {
            if let Some(done) = self.poll(i) {
                self.pending.remove(i);
                self.apply(done);
                changed = true;
            } else {
                i += 1;
            }
        }
        if changed {
            self.serial += 1;
        }
        changed
    }

    /// Is one of this module's operations still running for this target?
    pub fn busy(&self, f: impl Fn(&Pending) -> bool) -> bool {
        self.pending.iter().any(f)
    }

    fn poll(&mut self, i: usize) -> Option<Done> {
        Some(match &self.pending[i] {
            Pending::Open(h, s, name) => Done::Open(h.poll()?, *s, name.clone()),
            Pending::Create(h) => Done::Create(h.poll()?),
            Pending::Save(h, to) => Done::Save(h.poll()?, to.clone()),
            Pending::Zip(h) => Done::Zip(h.poll()?),
            Pending::Export(h, req, e) => Done::Export(h.poll()?, req.clone(), *e),
            Pending::Calibrate(h) => Done::Calibrate(h.poll()?),
            Pending::Reopen(h) => Done::Reopen(h.poll()?),
            Pending::Scan(h) => Done::Scan(h.poll()?),
            Pending::Recreate(h, p) => Done::Recreate(h.poll()?, p.clone()),
        })
    }

    fn apply(&mut self, done: Done) {
        let cancelled = |e: &String| e == "cancelled";
        match done {
            Done::Open(Ok(p), scenario, _) => {
                let name = p.name.clone();
                self.project = p;
                match scenario {
                    Scenario::LockedElsewhere => {
                        self.notices.push(
                            "{0} is open in another program; it opened read-only",
                            vec![name],
                            Severity::Warning,
                            Some(Target::Project),
                            vec![NoticeAction::new("file.save_as", "")],
                        );
                    }
                    Scenario::Newer => {
                        self.notices.push(
                            "{0} was saved by a newer version; it opened read-only and is never saved over",
                            vec![name],
                            Severity::Warning,
                            Some(Target::Project),
                            vec![NoticeAction::new("file.save_as", "")],
                        );
                    }
                    Scenario::StaleLock => {
                        self.notices.push(
                            "A lock left by a program that closed was removed from {0}",
                            vec![name],
                            Severity::Info,
                            Some(Target::Project),
                            vec![],
                        );
                    }
                    Scenario::CrashedRecording => {
                        let n = self.project.recovery.len().to_string();
                        self.notices.push(
                            "The program closed while recording: {0} takes can be recovered",
                            vec![n],
                            Severity::Warning,
                            Some(Target::Project),
                            vec![NoticeAction::new("file.open", "")],
                        );
                    }
                    Scenario::CrashedPlugin => {
                        let (p, t) = self.project.safe_mode.first().map(|s| (s.plugin.clone(), s.track.clone())).unwrap_or_default();
                        self.notices.push(
                            "Safe mode: {0} on {1} is off, because the program stopped while it ran",
                            vec![p, t],
                            Severity::Warning,
                            Some(Target::Project),
                            vec![NoticeAction::new("file.open", "")],
                        );
                    }
                    Scenario::Older => {
                        self.notices.push(
                            "{0} was converted from an older version; the original is kept beside it",
                            vec![name],
                            Severity::Info,
                            Some(Target::Project),
                            vec![],
                        );
                    }
                    Scenario::Normal | Scenario::Missing => {}
                }
            }
            Done::Open(Err(e), _, _) if cancelled(&e) => {}
            Done::Open(Err(e), _, name) => {
                let id = self.notices.push(fail_key(&e), vec![name.clone()], Severity::Error, Some(Target::Project), vec![]);
                self.add_retry(id, Retry::Open(name));
            }
            Done::Create(Ok(p)) => self.project = p,
            Done::Create(Err(_)) => {}
            Done::Save(Ok(()), to) => {
                if let Some((name, folder)) = to {
                    self.project.name = name;
                    self.project.folder = folder;
                    self.project.access = Access::ReadWrite;
                }
                self.project.modified = false;
                self.project.saved = Some(Instant::now());
                self.project.journal = 0;
                self.project.migrated_from = None;
            }
            Done::Save(Err(e), _) => {
                self.notices.push(fail_key(&e), vec![], Severity::Error, Some(Target::Project), vec![]);
            }
            Done::Zip(Ok(path)) => {
                self.notices.push("Exported the project as {0}", vec![path.clone()], Severity::Info, None, vec![NoticeAction::new("export.reveal", path)]);
            }
            Done::Zip(Err(_)) => {}
            Done::Export(r, req, op) => {
                if self.exporting.is_some_and(|(e, _)| e == op) {
                    self.exporting = None;
                }
                match r {
                    Ok(report) => {
                        self.notices.push(
                            "Exported {0} files to {1}",
                            vec![report.files.len().to_string(), report.folder.clone()],
                            Severity::Info,
                            Some(Target::Export(op)),
                            vec![NoticeAction::new("export.reveal", report.folder.clone())],
                        );
                        self.last_export = Some(report);
                    }
                    Err(e) if cancelled(&e) => {}
                    Err(e) => {
                        let id = self.notices.push(
                            fail_key(&e),
                            vec![],
                            Severity::Error,
                            Some(Target::Export(op)),
                            vec![],
                        );
                        self.add_retry(id, Retry::Export(req));
                    }
                }
            }
            Done::Calibrate(Ok(c)) => {
                self.notices.push(
                    "Latency calibrated on {0} at {1} Hz: {2} frames beyond what the device reports",
                    vec![c.devices.clone(), c.rate.to_string(), c.residual.to_string()],
                    Severity::Info,
                    Some(Target::Device),
                    vec![],
                );
                self.devices.calibrations.retain(|k| !(k.devices == c.devices && k.rate == c.rate && k.edge_resampler == c.edge_resampler));
                self.devices.calibrations.push(c);
            }
            Done::Calibrate(Err(e)) if cancelled(&e) => {}
            Done::Calibrate(Err(e)) => {
                let id = self.notices.push(fail_key(&e), vec![], Severity::Error, Some(Target::Device), vec![]);
                self.add_retry(id, Retry::Calibrate);
            }
            Done::Reopen(Ok(o)) => {
                if let Some(d) = self.devices.find(&o.input).filter(|d| d.bluetooth) {
                    let name = d.name.clone();
                    self.notices.push(
                        "{0}: using its microphone drops the output to the call profile, over 150 ms of latency",
                        vec![name],
                        Severity::Warning,
                        Some(Target::Device),
                        vec![NoticeAction::new("settings.realtime", "")],
                    );
                }
                self.devices.opened = Some(o);
            }
            Done::Reopen(Err(e)) => {
                self.devices.opened = None;
                let id = self.notices.push(
                    fail_key(&e),
                    vec![],
                    Severity::Error,
                    Some(Target::Device),
                    vec![NoticeAction::new("settings.realtime", "")],
                );
                self.add_retry(id, Retry::Reopen);
            }
            Done::Scan(Ok((audio, midi))) => {
                self.devices.audio = audio;
                // keep the user's choices for the ports that are still there
                let old = std::mem::take(&mut self.devices.midi);
                self.devices.midi = midi
                    .into_iter()
                    .map(|p| old.iter().find(|o| o.id == p.id).cloned().unwrap_or(p))
                    .collect();
                self.devices.scanned = Instant::now();
            }
            Done::Scan(Err(_)) => {}
            Done::Recreate(Ok(()), plugin) => {
                self.notices.push("{0} was recreated from the project", vec![plugin], Severity::Info, None, vec![]);
            }
            Done::Recreate(Err(_), _) => {}
        }
    }

    fn add_retry(&mut self, notice: u64, r: Retry) {
        self.retries.insert(notice, r);
        let arg = notice.to_string();
        // the notice gets its "try again" with its own id as the argument
        if let Some(n) = self.notices.actions_mut(notice) {
            n.push(NoticeAction::new("op.retry", arg));
        }
    }
}

enum Done {
    Open(Result<ProjectInfo, String>, Scenario, String),
    Create(Result<ProjectInfo, String>),
    Save(Result<(), String>, Option<(String, String)>),
    Zip(Result<String, String>),
    Export(Result<ExportReport, String>, ExportRequest, u64),
    Calibrate(Result<Calibration, String>),
    Reopen(Result<Opened, String>),
    Scan(Result<(Vec<AudioDevice>, Vec<MidiPort>), String>),
    Recreate(Result<(), String>, String),
}

// Why an operation failed: whole sentences as English keys (the worker's `Err` carries one of them; the notice
// translates it). "cancelled" is not a failure.
pub const F_MISSING: &str = "Could not open {0}: its folder is not there";
pub const F_DISK: &str = "The export failed: there is not enough space on the disk";
pub const F_JACK: &str = "The audio device did not open: JACK comes in phase 2";
pub const F_CHOOSE: &str = "The audio device did not open: choose an input device and an output device";
pub const F_BUSY: &str = "The audio device did not open: PipeWire did not release it";
pub const F_CARDS: &str =
    "The audio device did not open: the input and the output are on different cards; turn on correcting the drift between cards, or choose one card";
const FAILS: &[&str] = &[F_MISSING, F_DISK, F_JACK, F_CHOOSE, F_BUSY, F_CARDS];

fn fail_key(e: &str) -> &'static str {
    FAILS.iter().copied().find(|k| *k == e).unwrap_or("The operation failed")
}

/// A worker's body: `steps` sleeps through `total`, with progress and cancel
fn wait(total: Duration, cancel: &crate::Cancel, progress: &crate::Progress) -> Result<(), String> {
    let steps = 40;
    for i in 0..steps {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        std::thread::sleep(total / steps);
        progress.set((i + 1) as f32 / steps as f32);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// The operations (what the actions and the screens start)

/// Opens a recent project (by its index in the list)
pub fn open_recent(s: &mut Session, index: usize) {
    let Some(r) = s.services.recent.get(index).cloned() else { return };
    let delay = s.services.delays.open;
    let scenario = r.scenario;
    let name = r.name.clone();
    let h = s.start("Opening the project", Target::Project, true, move |cancel, progress| {
        wait(delay, cancel, progress)?;
        let mut p = demo_project();
        p.name = r.name.clone();
        p.folder = r.folder.clone();
        p.rate = r.rate;
        p.modified = false;
        p.saved = Some(Instant::now() - r.modified_ago);
        p.journal = 0;
        p.audio_files = r.tracks * 2;
        p.audio_mb = r.minutes * r.tracks as f64 * 10.0;
        match scenario {
            Scenario::Missing => return Err(F_MISSING.into()),
            Scenario::LockedElsewhere => {
                p.access = Access::ReadOnly(ReadOnly::Locked {
                    machine: "estudio-b".into(),
                    pid: 48_213,
                    since: "09:41".into(),
                    this_machine: false,
                })
            }
            Scenario::Newer => p.access = Access::ReadOnly(ReadOnly::Newer { schema: SCHEMA + 2, version: "1.3".into() }),
            Scenario::CrashedRecording => {
                p.journal = 112;
                p.recovery = vec![
                    InterruptedTake { track: "Voz principal 1".into(), input: "Entrada 1 (mono)".into(), start_bar: 33, seconds: 74.5, loop_cuts: 0, decision: None },
                    InterruptedTake { track: "Guitarra 1".into(), input: "Entrada 2 (mono)".into(), start_bar: 33, seconds: 74.5, loop_cuts: 0, decision: None },
                    InterruptedTake { track: "Backing 1".into(), input: "Entradas 3–4 (estéreo)".into(), start_bar: 17, seconds: 212.0, loop_cuts: 3, decision: None },
                ];
            }
            Scenario::CrashedPlugin => {
                p.safe_mode = vec![SafeModePlugin { plugin: "Vital (VST3)".into(), track: "Teclado 2".into(), why: "stopped answering", enabled: false }];
            }
            Scenario::Older => p.migrated_from = Some(SCHEMA - 1),
            Scenario::Normal | Scenario::StaleLock => {}
        }
        Ok(p)
    });
    s.services.pending.push(Pending::Open(h, scenario, name));
    // the opened project goes to the top of the list
    let item = s.services.recent.remove(index);
    s.services.recent.insert(0, item);
}

/// A new project from a template (the rate is fixed from here on)
pub fn create(s: &mut Session, name: String, folder: String, rate: u32, template: &'static str) {
    let delay = s.services.delays.reopen;
    let h = s.start("Creating the project", Target::Project, false, move |cancel, progress| {
        wait(delay, cancel, progress)?;
        Ok(ProjectInfo {
            name,
            folder,
            rate,
            template,
            saved: Some(Instant::now()),
            modified: false,
            access: Access::ReadWrite,
            recovery: Vec::new(),
            safe_mode: Vec::new(),
            journal: 0,
            audio_files: 0,
            audio_mb: 0.0,
            migrated_from: None,
        })
    });
    s.services.pending.push(Pending::Create(h));
}

pub fn save(s: &mut Session) {
    if s.services.project.read_only() {
        s.services.notices.push(
            "This project is read-only: use Save as",
            vec![],
            Severity::Warning,
            Some(Target::Project),
            vec![NoticeAction::new("file.save_as", "")],
        );
        return;
    }
    start_save(s, None);
}

pub fn save_as(s: &mut Session, name: String, folder: String) {
    start_save(s, Some((name, folder)));
}

fn start_save(s: &mut Session, to: Option<(String, String)>) {
    let delay = s.services.delays.save;
    let h = s.start("Saving", Target::Project, false, move |cancel, progress| wait(delay, cancel, progress));
    s.services.pending.push(Pending::Save(h, to));
}

/// "Export as a single file": the folder in a `.zip`
pub fn export_zip(s: &mut Session) {
    let delay = s.services.delays.zip;
    let path = format!("{}.zip", s.services.project.folder);
    let h = s.start("Packing the project", Target::Project, true, move |cancel, progress| {
        wait(delay, cancel, progress)?;
        Ok(path)
    });
    s.services.pending.push(Pending::Zip(h));
}

/// Starts an export; its id is the export's target (`Target::Export(id)`)
pub fn export(s: &mut Session, req: ExportRequest) -> u64 {
    let (_, cost) = export_cost(&song(s), &req);
    let names: Vec<String> = match &req.stems {
        Stems::None => Vec::new(),
        Stems::Tracks(ids) => s.tracks().iter().filter(|t| ids.contains(&t.id)).map(|t| t.name.clone()).collect(),
        Stems::Channels(ids) => s.channels().iter().filter(|c| ids.contains(&c.id)).map(|c| c.name.clone()).collect(),
    };
    let files = export_files(&req, &names);
    let identical = !matches!(req.scope, Scope::Range { mode: RangeMode::Fast { .. }, .. });
    let folder = req.folder.clone();
    // the prototype's export takes a tenth of the estimate (no one waits a minute to see a screen)
    let total = Duration::from_secs_f64((cost / 10.0).clamp(0.5, 8.0));
    // a fake failure, to see "try again": a folder on a full disk
    let full = folder.contains("cheio");
    s.services.export_seq += 1;
    let id = s.services.export_seq;
    let h = s.start("Exporting", Target::Export(id), true, move |cancel, progress| {
        if full {
            wait(total / 3, cancel, progress)?;
            return Err(F_DISK.into());
        }
        wait(total, cancel, progress)?;
        Ok(ExportReport { files, seconds: cost, identical, folder })
    });
    s.services.exporting = Some((id, h.id));
    s.services.pending.push(Pending::Export(h, req, id));
    id
}

/// The latency calibration (I-10.4, H19): five measures of an impulse, the median kept as the residual of this
/// device pair, rate and edge resampler
pub fn calibrate(s: &mut Session) {
    let Some(o) = s.services.devices.opened.clone() else {
        s.services.notices.push("Open an audio device first", vec![], Severity::Warning, Some(Target::Device), vec![]);
        return;
    };
    if s.services.busy(|p| matches!(p, Pending::Calibrate(_))) {
        return;
    }
    let delay = s.services.delays.calibrate;
    let devices = pair_name(&o.input, &o.output);
    let seed = devices.len() as i32 + o.rate as i32 / 1000;
    let h = s.start("Measuring the latency", Target::Device, true, move |cancel, progress| {
        wait(delay, cancel, progress)?;
        Ok(Calibration { devices, rate: o.rate, edge_resampler: o.edge_resampler, residual: 17 + seed % 13, spread: 1, at: Instant::now() })
    });
    s.services.pending.push(Pending::Calibrate(h));
}

/// Reopens the device with the preferences (I-10.3): refused while recording (U10)
pub fn reopen_device(s: &mut Session) {
    if s.position().recording {
        s.services.notices.push(
            "The audio device does not change while recording; stop first",
            vec![],
            Severity::Warning,
            Some(Target::Device),
            vec![],
        );
        return;
    }
    let p = &s.services.prefs;
    let devs = &s.services.devices;
    let rate = p.device_rate.unwrap_or(s.services.project.rate);
    let block = p.block;
    let system = p.audio_system;
    let input = p.input_device.clone();
    let output = p.output_device.clone();
    let drift = p.drift_correction;
    let in_dev = input.as_deref().and_then(|i| devs.find(i)).cloned();
    let out_dev = output.as_deref().and_then(|o| devs.find(o)).cloned();
    let project_rate = s.services.project.rate;
    let delay = s.services.delays.reopen;
    let h = s.start("Opening the audio device", Target::Device, false, move |cancel, progress| {
        wait(delay, cancel, progress)?;
        if system == AudioSystem::Jack {
            return Err(F_JACK.into());
        }
        let (Some(i), Some(o)) = (in_dev, out_dev) else {
            return if system == AudioSystem::Dummy {
                Ok(Opened { input: "dummy".into(), output: "dummy".into(), rate, block, latency_in: 0, latency_out: block, edge_resampler: false })
            } else {
                Err(F_CHOOSE.into())
            };
        };
        if let Some(by) = o.busy_by.or(i.busy_by) {
            let _ = by;
            return Err(F_BUSY.into());
        }
        if i.id != o.id && !drift {
            return Err(F_CARDS.into());
        }
        let edge = !o.rates.contains(&rate) || rate != project_rate;
        let open_rate = if o.rates.contains(&rate) { rate } else { *o.rates.iter().max().unwrap_or(&48_000) };
        // C7: the hw takes powers of two from 32
        let negotiated = block.next_power_of_two().max(32);
        let drift_extra = if i.id != o.id { 64 } else { 0 };
        Ok(Opened {
            input: i.id.clone(),
            output: o.id.clone(),
            rate: open_rate,
            block: negotiated,
            latency_in: negotiated + i.reported_extra + drift_extra,
            latency_out: negotiated + o.reported_extra,
            edge_resampler: edge,
        })
    });
    s.services.pending.push(Pending::Reopen(h));
}

/// Lists the audio devices and the MIDI ports again (U17, I-10.2): an operation
pub fn scan_devices(s: &mut Session) {
    if s.services.busy(|p| matches!(p, Pending::Scan(_))) {
        return;
    }
    let delay = s.services.delays.scan;
    let h = s.start("Looking for devices", Target::Scan, true, move |cancel, progress| {
        wait(delay, cancel, progress)?;
        Ok((audio_examples(), midi_examples()))
    });
    s.services.pending.push(Pending::Scan(h));
}

/// "Try again" of a notice (the argument is the notice's id)
pub fn retry(s: &mut Session, arg: &str) {
    let Ok(id) = arg.parse::<u64>() else { return };
    let Some(r) = s.services.retries.remove(&id) else { return };
    s.services.notices.dismiss(id);
    match r {
        Retry::Export(req) => drop(export(s, req)),
        Retry::Open(name) => {
            if let Some(i) = s.services.recent.iter().position(|r| r.name == name) {
                open_recent(s, i)
            }
        }
        Retry::Calibrate => calibrate(s),
        Retry::Reopen => reopen_device(s),
    }
}

/// Recreates a plugin from the project's tree (the process of a third-party plugin started again)
pub fn recreate_plugin(s: &mut Session, plugin: &str) {
    let plugin = plugin.to_string();
    let delay = s.services.delays.reopen;
    let h = s.start("Recreating the plugin", Target::Project, true, move |cancel, progress| wait(delay * 2, cancel, progress));
    s.services.notices.dismiss_matching(|n| n.actions.iter().any(|a| a.action == "plugin.recreate" && a.arg == plugin));
    s.services.pending.push(Pending::Recreate(h, plugin));
}

/// "Locate the file" (H17): the prototype has no file dialog in the core; the notice goes away as if found
pub fn locate_resource(s: &mut Session, what: &str) {
    let what = what.to_string();
    s.services.notices.dismiss_matching(|n| n.actions.iter().any(|a| a.action.starts_with("resource.") && (a.arg == what || what.is_empty())));
}

/// "Allow this folder" (C9)
pub fn allow_folder(s: &mut Session, folder: &str) {
    if !folder.is_empty() && !s.services.prefs.allowed.iter().any(|f| f == folder) {
        s.services.prefs.allowed.push(folder.to_string());
    }
    let folder = folder.to_string();
    s.services.notices.dismiss_matching(|n| n.actions.iter().any(|a| a.action == "folder.allow" && a.arg == folder));
    s.services.serial += 1;
}

/// Decides an interrupted take (U5): recovered onto its track at its place, or left out (its file stays)
pub fn decide_take(s: &mut Session, index: usize, recover: bool) {
    if let Some(t) = s.services.project.recovery.get_mut(index) {
        t.decision = Some(recover);
        s.services.project.modified = true;
        s.services.serial += 1;
    }
}

/// Turns a plugin that safe mode disabled back on (U21)
pub fn enable_safe_mode_plugin(s: &mut Session, index: usize, on: bool) {
    if let Some(p) = s.services.project.safe_mode.get_mut(index) {
        p.enabled = on;
        s.services.project.modified = true;
        s.services.serial += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_costs_more_than_fast_and_song_is_whole() {
        let song = Song { bars: 96, bpm: 120.0, loop_bars: (17, 33), render_speed: 38.0 };
        let mut req = ExportRequest {
            scope: Scope::Range { start: 81, end: 89, mode: RangeMode::Exact },
            formats: vec![Format::Wav],
            rate: 48_000,
            depth: Depth::I24,
            dither: true,
            mp3_kbps: 320,
            mp3_mode: Mp3Mode::Joint,
            ogg_quality: 6,
            flac_level: 5,
            normalize: Normalize::None,
            fade_in: 0.0,
            fade_out: 0.0,
            tail: false,
            repeats: 1,
            only_track: None,
            stems: Stems::None,
            folder: "x".into(),
            name: "y".into(),
        };
        let (exact, _) = export_cost(&song, &req);
        assert_eq!(exact, 80.0 * 2.0 + 16.0);
        req.scope = Scope::Range { start: 81, end: 89, mode: RangeMode::Fast { warmup: 10.0 } };
        let (fast, _) = export_cost(&song, &req);
        assert_eq!(fast, 26.0);
        // a warm-up longer than what is before the range starts at the beginning
        req.scope = Scope::Range { start: 2, end: 3, mode: RangeMode::Fast { warmup: 10.0 } };
        assert_eq!(export_cost(&song, &req).0, 4.0);
        req.scope = Scope::Song;
        assert_eq!(export_cost(&song, &req).0, 192.0);
        req.repeats = 2;
        assert_eq!(export_cost(&song, &req).0, 384.0);
    }

    #[test]
    fn allowed_folders_are_prefixes_of_whole_names() {
        let p = Prefs::default();
        assert!(p.allows("/usr/share/sounds/sf2/FluidR3.sf2"));
        assert!(!p.allows("/usr/share/sounds/sf2-other/x.sf2"));
    }
}
