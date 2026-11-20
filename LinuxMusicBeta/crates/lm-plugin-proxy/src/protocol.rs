//! The control channel between the core and a helper (I-9.6): one JSON object per line (the protocol's own JSON,
//! `lm-state`) on the helper's standard input and output. Versioned: the helper's first line says its
//! [`PROTOCOL`], and a different one is refused. Only slow things go here (load, state, GUI, scan results); audio and
//! events go through the shared block.

use std::io::{BufRead, Write};

use lm_base::{Error, Result};
use lm_state::Value;

/// The control protocol's version
pub const PROTOCOL: i64 = 1;

pub fn write_message(w: &mut impl Write, v: &Value) -> Result<()> {
    let line = lm_state::to_json_compact(v)?;
    let io = |e: std::io::Error| Error::new("plugin_host.pipe").arg("error", e.to_string());
    w.write_all(line.as_bytes()).map_err(io)?;
    w.write_all(b"\n").map_err(io)?;
    w.flush().map_err(io)
}

/// The next message; `Ok(None)` at the end of the stream. Lines over 16 MB are refused
pub fn read_message(r: &mut impl BufRead) -> Result<Option<Value>> {
    let mut line = String::new();
    let n = std::io::Read::take(&mut *r, 16 << 20)
        .read_line(&mut line)
        .map_err(|e| Error::new("plugin_host.pipe").arg("error", e.to_string()))?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(lm_state::from_json(line.trim_end())?))
}

/// Bytes as lowercase hex (a state chunk in a message or in the tree)
pub fn hex(bytes: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

pub fn unhex(s: &str) -> Result<Vec<u8>> {
    let bad = || Error::new("plugin_host.hex");
    if !s.len().is_multiple_of(2) {
        return Err(bad());
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        _ => Err(bad()),
    };
    s.as_bytes().chunks(2).map(|p| Ok((digit(p[0])? << 4) | digit(p[1])?)).collect()
}

pub fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_text).ok_or_else(|| Error::new("plugin_host.message").arg("field", key))
}

pub fn int(v: &Value, key: &str) -> Result<i64> {
    v.get(key).and_then(Value::as_int).ok_or_else(|| Error::new("plugin_host.message").arg("field", key))
}

/// A label sent as `{key, args}` (args as text), interned on arrival (keys are a finite set; the table is bounded)
pub fn error_to(e: &Error) -> Value {
    let args = e.label.args.iter().map(|(k, a)| {
        let t = match a {
            lm_base::Arg::Int(i) => i.to_string(),
            lm_base::Arg::Real(r) => r.to_string(),
            lm_base::Arg::Text(t) => t.clone(),
        };
        Value::list([Value::text(*k), Value::text(t)])
    });
    Value::object([("key", Value::text(e.key())), ("args", Value::list(args))])
}

pub fn error_from(v: &Value) -> Error {
    let key = v.get("key").and_then(Value::as_text).unwrap_or("plugin_host.failed");
    let mut e = Error::new(intern(key));
    for a in v.get("args").into_iter().flat_map(Value::items) {
        if let (Some(k), Some(t)) = (a.at(0).and_then(Value::as_text), a.at(1).and_then(Value::as_text)) {
            e = e.arg(intern(k), t);
        }
    }
    e
}

fn intern(key: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock, PoisonError};
    static T: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    if key.len() > 160 {
        return "plugin_host.failed";
    }
    let mut t = T.get_or_init(|| Mutex::new(HashSet::new())).lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(k) = t.get(key) {
        return k;
    }
    if t.len() > 4096 {
        return "plugin_host.failed";
    }
    let k: &'static str = Box::leak(key.to_owned().into_boxed_str());
    t.insert(k);
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_hex_and_errors() {
        let mut buf = Vec::new();
        write_message(&mut buf, &Value::object([("t", Value::text("x"))])).unwrap();
        let mut r = std::io::BufReader::new(buf.as_slice());
        assert_eq!(read_message(&mut r).unwrap().unwrap().get("t").unwrap().as_text(), Some("x"));
        assert!(read_message(&mut r).unwrap().is_none());
        assert_eq!(unhex(&hex(&[0, 1, 0xab, 255])).unwrap(), vec![0, 1, 0xab, 255]);
        assert!(unhex("abc").is_err() && unhex("zz").is_err());
        let e = Error::new("lilv.missing_feature").arg("feature", "x");
        assert_eq!(error_from(&error_to(&e)), e);
    }
}
