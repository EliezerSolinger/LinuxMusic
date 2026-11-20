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
    check_key(key)?;
    let audio = audio_folder(project);
    for ext in AUDIO_EXTENSIONS {
        let p = audio.join(format!("{key}.{ext}"));
        if std::fs::symlink_metadata(&p).is_ok() {
            let real = p.canonicalize().map_err(|e| io_error("io.resource.read_failed", &p, &e))?;
            let root = audio.canonicalize().map_err(|e| io_error("io.resource.read_failed", &audio, &e))?;
            if !real.starts_with(&root) {
                return Err(Error::new("io.resource.outside_project").arg("path", p.display().to_string()));
            }
            return Ok(Some(p));
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
pub fn import_file(source: &Path, project: &Path, rate: u32, cancel: &Cancel, progress: &Progress) -> Result<ImportedFile> {
    progress.stage(Label::new("io.import.reading"));
    let mut reader = Reader::open(source)?;
    let info = reader.info().clone();
    let audio = audio_folder(project);
    std::fs::create_dir_all(&audio).map_err(|e| io_error("io.import.write_failed", &audio, &e))?;
    let converted = info.rate != rate;
    let out_frames = if converted { lm_sys_samplerate::output_frames(info.frames as usize, info.rate, rate) as u64 } else { info.frames };
    let needed = if converted {
        out_frames * info.channels as u64 * 4 + 4096
    } else {
        std::fs::metadata(source).map(|m| m.len()).unwrap_or(0)
    };
    // The peaks and a margin to close cleanly
    let needed = needed + needed / 16 + (1 << 20);
    let free = free_space(&audio)?;
    if free < needed {
        return Err(Error::new("io.import.disk_full").arg("needed", needed as i64).arg("free", free as i64).arg("missing", (needed - free) as i64));
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
        let mut w = Writer::create(&tmp.path, WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate, channels: info.channels }, Existing::Replace)?;
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

/// Removes the `.import-*.tmp` files a crash left in `audio/` (U14), when a project is opened. Returns how many
pub fn clean_import_leftovers(project: &Path) -> Result<usize> {
    let audio = audio_folder(project);
    let entries = match std::fs::read_dir(&audio) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(io_error("io.resource.read_failed", &audio, &e)),
    };
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if (name.starts_with(".import-") && name.ends_with(".tmp")) || name.ends_with(".convert.tmp") || name.ends_with(".peaks.tmp") {
            std::fs::remove_file(e.path()).map_err(|err| io_error("io.resource.remove_failed", &e.path(), &err))?;
            n += 1;
        }
    }
    Ok(n)
}

/// Decodes a whole file into immutable planar audio (I-12), inside an `Operation`. Cancellable, with progress
pub fn load_file(path: &Path, cancel: &Cancel, progress: &Progress) -> Result<Arc<SampleData>> {
    let mut reader = Reader::open(path)?;
    let info = reader.info().clone();
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

    pub fn len(&self) -> usize {
        self.data.len() + self.on_disk.len()
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

/// Faults injected in test mode (I-2.4): a delay before every load, to check that the loading state shows and
/// that nothing half loaded is ever shown
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IoFaults {
    pub load_delay: Duration,
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
        if !self.faults.load_delay.is_zero() {
            let until = std::time::Instant::now() + self.faults.load_delay;
            while std::time::Instant::now() < until {
                cancel.check()?;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        progress.stage(Label::new("io.resource.peaks"));
        let peaks_path = self.path.with_extension("peaks");
        let info = Reader::open(&self.path)?.info().clone();
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
                on_disk: Some(OnDisk { path: self.path, rate: info.rate, channels: info.channels, frames: info.frames }),
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
    Loading { since: HostTime },
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
    view: ResourceView,
    clock: Arc<dyn Clock>,
    pub evict_after: Duration,
    pub stream_above: Duration,
    pub faults: IoFaults,
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
            view: ResourceView::default(),
            clock,
            evict_after: EVICT_AFTER,
            stream_above: STREAM_ABOVE,
            faults: IoFaults::default(),
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
        if let Some(e) = self.entries.get_mut(key) {
            match e.state {
                ResourceState::Loading { .. } | ResourceState::Ready | ResourceState::OnDisk => {
                    e.last_used = self.clock.now();
                    return Err(e.state.clone());
                }
                _ => {}
            }
        }
        let now = self.clock.now();
        let path = match resource_path(&self.project, key) {
            Ok(Some(p)) => p,
            Ok(None) => {
                self.entries.insert(key.to_owned(), Entry { state: ResourceState::Missing, peaks: None, last_used: now });
                return Err(ResourceState::Missing);
            }
            Err(e) => {
                let state = ResourceState::Failed(e);
                self.entries.insert(key.to_owned(), Entry { state: state.clone(), peaks: None, last_used: now });
                return Err(state);
            }
        };
        self.entries.insert(key.to_owned(), Entry { state: ResourceState::Loading { since: now }, peaks: None, last_used: now });
        Ok(LoadJob { key: key.to_owned(), path, faults: self.faults, stream_above: self.stream_above })
    }

    /// The outcome of a load: the resource enters the view whole, in one step, or fails
    pub fn finish(&mut self, key: &str, result: Result<Loaded>) {
        let now = self.clock.now();
        match result {
            Ok(l) => {
                if let Some(d) = l.data {
                    self.loaded(key.to_owned(), d, l.peaks);
                } else if let Some(od) = l.on_disk {
                    self.on_disk.insert(key.to_owned(), od);
                    self.entries.insert(key.to_owned(), Entry { state: ResourceState::OnDisk, peaks: Some(l.peaks), last_used: now });
                    self.publish();
                }
            }
            // A cancelled load leaves the resource as if never asked
            Err(e) if e.is_cancelled() => {
                self.entries.remove(key);
            }
            Err(e) => {
                self.entries.insert(key.to_owned(), Entry { state: ResourceState::Failed(e), peaks: None, last_used: now });
            }
        }
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
                matches!(e.state, ResourceState::Ready | ResourceState::OnDisk) && now.since(e.last_used) >= limit && !in_use(k)
            })
            .map(|(k, _)| k.clone())
            .collect();
        let mut out = Vec::new();
        for k in &old {
            self.entries.remove(k);
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
        self.view = ResourceView { data: Arc::new(self.data.clone()), on_disk: Arc::new(self.on_disk.clone()) };
    }
}

/// H17: a file the user chose for a missing resource is that resource only if its hash is the key. Inside an
/// `Operation`
pub fn verify_located(path: &Path, key: &str, cancel: &Cancel, progress: &Progress) -> Result<bool> {
    check_key(key)?;
    Ok(crate::take::hash_file(path, cancel, progress)? == key)
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
