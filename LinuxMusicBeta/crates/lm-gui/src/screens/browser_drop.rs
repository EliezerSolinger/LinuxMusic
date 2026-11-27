//! What the browser's rows do when they are dropped on the arrangement or on a track (F-BRW-007): audio becomes an
//! item (a new track, or on the track under the pointer), a MIDI file is imported at the drop, a preset loads on the
//! track's instrument, an instrument plugin takes the track. The files are the core's: the path comes from the
//! entry's info, the import is the core's operation.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, Entity};
use lm_gui_plugins::PortHost;
use lm_port::{EditOp, EntryKind, Label, NodeId, Pending, Ref, Resource, Value};

use super::browser::Dragged;
use super::files;
use crate::core::Core;
use crate::model;

/// The tick of the playhead
pub fn playhead_tick(core: &Entity<Core>, cx: &App) -> i64 {
    let c = core.read(cx);
    c.port().tempo_map().tick_at(c.port().telemetry().position.samples)
}

/// What a drop does, decided from the entry alone (the part the tests cover)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drop {
    /// Audio at the tick: on the track, else a new track
    Audio(PathBuf),
    Midi(PathBuf),
    /// A preset of the entry id on the track
    Preset(u64),
    /// An instrument plugin on the track
    Instrument(String),
    Nothing,
}

pub fn plan(d: &Dragged, track: Option<NodeId>, instrument_plugin: impl Fn(&str) -> bool) -> Drop {
    match d {
        Dragged::Plugin(id) if track.is_some() && instrument_plugin(id) => Drop::Instrument(id.to_string()),
        Dragged::Plugin(_) => Drop::Nothing,
        Dragged::Entry(e) => {
            let path = e.info.get("path").and_then(Value::as_text).map(PathBuf::from);
            match (e.kind, path) {
                (EntryKind::Audio, Some(p)) => Drop::Audio(p),
                (EntryKind::Midi, Some(p)) => Drop::Midi(p),
                (EntryKind::Preset, _) if track.is_some() => Drop::Preset(e.id.0),
                _ => Drop::Nothing,
            }
        }
    }
}

/// Does a drop of `d` at `tick`, on `track` when the pointer was over one
pub fn drop_entry(core: &Entity<Core>, d: &Dragged, track: Option<NodeId>, tick: i64, cx: &mut App) {
    let plugins = core.read(cx).port().plugins();
    let action = plan(d, track, |id| plugins.iter().any(|p| &*p.id == id && p.instrument));
    match action {
        Drop::Audio(path) => match track {
            Some(t) => audio_on_track(core, path, t, tick, cx),
            None => files::import_audio(core, path, tick, cx),
        },
        Drop::Midi(path) => files::import_paths(core, &[path], tick, cx),
        Drop::Preset(entry) => {
            if let Some(t) = track {
                let params = Value::object([("node", Value::text(t.to_string())), ("entry", Value::int(entry as i64))]);
                core.update(cx, |c, _| {
                    c.report(|pt| pt.run_action("plugin.load_preset".into(), params));
                });
            }
        }
        Drop::Instrument(id) => {
            if let Some(t) = track {
                let node = Value::object([("plugin", Value::text(id)), ("state", Value::empty_object())]);
                let ops = vec![EditOp::set(t, &["instrument"], node)];
                core.update(cx, |c, _| {
                    c.report(|pt| pt.apply(Label::new("Swap the instrument"), ops));
                });
            }
        }
        Drop::Nothing => {}
    }
}

/// An audio file as an item of this track at `tick`: imported by the core, then added when its length is known
fn audio_on_track(core: &Entity<Core>, path: PathBuf, track: NodeId, tick: i64, cx: &mut App) {
    let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().to_string());
    let answer = core.update(cx, |c, _| c.import(path));
    let core = core.clone();
    match answer {
        Pending::Ready(Ok(res)) => wait_length(core, res, name, track, tick, cx),
        Pending::Ready(Err(_)) => {} // `Core::import` already reported it
        Pending::Waiting(id) => {
            cx.spawn(async move |cx| {
                for _ in 0..1200 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    if let Some(r) = cx.update(|cx| core.update(cx, |c, _| c.take_imported(id))) {
                        match r {
                            Ok(res) => cx.update(|cx| wait_length(core.clone(), res, name.clone(), track, tick, cx)),
                            Err(e) => cx.update(|cx| core.update(cx, |c, _| c.fail(e))),
                        }
                        return;
                    }
                }
                cx.update(|cx| core.update(cx, |c, _| c.forget_request(id)));
            })
            .detach();
        }
    }
}

fn wait_length(core: Entity<Core>, res: Resource, name: String, track: NodeId, tick: i64, cx: &mut App) {
    cx.spawn(async move |cx| {
        for _ in 0..1200 {
            match cx.update(|cx| core.update(cx, |c, _| files::length_now(c, &res))) {
                Some(Some(samples)) => {
                    cx.update(|cx| add_item(&core, &res, &name, track, tick, samples as i64, cx));
                    return;
                }
                Some(None) => return,
                None => {}
            }
            cx.background_executor().timer(Duration::from_millis(50)).await;
        }
    })
    .detach();
}

fn add_item(core: &Entity<Core>, res: &Resource, name: &str, track: NodeId, tick: i64, samples: i64, cx: &mut App) {
    let root = core.read(cx).port().root().clone();
    let map = model::tempo_map(&root);
    let length = (map.tick_at(map.sample_at(tick) + samples) - tick).max(1);
    let index = model::tracks(&root).into_iter().find(|t| t.id == track).map_or(0, |t| t.items.len());
    let lane = Value::object([("source", Value::resource(res.clone())), ("name", Value::text(name))]);
    let item = Value::object([
        ("kind", Value::text("audio")),
        ("name", Value::text(name)),
        ("start", Value::int(tick)),
        ("length", Value::int(length)),
        ("timebase", Value::text("audio")),
        ("lanes", Value::list([lane])),
    ]);
    core.update(cx, |c, cx| {
        c.report(|pt| {
            pt.apply(
                Label::new("Add audio"),
                vec![EditOp::Insert { temp: 1, parent: Ref::Id(track), list: "items".into(), index, node: item }],
            )
        });
        cx.notify();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use lm_port::{BrowseEntry, EntryId};

    fn entry(kind: EntryKind, path: &str) -> BrowseEntry {
        BrowseEntry {
            id: EntryId(5),
            parent: None,
            name: "x".into(),
            kind,
            factory: false,
            hidden: false,
            favorite: false,
            info: Value::object([("path", Value::text(path))]),
        }
    }

    #[test]
    fn a_drop_is_planned_from_the_entry() {
        let t = NodeId::parse("t:1").ok();
        let none = |_: &str| false;
        assert_eq!(plan(&Dragged::Entry(entry(EntryKind::Audio, "/a.wav")), None, none), Drop::Audio("/a.wav".into()));
        assert_eq!(plan(&Dragged::Entry(entry(EntryKind::Midi, "/a.mid")), t, none), Drop::Midi("/a.mid".into()));
        assert_eq!(plan(&Dragged::Entry(entry(EntryKind::Preset, "/p.json")), t, none), Drop::Preset(5));
        assert_eq!(plan(&Dragged::Entry(entry(EntryKind::Preset, "/p.json")), None, none), Drop::Nothing);
        assert_eq!(plan(&Dragged::Plugin("monstro".into()), t, |_| true), Drop::Instrument("monstro".into()));
        assert_eq!(plan(&Dragged::Plugin("compressor".into()), t, |_| false), Drop::Nothing);
    }
}
