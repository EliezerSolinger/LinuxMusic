//! The journal and the autosave (I-4, R16, T11, H3, U6).
//!
//! `backup/diario.jsonl` holds the document's changes as patches, one line per step:
//!
//! ```text
//! {"base": "<BLAKE3 of projeto.json>", "seq": 0}
//! {"seq": 1, "patch": [{"op": "set", ...}]}
//! {"incomplete": true, "seq": 7}        a change could not be written: the autosave that follows is the truth
//! ```
//!
//! Opening a project replays the lines whose base is the saved file's hash; saving starts the journal over. The writes
//! go to the session's `Disk` thread by a ring (the recording has a thread of its own, U6). If the ring fills or the
//! disk refuses a line, the journal is marked incomplete and the session makes a full autosave next: nothing is lost
//! in silence. The autosave (`backup/autosave.json`) holds the whole tree with the base and the journal number it
//! includes, so recovery takes it and replays only the lines after it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use lm_base::ring::{Consumer, Producer, ring};
use lm_base::{Error, Result, Role, set_role};
use lm_doc::JournalPatch;
use lm_state::{Value, from_json, to_json, to_json_compact};

use crate::folder::{DiskFaults, ProjectFolder, append_line, open_append, write_atomic};

/// Lines in flight to the disk thread: the pump runs every 10 ms and makes at most one line per step; 10 s of a stopped
/// disk thread at 100 lines per second (C4)
pub const JOURNAL_RING: usize = 1024;
/// Problems reported back to the main thread; more are counted
const EVENTS_RING: usize = 64;

/// The BLAKE3 of a saved file's bytes: the journal's base
pub fn base_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

enum Job {
    /// Starts the journal over (after a save): truncates it and writes the header
    Reset {
        base: String,
    },
    Line {
        seq: u64,
        text: String,
    },
    /// The whole tree, written to the autosave
    Autosave {
        base: String,
        seq: u64,
        tree: Value,
    },
    /// Marks the journal incomplete at `seq`
    Incomplete {
        seq: u64,
    },
    /// Answers when everything before it is on the disk
    Sync {
        done: Arc<AtomicBool>,
    },
    /// Saves the project (H3, H11, U6) and starts the journal over on the new file. In order with the lines: the
    /// lines sent before it are in the file saved, those after go to the new journal
    Save {
        /// Versions of the previous file kept in `backup/` (0: none)
        keep: usize,
        tree: Value,
        interface: Option<Value>,
        migrated_from: Option<u32>,
        id: u64,
    },
}

/// What the disk thread reports
#[derive(Debug)]
pub enum JournalEvent {
    /// A journal line could not be written: the journal is incomplete from `seq`
    LineFailed {
        seq: u64,
        error: Error,
    },
    AutosaveFailed(Error),
    /// The copy of the previous file in `backup/` could not be made or pruned: the save itself went on
    BackupFailed(Error),
    AutosaveDone {
        seq: u64,
    },
    /// The project is saved: `base` is the new file's hash
    Saved {
        id: u64,
        base: String,
    },
    /// The save failed: the old file is intact, the journal goes on on it
    SaveFailed {
        id: u64,
        error: Error,
    },
}

/// The main thread's end of the journal
pub struct Journal {
    jobs: Producer<Job>,
    events: Consumer<JournalEvent>,
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    base: String,
    seq: u64,
    /// A line was refused (the ring was full, or the disk): an autosave must follow
    incomplete: bool,
    lost_events: Arc<AtomicU64>,
    enabled: bool,
}

impl std::fmt::Debug for Journal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Journal").field("seq", &self.seq).field("incomplete", &self.incomplete).finish()
    }
}

struct DiskSide {
    folder: ProjectFolder,
    faults: Arc<DiskFaults>,
    file: Option<std::fs::File>,
    events: Producer<JournalEvent>,
    lost: Arc<AtomicU64>,
}

impl DiskSide {
    fn report(&mut self, e: JournalEvent) {
        if self.events.push(e).is_err() {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn line(&mut self, text: &str, seq: u64) {
        let path = self.folder.journal();
        let result = (|| -> Result<()> {
            if self.file.is_none() {
                self.file = Some(open_append(&path)?);
            }
            let file = self.file.as_mut().ok_or_else(|| Error::new("session.journal.closed"))?;
            append_line(file, text, &path, &self.faults)
        })();
        if let Err(error) = result {
            self.report(JournalEvent::LineFailed { seq, error });
        }
    }

    fn run(&mut self, job: Job) {
        match job {
            Job::Reset { base } => {
                self.file = None;
                let header = Value::object([("base", Value::text(base)), ("seq", Value::int(0))]);
                let text = to_json_compact(&header).unwrap_or_default();
                let path = self.folder.journal();
                match write_atomic(&path, format!("{text}\n").as_bytes(), &self.faults) {
                    Ok(()) => {}
                    Err(error) => self.report(JournalEvent::LineFailed { seq: 0, error }),
                }
            }
            Job::Line { seq, text } => self.line(&text, seq),
            Job::Incomplete { seq } => {
                let v = Value::object([("incomplete", Value::bool(true)), ("seq", Value::int(seq as i64))]);
                let text = to_json_compact(&v).unwrap_or_default();
                self.line(&text, seq);
            }
            Job::Autosave { base, seq, tree } => {
                let envelope =
                    Value::object([("base", Value::text(base)), ("seq", Value::int(seq as i64)), ("saved", tree)]);
                let result = to_json(&envelope)
                    .and_then(|text| write_atomic(&self.folder.autosave(), text.as_bytes(), &self.faults));
                match result {
                    Ok(()) => self.report(JournalEvent::AutosaveDone { seq }),
                    Err(e) => self.report(JournalEvent::AutosaveFailed(e)),
                }
            }
            Job::Sync { done } => done.store(true, Ordering::Release),
            Job::Save { keep, tree, interface, migrated_from, id } => {
                if keep > 0
                    && let Err(error) = self.keep_previous(keep)
                {
                    self.report(JournalEvent::BackupFailed(error));
                }
                match self.save(&tree, interface.as_ref(), migrated_from) {
                    Ok(base) => self.report(JournalEvent::Saved { id, base }),
                    Err(error) => self.report(JournalEvent::SaveFailed { id, error }),
                }
            }
        }
    }
}

/// The name of a kept version in `backup/`: `projeto-0001.json`, numbered up (the order is the age; no clock)
const VERSION_PREFIX: &str = "projeto-";

/// The number in a kept version's file name
fn version_number(name: &str) -> Option<u64> {
    name.strip_prefix(VERSION_PREFIX)?.strip_suffix(".json")?.parse().ok()
}

impl DiskSide {
    /// Copies the file about to be replaced to `backup/projeto-NNNN.json` and removes the oldest copies beyond
    /// `keep` (F-PRJ-010). A project never saved has nothing to keep
    fn keep_previous(&mut self, keep: usize) -> Result<()> {
        let project = self.folder.project();
        if !project.exists() {
            return Ok(());
        }
        let dir = self.folder.backup();
        let mut numbers: Vec<u64> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| version_number(&e.file_name().to_string_lossy()))
            .collect();
        numbers.sort_unstable();
        let next = numbers.last().map_or(1, |n| n + 1);
        let name = |n: u64| dir.join(format!("{VERSION_PREFIX}{n:04}.json"));
        let size = std::fs::metadata(&project).map(|m| m.len()).unwrap_or(0);
        self.faults.check(size)?;
        std::fs::create_dir_all(&dir).map_err(|e| crate::folder::io_error("session.folder.create", &dir, &e))?;
        let (dest, tmp) = (name(next), dir.join(format!("{VERSION_PREFIX}{next:04}.tmp")));
        crate::folder::copy_via_tmp(&project, &tmp, &dest, "session.backup.copy")?;
        numbers.push(next);
        // the oldest beyond the limit go; one that cannot be removed is reported with the others tried
        let excess = numbers.len().saturating_sub(keep);
        let mut failed = None;
        for n in numbers.into_iter().take(excess) {
            if let Err(e) = std::fs::remove_file(name(n)) {
                failed.get_or_insert_with(|| crate::folder::io_error("session.backup.prune", &name(n), &e));
            }
        }
        failed.map_or(Ok(()), Err)
    }

    fn save(&mut self, tree: &Value, interface: Option<&Value>, migrated_from: Option<u32>) -> Result<String> {
        let project = self.folder.project();
        // H11: the original of a migrated project is kept before it is first written over
        if let Some(n) = migrated_from {
            let bak = self.folder.migrated_backup(n);
            if !bak.exists()
                && let Ok(old) = std::fs::read(&project)
            {
                write_atomic(&bak, &old, &self.faults)?;
            }
        }
        let text = to_json(tree)?;
        write_atomic(&project, text.as_bytes(), &self.faults)?;
        if let Some(ui) = interface {
            // The interface's file is not the project: a failure there is reported, the save stands
            let _ = to_json(ui).and_then(|t| write_atomic(&self.folder.interface(), t.as_bytes(), &self.faults));
        }
        let base = base_hash(text.as_bytes());
        self.run(Job::Reset { base: base.clone() });
        Ok(base)
    }
}

impl Journal {
    /// The journal of a saved project whose `projeto.json` hashes to `base`, numbering from `seq`; `reset` starts
    /// the file over with that base. Starts the disk thread
    pub fn start(
        folder: &ProjectFolder,
        faults: Arc<DiskFaults>,
        base: String,
        seq: u64,
        reset: bool,
    ) -> Result<Journal> {
        let (jobs, mut rx) = ring::<Job>(JOURNAL_RING);
        let (events_tx, events) = ring::<JournalEvent>(EVENTS_RING);
        let stop = Arc::new(AtomicBool::new(false));
        let lost_events = Arc::new(AtomicU64::new(0));
        let mut side =
            DiskSide { folder: folder.clone(), faults, file: None, events: events_tx, lost: lost_events.clone() };
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("lm-disk".into())
            .spawn(move || {
                set_role(Role::Disk);
                loop {
                    let mut worked = false;
                    while let Ok(job) = rx.pop() {
                        side.run(job);
                        worked = true;
                    }
                    if thread_stop.load(Ordering::Acquire) && rx.is_empty() {
                        break;
                    }
                    if !worked {
                        std::thread::park_timeout(Duration::from_millis(20));
                    }
                }
            })
            // Without the disk thread nothing is written or saved: the project does not open (M4)
            .map_err(|e| Error::new("session.thread.failed").arg("name", "lm-disk").arg("cause", e.to_string()))?;
        let thread = Some(thread);
        let mut journal = Journal {
            jobs,
            events,
            thread,
            stop,
            base: base.clone(),
            seq,
            incomplete: false,
            lost_events,
            enabled: true,
        };
        if reset {
            journal.send(Job::Reset { base });
        }
        Ok(journal)
    }

    /// A journal that writes nothing (a read-only project, H10, H11)
    pub fn disabled(folder: &ProjectFolder) -> Result<Journal> {
        let mut j = Journal::start(folder, DiskFaults::new(), String::new(), 0, false)?;
        j.enabled = false;
        Ok(j)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn wake(&self) {
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
    }

    fn send(&mut self, job: Job) -> bool {
        let ok = self.jobs.push(job).is_ok();
        self.wake();
        ok
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Whether a line was lost since the last autosave
    pub fn is_incomplete(&self) -> bool {
        self.incomplete
    }

    /// Writes one step's patch
    pub fn write(&mut self, patch: &JournalPatch) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        self.seq += 1;
        let seq = self.seq;
        let line = Value::object([("seq", Value::int(seq as i64)), ("patch", patch.to_value()?)]);
        let text = to_json_compact(&line)?;
        if !self.send(Job::Line { seq, text }) {
            self.mark_incomplete();
        }
        Ok(())
    }

    fn mark_incomplete(&mut self) {
        if !self.incomplete {
            self.incomplete = true;
            let seq = self.seq;
            let _ = self.send(Job::Incomplete { seq });
        }
    }

    /// A full autosave of `tree` (the document's saved tree) at the current number
    pub fn autosave(&mut self, tree: Value) -> bool {
        if !self.enabled {
            return false;
        }
        let job = Job::Autosave { base: self.base.clone(), seq: self.seq, tree };
        self.send(job)
    }

    /// Saves the project on the disk thread, in order with the journal; the answer comes as `Saved` or `SaveFailed`
    /// with `id`. `false` when the disk thread's ring is full (try again)
    pub fn save(
        &mut self,
        tree: Value,
        interface: Option<Value>,
        migrated_from: Option<u32>,
        keep: usize,
        id: u64,
    ) -> bool {
        self.send(Job::Save { keep, tree, interface, migrated_from, id })
    }

    /// After a save: the journal starts over on the new file
    pub fn reset(&mut self, base: String) {
        if !self.enabled {
            return;
        }
        self.base = base.clone();
        self.seq = 0;
        self.incomplete = false;
        if !self.send(Job::Reset { base }) {
            self.incomplete = true;
        }
    }

    /// What the disk thread reported. A failed line marks the journal incomplete
    pub fn poll(&mut self, mut f: impl FnMut(JournalEvent)) {
        while let Ok(e) = self.events.pop() {
            match &e {
                JournalEvent::LineFailed { .. } => self.incomplete = true,
                JournalEvent::AutosaveDone { seq } if *seq >= self.seq => self.incomplete = false,
                JournalEvent::Saved { base, .. } => {
                    self.base = base.clone();
                    self.incomplete = false;
                }
                _ => {}
            }
            f(e);
        }
    }

    /// A flag that turns true once everything sent before it is on the disk (without waiting: polled by the pump)
    pub fn marker(&mut self) -> Option<Arc<AtomicBool>> {
        let done = Arc::new(AtomicBool::new(!self.enabled));
        if !self.enabled {
            return Some(done);
        }
        self.send(Job::Sync { done: done.clone() }).then_some(done)
    }

    /// Waits until everything sent is on the disk (closing; the tests of a crash), at most `timeout`
    pub fn sync(&mut self, timeout: Duration) -> bool {
        let done = Arc::new(AtomicBool::new(false));
        if !self.send(Job::Sync { done: done.clone() }) {
            return false;
        }
        let start = std::time::Instant::now();
        while !done.load(Ordering::Acquire) {
            if start.elapsed() > timeout {
                return false;
            }
            self.wake();
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }

    /// Events lost because the main thread did not read them in time
    pub fn lost_events(&self) -> u64 {
        self.lost_events.load(Ordering::Relaxed)
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.wake();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A journal read back
#[derive(Debug, Default)]
pub struct ReadJournal {
    pub base: Option<String>,
    /// The patches by number, in order
    pub patches: Vec<(u64, JournalPatch)>,
    /// A line could not be written (the autosave after it is the truth)
    pub incomplete: bool,
    /// Broken lines at the end (a crash in the middle of a write): ignored
    pub broken_tail: usize,
}

/// Reads a journal file; a missing file is an empty journal
pub fn read_journal(path: &Path) -> Result<ReadJournal> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ReadJournal::default()),
        Err(e) => return Err(crate::folder::io_error("session.journal.read", path, &e)),
    };
    let mut out = ReadJournal::default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    for (i, line) in lines.iter().enumerate() {
        let parsed = from_json(line).ok();
        let Some(v) = parsed else {
            // Only the last line may be broken (the crash); a broken line in the middle ends the journal there
            out.broken_tail = lines.len() - i;
            break;
        };
        if let Some(base) = v.get("base").and_then(Value::as_text) {
            out.base = Some(base.to_owned());
            continue;
        }
        if v.get("incomplete").and_then(Value::as_bool) == Some(true) {
            out.incomplete = true;
            continue;
        }
        let seq = v.get("seq").and_then(Value::as_int).unwrap_or(0) as u64;
        match v.get("patch").map(JournalPatch::from_value) {
            Some(Ok(p)) => out.patches.push((seq, p)),
            _ => {
                out.broken_tail = lines.len() - i;
                break;
            }
        }
    }
    Ok(out)
}

/// An autosave read back
#[derive(Debug)]
pub struct ReadAutosave {
    pub base: String,
    pub seq: u64,
    pub saved: Value,
}

pub fn read_autosave(path: &Path) -> Option<ReadAutosave> {
    let text = std::fs::read_to_string(path).ok()?;
    let v = from_json(&text).ok()?;
    Some(ReadAutosave {
        base: v.get("base")?.as_text()?.to_owned(),
        seq: u64::try_from(v.get("seq")?.as_int()?).ok()?,
        saved: v.get("saved")?.clone(),
    })
}

/// Moves a journal that does not belong to the saved file aside, keeping it (never deleted)
pub fn set_aside(path: &Path) -> Result<PathBuf> {
    let aside = path.with_extension(format!("jsonl.{}.orphan", crate::lock::now_seconds()));
    std::fs::rename(path, &aside).map_err(|e| crate::folder::io_error("session.journal.aside", path, &e))?;
    Ok(aside)
}
