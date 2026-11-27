//! The actions (I-19, D13): every command the program has, by a permanent id, as data. Menus, shortcuts, the
//! toolbar, the command palette and the external API are views of this table; the `CorePort::run_action` of the
//! interface comes here. An action that may pass 200 ms starts an operation and returns at once.

use std::path::PathBuf;

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, ROOT, Value};

use crate::session::{Session, tempo_map};

/// One action
#[derive(Clone, Copy)]
pub struct ActionSpec {
    /// Permanent (`"transport.play"`): shortcuts and the external API name it
    pub id: &'static str,
    pub label: &'static str,
    pub run: fn(&mut Session, &Value) -> Result<()>,
    pub enabled: fn(&Session) -> bool,
}

impl std::fmt::Debug for ActionSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id)
    }
}

fn params() -> Error {
    Error::new("action.params")
}

fn text(p: &Value, k: &str) -> Result<String> {
    p.get(k).and_then(Value::as_text).map(str::to_owned).ok_or_else(params)
}

fn always(_: &Session) -> bool {
    true
}

fn writable(s: &Session) -> bool {
    !s.project().read_only
}

/// The session's own actions
pub static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "transport.play",
        label: "action.transport.play",
        run: |s, _| {
            s.play();
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "transport.stop",
        label: "action.transport.stop",
        run: |s, _| {
            s.stop();
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "transport.toggle",
        label: "action.transport.toggle",
        run: |s, _| {
            if s.telemetry().position.playing {
                s.stop()
            } else {
                s.play()
            }
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "transport.record",
        label: "action.transport.record",
        // The parameter: the selected track's id as text, recorded if no track is armed
        run: |s, p| s.record(p.as_text().and_then(|t| NodeId::parse(t).ok())),
        enabled: writable,
    },
    ActionSpec {
        id: "transport.locate",
        label: "action.transport.locate",
        run: |s, p| {
            let ticks = p.as_int().ok_or_else(params)?;
            let map = tempo_map(s.document().root())?;
            s.locate(map.sample_at(ticks.max(0)));
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "transport.panic",
        label: "action.transport.panic",
        run: |s, _| {
            s.panic();
            Ok(())
        },
        enabled: always,
    },
    ActionSpec { id: "project.save", label: "action.project.save", run: |s, _| s.save(), enabled: writable },
    ActionSpec {
        id: "project.save_as",
        label: "action.project.save_as",
        run: |s, p| s.save_as(PathBuf::from(text(p, "folder")?)),
        enabled: always,
    },
    ActionSpec {
        id: "project.open",
        label: "action.project.open",
        run: |s, p| {
            let _ = s.open(PathBuf::from(text(p, "folder")?));
            Ok(())
        },
        enabled: always,
    },
    ActionSpec { id: "project.close", label: "action.project.close", run: |s, _| s.close_project(), enabled: always },
    ActionSpec {
        id: "project.new",
        label: "action.project.new",
        // `{folder, rate?, template?}`: a template is a name in the templates folder
        run: |s, p| {
            let rate =
                p.get("rate").and_then(Value::as_int).map(|r| u32::try_from(r).map_err(|_| params())).transpose()?;
            s.create_with(
                PathBuf::from(text(p, "folder")?),
                rate,
                p.get("template").and_then(Value::as_text).map(str::to_owned),
            )
        },
        enabled: always,
    },
    ActionSpec {
        id: "project.export",
        label: "action.project.export",
        // The export screen's request (lm_port::trees); `{file}` alone: the whole song as a float WAV
        run: |s, p| {
            if let Some(file) = p.get("file").and_then(Value::as_text) {
                let _ = s.export(crate::export::ExportTarget::wav(PathBuf::from(file)));
            } else {
                let _ = s.export_request(crate::export::ExportRequest::from_value(p)?);
            }
            Ok(())
        },
        enabled: always,
    },
    // Records a track's output (inserts, volume and pan) onto a new audio track (F-REC-011)
    ActionSpec {
        id: "track.resample",
        label: "action.track.resample",
        run: |s, p| s.resample_track(NodeId::parse(&text(p, "track")?)?),
        enabled: writable,
    },
    // MIDI files (agent D, midifile.rs): { path, at: tick } and { path, track: "t:N" }
    ActionSpec {
        id: "midi.import",
        label: "action.midi.import",
        run: |s, p| {
            let at = p.get("at").and_then(Value::as_int).unwrap_or(0).max(0);
            let _ = s.midi_import(PathBuf::from(text(p, "path")?), at);
            Ok(())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "midi.export",
        label: "action.midi.export",
        run: |s, p| {
            let only = p.get("track").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
            let item = p.get("item").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
            let _ = s.midi_export_of(PathBuf::from(text(p, "path")?), only, item);
            Ok(())
        },
        enabled: always,
    },
    ActionSpec { id: "takes.recover", label: "action.takes.recover", run: |s, _| s.recover_takes(), enabled: writable },
    ActionSpec {
        id: "resources.import",
        label: "action.resources.import",
        run: |s, p| {
            let _ = s.import(PathBuf::from(text(p, "path")?));
            Ok(())
        },
        enabled: writable,
    },
    // A file replaced on the disk: every resource is asked for again
    ActionSpec {
        id: "take.reload",
        label: "action.take.reload",
        run: |s, _| {
            s.reload_resources();
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "items.rerender",
        label: "action.items.rerender",
        run: |s, _| {
            s.reload_resources();
            Ok(())
        },
        enabled: always,
    },
    // Third-party plugins are F2 (H15): nothing to scan among the internal ones
    ActionSpec { id: "plugins.rescan", label: "action.plugins.rescan", run: |_, _| Ok(()), enabled: always },
    ActionSpec {
        id: "edit.copy",
        label: "action.edit.copy",
        run: |s, p| {
            let id = p.as_text().and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)?;
            let clip = s.copy(id)?;
            s.clipboard = Some(clip);
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "edit.paste",
        label: "action.edit.paste",
        run: |s, p| {
            let clip = s.clipboard.clone().ok_or_else(|| Error::new("action.clipboard.empty"))?;
            let parent = p.get("parent").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).unwrap_or(ROOT);
            let list = p.get("list").and_then(Value::as_text).unwrap_or("tracks").to_owned();
            let index = p.get("index").and_then(Value::as_int).map_or(usize::MAX, |i| i.max(0) as usize);
            let len = s.document().find(parent).and_then(|n| n.get(&list).map(Value::len)).unwrap_or(0);
            s.paste(&clip, parent, &list, index.min(len))?;
            Ok(())
        },
        enabled: writable,
    },
    // The master's loudness meter starts again: integrated value, range and true-peak hold (F-MIX, BS.1770)
    ActionSpec {
        id: "loudness.reset",
        label: "action.loudness.reset",
        run: |_, _| {
            crate::rack::reset_loudness();
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "edit.undo",
        label: "action.edit.undo",
        run: |s, _| s.document_mut().undo().map(|_| ()),
        enabled: |s| s.document().can_undo(),
    },
    ActionSpec {
        id: "edit.redo",
        label: "action.edit.redo",
        run: |s, _| s.document_mut().redo().map(|_| ()),
        enabled: |s| s.document().can_redo(),
    },
];

/// Every action: the session's, then the ones other modules add (I-21's, when they exist)
pub fn all() -> impl Iterator<Item = &'static ActionSpec> {
    ACTIONS
        .iter()
        .chain(crate::arrange::ACTIONS.iter())
        .chain(crate::clips::ACTIONS.iter())
        .chain(crate::session::midi::actions::ACTIONS.iter())
        .chain(crate::session::external::ACTIONS.iter())
        .chain(crate::session::midi::actions::ACTIONS.iter())
        .chain(crate::session::external::ACTIONS.iter())
        .chain(crate::plugin_actions::ACTIONS.iter())
        .chain(crate::tuning_actions::ACTIONS.iter())
        .chain(crate::sampler_presets::ACTIONS.iter())
        .chain(crate::modulators::ACTIONS.iter())
        .chain(crate::field_load::ACTIONS.iter())
        .chain(crate::session::core_actions::ACTIONS.iter())
}

/// The action with this id
pub fn find(id: &str) -> Option<&'static ActionSpec> {
    all().find(|a| a.id == id)
}

impl Session {
    /// Runs an action by id (`action.unknown`, `action.disabled`)
    pub fn run_action(&mut self, id: &str, params: &Value) -> Result<()> {
        let Some(a) = find(id) else {
            return Err(Error::new("action.unknown").arg("action", id));
        };
        if !(a.enabled)(self) {
            return Err(Error::new("action.disabled").arg("action", id));
        }
        // A browser entry (`entry`) becomes its file, whoever asks: the local port, a remote client, the runner
        let resolved = self.browser.resolve(id, params.clone())?;
        (a.run)(self, &resolved)
    }

    /// Whether an action can run now (menus grey it out)
    pub fn action_enabled(&self, id: &str) -> bool {
        find(id).is_some_and(|a| (a.enabled)(self))
    }

    /// The label of an action
    pub fn action_label(id: &str) -> Option<Label> {
        find(id).map(|a| Label::new(a.label))
    }
}
