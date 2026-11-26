//! The actions of a track's tuning by file (F-IJN-019/020): a Scala scale (`.scl`) and an optional keymap (`.kbm`)
//! copied into the project as resources of kind `"tuning"`, and cited by the track's `tuning`.
//!
//! - `tuning.set_scale {track, path}`: the file is checked, copied into the project and cited; the tuning is on.
//! - `tuning.set_keymap {track, path}`: the same for the keymap.
//! - `tuning.clear {track}`: the track plays in 12-TET again (the files stay in the project until cleaned).
//!
//! One undo step each. `path` is a file on the machine of the session (the server, in client mode).

use std::path::Path;

use lm_base::{Cancel, Error, Label, Progress, Result};
use lm_doc::schemas::{TUNING_KIND, types};
use lm_state::{NodeId, PathStep, Resource, Value};

use crate::actions::ActionSpec;
use crate::session::Session;

fn params() -> Error {
    Error::new("action.params")
}

/// The tuning actions (chained into `actions::all`)
pub(crate) static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "tuning.set_scale",
        label: "action.tuning.set_scale",
        run: |s, p| set_file(s, p, false),
        enabled: |s| !s.project().read_only,
    },
    ActionSpec {
        id: "tuning.set_keymap",
        label: "action.tuning.set_keymap",
        run: |s, p| set_file(s, p, true),
        enabled: |s| !s.project().read_only,
    },
    ActionSpec { id: "tuning.clear", label: "action.tuning.clear", run: clear, enabled: |s| !s.project().read_only },
];

/// The track of the parameters, checked
fn track_of(s: &Session, p: &Value) -> Result<(NodeId, Value)> {
    let node = p.get("track").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)?;
    let found =
        s.document().find(node).ok_or_else(|| Error::new("action.node.missing").arg("node", node.to_string()))?;
    if s.document().type_of(node) != Some(types::TRACK) {
        return Err(params());
    }
    Ok((node, found))
}

/// The file's bytes, up to the size a tuning file can have
fn read_checked(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let fail = |e: std::io::Error| {
        Error::new("tuning.file.unreadable").arg("path", path.display().to_string()).arg("why", e.to_string())
    };
    let file = std::fs::File::open(path).map_err(fail)?;
    let mut bytes = Vec::new();
    file.take(lm_doc::tuning::FILE_MAX_BYTES + 1).read_to_end(&mut bytes).map_err(fail)?;
    if bytes.len() as u64 > lm_doc::tuning::FILE_MAX_BYTES {
        return Err(Error::new("tuning.file.too_big").arg("path", path.display().to_string()));
    }
    Ok(bytes)
}

fn set_file(s: &mut Session, p: &Value, keymap: bool) -> Result<()> {
    let (node, track) = track_of(s, p)?;
    let path = p.get("path").and_then(Value::as_text).ok_or_else(params)?;
    let path = Path::new(path);
    let bytes = read_checked(path)?;
    // The file must be one before it enters the project
    let text = lm_doc::tuning::text_of(&bytes);
    if keymap {
        lm_doc::tuning::parse_kbm(&text)?;
    } else {
        lm_doc::tuning::parse_scl(&text)?;
    }
    // A few kilobytes: copied here, not in an operation
    let imported =
        lm_io::import_file_as(path, s.project().folder.root(), TUNING_KIND, &Cancel::new(), &Progress::new())?;
    let resource = Resource::new(TUNING_KIND, &imported.key)?;
    let field = if keymap { "keymap" } else { "scale" };
    let current = track
        .get("tuning")
        .filter(|t| !t.is_null())
        .cloned()
        .unwrap_or_else(|| Value::object([("on", Value::bool(true)), ("importRanges", Value::bool(true))]));
    // The first file switches the tuning on; a later one keeps the switch where the user left it
    let on = if current.get("scale").is_none_or(Value::is_null) && current.get("keymap").is_none_or(Value::is_null) {
        true
    } else {
        current.get("on").and_then(Value::as_bool).unwrap_or(true)
    };
    let tuning = current.with(field, Value::resource(resource.clone())).with("on", Value::bool(on));
    let label = Label::new(if keymap { "tuning.set_keymap" } else { "tuning.set_scale" });
    s.document_mut().transact(label, |e| e.set(node, &[PathStep::key("tuning")], tuning))?;
    s.request(&resource);
    Ok(())
}

fn clear(s: &mut Session, p: &Value) -> Result<()> {
    let (node, track) = track_of(s, p)?;
    if track.get("tuning").is_none_or(Value::is_null) {
        return Ok(());
    }
    s.document_mut().transact(Label::new("tuning.clear"), |e| e.set(node, &[PathStep::key("tuning")], Value::NULL))?;
    Ok(())
}
