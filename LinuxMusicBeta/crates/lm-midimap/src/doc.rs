//! The document form of a mapping (interfaces.md, I-21.2): `project.midiMap` and the machine preferences' `midiMap`
//! hold `midi.mapping` nodes (schemas in `lm-doc`); this reads and writes them.

use std::sync::Arc;

use lm_base::{Error, Result};
use lm_state::{NodeId, PathStep, Value};

use crate::feedback::Ring;
use crate::message::Message;
use crate::transform::{Button, Mode, Takeover, Transform};

/// What a selection-following target follows
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelRole {
    /// The selected track, plus `offset` tracks
    Track,
    /// The selected desk channel, plus `offset`
    Channel,
    /// The focused plugin
    Plugin,
    /// The track at the controller's bank start plus `offset` (a control surface's strips; the bank moves with
    /// `midi.bank`)
    TrackBank,
    /// The same for desk channels
    ChannelBank,
}

impl SelRole {
    pub fn name(self) -> &'static str {
        match self {
            SelRole::Track => "track",
            SelRole::Channel => "channel",
            SelRole::Plugin => "plugin",
            SelRole::TrackBank => "trackBank",
            SelRole::ChannelBank => "channelBank",
        }
    }

    pub fn from_name(name: &str) -> Option<SelRole> {
        Some(match name {
            "track" => SelRole::Track,
            "channel" => SelRole::Channel,
            "plugin" => SelRole::Plugin,
            "trackBank" => SelRole::TrackBank,
            "channelBank" => SelRole::ChannelBank,
            _ => return None,
        })
    }
}

/// A fixed node, or one that follows the selection (a control surface works on "the selected track"). `offset`
/// counts from the selected one (plus the bank) in the order of the tracks or channels
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NodeRef {
    Id(NodeId),
    Selected { role: SelRole, offset: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum MapTarget {
    /// Any parameter: plugin, rack section (`rack.tube.drive`), fader (`volume`), send (`amount`), pan
    Param { node: NodeRef, key: String },
    /// A field: switches and choices (mute, solo, arm, monitor, a wave shape)
    Field { node: NodeRef, path: Vec<PathStep> },
    /// A registered action; `value_into` puts the control's value into its parameters
    Action { id: Arc<str>, params: Value, value_into: Option<Vec<PathStep>> },
}

impl MapTarget {
    pub fn node(&self) -> Option<&NodeRef> {
        match self {
            MapTarget::Param { node, .. } | MapTarget::Field { node, .. } => Some(node),
            MapTarget::Action { .. } => None,
        }
    }
}

/// Where a mapping is kept
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapScope {
    Project,
    Machine,
}

/// A source as the document has it: a port alias (`""` = any controller port), a channel (or any), a message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocSource {
    pub port: Arc<str>,
    pub channel: Option<u8>,
    pub message: Message,
}

/// One mapping (`midi.mapping`)
#[derive(Clone, Debug, PartialEq)]
pub struct Mapping {
    /// Its node id (`k:N`), when it is in a document
    pub id: Option<NodeId>,
    pub source: DocSource,
    pub touch: Option<DocSource>,
    pub target: MapTarget,
    pub transform: Transform,
    pub feedback: bool,
    pub consume: bool,
    /// Feedback to another message than the source (a Mackie Control V-pot's ring), with the ring's style
    pub feedback_to: Option<(Message, Ring)>,
}

fn bad(what: &'static str) -> Error {
    Error::new("midimap.bad_mapping").arg("field", what)
}

fn text<'a>(v: &'a Value, key: &'static str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_text).ok_or_else(|| bad(key))
}

fn int(v: &Value, key: &'static str) -> Result<i64> {
    v.get(key).and_then(Value::as_int).ok_or_else(|| bad(key))
}

fn real_or(v: &Value, key: &'static str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_real).unwrap_or(default)
}

fn keys(v: &Value) -> Result<Vec<PathStep>> {
    v.items().map(|s| s.as_text().map(PathStep::key).ok_or_else(|| bad("path"))).collect()
}

fn keys_value(path: &[PathStep]) -> Value {
    Value::list(path.iter().map(|s| match s {
        PathStep::Key(k) => Value::text(k.clone()),
        PathStep::Index(i) => Value::text(i.to_string()),
        PathStep::Note(n) => Value::text(n.to_string()),
    }))
}

fn real(x: f64) -> Value {
    Value::real(x).unwrap_or(Value::int(0))
}

impl DocSource {
    pub fn from_value(v: &Value) -> Result<DocSource> {
        let port: Arc<str> = v.get("port").and_then(Value::as_shared_text).cloned().unwrap_or_else(|| Arc::from(""));
        let channel = int(v, "channel").unwrap_or(-1);
        let channel = if channel < 0 { None } else { Some(u8::try_from(channel.min(15)).map_err(|_| bad("channel"))?) };
        let number = u16::try_from(int(v, "number").unwrap_or(0)).map_err(|_| bad("number"))?;
        let message = Message::from_parts(text(v, "message")?, number).ok_or_else(|| bad("message"))?;
        Ok(DocSource { port, channel, message })
    }

    pub fn to_value(&self) -> Value {
        Value::object([
            ("port", Value::text(self.port.clone())),
            ("channel", Value::int(self.channel.map_or(-1, i64::from))),
            ("message", Value::text(self.message.kind_name())),
            ("number", Value::int(i64::from(self.message.number()))),
        ])
    }
}

/// The source kind of a mapping that listens to an OSC address instead of a MIDI message (`message: "osc"`): the
/// source's `port` holds the address (`decisoes-autonomas.md`, 26/11, item 5). The MIDI map skips these
pub const OSC_MESSAGE: &str = "osc";

/// The OSC address of a mapping node, when its source is one
pub fn osc_address(v: &Value) -> Option<&str> {
    let s = v.get("source")?;
    if s.get("message")?.as_text()? != OSC_MESSAGE {
        return None;
    }
    s.get("port")?.as_text().filter(|a| a.starts_with('/'))
}

/// What an OSC alias drives: its target and the transform of the value (the source's MIDI fields do not apply)
#[derive(Clone, Debug, PartialEq)]
pub struct OscMapping {
    pub address: Arc<str>,
    pub target: MapTarget,
    pub transform: Transform,
}

/// A MIDI source in the place of an OSC one, to read the rest of the node with the MIDI reader
fn midi_stand_in() -> Value {
    Value::object([
        ("port", Value::text("")),
        ("channel", Value::int(-1)),
        ("message", Value::text("cc")),
        ("number", Value::int(0)),
    ])
}

impl OscMapping {
    /// Reads a mapping node whose source is an OSC address (the same node as a MIDI mapping: target and transform)
    pub fn from_value(v: &Value) -> Result<OscMapping> {
        let address = osc_address(v).ok_or_else(|| bad("source"))?;
        let m = Mapping::from_value(&v.with("source", midi_stand_in()))?;
        Ok(OscMapping { address: Arc::from(address), target: m.target, transform: m.transform })
    }

    /// The mapping node (without `id`) for the document: the source holds the address
    pub fn to_value(&self) -> Value {
        let m = Mapping {
            id: None,
            source: DocSource { port: self.address.clone(), channel: None, message: Message::Cc { number: 0 } },
            touch: None,
            target: self.target.clone(),
            transform: self.transform,
            feedback: true,
            consume: false,
            feedback_to: None,
        };
        m.to_value().with(
            "source",
            Value::object([
                ("port", Value::text(self.address.clone())),
                ("channel", Value::int(-1)),
                ("message", Value::text(OSC_MESSAGE)),
                ("number", Value::int(0)),
            ]),
        )
    }
}

impl Mapping {
    pub fn from_value(v: &Value) -> Result<Mapping> {
        let id = match v.get("id").and_then(Value::as_text) {
            Some(t) => Some(NodeId::parse(t)?),
            None => None,
        };
        let source = DocSource::from_value(v.get("source").ok_or_else(|| bad("source"))?)?;
        let touch = match v.get("touch") {
            Some(t) if !t.is_null() => Some(DocSource::from_value(t)?),
            _ => None,
        };
        let t = v.get("target").ok_or_else(|| bad("target"))?;
        let node = || -> Result<NodeRef> {
            match t.get("follow").and_then(Value::as_text).unwrap_or("none") {
                "none" => {
                    let id = t.get("node").and_then(Value::as_text).ok_or_else(|| bad("target.node"))?;
                    Ok(NodeRef::Id(NodeId::parse(id)?))
                }
                role => Ok(NodeRef::Selected {
                    role: SelRole::from_name(role).ok_or_else(|| bad("target.follow"))?,
                    offset: u32::try_from(int(t, "offset").unwrap_or(0)).map_err(|_| bad("target.offset"))?,
                }),
            }
        };
        let target = match text(t, "kind")? {
            "param" => MapTarget::Param { node: node()?, key: text(t, "param")?.to_owned() },
            "field" => {
                MapTarget::Field { node: node()?, path: keys(t.get("path").ok_or_else(|| bad("target.path"))?)? }
            }
            "action" => MapTarget::Action {
                id: Arc::from(text(t, "action")?),
                params: t.get("params").cloned().unwrap_or_else(Value::empty_object),
                value_into: match t.get("valueInto") {
                    Some(p) if !p.is_null() => Some(keys(p)?),
                    _ => None,
                },
            },
            _ => return Err(bad("target.kind")),
        };
        let x = v.get("transform").cloned().unwrap_or_else(Value::empty_object);
        let d = Transform::default();
        let name = |key: &'static str, default: &'static str| x.get(key).and_then(Value::as_text).unwrap_or(default);
        let transform = Transform {
            min: real_or(&x, "min", f64::from(d.min)) as f32,
            max: real_or(&x, "max", f64::from(d.max)) as f32,
            invert: x.get("invert").and_then(Value::as_bool).unwrap_or(false),
            curve: real_or(&x, "curve", 0.0) as f32,
            mode: Mode::from_name(name("mode", "absolute")).ok_or_else(|| bad("transform.mode"))?,
            step: real_or(&x, "step", f64::from(d.step)) as f32,
            takeover: Takeover::from_name(name("takeover", "pickup")).ok_or_else(|| bad("transform.takeover"))?,
            button: Button::from_name(name("button", "toggle")).ok_or_else(|| bad("transform.button"))?,
        };
        // (a value past f32 range became infinite in the cast, and an infinite range gives NaN in the transform)
        for (value, field) in [
            (transform.min, "transform.min"),
            (transform.max, "transform.max"),
            (transform.curve, "transform.curve"),
            (transform.step, "transform.step"),
        ] {
            if !value.is_finite() {
                return Err(bad(field));
            }
        }
        let feedback_to = match v.get("feedbackTo") {
            Some(f) if !f.is_null() => {
                let s = DocSource::from_value(f)?;
                let ring = match v.get("ring").and_then(Value::as_text) {
                    Some(r) => Ring::from_name(r).ok_or_else(|| bad("ring"))?,
                    None => Ring::None,
                };
                Some((s.message, ring))
            }
            _ => None,
        };
        Ok(Mapping {
            id,
            source,
            touch,
            target,
            transform,
            feedback: v.get("feedback").and_then(Value::as_bool).unwrap_or(true),
            consume: v.get("consume").and_then(Value::as_bool).unwrap_or(true),
            feedback_to,
        })
    }

    /// The node, without `id` (the document gives it on insert) unless it has one
    pub fn to_value(&self) -> Value {
        let (follow, offset, node) = match self.target.node() {
            Some(NodeRef::Id(id)) => ("none", 0, Value::text(id.to_string())),
            Some(NodeRef::Selected { role, offset }) => (role.name(), *offset, Value::NULL),
            None => ("none", 0, Value::NULL),
        };
        let mut target =
            Value::object([("follow", Value::text(follow)), ("offset", Value::int(i64::from(offset))), ("node", node)]);
        target = match &self.target {
            MapTarget::Param { key, .. } => {
                target.with("kind", Value::text("param")).with("param", Value::text(key.as_str()))
            }
            MapTarget::Field { path, .. } => target.with("kind", Value::text("field")).with("path", keys_value(path)),
            MapTarget::Action { id, params, value_into } => target
                .with("kind", Value::text("action"))
                .with("action", Value::text(id.clone()))
                .with("params", params.clone())
                .with("valueInto", value_into.as_deref().map_or(Value::NULL, keys_value)),
        };
        let t = &self.transform;
        let transform = Value::object([
            ("min", real(f64::from(t.min))),
            ("max", real(f64::from(t.max))),
            ("invert", Value::bool(t.invert)),
            ("curve", real(f64::from(t.curve))),
            ("mode", Value::text(t.mode.name())),
            ("step", real(f64::from(t.step))),
            ("takeover", Value::text(t.takeover.name())),
            ("button", Value::text(t.button.name())),
        ]);
        let mut v = Value::object([
            ("source", self.source.to_value()),
            ("touch", self.touch.as_ref().map_or(Value::NULL, DocSource::to_value)),
            ("target", target),
            ("transform", transform),
            ("feedback", Value::bool(self.feedback)),
            ("consume", Value::bool(self.consume)),
        ]);
        if let Some((m, ring)) = self.feedback_to {
            let s = DocSource { port: Arc::from(""), channel: self.source.channel, message: m };
            v = v.with("feedbackTo", s.to_value()).with("ring", Value::text(ring.name()));
        }
        if let Some(id) = self.id {
            v = v.with("id", Value::text(id.to_string()));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::Encoding;

    #[test]
    fn a_transform_that_overflows_f32_is_refused() {
        let m = Mapping {
            id: None,
            source: DocSource { port: Arc::from(""), channel: None, message: Message::Cc { number: 1 } },
            touch: None,
            target: MapTarget::Param { node: NodeRef::Id(NodeId::parse("k:3").unwrap()), key: "volume".into() },
            transform: Transform::default(),
            feedback: true,
            consume: false,
            feedback_to: None,
        };
        let node = m.to_value();
        assert!(Mapping::from_value(&node).is_ok());
        let bad_transform = Value::object([("max", Value::real(1.0e39).unwrap())]);
        assert!(Mapping::from_value(&node.with("transform", bad_transform)).is_err());
    }

    #[test]
    fn round_trip() {
        let m = Mapping {
            id: Some(NodeId::parse("k:3").unwrap()),
            source: DocSource { port: Arc::from("Controlador 1"), channel: Some(2), message: Message::Cc14 { msb: 7 } },
            touch: Some(DocSource { port: Arc::from(""), channel: None, message: Message::Note { key: 0x68 } }),
            target: MapTarget::Param {
                node: NodeRef::Selected { role: SelRole::Track, offset: 3 },
                key: "volume".into(),
            },
            transform: Transform {
                min: 0.25,
                max: 0.75,
                invert: true,
                curve: 0.5,
                mode: Mode::Relative(Encoding::Offset64),
                step: 0.125,
                takeover: Takeover::Scale,
                button: Button::Momentary,
            },
            feedback: false,
            consume: false,
            feedback_to: Some((Message::Cc { number: 0x30 }, Ring::Wrap)),
        };
        assert_eq!(Mapping::from_value(&m.to_value()).unwrap(), m);
        let a = Mapping {
            id: None,
            target: MapTarget::Action {
                id: Arc::from("preset.select"),
                params: Value::object([("node", Value::text("p:4"))]),
                value_into: Some(vec![PathStep::key("index")]),
            },
            feedback_to: None,
            ..m.clone()
        };
        assert_eq!(Mapping::from_value(&a.to_value()).unwrap(), a);
        let f = Mapping {
            target: MapTarget::Field {
                node: NodeRef::Id(NodeId::parse("t:1").unwrap()),
                path: vec![PathStep::key("mute")],
            },
            ..a
        };
        assert_eq!(Mapping::from_value(&f.to_value()).unwrap(), f);
    }

    #[test]
    fn an_osc_alias_is_a_mapping_node_with_an_address_as_its_source() {
        let o = OscMapping {
            address: Arc::from("/mine/fader1"),
            target: MapTarget::Param { node: NodeRef::Id(NodeId::parse("t:1").unwrap()), key: "volume".into() },
            transform: Transform { min: 0.0, max: 0.5, ..Transform::default() },
        };
        let v = o.to_value();
        assert_eq!(osc_address(&v), Some("/mine/fader1"));
        assert_eq!(OscMapping::from_value(&v).unwrap(), o);
        assert!(Mapping::from_value(&v).is_err(), "the MIDI reader does not take it");
        let midi = Mapping {
            id: None,
            source: DocSource { port: Arc::from(""), channel: None, message: Message::Cc { number: 7 } },
            touch: None,
            target: MapTarget::Action {
                id: Arc::from("transport.play"),
                params: Value::empty_object(),
                value_into: None,
            },
            transform: Transform::default(),
            feedback: true,
            consume: true,
            feedback_to: None,
        };
        assert_eq!(osc_address(&midi.to_value()), None);
    }
}
