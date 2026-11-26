//! A scanned plugin made an external plugin of the registry (`lm_plugin::External`): its id, its schema (its
//! parameters, so they are params the slots, automation and MIDI mapping reach, I-21; its own state as `"chunk"`), its
//! ports, and a build that starts its helper
use std::sync::Arc;

use lm_doc::{Port, Ports};
use lm_plugin::{BuildContext, External, PluginKind};
use lm_render::Processor;
use lm_state::{FieldKind, FieldSpec, NodeSchema, PARAMS_KEY, ParamSpec, Scale, Value};

use crate::hosted::{HostConfig, Hosted, LoadRequest, start};
use crate::protocol::unhex;
use crate::scan::Scanned;

/// Called with each helper started, so the session can reach it (its state, its window)
pub type OnStart = Arc<dyn Fn(&Scanned, Arc<Hosted>) + Send + Sync>;

/// The registry id of a scanned plugin: its format and 16 hex digits of the BLAKE3 of its URI
pub fn plugin_id(format: &str, uri: &str) -> String {
    let h = blake3::hash(uri.as_bytes()).to_hex();
    format!("{format}-{}", &h[..16])
}

/// The audio channels of the helper's shared block for a plugin on a channel of `channels` (0 for an instrument,
/// which has no channel to follow): (inputs, outputs). The track's or channel's count rules, not the plugin's own: a mono
/// effect on a stereo channel gets 2/2 (the helper runs it as two instances, dual mono); a mono instrument gets two
/// outputs (the helper spreads its one to both). Anything else keeps the plugin's own count
pub fn layout_channels(s: &Scanned, channels: u16) -> (usize, usize) {
    if s.instrument {
        return (s.inputs, if s.outputs == 1 { 2 } else { s.outputs });
    }
    if s.inputs == 1 && s.outputs == 1 && channels == 2 { (2, 2) } else { (s.inputs, s.outputs) }
}

/// The external plugin for a scanned one
pub fn external(scanned: &Scanned, config: Arc<HostConfig>, on_start: Option<OnStart>) -> External {
    let id = plugin_id(&scanned.format, &scanned.uri);
    let type_name = lm_base::intern(&lm_plugin::plugin_type(&id));
    // The plugin's format and URI are its id's (fixed); its own state is `"chunk"`, in hex (above 64 KB the session
    // keeps it as a resource, U24). A chunk the plugin cannot read is not an error: it starts from its defaults
    let mut schema = NodeSchema::new(type_name, 1).field(FieldSpec::new("chunk", FieldKind::Text).optional());
    for p in &scanned.params {
        let max = if p.max > p.min { p.max } else { p.min + 1.0 };
        let mut spec = ParamSpec::new(lm_base::intern(&p.key), p.min, max, p.default.clamp(p.min, max));
        if p.integer || p.toggled {
            spec = spec.scale(Scale::Stepped);
        } else if p.logarithmic && p.min > 0.0 {
            spec = spec.scale(Scale::Log);
        }
        schema = schema.param(spec);
    }
    let channels = |n: usize| u32::try_from(n.clamp(1, 64)).unwrap_or(2);
    let ports = Ports { outputs: vec![Port::new("main", channels(layout_channels(scanned, 0).1))], sidechains: vec![] };
    let kind = if scanned.instrument { PluginKind::Instrument } else { PluginKind::Effect };
    let s = scanned.clone();
    let build = Arc::new(move |state: &Value, cx: &BuildContext<'_>| -> lm_base::Result<Box<dyn Processor>> {
        let chunk = state.get("chunk").and_then(Value::as_text).and_then(|h| unhex(h).ok());
        let params = state
            .get(PARAMS_KEY)
            .map(|p| p.entries().filter_map(|(k, v)| Some((k.to_owned(), v.as_real()? as f32))).collect())
            .unwrap_or_default();
        let (inputs, outputs) = layout_channels(&s, cx.channels);
        let req = LoadRequest {
            format: s.format.clone(),
            uri: s.uri.clone(),
            file: s.file.clone(),
            rate: cx.spec.sample_rate,
            max_block: cx.spec.max_block,
            inputs,
            outputs,
            sidechain: 0,
            chunk,
            params,
        };
        let (proxy, hosted) = start(&config, &req)?;
        if let Some(f) = &on_start {
            f(&s, hosted);
        }
        Ok(Box::new(proxy) as Box<dyn Processor>)
    });
    External {
        id,
        name: format!("{} ({})", scanned.name, scanned.format.to_uppercase()),
        kind,
        schema,
        ports,
        display: None,
        build,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scanned(instrument: bool, inputs: usize, outputs: usize) -> Scanned {
        Scanned {
            format: "test".into(),
            uri: "x".into(),
            name: "x".into(),
            instrument,
            inputs,
            outputs,
            midi_in: instrument,
            params: Vec::new(),
            latency: false,
            has_window: false,
            file: None,
        }
    }

    #[test]
    fn the_channels_decide_not_the_plugin() {
        // A mono effect on a stereo channel: two instances (2/2)
        assert_eq!(layout_channels(&scanned(false, 1, 1), 2), (2, 2));
        // On a mono channel it stays mono
        assert_eq!(layout_channels(&scanned(false, 1, 1), 1), (1, 1));
        // A stereo effect, a mono-in stereo-out one: the plugin's own
        assert_eq!(layout_channels(&scanned(false, 2, 2), 2), (2, 2));
        assert_eq!(layout_channels(&scanned(false, 1, 2), 2), (1, 2));
        // Instruments: a mono one is spread to two outputs, a stereo one stays
        assert_eq!(layout_channels(&scanned(true, 0, 1), 0), (0, 2));
        assert_eq!(layout_channels(&scanned(true, 0, 2), 0), (0, 2));
    }
}
