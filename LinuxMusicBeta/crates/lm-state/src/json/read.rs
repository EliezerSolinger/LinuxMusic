//! The JSON reader: strict RFC 8259, with the tree's rules. Our own and not `serde_json`, because the tree needs what
//! `serde_json` cannot give without changing its behaviour for the whole workspace (see `decisoes`): an integer
//! literal beyond `u64` must be refused and not silently become a real; reals must be correctly rounded so the round
//! trip is bit-exact (`str::parse::<f64>` is); and a repeated key must be an error, not "last wins".

use std::sync::Arc;

use lm_base::{Error, Result};

use super::{MAX_SAFE_INT, resource_shape};
use crate::resource::Resource;
use crate::value::{List, Map, Value};

/// The deepest nesting of lists and objects the reader accepts. A project is a dozen levels deep; the limit keeps a
/// hostile file from exhausting the stack
pub const MAX_DEPTH: usize = 128;

/// Reads a JSON document into a tree. Never panics; every problem is an error with its position (`offset` in bytes,
/// `line` and `column` from 1)
pub fn from_json(text: &str) -> Result<Value> {
    let mut reader = Reader { text, bytes: text.as_bytes(), pos: 0 };
    reader.skip_whitespace();
    let value = reader.value(0)?;
    reader.skip_whitespace();
    if reader.pos != reader.bytes.len() {
        return Err(reader.error("state.json.trailing"));
    }
    Ok(value)
}

/// Like [`from_json`], from bytes that must be UTF-8 (no byte order mark)
pub fn from_json_bytes(bytes: &[u8]) -> Result<Value> {
    match std::str::from_utf8(bytes) {
        Ok(text) => from_json(text),
        Err(e) => Err(Error::new("state.json.not_utf8").arg("offset", offset_arg(e.valid_up_to()))),
    }
}

fn offset_arg(offset: usize) -> i64 {
    i64::try_from(offset).unwrap_or(i64::MAX)
}

struct Reader<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn error(&self, key: &'static str) -> Error {
        self.error_at(key, self.pos)
    }

    fn error_at(&self, key: &'static str, offset: usize) -> Error {
        let offset = offset.min(self.bytes.len());
        let before = &self.bytes[..offset];
        let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
        let line_start = before.iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
        // Columns count characters, not bytes
        let column = self.text.get(line_start..offset).map_or(offset - line_start, |s| s.chars().count()) + 1;
        Error::new(key)
            .arg("offset", offset_arg(offset))
            .arg("line", offset_arg(line))
            .arg("column", offset_arg(column))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn expect_literal(&mut self, literal: &[u8], value: Value) -> Result<Value> {
        if self.bytes[self.pos..].starts_with(literal) {
            self.pos += literal.len();
            Ok(value)
        } else {
            Err(self.error("state.json.syntax"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value> {
        match self.peek() {
            None => Err(self.error("state.json.unexpected_end")),
            Some(b'n') => self.expect_literal(b"null", Value::NULL),
            Some(b't') => self.expect_literal(b"true", Value::bool(true)),
            Some(b'f') => self.expect_literal(b"false", Value::bool(false)),
            Some(b'"') => Ok(Value::text(self.string()?)),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b'[') => self.list(depth + 1),
            Some(b'{') => self.object(depth + 1),
            Some(_) => Err(self.error("state.json.syntax")),
        }
    }

    fn list(&mut self, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(self.error("state.json.too_deep"));
        }
        self.pos += 1; // '['
        let mut list = List::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::from_vector(list));
        }
        loop {
            self.skip_whitespace();
            list.push_back(self.value(depth)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::from_vector(list));
                }
                None => return Err(self.error("state.json.unexpected_end")),
                Some(_) => return Err(self.error("state.json.syntax")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(self.error("state.json.too_deep"));
        }
        self.pos += 1; // '{'
        let mut map = Map::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::from_map(map));
        }
        loop {
            self.skip_whitespace();
            let key_at = self.pos;
            match self.peek() {
                Some(b'"') => {}
                None => return Err(self.error("state.json.unexpected_end")),
                Some(_) => return Err(self.error("state.json.syntax")),
            }
            let key: Arc<str> = Arc::from(self.string()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b':') => self.pos += 1,
                None => return Err(self.error("state.json.unexpected_end")),
                Some(_) => return Err(self.error("state.json.syntax")),
            }
            self.skip_whitespace();
            let value = self.value(depth)?;
            if map.insert(key, value).is_some() {
                return Err(self.error_at("state.json.duplicate_key", key_at));
            }
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(match resource_shape(map.iter().map(|(k, v)| (&**k, v))) {
                        Some((kind, key, name)) => match Resource::new(kind, key) {
                            Ok(r) => Value::resource(match name {
                                Some(n) => r.with_name(n),
                                None => r,
                            }),
                            Err(_) => Value::from_map(map),
                        },
                        None => Value::from_map(map),
                    });
                }
                None => return Err(self.error("state.json.unexpected_end")),
                Some(_) => return Err(self.error("state.json.syntax")),
            }
        }
    }

    /// A string; `self.pos` is at the opening quote
    fn string(&mut self) -> Result<String> {
        self.pos += 1;
        let mut out = String::new();
        let mut run_start = self.pos;
        loop {
            let Some(b) = self.peek() else {
                return Err(self.error("state.json.unexpected_end"));
            };
            match b {
                b'"' => {
                    out.push_str(&self.text[run_start..self.pos]);
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    out.push_str(&self.text[run_start..self.pos]);
                    self.pos += 1;
                    self.escape(&mut out)?;
                    run_start = self.pos;
                }
                0x00..=0x1f => return Err(self.error("state.json.control_character")),
                // Any other byte, including the bytes of a multi-byte character: the input is valid UTF-8, and a
                // run only ends at an ASCII quote or backslash, so the run is always cut at a character boundary
                _ => self.pos += 1,
            }
        }
    }

    /// An escape; `self.pos` is just after the backslash
    fn escape(&mut self, out: &mut String) -> Result<()> {
        let at = self.pos - 1;
        let Some(b) = self.peek() else {
            return Err(self.error("state.json.unexpected_end"));
        };
        self.pos += 1;
        let c = match b {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let first = self.hex4()?;
                let code = match first {
                    0xd800..=0xdbff => {
                        // A high surrogate must be followed by an escaped low surrogate
                        if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                            return Err(self.error_at("state.json.lone_surrogate", at));
                        }
                        self.pos += 2;
                        let second = self.hex4()?;
                        if !(0xdc00..=0xdfff).contains(&second) {
                            return Err(self.error_at("state.json.lone_surrogate", at));
                        }
                        0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
                    }
                    0xdc00..=0xdfff => return Err(self.error_at("state.json.lone_surrogate", at)),
                    code => code,
                };
                char::from_u32(code).ok_or_else(|| self.error_at("state.json.lone_surrogate", at))?
            }
            _ => return Err(self.error_at("state.json.bad_escape", at)),
        };
        out.push(c);
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32> {
        let Some(digits) = self.bytes.get(self.pos..self.pos + 4) else {
            return Err(self.error("state.json.unexpected_end"));
        };
        let mut code = 0u32;
        for &d in digits {
            let v = match d {
                b'0'..=b'9' => d - b'0',
                b'a'..=b'f' => d - b'a' + 10,
                b'A'..=b'F' => d - b'A' + 10,
                _ => return Err(self.error("state.json.bad_escape")),
            };
            code = code * 16 + u32::from(v);
        }
        self.pos += 4;
        Ok(code)
    }

    /// A number, by the JSON grammar: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`. Without a fraction or an
    /// exponent it is an int
    fn number(&mut self) -> Result<Value> {
        let start = self.pos;
        let digits = |r: &mut Self| {
            let from = r.pos;
            while let Some(b'0'..=b'9') = r.peek() {
                r.pos += 1;
            }
            r.pos - from
        };
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            None => return Err(self.error("state.json.unexpected_end")),
            Some(_) => return Err(self.error("state.json.syntax")),
        }
        let mut real = false;
        if self.peek() == Some(b'.') {
            real = true;
            self.pos += 1;
            if digits(self) == 0 {
                return Err(self.error("state.json.syntax"));
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            real = true;
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if digits(self) == 0 {
                return Err(self.error("state.json.syntax"));
            }
        }
        let token = &self.text[start..self.pos];
        if real {
            match token.parse::<f64>() {
                Ok(v) if v.is_finite() => Value::real(v),
                _ => Err(self.error_at("state.json.real_out_of_range", start)),
            }
        } else {
            match token.parse::<i64>() {
                Ok(v) if (-MAX_SAFE_INT..=MAX_SAFE_INT).contains(&v) => Ok(Value::int(v)),
                _ => Err(self.error_at("state.json.int_out_of_range", start)),
            }
        }
    }
}
