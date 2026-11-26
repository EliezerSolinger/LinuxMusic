//! Actions that belong to one internal plugin (I-19): what the legacy plugin's window did to its own state, as an
//! action of the core, so that a panel, the command palette and a remote client all run the same edit, one undo
//! step each.
//!
//! - `opulenz.import_sbi {node, path | bytes}` (`node`: the track whose instrument it is, or the plugin's node): a
//!   SoundBlaster instrument file (`.sbi`) into an OpulenZ's knobs, as
//!   the legacy `loadFile` read it (`lm_plugins_opulenz::sbi`). `path` is a file on the machine of the session (the
//!   server, in client mode); `bytes` (a list of 0–255) is the file itself, for a client that read it where it
//!   runs. The legacy only imported `.sbi` files: there is no export.

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, PathStep, Value};

use crate::actions::ActionSpec;
use crate::session::Session;

fn params() -> Error {
    Error::new("action.params")
}

/// The largest `.sbi` file read (they are 52 bytes; anything bigger is not one)
const SBI_MAX: u64 = 4096;

/// The plugin actions (chained into `actions::all`)
pub(crate) static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "opulenz.import_sbi",
        label: "action.opulenz.import_sbi",
        run: import_sbi,
        enabled: |s| !s.project().read_only,
    },
    ActionSpec {
        id: "slicert.analyze",
        label: "action.slicert.analyze",
        run: |s, p| {
            let node = p.get("node").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)?;
            s.slicert_analyze(node).map(|_| ())
        },
        enabled: |s| !s.project().read_only,
    },
];

fn import_sbi(s: &mut Session, p: &Value) -> Result<()> {
    let node = p.get("node").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)?;
    let bytes = if let Some(list) = p.get("bytes") {
        list.items()
            .map(|v| v.as_int().and_then(|b| u8::try_from(b).ok()).ok_or_else(params))
            .collect::<Result<Vec<u8>>>()?
    } else {
        let path = p.get("path").and_then(Value::as_text).ok_or_else(params)?;
        read_small(std::path::Path::new(path))?
    };
    let patch = lm_plugins_opulenz::sbi::read(&bytes)?;
    let found =
        s.document().find(node).ok_or_else(|| Error::new("action.node.missing").arg("node", node.to_string()))?;
    // (the node of a track, whose instrument it is, or of the plugin itself)
    let (plugin, path) = match found.get("instrument") {
        Some(instrument) => (instrument.clone(), vec![PathStep::key("instrument"), PathStep::key("state")]),
        None => (found, vec![PathStep::key("state")]),
    };
    if plugin.get("plugin").and_then(Value::as_text) != Some(lm_plugins_opulenz::ID) {
        return Err(Error::new("opulenz.sbi.not_opulenz").arg("node", node.to_string()));
    }
    let state = plugin.get("state").cloned().unwrap_or_else(Value::empty_object);
    let state = patch.apply(&state)?;
    s.document_mut()
        .transact(Label::new("opulenz.import_sbi").arg("name", patch.name.as_str()), |e| e.set(node, &path, state))?;
    Ok(())
}

/// A small file, whole (an `.sbi` is 52 bytes: no operation needed)
fn read_small(path: &std::path::Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| {
        Error::new("opulenz.sbi.read").arg("path", path.display().to_string()).arg("why", e.to_string())
    })?;
    let mut bytes = Vec::new();
    file.take(SBI_MAX).read_to_end(&mut bytes).map_err(|e| {
        Error::new("opulenz.sbi.read").arg("path", path.display().to_string()).arg("why", e.to_string())
    })?;
    Ok(bytes)
}

// ---- SlicerT: the analysis of a sample ----

/// Where a SlicerT's state is: the path to it from `node` (a track, whose instrument it is, or the plugin's own node)
/// and the state itself; `None` when the node is not a SlicerT
fn slicert_state(s: &Session, node: NodeId) -> Option<(Vec<PathStep>, Value)> {
    let found = s.document().find(node)?;
    let (plugin, path) = match found.get("instrument") {
        Some(instrument) => (instrument.clone(), vec![PathStep::key("instrument"), PathStep::key("state")]),
        None => (found, vec![PathStep::key("state")]),
    };
    if plugin.get("plugin").and_then(Value::as_text) != Some(lm_plugins_instruments::slicert::ID) {
        return None;
    }
    let state = plugin.get("state").cloned().unwrap_or_else(Value::empty_object);
    Some((path, state))
}

/// A knob of a state that may be partial (what the tree left out is the schema's default)
fn knob(state: &Value, key: &str, default: f64) -> f64 {
    state.get(lm_state::PARAMS_KEY).and_then(|p| p.get(key)).and_then(Value::as_real).unwrap_or(default)
}

impl Session {
    /// Starts the analysis of the SlicerT at `node` (`slicert.analyze`): its `sample` is read and analysed on a
    /// worker, and the result is applied by [`Session::slicert_finished`] when the pump collects it. A SlicerT with
    /// no sample has nothing to analyse (`slicert.no_sample`)
    pub fn slicert_analyze(&mut self, node: NodeId) -> Result<lm_base::OperationId> {
        let (_, state) =
            slicert_state(self, node).ok_or_else(|| Error::new("slicert.not_slicert").arg("node", node.to_string()))?;
        let sample = state
            .get("sample")
            .and_then(Value::as_resource)
            .cloned()
            .ok_or_else(|| Error::new("slicert.no_sample").arg("node", node.to_string()))?;
        let threshold = knob(&state, "noteThreshold", 0.6) as f32;
        let snap_key = state.get("snap").and_then(Value::as_text).unwrap_or("off");
        let snap = lm_plugins_instruments::slicert::SNAP_KEYS.iter().position(|k| *k == snap_key).unwrap_or(0);
        let loaded = self.resources().view().get(sample.key()).cloned();
        let project = self.project().folder.root().to_path_buf();
        let key = sample.key().to_owned();
        Ok(self.start(
            Label::new("slicert.analyzing"),
            lm_port::Target::Node(node),
            true,
            "slicert.analyze",
            move |cancel, progress| {
                let data = match loaded {
                    Some(data) => data,
                    None => {
                        let path = lm_io::resource_path(&project, &key)?
                            .ok_or_else(|| Error::new("slicert.sample_missing").arg("key", key.as_str()))?;
                        lm_io::load_file(&path, cancel, progress)?
                    }
                };
                cancel.check()?;
                let audio = lm_plugins_instruments::audio_file_processor::Audio::of(&data);
                let found = lm_plugins_instruments::slicert::analyze(&audio, threshold, snap);
                cancel.check()?;
                let slices = found.slices.iter().filter_map(|p| Value::real(f64::from(*p)).ok());
                Ok(Value::object([
                    ("node", Value::text(node.to_string())),
                    ("sample", Value::text(key.as_str())),
                    ("bpm", Value::int(i64::from(found.bpm))),
                    ("slices", Value::list(slices)),
                ]))
            },
        ))
    }

    /// The analysis is back: `originalBpm` and `slices` go into the node in one transaction (one undo step). Nothing
    /// is written when the sample was changed meanwhile (its own analysis is on its way)
    pub(crate) fn slicert_finished(&mut self, result: &Value) -> Result<()> {
        let node =
            result.get("node").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)?;
        let Some((path, state)) = slicert_state(self, node) else { return Ok(()) };
        let key = result.get("sample").and_then(Value::as_text).unwrap_or_default();
        if state.get("sample").and_then(Value::as_resource).map(|r| r.key()) != Some(key) {
            return Ok(());
        }
        let bpm = result.get("bpm").and_then(Value::as_int).unwrap_or(1).clamp(1, 999);
        let slices = result.get("slices").cloned().unwrap_or_else(Value::empty_list);
        let params_now = state.get(lm_state::PARAMS_KEY).cloned().unwrap_or_else(Value::empty_object);
        let params_new = params_now.with("originalBpm", Value::real(bpm as f64)?);
        let mut params_path = path.clone();
        params_path.push(PathStep::key(lm_state::PARAMS_KEY));
        let mut slices_path = path;
        slices_path.push(PathStep::key("slices"));
        self.document_mut().transact(Label::new("slicert.analyze"), |e| {
            e.set(node, &params_path, params_new)?;
            e.set(node, &slices_path, slices)
        })?;
        Ok(())
    }

    /// The legacy analysed a sample when it was loaded: here, when a SlicerT's `sample` field changes (by the user or
    /// by a client; not by undo, redo or the journal, which restore the slices with it), the analysis starts
    pub(crate) fn slicert_auto(&mut self, diff: &lm_doc::Diff) {
        if matches!(diff.origin, lm_doc::Origin::Undo | lm_doc::Origin::Redo | lm_doc::Origin::Journal) {
            return;
        }
        let mut nodes = Vec::new();
        for c in &diff.changed {
            let sample_set = c.paths.iter().any(|p| {
                matches!(p.last(), Some(PathStep::Key(k)) if &**k == "sample")
                    && matches!(p.len().checked_sub(2).and_then(|i| p.get(i)), Some(PathStep::Key(k)) if &**k == "state")
            });
            if sample_set {
                nodes.push(c.node);
            }
        }
        for node in nodes {
            // (a SlicerT whose sample was emptied has nothing to analyse; any other node is not one)
            if let Some((_, state)) = slicert_state(self, node)
                && state.get("sample").and_then(Value::as_resource).is_some()
            {
                let _ = self.slicert_analyze(node);
            }
        }
    }
}
