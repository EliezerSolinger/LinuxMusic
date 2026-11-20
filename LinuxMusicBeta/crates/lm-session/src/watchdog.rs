//! The watchdog and the safe mode (U21): a plugin in an endless loop (or a real crash in a `lm-sys`) must not keep
//! the project from opening again.
//!
//! The renderer marks the node each render thread is in (`lm_render::Watch`). A thread of the session reads it every
//! quarter of a second; when the blocks stop advancing for [`STALL`] while the device should be running, the node
//! the callback is stuck in is written to `backup/suspeito.json` (synced at once: the process may be killed next).
//! Opening a project that has the file starts it in safe mode: that plugin bypassed (one undoable step), a notice,
//! and `status().safeMode` with the plugin, then the file is removed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lm_compile::{InstalledTable, NodeKey};
use lm_render::Watch;
use lm_state::{NodeId, Value, from_json, to_json};

use crate::folder::{BACKUP_DIR, DiskFaults, write_atomic};

/// The blocks stop for this long: the callback is stuck
pub const STALL: Duration = Duration::from_secs(2);
const CHECK: Duration = Duration::from_millis(250);
pub const SUSPECT_FILE: &str = "suspeito.json";

/// What the watchdog shares with the session (all immutable data or atomics: rule 3)
#[derive(Debug)]
pub struct Shared {
    pub table: Mutex<Arc<InstalledTable>>,
    pub folder: Mutex<PathBuf>,
    /// The device should be calling back now (false while it is stopped, restarted or switched)
    pub running: AtomicBool,
    pub watch: Mutex<Arc<Watch>>,
}

/// The watchdog's thread, stopped on drop
#[derive(Debug)]
pub struct Watchdog {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub shared: Arc<Shared>,
}

/// One suspect: a plugin node of the document and why
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suspect {
    pub node: NodeId,
    pub reason: String,
}

pub fn suspect_path(folder: &Path) -> PathBuf {
    folder.join(BACKUP_DIR).join(SUSPECT_FILE)
}

/// Writes the suspect file now (the watchdog; the tests)
pub fn write_suspect(folder: &Path, suspect: &Suspect) -> lm_base::Result<()> {
    let v = Value::object([
        ("node", Value::text(suspect.node.to_string())),
        ("reason", Value::text(suspect.reason.as_str())),
    ]);
    write_atomic(&suspect_path(folder), to_json(&v)?.as_bytes(), &DiskFaults::default())
}

/// The suspect a project folder holds, if any
pub fn read_suspect(folder: &Path) -> Option<Suspect> {
    let text = std::fs::read_to_string(suspect_path(folder)).ok()?;
    let v = from_json(&text).ok()?;
    Some(Suspect {
        node: NodeId::parse(v.get("node")?.as_text()?).ok()?,
        reason: v.get("reason").and_then(Value::as_text).unwrap_or("stuck").to_owned(),
    })
}

/// The document node behind a table slot (the plugin whose processor the thread is in)
fn owner_of(table: &InstalledTable, slot: u32) -> Option<NodeId> {
    table.nodes.iter().find(|(_, i)| i.index == slot).and_then(|(k, _)| match k {
        NodeKey::Instrument(id) | NodeKey::Insert(id) => Some(*id),
        _ => None,
    })
}

impl Watchdog {
    pub fn start(shared: Arc<Shared>) -> Watchdog {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let sh = shared.clone();
        let thread = std::thread::Builder::new()
            .name("lm-watchdog".into())
            .spawn(move || {
                let mut last = u64::MAX;
                let mut since = Instant::now();
                let mut reported = false;
                while !s.load(Ordering::Acquire) {
                    std::thread::park_timeout(CHECK);
                    let watch = sh.watch.lock().map(|w| w.clone()).ok();
                    let Some(watch) = watch else { continue };
                    let periods = watch.periods();
                    if periods != last || !sh.running.load(Ordering::Acquire) {
                        last = periods;
                        since = Instant::now();
                        reported = false;
                        continue;
                    }
                    if reported || since.elapsed() < STALL {
                        continue;
                    }
                    reported = true;
                    let table = sh.table.lock().map(|t| t.clone()).ok();
                    let folder = sh.folder.lock().map(|f| f.clone()).ok();
                    if let (Some(table), Some(folder)) = (table, folder) {
                        for slot in watch.current_nodes().flatten() {
                            if let Some(node) = owner_of(&table, slot) {
                                let _ = write_suspect(&folder, &Suspect { node, reason: "stuck".into() });
                                break;
                            }
                        }
                    }
                }
            })
            .ok();
        Watchdog { stop, thread, shared }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}
