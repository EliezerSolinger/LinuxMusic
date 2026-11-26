//! The LADSPA 1.1 ABI (ladspa.h), by hand. Nothing here is public outside the crate.
#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_ulong, c_void};

/// `LADSPA_PortRangeHint`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PortRangeHint {
    pub hint_descriptor: c_int,
    pub lower_bound: f32,
    pub upper_bound: f32,
}

/// `LADSPA_Descriptor`
#[repr(C)]
pub struct Descriptor {
    pub unique_id: c_ulong,
    pub label: *const c_char,
    pub properties: c_int,
    pub name: *const c_char,
    pub maker: *const c_char,
    pub copyright: *const c_char,
    pub port_count: c_ulong,
    pub port_descriptors: *const c_int,
    pub port_names: *const *const c_char,
    pub port_range_hints: *const PortRangeHint,
    pub implementation_data: *mut c_void,
    pub instantiate: Option<unsafe extern "C" fn(*const Descriptor, c_ulong) -> *mut c_void>,
    pub connect_port: Option<unsafe extern "C" fn(*mut c_void, c_ulong, *mut f32)>,
    pub activate: Option<unsafe extern "C" fn(*mut c_void)>,
    pub run: Option<unsafe extern "C" fn(*mut c_void, c_ulong)>,
    pub run_adding: Option<unsafe extern "C" fn(*mut c_void, c_ulong)>,
    pub set_run_adding_gain: Option<unsafe extern "C" fn(*mut c_void, f32)>,
    pub deactivate: Option<unsafe extern "C" fn(*mut c_void)>,
    pub cleanup: Option<unsafe extern "C" fn(*mut c_void)>,
}

/// `ladspa_descriptor`
pub type DescriptorFn = unsafe extern "C" fn(c_ulong) -> *const Descriptor;

/// `LADSPA_PORT_*`
pub mod port {
    pub const INPUT: i32 = 1;
    pub const OUTPUT: i32 = 2;
    pub const CONTROL: i32 = 4;
    pub const AUDIO: i32 = 8;
}

/// `LADSPA_HINT_*`
pub mod hint {
    pub const BOUNDED_BELOW: i32 = 0x1;
    pub const BOUNDED_ABOVE: i32 = 0x2;
    pub const TOGGLED: i32 = 0x4;
    pub const SAMPLE_RATE: i32 = 0x8;
    pub const LOGARITHMIC: i32 = 0x10;
    pub const INTEGER: i32 = 0x20;
    pub const DEFAULT_MASK: i32 = 0x3C0;
    pub const DEFAULT_MINIMUM: i32 = 0x40;
    pub const DEFAULT_LOW: i32 = 0x80;
    pub const DEFAULT_MIDDLE: i32 = 0xC0;
    pub const DEFAULT_HIGH: i32 = 0x100;
    pub const DEFAULT_MAXIMUM: i32 = 0x140;
    pub const DEFAULT_0: i32 = 0x200;
    pub const DEFAULT_1: i32 = 0x240;
    pub const DEFAULT_100: i32 = 0x280;
    pub const DEFAULT_440: i32 = 0x2C0;
}

/// `LADSPA_PROPERTY_*`
pub mod prop {
    pub const REALTIME: i32 = 1;
    pub const INPLACE_BROKEN: i32 = 2;
    pub const HARD_RT_CAPABLE: i32 = 4;
}
