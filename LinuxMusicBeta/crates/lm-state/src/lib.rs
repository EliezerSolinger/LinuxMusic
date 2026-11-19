//! The program as data (interfaces.md, I-3; arquitetura-nova.md, piece 2): the immutable tree every part of the
//! program reads, and the rules of its shape.
//!
//! - [`Value`]: the immutable JSON-like tree. O(1) clone, structural sharing (`imbl`), sorted object keys, exact
//!   equality, O(1) identity ([`Value::same_node`]);
//! - [`PathStep`]: paths into the tree, notes included ([`PathStep::Note`], C1);
//! - [`NodeId`] and [`ROOT`]: ids as `"t:17"`;
//! - [`Resource`]: files of the project by BLAKE3 hash, validated so they can never name a path (C9);
//! - [`to_json`] / [`from_json`]: the stable JSON, bit-exact both ways;
//! - [`SchemaRegistry`], [`NodeSchema`], [`FieldSpec`], [`FieldKind`], [`ParamSpec`], [`OnRemove`]: the shape and
//!   the versions of every type of node, the validation of one node, the upgrades;
//! - [`frozen`]: the test helper that keeps defaults frozen per schema version (U29).
//!
//! Errors are [`lm_base::Error`] values with keys under `state.` (`state.json.syntax`, `state.schema.out_of_range`,
//! `state.resource.invalid_key`...), with the position or the path as arguments.

pub mod frozen;
mod id;
mod json;
mod resource;
mod schema;
mod value;

pub use id::{NodeId, ROOT};
pub use json::{MAX_DEPTH, MAX_SAFE_INT, from_json, from_json_bytes, to_json, to_json_compact};
pub use resource::{
    INLINE_MAX_BYTES, RESOURCE_KEY_LEN, RESOURCE_KIND_MAX_LEN, Resource, is_valid_resource_key, is_valid_resource_kind,
};
pub use schema::{
    FieldKind, FieldSpec, FieldVisitor, NodeSchema, OnRemove, PARAMS_KEY, ParamSpec, Rate, RefSite, Scale,
    SchemaRegistry, Upgrade, UpgradeFn,
};
pub use value::{Kind, PathStep, Value, note_key, parse_note_key, path_text};
