//! Readers (arquitetura-nova.md, piece 2, "Leitor"): typed, read-only access to a node that holds only its id and
//! reads the current root at every call. A node that left the tree answers `None`, never stale memory.

use std::sync::Arc;

use lm_state::{NodeId, ROOT, Value};

use crate::document::Document;
use crate::schemas::{MASTER, types};

fn ids_in(list: Option<&Value>) -> Vec<NodeId> {
    list.map(|l| l.items().filter_map(|i| i.get("id").and_then(Value::as_text)).filter_map(|t| NodeId::parse(t).ok()).collect())
        .unwrap_or_default()
}

fn text(v: Option<Value>, key: &str) -> Option<Arc<str>> {
    v?.get(key)?.as_shared_text().cloned()
}

fn real(v: Option<Value>, key: &str) -> Option<f64> {
    v?.get(key)?.as_real()
}

fn int(v: Option<Value>, key: &str) -> Option<i64> {
    v?.get(key)?.as_int()
}

fn boolean(v: Option<Value>, key: &str) -> Option<bool> {
    v?.get(key)?.as_bool()
}

fn reference(v: Option<Value>, key: &str) -> Option<NodeId> {
    NodeId::parse(v?.get(key)?.as_text()?).ok()
}

macro_rules! reader {
    ($name:ident, $type_name:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub NodeId);

        impl $name {
            pub fn id(self) -> NodeId {
                self.0
            }

            /// The node, if it exists and is of this type
            pub fn value(self, doc: &Document) -> Option<Value> {
                (doc.type_of(self.0) == Some($type_name)).then(|| doc.find(self.0)).flatten()
            }

            pub fn exists(self, doc: &Document) -> bool {
                doc.type_of(self.0) == Some($type_name)
            }
        }
    };
}

reader!(TrackRef, types::TRACK);
reader!(OutputRef, types::OUTPUT);
reader!(ItemRef, types::ITEM);
reader!(LaneRef, types::LANE);
reader!(AutomationRef, types::AUTOMATION);
reader!(PluginRef, types::PLUGIN);
reader!(ChannelRef, types::CHANNEL);
reader!(MasterRef, types::MASTER);
reader!(SendRef, types::SEND);

impl Document {
    /// The tracks, in order
    pub fn tracks(&self) -> Vec<TrackRef> {
        ids_in(self.root().get("tracks")).into_iter().map(TrackRef).collect()
    }

    /// The desk's channels, in order (the master apart)
    pub fn channels(&self) -> Vec<ChannelRef> {
        ids_in(self.root().get("mixer").and_then(|m| m.get("channels"))).into_iter().map(ChannelRef).collect()
    }

    pub fn master(&self) -> MasterRef {
        MasterRef(MASTER)
    }

    /// The root's own fields (the project's settings, transport and tempo map are read from here)
    pub fn root_field(&self, key: &str) -> Option<Value> {
        self.find(ROOT)?.get(key).cloned()
    }
}

impl TrackRef {
    pub fn name(self, doc: &Document) -> Option<Arc<str>> {
        text(self.value(doc), "name")
    }
    pub fn volume(self, doc: &Document) -> Option<f64> {
        real(self.value(doc), "volume")
    }
    pub fn mute(self, doc: &Document) -> Option<bool> {
        boolean(self.value(doc), "mute")
    }
    pub fn solo(self, doc: &Document) -> Option<bool> {
        boolean(self.value(doc), "solo")
    }
    pub fn is_folder(self, doc: &Document) -> Option<bool> {
        boolean(self.value(doc), "folder")
    }
    pub fn parent(self, doc: &Document) -> Option<TrackRef> {
        reference(self.value(doc), "parent").map(TrackRef)
    }
    pub fn instrument(self, doc: &Document) -> Option<PluginRef> {
        let v = self.value(doc)?;
        let id = v.get("instrument")?.get("id")?.as_text()?;
        NodeId::parse(id).ok().map(PluginRef)
    }
    pub fn outputs(self, doc: &Document) -> Vec<OutputRef> {
        ids_in(self.value(doc).as_ref().and_then(|v| v.get("outputs"))).into_iter().map(OutputRef).collect()
    }
    pub fn items(self, doc: &Document) -> Vec<ItemRef> {
        ids_in(self.value(doc).as_ref().and_then(|v| v.get("items"))).into_iter().map(ItemRef).collect()
    }
    pub fn automation(self, doc: &Document) -> Vec<AutomationRef> {
        ids_in(self.value(doc).as_ref().and_then(|v| v.get("automation"))).into_iter().map(AutomationRef).collect()
    }
}

impl OutputRef {
    pub fn port(self, doc: &Document) -> Option<Arc<str>> {
        text(self.value(doc), "port")
    }
    pub fn channels(self, doc: &Document) -> Option<i64> {
        int(self.value(doc), "channels")
    }
    /// The desk channel it goes to
    pub fn to(self, doc: &Document) -> Option<NodeId> {
        reference(self.value(doc), "to")
    }
}

impl ItemRef {
    pub fn kind(self, doc: &Document) -> Option<Arc<str>> {
        text(self.value(doc), "kind")
    }
    pub fn start(self, doc: &Document) -> Option<i64> {
        int(self.value(doc), "start")
    }
    pub fn length(self, doc: &Document) -> Option<i64> {
        int(self.value(doc), "length")
    }
    /// A note by its number (C1)
    pub fn note(self, doc: &Document, n: u64) -> Option<Value> {
        self.value(doc)?.get("notes")?.note(n).cloned()
    }
    pub fn note_count(self, doc: &Document) -> usize {
        self.value(doc).and_then(|v| v.get("notes").map(Value::len)).unwrap_or(0)
    }
    pub fn lanes(self, doc: &Document) -> Vec<LaneRef> {
        ids_in(self.value(doc).as_ref().and_then(|v| v.get("lanes"))).into_iter().map(LaneRef).collect()
    }
}

impl LaneRef {
    pub fn source(self, doc: &Document) -> Option<lm_state::Resource> {
        self.value(doc)?.get("source")?.as_resource().cloned()
    }
}

impl AutomationRef {
    pub fn target(self, doc: &Document) -> Option<(NodeId, Arc<str>)> {
        let target = self.value(doc)?.get("target")?.clone();
        Some((NodeId::parse(target.get("node")?.as_text()?).ok()?, target.get("param")?.as_shared_text()?.clone()))
    }
}

impl PluginRef {
    /// The plugin's id (`"tripleoscillator"`)
    pub fn plugin(self, doc: &Document) -> Option<Arc<str>> {
        text(self.value(doc), "plugin")
    }
    pub fn state(self, doc: &Document) -> Option<Value> {
        self.value(doc)?.get("state").cloned()
    }
    pub fn param(self, doc: &Document, key: &str) -> Option<f64> {
        doc.param(self.0, key)
    }
}

impl ChannelRef {
    pub fn name(self, doc: &Document) -> Option<Arc<str>> {
        text(self.value(doc), "name")
    }
    pub fn volume(self, doc: &Document) -> Option<f64> {
        real(self.value(doc), "volume")
    }
    pub fn pan(self, doc: &Document) -> Option<f64> {
        real(self.value(doc), "pan")
    }
    pub fn output(self, doc: &Document) -> Option<NodeId> {
        reference(self.value(doc), "output")
    }
    pub fn inserts(self, doc: &Document) -> Vec<PluginRef> {
        ids_in(self.value(doc).as_ref().and_then(|v| v.get("inserts"))).into_iter().map(PluginRef).collect()
    }
    pub fn sends(self, doc: &Document) -> Vec<SendRef> {
        ids_in(self.value(doc).as_ref().and_then(|v| v.get("sends"))).into_iter().map(SendRef).collect()
    }
    /// A parameter of the channel (`"rack.tube.drive"`)
    pub fn param(self, doc: &Document, key: &str) -> Option<f64> {
        doc.param(self.0, key)
    }
}

impl MasterRef {
    pub fn volume(self, doc: &Document) -> Option<f64> {
        real(self.value(doc), "volume")
    }
    pub fn inserts(self, doc: &Document) -> Vec<PluginRef> {
        ids_in(self.value(doc).as_ref().and_then(|v| v.get("inserts"))).into_iter().map(PluginRef).collect()
    }
    pub fn param(self, doc: &Document, key: &str) -> Option<f64> {
        doc.param(self.0, key)
    }
}

impl SendRef {
    pub fn to(self, doc: &Document) -> Option<NodeId> {
        reference(self.value(doc), "to")
    }
    pub fn amount(self, doc: &Document) -> Option<f64> {
        real(self.value(doc), "amount")
    }
    pub fn port(self, doc: &Document) -> Option<Arc<str>> {
        text(self.value(doc), "port")
    }
}
