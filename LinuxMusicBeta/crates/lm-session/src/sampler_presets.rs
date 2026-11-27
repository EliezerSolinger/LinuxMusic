//! `sampler.list_presets {node}`: the presets (bank, program, name) of the bank file the Sampler at `node` has loaded
//! (a SoundFont or a GIG), for the panel's list. Reading the file may take longer than 200 ms, so it runs in an
//! operation; the answer goes into the session's status tree under `pluginInfo.<node>`:
//! `{format, key, presets: [{bank, program, name}]}`. The status is not the project: nothing is saved, no undo step.
//! The panel reads it from `CorePort::status()` and asks again when the file it lists is not the one it shows (`key`).
//!
//! The same listing runs by itself when `plugin.load_into_field` puts a bank file into a Sampler.

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, Value};

use crate::actions::ActionSpec;
use crate::session::Session;

pub(crate) const ACTION: &str = "sampler.list_presets";

pub(crate) static ACTIONS: &[ActionSpec] = &[ActionSpec {
    id: ACTION,
    label: "action.sampler.list_presets",
    run: |s, p| s.sampler_list_presets(p),
    enabled: |_| true,
}];

fn params() -> Error {
    Error::new("action.params")
}

impl Session {
    /// Starts reading the presets of the Sampler `p.node`
    pub fn sampler_list_presets(&mut self, p: &Value) -> Result<()> {
        let node = p.get("node").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)?;
        self.sampler_list(node)
    }

    pub(crate) fn sampler_list(&mut self, node: NodeId) -> Result<()> {
        let found = self
            .document()
            .find(node)
            .ok_or_else(|| Error::new("action.node.missing").arg("node", node.to_string()))?;
        let plugin = found.get("instrument").cloned().unwrap_or(found);
        if plugin.get("plugin").and_then(Value::as_text) != Some("sampler") {
            return Err(Error::new("sampler.list.not_a_sampler").arg("node", node.to_string()));
        }
        let state = plugin.get("state").cloned().unwrap_or_else(Value::empty_object);
        let format = state.get("format").and_then(Value::as_text).unwrap_or("none").to_owned();
        let node_text = node.to_string();
        let Some(field) = lm_plugins_sampler::list::field_of(&format) else {
            // (a format without presets: the list is empty)
            self.status_presets(&node_text, &format, "", Value::empty_list());
            return Ok(());
        };
        let Some(resource) = state.get(field).and_then(Value::as_resource).cloned() else {
            self.status_presets(&node_text, &format, "", Value::empty_list());
            return Ok(());
        };
        let project = self.project().folder.root().to_path_buf();
        let label = Label::new("sampler.listing_presets");
        self.start(label, lm_port::Target::Node(node), true, ACTION, move |cancel, _progress| {
            let result = |presets: Value| {
                Value::object([
                    ("node", Value::text(node_text.as_str())),
                    ("format", Value::text(format.as_str())),
                    ("key", Value::text(resource.key())),
                    ("presets", presets),
                ])
            };
            let Some(path) = lm_io::resource_path_of(&project, resource.kind(), resource.key())? else {
                // (not in the project yet, or missing: nothing to list)
                return Ok(result(Value::empty_list()));
            };
            let bytes = std::fs::read(&path).map_err(|e| {
                Error::new("sampler.read_failed").arg("path", path.display().to_string()).arg("error", e.to_string())
            })?;
            cancel.check()?;
            let presets = lm_plugins_sampler::list::presets(&format, &bytes)?;
            Ok(result(lm_plugins_sampler::list::to_value(&presets)))
        });
        Ok(())
    }

    fn status_presets(&mut self, node: &str, format: &str, key: &str, presets: Value) {
        let entry = Value::object([("format", Value::text(format)), ("key", Value::text(key)), ("presets", presets)]);
        let info = self.status().get("pluginInfo").cloned().unwrap_or_else(Value::empty_object);
        self.set_status_key("pluginInfo", info.with(node, entry));
    }

    /// The listing arrived: the status holds it
    pub(crate) fn sampler_presets_finished(&mut self, result: &Value) {
        let node = result.get("node").and_then(Value::as_text).unwrap_or("").to_owned();
        let format = result.get("format").and_then(Value::as_text).unwrap_or("").to_owned();
        let key = result.get("key").and_then(Value::as_text).unwrap_or("").to_owned();
        let presets = result.get("presets").cloned().unwrap_or_else(Value::empty_list);
        if !node.is_empty() {
            self.status_presets(&node, &format, &key, presets);
        }
    }
}
