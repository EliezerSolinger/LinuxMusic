//! The native-crash note of the safe mode (U21): the watchdog catches a callback that stops; this catches one that
//! crashes. While a project is open, the handlers of `lm_sys_linux::crash` are armed with its `backup/suspeito.json`
//! and, after each install, one note per plugin node, formatted in advance (`{"node": .., "reason": "crash"}`, what
//! [`crate::watchdog::read_suspect`] reads). The render threads mark themselves through the session's thread hook,
//! so the handler blames the node of the thread that faulted, and nobody when the main thread does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lm_compile::{InstalledTable, NodeKey};
use lm_render::{RenderThreads, ThreadKind, Watch};

/// The render threads of the session: each one marked with its index in [`Watch`] while it runs
pub(crate) fn render_threads() -> RenderThreads {
    RenderThreads::new(Arc::new(|kind: ThreadKind, body: &mut dyn FnMut()| {
        #[cfg(unix)]
        let _mark = lm_sys_linux::crash::RenderThreadMark::enter(match kind {
            ThreadKind::Audio => 0,
            ThreadKind::Worker(i) => i + 1,
        });
        #[cfg(not(unix))]
        let _ = kind;
        body();
    }))
}

/// The armed note of the open project (nothing on platforms without the handlers)
#[derive(Debug, Default)]
pub(crate) struct CrashNote {
    folder: Option<PathBuf>,
    #[cfg(unix)]
    file: Option<lm_sys_linux::crash::CrashFile>,
}

impl CrashNote {
    /// Arms the note for the project in `folder`; `true` when it was armed just now (the notes are to be set), `false`
    /// when it already was, or could not be
    pub(crate) fn arm(&mut self, folder: &Path) -> bool {
        if self.folder.as_deref() == Some(folder) {
            return false;
        }
        self.disarm();
        self.folder = Some(folder.to_path_buf());
        #[cfg(unix)]
        {
            let backup = folder.join(crate::folder::BACKUP_DIR);
            if std::fs::create_dir_all(&backup).is_err() {
                return false;
            }
            self.file = lm_sys_linux::crash::arm(&backup, crate::watchdog::SUSPECT_FILE).ok();
            self.file.is_some()
        }
        #[cfg(not(unix))]
        true
    }

    pub(crate) fn disarm(&mut self) {
        self.folder = None;
        #[cfg(unix)]
        {
            self.file = None;
        }
    }

    /// The notes of a new program: one per plugin node, chosen by what each render thread is processing
    pub(crate) fn update(&self, table: &InstalledTable, watch: &Arc<Watch>) {
        #[cfg(unix)]
        if let Some(file) = &self.file {
            let notes = table
                .nodes
                .iter()
                .filter_map(|(k, i)| match k {
                    NodeKey::Instrument(id) | NodeKey::Insert(id) | NodeKey::NoteEffect(id) => {
                        let v = lm_state::Value::object([
                            ("node", lm_state::Value::text(id.to_string())),
                            ("reason", lm_state::Value::text("crash")),
                        ]);
                        lm_state::to_json(&v).ok().map(|text| (i.index, text.into_bytes()))
                    }
                    _ => None,
                })
                .collect();
            let watch = watch.clone();
            // Atomics only: this runs inside the signal handler
            let probe = Box::new(move |index: usize| watch.current_nodes().nth(index).flatten());
            file.set_notes(lm_sys_linux::crash::CrashNotes::new(probe, notes));
        }
        #[cfg(not(unix))]
        let _ = (table, watch);
    }
}
