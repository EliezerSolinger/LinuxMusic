//! The project's lock (H10): `projeto.lock` holds the process, its start time, the machine and the time it was taken.
//! The start time is what makes a reused process number harmless after a restart.
//!
//! - no lock, or a lock of a process of this machine that is gone (or whose number now belongs to a process that
//!   started at another time): the lock is taken (a stale one with a notice);
//! - a lock of a living process, or of another machine: the project opens read-only, with a notice.

use std::path::Path;

use lm_base::Result;
use lm_state::{Value, from_json, to_json};

use crate::folder::{DiskFaults, ProjectFolder, write_atomic};

/// Who holds a lock
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockOwner {
    pub host: String,
    pub pid: u32,
    /// The process's start time, as the system tells it
    pub started: String,
    /// When the lock was taken (seconds since 1970)
    pub since: u64,
}

impl LockOwner {
    /// This process
    pub fn me() -> Self {
        let pid = std::process::id();
        Self { host: host_name(), pid, started: process_start(pid).unwrap_or_default(), since: now_seconds() }
    }

    fn to_value(&self) -> Value {
        Value::object([
            ("host", Value::text(self.host.as_str())),
            ("pid", Value::int(i64::from(self.pid))),
            ("started", Value::text(self.started.as_str())),
            ("since", Value::int(self.since as i64)),
        ])
    }

    fn from_value(v: &Value) -> Option<Self> {
        Some(Self {
            host: v.get("host")?.as_text()?.to_owned(),
            pid: u32::try_from(v.get("pid")?.as_int()?).ok()?,
            started: v.get("started")?.as_text()?.to_owned(),
            since: u64::try_from(v.get("since")?.as_int()?).ok()?,
        })
    }

    /// As `status().project.lockedBy`
    pub fn status(&self) -> Value {
        Value::object([
            ("host", Value::text(self.host.as_str())),
            ("pid", Value::int(i64::from(self.pid))),
            ("since", Value::text(self.since.to_string())),
        ])
    }

    /// Whether this owner is alive: on this machine, its number names a process that started when it did
    fn alive(&self) -> bool {
        if self.host != host_name() {
            return true;
        }
        match process_start(self.pid) {
            Some(started) => self.started.is_empty() || started == self.started,
            None => false,
        }
    }
}

/// The machine's name
pub fn host_name() -> String {
    rustix::system::uname().nodename().to_string_lossy().into_owned()
}

pub(crate) fn now_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The start time of a process as `ps` gives it (`lstart`, the same text for the same process on Linux and macOS);
/// `None` when there is no such process
pub fn process_start(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps").args(["-o", "lstart=", "-p", &pid.to_string()]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// What happened when the lock was asked for
#[derive(Debug, PartialEq, Eq)]
pub enum Locking {
    /// Taken: the project is ours
    Taken,
    /// Taken over from a process that is gone (with a notice)
    TookStale(LockOwner),
    /// Held by someone else: read-only
    Held(LockOwner),
}

/// The lock of one open project, released on drop when it is ours
#[derive(Debug)]
pub struct ProjectLock {
    path: std::path::PathBuf,
    owner: Option<LockOwner>,
}

impl ProjectLock {
    /// Takes the lock of a folder, or says who holds it
    pub fn take(folder: &ProjectFolder, faults: &DiskFaults) -> Result<(ProjectLock, Locking)> {
        let path = folder.lock();
        let current = read_lock(&path);
        let me = LockOwner::me();
        let outcome = match current {
            Some(owner) if owner.pid == me.pid && owner.host == me.host && owner.started == me.started => {
                // Ours already (the same process opened it twice): read-only for the second one
                Locking::Held(owner)
            }
            Some(owner) if owner.alive() => Locking::Held(owner),
            Some(owner) => Locking::TookStale(owner),
            None => Locking::Taken,
        };
        if let Locking::Held(_) = outcome {
            return Ok((ProjectLock { path, owner: None }, outcome));
        }
        let bytes = to_json(&me.to_value())?;
        write_atomic(&path, bytes.as_bytes(), faults)?;
        Ok((ProjectLock { path, owner: Some(me) }, outcome))
    }

    /// No lock (a project not yet saved anywhere)
    pub fn none() -> ProjectLock {
        ProjectLock { path: std::path::PathBuf::new(), owner: None }
    }

    pub fn is_ours(&self) -> bool {
        self.owner.is_some()
    }
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        if self.owner.is_some() && read_lock(&self.path).as_ref() == self.owner.as_ref() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn read_lock(path: &Path) -> Option<LockOwner> {
    let text = std::fs::read_to_string(path).ok()?;
    LockOwner::from_value(&from_json(&text).ok()?)
}

/// Writes a lock as another owner would (the tests of H10)
pub fn write_foreign_lock(folder: &ProjectFolder, owner: &LockOwner) -> Result<()> {
    write_atomic(&folder.lock(), to_json(&owner.to_value())?.as_bytes(), &DiskFaults::default())
}
