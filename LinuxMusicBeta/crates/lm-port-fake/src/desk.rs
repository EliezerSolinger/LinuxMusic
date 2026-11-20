//! The fake's desk actions (agent A): grouping channels (P-18) and the loudness meter's reset

use lm_port::{Error, Label, NodeId, PathStep, ROOT, Result, Value};

use crate::FakePort;

impl FakePort {
    /// `mixer.group {channels: [ids], name?}`: a new group channel before the first of them, their outputs into
    /// it, in one transaction (one undo step)
    pub(crate) fn mixer_group(&mut self, params: &Value) -> Result<()> {
        let ids: Vec<NodeId> = params
            .get("channels")
            .map(|l| l.items().filter_map(|v| v.as_text().and_then(|t| NodeId::parse(t).ok())).collect())
            .unwrap_or_default();
        if ids.is_empty() {
            return Err(Error::new("port.params"));
        }
        let list: Vec<NodeId> = self
            .doc
            .root()
            .get("mixer")
            .and_then(|m| m.get("channels"))
            .map(|l| l.items().filter_map(crate::lm_id).collect())
            .unwrap_or_default();
        let index = ids
            .iter()
            .filter_map(|id| list.iter().position(|c| c == id))
            .min()
            .ok_or_else(|| Error::new("port.no_such_node"))?;
        let groups = list.len();
        let name =
            params.get("name").and_then(Value::as_text).map_or_else(|| format!("Grupo {}", groups + 1), str::to_owned);
        self.doc
            .transact(Label::new("Group the channels"), |e| {
                let group =
                    e.insert(ROOT, "mixer.channels", index, Value::object([("name", Value::text(name.as_str()))]))?;
                for id in &ids {
                    e.set(*id, &[PathStep::key("output")], Value::text(group.to_string()))?;
                }
                Ok(())
            })
            .map(|_| ())
    }
}
