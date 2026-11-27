//! Open Sound Control 1.0 (`decisoes-autonomas.md`, 26/11, item 5): messages and bundles encoded and decoded, in pure
//! Rust, without dependencies.
//!
//! Argument types: `i` (32-bit integer), `f` (32-bit float), `s` (string), `b` (blob), `T` and `F` (true and false,
//! which carry no data). A bundle has a time tag and holds messages and bundles. A packet with another type tag, a
//! size that does not fit, or a nesting deeper than [`MAX_DEPTH`] is refused with a key (`osc.*`), never a panic.
//! Address patterns (`?*[]{}`) are not expanded: an address is matched as it is written.

#![forbid(unsafe_code)]

use std::fmt;

/// The deepest nesting of bundles read
pub const MAX_DEPTH: usize = 8;
/// The most arguments of a message read
pub const MAX_ARGS: usize = 256;

/// A failure, as a translation key with the byte offset where it happened
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OscError {
    pub key: &'static str,
    pub at: usize,
}

impl fmt::Display for OscError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.key, self.at)
    }
}

impl std::error::Error for OscError {}

pub type Result<T> = std::result::Result<T, OscError>;

fn fail<T>(key: &'static str, at: usize) -> Result<T> {
    Err(OscError { key, at })
}

/// One argument of a message
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Int(i32),
    Float(f32),
    Str(String),
    Blob(Vec<u8>),
    True,
    False,
}

impl Arg {
    fn tag(&self) -> char {
        match self {
            Arg::Int(_) => 'i',
            Arg::Float(_) => 'f',
            Arg::Str(_) => 's',
            Arg::Blob(_) => 'b',
            Arg::True => 'T',
            Arg::False => 'F',
        }
    }

    /// The argument as a number: an integer, a float, or `T` and `F` as 1 and 0
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Arg::Int(i) => Some(*i as f32),
            Arg::Float(f) => Some(*f),
            Arg::True => Some(1.0),
            Arg::False => Some(0.0),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Arg::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// A message: an address and its arguments
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub addr: String,
    pub args: Vec<Arg>,
}

impl Message {
    pub fn new(addr: impl Into<String>, args: Vec<Arg>) -> Self {
        Self { addr: addr.into(), args }
    }
}

/// A time tag: the seconds since 1900 in the high 32 bits and the fraction of a second in the low ones
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeTag(pub u64);

impl TimeTag {
    /// "Immediately" (the value 1)
    pub const IMMEDIATELY: TimeTag = TimeTag(1);

    pub fn seconds(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub fn fraction(self) -> u32 {
        self.0 as u32
    }

    pub fn is_immediate(self) -> bool {
        self.0 <= 1
    }

    /// The tag of a number of seconds since 1900
    pub fn from_secs_f64(s: f64) -> TimeTag {
        let s = s.max(0.0);
        TimeTag(((s.trunc() as u64) << 32) | (s.fract() * 4_294_967_296.0) as u64)
    }

    pub fn as_secs_f64(self) -> f64 {
        f64::from(self.seconds()) + f64::from(self.fraction()) / 4_294_967_296.0
    }
}

/// A bundle: a time tag and its elements
#[derive(Clone, Debug, PartialEq)]
pub struct Bundle {
    pub time: TimeTag,
    pub items: Vec<Packet>,
}

/// What a datagram holds
#[derive(Clone, Debug, PartialEq)]
pub enum Packet {
    Message(Message),
    Bundle(Bundle),
}

impl Packet {
    /// Every message of the packet in order, with the time tag of the bundle that holds it (the innermost)
    pub fn flatten(self) -> Vec<(TimeTag, Message)> {
        fn go(p: Packet, time: TimeTag, out: &mut Vec<(TimeTag, Message)>) {
            match p {
                Packet::Message(m) => out.push((time, m)),
                Packet::Bundle(b) => {
                    for i in b.items {
                        go(i, b.time, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        go(self, TimeTag::IMMEDIATELY, &mut out);
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Packet::Message(m) => m.write(out),
            Packet::Bundle(b) => {
                write_str(out, "#bundle");
                out.extend_from_slice(&b.time.0.to_be_bytes());
                for item in &b.items {
                    let mut body = Vec::new();
                    item.write(&mut body);
                    out.extend_from_slice(&(body.len() as i32).to_be_bytes());
                    out.extend_from_slice(&body);
                }
            }
        }
    }
}

/// A string and its NULs up to a multiple of four bytes (at least one NUL)
fn write_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    let pad = 4 - s.len() % 4;
    out.extend(std::iter::repeat_n(0u8, pad));
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut Vec<u8>) {
        write_str(out, &self.addr);
        let tags: String = std::iter::once(',').chain(self.args.iter().map(Arg::tag)).collect();
        write_str(out, &tags);
        for a in &self.args {
            match a {
                Arg::Int(i) => out.extend_from_slice(&i.to_be_bytes()),
                Arg::Float(f) => out.extend_from_slice(&f.to_bits().to_be_bytes()),
                Arg::Str(s) => write_str(out, s),
                Arg::Blob(b) => {
                    out.extend_from_slice(&(b.len() as i32).to_be_bytes());
                    out.extend_from_slice(b);
                    out.extend(std::iter::repeat_n(0u8, (4 - b.len() % 4) % 4));
                }
                Arg::True | Arg::False => {}
            }
        }
    }
}

/// A reader over the bytes of one packet
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    base: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.data.len());
        match end {
            Some(e) => {
                let s = &self.data[self.pos..e];
                self.pos = e;
                Ok(s)
            }
            None => fail("osc.truncated", self.base + self.pos),
        }
    }

    fn i32(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A NUL-terminated string padded to a multiple of four bytes
    fn string(&mut self) -> Result<String> {
        let at = self.base + self.pos;
        let rest = &self.data[self.pos..];
        let Some(len) = rest.iter().position(|b| *b == 0) else { return fail("osc.unterminated", at) };
        let padded = (len / 4 + 1) * 4;
        if padded > rest.len() {
            return fail("osc.truncated", at);
        }
        if rest[len..padded].iter().any(|b| *b != 0) {
            return fail("osc.bad_padding", at);
        }
        let s = std::str::from_utf8(&rest[..len]).map_err(|_| OscError { key: "osc.not_utf8", at })?.to_owned();
        self.pos += padded;
        Ok(s)
    }
}

/// Reads one datagram
pub fn decode(data: &[u8]) -> Result<Packet> {
    decode_at(data, 0, 0)
}

fn decode_at(data: &[u8], base: usize, depth: usize) -> Result<Packet> {
    if data.len() % 4 != 0 {
        return fail("osc.size", base);
    }
    if data.starts_with(b"#bundle\0") {
        if depth >= MAX_DEPTH {
            return fail("osc.too_deep", base);
        }
        let mut r = Reader { data, pos: 8, base };
        let t = r.take(8)?;
        let time = TimeTag(u64::from_be_bytes([t[0], t[1], t[2], t[3], t[4], t[5], t[6], t[7]]));
        let mut items = Vec::new();
        while r.pos < data.len() {
            let at = base + r.pos;
            let n = r.i32()?;
            let Ok(n) = usize::try_from(n) else { return fail("osc.size", at) };
            let start = r.pos;
            let body = r.take(n)?;
            items.push(decode_at(body, base + start, depth + 1)?);
        }
        return Ok(Packet::Bundle(Bundle { time, items }));
    }
    let mut r = Reader { data, pos: 0, base };
    let addr = r.string()?;
    if !addr.starts_with('/') {
        return fail("osc.address", base);
    }
    // A message without a type tag string (old senders) has no arguments
    if r.pos >= data.len() {
        return Ok(Packet::Message(Message { addr, args: Vec::new() }));
    }
    let tags_at = base + r.pos;
    let tags = r.string()?;
    let Some(tags) = tags.strip_prefix(',') else { return fail("osc.type_tags", tags_at) };
    if tags.len() > MAX_ARGS {
        return fail("osc.too_many_args", tags_at);
    }
    let mut args = Vec::with_capacity(tags.len());
    for t in tags.chars() {
        args.push(match t {
            'i' => Arg::Int(r.i32()?),
            'f' => Arg::Float(f32::from_bits(r.i32()? as u32)),
            's' => Arg::Str(r.string()?),
            'b' => {
                let at = base + r.pos;
                let Ok(n) = usize::try_from(r.i32()?) else { return fail("osc.size", at) };
                let blob = r.take(n)?.to_vec();
                r.take((4 - n % 4) % 4)?;
                Arg::Blob(blob)
            }
            'T' => Arg::True,
            'F' => Arg::False,
            _ => return fail("osc.type_tag", tags_at),
        });
    }
    if r.pos != data.len() {
        return fail("osc.trailing", base + r.pos);
    }
    Ok(Packet::Message(Message { addr, args }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_the_bytes_of_the_specification() {
        // The example of the OSC 1.0 specification: "/oscillator/4/frequency" with the float 440.0
        let m = Message::new("/oscillator/4/frequency", vec![Arg::Float(440.0)]);
        let bytes = m.encode();
        let mut want = b"/oscillator/4/frequency\0".to_vec();
        want.extend_from_slice(b",f\0\0");
        want.extend_from_slice(&440.0f32.to_bits().to_be_bytes());
        assert_eq!(bytes, want);
        assert_eq!(decode(&bytes).unwrap(), Packet::Message(m));
    }

    #[test]
    fn every_argument_type_round_trips() {
        let m = Message::new(
            "/x",
            vec![
                Arg::Int(-7),
                Arg::Float(0.25),
                Arg::Str("olá".into()),
                Arg::Blob(vec![1, 2, 3]),
                Arg::True,
                Arg::False,
                Arg::Str(String::new()),
                Arg::Blob(Vec::new()),
            ],
        );
        let bytes = m.encode();
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(decode(&bytes).unwrap(), Packet::Message(m));
    }

    #[test]
    fn a_message_with_no_arguments_and_one_without_type_tags() {
        let m = Message::new("/transport/play", Vec::new());
        assert_eq!(decode(&m.encode()).unwrap(), Packet::Message(m.clone()));
        assert_eq!(decode(b"/transport/play\0").unwrap(), Packet::Message(m));
    }

    #[test]
    fn bundles_nest_and_flatten_with_their_time_tags() {
        let a = Message::new("/a", vec![Arg::Int(1)]);
        let b = Message::new("/b", vec![Arg::Float(2.0)]);
        let tag = TimeTag::from_secs_f64(3_900_000_000.5);
        let inner = Packet::Bundle(Bundle { time: tag, items: vec![Packet::Message(b.clone())] });
        let outer =
            Packet::Bundle(Bundle { time: TimeTag::IMMEDIATELY, items: vec![Packet::Message(a.clone()), inner] });
        let back = decode(&outer.encode()).unwrap();
        assert_eq!(back, outer);
        assert_eq!(back.flatten(), vec![(TimeTag::IMMEDIATELY, a), (tag, b)]);
        assert!((tag.as_secs_f64() - 3_900_000_000.5).abs() < 1e-6);
        assert!(TimeTag::IMMEDIATELY.is_immediate() && !tag.is_immediate());
    }

    #[test]
    fn what_is_not_osc_is_refused_with_a_key() {
        assert_eq!(decode(b"").unwrap_err().key, "osc.unterminated");
        assert_eq!(decode(b"abc").unwrap_err().key, "osc.size");
        assert_eq!(decode(b"abcd").unwrap_err().key, "osc.unterminated");
        assert_eq!(decode(b"x\0\0\0").unwrap_err().key, "osc.address");
        assert_eq!(decode(b"/x\0\0,q\0\0").unwrap_err().key, "osc.type_tag");
        assert_eq!(decode(b"/x\0\0,i\0\0").unwrap_err().key, "osc.truncated");
        assert_eq!(decode(b"/x\0\0,\0\0\0\0\0\0\0").unwrap_err().key, "osc.trailing");
        let mut blob = b"/x\0\0,b\0\0".to_vec();
        blob.extend_from_slice(&1000i32.to_be_bytes());
        assert_eq!(decode(&blob).unwrap_err().key, "osc.truncated");
        let mut neg = b"/x\0\0,b\0\0".to_vec();
        neg.extend_from_slice(&(-4i32).to_be_bytes());
        assert_eq!(decode(&neg).unwrap_err().key, "osc.size");
    }

    #[test]
    fn a_bundle_nested_too_deep_is_refused() {
        let mut p = Packet::Message(Message::new("/a", Vec::new()));
        for _ in 0..=MAX_DEPTH {
            p = Packet::Bundle(Bundle { time: TimeTag::IMMEDIATELY, items: vec![p] });
        }
        assert_eq!(decode(&p.encode()).unwrap_err().key, "osc.too_deep");
    }

    #[test]
    fn numbers_read_as_floats() {
        assert_eq!(Arg::Int(3).as_f32(), Some(3.0));
        assert_eq!(Arg::True.as_f32(), Some(1.0));
        assert_eq!(Arg::Str("a".into()).as_f32(), None);
        assert_eq!(Arg::Str("a".into()).as_str(), Some("a"));
    }
}
