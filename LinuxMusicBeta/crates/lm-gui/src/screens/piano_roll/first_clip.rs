//! Opening the editor on a project without a MIDI clip (F-NOT-022): the first clip of the first instrument track,
//! a bar long at the start of the song (by the tempo map), made and opened.

use lm_port::{EditOp, Ref, Value};

use crate::model;

/// The edit that makes the clip; none when no track has an instrument
pub fn first_clip_op(root: &Value) -> Option<EditOp> {
    let track = model::tracks(root).into_iter().find(|t| t.instrument.is_some() && !t.folder)?;
    let map = model::tempo_map(root);
    let length = map.bar_start(2).unwrap_or(4 * i64::from(model::ppq(root))).max(1);
    let node = Value::object([
        ("kind", Value::text("midi")),
        ("name", Value::text(track.name.to_string())),
        ("start", Value::int(0)),
        ("length", Value::int(length)),
    ]);
    Some(EditOp::Insert { temp: 0, parent: Ref::Id(track.id), list: "items".into(), index: track.items.len(), node })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_without_tracks_makes_no_clip() {
        assert!(first_clip_op(&Value::object([("tracks", Value::empty_list())])).is_none());
    }
}
