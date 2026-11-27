//! `plugin.load_into_field {node, field, path | bytes, name?}`: a file into one field of a plugin's state, the one
//! action behind every "open a file" button of a plugin panel (a Patman patch, a Watsyn, Vibed or BitInvader wave) and
//! the same for the command palette and a remote client.
//!
//! - `node`: the plugin's node, or the track whose instrument it is. `field`: a field of the plugin's schema.
//! - `path` is a file on the machine of the session; `bytes` (a list of 0–255) is the file itself, for a client that
//!   read it where it runs, and `name` the file's name then (it gives the extension and what the panel shows).
//! - A **resource field** (`FieldKind::Resource(kind)`): the file is imported into the project through the resource
//!   store (an audio file as in `session.import`, anything else copied to `state/` by its hash) and the field is set
//!   to the resource, which keeps the file's name for the panels to show.
//! - A **field that is a list of reals** (a wave graph): the audio file is decoded and its first channel is read at
//!   as many points as the field has (the legacy's `setWaveToUser`: linear interpolation around the wave), within
//!   the field's range.
//!
//! Reading, hashing and decoding run in an operation (a file may take more than 200 ms); the change is made when
//! the pump collects it, in one transaction: one undo step. Nothing is written when the node went away meanwhile.

use std::path::{Path, PathBuf};

use lm_base::{Error, Label, Result};
use lm_state::{FieldKind, NodeId, PathStep, Resource, Value};

use crate::actions::ActionSpec;
use crate::session::Session;

pub(crate) const ACTION: &str = "plugin.load_into_field";
const KIND: &str = ACTION;

/// A file of a plugin's field is never larger than this when it comes as bytes
const BYTES_MAX: usize = 64 << 20;

pub(crate) static ACTIONS: &[ActionSpec] = &[ActionSpec {
    id: ACTION,
    label: "action.plugin.load_into_field",
    run: |s, p| s.load_into_field(p).map(|_| ()),
    enabled: |s| !s.project().read_only,
}];

fn params() -> Error {
    Error::new("action.params")
}

/// What the field is, as far as loading goes
#[derive(Clone, Debug)]
enum Target {
    Resource(String),
    Wave { length: usize, min: f64, max: f64 },
}

/// Where a plugin's state is: the plugin id, the path to the state from `node`, and the state
fn plugin_at(s: &Session, node: NodeId) -> Result<(String, Vec<PathStep>, Value)> {
    let found =
        s.document().find(node).ok_or_else(|| Error::new("action.node.missing").arg("node", node.to_string()))?;
    // A modulator holds its fields itself (its user wave is the field `userWave`)
    if node.kind == lm_doc::schemas::KIND_MODULATOR {
        return Ok((lm_doc::schemas::types::MODULATOR.to_owned(), Vec::new(), found));
    }
    let (plugin, path) = match found.get("instrument") {
        Some(instrument) => (instrument.clone(), vec![PathStep::key("instrument"), PathStep::key("state")]),
        None => (found, vec![PathStep::key("state")]),
    };
    let id = plugin
        .get("plugin")
        .and_then(Value::as_text)
        .ok_or_else(|| Error::new("plugin.load.not_a_plugin").arg("node", node.to_string()))?
        .to_owned();
    let state = plugin.get("state").cloned().unwrap_or_else(Value::empty_object);
    Ok((id, path, state))
}

fn extension_of(name: &str) -> String {
    let ext: String = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(10)
        .collect::<String>()
        .to_ascii_lowercase();
    if ext.is_empty() { "bin".to_owned() } else { ext }
}

/// A file made from bytes, removed when dropped
struct Scratch(PathBuf);

impl Scratch {
    fn new(bytes: &[u8], name: &str) -> Result<Self> {
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path =
            std::env::temp_dir().join(format!("lm-field-{}-{unique}.{}", std::process::id(), extension_of(name)));
        std::fs::write(&path, bytes).map_err(|e| {
            Error::new("plugin.load.write").arg("path", path.display().to_string()).arg("why", e.to_string())
        })?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The wave of the first channel of @p data at @p length points, around in a loop (`Oscillator::userWaveSample`),
/// within `min..=max`
pub(crate) fn wave_points(data: &lm_render::SampleData, length: usize, min: f64, max: f64) -> Vec<f32> {
    let wave = if data.channels() > 0 { data.channel(0) } else { &[] };
    (0..length)
        .map(|i| {
            let v = lm_dsp::oscillator::user_wave_sample(wave, i as f32 / length as f32);
            v.clamp(min as f32, max as f32)
        })
        .collect()
}

impl Session {
    /// Starts loading a file into a plugin's field (`plugin.load_into_field`); the change is made by
    /// [`Session::field_load_finished`] when the pump collects the operation
    pub fn load_into_field(&mut self, p: &Value) -> Result<lm_base::OperationId> {
        let node = p.get("node").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)?;
        let field = p.get("field").and_then(Value::as_text).ok_or_else(params)?.to_owned();
        let (plugin, _, state) = plugin_at(self, node)?;
        let spec = self
            .registry()
            .schema(&plugin)
            .and_then(|s| s.find_field(&field))
            .ok_or_else(|| Error::new("plugin.load.no_field").arg("field", field.as_str()))?;
        let target = match &spec.kind {
            FieldKind::Resource(kind) => Target::Resource((*kind).to_owned()),
            FieldKind::List(inner) => match **inner {
                FieldKind::Real { min, max } => {
                    let length = state.get(&field).or(spec.default.as_ref()).map_or(0, |v| v.items().count());
                    if length == 0 {
                        return Err(Error::new("plugin.load.no_length").arg("field", field.as_str()));
                    }
                    Target::Wave { length, min, max }
                }
                _ => return Err(Error::new("plugin.load.field_kind").arg("field", field.as_str())),
            },
            _ => return Err(Error::new("plugin.load.field_kind").arg("field", field.as_str())),
        };
        // The file: on the session's machine, or the bytes of a client's
        let (source, name) = match (p.get("path").and_then(Value::as_text), p.get("bytes")) {
            (Some(path), _) => {
                let name =
                    p.get("name").and_then(Value::as_text).map(str::to_owned).unwrap_or_else(|| {
                        Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned()
                    });
                (Source::Path(PathBuf::from(path)), name)
            }
            (None, Some(list)) => {
                let bytes = list
                    .items()
                    .take(BYTES_MAX + 1)
                    .map(|v| v.as_int().and_then(|b| u8::try_from(b).ok()).ok_or_else(params))
                    .collect::<Result<Vec<u8>>>()?;
                if bytes.len() > BYTES_MAX {
                    return Err(params());
                }
                (Source::Bytes(bytes), p.get("name").and_then(Value::as_text).unwrap_or("").to_owned())
            }
            _ => return Err(params()),
        };
        let project = self.project().folder.root().to_path_buf();
        let rate = self.spec().sample_rate;
        let faults = self.faults().clone();
        let label = Label::new("plugin.loading_file").arg("field", field.as_str());
        let node_text = node.to_string();
        Ok(self.start(label, lm_port::Target::Node(node), true, KIND, move |cancel, progress| {
            let scratch;
            let path: &Path = match &source {
                Source::Path(p) => p,
                Source::Bytes(b) => {
                    scratch = Scratch::new(b, &name)?;
                    &scratch.0
                }
            };
            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            faults.check(size).map_err(|e| e.arg("file", path.display().to_string()))?;
            let base = Value::object([
                ("node", Value::text(node_text.as_str())),
                ("field", Value::text(field.as_str())),
                ("name", Value::text(name.as_str())),
            ]);
            match target {
                Target::Resource(kind) => {
                    let key = if kind == "audio" {
                        lm_io::import_file(path, &project, rate, cancel, progress)?.key
                    } else {
                        lm_io::import_file_as(path, &project, &kind, cancel, progress)?.key
                    };
                    Ok(base.with("kind", Value::text(kind.as_str())).with("key", Value::text(key.as_str())))
                }
                Target::Wave { length, min, max } => {
                    let data = lm_io::load_file(path, cancel, progress)?;
                    cancel.check()?;
                    let points = wave_points(&data, length, min, max);
                    let mut list = Vec::with_capacity(points.len());
                    for v in points {
                        list.push(Value::real(f64::from(v))?);
                    }
                    Ok(base.with("samples", Value::list(list)))
                }
            }
        }))
    }

    /// The file is read: the field is set in one transaction (one undo step), and a resource is asked to load
    pub(crate) fn field_load_finished(&mut self, result: &Value) -> Result<()> {
        let node =
            result.get("node").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)?;
        let field = result.get("field").and_then(Value::as_text).ok_or_else(params)?.to_owned();
        // (the node removed meanwhile, or no longer the plugin: nothing to set)
        let Ok((_, mut path, _)) = plugin_at(self, node) else { return Ok(()) };
        path.push(PathStep::key(&field));
        let mut asked = None;
        let value = if let Some(samples) = result.get("samples") {
            samples.clone()
        } else {
            let kind = result.get("kind").and_then(Value::as_text).ok_or_else(params)?;
            let key = result.get("key").and_then(Value::as_text).ok_or_else(params)?;
            let name = result.get("name").and_then(Value::as_text).unwrap_or("");
            let resource = Resource::new(kind, key)?.with_name(name);
            asked = Some(resource.clone());
            Value::resource(resource)
        };
        self.document_mut().transact(Label::new("plugin.load_into_field").arg("field", field.as_str()), |e| {
            e.set(node, &path, value)
        })?;
        if let Some(r) = asked {
            self.request(&r);
            // A bank file in a Sampler: its presets are listed for the panel
            if matches!(r.kind(), "soundfont" | "gig") {
                let _ = self.sampler_list(node);
            }
        }
        Ok(())
    }
}

enum Source {
    Path(PathBuf),
    Bytes(Vec<u8>),
}
