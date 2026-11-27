//! A plugin's state (state:interface) as an opaque chunk: the `"chunk"` of a third-party plugin in the project tree
//! (arquitetura-nova.md 7a).
//!
//! The chunk holds the properties the plugin stores, by **URI** (URIDs are per process), sorted by key so that the
//! same state always gives the same bytes. Format, little-endian:
//!
//! ```text
//! "LMLV2ST\0"  u32 version (1)  u32 count
//! count × { u32 key length, key URI, u32 type length, type URI, u32 flags, u64 value length, value }
//! ```
//!
//! Only plain-old-data values are accepted (`LV2_STATE_IS_POD`): the host copies bytes and cannot copy anything else.
//! Paths go through state:mapPath unchanged (absolute), so a chunk that names files is tied to this machine.

use crate::ffi;
use crate::urid::UridMap;
use lm_base::{Error, Result};
use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

const MAGIC: &[u8; 8] = b"LMLV2ST\0";
const VERSION: u32 = 1;

/// One stored property
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Property {
    pub type_uri: String,
    pub flags: u32,
    pub value: Vec<u8>,
}

/// What a save collects
pub(crate) struct SaveContext<'a> {
    pub map: &'a UridMap,
    pub properties: BTreeMap<String, Property>,
    /// The first property that could not be kept, if any
    pub refused: Option<Error>,
}

const MAX_PROPERTY: usize = 64 << 20;
const MAX_STATE: usize = 64 << 20;

/// `LV2_State_Store_Function`. Never unwinds into C
pub(crate) unsafe extern "C" fn store(
    handle: *mut c_void,
    key: u32,
    value: *const c_void,
    size: usize,
    type_: u32,
    flags: u32,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() || (value.is_null() && size > 0) {
            return ffi::LV2_STATE_ERR_UNKNOWN;
        }
        // SAFETY: the SaveContext of the save in progress, on this thread, not otherwise borrowed during the call
        let ctx = unsafe { &mut *handle.cast::<SaveContext<'_>>() };
        let (Some(key_uri), Some(type_uri)) = (ctx.map.unmap(key), ctx.map.unmap(type_)) else {
            ctx.refused.get_or_insert_with(|| Error::new("lilv.state_bad_key").arg("key", i64::from(key)));
            return ffi::LV2_STATE_ERR_UNKNOWN;
        };
        if flags & ffi::LV2_STATE_IS_POD == 0 {
            ctx.refused.get_or_insert_with(|| {
                Error::new("lilv.state_not_pod").arg("key", key_uri.clone()).arg("type", type_uri.clone())
            });
            return ffi::LV2_STATE_ERR_BAD_FLAGS;
        }
        // The plugin decides the size: bounded per property and in total, so a save cannot grow the host without end
        let total: usize = ctx.properties.values().map(|p| p.value.len()).sum();
        if size > MAX_PROPERTY || total.saturating_add(size) > MAX_STATE {
            ctx.refused.get_or_insert_with(|| Error::new("lilv.state_too_large").arg("key", key_uri.clone()));
            return ffi::LV2_STATE_ERR_UNKNOWN;
        }
        let bytes = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: the plugin passes `size` readable bytes of plain data, valid for the call
            unsafe { std::slice::from_raw_parts(value.cast::<u8>(), size) }.to_vec()
        };
        ctx.properties.insert(key_uri, Property { type_uri, flags, value: bytes });
        ffi::LV2_STATE_SUCCESS
    }))
    .unwrap_or(ffi::LV2_STATE_ERR_UNKNOWN)
}

/// The chunk of a set of properties
pub(crate) fn encode(properties: &BTreeMap<String, Property>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(properties.len() as u32).to_le_bytes());
    for (key, p) in properties {
        out.extend_from_slice(&(key.len() as u32).to_le_bytes());
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&(p.type_uri.len() as u32).to_le_bytes());
        out.extend_from_slice(p.type_uri.as_bytes());
        out.extend_from_slice(&p.flags.to_le_bytes());
        out.extend_from_slice(&(p.value.len() as u64).to_le_bytes());
        out.extend_from_slice(&p.value);
    }
    out
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|end| *end <= self.data.len()).ok_or_else(bad_chunk)?;
        let out = &self.data[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn text(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        let bytes = self.take(n)?;
        if bytes.contains(&0) {
            return Err(bad_chunk());
        }
        String::from_utf8(bytes.to_vec()).map_err(|_| bad_chunk())
    }
}

fn bad_chunk() -> Error {
    Error::new("lilv.bad_chunk")
}

/// The properties of a chunk; strict (any trailing byte, duplicate key or unknown version is an error)
pub(crate) fn decode(chunk: &[u8]) -> Result<BTreeMap<String, Property>> {
    let mut r = Reader { data: chunk, at: 0 };
    if r.take(MAGIC.len())? != MAGIC {
        return Err(bad_chunk());
    }
    let version = r.u32()?;
    if version != VERSION {
        return Err(Error::new("lilv.chunk_version").arg("version", i64::from(version)));
    }
    let count = r.u32()?;
    let mut out = BTreeMap::new();
    for _ in 0..count {
        let key = r.text()?;
        let type_uri = r.text()?;
        let flags = r.u32()?;
        let len = usize::try_from(r.u64()?).map_err(|_| bad_chunk())?;
        let value = r.take(len)?.to_vec();
        if out.insert(key, Property { type_uri, flags, value }).is_some() {
            return Err(bad_chunk());
        }
    }
    if r.at != chunk.len() {
        return Err(bad_chunk());
    }
    Ok(out)
}

/// One property ready for `retrieve`: the value in 8-aligned memory (a plugin may read it as a struct)
pub(crate) struct Retrievable {
    pub key: u32,
    pub type_: u32,
    pub flags: u32,
    pub size: usize,
    pub value: Box<[u64]>,
}

/// What a restore offers
pub(crate) struct RestoreContext {
    pub properties: Vec<Retrievable>,
}

impl RestoreContext {
    pub fn new(map: &UridMap, properties: &BTreeMap<String, Property>) -> Self {
        let properties = properties
            .iter()
            .map(|(key, p)| {
                let mut value = vec![0_u64; p.value.len().div_ceil(8)].into_boxed_slice();
                // SAFETY: `value` has at least `p.value.len()` bytes; u8 writes into u64 storage are always valid
                unsafe {
                    std::ptr::copy_nonoverlapping(p.value.as_ptr(), value.as_mut_ptr().cast::<u8>(), p.value.len())
                };
                Retrievable {
                    key: map.map(key),
                    type_: map.map(&p.type_uri),
                    flags: p.flags,
                    size: p.value.len(),
                    value,
                }
            })
            .collect();
        Self { properties }
    }
}

/// `LV2_State_Retrieve_Function`. Never unwinds into C
pub(crate) unsafe extern "C" fn retrieve(
    handle: *mut c_void,
    key: u32,
    size: *mut usize,
    type_: *mut u32,
    flags: *mut u32,
) -> *const c_void {
    catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return std::ptr::null();
        }
        // SAFETY: the RestoreContext of the restore in progress, on this thread, only read
        let ctx = unsafe { &*handle.cast::<RestoreContext>() };
        let Some(p) = ctx.properties.iter().find(|p| p.key == key) else {
            return std::ptr::null();
        };
        // SAFETY: the out pointers are either NULL (skipped) or valid for one write, per the LV2 state spec
        unsafe {
            if !size.is_null() {
                *size = p.size;
            }
            if !type_.is_null() {
                *type_ = p.type_;
            }
            if !flags.is_null() {
                *flags = p.flags;
            }
        }
        // Valid until the restore returns (the context outlives the call)
        p.value.as_ptr().cast::<c_void>()
    }))
    .unwrap_or(std::ptr::null())
}

/// state:mapPath, both ways (no `catch_unwind`: a null check and one libc call, which cannot panic): the identity (absolute paths are stored as they are). The plugin frees the copy with
/// `free` or state:freePath
pub(crate) unsafe extern "C" fn map_path(_handle: *mut c_void, path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: a NUL-terminated path from the plugin; strdup returns a malloc'ed copy (or NULL)
    unsafe { ffi::strdup(path) }
}

/// state:freePath
pub(crate) unsafe extern "C" fn free_path(_handle: *mut c_void, path: *mut c_char) {
    // SAFETY: a path this host handed out with strdup (malloc), freed once by the plugin's request; free(NULL) is fine
    unsafe { ffi::free(path.cast()) };
}

/// The keys of a chunk, for diagnostics
pub fn chunk_keys(chunk: &[u8]) -> Result<Vec<String>> {
    Ok(decode(chunk)?.into_keys().collect())
}
