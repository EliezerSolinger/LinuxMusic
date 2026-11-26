//! Resources (interfaces.md, I-12, T1; C8, C9, C10, C11; H3, H17; U14, U15; I-20.4; decisoes-autonomas.md P-32).
//!
//! The pure work runs inside `Operation`s the session starts: [`import_file`] (copy or convert into `audio/`, hash,
//! peaks), [`load_file`] (decode into an immutable `Arc<SampleData>`), and [`LoadJob::run`]. The state of what is
//! loaded is the [`ResourceStore`]'s, on the main thread: a resource is `Loading` from the moment it is asked until it
//! is whole, and only then enters the [`ResourceView`] (never half loaded, I-2.3); a file that is not where the
//! project says is `Missing` (H17); one unused for 5 minutes is evicted (C8).
//!
//! Items longer than 30 s are not decoded into memory (P-32): their state is `OnDisk`, and the disk streamer reads
//! them as they play.
//!
//! **Keys and paths** (C9): a key is exactly 64 lowercase hex digits (a BLAKE3 hash, C10); anything else is refused
//! before a path is ever made from it. A file in `audio/` that is a link out of the project is not followed. External
//! files (system soundfonts and kits) are followed only inside the folders the user allowed.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lm_base::{Cancel, Clock, Error, HostTime, Label, Progress, Result};
use lm_render::SampleData;
use lm_sys_samplerate::Quality;
use lm_sys_sndfile::{Container, Existing, Reader, Sample, WriteFormat, WriteSpec, Writer};

use crate::peaks::{Peaks, PeaksBuilder};
use crate::take::{free_space, io_error, new_uuid, sync_dir};

/// The length of a resource key (C9, C10)
pub const KEY_LEN: usize = 64;
/// Items longer than this play from disk instead of memory (P-32)
pub const STREAM_ABOVE: Duration = Duration::from_secs(30);
/// A loaded resource no program uses and nobody asked for in this long is evicted (C8)
pub const EVICT_AFTER: Duration = Duration::from_secs(300);
/// The extensions of audio resources, in the order they are looked for
pub const AUDIO_EXTENSIONS: [&str; 5] = ["wav", "flac", "aiff", "ogg", "mp3"];
const CHUNK: usize = 65_536;

/// Whether `key` is a valid resource key: 64 lowercase hex digits (C9)
pub fn is_valid_key(key: &str) -> bool {
    key.len() == KEY_LEN && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn check_key(key: &str) -> Result<()> {
    if is_valid_key(key) {
        Ok(())
    } else {
        let shown: String = key.chars().take(80).collect();
        Err(Error::new("io.resource.invalid_key").arg("key", shown))
    }
}

/// The project's `audio/` folder
pub fn audio_folder(project: &Path) -> PathBuf {
    project.join("audio")
}

/// The file of audio resource `key` in the project, if it is there (C9: the key is validated first; a link that
/// leads out of the project's `audio/` is refused with `io.resource.outside_project`)
pub fn resource_path(project: &Path, key: &str) -> Result<Option<PathBuf>> {
    resource_path_in(project, key, None)
}

/// A link inside a project folder that leads out of it: refused (`io.resource.outside_project`) when `allowed` is
/// `None`, followed only inside the allowed folders (C9) otherwise (`io.resource.not_allowed` with the folder to offer)
fn checked_link(p: &Path, root_folder: &Path, allowed: Option<&[PathBuf]>) -> Result<PathBuf> {
    let real = p.canonicalize().map_err(|e| io_error("io.resource.read_failed", p, &e))?;
    let root = root_folder.canonicalize().map_err(|e| io_error("io.resource.read_failed", root_folder, &e))?;
    if real.starts_with(&root) {
        return Ok(p.to_path_buf());
    }
    match allowed {
        None => Err(Error::new("io.resource.outside_project").arg("path", p.display().to_string())),
        Some(a) => resolve_external(&real, a),
    }
}

/// [`resource_path`] with the folders the user allowed (C9): a link out of `audio/` into one of them is followed
pub fn resource_path_in(project: &Path, key: &str, allowed: Option<&[PathBuf]>) -> Result<Option<PathBuf>> {
    check_key(key)?;
    let audio = audio_folder(project);
    for ext in AUDIO_EXTENSIONS {
        let p = audio.join(format!("{key}.{ext}"));
        if std::fs::symlink_metadata(&p).is_ok() {
            return checked_link(&p, &audio, allowed).map(Some);
        }
    }
    Ok(None)
}

/// An external file (a system soundfont) followed only inside an allowed folder (C9). Outside them:
/// `io.resource.not_allowed` with the folder, for the action "allow this folder"
pub fn resolve_external(path: &Path, allowed: &[PathBuf]) -> Result<PathBuf> {
    let real = path.canonicalize().map_err(|e| io_error("io.resource.missing", path, &e))?;
    for folder in allowed {
        if let Ok(root) = folder.canonicalize()
            && real.starts_with(&root)
        {
            return Ok(real);
        }
    }
    let folder = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    Err(Error::new("io.resource.not_allowed").arg("path", path.display().to_string()).arg("folder", folder))
}

/// What an import made
#[derive(Clone, Debug)]
pub struct ImportedFile {
    /// The resource key: the BLAKE3 hash of the file in `audio/`
    pub key: String,
    pub path: PathBuf,
    pub peaks_path: PathBuf,
    pub peaks: Arc<Peaks>,
    pub frames: u64,
    pub channels: usize,
    /// The project's rate (the file's after a conversion)
    pub rate: u32,
    /// The rate of the original
    pub source_rate: u32,
    /// Converted to the project's rate (arquitetura-nova.md, item 8): the original was not copied
    pub converted: bool,
}

/// Removes the file on drop unless it was kept (U14: a cancelled import leaves nothing half done)
struct TempFile {
    path: PathBuf,
    keep: bool,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Imports an audio file into the project (I-12), inside an `Operation`: checks the disk first (H3: fails before
/// copying, saying how much is missing), copies it (same rate) or converts it once at the best quality to the
/// project's rate (RF64 float), hashes it, writes its peaks, and renames the `.tmp` to `audio/<hash>.<ext>` only at
/// the end (U14). An identical file already there is reused. Cancellable, with progress
pub fn import_file(
    source: &Path,
    project: &Path,
    rate: u32,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<ImportedFile> {
    progress.stage(Label::new("io.import.reading"));
    let mut reader = Reader::open(source)?;
    let info = *reader.info();
    let audio = audio_folder(project);
    std::fs::create_dir_all(&audio).map_err(|e| io_error("io.import.write_failed", &audio, &e))?;
    let converted = info.rate != rate;
    let out_frames = if converted {
        lm_sys_samplerate::output_frames(info.frames as usize, info.rate, rate) as u64
    } else {
        info.frames
    };
    let needed = if converted {
        out_frames * info.channels as u64 * 4 + 4096
    } else {
        std::fs::metadata(source).map(|m| m.len()).unwrap_or(0)
    };
    // The peaks and a margin to close cleanly
    let needed = needed + needed / 16 + (1 << 20);
    let free = free_space(&audio)?;
    if free < needed {
        return Err(Error::new("io.import.disk_full")
            .arg("needed", needed as i64)
            .arg("free", free as i64)
            .arg("missing", (needed - free) as i64));
    }
    let mut tmp = TempFile { path: audio.join(format!(".import-{}.tmp", new_uuid())), keep: false };
    let (key, ext, peaks) = if converted {
        progress.stage(Label::new("io.import.converting"));
        let mut all = Vec::with_capacity(info.frames as usize * info.channels);
        let mut buf = vec![0.0f32; CHUNK * info.channels];
        loop {
            cancel.check()?;
            let n = reader.read_f32(&mut buf)?;
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n * info.channels]);
            progress.set(0.3 * all.len() as f32 / (info.frames.max(1) as f32 * info.channels as f32));
        }
        let out = lm_sys_samplerate::convert(&all, info.channels, info.rate, rate, Quality::Best, cancel, progress)?;
        drop(all);
        progress.stage(Label::new("io.import.writing"));
        let mut w = Writer::create(
            &tmp.path,
            WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate, channels: info.channels },
            Existing::Replace,
        )?;
        let mut builder = PeaksBuilder::new(info.channels);
        for chunk in out.chunks(CHUNK * info.channels) {
            cancel.check()?;
            w.write_f32(chunk)?;
            builder.push_interleaved(chunk);
        }
        w.close()?;
        let key = crate::take::hash_file(&tmp.path, cancel, progress)?;
        (key, "wav", builder.finish())
    } else {
        progress.stage(Label::new("io.import.copying"));
        let mut src = File::open(source).map_err(|e| io_error("io.import.read_failed", source, &e))?;
        let mut dst = File::create(&tmp.path).map_err(|e| io_error("io.import.write_failed", &tmp.path, &e))?;
        let total = std::fs::metadata(source).map(|m| m.len()).unwrap_or(1).max(1);
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut done = 0u64;
        loop {
            cancel.check()?;
            let n = src.read(&mut buf).map_err(|e| io_error("io.import.read_failed", source, &e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            dst.write_all(&buf[..n]).map_err(|e| {
                if e.raw_os_error() == Some(28) {
                    Error::new("io.import.disk_full").arg("path", tmp.path.display().to_string())
                } else {
                    io_error("io.import.write_failed", &tmp.path, &e)
                }
            })?;
            done += n as u64;
            progress.set(0.5 * done as f32 / total as f32);
        }
        dst.sync_all().map_err(|e| io_error("io.import.write_failed", &tmp.path, &e))?;
        drop(dst);
        progress.stage(Label::new("io.import.peaks"));
        let peaks = crate::take::peaks_of_file(&tmp.path, cancel, progress)?;
        let ext = match info.container {
            Container::Wav | Container::Rf64 => "wav",
            Container::Aiff => "aiff",
            Container::Flac => "flac",
            Container::Ogg => "ogg",
            Container::Mpeg => "mp3",
            Container::Other(_) => "wav",
        };
        (hasher.finalize().to_hex().to_string(), ext, peaks)
    };
    cancel.check()?;
    let path = audio.join(format!("{key}.{ext}"));
    if path.exists() {
        // The same content is already in the project
        drop(tmp);
    } else {
        std::fs::rename(&tmp.path, &path).map_err(|e| io_error("io.import.write_failed", &path, &e))?;
        tmp.keep = true;
    }
    let peaks_path = audio.join(format!("{key}.peaks"));
    if Peaks::read(&peaks_path).map(|p| p != peaks).unwrap_or(true) {
        peaks.write(&peaks_path)?;
    }
    sync_dir(&audio);
    progress.set(1.0);
    Ok(ImportedFile {
        key,
        path,
        peaks_path,
        peaks: Arc::new(peaks),
        frames: out_frames,
        channels: info.channels,
        rate,
        source_rate: info.rate,
        converted,
    })
}

/// Removes the temporary files a crash left in `audio/`, `state/` and `render/` (U14: imports, conversions, peaks,
/// renders; every one ends in `.tmp`), when a project is opened. Returns how many
pub fn clean_import_leftovers(project: &Path) -> Result<usize> {
    let mut n = 0;
    for folder in [audio_folder(project), state_folder(project), render_folder(project)] {
        let entries = match std::fs::read_dir(&folder) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io_error("io.resource.read_failed", &folder, &e)),
        };
        for e in entries.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.ends_with(".tmp") {
                std::fs::remove_file(e.path()).map_err(|err| io_error("io.resource.remove_failed", &e.path(), &err))?;
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Decodes a whole file into immutable planar audio (I-12), inside an `Operation`. Cancellable, with progress
pub fn load_file(path: &Path, cancel: &Cancel, progress: &Progress) -> Result<Arc<SampleData>> {
    let mut reader = Reader::open(path)?;
    let info = *reader.info();
    let ch = info.channels;
    let frames = info.frames as usize;
    let mut planar = vec![0.0f32; frames * ch];
    let mut buf = vec![0.0f32; CHUNK * ch];
    let mut at = 0usize;
    loop {
        cancel.check()?;
        let n = reader.read_f32(&mut buf)?;
        if n == 0 {
            break;
        }
        let n = n.min(frames - at);
        for f in 0..n {
            for c in 0..ch {
                planar[c * frames + at + f] = buf[f * ch + c];
            }
        }
        at += n;
        progress.set(at as f32 / frames.max(1) as f32);
        if at == frames {
            break;
        }
    }
    if at < frames {
        // A file shorter than its header says (a take cut by a crash): the frames there are
        let mut short = vec![0.0f32; at * ch];
        for c in 0..ch {
            short[c * at..(c + 1) * at].copy_from_slice(&planar[c * frames..c * frames + at]);
        }
        planar = short;
    }
    Ok(Arc::new(SampleData::from_planar(info.rate, ch, planar.into_boxed_slice())?))
}

/// What the loaded resources are now: immutable, cheap to clone, handed to the compiler (I-12)
#[derive(Clone, Debug, Default)]
pub struct ResourceView {
    data: Arc<BTreeMap<String, Arc<SampleData>>>,
    on_disk: Arc<BTreeMap<String, OnDisk>>,
    others: Arc<BTreeMap<(String, String), Content>>,
}

impl ResourceView {
    /// The decoded audio of `key`, when it is loaded in memory
    pub fn get(&self, key: &str) -> Option<&Arc<SampleData>> {
        self.data.get(key)
    }

    /// A long resource that plays from disk (P-32)
    pub fn on_disk(&self, key: &str) -> Option<&OnDisk> {
        self.on_disk.get(key)
    }

    /// Whether `key` is ready to play (in memory or from disk)
    pub fn is_ready(&self, key: &str) -> bool {
        self.data.contains_key(key) || self.on_disk.contains_key(key)
    }

    /// A tempo render, decoded in memory (F-REC-090)
    pub fn render(&self, key: &str) -> Option<&Arc<SampleData>> {
        self.data.get(&format!("render/{key}"))
    }

    /// A long tempo render that plays from disk (P-32)
    pub fn render_on_disk(&self, key: &str) -> Option<&OnDisk> {
        self.on_disk.get(&format!("render/{key}"))
    }

    /// A resource of another kind (a NAM model, an impulse response, a SoundFont): its bytes or its checked file
    pub fn content(&self, kind: &str, key: &str) -> Option<&Content> {
        self.others.get(&(kind.to_owned(), key.to_owned()))
    }

    pub fn len(&self) -> usize {
        self.data.len() + self.on_disk.len() + self.others.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A long resource that plays from disk (P-32): what the disk streamer needs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnDisk {
    pub path: PathBuf,
    pub rate: u32,
    pub channels: usize,
    pub frames: u64,
}

/// Faults injected in test mode (I-2.4), to check that the loading state shows and that nothing half loaded is
/// ever shown: a delay before every load, and a hold that keeps every load waiting while it is set (a test sets it,
/// moves its injected clock, checks, then clears it: no race with the real time)
#[derive(Clone, Debug, Default)]
pub struct IoFaults {
    pub load_delay: Duration,
    pub hold: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl IoFaults {
    /// The injected wait: the delay, then the hold while it is set. Cancellable
    fn wait(&self, cancel: &Cancel) -> Result<()> {
        if !self.load_delay.is_zero() {
            let until = std::time::Instant::now() + self.load_delay;
            while std::time::Instant::now() < until {
                cancel.check()?;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if let Some(hold) = &self.hold {
            while hold.load(std::sync::atomic::Ordering::Acquire) {
                cancel.check()?;
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(())
    }
}

/// A load to run in an `Operation` (T1: the session starts it)
#[derive(Clone, Debug)]
pub struct LoadJob {
    pub key: String,
    pub path: PathBuf,
    faults: IoFaults,
    stream_above: Duration,
}

/// What a load made
#[derive(Clone, Debug)]
pub struct Loaded {
    pub key: String,
    /// Decoded, for a short resource
    pub data: Option<Arc<SampleData>>,
    /// Played from disk, for a long one (P-32)
    pub on_disk: Option<OnDisk>,
    pub peaks: Arc<Peaks>,
}

impl LoadJob {
    /// Loads the resource: its peaks (read from `audio/<key>.peaks`, or built and written), then the audio,
    /// decoded unless it is longer than 30 s. Cancellable, with progress
    pub fn run(self, cancel: &Cancel, progress: &Progress) -> Result<Loaded> {
        self.faults.wait(cancel)?;
        progress.stage(Label::new("io.resource.peaks"));
        let peaks_path = self.path.with_extension("peaks");
        let info = *Reader::open(&self.path)?.info();
        let peaks = match Peaks::read(&peaks_path) {
            Ok(p) if p.frames() == info.frames && p.channels() == info.channels => p,
            _ => {
                let p = crate::take::peaks_of_file(&self.path, cancel, progress)?;
                p.write(&peaks_path)?;
                p
            }
        };
        let seconds = info.frames as f64 / f64::from(info.rate.max(1));
        progress.stage(Label::new("io.resource.loading"));
        if seconds > self.stream_above.as_secs_f64() {
            return Ok(Loaded {
                key: self.key,
                data: None,
                on_disk: Some(OnDisk {
                    path: self.path,
                    rate: info.rate,
                    channels: info.channels,
                    frames: info.frames,
                }),
                peaks: Arc::new(peaks),
            });
        }
        let data = load_file(&self.path, cancel, progress)?;
        Ok(Loaded { key: self.key, data: Some(data), on_disk: None, peaks: Arc::new(peaks) })
    }
}

/// The state of one resource, as the session sees it
#[derive(Clone, Debug, PartialEq)]
pub enum ResourceState {
    /// Never asked for
    Absent,
    /// Being loaded since `since`
    Loading {
        since: HostTime,
    },
    /// In the view, in memory
    Ready,
    /// In the view, played from disk (P-32)
    OnDisk,
    /// The file is not where the project says (H17): "locate file", "locate folder"
    Missing,
    Failed(Error),
}

#[derive(Debug)]
struct Entry {
    state: ResourceState,
    peaks: Option<Arc<Peaks>>,
    last_used: HostTime,
}

/// What is loaded (I-12), on the main thread
pub struct ResourceStore {
    project: PathBuf,
    entries: BTreeMap<String, Entry>,
    data: BTreeMap<String, Arc<SampleData>>,
    on_disk: BTreeMap<String, OnDisk>,
    others: BTreeMap<(String, String), Content>,
    view: ResourceView,
    clock: Arc<dyn Clock>,
    pub evict_after: Duration,
    pub stream_above: Duration,
    pub faults: IoFaults,
    /// The folders the user allowed (`allowedFolders`, C9): a link in the project may lead into them
    allowed: Vec<PathBuf>,
    /// The loads refused for lack of an allowed folder (`io.resource.not_allowed`): the resource is `Missing`; the
    /// session offers "allow this folder"
    refused: Vec<Error>,
}

impl std::fmt::Debug for ResourceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceStore").field("project", &self.project).field("entries", &self.entries.len()).finish()
    }
}

impl ResourceStore {
    pub fn new(project: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self {
            project,
            entries: BTreeMap::new(),
            data: BTreeMap::new(),
            on_disk: BTreeMap::new(),
            others: BTreeMap::new(),
            view: ResourceView::default(),
            clock,
            evict_after: EVICT_AFTER,
            stream_above: STREAM_ABOVE,
            faults: IoFaults::default(),
            allowed: Vec::new(),
            refused: Vec::new(),
        }
    }

    /// The folders the user allowed (C9); loads that failed for lack of one are asked for again by the session
    pub fn set_allowed(&mut self, folders: Vec<PathBuf>) {
        self.allowed = folders;
    }

    /// The refusals since the last call (each `io.resource.not_allowed`, with the `folder` to offer)
    pub fn take_refused(&mut self) -> Vec<Error> {
        std::mem::take(&mut self.refused)
    }

    /// A refused link is `Missing` for the project (C9), and remembered for the notice
    fn missing_if_refused(&mut self, path: Result<Option<PathBuf>>) -> Result<Option<PathBuf>> {
        match path {
            Err(e) if e.key() == "io.resource.not_allowed" => {
                if !self.refused.iter().any(|r| r == &e) {
                    self.refused.push(e);
                }
                Ok(None)
            }
            other => other,
        }
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    /// What is loaded now (immutable: hand it to the compiler)
    pub fn view(&self) -> ResourceView {
        self.view.clone()
    }

    /// The peaks of a resource, once loaded (the interface draws from them, never from the audio)
    pub fn peaks(&self, key: &str) -> Option<Arc<Peaks>> {
        self.entries.get(key).and_then(|e| e.peaks.clone())
    }

    pub fn state(&self, key: &str) -> ResourceState {
        self.entries.get(key).map_or(ResourceState::Absent, |e| e.state.clone())
    }

    /// Starts loading `key`: the job to run in an `Operation`, and the resource is `Loading` from now. `Err` with the
    /// state when there is nothing to start (already loading, ready, or the file is missing: `Missing`, H17). An
    /// invalid key is refused (C9)
    pub fn begin_load(&mut self, key: &str) -> std::result::Result<LoadJob, ResourceState> {
        let path = resource_path_in(&self.project, key, Some(&self.allowed));
        let path = self.missing_if_refused(path);
        self.begin_audio_like(key.to_owned(), key, path)
    }

    /// Starts loading a tempo render (`render/<key>.wav`, F-REC-090): decoded like audio, with peaks and the 30 s
    /// rule; it enters the view as [`ResourceView::render`]. The key names the render's plan, not its content: the
    /// folder is a rebuildable cache, apart from `audio/`, whose keys are content hashes (C10)
    pub fn begin_load_render(&mut self, key: &str) -> std::result::Result<LoadJob, ResourceState> {
        let path = resource_path_of(&self.project, "render", key);
        self.begin_audio_like(entry_key("render", key), key, path)
    }

    /// The outcome of a render's load
    pub fn finish_render(&mut self, key: &str, result: Result<Loaded>) {
        self.finish_audio_like(entry_key("render", key), result);
    }

    fn begin_audio_like(
        &mut self,
        ek: String,
        key: &str,
        path: Result<Option<PathBuf>>,
    ) -> std::result::Result<LoadJob, ResourceState> {
        if let Some(e) = self.entries.get_mut(&ek) {
            match e.state {
                ResourceState::Loading { .. } | ResourceState::Ready | ResourceState::OnDisk => {
                    e.last_used = self.clock.now();
                    return Err(e.state.clone());
                }
                _ => {}
            }
        }
        let now = self.clock.now();
        let path = match path {
            Ok(Some(p)) => p,
            Ok(None) => {
                self.entries.insert(ek, Entry { state: ResourceState::Missing, peaks: None, last_used: now });
                return Err(ResourceState::Missing);
            }
            Err(e) => {
                let state = ResourceState::Failed(e);
                self.entries.insert(ek, Entry { state: state.clone(), peaks: None, last_used: now });
                return Err(state);
            }
        };
        self.entries.insert(ek, Entry { state: ResourceState::Loading { since: now }, peaks: None, last_used: now });
        Ok(LoadJob { key: key.to_owned(), path, faults: self.faults.clone(), stream_above: self.stream_above })
    }

    /// The outcome of a load: the resource enters the view whole, in one step, or fails
    pub fn finish(&mut self, key: &str, result: Result<Loaded>) {
        self.finish_audio_like(key.to_owned(), result);
    }

    fn finish_audio_like(&mut self, ek: String, result: Result<Loaded>) {
        let now = self.clock.now();
        match result {
            Ok(l) => {
                if let Some(d) = l.data {
                    self.loaded(ek, d, l.peaks);
                } else if let Some(od) = l.on_disk {
                    self.on_disk.insert(ek.clone(), od);
                    self.entries
                        .insert(ek, Entry { state: ResourceState::OnDisk, peaks: Some(l.peaks), last_used: now });
                    self.publish();
                }
            }
            // A cancelled load leaves the resource as if never asked
            Err(e) if e.is_cancelled() => {
                self.entries.remove(&ek);
            }
            Err(e) => {
                self.entries.insert(ek, Entry { state: ResourceState::Failed(e), peaks: None, last_used: now });
            }
        }
    }

    /// The peaks of a render, once loaded
    pub fn render_peaks(&self, key: &str) -> Option<Arc<Peaks>> {
        self.entries.get(&entry_key("render", key)).and_then(|e| e.peaks.clone())
    }

    /// A resource made elsewhere (an import, a finalised take): it enters the view whole (I-12)
    pub fn loaded(&mut self, key: String, data: Arc<SampleData>, peaks: Arc<Peaks>) {
        let now = self.clock.now();
        self.data.insert(key.clone(), data);
        self.entries.insert(key, Entry { state: ResourceState::Ready, peaks: Some(peaks), last_used: now });
        self.publish();
    }

    /// The resource is still used (by a program alive now, or asked again): keeps it from eviction (C8)
    pub fn touch(&mut self, key: &str) {
        let now = self.clock.now();
        if let Some(e) = self.entries.get_mut(key) {
            e.last_used = now;
        }
    }

    /// Evicts what no live program uses (`in_use`) and nobody asked for in `evict_after` (C8). The audio comes back
    /// to the caller, who sends it to the disposal thread (U15): a big `Arc` is never freed on the main thread
    pub fn evict(&mut self, in_use: &dyn Fn(&str) -> bool) -> Vec<Arc<SampleData>> {
        let now = self.clock.now();
        let limit = self.evict_after.as_nanos() as u64;
        let old: Vec<String> = self
            .entries
            .iter()
            .filter(|(k, e)| {
                matches!(e.state, ResourceState::Ready | ResourceState::OnDisk)
                    && now.since(e.last_used) >= limit
                    && !in_use(k)
            })
            .map(|(k, _)| k.clone())
            .collect();
        let mut out = Vec::new();
        for k in &old {
            self.entries.remove(k);
            if let Some((kind, key)) = k.split_once('/') {
                self.others.remove(&(kind.to_owned(), key.to_owned()));
            }
            // Renders keep their audio under the composite key ("render/<key>")
            self.on_disk.remove(k);
            if let Some(d) = self.data.remove(k) {
                out.push(d);
            }
        }
        if !old.is_empty() {
            self.publish();
        }
        out
    }

    /// A missing resource was located (H17), after its hash was checked ([`verify_located`]): copy or link it into
    /// `audio/` is the session's choice; here it is forgotten so it can be loaded again
    pub fn forget(&mut self, key: &str) {
        self.entries.remove(key);
        self.data.remove(key);
        if self.on_disk.remove(key).is_some() {
            self.publish();
        }
    }

    fn publish(&mut self) {
        self.view = ResourceView {
            data: Arc::new(self.data.clone()),
            on_disk: Arc::new(self.on_disk.clone()),
            others: Arc::new(self.others.clone()),
        };
    }
}

/// H17: a file the user chose for a missing resource is that resource only if its hash is the key. Inside an
/// `Operation`
pub fn verify_located(path: &Path, key: &str, cancel: &Cancel, progress: &Progress) -> Result<bool> {
    check_key(key)?;
    Ok(crate::take::hash_file(path, cancel, progress)? == key)
}

/// How a kind of resource is loaded (a table of `lm-io`'s): audio is decoded; small data is read into memory; large
/// libraries stay files that the plugin opens itself
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindPolicy {
    /// `audio/`: decoded into `SampleData`, with peaks (I-12)
    Audio,
    /// `state/`: read whole into memory (NAM models, impulse responses, opaque state, U24), up to [`BYTES_MAX`]
    Bytes,
    /// `state/`: a checked path (SoundFonts, GIG and SFZ libraries, kits): the plugin reads it in its build
    File,
}

/// The largest file loaded as bytes; a bigger one is handed over as a file
pub const BYTES_MAX: u64 = 64 << 20;

/// The policy of a resource kind
pub fn kind_policy(kind: &str) -> KindPolicy {
    match kind {
        "audio" | "render" => KindPolicy::Audio,
        "nam" | "ir" | "state" | "wavetable" | "preset" | "pat" => KindPolicy::Bytes,
        // "sf2", "sf3", "sfz", "gig", "kit", "soundfont" and anything not known: the plugin opens the file
        _ => KindPolicy::File,
    }
}

/// Whether `kind` is a valid resource kind: 1 to 32 of `a-z`, `0-9`, `-`, `_`, starting with a letter (C9: it never
/// names a path)
pub fn is_valid_kind(kind: &str) -> bool {
    let b = kind.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// What a non-audio resource is once loaded
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Bytes(Arc<[u8]>),
    File(PathBuf),
}

/// The project's `render/` folder: tempo renders (F-REC-090), named by their plan, a cache that can be rebuilt
pub fn render_folder(project: &Path) -> PathBuf {
    project.join("render")
}

/// The project's `state/` folder (non-audio resources, U24)
pub fn state_folder(project: &Path) -> PathBuf {
    project.join("state")
}

/// The file of resource (`kind`, `key`) in the project, if it is there (C9: kind and key validated first; a link out
/// of its folder is refused). Audio is `audio/<key>.<audio extension>`; any other kind is `state/<key>.<extension>`
pub fn resource_path_of(project: &Path, kind: &str, key: &str) -> Result<Option<PathBuf>> {
    resource_path_of_in(project, kind, key, None)
}

/// [`resource_path_of`] with the folders the user allowed (C9)
pub fn resource_path_of_in(
    project: &Path,
    kind: &str,
    key: &str,
    allowed: Option<&[PathBuf]>,
) -> Result<Option<PathBuf>> {
    if !is_valid_kind(kind) {
        return Err(Error::new("io.resource.invalid_kind").arg("kind", kind.chars().take(40).collect::<String>()));
    }
    if kind == "audio" {
        return resource_path_in(project, key, allowed);
    }
    check_key(key)?;
    if kind == "render" {
        let folder = render_folder(project);
        let p = folder.join(format!("{key}.wav"));
        if std::fs::symlink_metadata(&p).is_err() {
            return Ok(None);
        }
        return checked_link(&p, &folder, allowed).map(Some);
    }
    let folder = state_folder(project);
    let entries = match std::fs::read_dir(&folder) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error("io.resource.read_failed", &folder, &e)),
    };
    let prefix = format!("{key}.");
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with(&prefix) && !n.ends_with(".tmp")))
        .map(|e| e.path())
        .collect();
    found.sort();
    let Some(p) = found.into_iter().next() else { return Ok(None) };
    checked_link(&p, &folder, allowed).map(Some)
}

/// What a non-audio import made
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedResource {
    pub kind: String,
    /// The BLAKE3 hash of the file, lowercase hex
    pub key: String,
    pub path: PathBuf,
    pub bytes: u64,
}

/// Imports a file of a non-audio kind into `state/` (I-12, U24), inside an `Operation`: the disk checked first (H3),
/// copied through a `.tmp` while hashing, renamed to `state/<hash>.<extension>` at the end (U14). Nothing decoded.
/// An identical file already there is reused. Audio goes through [`import_file`]
pub fn import_file_as(
    source: &Path,
    project: &Path,
    kind: &str,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<ImportedResource> {
    if !is_valid_kind(kind) || kind == "audio" {
        return Err(Error::new("io.resource.invalid_kind").arg("kind", kind.chars().take(40).collect::<String>()));
    }
    progress.stage(Label::new("io.import.copying"));
    let folder = state_folder(project);
    std::fs::create_dir_all(&folder).map_err(|e| io_error("io.import.write_failed", &folder, &e))?;
    let size = std::fs::metadata(source).map_err(|e| io_error("io.import.read_failed", source, &e))?.len();
    let needed = size + (1 << 20);
    let free = free_space(&folder)?;
    if free < needed {
        return Err(Error::new("io.import.disk_full")
            .arg("needed", needed as i64)
            .arg("free", free as i64)
            .arg("missing", (needed - free) as i64));
    }
    // The extension, lowercase letters and digits only (it ends up in a file name)
    let ext: String = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(10)
        .collect::<String>()
        .to_ascii_lowercase();
    let ext = if ext.is_empty() { "bin".to_owned() } else { ext };
    let mut tmp = TempFile { path: folder.join(format!(".import-{}.tmp", new_uuid())), keep: false };
    let mut src = File::open(source).map_err(|e| io_error("io.import.read_failed", source, &e))?;
    let mut dst = File::create(&tmp.path).map_err(|e| io_error("io.import.write_failed", &tmp.path, &e))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        cancel.check()?;
        let n = src.read(&mut buf).map_err(|e| io_error("io.import.read_failed", source, &e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        dst.write_all(&buf[..n]).map_err(|e| io_error("io.import.write_failed", &tmp.path, &e))?;
        done += n as u64;
        progress.set(done as f32 / size.max(1) as f32);
    }
    dst.sync_all().map_err(|e| io_error("io.import.write_failed", &tmp.path, &e))?;
    drop(dst);
    cancel.check()?;
    let key = hasher.finalize().to_hex().to_string();
    let path = match resource_path_of(project, kind, &key)? {
        // The same content is already in the project (perhaps under another extension)
        Some(existing) => existing,
        None => {
            let path = folder.join(format!("{key}.{ext}"));
            std::fs::rename(&tmp.path, &path).map_err(|e| io_error("io.import.write_failed", &path, &e))?;
            tmp.keep = true;
            path
        }
    };
    drop(tmp);
    sync_dir(&folder);
    progress.set(1.0);
    Ok(ImportedResource { kind: kind.to_owned(), key, path, bytes: done })
}

/// A load of a non-audio resource, to run in an `Operation`
#[derive(Clone, Debug)]
pub struct KindJob {
    pub kind: String,
    pub key: String,
    pub path: PathBuf,
    faults: IoFaults,
}

/// What a non-audio load made
#[derive(Clone, Debug)]
pub struct KindLoaded {
    pub kind: String,
    pub key: String,
    pub content: Content,
}

impl KindJob {
    /// Reads the bytes (kinds loaded as bytes, up to [`BYTES_MAX`]) or checks the file can be opened (the others).
    /// Cancellable, with progress
    pub fn run(self, cancel: &Cancel, progress: &Progress) -> Result<KindLoaded> {
        self.faults.wait(cancel)?;
        progress.stage(Label::new("io.resource.loading"));
        let size =
            std::fs::metadata(&self.path).map_err(|e| io_error("io.resource.read_failed", &self.path, &e))?.len();
        let content = if kind_policy(&self.kind) == KindPolicy::Bytes && size <= BYTES_MAX {
            let mut f = File::open(&self.path).map_err(|e| io_error("io.resource.read_failed", &self.path, &e))?;
            let mut bytes = Vec::with_capacity(size as usize);
            let mut buf = vec![0u8; 1 << 20];
            loop {
                cancel.check()?;
                let n = f.read(&mut buf).map_err(|e| io_error("io.resource.read_failed", &self.path, &e))?;
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                progress.set(bytes.len() as f32 / size.max(1) as f32);
            }
            Content::Bytes(Arc::from(bytes.into_boxed_slice()))
        } else {
            File::open(&self.path).map_err(|e| io_error("io.resource.read_failed", &self.path, &e))?;
            Content::File(self.path.clone())
        };
        progress.set(1.0);
        Ok(KindLoaded { kind: self.kind, key: self.key, content })
    }
}

fn entry_key(kind: &str, key: &str) -> String {
    format!("{kind}/{key}")
}

impl ResourceStore {
    /// Starts loading a resource of a non-audio kind (I-12): `Loading` from now; `Err` with the state when there is
    /// nothing to start (loading, ready, `Missing`, or an invalid kind or key, C9). Audio uses [`ResourceStore::begin_load`]
    pub fn begin_load_kind(&mut self, kind: &str, key: &str) -> std::result::Result<KindJob, ResourceState> {
        if kind_policy(kind) == KindPolicy::Audio {
            return Err(ResourceState::Failed(Error::new("io.resource.use_begin_load")));
        }
        let ek = entry_key(kind, key);
        let now = self.clock.now();
        if let Some(e) = self.entries.get_mut(&ek) {
            match e.state {
                ResourceState::Loading { .. } | ResourceState::Ready => {
                    e.last_used = now;
                    return Err(e.state.clone());
                }
                _ => {}
            }
        }
        let found = resource_path_of_in(&self.project, kind, key, Some(&self.allowed));
        let path = match self.missing_if_refused(found) {
            Ok(Some(p)) => p,
            Ok(None) => {
                self.entries.insert(ek, Entry { state: ResourceState::Missing, peaks: None, last_used: now });
                return Err(ResourceState::Missing);
            }
            Err(e) => {
                let state = ResourceState::Failed(e);
                self.entries.insert(ek, Entry { state: state.clone(), peaks: None, last_used: now });
                return Err(state);
            }
        };
        self.entries.insert(ek, Entry { state: ResourceState::Loading { since: now }, peaks: None, last_used: now });
        Ok(KindJob { kind: kind.to_owned(), key: key.to_owned(), path, faults: self.faults.clone() })
    }

    /// The outcome of a non-audio load: it enters the view whole, or fails
    pub fn finish_kind(&mut self, kind: &str, key: &str, result: Result<KindLoaded>) {
        let ek = entry_key(kind, key);
        let now = self.clock.now();
        match result {
            Ok(l) => {
                self.others.insert((kind.to_owned(), key.to_owned()), l.content);
                self.entries.insert(ek, Entry { state: ResourceState::Ready, peaks: None, last_used: now });
                self.publish();
            }
            Err(e) if e.is_cancelled() => {
                self.entries.remove(&ek);
            }
            Err(e) => {
                self.entries.insert(ek, Entry { state: ResourceState::Failed(e), peaks: None, last_used: now });
            }
        }
    }

    /// The state of a non-audio resource
    pub fn state_kind(&self, kind: &str, key: &str) -> ResourceState {
        if kind == "audio" {
            return self.state(key);
        }
        self.entries.get(&entry_key(kind, key)).map_or(ResourceState::Absent, |e| e.state.clone())
    }

    /// Keeps a non-audio resource from eviction (C8)
    pub fn touch_kind(&mut self, kind: &str, key: &str) {
        let now = self.clock.now();
        let ek = if kind == "audio" { key.to_owned() } else { entry_key(kind, key) };
        if let Some(e) = self.entries.get_mut(&ek) {
            e.last_used = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_hashes_only() {
        assert!(is_valid_key(&"a".repeat(64)));
        for bad in ["../../etc/passwd", "/abs", &"A".repeat(64), &"a".repeat(63), &format!("{}/", "a".repeat(63))] {
            assert!(!is_valid_key(bad), "{bad}");
        }
        assert_eq!(resource_path(Path::new("/tmp"), "../x").expect_err("refused").key(), "io.resource.invalid_key");
    }
}
