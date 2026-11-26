//! Resources (interfaces.md, C9, C10, C13, U24): audio files and large opaque state live outside the tree, in the
//! project folder, named by the BLAKE3 hash of their content. The tree holds only `{ kind, key }`.

use std::fmt;
use std::sync::Arc;

use lm_base::{Error, Result};

use crate::id::truncated;

/// The length of a resource key: a BLAKE3 hash (256 bits) in lowercase hexadecimal
pub const RESOURCE_KEY_LEN: usize = 64;

/// The longest resource kind (`"audio"`, `"state"`, `"soundfont"`...)
pub const RESOURCE_KIND_MAX_LEN: usize = 32;

/// Opaque state above this size goes to a resource instead of the tree (U24). Applied by the schema validation to
/// every text of a node
pub const INLINE_MAX_BYTES: usize = 64 * 1024;

/// A reference to a file of the project by content: `kind` says which folder and loader (`"audio"` → `audio/`),
/// `key` is the BLAKE3 hash of the content in lowercase hexadecimal.
///
/// Both are validated when made, so a `Resource` can never name a path: a key is exactly 64 hex digits, never
/// `../`, a slash or an absolute path (C9)
///
/// It may also carry the `name` it was imported under (the file's name, for the panels to show instead of a hash). The
/// name is a label, not an identity: two resources of the same kind and key are equal, ordered and hashed alike
/// whatever their names (the value that holds one does see the difference, so that setting the same file under
/// another name is a change)
#[derive(Clone)]
pub struct Resource {
    kind: Arc<str>,
    key: Arc<str>,
    name: Option<Arc<str>>,
}

/// The longest name kept (a file's name; longer is cut)
pub const RESOURCE_NAME_MAX_LEN: usize = 255;

impl PartialEq for Resource {
    fn eq(&self, o: &Self) -> bool {
        self.kind == o.kind && self.key == o.key
    }
}
impl Eq for Resource {}
impl PartialOrd for Resource {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Resource {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (&self.kind, &self.key).cmp(&(&o.kind, &o.key))
    }
}
impl std::hash::Hash for Resource {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.kind.hash(h);
        self.key.hash(h);
    }
}

impl Resource {
    pub fn new(kind: &str, key: &str) -> Result<Self> {
        check_kind(kind)?;
        check_key(key)?;
        Ok(Self { kind: Arc::from(kind), key: Arc::from(key), name: None })
    }

    /// The resource for this content: its key is the BLAKE3 hash of `content`
    pub fn of_content(kind: &str, content: &[u8]) -> Result<Self> {
        check_kind(kind)?;
        let key = blake3::hash(content).to_hex();
        Ok(Self { kind: Arc::from(kind), key: Arc::from(key.as_str()), name: None })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// The name it was imported under, if it has one
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The same resource with this display name (empty: none; cut at [`RESOURCE_NAME_MAX_LEN`] bytes on a
    /// character boundary)
    pub fn with_name(mut self, name: &str) -> Self {
        let mut end = name.len().min(RESOURCE_NAME_MAX_LEN);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        self.name = (end > 0).then(|| Arc::from(&name[..end]));
        self
    }
}

impl fmt::Debug for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Resource({}:{})", self.kind, self.key)
    }
}

/// Whether `key` is a valid resource key: exactly 64 lowercase hexadecimal digits
pub fn is_valid_resource_key(key: &str) -> bool {
    key.len() == RESOURCE_KEY_LEN && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `kind` is a valid resource kind: 1 to 32 of `a-z`, `0-9`, `-`, `_`, starting with a letter (it names a
/// folder and a loader, so it can never be a path either)
pub fn is_valid_resource_kind(kind: &str) -> bool {
    let bytes = kind.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= RESOURCE_KIND_MAX_LEN
        && bytes[0].is_ascii_lowercase()
        && bytes.iter().all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn check_kind(kind: &str) -> Result<()> {
    if is_valid_resource_kind(kind) {
        Ok(())
    } else {
        Err(Error::new("state.resource.invalid_kind").arg("kind", truncated(kind)))
    }
}

fn check_key(key: &str) -> Result<()> {
    if is_valid_resource_key(key) {
        Ok(())
    } else {
        Err(Error::new("state.resource.invalid_key").arg("key", truncated(key)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    #[test]
    fn valid_keys_are_accepted() {
        let r = Resource::new("audio", HASH).unwrap();
        assert_eq!(r.kind(), "audio");
        assert_eq!(r.key(), HASH);
    }

    #[test]
    fn content_hash_is_blake3() {
        // The BLAKE3 hash of the empty input
        assert_eq!(Resource::of_content("state", b"").unwrap().key(), HASH);
        let r = Resource::of_content("audio", b"take 3").unwrap();
        assert!(is_valid_resource_key(r.key()));
    }

    #[test]
    fn paths_and_malformed_keys_are_refused() {
        let upper = HASH.to_uppercase();
        let short = &HASH[..63];
        let long = format!("{HASH}0");
        let slash = format!("{}/{}", &HASH[..31], &HASH[32..]);
        let dots = format!("../{}", &HASH[3..]);
        let absolute = format!("/{}", &HASH[1..]);
        let non_hex = format!("g{}", &HASH[1..]);
        let backslash = format!("..\\{}", &HASH[3..]);
        let nul = format!("\0{}", &HASH[1..]);
        let unicode = format!("é{}", &HASH[2..]);
        for bad in [
            "",
            "../../etc/passwd",
            "/etc/passwd",
            "audio/x.wav",
            short,
            &long,
            &upper,
            &slash,
            &dots,
            &absolute,
            &non_hex,
            &backslash,
            &nul,
            &unicode,
        ] {
            assert_eq!(Resource::new("audio", bad).unwrap_err().key(), "state.resource.invalid_key", "{bad:?}");
        }
    }

    #[test]
    fn kinds_are_plain_names() {
        for bad in ["", "Audio", "1audio", "../audio", "au/dio", "au dio", ".audio", &"a".repeat(33)] {
            assert_eq!(Resource::new(bad, HASH).unwrap_err().key(), "state.resource.invalid_kind", "{bad:?}");
        }
        for good in ["audio", "state", "soundfont", "drum-kit", "x_2"] {
            assert!(Resource::new(good, HASH).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn the_name_is_a_label_not_an_identity() {
        let a = Resource::new("audio", HASH).unwrap();
        let b = a.clone().with_name("kick.wav");
        assert_eq!(a, b);
        assert_eq!(b.name(), Some("kick.wav"));
        assert_eq!(a.name(), None);
        assert_eq!(b.clone().with_name("").name(), None);
        assert_ne!(crate::Value::resource(a), crate::Value::resource(b));
    }

    #[test]
    fn the_name_round_trips_through_json() {
        let r = Resource::new("audio", HASH).unwrap().with_name("a \"b\" é.pat");
        let v = crate::Value::object([("patch", crate::Value::resource(r))]);
        let text = crate::to_json(&v).unwrap();
        assert!(text.contains("\"name\""));
        assert_eq!(crate::from_json(&text).unwrap(), v);
    }
}
