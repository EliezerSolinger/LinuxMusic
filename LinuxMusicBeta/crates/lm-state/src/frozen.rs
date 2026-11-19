//! Frozen defaults (U29): changing a field's default value needs a new schema version, with an `upgrade` that
//! writes the old default into old projects. Otherwise a project saved without the field (or a test written against
//! it) would silently change its sound.
//!
//! Every crate that registers schemas keeps a reference file with the defaults of all its types (the stable JSON of
//! [`crate::SchemaRegistry::defaults_document`]) and has a test that calls [`check_frozen_defaults`]:
//!
//! ```no_run
//! # fn my_registry() -> lm_state::SchemaRegistry { lm_state::SchemaRegistry::new() }
//! // In tests/defaults.rs, as a #[test]:
//! let reference = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/defaults.json");
//! if let Err(mismatch) = lm_state::frozen::check_frozen_defaults(&my_registry(), reference.as_ref()) {
//!     panic!("{mismatch}");
//! }
//! ```
//!
//! The test fails when a default changed and the version did not, and also when the reference is out of date for
//! any other reason (a type added, removed or bumped), so the reference is always the current truth. After a
//! deliberate change (a version bump with its `upgrade`, a new type), run the test once with
//! `LM_BLESS_DEFAULTS=1` to rewrite the reference, and commit it with the change.

use std::fmt;
use std::path::Path;

use crate::json::from_json;
use crate::schema::SchemaRegistry;
use crate::value::Value;

/// The environment variable that rewrites the reference instead of comparing
pub const BLESS_ENV: &str = "LM_BLESS_DEFAULTS";

/// What differs between the registry and the reference, one line per problem (for the developer: this is test
/// tooling, not a message for the user)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultsMismatch {
    pub problems: Vec<String>,
}

impl fmt::Display for DefaultsMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "the schema defaults differ from the reference (U29):")?;
        for p in &self.problems {
            writeln!(f, "  - {p}")?;
        }
        write!(f, "a deliberate change (with its version bump) is accepted by running once with {BLESS_ENV}=1")
    }
}

impl std::error::Error for DefaultsMismatch {}

/// Compares the defaults of every type in `registry` with the reference file. With `LM_BLESS_DEFAULTS=1` in the
/// environment, writes the reference instead, unless a default changed without a version bump (that is never
/// accepted)
pub fn check_frozen_defaults(registry: &SchemaRegistry, reference: &Path) -> Result<(), DefaultsMismatch> {
    let bless = std::env::var_os(BLESS_ENV).is_some_and(|v| v == "1");
    compare_or_bless(registry, reference, bless)
}

/// [`check_frozen_defaults`] with the blessing decided by the caller
pub fn compare_or_bless(registry: &SchemaRegistry, reference: &Path, bless: bool) -> Result<(), DefaultsMismatch> {
    let fail = |p: String| DefaultsMismatch { problems: vec![p] };
    let current = registry.defaults_document().map_err(|e| fail(format!("the defaults cannot be built: {e}")))?;
    let current_json = registry.defaults_json().map_err(|e| fail(format!("the defaults cannot be written: {e}")))?;
    let old = match std::fs::read_to_string(reference) {
        Ok(text) => Some(
            from_json(&text).map_err(|e| fail(format!("the reference {} is not valid: {e}", reference.display())))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(fail(format!("the reference {} cannot be read: {e}", reference.display()))),
    };
    let problems = match &old {
        Some(old) => differences(old, &current),
        None => vec![format!("the reference {} does not exist", reference.display())],
    };
    if problems.is_empty() {
        return Ok(());
    }
    // Never bless a default changed in place: that is exactly what the rule forbids
    let forbidden: Vec<String> = problems.iter().filter(|p| p.contains("without a version bump")).cloned().collect();
    if bless && forbidden.is_empty() {
        if let Some(dir) = reference.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(format!("cannot create {}: {e}", dir.display())))?;
        }
        std::fs::write(reference, current_json)
            .map_err(|e| fail(format!("cannot write {}: {e}", reference.display())))?;
        return Ok(());
    }
    Err(DefaultsMismatch { problems: if bless { forbidden } else { problems } })
}

fn differences(old: &Value, current: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    for (type_name, now) in current.entries() {
        let Some(before) = old.get(type_name) else {
            problems.push(format!("type {type_name:?} is new (not in the reference)"));
            continue;
        };
        let version = |v: &Value| v.get("version").and_then(Value::as_int);
        let (vb, vn) = (version(before), version(now));
        let (db, dn) = (before.get("defaults"), now.get("defaults"));
        match (vb, vn) {
            (Some(b), Some(n)) if b == n => {
                if db != dn {
                    let fields = changed_fields(db, dn);
                    problems.push(format!(
                        "type {type_name:?}: default of {fields} changed without a version bump (still version {n})"
                    ));
                }
            }
            (Some(b), Some(n)) if n > b => {
                problems.push(format!("type {type_name:?}: version {b} → {n}; the reference must be updated"));
            }
            (b, n) => problems.push(format!("type {type_name:?}: version went from {b:?} to {n:?}")),
        }
    }
    for type_name in old.keys() {
        if current.get(type_name).is_none() {
            problems.push(format!("type {type_name:?} was removed (still in the reference)"));
        }
    }
    problems
}

fn changed_fields(before: Option<&Value>, now: Option<&Value>) -> String {
    let empty = Value::empty_object();
    let (b, n) = (before.unwrap_or(&empty), now.unwrap_or(&empty));
    let mut keys: Vec<&str> = b.keys().chain(n.keys()).filter(|k| b.get(k) != n.get(k)).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>().join(", ")
}
