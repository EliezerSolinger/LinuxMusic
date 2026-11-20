//! The part of libasound's C API this crate uses, declared by hand (see the safety sheet in `lib.rs` for why not
//! `alsa-sys`). Every declaration follows alsa-lib 1.2's headers (`alsa/pcm.h`, `alsa/control.h`, `alsa/seq.h`,
//! `alsa/seq_event.h`, `alsa/error.h`); the layouts of the two structs we read field by field
//! (`snd_pcm_channel_area_t`, `snd_seq_event_t`) are checked by size and alignment at compile time.

#![allow(non_camel_case_types, dead_code)]

use libc::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_ushort, c_void, pollfd, timespec};

macro_rules! opaque {
    ($($name:ident),* $(,)?) => {$(
        #[repr(C)]
        pub struct $name {
            _private: [u8; 0],
            _not_send_sync: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
        }
    )*};
}

opaque!(
    snd_pcm_t,
    snd_pcm_hw_params_t,
    snd_pcm_sw_params_t,
    snd_pcm_status_t,
    snd_pcm_info_t,
    snd_ctl_t,
    snd_ctl_card_info_t,
    snd_seq_t,
    snd_seq_client_info_t,
    snd_seq_port_info_t,
    snd_seq_port_subscribe_t,
    snd_seq_queue_status_t,
    snd_seq_queue_timer_t,
);

pub type snd_pcm_uframes_t = c_ulong;
pub type snd_pcm_sframes_t = c_long;
pub type snd_htimestamp_t = timespec;

// snd_pcm_stream_t
pub const SND_PCM_STREAM_PLAYBACK: c_int = 0;
pub const SND_PCM_STREAM_CAPTURE: c_int = 1;

// open modes
pub const SND_PCM_NONBLOCK: c_int = 0x0001;

// snd_pcm_access_t
pub const SND_PCM_ACCESS_MMAP_INTERLEAVED: c_int = 0;
pub const SND_PCM_ACCESS_MMAP_NONINTERLEAVED: c_int = 1;
pub const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;
pub const SND_PCM_ACCESS_RW_NONINTERLEAVED: c_int = 4;

// snd_pcm_format_t (the little-endian ones)
pub const SND_PCM_FORMAT_S16_LE: c_int = 2;
pub const SND_PCM_FORMAT_S24_LE: c_int = 6;
pub const SND_PCM_FORMAT_S32_LE: c_int = 10;
pub const SND_PCM_FORMAT_FLOAT_LE: c_int = 14;
pub const SND_PCM_FORMAT_S24_3LE: c_int = 32;

// snd_pcm_tstamp_t and snd_pcm_tstamp_type_t
pub const SND_PCM_TSTAMP_ENABLE: c_int = 1;
pub const SND_PCM_TSTAMP_TYPE_MONOTONIC: c_int = 1;

/// `snd_pcm_channel_area_t`: where one channel's samples are, in bits
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct snd_pcm_channel_area_t {
    pub addr: *mut c_void,
    pub first: c_uint,
    pub step: c_uint,
}

const _: () = assert!(size_of::<snd_pcm_channel_area_t>() == size_of::<usize>() + 8);

#[link(name = "asound")]
unsafe extern "C" {
    pub fn snd_strerror(errnum: c_int) -> *const c_char;

    // ---- PCM ----
    pub fn snd_pcm_open(pcm: *mut *mut snd_pcm_t, name: *const c_char, stream: c_int, mode: c_int) -> c_int;
    pub fn snd_pcm_close(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_nonblock(pcm: *mut snd_pcm_t, nonblock: c_int) -> c_int;
    pub fn snd_pcm_prepare(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_start(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_drop(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_drain(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_resume(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_recover(pcm: *mut snd_pcm_t, err: c_int, silent: c_int) -> c_int;
    pub fn snd_pcm_state(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_avail_update(pcm: *mut snd_pcm_t) -> snd_pcm_sframes_t;
    pub fn snd_pcm_avail(pcm: *mut snd_pcm_t) -> snd_pcm_sframes_t;
    pub fn snd_pcm_delay(pcm: *mut snd_pcm_t, delayp: *mut snd_pcm_sframes_t) -> c_int;
    pub fn snd_pcm_htimestamp(
        pcm: *mut snd_pcm_t,
        avail: *mut snd_pcm_uframes_t,
        tstamp: *mut snd_htimestamp_t,
    ) -> c_int;
    pub fn snd_pcm_link(pcm1: *mut snd_pcm_t, pcm2: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_unlink(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_writei(pcm: *mut snd_pcm_t, buffer: *const c_void, size: snd_pcm_uframes_t) -> snd_pcm_sframes_t;
    pub fn snd_pcm_readi(pcm: *mut snd_pcm_t, buffer: *mut c_void, size: snd_pcm_uframes_t) -> snd_pcm_sframes_t;
    pub fn snd_pcm_writen(pcm: *mut snd_pcm_t, bufs: *mut *mut c_void, size: snd_pcm_uframes_t) -> snd_pcm_sframes_t;
    pub fn snd_pcm_readn(pcm: *mut snd_pcm_t, bufs: *mut *mut c_void, size: snd_pcm_uframes_t) -> snd_pcm_sframes_t;
    pub fn snd_pcm_mmap_begin(
        pcm: *mut snd_pcm_t,
        areas: *mut *const snd_pcm_channel_area_t,
        offset: *mut snd_pcm_uframes_t,
        frames: *mut snd_pcm_uframes_t,
    ) -> c_int;
    pub fn snd_pcm_mmap_commit(
        pcm: *mut snd_pcm_t,
        offset: snd_pcm_uframes_t,
        frames: snd_pcm_uframes_t,
    ) -> snd_pcm_sframes_t;
    pub fn snd_pcm_poll_descriptors_count(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_poll_descriptors(pcm: *mut snd_pcm_t, pfds: *mut pollfd, space: c_uint) -> c_int;
    pub fn snd_pcm_poll_descriptors_revents(
        pcm: *mut snd_pcm_t,
        pfds: *mut pollfd,
        nfds: c_uint,
        revents: *mut c_ushort,
    ) -> c_int;

    // hw params
    pub fn snd_pcm_hw_params_malloc(ptr: *mut *mut snd_pcm_hw_params_t) -> c_int;
    pub fn snd_pcm_hw_params_free(obj: *mut snd_pcm_hw_params_t);
    pub fn snd_pcm_hw_params_any(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t) -> c_int;
    pub fn snd_pcm_hw_params(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t) -> c_int;
    pub fn snd_pcm_hw_params_set_rate_resample(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_hw_params_t,
        val: c_uint,
    ) -> c_int;
    pub fn snd_pcm_hw_params_test_access(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, access: c_int)
    -> c_int;
    pub fn snd_pcm_hw_params_set_access(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, access: c_int) -> c_int;
    pub fn snd_pcm_hw_params_get_access(params: *const snd_pcm_hw_params_t, access: *mut c_int) -> c_int;
    pub fn snd_pcm_hw_params_test_format(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, val: c_int) -> c_int;
    pub fn snd_pcm_hw_params_set_format(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, val: c_int) -> c_int;
    pub fn snd_pcm_hw_params_get_format(params: *const snd_pcm_hw_params_t, val: *mut c_int) -> c_int;
    pub fn snd_pcm_hw_params_set_channels(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, val: c_uint) -> c_int;
    pub fn snd_pcm_hw_params_get_channels(params: *const snd_pcm_hw_params_t, val: *mut c_uint) -> c_int;
    pub fn snd_pcm_hw_params_get_channels_min(params: *const snd_pcm_hw_params_t, val: *mut c_uint) -> c_int;
    pub fn snd_pcm_hw_params_get_channels_max(params: *const snd_pcm_hw_params_t, val: *mut c_uint) -> c_int;
    pub fn snd_pcm_hw_params_test_rate(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_hw_params_t,
        val: c_uint,
        dir: c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_set_rate(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_hw_params_t,
        val: c_uint,
        dir: c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_get_rate(params: *const snd_pcm_hw_params_t, val: *mut c_uint, dir: *mut c_int) -> c_int;
    pub fn snd_pcm_hw_params_get_rate_min(
        params: *const snd_pcm_hw_params_t,
        val: *mut c_uint,
        dir: *mut c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_get_rate_max(
        params: *const snd_pcm_hw_params_t,
        val: *mut c_uint,
        dir: *mut c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_set_period_size_near(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_hw_params_t,
        val: *mut snd_pcm_uframes_t,
        dir: *mut c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_get_period_size(
        params: *const snd_pcm_hw_params_t,
        frames: *mut snd_pcm_uframes_t,
        dir: *mut c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_get_period_size_min(
        params: *const snd_pcm_hw_params_t,
        frames: *mut snd_pcm_uframes_t,
        dir: *mut c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_get_period_size_max(
        params: *const snd_pcm_hw_params_t,
        frames: *mut snd_pcm_uframes_t,
        dir: *mut c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_set_periods_near(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_hw_params_t,
        val: *mut c_uint,
        dir: *mut c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_get_periods(
        params: *const snd_pcm_hw_params_t,
        val: *mut c_uint,
        dir: *mut c_int,
    ) -> c_int;
    pub fn snd_pcm_hw_params_set_buffer_size_near(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_hw_params_t,
        val: *mut snd_pcm_uframes_t,
    ) -> c_int;
    pub fn snd_pcm_hw_params_get_buffer_size(params: *const snd_pcm_hw_params_t, val: *mut snd_pcm_uframes_t) -> c_int;
    pub fn snd_pcm_hw_params_get_buffer_size_max(
        params: *const snd_pcm_hw_params_t,
        val: *mut snd_pcm_uframes_t,
    ) -> c_int;

    // sw params
    pub fn snd_pcm_sw_params_malloc(ptr: *mut *mut snd_pcm_sw_params_t) -> c_int;
    pub fn snd_pcm_sw_params_free(obj: *mut snd_pcm_sw_params_t);
    pub fn snd_pcm_sw_params_current(pcm: *mut snd_pcm_t, params: *mut snd_pcm_sw_params_t) -> c_int;
    pub fn snd_pcm_sw_params(pcm: *mut snd_pcm_t, params: *mut snd_pcm_sw_params_t) -> c_int;
    pub fn snd_pcm_sw_params_get_boundary(params: *const snd_pcm_sw_params_t, val: *mut snd_pcm_uframes_t) -> c_int;
    pub fn snd_pcm_sw_params_set_start_threshold(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_sw_params_t,
        val: snd_pcm_uframes_t,
    ) -> c_int;
    pub fn snd_pcm_sw_params_set_stop_threshold(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_sw_params_t,
        val: snd_pcm_uframes_t,
    ) -> c_int;
    pub fn snd_pcm_sw_params_set_avail_min(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_sw_params_t,
        val: snd_pcm_uframes_t,
    ) -> c_int;
    pub fn snd_pcm_sw_params_set_silence_threshold(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_sw_params_t,
        val: snd_pcm_uframes_t,
    ) -> c_int;
    pub fn snd_pcm_sw_params_set_silence_size(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_sw_params_t,
        val: snd_pcm_uframes_t,
    ) -> c_int;
    pub fn snd_pcm_sw_params_set_tstamp_mode(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_sw_params_t,
        val: c_int,
    ) -> c_int;
    pub fn snd_pcm_sw_params_set_tstamp_type(
        pcm: *mut snd_pcm_t,
        params: *mut snd_pcm_sw_params_t,
        val: c_int,
    ) -> c_int;

    // status
    pub fn snd_pcm_status_malloc(ptr: *mut *mut snd_pcm_status_t) -> c_int;
    pub fn snd_pcm_status_free(obj: *mut snd_pcm_status_t);
    pub fn snd_pcm_status(pcm: *mut snd_pcm_t, status: *mut snd_pcm_status_t) -> c_int;
    pub fn snd_pcm_status_get_state(obj: *const snd_pcm_status_t) -> c_int;
    pub fn snd_pcm_status_get_trigger_htstamp(obj: *const snd_pcm_status_t, ptr: *mut snd_htimestamp_t);
    pub fn snd_pcm_status_get_htstamp(obj: *const snd_pcm_status_t, ptr: *mut snd_htimestamp_t);
    pub fn snd_pcm_status_get_delay(obj: *const snd_pcm_status_t) -> snd_pcm_sframes_t;
    pub fn snd_pcm_status_get_avail(obj: *const snd_pcm_status_t) -> snd_pcm_uframes_t;
    pub fn snd_pcm_status_get_avail_max(obj: *const snd_pcm_status_t) -> snd_pcm_uframes_t;

    // info
    pub fn snd_pcm_info_malloc(ptr: *mut *mut snd_pcm_info_t) -> c_int;
    pub fn snd_pcm_info_free(obj: *mut snd_pcm_info_t);
    pub fn snd_pcm_info_set_device(obj: *mut snd_pcm_info_t, val: c_uint);
    pub fn snd_pcm_info_set_subdevice(obj: *mut snd_pcm_info_t, val: c_uint);
    pub fn snd_pcm_info_set_stream(obj: *mut snd_pcm_info_t, val: c_int);
    pub fn snd_pcm_info_get_id(obj: *const snd_pcm_info_t) -> *const c_char;
    pub fn snd_pcm_info_get_name(obj: *const snd_pcm_info_t) -> *const c_char;
    pub fn snd_pcm_info_get_subdevices_count(obj: *const snd_pcm_info_t) -> c_uint;

    // ---- control (cards) ----
    pub fn snd_card_next(card: *mut c_int) -> c_int;
    pub fn snd_ctl_open(ctl: *mut *mut snd_ctl_t, name: *const c_char, mode: c_int) -> c_int;
    pub fn snd_ctl_close(ctl: *mut snd_ctl_t) -> c_int;
    pub fn snd_ctl_card_info_malloc(ptr: *mut *mut snd_ctl_card_info_t) -> c_int;
    pub fn snd_ctl_card_info_free(obj: *mut snd_ctl_card_info_t);
    pub fn snd_ctl_card_info(ctl: *mut snd_ctl_t, info: *mut snd_ctl_card_info_t) -> c_int;
    pub fn snd_ctl_card_info_get_id(obj: *const snd_ctl_card_info_t) -> *const c_char;
    pub fn snd_ctl_card_info_get_driver(obj: *const snd_ctl_card_info_t) -> *const c_char;
    pub fn snd_ctl_card_info_get_name(obj: *const snd_ctl_card_info_t) -> *const c_char;
    pub fn snd_ctl_card_info_get_longname(obj: *const snd_ctl_card_info_t) -> *const c_char;
    pub fn snd_ctl_pcm_next_device(ctl: *mut snd_ctl_t, device: *mut c_int) -> c_int;
    pub fn snd_ctl_pcm_info(ctl: *mut snd_ctl_t, info: *mut snd_pcm_info_t) -> c_int;

    // ---- device name hints ----
    pub fn snd_device_name_hint(card: c_int, iface: *const c_char, hints: *mut *mut *mut c_void) -> c_int;
    pub fn snd_device_name_free_hint(hints: *mut *mut c_void) -> c_int;
    pub fn snd_device_name_get_hint(hint: *const c_void, id: *const c_char) -> *mut c_char;

    // ---- sequencer ----
    pub fn snd_seq_open(handle: *mut *mut snd_seq_t, name: *const c_char, streams: c_int, mode: c_int) -> c_int;
    pub fn snd_seq_close(handle: *mut snd_seq_t) -> c_int;
    pub fn snd_seq_client_id(handle: *mut snd_seq_t) -> c_int;
    pub fn snd_seq_set_client_name(handle: *mut snd_seq_t, name: *const c_char) -> c_int;
    pub fn snd_seq_poll_descriptors_count(handle: *mut snd_seq_t, events: c_short) -> c_int;
    pub fn snd_seq_poll_descriptors(handle: *mut snd_seq_t, pfds: *mut pollfd, space: c_uint, events: c_short)
    -> c_int;
    pub fn snd_seq_event_input(handle: *mut snd_seq_t, ev: *mut *mut snd_seq_event_t) -> c_int;
    pub fn snd_seq_event_output_direct(handle: *mut snd_seq_t, ev: *mut snd_seq_event_t) -> c_int;
    pub fn snd_seq_drain_output(handle: *mut snd_seq_t) -> c_int;
    pub fn snd_seq_alloc_named_queue(handle: *mut snd_seq_t, name: *const c_char) -> c_int;
    pub fn snd_seq_free_queue(handle: *mut snd_seq_t, q: c_int) -> c_int;
    pub fn snd_seq_control_queue(
        handle: *mut snd_seq_t,
        q: c_int,
        type_: c_int,
        value: c_int,
        ev: *mut snd_seq_event_t,
    ) -> c_int;
    pub fn snd_seq_queue_status_malloc(ptr: *mut *mut snd_seq_queue_status_t) -> c_int;
    pub fn snd_seq_queue_status_free(obj: *mut snd_seq_queue_status_t);
    pub fn snd_seq_get_queue_status(handle: *mut snd_seq_t, q: c_int, status: *mut snd_seq_queue_status_t) -> c_int;
    pub fn snd_seq_queue_status_get_real_time(info: *const snd_seq_queue_status_t) -> *const snd_seq_real_time_t;
    pub fn snd_seq_queue_timer_malloc(ptr: *mut *mut snd_seq_queue_timer_t) -> c_int;
    pub fn snd_seq_queue_timer_free(obj: *mut snd_seq_queue_timer_t);
    pub fn snd_seq_get_queue_timer(handle: *mut snd_seq_t, q: c_int, timer: *mut snd_seq_queue_timer_t) -> c_int;
    pub fn snd_seq_set_queue_timer(handle: *mut snd_seq_t, q: c_int, timer: *mut snd_seq_queue_timer_t) -> c_int;
    pub fn snd_seq_queue_timer_set_resolution(info: *mut snd_seq_queue_timer_t, resolution: c_uint);
    pub fn snd_seq_create_port(handle: *mut snd_seq_t, info: *mut snd_seq_port_info_t) -> c_int;
    pub fn snd_seq_delete_port(handle: *mut snd_seq_t, port: c_int) -> c_int;

    pub fn snd_seq_client_info_malloc(ptr: *mut *mut snd_seq_client_info_t) -> c_int;
    pub fn snd_seq_client_info_free(obj: *mut snd_seq_client_info_t);
    pub fn snd_seq_client_info_set_client(info: *mut snd_seq_client_info_t, client: c_int);
    pub fn snd_seq_client_info_get_client(info: *const snd_seq_client_info_t) -> c_int;
    pub fn snd_seq_client_info_get_name(info: *mut snd_seq_client_info_t) -> *const c_char;
    pub fn snd_seq_client_info_get_type(info: *const snd_seq_client_info_t) -> c_int;
    pub fn snd_seq_query_next_client(handle: *mut snd_seq_t, info: *mut snd_seq_client_info_t) -> c_int;

    pub fn snd_seq_port_info_malloc(ptr: *mut *mut snd_seq_port_info_t) -> c_int;
    pub fn snd_seq_port_info_free(obj: *mut snd_seq_port_info_t);
    pub fn snd_seq_port_info_set_client(info: *mut snd_seq_port_info_t, client: c_int);
    pub fn snd_seq_port_info_set_port(info: *mut snd_seq_port_info_t, port: c_int);
    pub fn snd_seq_port_info_set_name(info: *mut snd_seq_port_info_t, name: *const c_char);
    pub fn snd_seq_port_info_set_capability(info: *mut snd_seq_port_info_t, capability: c_uint);
    pub fn snd_seq_port_info_set_type(info: *mut snd_seq_port_info_t, type_: c_uint);
    pub fn snd_seq_port_info_set_midi_channels(info: *mut snd_seq_port_info_t, channels: c_int);
    pub fn snd_seq_port_info_set_timestamping(info: *mut snd_seq_port_info_t, enable: c_int);
    pub fn snd_seq_port_info_set_timestamp_real(info: *mut snd_seq_port_info_t, realtime: c_int);
    pub fn snd_seq_port_info_set_timestamp_queue(info: *mut snd_seq_port_info_t, queue: c_int);
    pub fn snd_seq_port_info_get_client(info: *const snd_seq_port_info_t) -> c_int;
    pub fn snd_seq_port_info_get_port(info: *const snd_seq_port_info_t) -> c_int;
    pub fn snd_seq_port_info_get_name(info: *const snd_seq_port_info_t) -> *const c_char;
    pub fn snd_seq_port_info_get_capability(info: *const snd_seq_port_info_t) -> c_uint;
    pub fn snd_seq_port_info_get_type(info: *const snd_seq_port_info_t) -> c_uint;
    pub fn snd_seq_query_next_port(handle: *mut snd_seq_t, info: *mut snd_seq_port_info_t) -> c_int;

    pub fn snd_seq_port_subscribe_malloc(ptr: *mut *mut snd_seq_port_subscribe_t) -> c_int;
    pub fn snd_seq_port_subscribe_free(obj: *mut snd_seq_port_subscribe_t);
    pub fn snd_seq_port_subscribe_set_sender(info: *mut snd_seq_port_subscribe_t, addr: *const snd_seq_addr_t);
    pub fn snd_seq_port_subscribe_set_dest(info: *mut snd_seq_port_subscribe_t, addr: *const snd_seq_addr_t);
    pub fn snd_seq_subscribe_port(handle: *mut snd_seq_t, sub: *mut snd_seq_port_subscribe_t) -> c_int;
    pub fn snd_seq_unsubscribe_port(handle: *mut snd_seq_t, sub: *mut snd_seq_port_subscribe_t) -> c_int;
}

// snd_seq_open streams and mode
pub const SND_SEQ_OPEN_DUPLEX: c_int = 3;
pub const SND_SEQ_NONBLOCK: c_int = 0x0001;

// special clients, ports and queues
pub const SND_SEQ_CLIENT_SYSTEM: u8 = 0;
pub const SND_SEQ_PORT_SYSTEM_ANNOUNCE: u8 = 1;
pub const SND_SEQ_ADDRESS_SUBSCRIBERS: u8 = 254;
pub const SND_SEQ_ADDRESS_UNKNOWN: u8 = 253;
pub const SND_SEQ_QUEUE_DIRECT: u8 = 253;

// port capabilities
pub const SND_SEQ_PORT_CAP_READ: c_uint = 1 << 0;
pub const SND_SEQ_PORT_CAP_WRITE: c_uint = 1 << 1;
pub const SND_SEQ_PORT_CAP_SUBS_READ: c_uint = 1 << 5;
pub const SND_SEQ_PORT_CAP_SUBS_WRITE: c_uint = 1 << 6;
pub const SND_SEQ_PORT_CAP_NO_EXPORT: c_uint = 1 << 7;

// port types
pub const SND_SEQ_PORT_TYPE_MIDI_GENERIC: c_uint = 1 << 1;
pub const SND_SEQ_PORT_TYPE_HARDWARE: c_uint = 1 << 16;
pub const SND_SEQ_PORT_TYPE_SOFTWARE: c_uint = 1 << 17;
pub const SND_SEQ_PORT_TYPE_PORT: c_uint = 1 << 19;
pub const SND_SEQ_PORT_TYPE_APPLICATION: c_uint = 1 << 20;

// client types
pub const SND_SEQ_KERNEL_CLIENT: c_int = 2;

// event flags
pub const SND_SEQ_TIME_STAMP_MASK: u8 = 1 << 0;
pub const SND_SEQ_TIME_STAMP_REAL: u8 = 1 << 0;
pub const SND_SEQ_TIME_MODE_MASK: u8 = 1 << 1;
pub const SND_SEQ_TIME_MODE_ABS: u8 = 0;
pub const SND_SEQ_EVENT_LENGTH_MASK: u8 = 3 << 2;
pub const SND_SEQ_EVENT_LENGTH_FIXED: u8 = 0;
pub const SND_SEQ_EVENT_LENGTH_VARIABLE: u8 = 1 << 2;

// event types used outside the short-message translation
pub const SND_SEQ_EVENT_SYSEX: u8 = 130;

// queue control event
pub const SND_SEQ_EVENT_START: c_int = 30;

/// `snd_seq_addr_t`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct snd_seq_addr_t {
    pub client: u8,
    pub port: u8,
}

/// `snd_seq_real_time_t`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct snd_seq_real_time_t {
    pub tv_sec: c_uint,
    pub tv_nsec: c_uint,
}

/// `snd_seq_event_t`: 28 bytes, alignment 4. The time is the union `snd_seq_timestamp_t` (a tick or a real time,
/// 8 bytes); the data is the 12-byte union, read and written through its byte image (`data.note`: channel, note,
/// velocity, off_velocity, duration; `data.control`: channel, 3 unused, param, value)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct snd_seq_event_t {
    pub type_: u8,
    pub flags: u8,
    pub tag: u8,
    pub queue: u8,
    pub time: snd_seq_real_time_t,
    pub source: snd_seq_addr_t,
    pub dest: snd_seq_addr_t,
    pub data: [u32; 3],
}

const _: () = assert!(size_of::<snd_seq_event_t>() == 28);
const _: () = assert!(align_of::<snd_seq_event_t>() == 4);
