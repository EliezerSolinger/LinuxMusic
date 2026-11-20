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
pub(crate) static ACTIONS: &[ActionSpec] = &[ActionSpec {
    id: "opulenz.import_sbi",
    label: "action.opulenz.import_sbi",
    run: import_sbi,
    enabled: |s| !s.project().read_only,
}];

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
