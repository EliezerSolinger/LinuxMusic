//! Schemas (interfaces.md, I-3.3, I-9.1, U29, U30, U24, C5): the shape of every type of node, with its version and
//! the pure functions that bring an older node up to date.
//!
//! The registry is built at start-up and then kept immutable inside an `Arc` (Parte 3): every method that reads
//! takes `&self`, and the registry is `Send + Sync`.
//!
//! What lives here is the validation of **one node** (and the objects nested in it by its schema): kinds, ranges,
//! choices, ids, the form of references and resources, parameters. Rules across nodes (a reference that resolves,
//! cycles in the routing, the engine limits) are the document's (`lm-doc`), which gets from here what it needs:
//! [`SchemaRegistry::refs`], [`SchemaRegistry::ids`], [`SchemaRegistry::resources`] and
//! [`SchemaRegistry::visit_fields`].

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use lm_base::{Error, Result};

use crate::id::{NodeId, truncated};
use crate::json::{MAX_SAFE_INT, to_json};
use crate::resource::{INLINE_MAX_BYTES, Resource, is_valid_resource_kind};
use crate::value::{Kind, PathStep, Value, parse_note_key, path_text};

/// What happens to a reference when the node it points to is removed (R10, D4). Applied by `lm-doc`, in the same
/// transaction as the removal
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnRemove {
    /// The object holding the reference is removed too (a send whose destination is gone). Every object that holds
    /// a reference has an `id`, so there is always an owner to remove
    RemoveOwner,
    /// The reference becomes `null` (the field must be nullable); inside a list, the item is removed from the list
    Clear,
    /// The removal is refused while the reference exists
    Refuse,
    /// The reference is pointed at this node instead (an output whose mixer channel is gone goes to the master)
    Redirect(NodeId),
}

/// The kind of a field, with its domain
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Bool,
    /// An int within `min..=max` (both within ±(2^53 − 1), the JSON limit)
    Int {
        min: i64,
        max: i64,
    },
    /// A real within `min..=max`
    Real {
        min: f64,
        max: f64,
    },
    /// A text up to [`INLINE_MAX_BYTES`] (larger opaque state goes to a resource, U24)
    Text,
    /// One of a closed set of stable text keys, never a number (U30)
    Choice(&'static [&'static str]),
    /// A resource of this kind (`"audio"`)
    Resource(&'static str),
    /// A reference to another node, as its id text (`"m:3"`), whose kind letter is one of `to`
    Ref {
        to: &'static [u8],
        on_remove: OnRemove,
    },
    /// A list whose items are all of this kind
    List(Box<FieldKind>),
    /// An object of the registered type with this name
    Object(&'static str),
    /// The node's own id (`"t:17"`), of this kind letter
    Id(u8),
    /// A note map (C1): an object keyed by the canonical decimal text of each note's number `n`, every value an
    /// object of the registered type with this name
    NoteMap(&'static str),
    /// Any tree: state the program does not interpret (a third-party plugin's `chunk`, the tree of a plugin that
    /// is not available, which is kept and saved back intact). Only the size rule of U24 applies
    Any,
}

/// One field of a node type
#[derive(Clone, Debug, PartialEq)]
pub struct FieldSpec {
    pub key: &'static str,
    pub kind: FieldKind,
    /// The value of a new node (frozen per schema version, U29). `None`: no default (an `id`, which the document
    /// gives); a required list, note map or object without one defaults to empty or to its type's defaults
    pub default: Option<Value>,
    /// The field must be present
    pub required: bool,
    /// `null` is accepted besides the kind
    pub nullable: bool,
}

impl FieldSpec {
    /// A required, non-nullable field without default
    pub fn new(key: &'static str, kind: FieldKind) -> Self {
        Self { key, kind, default: None, required: true, nullable: false }
    }

    #[must_use]
    pub fn default(mut self, v: Value) -> Self {
        self.default = Some(v);
        self
    }

    #[must_use]
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    #[must_use]
    pub fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }

    pub fn bool(key: &'static str, default: bool) -> Self {
        Self::new(key, FieldKind::Bool).default(Value::bool(default))
    }

    pub fn int(key: &'static str, min: i64, max: i64, default: i64) -> Self {
        Self::new(key, FieldKind::Int { min, max }).default(Value::int(default))
    }

    /// A real field. A non-finite default is caught by [`SchemaRegistry::add`]
    pub fn real(key: &'static str, min: f64, max: f64, default: f64) -> Self {
        let field = Self::new(key, FieldKind::Real { min, max });
        match Value::real(default) {
            Ok(v) => field.default(v),
            Err(_) => field.default(Value::NULL),
        }
    }

    pub fn text(key: &'static str, default: &str) -> Self {
        Self::new(key, FieldKind::Text).default(Value::text(default))
    }

    pub fn choice(key: &'static str, choices: &'static [&'static str], default: &str) -> Self {
        Self::new(key, FieldKind::Choice(choices)).default(Value::text(default))
    }

    pub fn id(kind: u8) -> Self {
        Self::new("id", FieldKind::Id(kind))
    }
}

/// How a parameter's range is shown and swept
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Linear,
    /// Logarithmic (frequencies, times): the minimum must be above 0
    Log,
    /// Whole numbers only (octaves, semitones). Named choices are a `Choice` field, never stepped numbers (U30)
    Stepped,
}

/// How often the engine updates a parameter under automation (I-6.2)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    /// Every sub-block of up to 32 samples
    Block,
    /// Every sample (volume, pan)
    Sample,
}

/// A plugin parameter: a real in the node's `"params"` object, a slot in the engine (I-6). Its key is permanent
/// (C5): it never changes and is never reused; a rename declares the old key in `aliases`
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSpec {
    pub key: &'static str,
    pub min: f64,
    pub max: f64,
    pub def: f64,
    pub scale: Scale,
    /// `"dB"`, `"Hz"`, `"%"`, or empty
    pub unit: &'static str,
    pub automatable: bool,
    pub rate: Rate,
    /// Old keys of this parameter. Whoever points by an old key resolves through it, and the node's own
    /// `"params"` are rewritten to the current key by [`SchemaRegistry::upgrade`]
    pub aliases: &'static [&'static str],
}

impl ParamSpec {
    /// A linear, automatable, block-rate parameter without unit or aliases
    pub fn new(key: &'static str, min: f64, max: f64, def: f64) -> Self {
        Self { key, min, max, def, scale: Scale::Linear, unit: "", automatable: true, rate: Rate::Block, aliases: &[] }
    }

    #[must_use]
    pub fn scale(mut self, scale: Scale) -> Self {
        self.scale = scale;
        self
    }

    #[must_use]
    pub fn unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    #[must_use]
    pub fn not_automatable(mut self) -> Self {
        self.automatable = false;
        self
    }

    #[must_use]
    pub fn rate(mut self, rate: Rate) -> Self {
        self.rate = rate;
        self
    }

    #[must_use]
    pub fn aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    /// Whether `v` is a valid value: within the range, and whole for a stepped parameter
    pub fn accepts(&self, v: f64) -> bool {
        v >= self.min && v <= self.max && (self.scale != Scale::Stepped || v.fract() == 0.0)
    }
}

/// One step of a type's migration: takes a node saved by version `from` to version `from + 1`. A pure function
pub type UpgradeFn = fn(Value) -> Result<Value>;

/// A migration step
#[derive(Clone, Copy)]
pub struct Upgrade {
    pub from: u32,
    pub run: UpgradeFn,
}

impl fmt::Debug for Upgrade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Upgrade({} → {})", self.from, self.from.saturating_add(1))
    }
}

/// The shape of one type of node: `"track"`, `"item.midi"`, `"plugin:tripleoscillator"`...
#[derive(Clone, Debug)]
pub struct NodeSchema {
    pub type_name: &'static str,
    /// From 1. Changing a default, removing a parameter or changing a field's meaning needs a new version (U29, C5)
    pub version: u32,
    pub fields: Vec<FieldSpec>,
    /// The parameters of a plugin, in the node's `"params"` object
    pub params: Vec<ParamSpec>,
    /// One step per version, contiguous, the last from `version - 1`. The oldest version that still opens is the
    /// first step's `from` (or `version` without steps)
    pub upgrades: Vec<Upgrade>,
}

impl NodeSchema {
    pub fn new(type_name: &'static str, version: u32) -> Self {
        Self { type_name, version, fields: Vec::new(), params: Vec::new(), upgrades: Vec::new() }
    }

    #[must_use]
    pub fn field(mut self, field: FieldSpec) -> Self {
        self.fields.push(field);
        self
    }

    #[must_use]
    pub fn param(mut self, param: ParamSpec) -> Self {
        self.params.push(param);
        self
    }

    #[must_use]
    pub fn upgrade(mut self, from: u32, run: UpgradeFn) -> Self {
        self.upgrades.push(Upgrade { from, run });
        self
    }

    pub fn find_field(&self, key: &str) -> Option<&FieldSpec> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// The parameter with this key, or the one that had it as an old key (C5)
    pub fn find_param(&self, key: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.key == key).or_else(|| self.params.iter().find(|p| p.aliases.contains(&key)))
    }

    /// The oldest saved version this schema can bring up to date
    pub fn oldest_version(&self) -> u32 {
        self.upgrades.first().map_or(self.version, |u| u.from)
    }
}

/// A reference found in a node: where it is, what it may point to and what removing the target does
#[derive(Clone, Debug, PartialEq)]
pub struct RefSite {
    /// From the node given to [`SchemaRegistry::refs`]
    pub path: Vec<PathStep>,
    /// The key of the field that holds it (for an item of a list, the list's key)
    pub field: &'static str,
    pub to: &'static [u8],
    pub on_remove: OnRemove,
    /// `None` for a null reference
    pub target: Option<NodeId>,
}

/// What [`SchemaRegistry::visit_fields`] calls: the path from the node, the field's key, its kind and the value
pub type FieldVisitor<'a> = dyn FnMut(&[PathStep], &'static str, &FieldKind, &Value) -> Result<()> + 'a;

/// The key under which a node keeps its parameters
pub const PARAMS_KEY: &str = "params";

/// All the node types, by name. Built at start-up, then immutable
#[derive(Debug, Default)]
pub struct SchemaRegistry {
    types: BTreeMap<&'static str, NodeSchema>,
}

/// Where a check is in the tree, for the error arguments
struct At<'a> {
    type_name: &'a str,
    field: &'a str,
}

fn schema_error(key: &'static str, at: &At<'_>, path: &[PathStep]) -> Error {
    Error::new(key).arg("type", at.type_name).arg("field", at.field).arg("path", path_text(path))
}

fn definition_error(type_name: &str, problem: &'static str) -> Error {
    Error::new("state.schema.definition").arg("type", type_name).arg("problem", problem)
}

fn is_valid_field_key(key: &str) -> bool {
    let b = key.as_bytes();
    !b.is_empty() && b[0].is_ascii_alphabetic() && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

fn is_valid_type_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b':' | b'-'))
}

/// The deepest nesting of schema types the registry follows (a defaults cycle, or a hostile tree built in code)
const MAX_TYPE_DEPTH: usize = 128;

impl SchemaRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a type. Its definition is checked: keys, ranges, choices, defaults, the parameters and the chain of
    /// upgrades. Types it nests are checked by [`SchemaRegistry::check_complete`], once every type is added
    pub fn add(&mut self, schema: NodeSchema) -> Result<()> {
        let name = schema.type_name;
        if !is_valid_type_name(name) {
            return Err(definition_error(name, "type_name"));
        }
        if self.types.contains_key(name) {
            return Err(definition_error(name, "duplicate_type"));
        }
        if schema.version == 0 {
            return Err(definition_error(name, "version_zero"));
        }
        for (i, field) in schema.fields.iter().enumerate() {
            if !is_valid_field_key(field.key) {
                return Err(definition_error(name, "field_key").arg("field", field.key));
            }
            if schema.fields[..i].iter().any(|f| f.key == field.key) {
                return Err(definition_error(name, "duplicate_field").arg("field", field.key));
            }
            check_kind_definition(name, field.key, &field.kind)?;
            if let FieldKind::Ref { on_remove: OnRemove::Clear, .. } = field.kind
                && !field.nullable
            {
                return Err(definition_error(name, "clear_needs_nullable").arg("field", field.key));
            }
            if let Some(default) = &field.default {
                let at = At { type_name: name, field: field.key };
                let path = [PathStep::key(field.key)];
                let ok = (default.is_null() && field.nullable)
                    || check_value_shallow(&field.kind, default, &at, &path).is_ok();
                if !ok {
                    return Err(definition_error(name, "default").arg("field", field.key));
                }
            }
        }
        if !schema.params.is_empty() && schema.find_field(PARAMS_KEY).is_some() {
            return Err(definition_error(name, "params_field"));
        }
        for (i, p) in schema.params.iter().enumerate() {
            check_param_definition(name, p)?;
            let earlier = &schema.params[..i];
            let clash = |k: &str| earlier.iter().any(|q| q.key == k || q.aliases.contains(&k));
            if clash(p.key) || p.aliases.iter().any(|a| clash(a) || *a == p.key) {
                return Err(definition_error(name, "duplicate_param").arg("param", p.key));
            }
            for (j, a) in p.aliases.iter().enumerate() {
                if p.aliases[..j].contains(a) || !is_valid_field_key(a) {
                    return Err(definition_error(name, "param_alias").arg("param", p.key));
                }
            }
        }
        if let Some(first) = schema.upgrades.first() {
            if first.from == 0 {
                return Err(definition_error(name, "upgrade_chain"));
            }
            let contiguous = schema
                .upgrades
                .iter()
                .enumerate()
                .all(|(i, u)| u32::try_from(i).ok().and_then(|i| first.from.checked_add(i)) == Some(u.from));
            let ends_right = schema.upgrades.last().map(|u| u.from.checked_add(1)) == Some(Some(schema.version));
            if !contiguous || !ends_right {
                return Err(definition_error(name, "upgrade_chain"));
            }
        }
        self.types.insert(name, schema);
        Ok(())
    }

    /// Checks what [`SchemaRegistry::add`] cannot check one type at a time: every nested type exists, no type
    /// requires itself (its defaults would be infinite), and the defaults of every type are valid. Called once at
    /// start-up, and in the tests of every crate that registers types
    pub fn check_complete(&self) -> Result<()> {
        for schema in self.types.values() {
            for field in &schema.fields {
                let mut nested = Vec::new();
                nested_types(&field.kind, &mut nested);
                for t in nested {
                    if !self.types.contains_key(t) {
                        return Err(definition_error(schema.type_name, "unknown_nested_type")
                            .arg("field", field.key)
                            .arg("nested", t));
                    }
                }
            }
        }
        for schema in self.types.values() {
            let defaults = self.defaults(schema.type_name)?;
            self.check_node(schema.type_name, &defaults, &mut Vec::new(), 0, true)
                .map_err(|e| e.arg("problem", "default"))?;
        }
        Ok(())
    }

    pub fn find(&self, type_name: &str) -> Option<&NodeSchema> {
        self.types.get(type_name)
    }

    /// Every registered type, in name order
    pub fn schemas(&self) -> impl Iterator<Item = &NodeSchema> + '_ {
        self.types.values()
    }

    fn schema(&self, type_name: &str) -> Result<&NodeSchema> {
        self.types
            .get(type_name)
            .ok_or_else(|| Error::new("state.schema.unknown_type").arg("type", truncated(type_name)))
    }

    /// The parameter `key` of a type, through its aliases (C5)
    pub fn resolve_param(&self, type_name: &str, key: &str) -> Option<&ParamSpec> {
        self.find(type_name)?.find_param(key)
    }

    /// Validates a node of this type (at its current version). The error names the type, the field and the path
    /// from the node (`"path": "/outputs/0/to"`)
    pub fn validate(&self, type_name: &str, node: &Value) -> Result<()> {
        self.check_node(type_name, node, &mut Vec::new(), 0, false)
    }

    /// Like [`SchemaRegistry::validate`], for a node found at `at` in a larger tree: the paths in the errors start
    /// from there
    pub fn validate_at(&self, type_name: &str, node: &Value, at: &[PathStep]) -> Result<()> {
        let mut path = at.to_vec();
        self.check_node(type_name, node, &mut path, 0, false)
    }

    fn check_node(
        &self,
        type_name: &str,
        node: &Value,
        path: &mut Vec<PathStep>,
        depth: usize,
        partial: bool,
    ) -> Result<()> {
        let schema = self.schema(type_name).map_err(|e| e.arg("path", path_text(path)))?;
        let at = At { type_name: schema.type_name, field: "" };
        if depth > MAX_TYPE_DEPTH {
            return Err(schema_error("state.schema.too_deep", &at, path));
        }
        if !node.is_object() {
            return Err(schema_error("state.schema.not_object", &at, path).arg("found", node.kind().name()));
        }
        for field in &schema.fields {
            let at = At { type_name: schema.type_name, field: field.key };
            path.push(PathStep::key(field.key));
            match node.get(field.key) {
                None if field.required && !partial => {
                    return Err(schema_error("state.schema.missing_field", &at, path));
                }
                None => {}
                Some(v) if v.is_null() && field.nullable => {}
                Some(v) => self.check_value(&field.kind, v, &at, path, depth, partial)?,
            }
            path.pop();
        }
        let has_params = !schema.params.is_empty();
        for key in node.keys() {
            let known = schema.find_field(key).is_some() || (has_params && key == PARAMS_KEY);
            if !known {
                path.push(PathStep::key(key));
                let e =
                    schema_error("state.schema.unknown_field", &At { type_name: schema.type_name, field: key }, path);
                return Err(e);
            }
        }
        if let Some(params) = node.get(PARAMS_KEY).filter(|_| has_params) {
            self.check_params(schema, params, path)?;
        }
        Ok(())
    }

    fn check_params(&self, schema: &NodeSchema, params: &Value, path: &mut Vec<PathStep>) -> Result<()> {
        path.push(PathStep::key(PARAMS_KEY));
        let at = At { type_name: schema.type_name, field: PARAMS_KEY };
        if !params.is_object() {
            return Err(schema_error("state.schema.wrong_kind", &at, path)
                .arg("expected", "object")
                .arg("found", params.kind().name()));
        }
        for (key, v) in params.entries() {
            path.push(PathStep::key(key));
            let at = At { type_name: schema.type_name, field: key };
            let Some(spec) = schema.params.iter().find(|p| p.key == key) else {
                let error_key = if schema.find_param(key).is_some() {
                    "state.schema.param_alias"
                } else {
                    "state.schema.unknown_param"
                };
                return Err(schema_error(error_key, &at, path));
            };
            let Some(x) = v.as_real() else {
                return Err(schema_error("state.schema.wrong_kind", &at, path)
                    .arg("expected", "real")
                    .arg("found", v.kind().name()));
            };
            if !spec.accepts(x) {
                return Err(schema_error("state.schema.param_out_of_range", &at, path)
                    .arg("value", x)
                    .arg("min", spec.min)
                    .arg("max", spec.max));
            }
            path.pop();
        }
        path.pop();
        Ok(())
    }

    fn check_value(
        &self,
        kind: &FieldKind,
        v: &Value,
        at: &At<'_>,
        path: &mut Vec<PathStep>,
        depth: usize,
        partial: bool,
    ) -> Result<()> {
        match kind {
            FieldKind::List(inner) => {
                if !v.is_list() {
                    return Err(wrong_kind(at, path, "list", v));
                }
                for (i, item) in v.items().enumerate() {
                    path.push(PathStep::Index(i));
                    self.check_value(inner, item, at, path, depth, partial)?;
                    path.pop();
                }
                Ok(())
            }
            FieldKind::Object(t) => self.check_node(t, v, path, depth + 1, partial),
            FieldKind::NoteMap(t) => {
                if !v.is_object() {
                    return Err(wrong_kind(at, path, "object", v));
                }
                for (key, note) in v.entries() {
                    path.push(PathStep::key(key));
                    if parse_note_key(key).is_none() {
                        return Err(schema_error("state.schema.note_key", at, path));
                    }
                    self.check_node(t, note, path, depth + 1, partial)?;
                    path.pop();
                }
                Ok(())
            }
            _ => check_value_shallow(kind, v, at, path),
        }
    }

    /// Brings a node saved by version `from` of its type up to the current version, one step at a time, then
    /// rewrites its parameters from old keys to current ones (C5). A version newer than the program's is refused
    /// (`state.schema.newer_version`: the project opens read-only, H11); one older than the oldest step too. The
    /// result is not validated: [`SchemaRegistry::load`] does both
    pub fn upgrade(&self, type_name: &str, from: u32, node: Value) -> Result<Value> {
        let schema = self.schema(type_name)?;
        let version_error = |key| {
            Error::new(key)
                .arg("type", schema.type_name)
                .arg("found", i64::from(from))
                .arg("version", i64::from(schema.version))
        };
        if from > schema.version {
            return Err(version_error("state.schema.newer_version"));
        }
        if from < schema.oldest_version() {
            return Err(version_error("state.schema.version_too_old"));
        }
        let mut node = normalize_params(schema, node)?;
        for step in schema.upgrades.iter().filter(|u| u.from >= from) {
            node = (step.run)(node).map_err(|e| e.arg("type", schema.type_name).arg("from", i64::from(step.from)))?;
        }
        normalize_params(schema, node)
    }

    /// Upgrade, then validate: what opening a node does
    pub fn load(&self, type_name: &str, from: u32, node: Value) -> Result<Value> {
        let node = self.upgrade(type_name, from, node)?;
        self.validate(type_name, &node)?;
        Ok(node)
    }

    /// A new node of this type with every default: fields with a default, required lists and note maps empty,
    /// required objects with their own type's defaults, and every parameter at its default. Fields without a
    /// default (the `id`) are left out, for the document to fill
    pub fn defaults(&self, type_name: &str) -> Result<Value> {
        self.defaults_inner(type_name, 0)
    }

    fn defaults_inner(&self, type_name: &str, depth: usize) -> Result<Value> {
        let schema = self.schema(type_name)?;
        if depth > MAX_TYPE_DEPTH {
            return Err(definition_error(schema.type_name, "recursive_defaults"));
        }
        let mut node = Value::empty_object();
        for field in &schema.fields {
            let v = match (&field.default, &field.kind) {
                (Some(v), _) => v.clone(),
                (None, _) if !field.required => continue,
                (None, FieldKind::List(_)) => Value::empty_list(),
                (None, FieldKind::NoteMap(_)) => Value::empty_object(),
                (None, FieldKind::Object(t)) => self.defaults_inner(t, depth + 1)?,
                (None, _) => continue,
            };
            node = node.with(field.key, v);
        }
        if !schema.params.is_empty() {
            let mut params = Value::empty_object();
            for p in &schema.params {
                params = params.with(p.key, Value::real(p.def)?);
            }
            node = node.with(PARAMS_KEY, params);
        }
        Ok(node)
    }

    /// Calls `visit` for every field value of the node and of the objects its schema nests in it (lists, objects,
    /// note maps), with its path from the node, the field's key and its kind. Lists call it for the list and then
    /// for each item (with the item's kind). `Any` subtrees are not entered. The node must be valid
    pub fn visit_fields(&self, type_name: &str, node: &Value, visit: &mut FieldVisitor<'_>) -> Result<()> {
        self.visit_node(type_name, node, &mut Vec::new(), 0, visit)
    }

    fn visit_node(
        &self,
        type_name: &str,
        node: &Value,
        path: &mut Vec<PathStep>,
        depth: usize,
        visit: &mut FieldVisitor<'_>,
    ) -> Result<()> {
        let schema = self.schema(type_name)?;
        if depth > MAX_TYPE_DEPTH {
            return Err(schema_error("state.schema.too_deep", &At { type_name, field: "" }, path));
        }
        for field in &schema.fields {
            if let Some(v) = node.get(field.key) {
                path.push(PathStep::key(field.key));
                self.visit_value(field.key, &field.kind, v, path, depth, visit)?;
                path.pop();
            }
        }
        Ok(())
    }

    fn visit_value(
        &self,
        field: &'static str,
        kind: &FieldKind,
        v: &Value,
        path: &mut Vec<PathStep>,
        depth: usize,
        visit: &mut FieldVisitor<'_>,
    ) -> Result<()> {
        visit(path, field, kind, v)?;
        match kind {
            FieldKind::List(inner) => {
                for (i, item) in v.items().enumerate() {
                    path.push(PathStep::Index(i));
                    self.visit_value(field, inner, item, path, depth, visit)?;
                    path.pop();
                }
            }
            FieldKind::Object(t) if v.is_object() => self.visit_node(t, v, path, depth + 1, visit)?,
            FieldKind::NoteMap(t) => {
                for (key, note) in v.entries() {
                    let step = parse_note_key(key).map_or_else(|| PathStep::key(key), PathStep::Note);
                    path.push(step);
                    self.visit_node(t, note, path, depth + 1, visit)?;
                    path.pop();
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Every reference in the node and in what its schema nests in it, with its rule (what `lm-doc` keeps in the
    /// reverse index and applies on remove)
    pub fn refs(&self, type_name: &str, node: &Value) -> Result<Vec<RefSite>> {
        let mut found = Vec::new();
        self.visit_fields(type_name, node, &mut |path, field, kind, v| {
            if let FieldKind::Ref { to, on_remove } = kind {
                let target = match v.as_text() {
                    Some(text) => Some(NodeId::parse(text)?),
                    None if v.is_null() => None,
                    None => return Err(Error::new("state.schema.wrong_kind").arg("path", path_text(path))),
                };
                found.push(RefSite { path: path.to_vec(), field, to, on_remove: *on_remove, target });
            }
            Ok(())
        })?;
        Ok(found)
    }

    /// Every node id (`Id` field) in the node and in what its schema nests in it, the node's own included (path
    /// `/id`), parents before children
    pub fn ids(&self, type_name: &str, node: &Value) -> Result<Vec<(Vec<PathStep>, NodeId)>> {
        let mut found = Vec::new();
        self.visit_fields(type_name, node, &mut |path, _, kind, v| {
            if let (FieldKind::Id(_), Some(text)) = (kind, v.as_text()) {
                found.push((path.to_vec(), NodeId::parse(text)?));
            }
            Ok(())
        })?;
        Ok(found)
    }

    /// Every resource the node cites in its `Resource` fields (what must be loaded before a build, C11, and what
    /// must not be deleted while a root cites it, H4)
    pub fn resources(&self, type_name: &str, node: &Value) -> Result<Vec<(Vec<PathStep>, Resource)>> {
        let mut found = Vec::new();
        self.visit_fields(type_name, node, &mut |path, _, kind, v| {
            if let (FieldKind::Resource(_), Some(r)) = (kind, v.as_resource()) {
                found.push((path.to_vec(), r.clone()));
            }
            Ok(())
        })?;
        Ok(found)
    }

    /// The defaults of every registered type, as one tree: `{ "<type>": { "version": N, "defaults": {...} } }`
    /// (U29). See [`crate::frozen`]
    pub fn defaults_document(&self) -> Result<Value> {
        let mut doc = Value::empty_object();
        for schema in self.types.values() {
            let entry = Value::object([
                ("version", Value::int(i64::from(schema.version))),
                ("defaults", self.defaults(schema.type_name)?),
            ]);
            doc = doc.with(schema.type_name, entry);
        }
        Ok(doc)
    }

    /// [`SchemaRegistry::defaults_document`] in the stable JSON
    pub fn defaults_json(&self) -> Result<String> {
        to_json(&self.defaults_document()?)
    }
}

fn wrong_kind(at: &At<'_>, path: &[PathStep], expected: &'static str, v: &Value) -> Error {
    schema_error("state.schema.wrong_kind", at, path).arg("expected", expected).arg("found", v.kind().name())
}

/// The checks of a value that need no other type
fn check_value_shallow(kind: &FieldKind, v: &Value, at: &At<'_>, path: &[PathStep]) -> Result<()> {
    match kind {
        FieldKind::Bool => v.as_bool().map(|_| ()).ok_or_else(|| wrong_kind(at, path, "bool", v)),
        FieldKind::Int { min, max } => {
            let x = v.as_int().ok_or_else(|| wrong_kind(at, path, "int", v))?;
            if x < *min || x > *max {
                return Err(schema_error("state.schema.out_of_range", at, path)
                    .arg("value", x)
                    .arg("min", *min)
                    .arg("max", *max));
            }
            Ok(())
        }
        FieldKind::Real { min, max } => {
            let x = v.as_real().ok_or_else(|| wrong_kind(at, path, "real", v))?;
            if x < *min || x > *max {
                return Err(schema_error("state.schema.out_of_range", at, path)
                    .arg("value", x)
                    .arg("min", *min)
                    .arg("max", *max));
            }
            Ok(())
        }
        FieldKind::Text => {
            let t = v.as_text().ok_or_else(|| wrong_kind(at, path, "text", v))?;
            check_text_size(t, at, path)
        }
        FieldKind::Choice(choices) => {
            let t = v.as_text().ok_or_else(|| wrong_kind(at, path, "text", v))?;
            if choices.contains(&t) {
                Ok(())
            } else {
                Err(schema_error("state.schema.not_a_choice", at, path).arg("value", truncated(t)))
            }
        }
        FieldKind::Resource(kind) => {
            let r = v.as_resource().ok_or_else(|| wrong_kind(at, path, "resource", v))?;
            if r.kind() == *kind {
                Ok(())
            } else {
                Err(schema_error("state.schema.wrong_resource_kind", at, path)
                    .arg("expected", *kind)
                    .arg("found", r.kind().to_owned()))
            }
        }
        FieldKind::Ref { to, .. } => {
            let t = v.as_text().ok_or_else(|| wrong_kind(at, path, "ref", v))?;
            let id = NodeId::parse(t).map_err(|_| schema_error("state.schema.invalid_id", at, path))?;
            if to.contains(&id.kind) {
                Ok(())
            } else {
                Err(schema_error("state.schema.wrong_ref_kind", at, path).arg("value", id.to_string()))
            }
        }
        FieldKind::Id(kind) => {
            let t = v.as_text().ok_or_else(|| wrong_kind(at, path, "id", v))?;
            let id = NodeId::parse(t).map_err(|_| schema_error("state.schema.invalid_id", at, path))?;
            if id.kind == *kind {
                Ok(())
            } else {
                Err(schema_error("state.schema.wrong_id_kind", at, path).arg("value", id.to_string()))
            }
        }
        FieldKind::List(_) => {
            if v.is_list() {
                Ok(())
            } else {
                Err(wrong_kind(at, path, "list", v))
            }
        }
        FieldKind::Object(_) | FieldKind::NoteMap(_) => {
            if v.is_object() {
                Ok(())
            } else {
                Err(wrong_kind(at, path, "object", v))
            }
        }
        FieldKind::Any => {
            // Only the size of texts (U24); the path of the first offending text is reported
            let mut problem = None;
            v.walk(&mut |sub, x| {
                if problem.is_none()
                    && x.kind() == Kind::Text
                    && x.as_text().is_some_and(|t| t.len() > INLINE_MAX_BYTES)
                {
                    let mut full = path.to_vec();
                    full.extend_from_slice(sub);
                    problem = Some(full);
                }
            });
            match problem {
                Some(full) => Err(schema_error("state.schema.text_too_large", at, &full)
                    .arg("max", i64::try_from(INLINE_MAX_BYTES).unwrap_or(i64::MAX))),
                None => Ok(()),
            }
        }
    }
}

fn check_text_size(t: &str, at: &At<'_>, path: &[PathStep]) -> Result<()> {
    if t.len() > INLINE_MAX_BYTES {
        Err(schema_error("state.schema.text_too_large", at, path)
            .arg("max", i64::try_from(INLINE_MAX_BYTES).unwrap_or(i64::MAX)))
    } else {
        Ok(())
    }
}

fn nested_types(kind: &FieldKind, out: &mut Vec<&'static str>) {
    match kind {
        FieldKind::Object(t) | FieldKind::NoteMap(t) => out.push(t),
        FieldKind::List(inner) => nested_types(inner, out),
        _ => {}
    }
}

fn check_kind_definition(type_name: &str, field: &'static str, kind: &FieldKind) -> Result<()> {
    let bad = |problem| Err(definition_error(type_name, problem).arg("field", field));
    match kind {
        FieldKind::Int { min, max } => {
            if min > max || *min < -MAX_SAFE_INT || *max > MAX_SAFE_INT {
                return bad("int_range");
            }
        }
        FieldKind::Real { min, max } => {
            if !min.is_finite() || !max.is_finite() || min > max {
                return bad("real_range");
            }
        }
        FieldKind::Choice(choices) => {
            let all_valid = choices.iter().all(|c| !c.is_empty() && c.len() <= 64);
            let unique = choices.iter().enumerate().all(|(i, c)| !choices[..i].contains(c));
            if choices.is_empty() || !all_valid || !unique {
                return bad("choices");
            }
        }
        FieldKind::Resource(kind) => {
            if !is_valid_resource_kind(kind) {
                return bad("resource_kind");
            }
        }
        FieldKind::Ref { to, on_remove } => {
            if to.is_empty() || !to.iter().all(u8::is_ascii_lowercase) {
                return bad("ref_kinds");
            }
            if let OnRemove::Redirect(target) = on_remove
                && (!target.is_valid() || !to.contains(&target.kind))
            {
                return bad("redirect_target");
            }
        }
        FieldKind::Id(kind) => {
            if !kind.is_ascii_lowercase() {
                return bad("id_kind");
            }
        }
        FieldKind::List(inner) => {
            if let FieldKind::Id(_) = **inner {
                return bad("id_in_list");
            }
            check_kind_definition(type_name, field, inner)?;
        }
        FieldKind::Object(t) | FieldKind::NoteMap(t) => {
            if !is_valid_type_name(t) {
                return bad("nested_type");
            }
        }
        FieldKind::Bool | FieldKind::Text | FieldKind::Any => {}
    }
    Ok(())
}

fn check_param_definition(type_name: &str, p: &ParamSpec) -> Result<()> {
    let bad = |problem| Err(definition_error(type_name, problem).arg("param", p.key));
    if !is_valid_field_key(p.key) {
        return bad("param_key");
    }
    if !(p.min.is_finite() && p.max.is_finite() && p.def.is_finite()) || p.min >= p.max {
        return bad("param_range");
    }
    if p.def < p.min || p.def > p.max {
        return bad("param_default");
    }
    if p.scale == Scale::Log && p.min <= 0.0 {
        return bad("param_log_range");
    }
    if p.scale == Scale::Stepped && (p.min.fract() != 0.0 || p.max.fract() != 0.0 || p.def.fract() != 0.0) {
        return bad("param_steps");
    }
    Ok(())
}

/// Rewrites parameter keys that are old names (aliases) to the current ones
fn normalize_params(schema: &NodeSchema, node: Value) -> Result<Value> {
    if schema.params.is_empty() {
        return Ok(node);
    }
    let Some(params) = node.get(PARAMS_KEY).filter(|p| p.is_object()) else {
        return Ok(node);
    };
    let mut renamed = params.clone();
    for (key, v) in params.entries() {
        if schema.params.iter().any(|p| p.key == key) {
            continue;
        }
        if let Some(spec) = schema.find_param(key) {
            if renamed.get(spec.key).is_some() {
                return Err(Error::new("state.schema.param_alias_conflict")
                    .arg("type", schema.type_name)
                    .arg("param", spec.key)
                    .arg("alias", truncated(key)));
            }
            renamed = renamed.without(key).with_key(Arc::from(spec.key), v.clone());
        }
    }
    if renamed.same_node(params) {
        return Ok(node);
    }
    Ok(node.with(PARAMS_KEY, renamed))
}
