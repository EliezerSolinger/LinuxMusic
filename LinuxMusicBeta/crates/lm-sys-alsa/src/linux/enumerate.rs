//! Device enumeration: cards, their PCM devices, what each can do, and ALSA's name hints.
//!
//! Slow (U17): opening control devices and probing PCMs can take a long time with USB devices. Every function here
//! blocks and allocates, so the caller runs it in an `Operation`, never on the main thread nor the audio thread.
//! A machine without cards gives an empty list, not an error.

use super::ffi;
use super::pcm::{HwParams, raw_format};
use super::{c_string, check, text};
use crate::error::{Phase, errno};
use crate::{Pcm, SampleFormat, Stream};
use lm_base::{Error, Result};
use std::ffi::{c_int, c_uint, c_void};
use std::ptr::{self, NonNull};

/// One sound card
#[derive(Clone, Debug, PartialEq)]
pub struct CardInfo {
    /// The card number (`hw:N`); may change between boots
    pub index: i32,
    /// The stable id (`hw:CARD=<id>`), what preferences store
    pub id: String,
    pub name: String,
    pub long_name: String,
    pub driver: String,
    pub devices: Vec<PcmDeviceInfo>,
}

/// One PCM device of a card
#[derive(Clone, Debug, PartialEq)]
pub struct PcmDeviceInfo {
    pub device: i32,
    pub id: String,
    pub name: String,
    /// The ALSA name to open it exclusively: `hw:CARD=<id>,DEV=<device>`
    pub hw_name: String,
    pub playback: Option<StreamInfo>,
    pub capture: Option<StreamInfo>,
}

/// One direction of a PCM device
#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub subdevices: u32,
    /// What the device can do, if it was probed
    pub probe: Probe,
}

/// The result of probing a PCM: opening it and reading its hardware parameter ranges
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    /// Not asked for
    NotProbed,
    Capabilities(Capabilities),
    /// In use by another program (U8: PipeWire or PulseAudio, usually; ask with ReserveDevice1)
    Busy,
    /// No permission (U8: the user is not in the `audio` group)
    Denied,
    Failed(Error),
}

/// The ranges a PCM accepts
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub channels_min: u32,
    pub channels_max: u32,
    pub rate_min: u32,
    pub rate_max: u32,
    /// The common rates (44.1 to 192 kHz) the device has
    pub rates: Vec<u32>,
    /// The sample formats of [`SampleFormat`] the device has
    pub formats: Vec<SampleFormat>,
    pub period_min: u64,
    pub period_max: u64,
    pub buffer_max: u64,
}

/// What a name hint says about direction (`IOID`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintDirection {
    Both,
    Output,
    Input,
}

/// One of ALSA's PCM name hints (`snd_device_name_hint`): the names the configuration offers (`default`,
/// `sysdefault:CARD=...`, `pipewire`, `null`...)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PcmHint {
    pub name: String,
    pub description: String,
    pub direction: HintDirection,
}

const COMMON_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

struct Ctl(NonNull<ffi::snd_ctl_t>);

impl Drop for Ctl {
    fn drop(&mut self) {
        // SAFETY: an open control handle, closed once
        unsafe { ffi::snd_ctl_close(self.0.as_ptr()) };
    }
}

struct CardInfoObj(NonNull<ffi::snd_ctl_card_info_t>);

impl Drop for CardInfoObj {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_ctl_card_info_malloc, freed once
        unsafe { ffi::snd_ctl_card_info_free(self.0.as_ptr()) };
    }
}

struct PcmInfoObj(NonNull<ffi::snd_pcm_info_t>);

impl Drop for PcmInfoObj {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_pcm_info_malloc, freed once
        unsafe { ffi::snd_pcm_info_free(self.0.as_ptr()) };
    }
}

/// The card numbers present
pub fn card_indices() -> Result<Vec<i32>> {
    let mut out = Vec::new();
    let mut card: c_int = -1;
    loop {
        // SAFETY: valid in/out pointer
        let r = unsafe { ffi::snd_card_next(&mut card) };
        if r < 0 {
            // No /dev/snd at all is "no cards", not an error
            if r == -errno::ENOENT {
                break;
            }
            return Err(crate::error::error_from_code(r, "snd_card_next", "", Phase::Open));
        }
        if card < 0 {
            break;
        }
        out.push(card);
    }
    Ok(out)
}

/// Every card with its PCM devices. With `probe`, each direction of each device is opened and its ranges read
/// (slower; a device in use shows as [`Probe::Busy`]). A card that cannot be read is skipped only if it vanished
/// meanwhile; other errors are returned
pub fn cards(probe: bool) -> Result<Vec<CardInfo>> {
    let mut out = Vec::new();
    for index in card_indices()? {
        match card(index, probe) {
            Ok(card) => out.push(card),
            Err(e) if e.key() == "alsa.device_not_found" => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// One card by number
pub fn card(index: i32, probe: bool) -> Result<CardInfo> {
    let ctl_name = format!("hw:{index}");
    let c_name = c_string(&ctl_name, "card")?;
    let mut raw = ptr::null_mut();
    // SAFETY: valid out pointer and name
    let r = unsafe { ffi::snd_ctl_open(&mut raw, c_name.as_ptr(), 0) };
    check(r, "snd_ctl_open", &ctl_name, Phase::Open)?;
    let ctl = Ctl(NonNull::new(raw).ok_or_else(|| Error::new("alsa.out_of_memory"))?);

    let mut info = ptr::null_mut();
    // SAFETY: valid out pointer
    check(unsafe { ffi::snd_ctl_card_info_malloc(&mut info) }, "snd_ctl_card_info_malloc", "", Phase::Open)?;
    let info = CardInfoObj(NonNull::new(info).ok_or_else(|| Error::new("alsa.out_of_memory"))?);
    // SAFETY: live handle and info object
    let r = unsafe { ffi::snd_ctl_card_info(ctl.0.as_ptr(), info.0.as_ptr()) };
    check(r, "snd_ctl_card_info", &ctl_name, Phase::Open)?;
    let i = info.0.as_ptr();
    // SAFETY: getters of a filled info object; the strings are copied before it is freed
    let (id, name, long_name, driver) = unsafe {
        (
            text(ffi::snd_ctl_card_info_get_id(i)),
            text(ffi::snd_ctl_card_info_get_name(i)),
            text(ffi::snd_ctl_card_info_get_longname(i)),
            text(ffi::snd_ctl_card_info_get_driver(i)),
        )
    };

    let mut pcm_info = ptr::null_mut();
    // SAFETY: valid out pointer
    check(unsafe { ffi::snd_pcm_info_malloc(&mut pcm_info) }, "snd_pcm_info_malloc", "", Phase::Open)?;
    let pcm_info = PcmInfoObj(NonNull::new(pcm_info).ok_or_else(|| Error::new("alsa.out_of_memory"))?);

    let mut devices = Vec::new();
    let mut device: c_int = -1;
    loop {
        // SAFETY: live handle, valid in/out pointer
        let r = unsafe { ffi::snd_ctl_pcm_next_device(ctl.0.as_ptr(), &mut device) };
        check(r, "snd_ctl_pcm_next_device", &ctl_name, Phase::Running)?;
        if device < 0 {
            break;
        }
        let hw_name = format!("hw:CARD={id},DEV={device}");
        let mut dev_id = String::new();
        let mut dev_name = String::new();
        let mut directions = [None, None];
        for (slot, stream) in [Stream::Playback, Stream::Capture].into_iter().enumerate() {
            let p = pcm_info.0.as_ptr();
            // SAFETY: live info object and handle
            let r = unsafe {
                ffi::snd_pcm_info_set_device(p, device as c_uint);
                ffi::snd_pcm_info_set_subdevice(p, 0);
                ffi::snd_pcm_info_set_stream(p, if stream == Stream::Playback { 0 } else { 1 });
                ffi::snd_ctl_pcm_info(ctl.0.as_ptr(), p)
            };
            if r < 0 {
                // ENOENT: the device has no such direction
                continue;
            }
            // SAFETY: getters of a filled info object
            let subdevices = unsafe {
                if dev_id.is_empty() {
                    dev_id = text(ffi::snd_pcm_info_get_id(p));
                    dev_name = text(ffi::snd_pcm_info_get_name(p));
                }
                ffi::snd_pcm_info_get_subdevices_count(p)
            };
            let probe = if probe { probe_result(&hw_name, stream) } else { Probe::NotProbed };
            directions[slot] = Some(StreamInfo { subdevices, probe });
        }
        let [playback, capture] = directions;
        devices.push(PcmDeviceInfo { device, id: dev_id, name: dev_name, hw_name, playback, capture });
    }
    Ok(CardInfo { index, id, name, long_name, driver, devices })
}

fn probe_result(name: &str, stream: Stream) -> Probe {
    match probe(name, stream) {
        Ok(c) => Probe::Capabilities(c),
        Err(e) if e.key() == "alsa.device_busy" => Probe::Busy,
        Err(e) if e.key() == "alsa.permission_denied" => Probe::Denied,
        Err(e) => Probe::Failed(e),
    }
}

/// Opens `name` (non-blocking) and reads the ranges of its hardware parameters, then closes it
pub fn probe(name: &str, stream: Stream) -> Result<Capabilities> {
    let pcm = Pcm::open(name, stream)?;
    let hw = HwParams::new()?;
    let (raw, p) = (pcm.raw(), hw.as_ptr());
    // SAFETY: open handle, live params object, valid out pointers
    unsafe {
        check(ffi::snd_pcm_hw_params_any(raw, p), "snd_pcm_hw_params_any", name, Phase::Open)?;
        let (mut cmin, mut cmax, mut rmin, mut rmax, mut dir) = (0, 0, 0, 0, 0);
        ffi::snd_pcm_hw_params_get_channels_min(p, &mut cmin);
        ffi::snd_pcm_hw_params_get_channels_max(p, &mut cmax);
        ffi::snd_pcm_hw_params_get_rate_min(p, &mut rmin, &mut dir);
        ffi::snd_pcm_hw_params_get_rate_max(p, &mut rmax, &mut dir);
        let (mut pmin, mut pmax, mut bmax) = (0, 0, 0);
        ffi::snd_pcm_hw_params_get_period_size_min(p, &mut pmin, &mut dir);
        ffi::snd_pcm_hw_params_get_period_size_max(p, &mut pmax, &mut dir);
        ffi::snd_pcm_hw_params_get_buffer_size_max(p, &mut bmax);
        let formats = SampleFormat::PREFERENCE
            .into_iter()
            .filter(|f| ffi::snd_pcm_hw_params_test_format(raw, p, raw_format(*f)) == 0)
            .collect();
        let rates = COMMON_RATES.into_iter().filter(|r| ffi::snd_pcm_hw_params_test_rate(raw, p, *r, 0) == 0).collect();
        Ok(Capabilities {
            channels_min: cmin,
            channels_max: cmax,
            rate_min: rmin,
            rate_max: rmax,
            rates,
            formats,
            period_min: pmin,
            period_max: pmax,
            buffer_max: bmax,
        })
    }
}

struct Hints(*mut *mut c_void);

impl Drop for Hints {
    fn drop(&mut self) {
        // SAFETY: the array snd_device_name_hint allocated, freed once
        unsafe { ffi::snd_device_name_free_hint(self.0) };
    }
}

fn hint_field(hint: *const c_void, id: &std::ffi::CStr) -> Option<String> {
    // SAFETY: a hint of the array and a NUL-terminated id; the result is malloc'd and ours to free
    let p = unsafe { ffi::snd_device_name_get_hint(hint, id.as_ptr()) };
    if p.is_null() {
        return None;
    }
    let s = text(p);
    // SAFETY: allocated with malloc by alsa-lib, freed once
    unsafe { libc::free(p.cast()) };
    Some(s)
}

/// ALSA's PCM name hints, from the configuration (no device is opened)
pub fn pcm_hints() -> Result<Vec<PcmHint>> {
    let mut raw = ptr::null_mut();
    // SAFETY: valid out pointer and interface name
    let r = unsafe { ffi::snd_device_name_hint(-1, c"pcm".as_ptr(), &mut raw) };
    check(r, "snd_device_name_hint", "", Phase::Open)?;
    let hints = Hints(raw);
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        // SAFETY: the array is NULL-terminated
        let hint = unsafe { *hints.0.add(i) };
        if hint.is_null() {
            break;
        }
        i += 1;
        let Some(name) = hint_field(hint, c"NAME") else { continue };
        let description = hint_field(hint, c"DESC").unwrap_or_default();
        let direction = match hint_field(hint, c"IOID").as_deref() {
            Some("Output") => HintDirection::Output,
            Some("Input") => HintDirection::Input,
            _ => HintDirection::Both,
        };
        out.push(PcmHint { name, description, direction });
    }
    Ok(out)
}
