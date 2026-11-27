//! Recording a track's output (F-REC-011): the track, with its inserts, volume and pan, rendered offline into a new
//! audio track of its own ("resample"). The render is the export's stem of that track (the same engine, so the file is
//! what the track sounds like), imported into the project, and the new track plays it from the start of the song.
//! Offline on purpose: a live capture of a track's post-effect signal would put a second copy of the track's
//! buffer on the audio thread for the whole take, and the result would be the same file.

use std::path::PathBuf;

use lm_base::{Error, Label, Result};
use lm_port::Target;
use lm_state::{NodeId, Resource, ROOT, Value};

use crate::export::{ExportRequest, FileFormat, Normalize, StemsBy};
use crate::session::Session;

/// The request that renders the track alone into `folder`, the whole song with a short tail
pub(crate) fn resample_request(folder: PathBuf, track: NodeId) -> ExportRequest {
    ExportRequest {
        folder,
        name: "resample".to_owned(),
        formats: vec![FileFormat::Wav(32)],
        rate: None,
        dither: false,
        normalize: Normalize::None,
        fade_in: 0.0,
        fade_out: 0.0,
        range: None,
        as_loop: false,
        repeats: 1,
        only_track: None,
        stems: Some((StemsBy::Tracks, vec![track])),
        tail: 2.0,
    }
}

/// The node of the new audio track: one item that plays the whole file from sample 0
pub(crate) fn resample_track(name: &str, resource: &Resource, length_ticks: i64) -> Value {
    let lane = Value::object([("source", Value::resource(resource.clone())), ("name", Value::text(name))]);
    let item = Value::object([
        ("kind", Value::text("audio")),
        ("name", Value::text(name)),
        ("start", Value::int(0)),
        ("length", Value::int(length_ticks.max(1))),
        ("lanes", Value::list([lane])),
    ]);
    Value::object([("name", Value::text(name)), ("items", Value::list([item]))])
}

impl Session {
    /// `track.resample {track}`: renders the track and puts the result on a new audio track, in an operation
    pub fn resample_track(&mut self, track: NodeId) -> Result<()> {
        let node = self.document().find(track).ok_or_else(|| Error::new("session.resample.no_track"))?;
        if node.get("items").is_none() {
            return Err(Error::new("session.resample.no_track"));
        }
        let name = match node.get("name").and_then(Value::as_text).filter(|n| !n.is_empty()) {
            Some(n) => format!("{n} (resample)"),
            None => "Resample".to_owned(),
        };
        let project = self.project().folder.root().to_path_buf();
        let rate = self.telemetry().position.sample_rate;
        let job = self.export_job();
        let temp = project.join(".resample");
        let request = resample_request(temp.clone(), track);
        self.start(Label::new("session.resampling"), Target::Project, true, "resample", move |cancel, progress| {
            let done = (|| {
                let report = crate::export::export_request(job, &request, cancel, progress)?;
                let file = report.get("file").and_then(Value::as_text).filter(|f| !f.is_empty()).ok_or_else(|| {
                    Error::new("session.export.nothing")
                })?;
                let imported = lm_io::import_file(std::path::Path::new(file), &project, rate, cancel, progress)?;
                Ok(Value::object([
                    ("resource", Value::resource(Resource::new("audio", &imported.key)?)),
                    ("frames", Value::int(i64::try_from(imported.frames).unwrap_or(i64::MAX))),
                    ("name", Value::text(name)),
                ]))
            })();
            // The temporary render never stays, whatever happened
            let _ = std::fs::remove_dir_all(&temp);
            done
        });
        Ok(())
    }

    /// The render came back: the new audio track enters the tree in one transaction (one undo removes it)
    pub(crate) fn resample_finished(&mut self, result: Value) {
        let (Some(resource), Some(name)) =
            (result.get("resource").and_then(Value::as_resource).cloned(), result.get("name").and_then(Value::as_text))
        else {
            return;
        };
        let frames = result.get("frames").and_then(Value::as_int).unwrap_or(0);
        let name = name.to_owned();
        let Ok(map) = crate::session::tempo_map(self.document().root()) else { return };
        let length = map.tick_at(frames);
        let index = self.document().root().get("tracks").map_or(0, Value::len);
        let node = resample_track(&name, &resource, length);
        self.request(&resource);
        let r = self.document_mut().transact(Label::new("session.resample"), |e| {
            e.insert(ROOT, "tracks", index, node).map(|_| ())
        });
        if let Err(e) = r {
            self.notices().error(&e, Some(Target::Project), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_renders_that_track_alone_as_a_float_file() {
        let id = NodeId::parse("t:7").expect("id");
        let r = resample_request(PathBuf::from("/tmp/x"), id);
        assert_eq!(r.stems, Some((StemsBy::Tracks, vec![id])));
        assert_eq!(r.formats, vec![FileFormat::Wav(32)]);
        assert_eq!(r.repeats, 1);
    }

    #[test]
    fn the_new_track_plays_the_file_from_the_start() {
        let res = Resource::of_content("audio", b"x").expect("resource");
        let t = resample_track("Bass (resample)", &res, 960);
        let item = t.get("items").and_then(|i| i.items().next().cloned()).expect("item");
        assert_eq!(item.get("start").and_then(Value::as_int), Some(0));
        assert_eq!(item.get("length").and_then(Value::as_int), Some(960));
        let lane = item.get("lanes").and_then(|l| l.items().next().cloned()).expect("lane");
        assert_eq!(lane.get("source").and_then(Value::as_resource), Some(&res));
    }
}
