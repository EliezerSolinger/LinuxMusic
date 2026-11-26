//! `plugin.load_into_field {node, field, path | bytes, name?}` in the fake core: what the real one does to the
//! document (a resource field gets the file's resource with its name; a wave field gets a wave of its length), in one
//! undo step. The fake decodes no audio: a wave is made from the bytes as they come.

use lm_base::{Label, Result};
use lm_port::{CorePort, EditOp, NodeId, Pending};
use lm_state::{FieldKind, Resource, Value};

use crate::{FakePort, err};

pub(crate) fn load_into_field(p: &mut FakePort, params: &Value) -> Result<()> {
    let node = params.get("node").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
    let field = params.get("field").and_then(Value::as_text);
    let (Some(node), Some(field)) = (node, field) else { return Err(err("port.params")) };
    let found = CorePort::find(p, node).ok_or_else(|| err("port.params"))?;
    let (plugin_node, prefix): (Value, &[&str]) = match found.get("instrument") {
        Some(i) => (i.clone(), &["instrument", "state"]),
        None => (found, &["state"]),
    };
    let plugin = plugin_node.get("plugin").and_then(Value::as_text).unwrap_or("").to_owned();
    let schema = CorePort::plugin_schema(p, &plugin).ok_or_else(|| err("port.params"))?;
    let spec = schema.find_field(field).ok_or_else(|| err("plugin.load.no_field"))?;
    let bytes: Vec<u8> = match (params.get("bytes"), params.get("path").and_then(Value::as_text)) {
        (Some(list), _) => list.items().filter_map(|v| v.as_int().and_then(|b| u8::try_from(b).ok())).collect(),
        (None, Some(path)) => std::fs::read(path).map_err(|_| err("plugin.load.read"))?,
        _ => return Err(err("port.params")),
    };
    let name = params.get("name").and_then(Value::as_text).map(str::to_owned).or_else(|| {
        let path = params.get("path").and_then(Value::as_text)?;
        std::path::Path::new(path).file_name().and_then(|n| n.to_str()).map(str::to_owned)
    });
    let value = match &spec.kind {
        FieldKind::Resource(kind) => {
            let r = Resource::of_content(kind, &bytes)?;
            Value::resource(r.with_name(name.as_deref().unwrap_or("")))
        }
        FieldKind::List(inner) if matches!(**inner, FieldKind::Real { .. }) => {
            let length = spec.default.as_ref().map_or(0, |v| v.items().count()).max(1);
            let at = |i: usize| bytes.get(i * bytes.len() / length).map_or(0.0, |b| f64::from(*b) / 127.5 - 1.0);
            Value::list((0..length).map(|i| Value::real(at(i)).unwrap_or_default()))
        }
        _ => return Err(err("plugin.load.field_kind")),
    };
    let mut path: Vec<&str> = prefix.to_vec();
    path.push(field);
    match CorePort::apply(p, Label::new("Load a file"), vec![EditOp::set(node, &path, value)]) {
        Pending::Ready(r) => r.map(|_| ()),
        Pending::Waiting(_) => Ok(()),
    }
}
