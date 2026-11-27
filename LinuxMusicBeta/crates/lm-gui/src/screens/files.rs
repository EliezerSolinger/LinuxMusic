//! Owner: agent D. Files from outside into the project, and the system's file dialogs:
//! - importing (F-ARR-011, F-PRJ-019, F-PRJ-020): a MIDI file goes to the session's `midi.import` at a tick; an
//!   audio file goes through the resource store (`CorePort::import`, an operation) and becomes a new audio track
//!   with its item at the tick; a project folder (or `.zip`) opens; several files at once go one after the other;
//! - the native dialogs (GPUI's `prompt_for_paths` and `prompt_for_new_path`) when the files are this machine's;
//!   with a remote core (`status().connection`, I-15) the files are the server's, and the screens keep their path
//!   fields instead.
//!
//! The legacy turned a short audio file into an instrument track with the sampler; until the AudioFileProcessor is
//! ported every audio file becomes an audio track (decisoes-autonomas.md).

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, Entity, PathPromptOptions, SharedString};
use lm_gui_plugins::PortHost;
use lm_port::{EditOp, Label, Pending, ROOT, Ref, Resource, Value};

use crate::core::Core;
use crate::model;

/// The files are this machine's (no remote core): the native dialogs can be used
pub fn local(core: &Entity<Core>, cx: &App) -> bool {
    core.read(cx).port().status().get("connection").is_none_or(Value::is_null)
}

/// What a dropped or chosen file is, by its extension
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Midi,
    Audio,
    Project,
    Other,
}

pub fn kind_of(path: &Path) -> Kind {
    if path.is_dir() || path.join("projeto.json").exists() {
        return Kind::Project;
    }
    match path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
        Some("mid" | "midi" | "rmi" | "kar") => Kind::Midi,
        Some("wav" | "flac" | "ogg" | "mp3" | "aif" | "aiff" | "rf64" | "w64" | "opus") => Kind::Audio,
        Some("zip") => Kind::Project,
        _ => Kind::Other,
    }
}

/// The system's dialog to choose files (or folders); `then` gets the paths chosen (nothing if cancelled)
pub fn choose(
    cx: &mut App,
    prompt: &'static str,
    folders: bool,
    multiple: bool,
    then: impl FnOnce(Vec<PathBuf>, &mut App) + 'static,
) {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: !folders,
        directories: folders,
        multiple,
        prompt: Some(SharedString::from(crate::ui::tr(prompt))),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = rx.await {
            cx.update(|cx| then(paths, cx));
        }
    })
    .detach();
}

/// The system's dialog to choose where to save; `then` gets the path
pub fn choose_new(cx: &mut App, folder: &Path, name: &str, then: impl FnOnce(PathBuf, &mut App) + 'static) {
    let rx = cx.prompt_for_new_path(folder, Some(name));
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(path))) = rx.await {
            cx.update(|cx| then(path, cx));
        }
    })
    .detach();
}

fn run(core: &Entity<Core>, action: &str, params: Value, cx: &mut App) {
    core.update(cx, |c, cx| {
        c.report(|pt| pt.run_action(action.into(), params));
        cx.notify();
    });
}

/// Imports files at `tick` (a drop on the arrangement, or the Import command at the cursor)
pub fn import_paths(core: &Entity<Core>, paths: &[PathBuf], tick: i64, cx: &mut App) {
    for path in paths {
        match kind_of(path) {
            Kind::Midi => run(
                core,
                "midi.import",
                Value::object([("path", Value::text(path.display().to_string())), ("at", Value::int(tick))]),
                cx,
            ),
            Kind::Audio => import_audio(core, path.clone(), tick, cx),
            Kind::Project => {
                run(core, "project.open", Value::object([("folder", Value::text(path.display().to_string()))]), cx)
            }
            Kind::Other => {}
        }
    }
}

/// An audio file into the store, then a new audio track with the item at `tick` (F-ARR-011, F-PRJ-020)
pub fn import_audio(core: &Entity<Core>, path: PathBuf, tick: i64, cx: &mut App) {
    let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().to_string());
    let answer = core.update(cx, |c, _| c.import(path));
    let core = core.clone();
    match answer {
        Pending::Ready(Ok(res)) => wait_length(core, res, name, tick, cx),
        Pending::Ready(Err(_)) => {} // the core's notice says why
        Pending::Waiting(id) => {
            cx.spawn(async move |cx| {
                // the import is an operation: its answer comes in a later pump (at most a minute for a big file)
                for _ in 0..1200 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    let done = cx.update(|cx| core.update(cx, |c, _| c.take_imported(id)));
                    if let Some(r) = done {
                        if let Ok(res) = r {
                            cx.update(|cx| wait_length(core.clone(), res, name.clone(), tick, cx));
                        }
                        return;
                    }
                }
                // gave up waiting: the answer, if it comes, is dropped
                cx.update(|cx| core.update(cx, |c, _| c.forget_request(id)));
            })
            .detach();
        }
    }
}

/// The resource's length comes with its peaks; then the track is made
fn wait_length(core: Entity<Core>, res: Resource, name: String, tick: i64, cx: &mut App) {
    cx.spawn(async move |cx| {
        for _ in 0..1200 {
            let length =
                cx.update(|cx| core.update(cx, |c, _| c.port_mut().peaks(&res, 12, 0, 1).ok().map(|b| b.length)));
            if let Some(samples) = length {
                cx.update(|cx| add_audio_track(&core, &res, &name, tick, samples as i64, cx));
                return;
            }
            cx.background_executor().timer(Duration::from_millis(50)).await;
        }
    })
    .detach();
}

/// A new audio track with one item that plays the whole file from `tick`: one undo step
pub fn add_audio_track(core: &Entity<Core>, res: &Resource, name: &str, tick: i64, samples: i64, cx: &mut App) {
    let root = core.read(cx).port().root().clone();
    let map = model::tempo_map(&root);
    let length = (map.tick_at(map.sample_at(tick) + samples) - tick).max(1);
    let index = model::list(&root, "tracks").count();
    let lane = Value::object([("source", Value::resource(res.clone())), ("name", Value::text(name))]);
    let item = Value::object([
        ("kind", Value::text("audio")),
        ("name", Value::text(name)),
        ("start", Value::int(tick)),
        ("length", Value::int(length)),
        ("timebase", Value::text("audio")),
        ("lanes", Value::list([lane])),
    ]);
    let track = Value::object([("name", Value::text(name)), ("items", Value::list([item]))]);
    core.update(cx, |c, cx| {
        c.report(|pt| pt.apply(
            Label::new("Import audio"),
            vec![EditOp::Insert { temp: 1, parent: Ref::Id(ROOT), list: "tracks".into(), index, node: track }],
        ));
        cx.notify();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of(Path::new("/x/a.MID")), Kind::Midi);
        assert_eq!(kind_of(Path::new("/x/voz.wav")), Kind::Audio);
        assert_eq!(kind_of(Path::new("/x/song.zip")), Kind::Project);
        assert_eq!(kind_of(Path::new("/x/readme.txt")), Kind::Other);
    }
}
