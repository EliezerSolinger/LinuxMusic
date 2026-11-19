//! Node ids (interfaces.md, I-3.2): a kind letter and a number from the project's one global counter, written
//! `"t:17"` in the tree and in JSON (ids are text, because integers above 2^53 lose precision in JSON).

use std::fmt;
use std::str::FromStr;

use lm_base::{Error, Result};

/// A node's id: a kind letter (`b't'` track, `b'c'` item, `b'p'` plugin, `b'm'` mixer channel, `b'o'` output...)
/// and a number from the document's global counter, never given twice (R7, S12). Note numbers come from the same
/// counter but are not ids: a note is reached by its item and its `n` ([`crate::PathStep::Note`]).
///
/// The fields are public (the spec's shape); [`NodeId::new`] and [`NodeId::parse`] only build valid ones, and
/// [`NodeId::is_valid`] tells whether a hand-made one is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    pub kind: u8,
    pub number: u64,
}

/// The root of the project tree: `"r:0"`
pub const ROOT: NodeId = NodeId { kind: b'r', number: 0 };

impl NodeId {
    /// A checked id: the kind must be an ASCII lowercase letter
    pub fn new(kind: u8, number: u64) -> Result<Self> {
        if is_kind_letter(kind) {
            Ok(Self { kind, number })
        } else {
            Err(Error::new("state.id.invalid_kind").arg("kind", i64::from(kind)))
        }
    }

    /// Whether the kind is an ASCII lowercase letter (the only ids [`NodeId::parse`] gives back)
    pub fn is_valid(self) -> bool {
        is_kind_letter(self.kind)
    }

    /// Reads `"t:17"`. The form is canonical: one lowercase letter, a colon, and a decimal number without sign or
    /// leading zeros that fits in a `u64`. Anything else is refused, so an id has exactly one text form
    pub fn parse(text: &str) -> Result<Self> {
        let invalid = || Error::new("state.id.invalid").arg("text", truncated(text));
        let bytes = text.as_bytes();
        if bytes.len() < 3 || bytes[1] != b':' || !is_kind_letter(bytes[0]) {
            return Err(invalid());
        }
        let number = parse_canonical_u64(&text[2..]).ok_or_else(invalid)?;
        Ok(Self { kind: bytes[0], number })
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", char::from(self.kind), self.number)
    }
}

impl FromStr for NodeId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

fn is_kind_letter(kind: u8) -> bool {
    kind.is_ascii_lowercase()
}

/// A decimal `u64` in its one canonical form: digits only, no sign, no leading zero (except `"0"` itself)
pub(crate) fn parse_canonical_u64(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > 20 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if bytes.len() > 1 && bytes[0] == b'0' {
        return None;
    }
    text.parse().ok()
}

/// For error arguments: the start of a text that may be huge or hostile
pub(crate) fn truncated(text: &str) -> String {
    const MAX: usize = 64;
    if text.len() <= MAX {
        return text.to_owned();
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_form_round_trips() {
        let id = NodeId::new(b't', 17).unwrap();
        assert_eq!(id.to_string(), "t:17");
        assert_eq!(NodeId::parse("t:17").unwrap(), id);
        assert_eq!(ROOT.to_string(), "r:0");
        assert_eq!("r:0".parse::<NodeId>().unwrap(), ROOT);
        let max = NodeId::new(b'n', u64::MAX).unwrap();
        assert_eq!(NodeId::parse(&max.to_string()).unwrap(), max);
    }

    #[test]
    fn only_the_canonical_form_is_read() {
        for bad in [
            "",
            "t",
            "t:",
            "t:x",
            "T:1",
            "1:1",
            "t:01",
            "t:+1",
            "t:-1",
            "t: 1",
            "t:1 ",
            "tt:1",
            "t;1",
            "t:18446744073709551616",
            "é:1",
            "t:1.0",
        ] {
            assert_eq!(NodeId::parse(bad).unwrap_err().key(), "state.id.invalid", "{bad:?}");
        }
        assert_eq!(NodeId::new(b'T', 1).unwrap_err().key(), "state.id.invalid_kind");
        assert!(!NodeId { kind: b'0', number: 1 }.is_valid());
    }
}
