//! The fake's parameter laws (`CorePort::param_range`), from the same tables as its schemas: what the session
//! answers from its compiled ranges and schemas (owner: agent B).

use lm_port::{NodeId, ParamRange, RangeScale, Value};

use crate::demo::PLUGINS;

/// The law of `key` on `node` (the node's tree as found, if any)
pub(crate) fn param_range(node_tree: Option<&Value>, node: NodeId, key: &str) -> Option<ParamRange> {
    let gain = ParamRange { min: 0.0, max: 4.0, scale: RangeScale::Gain };
    match (node.kind, key) {
        // a track's or a channel's fader and a send's amount: the fader's gain law
        (b't' | b'm', "volume") | (_, "amount") => Some(gain),
        (b'm', "pan") => Some(ParamRange::linear(-1.0, 1.0)),
        (b'p', key) => {
            let plugin = node_tree?.get("plugin")?.as_text()?;
            let key = key.strip_prefix("state.params.").unwrap_or(key);
            PLUGINS
                .iter()
                .find(|(id, ..)| *id == plugin)?
                .2
                .iter()
                .find(|(k, ..)| *k == key)
                .map(|(_, min, max, _)| ParamRange { min: *min as f32, max: *max as f32, scale: RangeScale::Linear })
        }
        _ => None,
    }
}
