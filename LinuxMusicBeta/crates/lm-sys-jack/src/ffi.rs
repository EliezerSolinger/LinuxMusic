//! The C API of libjack used here, declared by hand (jack/jack.h, types.h, midiport.h): about forty functions, stable
//! since JACK 0.121 in JACK1, JACK2 and PipeWire's libjack alike. The library is linked by the build script.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_void};

/// `jack_client_t`
#[repr(C)]
pub struct jack_client_t {
    _private: [u8; 0],
}

/// `jack_port_t`
#[repr(C)]
pub struct jack_port_t {
    _private: [u8; 0],
}

pub type jack_nframes_t = u32;
pub type jack_time_t = u64;
pub type jack_status_t = c_int;
pub type jack_options_t = c_int;
pub type jack_latency_callback_mode_t = c_int;

/// `jack_latency_range_t`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct jack_latency_range_t {
    pub min: jack_nframes_t,
    pub max: jack_nframes_t,
}

/// `jack_midi_event_t`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct jack_midi_event_t {
    pub time: jack_nframes_t,
    pub size: usize,
    pub buffer: *mut u8,
}

// jack_options_t
pub const JACK_NO_START_SERVER: jack_options_t = 0x01;
pub const JACK_SERVER_NAME: jack_options_t = 0x04;

// jack_status_t
pub const JACK_FAILURE: jack_status_t = 0x01;
pub const JACK_SERVER_FAILED: jack_status_t = 0x10;
pub const JACK_SERVER_ERROR: jack_status_t = 0x20;
pub const JACK_VERSION_ERROR: jack_status_t = 0x400;

// JackPortFlags
pub const JACK_PORT_IS_INPUT: u32 = 0x1;
pub const JACK_PORT_IS_OUTPUT: u32 = 0x2;
pub const JACK_PORT_IS_PHYSICAL: u32 = 0x4;

// jack_latency_callback_mode_t
pub const JACK_CAPTURE_LATENCY: jack_latency_callback_mode_t = 0;
pub const JACK_PLAYBACK_LATENCY: jack_latency_callback_mode_t = 1;

pub const JACK_DEFAULT_AUDIO_TYPE: &[u8] = b"32 bit float mono audio\0";
pub const JACK_DEFAULT_MIDI_TYPE: &[u8] = b"8 bit raw midi\0";

pub type JackProcessCallback = unsafe extern "C" fn(nframes: jack_nframes_t, arg: *mut c_void) -> c_int;
pub type JackXRunCallback = unsafe extern "C" fn(arg: *mut c_void) -> c_int;
pub type JackInfoShutdownCallback = unsafe extern "C" fn(code: jack_status_t, reason: *const c_char, arg: *mut c_void);
pub type JackBufferSizeCallback = unsafe extern "C" fn(nframes: jack_nframes_t, arg: *mut c_void) -> c_int;
pub type JackPortRegistrationCallback = unsafe extern "C" fn(port: u32, register: c_int, arg: *mut c_void);
pub type JackPortConnectCallback = unsafe extern "C" fn(a: u32, b: u32, connect: c_int, arg: *mut c_void);

unsafe extern "C" {
    pub fn jack_client_open(
        client_name: *const c_char,
        options: jack_options_t,
        status: *mut jack_status_t,
        ...
    ) -> *mut jack_client_t;
    pub fn jack_client_close(client: *mut jack_client_t) -> c_int;
    pub fn jack_get_client_name(client: *mut jack_client_t) -> *mut c_char;
    pub fn jack_activate(client: *mut jack_client_t) -> c_int;
    pub fn jack_deactivate(client: *mut jack_client_t) -> c_int;
    pub fn jack_get_sample_rate(client: *mut jack_client_t) -> jack_nframes_t;
    pub fn jack_get_buffer_size(client: *mut jack_client_t) -> jack_nframes_t;
    pub fn jack_cpu_load(client: *mut jack_client_t) -> f32;

    pub fn jack_set_process_callback(
        client: *mut jack_client_t,
        callback: Option<JackProcessCallback>,
        arg: *mut c_void,
    ) -> c_int;
    pub fn jack_set_xrun_callback(
        client: *mut jack_client_t,
        callback: Option<JackXRunCallback>,
        arg: *mut c_void,
    ) -> c_int;
    pub fn jack_on_info_shutdown(
        client: *mut jack_client_t,
        callback: Option<JackInfoShutdownCallback>,
        arg: *mut c_void,
    );
    pub fn jack_set_buffer_size_callback(
        client: *mut jack_client_t,
        callback: Option<JackBufferSizeCallback>,
        arg: *mut c_void,
    ) -> c_int;
    pub fn jack_set_port_registration_callback(
        client: *mut jack_client_t,
        callback: Option<JackPortRegistrationCallback>,
        arg: *mut c_void,
    ) -> c_int;
    pub fn jack_set_port_connect_callback(
        client: *mut jack_client_t,
        callback: Option<JackPortConnectCallback>,
        arg: *mut c_void,
    ) -> c_int;

    pub fn jack_port_register(
        client: *mut jack_client_t,
        port_name: *const c_char,
        port_type: *const c_char,
        flags: std::ffi::c_ulong,
        buffer_size: std::ffi::c_ulong,
    ) -> *mut jack_port_t;
    pub fn jack_port_get_buffer(port: *mut jack_port_t, nframes: jack_nframes_t) -> *mut c_void;
    pub fn jack_port_name(port: *const jack_port_t) -> *const c_char;
    pub fn jack_port_by_name(client: *mut jack_client_t, port_name: *const c_char) -> *mut jack_port_t;
    pub fn jack_port_by_id(client: *mut jack_client_t, port_id: u32) -> *mut jack_port_t;
    pub fn jack_get_ports(
        client: *mut jack_client_t,
        port_name_pattern: *const c_char,
        type_name_pattern: *const c_char,
        flags: std::ffi::c_ulong,
    ) -> *mut *const c_char;
    pub fn jack_free(ptr: *mut c_void);
    pub fn jack_connect(
        client: *mut jack_client_t,
        source_port: *const c_char,
        destination_port: *const c_char,
    ) -> c_int;
    pub fn jack_disconnect(
        client: *mut jack_client_t,
        source_port: *const c_char,
        destination_port: *const c_char,
    ) -> c_int;

    pub fn jack_port_get_latency_range(
        port: *mut jack_port_t,
        mode: jack_latency_callback_mode_t,
        range: *mut jack_latency_range_t,
    );

    pub fn jack_get_cycle_times(
        client: *const jack_client_t,
        current_frames: *mut jack_nframes_t,
        current_usecs: *mut jack_time_t,
        next_usecs: *mut jack_time_t,
        period_usecs: *mut f32,
    ) -> c_int;
    pub fn jack_get_time() -> jack_time_t;
    pub fn jack_get_xrun_delayed_usecs(client: *mut jack_client_t) -> f32;

    pub fn jack_midi_get_event_count(port_buffer: *mut c_void) -> u32;
    pub fn jack_midi_event_get(event: *mut jack_midi_event_t, port_buffer: *mut c_void, event_index: u32) -> c_int;
    pub fn jack_midi_clear_buffer(port_buffer: *mut c_void);
    pub fn jack_midi_event_write(
        port_buffer: *mut c_void,
        time: jack_nframes_t,
        data: *const u8,
        data_size: usize,
    ) -> c_int;

    pub fn jack_set_error_function(func: Option<unsafe extern "C" fn(msg: *const c_char)>);
    pub fn jack_set_info_function(func: Option<unsafe extern "C" fn(msg: *const c_char)>);
}
