//! The URIs the host knows, and the URID map the plugins get (urid:map and urid:unmap)

use crate::ffi;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, PoisonError};

/// The URIs of the LV2 specification this crate uses, as C strings
pub mod uri {
    use std::ffi::CStr;

    pub const LV2_PLUGIN: &CStr = c"http://lv2plug.in/ns/lv2core#Plugin";
    pub const AUDIO_PORT: &CStr = c"http://lv2plug.in/ns/lv2core#AudioPort";
    pub const CONTROL_PORT: &CStr = c"http://lv2plug.in/ns/lv2core#ControlPort";
    pub const CV_PORT: &CStr = c"http://lv2plug.in/ns/lv2core#CVPort";
    pub const INPUT_PORT: &CStr = c"http://lv2plug.in/ns/lv2core#InputPort";
    pub const OUTPUT_PORT: &CStr = c"http://lv2plug.in/ns/lv2core#OutputPort";
    pub const CONNECTION_OPTIONAL: &CStr = c"http://lv2plug.in/ns/lv2core#connectionOptional";
    pub const TOGGLED: &CStr = c"http://lv2plug.in/ns/lv2core#toggled";
    pub const INTEGER: &CStr = c"http://lv2plug.in/ns/lv2core#integer";
    pub const ENUMERATION: &CStr = c"http://lv2plug.in/ns/lv2core#enumeration";
    pub const SAMPLE_RATE: &CStr = c"http://lv2plug.in/ns/lv2core#sampleRate";
    pub const REPORTS_LATENCY: &CStr = c"http://lv2plug.in/ns/lv2core#reportsLatency";
    pub const LOGARITHMIC: &CStr = c"http://lv2plug.in/ns/ext/port-props#logarithmic";
    pub const HARD_RT_CAPABLE: &CStr = c"http://lv2plug.in/ns/lv2core#hardRTCapable";
    pub const IN_PLACE_BROKEN: &CStr = c"http://lv2plug.in/ns/lv2core#inPlaceBroken";
    pub const IS_LIVE: &CStr = c"http://lv2plug.in/ns/lv2core#isLive";

    pub const ATOM_PORT: &CStr = c"http://lv2plug.in/ns/ext/atom#AtomPort";
    pub const ATOM_CHUNK: &CStr = c"http://lv2plug.in/ns/ext/atom#Chunk";
    pub const ATOM_FLOAT: &CStr = c"http://lv2plug.in/ns/ext/atom#Float";
    pub const ATOM_INT: &CStr = c"http://lv2plug.in/ns/ext/atom#Int";
    pub const ATOM_SEQUENCE: &CStr = c"http://lv2plug.in/ns/ext/atom#Sequence";
    pub const MIDI_EVENT: &CStr = c"http://lv2plug.in/ns/ext/midi#MidiEvent";
    pub const MINIMUM_SIZE: &CStr = c"http://lv2plug.in/ns/ext/resize-port#minimumSize";

    pub const URID_MAP: &CStr = c"http://lv2plug.in/ns/ext/urid#map";
    pub const URID_UNMAP: &CStr = c"http://lv2plug.in/ns/ext/urid#unmap";
    pub const OPTIONS: &CStr = c"http://lv2plug.in/ns/ext/options#options";
    pub const REQUIRED_OPTION: &CStr = c"http://lv2plug.in/ns/ext/options#requiredOption";
    pub const PARAM_SAMPLE_RATE: &CStr = c"http://lv2plug.in/ns/ext/parameters#sampleRate";
    pub const BOUNDED_BLOCK_LENGTH: &CStr = c"http://lv2plug.in/ns/ext/buf-size#boundedBlockLength";
    pub const FIXED_BLOCK_LENGTH: &CStr = c"http://lv2plug.in/ns/ext/buf-size#fixedBlockLength";
    pub const POWER_OF_2_BLOCK_LENGTH: &CStr = c"http://lv2plug.in/ns/ext/buf-size#powerOf2BlockLength";
    pub const MIN_BLOCK_LENGTH: &CStr = c"http://lv2plug.in/ns/ext/buf-size#minBlockLength";
    pub const MAX_BLOCK_LENGTH: &CStr = c"http://lv2plug.in/ns/ext/buf-size#maxBlockLength";
    pub const NOMINAL_BLOCK_LENGTH: &CStr = c"http://lv2plug.in/ns/ext/buf-size#nominalBlockLength";
    pub const SEQUENCE_SIZE: &CStr = c"http://lv2plug.in/ns/ext/buf-size#sequenceSize";
    pub const WORKER_SCHEDULE: &CStr = c"http://lv2plug.in/ns/ext/worker#schedule";
    pub const WORKER_INTERFACE: &CStr = c"http://lv2plug.in/ns/ext/worker#interface";
    pub const STATE_INTERFACE: &CStr = c"http://lv2plug.in/ns/ext/state#interface";
    pub const STATE_MAP_PATH: &CStr = c"http://lv2plug.in/ns/ext/state#mapPath";
    pub const STATE_FREE_PATH: &CStr = c"http://lv2plug.in/ns/ext/state#freePath";

    pub const LILV_OPTION_LV2_PATH: &CStr = c"http://drobilla.net/ns/lilv#lv2-path";
    pub const LILV_OPTION_DYN_MANIFEST: &CStr = c"http://drobilla.net/ns/lilv#dyn-manifest";
}

/// The ids of the URIs the host itself uses, mapped once when the world is made
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Urids {
    pub atom_chunk: u32,
    pub atom_float: u32,
    pub atom_int: u32,
    pub atom_sequence: u32,
    pub midi_event: u32,
    pub param_sample_rate: u32,
    pub min_block_length: u32,
    pub max_block_length: u32,
    pub nominal_block_length: u32,
    pub sequence_size: u32,
}

#[derive(Default)]
struct Table {
    ids: HashMap<Box<[u8]>, u32>,
    /// Index `id - 1`. A `Box<CStr>` never moves, so the pointer `unmap` hands out stays valid for the map's life
    uris: Vec<Box<CStr>>,
}

/// The URID map of one world, shared by all its instances (so the same URI has the same id in every instance of the
/// process). Behind a mutex: plugins may call it from any thread (the spec says it is not real-time safe; plugins map
/// what they need at instantiation)
pub struct UridMap {
    table: Mutex<Table>,
}

impl UridMap {
    pub(crate) fn new() -> Self {
        Self { table: Mutex::new(Table::default()) }
    }

    fn map_bytes(&self, uri: &[u8]) -> u32 {
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(&id) = table.ids.get(uri) {
            return id;
        }
        let Ok(owned) = CString::new(uri) else {
            return 0;
        };
        let Ok(id) = u32::try_from(table.uris.len() + 1) else {
            return 0;
        };
        table.uris.push(owned.into_boxed_c_str());
        table.ids.insert(uri.into(), id);
        id
    }

    /// The id of `uri`, made if it is new (never 0 for a URI without NUL bytes)
    pub fn map(&self, uri: &str) -> u32 {
        self.map_bytes(uri.as_bytes())
    }

    /// The URI of `id`, if it was mapped
    pub fn unmap(&self, id: u32) -> Option<String> {
        let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        table.uris.get(index).map(|uri| uri.to_string_lossy().into_owned())
    }

    fn unmap_ptr(&self, id: u32) -> *const c_char {
        let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(index) = usize::try_from(id).ok().and_then(|i| i.checked_sub(1)) else {
            return std::ptr::null();
        };
        table.uris.get(index).map_or(std::ptr::null(), |uri| uri.as_ptr())
    }

    pub(crate) fn urids(&self) -> Urids {
        let m = |uri: &CStr| self.map_bytes(uri.to_bytes());
        Urids {
            atom_chunk: m(uri::ATOM_CHUNK),
            atom_float: m(uri::ATOM_FLOAT),
            atom_int: m(uri::ATOM_INT),
            atom_sequence: m(uri::ATOM_SEQUENCE),
            midi_event: m(uri::MIDI_EVENT),
            param_sample_rate: m(uri::PARAM_SAMPLE_RATE),
            min_block_length: m(uri::MIN_BLOCK_LENGTH),
            max_block_length: m(uri::MAX_BLOCK_LENGTH),
            nominal_block_length: m(uri::NOMINAL_BLOCK_LENGTH),
            sequence_size: m(uri::SEQUENCE_SIZE),
        }
    }

    /// The `LV2_URID_Map` and `LV2_URID_Unmap` for this map. The map must outlive every plugin given them (the
    /// instance holds an `Arc` of it)
    pub(crate) fn features(&self) -> (ffi::LV2_URID_Map, ffi::LV2_URID_Unmap) {
        let handle = std::ptr::from_ref(self).cast_mut().cast::<c_void>();
        (ffi::LV2_URID_Map { handle, map: map_trampoline }, ffi::LV2_URID_Unmap { handle, unmap: unmap_trampoline })
    }
}

impl fmt::Debug for UridMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("UridMap").field("uris", &table.uris.len()).finish()
    }
}

/// `LV2_URID_Map::map`, called by plugins from any thread. Never unwinds into C
unsafe extern "C" fn map_trampoline(handle: *mut c_void, uri: *const c_char) -> u32 {
    if handle.is_null() || uri.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `handle` is the `UridMap` this feature was made from (`features`), kept alive by an `Arc` in the
        // instance for as long as the plugin exists; it is only read through `&` (the table has its own lock)
        let map = unsafe { &*handle.cast::<UridMap>() };
        // SAFETY: the plugin passes a NUL-terminated string (LV2 urid spec), valid for the call
        let uri = unsafe { CStr::from_ptr(uri) };
        map.map_bytes(uri.to_bytes())
    }))
    .unwrap_or(0)
}

/// `LV2_URID_Unmap::unmap`. Never unwinds into C
unsafe extern "C" fn unmap_trampoline(handle: *mut c_void, id: u32) -> *const c_char {
    if handle.is_null() {
        return std::ptr::null();
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in map_trampoline
        let map = unsafe { &*handle.cast::<UridMap>() };
        map.unmap_ptr(id)
    }))
    .unwrap_or(std::ptr::null())
}
