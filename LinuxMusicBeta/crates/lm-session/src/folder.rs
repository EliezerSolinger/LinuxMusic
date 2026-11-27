//! The project folder (arquitetura-nova.md, piece 2; I-4, H3, H10, H11, U6):
//!
//! ```text
//! <folder>/projeto.json            the project (the document's saved tree)
//! <folder>/interface.json          the interface's document (I-3.8): zoom, scroll, open windows
//! <folder>/projeto.lock            the lock (H10)
//! <folder>/projeto.json.v<N>.bak   the original of a project migrated from version N (H11)
//! <folder>/audio/                  the resources, by BLAKE3 hash (C9, C10)
//! <folder>/backup/diario.jsonl     the journal (I-4)
//! <folder>/backup/autosave.json    the last autosave
//! <folder>/backup/projeto-NNNN.json the previous file at each save, the last `backup.kept` (F-PRJ-010)
//! ```
//!
//! Every file written whole is written to a `.tmp`, synced, renamed over the old one, and the folder synced (U6): a
//! full disk or a crash leaves the old file intact (H3). The disk can be made to fail on purpose ([`DiskFaults`]) for
//! the tests of H3.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use lm_base::{Error, Result};

pub const PROJECT_FILE: &str = "projeto.json";
pub const INTERFACE_FILE: &str = "interface.json";
pub const LOCK_FILE: &str = "projeto.lock";
pub const AUDIO_DIR: &str = "audio";
pub const BACKUP_DIR: &str = "backup";
pub const JOURNAL_FILE: &str = "diario.jsonl";
pub const AUTOSAVE_FILE: &str = "autosave.json";

/// The files of a project folder
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFolder {
    root: PathBuf,
}

impl ProjectFolder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn project(&self) -> PathBuf {
        self.root.join(PROJECT_FILE)
    }

    pub fn interface(&self) -> PathBuf {
        self.root.join(INTERFACE_FILE)
    }

    pub fn lock(&self) -> PathBuf {
        self.root.join(LOCK_FILE)
    }

    pub fn audio(&self) -> PathBuf {
        self.root.join(AUDIO_DIR)
    }

    pub fn backup(&self) -> PathBuf {
        self.root.join(BACKUP_DIR)
    }

    pub fn journal(&self) -> PathBuf {
        self.backup().join(JOURNAL_FILE)
    }

    pub fn autosave(&self) -> PathBuf {
        self.backup().join(AUTOSAVE_FILE)
    }

    /// The original of a project migrated from version `n` (H11)
    pub fn migrated_backup(&self, n: u32) -> PathBuf {
        self.root.join(format!("{PROJECT_FILE}.v{n}.bak"))
    }

    /// The name shown for the project: the folder's name
    pub fn name(&self) -> String {
        self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Makes the folder and its subfolders
    pub fn create(&self, faults: &DiskFaults) -> Result<()> {
        faults.check(0)?;
        for dir in [self.root.clone(), self.audio(), self.backup()] {
            fs::create_dir_all(&dir).map_err(|e| io_error("session.folder.create", &dir, &e))?;
        }
        Ok(())
    }
}

/// An I/O error as a value with its key, the path and the system's reason
pub fn io_error(key: &'static str, path: &Path, e: &std::io::Error) -> Error {
    let full = e.raw_os_error() == Some(28) || e.kind() == std::io::ErrorKind::StorageFull;
    let key = if full { "session.disk.full" } else { key };
    Error::new(key).arg("path", path.display().to_string()).arg("reason", e.to_string())
}

/// Failures of the disk made on purpose (the tests of H3): writes fail as with a full disk
#[derive(Debug, Default)]
pub struct DiskFaults {
    full: AtomicBool,
    /// Bytes that may still be written before the disk is full (when `full_after` was set)
    budget: AtomicU64,
    budget_on: AtomicBool,
}

impl DiskFaults {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The disk is full from now on (or not)
    pub fn set_full(&self, full: bool) {
        self.full.store(full, Ordering::SeqCst);
        self.budget_on.store(false, Ordering::SeqCst);
    }

    /// The disk fills up after `bytes` more bytes
    pub fn full_after(&self, bytes: u64) {
        self.budget.store(bytes, Ordering::SeqCst);
        self.budget_on.store(true, Ordering::SeqCst);
        self.full.store(false, Ordering::SeqCst);
    }

    pub fn is_full(&self) -> bool {
        self.full.load(Ordering::SeqCst)
    }

    /// Whether `bytes` more may be written; `session.disk.full` if not
    pub fn check(&self, bytes: u64) -> Result<()> {
        if self.full.load(Ordering::SeqCst) {
            return Err(Error::new("session.disk.full").arg("needed", bytes as i64));
        }
        if self.budget_on.load(Ordering::SeqCst) {
            let left = self.budget.load(Ordering::SeqCst);
            if bytes > left {
                self.full.store(true, Ordering::SeqCst);
                return Err(Error::new("session.disk.full").arg("needed", bytes as i64));
            }
            self.budget.store(left - bytes, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// Syncs a folder (after a rename, so the new name survives a crash, U6)
pub fn sync_dir(dir: &Path) -> Result<()> {
    match File::open(dir) {
        Ok(f) => f.sync_all().or_else(|e| {
            // Some systems refuse to sync a folder handle: the rename is still atomic there
            if e.kind() == std::io::ErrorKind::InvalidInput || e.raw_os_error() == Some(9) { Ok(()) } else { Err(e) }
        }),
        Err(e) => Err(e),
    }
    .map_err(|e| io_error("session.folder.sync", dir, &e))
}

/// Writes a file whole and atomically: `.tmp`, sync, rename, sync the folder. On any failure the old file is
/// intact and the `.tmp` is removed (H3)
pub fn write_atomic(path: &Path, bytes: &[u8], faults: &DiskFaults) -> Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    let result = (|| -> Result<()> {
        faults.check(bytes.len() as u64)?;
        let mut f = File::create(&tmp).map_err(|e| io_error("session.file.write", &tmp, &e))?;
        f.write_all(bytes).map_err(|e| io_error("session.file.write", &tmp, &e))?;
        f.sync_all().map_err(|e| io_error("session.file.write", &tmp, &e))?;
        drop(f);
        fs::rename(&tmp, path).map_err(|e| io_error("session.file.rename", path, &e))?;
        if let Some(dir) = path.parent() {
            sync_dir(dir)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Copies `from` to `dest` through `tmp`: on any failure the `.tmp` is removed, never left behind
pub fn copy_via_tmp(from: &Path, tmp: &Path, dest: &Path, key: &'static str) -> Result<()> {
    let result = fs::copy(from, tmp)
        .map_err(|e| io_error(key, tmp, &e))
        .and_then(|_| fs::rename(tmp, dest).map_err(|e| io_error(key, dest, &e)));
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

/// Appends a line and syncs it (the journal)
pub fn append_line(file: &mut File, line: &str, path: &Path, faults: &DiskFaults) -> Result<()> {
    faults.check(line.len() as u64 + 1)?;
    file.write_all(line.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_data())
        .map_err(|e| io_error("session.journal.write", path, &e))
}

/// Opens a file for appending, made if absent
pub fn open_append(path: &Path) -> Result<File> {
    OpenOptions::new().create(true).append(true).open(path).map_err(|e| io_error("session.file.open", path, &e))
}

#[cfg(test)]
mod copy_tests {
    use super::*;

    #[test]
    fn a_failed_copy_leaves_no_tmp() {
        let dir = std::env::temp_dir().join(format!("lm-copy-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.wav");
        fs::write(&src, b"x").unwrap();
        let tmp = dir.join("a.tmp");
        // The destination is a non-empty directory: the rename fails after the copy made the tmp
        let dest = dir.join("d");
        fs::create_dir_all(dest.join("inner")).unwrap();
        assert!(copy_via_tmp(&src, &tmp, &dest, "session.test").is_err());
        assert!(!tmp.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
