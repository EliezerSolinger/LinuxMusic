//! The plugin registry (Parte 3: built at start-up, then immutable inside an `Arc`, so it can go to any `Operation`,
//! T3). It lists the descriptors, gives `lm-doc` its [`lm_doc::PortsFn`] and the plugins' schemas, and builds a
//! processor from a node's state.

use std::collections::BTreeMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use lm_base::{Error, Result};
use lm_doc::{Ports, PortsFn};
use lm_render::Processor;
use lm_state::{NodeSchema, PARAMS_KEY, ParamSpec, SchemaRegistry, Value};

use crate::descriptor::{BuildContext, Descriptor, PluginKind, is_valid_plugin_id, plugin_type};

/// How an external plugin is built: the state (complete and valid) and the context, to a processor (for a
/// third-party plugin, a proxy of a helper process, I-9.6)
/// The field of a third-party plugin's own opaque state (arquitetura-nova.md 7a). The plugin changes it by itself
/// (`PluginRequest::StateChanged`), so a change of it alone never builds the processor again: the session gives the
/// running plugin the new value (an undo, a preset)
pub const CHUNK_KEY: &str = "chunk";

pub type ExternalBuild = Arc<dyn Fn(&Value, &BuildContext<'_>) -> Result<Box<dyn Processor>> + Send + Sync>;

/// A plugin known only at run time: a third-party plugin the scan found (I-9.6). Its id, schema and ports are made
/// from what the scan saw; its build starts the helper process. The `&'static` names of its schema are leaked once
/// per plugin per start (bounded by the scan)
#[derive(Clone)]
pub struct External {
    /// `"lv2-<16 hex>"`, `"clap-…"`, `"vst3-…"`, `"ladspa-…"`: a valid plugin id
    pub id: String,
    /// Shown as is: a third-party plugin's name is not translated
    pub name: String,
    pub kind: PluginKind,
    /// `"plugin:<id>"`, with the plugin's parameters
    pub schema: NodeSchema,
    /// Fixed for the plugin: an external plugin's ports do not depend on its state
    pub ports: Ports,
    pub build: ExternalBuild,
}

impl fmt::Debug for External {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("External").field("id", &self.id).field("name", &self.name).field("kind", &self.kind).finish()
    }
}

enum Source {
    Static(&'static Descriptor),
    External(Arc<External>),
}

struct Entry {
    source: Source,
    type_name: String,
    /// The main schema (its parameters, by index: the index a processor's `Param` events carry)
    schema: NodeSchema,
}

impl Entry {
    fn kind(&self) -> PluginKind {
        match &self.source {
            Source::Static(d) => d.kind,
            Source::External(e) => e.kind,
        }
    }
}

/// One plugin of the registry, as the lists show it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryInfo<'a> {
    pub id: &'a str,
    /// A translation key for a plugin of the program; the plugin's own name for a third-party one
    pub name: &'a str,
    pub kind: PluginKind,
    pub third_party: bool,
}

/// Every plugin of the program, immutable once made
pub struct Registry {
    plugins: BTreeMap<String, Entry>,
    /// The plugins' types only (and the types they nest)
    schemas: SchemaRegistry,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry").field("plugins", &self.plugins.keys().collect::<Vec<_>>()).finish()
    }
}

fn definition(id: &str, problem: &'static str) -> Error {
    Error::new("plugin.definition").arg("plugin", id).arg("problem", problem)
}

/// Checks the ports a plugin gives for a state: at least one output for an instrument, keys valid and unique,
/// 1 to 64 channels each (the document's rule, M1)
pub fn check_ports(id: &str, kind: PluginKind, ports: &Ports) -> Result<()> {
    if kind == PluginKind::Instrument && ports.outputs.is_empty() {
        return Err(definition(id, "no_outputs"));
    }
    let all = ports.outputs.iter().map(|p| (p, "output")).chain(ports.sidechains.iter().map(|p| (p, "sidechain")));
    let mut seen = std::collections::BTreeSet::new();
    for (port, _) in all {
        let key: &str = &port.key;
        let valid = !key.is_empty()
            && key.len() <= 64
            && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
        if !valid {
            return Err(definition(id, "port_key").arg("port", key));
        }
        if !seen.insert(key.to_owned()) {
            return Err(definition(id, "duplicate_port").arg("port", key));
        }
        if !(1..=64).contains(&port.channels) {
            return Err(definition(id, "port_channels").arg("port", key));
        }
    }
    Ok(())
}

impl Registry {
    /// A registry of these plugins. Every descriptor is checked: its id, its schemas (names, definitions, nested types,
    /// defaults), and the ports of its default state. Keys `plugin.definition` (with `problem`) and the schema keys
    pub fn new(descriptors: &[&'static Descriptor]) -> Result<Arc<Self>> {
        Self::with_external(descriptors, Vec::new())
    }

    /// The program's plugins and the external ones the scan found. Each external one is checked like a descriptor
    /// (id, schema name and definition, ports)
    pub fn with_external(descriptors: &[&'static Descriptor], external: Vec<External>) -> Result<Arc<Self>> {
        let mut plugins: BTreeMap<String, Entry> = BTreeMap::new();
        let mut schemas = SchemaRegistry::new();
        for &d in descriptors {
            if !is_valid_plugin_id(d.id) {
                return Err(definition(d.id, "id"));
            }
            if plugins.contains_key(d.id) {
                return Err(definition(d.id, "duplicate_id"));
            }
            let type_name = plugin_type(d.id);
            let all = (d.schemas)();
            let Some(main) = all.first().cloned() else { return Err(definition(d.id, "no_schema")) };
            if main.type_name != type_name {
                return Err(definition(d.id, "schema_name").arg("type", main.type_name));
            }
            let nested_prefix = format!("{type_name}.");
            for s in &all[1..] {
                if !s.type_name.starts_with(&nested_prefix) {
                    return Err(definition(d.id, "nested_schema_name").arg("type", s.type_name));
                }
            }
            for s in all {
                schemas.add(s)?;
            }
            plugins.insert(d.id.to_owned(), Entry { source: Source::Static(d), type_name, schema: main });
        }
        for e in external {
            if !is_valid_plugin_id(&e.id) {
                return Err(definition(&e.id, "id"));
            }
            if plugins.contains_key(&e.id) {
                return Err(definition(&e.id, "duplicate_id"));
            }
            let type_name = plugin_type(&e.id);
            if e.schema.type_name != type_name {
                return Err(definition(&e.id, "schema_name").arg("type", e.schema.type_name));
            }
            check_ports(&e.id, e.kind, &e.ports)?;
            schemas.add(e.schema.clone())?;
            let schema = e.schema.clone();
            plugins.insert(e.id.clone(), Entry { source: Source::External(Arc::new(e)), type_name, schema });
        }
        schemas.check_complete()?;
        let registry = Self { plugins, schemas };
        for (id, entry) in &registry.plugins {
            if let Source::Static(d) = &entry.source {
                let defaults = registry.defaults(id)?;
                let ports = (d.ports)(&defaults);
                check_ports(id, d.kind, &ports)?;
            }
        }
        Ok(Arc::new(registry))
    }

    /// A plugin of the program (an external one has no descriptor: see [`Registry::kind`], [`Registry::entries`])
    pub fn get(&self, id: &str) -> Option<&'static Descriptor> {
        match &self.plugins.get(id)?.source {
            Source::Static(d) => Some(*d),
            Source::External(_) => None,
        }
    }

    /// Every descriptor of the program's plugins, by id
    pub fn descriptors(&self) -> impl Iterator<Item = &'static Descriptor> + '_ {
        self.plugins.values().filter_map(|e| match &e.source {
            Source::Static(d) => Some(*d),
            Source::External(_) => None,
        })
    }

    /// Every plugin, the program's and the external ones, by id
    pub fn entries(&self) -> impl Iterator<Item = EntryInfo<'_>> + '_ {
        self.plugins.iter().map(|(id, e)| match &e.source {
            Source::Static(d) => EntryInfo { id, name: d.name, kind: d.kind, third_party: false },
            Source::External(x) => EntryInfo { id, name: &x.name, kind: x.kind, third_party: true },
        })
    }

    /// What a plugin is, for both kinds of entry
    pub fn kind(&self, id: &str) -> Option<PluginKind> {
        self.plugins.get(id).map(Entry::kind)
    }

    /// A plugin's main schema
    pub fn schema(&self, id: &str) -> Option<&NodeSchema> {
        self.plugins.get(id).map(|e| &e.schema)
    }

    /// Whether a plugin is a third-party one (out of process)
    pub fn is_external(&self, id: &str) -> bool {
        self.plugins.get(id).is_some_and(|e| matches!(e.source, Source::External(_)))
    }

    /// The plugins' schemas alone (their types and the types they nest)
    pub fn schemas(&self) -> &SchemaRegistry {
        &self.schemas
    }

    /// Adds every plugin's schemas to a registry (the project's, `lm_doc::register_project_schemas`)
    pub fn register_schemas(&self, registry: &mut SchemaRegistry) -> Result<()> {
        for entry in self.plugins.values() {
            match &entry.source {
                Source::Static(d) => {
                    for schema in (d.schemas)() {
                        registry.add(schema)?;
                    }
                }
                Source::External(e) => registry.add(e.schema.clone())?,
            }
        }
        Ok(())
    }

    /// The whole schema registry of a project: the project's types and every plugin's, checked complete
    pub fn project_schemas(&self) -> Result<SchemaRegistry> {
        let mut registry = SchemaRegistry::new();
        lm_doc::register_project_schemas(&mut registry)?;
        self.register_schemas(&mut registry)?;
        registry.check_complete()?;
        Ok(registry)
    }

    /// `lm-doc`'s ports function (M1): the ports of a plugin for a state, `None` for a plugin this registry does not
    /// know or a state that does not validate
    pub fn ports_fn(self: &Arc<Self>) -> PortsFn {
        let registry = Arc::clone(self);
        Arc::new(move |id: &str, state: &Value| registry.ports(id, state))
    }

    /// The ports of a plugin for a state (completed with the defaults first)
    pub fn ports(&self, id: &str, state: &Value) -> Option<Ports> {
        let entry = self.plugins.get(id)?;
        let complete = self.complete(id, state).ok()?;
        self.schemas.validate(&entry.type_name, &complete).ok()?;
        Some(match &entry.source {
            Source::Static(d) => (d.ports)(&complete),
            Source::External(e) => e.ports.clone(),
        })
    }

    /// The parameters of a plugin, in index order: the index of a parameter is its position here, the index the
    /// compiler binds a slot to and the processor's `Param` events carry
    pub fn params(&self, id: &str) -> Option<&[ParamSpec]> {
        self.plugins.get(id).map(|e| e.schema.params.as_slice())
    }

    /// The index of a parameter by its key or an old key of it (C5)
    pub fn param_index(&self, id: &str, key: &str) -> Option<u32> {
        let params = self.params(id)?;
        let i = params
            .iter()
            .position(|p| p.key == key)
            .or_else(|| params.iter().position(|p| p.aliases.contains(&key)))?;
        u32::try_from(i).ok()
    }

    /// The current schema version of a plugin's state
    pub fn version(&self, id: &str) -> Option<u32> {
        self.plugins.get(id).map(|e| e.schema.version)
    }

    /// A new state with every default (frozen per version, U29)
    pub fn defaults(&self, id: &str) -> Result<Value> {
        let entry = self.plugins.get(id).ok_or_else(|| unknown(id))?;
        self.schemas.defaults(&entry.type_name)
    }

    /// The state with every field and parameter it leaves out at its default. Not validated
    pub fn complete(&self, id: &str, state: &Value) -> Result<Value> {
        let mut full = self.defaults(id)?;
        for (key, v) in state.entries() {
            if key == PARAMS_KEY && v.is_object() {
                let mut params = full.get(PARAMS_KEY).cloned().unwrap_or_else(Value::empty_object);
                for (p, x) in v.entries() {
                    params = params.with(p, x.clone());
                }
                full = full.with(PARAMS_KEY, params);
            } else {
                full = full.with(key, v.clone());
            }
        }
        Ok(full)
    }

    /// Brings a state saved by version `from` up to date and validates it (opening a project; the document does
    /// the same through the project's registry)
    pub fn load(&self, id: &str, from: u32, state: Value) -> Result<Value> {
        let entry = self.plugins.get(id).ok_or_else(|| unknown(id))?;
        self.schemas.load(&entry.type_name, from, state)
    }

    /// Builds a processor for a state: completes it with the defaults, validates it with the plugin's schema
    /// (`state.schema.*`), then runs the plugin's build inside `catch_unwind` (H14: a panic becomes the error
    /// `plugin.build.panicked`, never a crash). Runs in an `Operation` (it may take long and allocates)
    pub fn build(&self, id: &str, state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
        let entry = self.plugins.get(id).ok_or_else(|| unknown(id))?;
        let complete = self.complete(id, state)?;
        self.schemas.validate(&entry.type_name, &complete).map_err(|e| e.arg("plugin", id))?;
        if cx.spec.sample_rate == 0 || cx.spec.max_block == 0 {
            return Err(Error::new("plugin.build.spec").arg("plugin", id));
        }
        if entry.kind() == PluginKind::Effect && cx.channels == 0 {
            return Err(Error::new("plugin.build.channels").arg("plugin", id));
        }
        let built = match &entry.source {
            Source::Static(d) => {
                let build = d.build;
                catch_unwind(AssertUnwindSafe(|| build(&complete, cx)))
            }
            Source::External(e) => {
                let build = Arc::clone(&e.build);
                catch_unwind(AssertUnwindSafe(|| build(&complete, cx)))
            }
        };
        match built {
            Ok(result) => result.map_err(|e| e.arg("plugin", id)),
            Err(_) => Err(Error::new("plugin.build.panicked").arg("plugin", id)),
        }
    }
}

fn unknown(id: &str) -> Error {
    Error::new("plugin.unknown").arg("plugin", id)
}

#[cfg(test)]
mod external_tests {
    use super::*;
    use lm_doc::Port;
    use lm_render::{Adopted, EventKind, ProcessContext, ProcessSpec};
    use lm_state::ParamSpec;

    /// A gain effect, as a scanned plugin would be: in place, one parameter
    struct Gain(f32);

    impl Processor for Gain {
        fn process(&mut self, cx: &mut ProcessContext<'_>) {
            for e in cx.events {
                if let EventKind::Param { index: 0, value } = e.kind {
                    self.0 = value;
                }
            }
            for ch in 0..cx.buffers.outputs() {
                for x in cx.buffers.in_out(ch).iter_mut() {
                    *x *= self.0;
                }
            }
        }
        fn adopt(&mut self, _: &mut dyn Processor) -> Adopted {
            Adopted::Yes
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    fn external(id: &str) -> External {
        let type_name: &'static str = Box::leak(plugin_type(id).into_boxed_str());
        External {
            id: id.to_owned(),
            name: "Some Gain (LV2)".into(),
            kind: PluginKind::Effect,
            schema: NodeSchema::new(type_name, 1).param(ParamSpec::new("gain", 0.0, 2.0, 1.0)),
            ports: Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![] },
            build: Arc::new(|state: &Value, _cx: &BuildContext<'_>| {
                let g = state.get(PARAMS_KEY).and_then(|p| p.get("gain")).and_then(Value::as_real).unwrap_or(1.0);
                Ok(Box::new(Gain(g as f32)) as Box<dyn Processor>)
            }),
        }
    }

    #[test]
    fn external_plugins_live_beside_the_program_s() {
        let registry = Registry::with_external(&[], vec![external("lv2-0123456789abcdef")]).unwrap();
        let id = "lv2-0123456789abcdef";
        assert_eq!(registry.kind(id), Some(PluginKind::Effect));
        assert!(registry.get(id).is_none() && registry.is_external(id));
        let entries: Vec<_> = registry.entries().collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].third_party && entries[0].name == "Some Gain (LV2)");
        assert_eq!(registry.params(id).unwrap()[0].key, "gain");
        assert_eq!(registry.ports(id, &Value::empty_object()).unwrap().outputs.len(), 1);
        let schemas = registry.project_schemas().unwrap();
        assert!(schemas.find(&plugin_type(id)).is_some());
        let view = crate::ResourceView::new();
        let cx =
            BuildContext { spec: ProcessSpec { sample_rate: 48_000, max_block: 256 }, resources: &view, channels: 2 };
        assert!(registry.build(id, &Value::empty_object(), &cx).is_ok());
        // The generated checks that apply to a third-party plugin
        crate::testing::external_contract(&registry, id, true);
        // A bad id or a schema of another name is refused
        assert!(Registry::with_external(&[], vec![external("Not Valid")]).is_err());
        let mut wrong = external("lv2-1");
        wrong.schema = NodeSchema::new("plugin:other", 1);
        assert!(Registry::with_external(&[], vec![wrong]).is_err());
    }
}
