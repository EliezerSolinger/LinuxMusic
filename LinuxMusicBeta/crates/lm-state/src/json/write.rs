//! The JSON writer: stable, sorted, indented, reals that read back to the same bits.

use std::fmt::Write as _;
use std::sync::Arc;

use lm_base::{Error, Result};

use super::{MAX_DEPTH, MAX_SAFE_INT, resource_shape};
use crate::value::{Kind, PathStep, Value, path_text};

/// The stable, indented form (2 spaces, one entry per line, a final newline): what the project file holds
pub fn to_json(value: &Value) -> Result<String> {
    write_document(value, true)
}

/// The same rules on one line, without spaces (for the journal and the remote protocol)
pub fn to_json_compact(value: &Value) -> Result<String> {
    write_document(value, false)
}

fn write_document(value: &Value, pretty: bool) -> Result<String> {
    let mut writer = Writer { out: String::new(), pretty, real: ryu::Buffer::new() };
    match writer.value(value, 0) {
        Ok(()) => {
            if pretty {
                writer.out.push('\n');
            }
            Ok(writer.out)
        }
        Err(mut failure) => {
            failure.path.reverse();
            Err(failure.error.arg("path", path_text(&failure.path)))
        }
    }
}

/// An error on the way back up, collecting the path (innermost step first)
struct Failure {
    error: Error,
    path: Vec<PathStep>,
}

impl Failure {
    fn new(error: Error) -> Self {
        Self { error, path: Vec::new() }
    }

    fn at(mut self, step: PathStep) -> Self {
        self.path.push(step);
        self
    }
}

struct Writer {
    out: String,
    pretty: bool,
    real: ryu::Buffer,
}

impl Writer {
    fn newline(&mut self, depth: usize) {
        if self.pretty {
            self.out.push('\n');
            for _ in 0..depth {
                self.out.push_str("  ");
            }
        }
    }

    fn value(&mut self, value: &Value, depth: usize) -> Result<(), Failure> {
        // The same nesting the reader accepts, so what is saved always opens again (and this recursion is bounded)
        if matches!(value.kind(), Kind::List | Kind::Object) && depth + 1 > MAX_DEPTH {
            return Err(Failure::new(Error::new("state.json.too_deep")));
        }
        match value.kind() {
            Kind::Null => self.out.push_str("null"),
            Kind::Bool => self.out.push_str(if value.as_bool() == Some(true) { "true" } else { "false" }),
            Kind::Int => {
                let v = value.as_int().unwrap_or(0);
                if !(-MAX_SAFE_INT..=MAX_SAFE_INT).contains(&v) {
                    return Err(Failure::new(Error::new("state.json.int_out_of_range").arg("value", v)));
                }
                // Writing to a String cannot fail
                let _ = write!(self.out, "{v}");
            }
            Kind::Real => {
                // Finite by construction; ryu always gives ".0" or an exponent, and the shortest digits that read
                // back to the same bits ("-0.0" included)
                let v = value.as_real().unwrap_or(0.0);
                let text = self.real.format_finite(v);
                self.out.push_str(text);
            }
            Kind::Text => self.string(value.as_text().unwrap_or("")),
            Kind::Resource => {
                if let Some(r) = value.as_resource() {
                    self.out.push('{');
                    self.newline(depth + 1);
                    self.string("key");
                    self.out.push_str(if self.pretty { ": " } else { ":" });
                    self.string(r.key());
                    self.out.push(',');
                    self.newline(depth + 1);
                    self.string("kind");
                    self.out.push_str(if self.pretty { ": " } else { ":" });
                    self.string(r.kind());
                    if let Some(name) = r.name() {
                        self.out.push(',');
                        self.newline(depth + 1);
                        self.string("name");
                        self.out.push_str(if self.pretty { ": " } else { ":" });
                        self.string(name);
                    }
                    self.newline(depth);
                    self.out.push('}');
                }
            }
            Kind::List => {
                if value.is_empty() {
                    self.out.push_str("[]");
                    return Ok(());
                }
                self.out.push('[');
                for (i, item) in value.items().enumerate() {
                    if i > 0 {
                        self.out.push(',');
                    }
                    self.newline(depth + 1);
                    self.value(item, depth + 1).map_err(|f| f.at(PathStep::Index(i)))?;
                }
                self.newline(depth);
                self.out.push(']');
            }
            Kind::Object => {
                if value.is_empty() {
                    self.out.push_str("{}");
                    return Ok(());
                }
                if resource_shape(value.entries()).is_some() {
                    return Err(Failure::new(Error::new("state.json.ambiguous_resource")));
                }
                self.out.push('{');
                for (i, (key, item)) in value.shared_entries().enumerate() {
                    if i > 0 {
                        self.out.push(',');
                    }
                    self.newline(depth + 1);
                    self.string(key);
                    self.out.push_str(if self.pretty { ": " } else { ":" });
                    self.value(item, depth + 1).map_err(|f| f.at(PathStep::Key(Arc::clone(key))))?;
                }
                self.newline(depth);
                self.out.push('}');
            }
        }
        Ok(())
    }

    /// A string with the minimal escapes: the quote, the backslash and the control characters. Everything else,
    /// non-ASCII included, is written as is (UTF-8)
    fn string(&mut self, text: &str) {
        self.out.push('"');
        let mut run_start = 0;
        for (i, b) in text.bytes().enumerate() {
            let escape = match b {
                b'"' => "\\\"",
                b'\\' => "\\\\",
                b'\n' => "\\n",
                b'\r' => "\\r",
                b'\t' => "\\t",
                0x08 => "\\b",
                0x0c => "\\f",
                0x00..=0x1f => "",
                _ => continue,
            };
            self.out.push_str(&text[run_start..i]);
            if escape.is_empty() {
                let _ = write!(self.out, "\\u{b:04x}");
            } else {
                self.out.push_str(escape);
            }
            run_start = i + 1;
        }
        self.out.push_str(&text[run_start..]);
        self.out.push('"');
    }
}

#[cfg(test)]
mod tests {
    use super::super::from_json;
    use super::*;
    use crate::resource::Resource;

    const HASH: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    fn real(v: f64) -> Value {
        Value::real(v).unwrap()
    }

    #[test]
    fn the_stable_form() {
        let v = Value::object([
            ("volume", real(0.8)),
            ("name", Value::text("Baixo \"1\"\n")),
            ("empty", Value::empty_list()),
            ("eo", Value::empty_object()),
            ("timeSig", Value::list([Value::int(4), Value::int(4)])),
            ("source", Value::resource(Resource::new("audio", HASH).unwrap())),
            ("parent", Value::NULL),
            ("mute", Value::bool(false)),
        ]);
        let expected = format!(
            "{{\n  \"empty\": [],\n  \"eo\": {{}},\n  \"mute\": false,\n  \"name\": \"Baixo \\\"1\\\"\\n\",\n  \
             \"parent\": null,\n  \"source\": {{\n    \"key\": \"{HASH}\",\n    \"kind\": \"audio\"\n  }},\n  \
             \"timeSig\": [\n    4,\n    4\n  ],\n  \"volume\": 0.8\n}}\n"
        );
        assert_eq!(to_json(&v).unwrap(), expected);
        assert_eq!(from_json(&expected).unwrap(), v);
        let compact = to_json_compact(&v).unwrap();
        assert!(!compact.contains('\n'));
        assert_eq!(from_json(&compact).unwrap(), v);
    }

    #[test]
    fn reals_always_look_real_and_round_trip() {
        for (v, text) in [
            (1.0, "1.0"),
            (-0.0, "-0.0"),
            (0.0, "0.0"),
            (1e300, "1e300"),
            (1.5e-7, "1.5e-7"),
            (0.1, "0.1"),
            (123456789.0, "123456789.0"),
        ] {
            let json = to_json_compact(&real(v)).unwrap();
            assert_eq!(json, text);
            assert_eq!(from_json(&json).unwrap().as_real().unwrap().to_bits(), v.to_bits());
        }
        for v in [f64::MIN_POSITIVE, f64::MAX, f64::MIN, 5e-324, -5e-324, 1.0 / 3.0, std::f64::consts::PI] {
            let back = from_json(&to_json(&real(v)).unwrap()).unwrap();
            assert_eq!(back.as_real().unwrap().to_bits(), v.to_bits(), "{v}");
        }
    }

    #[test]
    fn ints_beyond_2_pow_53_are_refused_both_ways() {
        assert_eq!(to_json(&Value::int(MAX_SAFE_INT)).unwrap(), "9007199254740991\n");
        assert_eq!(to_json(&Value::int(-MAX_SAFE_INT)).unwrap(), "-9007199254740991\n");
        let e = to_json(&Value::list([Value::int(1 << 53)])).unwrap_err();
        assert_eq!(e.key(), "state.json.int_out_of_range");
        assert!(e.to_string().contains("path=\"/0\""), "{e}");
        assert_eq!(to_json(&Value::int(i64::MIN)).unwrap_err().key(), "state.json.int_out_of_range");
        for bad in ["9007199254740992", "-9007199254740992", "18446744073709551616", "99999999999999999999999"] {
            assert_eq!(from_json(bad).unwrap_err().key(), "state.json.int_out_of_range", "{bad}");
        }
        assert_eq!(from_json("9007199254740991").unwrap(), Value::int(MAX_SAFE_INT));
        // The same digits with a fraction are a real
        assert_eq!(from_json("9007199254740992.0").unwrap(), real(9007199254740992.0));
    }

    #[test]
    fn nesting_beyond_what_the_reader_accepts_is_refused_on_write() {
        let nest = |levels: usize| {
            let mut value = Value::int(1);
            for _ in 0..levels {
                value = Value::list([value]);
            }
            value
        };
        let ok = to_json_compact(&nest(MAX_DEPTH)).expect("at the limit");
        assert!(from_json(&ok).is_ok(), "what is written reads back");
        let err = to_json_compact(&nest(MAX_DEPTH + 1)).expect_err("one too deep");
        assert_eq!(err.key(), "state.json.too_deep");
    }

    #[test]
    fn control_characters_are_escaped() {
        let v = Value::text("\u{0}\u{1f}\u{7f}é\u{2028}\\/");
        let json = to_json_compact(&v).unwrap();
        assert_eq!(json, "\"\\u0000\\u001f\u{7f}é\u{2028}\\\\/\"");
        assert_eq!(from_json(&json).unwrap(), v);
    }

    #[test]
    fn the_resource_shape_is_reserved() {
        let shaped = Value::object([("kind", Value::text("audio")), ("key", Value::text(HASH))]);
        let e = to_json(&Value::object([("x", shaped)])).unwrap_err();
        assert_eq!(e.key(), "state.json.ambiguous_resource");
        assert!(e.to_string().contains("/x"), "{e}");
        // Not a valid resource: an ordinary object, both ways
        let scale = Value::object([("kind", Value::text("scale")), ("key", Value::text("C#"))]);
        let json = to_json(&scale).unwrap();
        assert_eq!(from_json(&json).unwrap(), scale);
        // A path as a key is never a resource
        let evil = from_json(r#"{"kind": "audio", "key": "../../etc/passwd"}"#).unwrap();
        assert!(evil.as_resource().is_none());
        // Three keys: an ordinary object
        let named = Value::object([
            ("kind", Value::text("audio")),
            ("key", Value::text(HASH)),
            ("name", Value::text("Voz take 3")),
        ]);
        assert_eq!(from_json(&to_json(&named).unwrap()).unwrap(), named);
    }
}
