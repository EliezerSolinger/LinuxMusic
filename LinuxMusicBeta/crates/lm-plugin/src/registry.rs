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

struct Entry {
    descriptor: &'static Descriptor,
    type_name: String,
    /// The main schema (its parameters, by index: the index a processor's `Param` events carry)
    schema: NodeSchema,
}

/// Every plugin of the program, immutable once made
pub struct Registry {
    plugins: BTreeMap<&'static str, Entry>,
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
        let mut plugins = BTreeMap::new();
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
            plugins.insert(d.id, Entry { descriptor: d, type_name, schema: main });
        }
        schemas.check_complete()?;
        let registry = Self { plugins, schemas };
        for entry in registry.plugins.values() {
            let defaults = registry.defaults(entry.descriptor.id)?;
            let ports = (entry.descriptor.ports)(&defaults);
            check_ports(entry.descriptor.id, entry.descriptor.kind, &ports)?;
        }
        Ok(Arc::new(registry))
    }

    pub fn get(&self, id: &str) -> Option<&'static Descriptor> {
        self.plugins.get(id).map(|e| e.descriptor)
    }

    /// Every descriptor, by id
    pub fn descriptors(&self) -> impl Iterator<Item = &'static Descriptor> + '_ {
        self.plugins.values().map(|e| e.descriptor)
    }

    /// The plugins' schemas alone (their types and the types they nest)
    pub fn schemas(&self) -> &SchemaRegistry {
        &self.schemas
    }

    /// Adds every plugin's schemas to a registry (the project's, `lm_doc::register_project_schemas`)
    pub fn register_schemas(&self, registry: &mut SchemaRegistry) -> Result<()> {
        for entry in self.plugins.values() {
            for schema in (entry.descriptor.schemas)() {
                registry.add(schema)?;
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
        Some((entry.descriptor.ports)(&complete))
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
        if entry.descriptor.kind == PluginKind::Effect && cx.channels == 0 {
            return Err(Error::new("plugin.build.channels").arg("plugin", id));
        }
        let build = entry.descriptor.build;
        match catch_unwind(AssertUnwindSafe(|| build(&complete, cx))) {
            Ok(result) => result.map_err(|e| e.arg("plugin", id)),
            Err(_) => Err(Error::new("plugin.build.panicked").arg("plugin", id)),
        }
    }
}

fn unknown(id: &str) -> Error {
    Error::new("plugin.unknown").arg("plugin", id)
}
