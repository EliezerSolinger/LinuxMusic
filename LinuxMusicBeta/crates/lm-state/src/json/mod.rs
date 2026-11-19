//! The stable JSON of the tree (interfaces.md, I-3.1; arquitetura-nova.md, piece 2).
//!
//! Writing ([`to_json`]):
//! - object keys in sorted order (they are already sorted in the tree, S14);
//! - indented with 2 spaces, one entry per line, a newline at the end, so two versions compare with `diff`;
//! - reals always with `.0` or an exponent, in the shortest form that reads back to the same bits (`ryu`); `-0.0`
//!   is kept;
//! - integers beyond ±(2^53 − 1) are refused (a JSON reader cannot keep them exact: ids are text).
//!
//! Reading ([`from_json`]): strict JSON (RFC 8259), with the same rules: a number without a fraction or exponent is
//! an int and must be within ±(2^53 − 1); a number with one is a real and must be finite; a key may appear only
//! once in an object. Nesting is limited to [`MAX_DEPTH`]. It never panics, whatever the input (fuzzed).
//!
//! **Resources** are written as an object with exactly the two keys `"key"` and `"kind"`, both valid
//! (`{"key": "<64 hex digits>", "kind": "audio"}`), and read back as [`crate::Resource`]. Such an object is
//! therefore reserved: the writer refuses an ordinary object of that exact shape (`state.json.ambiguous_resource`),
//! so what is written always reads back to the same tree. An object with a `kind` and a `key` that are not a valid
//! resource (`{"kind": "scale", "key": "C#"}`) is an ordinary object.
//!
//! Round trip: `from_json(&to_json(v)?)? == v` bit for bit, and `to_json(&from_json(s)?)? == s` for any `s` the
//! writer made.

mod read;
mod write;

pub use read::{MAX_DEPTH, from_json, from_json_bytes};
pub use write::{to_json, to_json_compact};

/// The largest integer JSON keeps exact: 2^53 − 1. Integers beyond ±this are refused both ways
pub const MAX_SAFE_INT: i64 = (1 << 53) - 1;

use crate::resource::{is_valid_resource_key, is_valid_resource_kind};
use crate::value::Value;

/// Whether an object of exactly the keys `key` and `kind` (in that order, sorted) holding these values is the JSON
/// form of a resource
fn resource_shape<'a>(mut entries: impl Iterator<Item = (&'a str, &'a Value)>) -> Option<(&'a str, &'a str)> {
    let (k1, v1) = entries.next()?;
    let (k2, v2) = entries.next()?;
    if entries.next().is_some() || k1 != "key" || k2 != "kind" {
        return None;
    }
    let (key, kind) = (v1.as_text()?, v2.as_text()?);
    (is_valid_resource_key(key) && is_valid_resource_kind(kind)).then_some((kind, key))
}
