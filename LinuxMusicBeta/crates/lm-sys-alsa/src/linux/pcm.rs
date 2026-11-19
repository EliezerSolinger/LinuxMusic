//! A PCM: one direction of one device, opened, configured and moved to the audio thread.
//!
//! Life of a duplex device (U7: one `hw` device, capture and playback linked so they share one clock):
//!
//! ```text
//! worker thread                         audio thread (lm-io's backend)
//! Pcm::open(hw, Playback), (hw, Capture)
//! configure(&request) -> HwConfig       (negotiated period: the engine's max block, C7)
//! capture.link(&mut playback)
//! PcmPoll::new(&[&playback, &capture])
//!          ── move the Pcms and the PcmPoll to the audio thread ──►
//!                                       playback.write_silence(buffer); playback.start()   (starts both)
//!                                       loop { poll.wait; capture.read_*; render; playback.write_* }
//!                                       on Err(Xrun): xrun_info, recover, prefill, start, lost_capture_frames
//! ```
//!
//! The PCM is opened non-blocking: reads and writes move what they can and return the count (0 when the device has
//! no room or no data); the wait is [`PcmPoll::wait`](crate::PcmPoll::wait).

use super::ffi;
use super::{c_string, check, host_time, keyed, text, zero_timespec};
use crate::error::{PcmIoError, Phase, errno};
use crate::{Access, HwConfig, HwRequest, PcmState, PcmStatus, SampleFormat, Stream, XrunInfo};
use lm_base::{Error, HostTime, Result};
use std::ffi::{c_int, c_uint, c_void};
use std::fmt;
use std::ptr::{self, NonNull};

pub(crate) fn raw_format(f: SampleFormat) -> c_int {
    match f {
        SampleFormat::F32 => ffi::SND_PCM_FORMAT_FLOAT_LE,
        SampleFormat::S32 => ffi::SND_PCM_FORMAT_S32_LE,
        SampleFormat::S24 => ffi::SND_PCM_FORMAT_S24_LE,
        SampleFormat::S24Packed => ffi::SND_PCM_FORMAT_S24_3LE,
        SampleFormat::S16 => ffi::SND_PCM_FORMAT_S16_LE,
    }
}

fn format_from_raw(v: c_int) -> Option<SampleFormat> {
    SampleFormat::PREFERENCE.into_iter().find(|f| raw_format(*f) == v)
}

fn raw_access(a: Access) -> c_int {
    match a {
        Access::MmapInterleaved => ffi::SND_PCM_ACCESS_MMAP_INTERLEAVED,
        Access::MmapNonInterleaved => ffi::SND_PCM_ACCESS_MMAP_NONINTERLEAVED,
        Access::ReadWriteInterleaved => ffi::SND_PCM_ACCESS_RW_INTERLEAVED,
        Access::ReadWriteNonInterleaved => ffi::SND_PCM_ACCESS_RW_NONINTERLEAVED,
    }
}

fn raw_stream(s: Stream) -> c_int {
    match s {
        Stream::Playback => ffi::SND_PCM_STREAM_PLAYBACK,
        Stream::Capture => ffi::SND_PCM_STREAM_CAPTURE,
    }
}

/// An `snd_pcm_hw_params_t`, freed on drop
pub(crate) struct HwParams(NonNull<ffi::snd_pcm_hw_params_t>);

impl HwParams {
    pub(crate) fn new() -> Result<Self> {
        let mut p = ptr::null_mut();
        // SAFETY: the out pointer is valid; on success it holds a fresh allocation this guard frees
        let r = unsafe { ffi::snd_pcm_hw_params_malloc(&mut p) };
        check(r, "snd_pcm_hw_params_malloc", "", Phase::Open)?;
        NonNull::new(p).map(Self).ok_or_else(|| Error::new("alsa.out_of_memory"))
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::snd_pcm_hw_params_t {
        self.0.as_ptr()
    }
}

impl Drop for HwParams {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_pcm_hw_params_malloc, freed once
        unsafe { ffi::snd_pcm_hw_params_free(self.0.as_ptr()) }
    }
}

struct SwParams(NonNull<ffi::snd_pcm_sw_params_t>);

impl SwParams {
    fn new() -> Result<Self> {
        let mut p = ptr::null_mut();
        // SAFETY: as HwParams::new
        let r = unsafe { ffi::snd_pcm_sw_params_malloc(&mut p) };
        check(r, "snd_pcm_sw_params_malloc", "", Phase::Open)?;
        NonNull::new(p).map(Self).ok_or_else(|| Error::new("alsa.out_of_memory"))
    }
}

impl Drop for SwParams {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_pcm_sw_params_malloc, freed once
        unsafe { ffi::snd_pcm_sw_params_free(self.0.as_ptr()) }
    }
}

/// Where one channel's samples are in a transfer: the first sample and the distance between samples, in bytes
#[derive(Clone, Copy)]
struct Lane {
    ptr: *mut u8,
    stride: usize,
}

/// The samples a playback transfer takes
#[derive(Clone, Copy)]
enum Src<'a> {
    Interleaved(&'a [f32]),
    Planar(&'a [&'a [f32]]),
    Silence,
}

impl Src<'_> {
    #[inline]
    fn sample(&self, channels: usize, ch: usize, frame: usize) -> f32 {
        match self {
            Src::Interleaved(s) => s[frame * channels + ch],
            Src::Planar(p) => p[ch][frame],
            Src::Silence => 0.0,
        }
    }
}

/// Where a capture transfer puts the samples
enum Dst<'a, 'b> {
    Interleaved(&'a mut [f32]),
    Planar(&'a mut [&'b mut [f32]]),
}

impl Dst<'_, '_> {
    #[inline]
    fn put(&mut self, channels: usize, ch: usize, frame: usize, x: f32) {
        match self {
            Dst::Interleaved(d) => d[frame * channels + ch] = x,
            Dst::Planar(p) => p[ch][frame] = x,
        }
    }
}

/// What `configure` made: the negotiated configuration and the buffers of the read/write path, allocated once here so
/// the transfers never allocate
struct Setup {
    config: HwConfig,
    /// read/write access: the device-format copy of up to one buffer
    scratch: Vec<u8>,
    /// read/write non-interleaved access: one pointer per channel into `scratch`
    planes: Vec<*mut c_void>,
}

/// One open PCM. Owns the `snd_pcm_t` (closed on drop) and its status object.
///
/// `Send` (it moves from the thread that opens it to the audio thread) and not `Sync` (libasound's PCM calls must not
/// run at the same time on one handle).
pub struct Pcm {
    raw: NonNull<ffi::snd_pcm_t>,
    status: NonNull<ffi::snd_pcm_status_t>,
    name: String,
    stream: Stream,
    setup: Option<Setup>,
    linked: bool,
}

// SAFETY: a snd_pcm_t may be used from any thread, one at a time (alsa-lib's thread-safety rules: handles are not
// shared, not tied to a thread). Pcm is not Sync and every call takes &mut self or happens from the owning thread,
// so no two threads call into the same handle at once. The raw pointers in `planes` point into `scratch`, which the
// Pcm owns.
unsafe impl Send for Pcm {}

impl fmt::Debug for Pcm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pcm")
            .field("name", &self.name)
            .field("stream", &self.stream)
            .field("config", &self.setup.as_ref().map(|s| s.config))
            .field("linked", &self.linked)
            .finish()
    }
}

impl Pcm {
    /// Opens a PCM by its ALSA name (`hw:CARD=PCH,DEV=0`, `hw:0,0`, `plughw:0`, `default`, `null`...). Non-blocking:
    /// a device another program has fails at once with `alsa.device_busy`, never waits.
    ///
    /// Not real-time safe (allocates, reads the ALSA configuration files on the first call of the process).
    pub fn open(name: &str, stream: Stream) -> Result<Pcm> {
        let c_name = c_string(name, "device")?;
        let mut status = ptr::null_mut();
        // SAFETY: valid out pointer; freed in Drop or below
        let r = unsafe { ffi::snd_pcm_status_malloc(&mut status) };
        check(r, "snd_pcm_status_malloc", name, Phase::Open)?;
        let status = NonNull::new(status).ok_or_else(|| Error::new("alsa.out_of_memory"))?;
        let mut raw = ptr::null_mut();
        // SAFETY: valid out pointer and NUL-terminated name; on success `raw` is a handle we own and close in Drop
        let r = unsafe { ffi::snd_pcm_open(&mut raw, c_name.as_ptr(), raw_stream(stream), ffi::SND_PCM_NONBLOCK) };
        let raw = match (r, NonNull::new(raw)) {
            (0.., Some(raw)) => raw,
            _ => {
                // SAFETY: allocated above, not used anywhere else
                unsafe { ffi::snd_pcm_status_free(status.as_ptr()) };
                let code = if r < 0 { r } else { -errno::EIO };
                return Err(
                    crate::error::error_from_code(code, "snd_pcm_open", name, Phase::Open).arg("stream", stream.name())
                );
            }
        };
        Ok(Pcm { raw, status, name: name.to_owned(), stream, setup: None, linked: false })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn stream(&self) -> Stream {
        self.stream
    }

    /// The negotiated configuration, once [`configure`](Pcm::configure) succeeded
    pub fn config(&self) -> Option<HwConfig> {
        self.setup.as_ref().map(|s| s.config)
    }

    pub(crate) fn raw(&self) -> *mut ffi::snd_pcm_t {
        self.raw.as_ptr()
    }

    /// Configures the device (hardware and software parameters) and leaves it prepared. Returns what was negotiated:
    /// the period may differ from the one asked (C7). Rate and channels are exact; a device that cannot do them fails
    /// with `alsa.rate_unsupported` / `alsa.channels_unsupported` and the range it has.
    ///
    /// Software parameters: the stream starts only by [`start`](Pcm::start) (never by itself), stops on an xrun
    /// (the default threshold: a full buffer), wakes the poll each period, and stamps its status with
    /// `CLOCK_MONOTONIC` (C6).
    ///
    /// Not real-time safe (allocates the transfer buffers, talks to the driver).
    pub fn configure(&mut self, req: &HwRequest) -> Result<HwConfig> {
        let pcm = self.raw.as_ptr();
        let dev = self.name.clone();
        let hw = HwParams::new()?;
        let p = hw.as_ptr();
        // SAFETY: (every call in this function) `pcm` is our open handle, `p` a live hw_params object, every out
        // pointer a valid local
        unsafe {
            check(ffi::snd_pcm_hw_params_any(pcm, p), "snd_pcm_hw_params_any", &dev, Phase::Open)?;
            // Only plug devices can resample; for hw this is a no-op either way
            let _ = ffi::snd_pcm_hw_params_set_rate_resample(pcm, p, c_uint::from(req.allow_resample));

            let access = req
                .access
                .candidates()
                .iter()
                .copied()
                .find(|a| ffi::snd_pcm_hw_params_set_access(pcm, p, raw_access(*a)) == 0)
                .ok_or_else(|| {
                    keyed("alsa.access_unsupported", "snd_pcm_hw_params_set_access", errno::EINVAL, &dev)
                        .arg("requested", format!("{:?}", req.access))
                })?;

            let format = req
                .formats
                .iter()
                .copied()
                .find(|f| ffi::snd_pcm_hw_params_test_format(pcm, p, raw_format(*f)) == 0)
                .ok_or_else(|| {
                    let available: Vec<&str> = SampleFormat::PREFERENCE
                        .iter()
                        .filter(|f| ffi::snd_pcm_hw_params_test_format(pcm, p, raw_format(**f)) == 0)
                        .map(|f| f.name())
                        .collect();
                    keyed("alsa.format_unsupported", "snd_pcm_hw_params_set_format", errno::EINVAL, &dev)
                        .arg("formats", available.join(","))
                })?;
            check(
                ffi::snd_pcm_hw_params_set_format(pcm, p, raw_format(format)),
                "snd_pcm_hw_params_set_format",
                &dev,
                Phase::Open,
            )?;

            let r = ffi::snd_pcm_hw_params_set_channels(pcm, p, req.channels);
            if r < 0 {
                let (mut min, mut max) = (0, 0);
                ffi::snd_pcm_hw_params_get_channels_min(p, &mut min);
                ffi::snd_pcm_hw_params_get_channels_max(p, &mut max);
                return Err(keyed("alsa.channels_unsupported", "snd_pcm_hw_params_set_channels", r, &dev)
                    .arg("requested", i64::from(req.channels))
                    .arg("min", i64::from(min))
                    .arg("max", i64::from(max)));
            }

            let r = ffi::snd_pcm_hw_params_set_rate(pcm, p, req.rate, 0);
            if r < 0 {
                let (mut min, mut max, mut dir) = (0, 0, 0);
                ffi::snd_pcm_hw_params_get_rate_min(p, &mut min, &mut dir);
                ffi::snd_pcm_hw_params_get_rate_max(p, &mut max, &mut dir);
                return Err(keyed("alsa.rate_unsupported", "snd_pcm_hw_params_set_rate", r, &dev)
                    .arg("requested", i64::from(req.rate))
                    .arg("min", i64::from(min))
                    .arg("max", i64::from(max)));
            }

            let mut period = ffi::snd_pcm_uframes_t::from(req.period_frames.max(1));
            let mut dir = 0;
            let r = ffi::snd_pcm_hw_params_set_period_size_near(pcm, p, &mut period, &mut dir);
            if r < 0 {
                return Err(keyed("alsa.period_unsupported", "snd_pcm_hw_params_set_period_size_near", r, &dev)
                    .arg("requested", i64::from(req.period_frames)));
            }
            let mut periods = req.periods.max(2);
            let mut dir = 0;
            if ffi::snd_pcm_hw_params_set_periods_near(pcm, p, &mut periods, &mut dir) < 0 {
                // Some drivers only constrain the buffer size: ask for it instead
                let mut buffer = period * ffi::snd_pcm_uframes_t::from(req.periods.max(2));
                let r = ffi::snd_pcm_hw_params_set_buffer_size_near(pcm, p, &mut buffer);
                if r < 0 {
                    return Err(keyed("alsa.period_unsupported", "snd_pcm_hw_params_set_buffer_size_near", r, &dev)
                        .arg("requested", i64::from(req.periods)));
                }
            }

            let r = ffi::snd_pcm_hw_params(pcm, p);
            if r < 0 {
                return Err(match r.saturating_abs() {
                    errno::EBUSY | errno::EACCES | errno::EPERM | errno::ENODEV => {
                        crate::error::error_from_code(r, "snd_pcm_hw_params", &dev, Phase::Running)
                    }
                    _ => keyed("alsa.config_rejected", "snd_pcm_hw_params", r, &dev),
                });
            }

            // What the device agreed to
            let (mut period, mut buffer, mut rate, mut channels, mut dir) = (0, 0, 0, 0, 0);
            ffi::snd_pcm_hw_params_get_period_size(p, &mut period, &mut dir);
            ffi::snd_pcm_hw_params_get_buffer_size(p, &mut buffer);
            ffi::snd_pcm_hw_params_get_rate(p, &mut rate, &mut dir);
            ffi::snd_pcm_hw_params_get_channels(p, &mut channels);
            let mut raw_fmt = 0;
            ffi::snd_pcm_hw_params_get_format(p, &mut raw_fmt);
            let mut raw_acc = 0;
            ffi::snd_pcm_hw_params_get_access(p, &mut raw_acc);
            let mut periods = 0;
            if ffi::snd_pcm_hw_params_get_periods(p, &mut periods, &mut dir) < 0 || periods == 0 {
                periods = (buffer / period.max(1)) as c_uint;
            }
            let format = format_from_raw(raw_fmt).unwrap_or(format);
            let access = [
                Access::MmapInterleaved,
                Access::MmapNonInterleaved,
                Access::ReadWriteInterleaved,
                Access::ReadWriteNonInterleaved,
            ]
            .into_iter()
            .find(|a| raw_access(*a) == raw_acc)
            .unwrap_or(access);
            let config = HwConfig {
                rate,
                channels,
                format,
                access,
                period_frames: u32::try_from(period).unwrap_or(u32::MAX),
                periods,
                buffer_frames: u32::try_from(buffer).unwrap_or(u32::MAX),
            };

            self.configure_software(&config)?;

            let (scratch, planes) = if access.is_mmap() {
                (Vec::new(), Vec::new())
            } else {
                let frames = config.buffer_frames as usize;
                let channels = config.channels as usize;
                let mut scratch = vec![0u8; frames * channels * format.bytes()];
                let plane = frames * format.bytes();
                let base = scratch.as_mut_ptr();
                let planes = (0..channels).map(|ch| base.add(ch * plane).cast::<c_void>()).collect();
                (scratch, planes)
            };
            self.setup = Some(Setup { config, scratch, planes });
            Ok(config)
        }
    }

    fn configure_software(&mut self, config: &HwConfig) -> Result<()> {
        let pcm = self.raw.as_ptr();
        let dev = self.name.as_str();
        let sw = SwParams::new()?;
        let s = sw.0.as_ptr();
        // SAFETY: our handle, a live sw_params object, valid out pointers
        unsafe {
            check(ffi::snd_pcm_sw_params_current(pcm, s), "snd_pcm_sw_params_current", dev, Phase::Open)?;
            let mut boundary = 0;
            check(
                ffi::snd_pcm_sw_params_get_boundary(s, &mut boundary),
                "snd_pcm_sw_params_get_boundary",
                dev,
                Phase::Open,
            )?;
            let buffer = ffi::snd_pcm_uframes_t::from(config.buffer_frames);
            let period = ffi::snd_pcm_uframes_t::from(config.period_frames);
            // Start only when told: a threshold the application pointer never reaches
            check(
                ffi::snd_pcm_sw_params_set_start_threshold(pcm, s, boundary),
                "snd_pcm_sw_params_set_start_threshold",
                dev,
                Phase::Open,
            )?;
            check(
                ffi::snd_pcm_sw_params_set_stop_threshold(pcm, s, buffer),
                "snd_pcm_sw_params_set_stop_threshold",
                dev,
                Phase::Open,
            )?;
            check(
                ffi::snd_pcm_sw_params_set_avail_min(pcm, s, period),
                "snd_pcm_sw_params_set_avail_min",
                dev,
                Phase::Open,
            )?;
            check(
                ffi::snd_pcm_sw_params_set_silence_threshold(pcm, s, 0),
                "snd_pcm_sw_params_set_silence_threshold",
                dev,
                Phase::Open,
            )?;
            check(
                ffi::snd_pcm_sw_params_set_silence_size(pcm, s, 0),
                "snd_pcm_sw_params_set_silence_size",
                dev,
                Phase::Open,
            )?;
            // C6: status time stamps on CLOCK_MONOTONIC, and on
            check(
                ffi::snd_pcm_sw_params_set_tstamp_mode(pcm, s, ffi::SND_PCM_TSTAMP_ENABLE),
                "snd_pcm_sw_params_set_tstamp_mode",
                dev,
                Phase::Open,
            )?;
            check(
                ffi::snd_pcm_sw_params_set_tstamp_type(pcm, s, ffi::SND_PCM_TSTAMP_TYPE_MONOTONIC),
                "snd_pcm_sw_params_set_tstamp_type",
                dev,
                Phase::Open,
            )?;
            check(ffi::snd_pcm_sw_params(pcm, s), "snd_pcm_sw_params", dev, Phase::Open)?;
        }
        Ok(())
    }

    /// Links this PCM to `other` (`snd_pcm_link`): start, stop and prepare act on both, and they run on one clock
    /// (U7). Fails with `alsa.link_unsupported` for devices that cannot (different cards, most plugins).
    /// Not real-time safe.
    pub fn link(&mut self, other: &mut Pcm) -> Result<()> {
        // SAFETY: two open handles we own
        let r = unsafe { ffi::snd_pcm_link(self.raw.as_ptr(), other.raw.as_ptr()) };
        if r < 0 {
            return Err(keyed("alsa.link_unsupported", "snd_pcm_link", r, &self.name).arg("other", other.name.as_str()));
        }
        self.linked = true;
        other.linked = true;
        Ok(())
    }

    /// Undoes [`link`](Pcm::link). Not real-time safe
    pub fn unlink(&mut self) -> Result<()> {
        if self.linked {
            // SAFETY: our open handle
            let r = unsafe { ffi::snd_pcm_unlink(self.raw.as_ptr()) };
            check(r, "snd_pcm_unlink", &self.name, Phase::Running)?;
            self.linked = false;
        }
        Ok(())
    }

    pub fn is_linked(&self) -> bool {
        self.linked
    }

    // ---- state changes ----

    /// Starts the stream (and every PCM linked to it). Real-time safe (one ioctl). A playback stream must have data
    /// written first ([`write_silence`](Pcm::write_silence)), or it underruns at once
    pub fn start(&mut self) -> Result<(), PcmIoError> {
        // SAFETY: our open handle
        io(unsafe { ffi::snd_pcm_start(self.raw.as_ptr()) }).map(drop)
    }

    /// Prepares the stream for a new start (after a stop or an xrun). Real-time safe
    pub fn prepare(&mut self) -> Result<(), PcmIoError> {
        // SAFETY: our open handle
        io(unsafe { ffi::snd_pcm_prepare(self.raw.as_ptr()) }).map(drop)
    }

    /// Stops at once, dropping what is in the buffer (`snd_pcm_drop`). Real-time safe
    pub fn stop_now(&mut self) -> Result<(), PcmIoError> {
        // SAFETY: our open handle
        io(unsafe { ffi::snd_pcm_drop(self.raw.as_ptr()) }).map(drop)
    }

    /// Plays what is in the buffer to the end, then stops (`snd_pcm_drain`). **Blocks** for up to one buffer: not
    /// real-time safe, never on the audio thread
    pub fn drain(&mut self) -> Result<()> {
        let pcm = self.raw.as_ptr();
        // SAFETY: our open handle; the handle goes back to non-blocking before returning
        let r = unsafe {
            ffi::snd_pcm_nonblock(pcm, 0);
            let r = ffi::snd_pcm_drain(pcm);
            ffi::snd_pcm_nonblock(pcm, 1);
            r
        };
        check(r, "snd_pcm_drain", &self.name, Phase::Running).map(drop)
    }

    /// Recovers from the error a transfer returned, leaving the stream prepared (stopped):
    /// - [`PcmIoError::Xrun`]: `snd_pcm_recover` (which prepares). Real-time safe.
    /// - [`PcmIoError::Suspended`]: one non-blocking `snd_pcm_resume`; if the device is still waking up, returns
    ///   `Err(Suspended)` and the caller tries again later (`snd_pcm_recover` would sleep 1 s in a loop here, which is
    ///   why it is not used for this case); a device that cannot resume is prepared instead.
    /// - anything else cannot be recovered and comes back as is (a disconnected device: U11).
    ///
    /// Call [`xrun_info`](Pcm::xrun_info) **before** this, to measure the lost frames (C2). After it, a playback
    /// stream needs its prefill and every stream a [`start`](Pcm::start).
    pub fn recover(&mut self, err: PcmIoError) -> Result<(), PcmIoError> {
        let pcm = self.raw.as_ptr();
        match err {
            // SAFETY: our open handle
            PcmIoError::Xrun => io(unsafe { ffi::snd_pcm_recover(pcm, -errno::EPIPE, 1) }).map(drop),
            PcmIoError::Suspended => {
                // SAFETY: our open handle
                let r = unsafe { ffi::snd_pcm_resume(pcm) };
                if r == -errno::EAGAIN {
                    Err(PcmIoError::Suspended)
                } else if r < 0 {
                    // SAFETY: our open handle
                    io(unsafe { ffi::snd_pcm_prepare(pcm) }).map(drop)
                } else {
                    Ok(())
                }
            }
            other => Err(other),
        }
    }

    // ---- state and time ----

    /// The state, from the shared status page (real-time safe; no system call for `hw`)
    pub fn state(&self) -> PcmState {
        // SAFETY: our open handle
        PcmState::from_raw(unsafe { ffi::snd_pcm_state(self.raw.as_ptr()) })
    }

    /// The full status (`snd_pcm_status`: one ioctl, into the status object allocated at open). Real-time safe
    pub fn status(&mut self) -> Result<PcmStatus, PcmIoError> {
        let s = self.status.as_ptr();
        // SAFETY: our open handle and our status object; the getters read the object
        unsafe {
            io(ffi::snd_pcm_status(self.raw.as_ptr(), s))?;
            let mut now = zero_timespec();
            let mut trigger = zero_timespec();
            ffi::snd_pcm_status_get_htstamp(s, &mut now);
            ffi::snd_pcm_status_get_trigger_htstamp(s, &mut trigger);
            Ok(PcmStatus {
                state: PcmState::from_raw(ffi::snd_pcm_status_get_state(s)),
                now: host_time(&now),
                trigger: host_time(&trigger),
                avail: ffi::snd_pcm_status_get_avail(s) as u64,
                avail_max: ffi::snd_pcm_status_get_avail_max(s) as u64,
                delay: ffi::snd_pcm_status_get_delay(s) as i64,
            })
        }
    }

    /// Frames available and the host time of the last hardware pointer update (`snd_pcm_htimestamp`), the cheap way
    /// to stamp a block (C6). Real-time safe
    pub fn htimestamp(&mut self) -> Result<(u64, HostTime), PcmIoError> {
        let mut avail = 0;
        let mut ts = zero_timespec();
        // SAFETY: our open handle, valid out pointers
        io(unsafe { ffi::snd_pcm_htimestamp(self.raw.as_ptr(), &mut avail, &mut ts) })?;
        Ok((avail, host_time(&ts)))
    }

    /// Frames ready, from the last hardware pointer update (`snd_pcm_avail_update`; no system call for mmap `hw`).
    /// Real-time safe
    pub fn avail_update(&mut self) -> Result<u64, PcmIoError> {
        // SAFETY: our open handle
        frames(unsafe { ffi::snd_pcm_avail_update(self.raw.as_ptr()) })
    }

    /// Frames ready, after syncing with the hardware (`snd_pcm_avail`: one ioctl). Real-time safe
    pub fn avail(&mut self) -> Result<u64, PcmIoError> {
        // SAFETY: our open handle
        frames(unsafe { ffi::snd_pcm_avail(self.raw.as_ptr()) })
    }

    /// Frames between the program and the converter (`snd_pcm_delay`). Real-time safe
    pub fn delay(&mut self) -> Result<i64, PcmIoError> {
        let mut d = 0;
        // SAFETY: our open handle, valid out pointer
        io(unsafe { ffi::snd_pcm_delay(self.raw.as_ptr(), &mut d) })?;
        Ok(d)
    }

    /// What an xrun left (C2): when the stream stopped and how many frames were pending then. Call it after a
    /// transfer returned [`PcmIoError::Xrun`] and before [`recover`](Pcm::recover). Real-time safe
    pub fn xrun_info(&mut self) -> Result<XrunInfo, PcmIoError> {
        let s = self.status()?;
        let at = if s.trigger.0 == 0 { s.now } else { s.trigger };
        Ok(XrunInfo { at, pending: s.avail })
    }

    /// When the stream was last started (the trigger time stamp), for [`lost_capture_frames`](crate::lost_capture_frames).
    /// Real-time safe
    pub fn trigger_time(&mut self) -> Result<HostTime, PcmIoError> {
        let s = self.status()?;
        Ok(if s.trigger.0 == 0 { s.now } else { s.trigger })
    }

    // ---- transfers (real-time safe: no allocation, no lock; one ioctl or none) ----

    /// Writes interleaved samples (`src.len()` a multiple of the channel count). Returns the frames written: fewer
    /// than given (even 0) when the buffer is full. Real-time safe
    pub fn write_interleaved(&mut self, src: &[f32]) -> Result<usize, PcmIoError> {
        let channels = self.channels()?;
        self.write_src(Src::Interleaved(src), src.len() / channels)
    }

    /// Writes one slice per channel, all of the same length. Real-time safe
    pub fn write_planar(&mut self, src: &[&[f32]]) -> Result<usize, PcmIoError> {
        let channels = self.channels()?;
        if src.len() != channels {
            return Err(PcmIoError::Other(errno::EINVAL));
        }
        let frames = src.iter().map(|c| c.len()).min().unwrap_or(0);
        self.write_src(Src::Planar(src), frames)
    }

    /// Writes `frames` of silence (the prefill before `start`). Real-time safe
    pub fn write_silence(&mut self, frames: usize) -> Result<usize, PcmIoError> {
        self.write_src(Src::Silence, frames)
    }

    /// Reads interleaved samples into `dst` (a multiple of the channel count). Returns the frames read: fewer than
    /// room (even 0) when the device has no more yet. Real-time safe
    pub fn read_interleaved(&mut self, dst: &mut [f32]) -> Result<usize, PcmIoError> {
        let channels = self.channels()?;
        let frames = dst.len() / channels;
        self.read_dst(Dst::Interleaved(dst), frames)
    }

    /// Reads into one slice per channel, all of the same length. Real-time safe
    pub fn read_planar(&mut self, dst: &mut [&mut [f32]]) -> Result<usize, PcmIoError> {
        let channels = self.channels()?;
        if dst.len() != channels {
            return Err(PcmIoError::Other(errno::EINVAL));
        }
        let frames = dst.iter().map(|c| c.len()).min().unwrap_or(0);
        self.read_dst(Dst::Planar(dst), frames)
    }

    fn channels(&self) -> Result<usize, PcmIoError> {
        self.setup.as_ref().map(|s| s.config.channels as usize).filter(|c| *c > 0).ok_or(PcmIoError::BadState)
    }

    fn write_src(&mut self, src: Src<'_>, frames: usize) -> Result<usize, PcmIoError> {
        if self.stream != Stream::Playback {
            return Err(PcmIoError::BadState);
        }
        let pcm = self.raw.as_ptr();
        let setup = self.setup.as_mut().ok_or(PcmIoError::BadState)?;
        let config = setup.config;
        let channels = config.channels as usize;
        let format = config.format;
        let bytes = format.bytes();
        let mut done = 0;
        if config.access.is_mmap() {
            while done < frames {
                let Some((areas, offset, n)) = mmap_begin(pcm, frames - done)? else { break };
                for ch in 0..channels {
                    // SAFETY: mmap_begin gave `channels` areas covering frames offset..offset+n
                    let lane = unsafe { area_lane(areas, ch, offset) };
                    encode_lane(format, lane, n, |i| src.sample(channels, ch, done + i));
                }
                // SAFETY: our handle; commits exactly the frames mmap_begin gave
                let c = unsafe { ffi::snd_pcm_mmap_commit(pcm, offset as _, n as _) };
                let c = transferred(c)?;
                done += c;
                if c < n {
                    break;
                }
            }
            return Ok(done);
        }
        let capacity = (config.buffer_frames as usize).max(1);
        while done < frames {
            let n = (frames - done).min(capacity);
            let r = match (config.access, src, format) {
                // Already in the device's layout: no copy
                (Access::ReadWriteInterleaved, Src::Interleaved(s), SampleFormat::F32) => {
                    // SAFETY: s holds at least (done + n) * channels samples (frames = s.len() / channels)
                    unsafe { ffi::snd_pcm_writei(pcm, s.as_ptr().add(done * channels).cast(), n as _) }
                }
                (Access::ReadWriteInterleaved, ..) => {
                    let base = setup.scratch.as_mut_ptr();
                    for ch in 0..channels {
                        // SAFETY: scratch holds capacity * channels samples; n <= capacity
                        let lane = Lane { ptr: unsafe { base.add(ch * bytes) }, stride: channels * bytes };
                        encode_lane(format, lane, n, |i| src.sample(channels, ch, done + i));
                    }
                    // SAFETY: n interleaved frames are in scratch
                    unsafe { ffi::snd_pcm_writei(pcm, base.cast(), n as _) }
                }
                _ => {
                    for (ch, plane) in setup.planes.iter().enumerate() {
                        let lane = Lane { ptr: plane.cast(), stride: bytes };
                        encode_lane(format, lane, n, |i| src.sample(channels, ch, done + i));
                    }
                    // SAFETY: one plane per channel, each holding at least n samples
                    unsafe { ffi::snd_pcm_writen(pcm, setup.planes.as_mut_ptr(), n as _) }
                }
            };
            let w = transferred(r)?;
            done += w;
            if w < n {
                break;
            }
        }
        Ok(done)
    }

    fn read_dst(&mut self, mut dst: Dst<'_, '_>, frames: usize) -> Result<usize, PcmIoError> {
        if self.stream != Stream::Capture {
            return Err(PcmIoError::BadState);
        }
        let pcm = self.raw.as_ptr();
        let setup = self.setup.as_mut().ok_or(PcmIoError::BadState)?;
        let config = setup.config;
        let channels = config.channels as usize;
        let format = config.format;
        let bytes = format.bytes();
        let mut done = 0;
        if config.access.is_mmap() {
            while done < frames {
                let Some((areas, offset, n)) = mmap_begin(pcm, frames - done)? else { break };
                for ch in 0..channels {
                    // SAFETY: as in write_src
                    let lane = unsafe { area_lane(areas, ch, offset) };
                    decode_lane(format, lane, n, |i, x| dst.put(channels, ch, done + i, x));
                }
                // SAFETY: our handle; commits exactly the frames mmap_begin gave
                let c = unsafe { ffi::snd_pcm_mmap_commit(pcm, offset as _, n as _) };
                let c = transferred(c)?;
                done += c;
                if c < n {
                    break;
                }
            }
            return Ok(done);
        }
        let capacity = (config.buffer_frames as usize).max(1);
        while done < frames {
            let n = (frames - done).min(capacity);
            let r = match (config.access, &mut dst, format) {
                (Access::ReadWriteInterleaved, Dst::Interleaved(d), SampleFormat::F32) => {
                    // SAFETY: d has room for at least (done + n) * channels samples
                    unsafe { ffi::snd_pcm_readi(pcm, d.as_mut_ptr().add(done * channels).cast(), n as _) }
                }
                (Access::ReadWriteInterleaved, ..) => {
                    let base = setup.scratch.as_mut_ptr();
                    // SAFETY: scratch has room for capacity interleaved frames
                    let r = unsafe { ffi::snd_pcm_readi(pcm, base.cast(), n as _) };
                    if r > 0 {
                        for ch in 0..channels {
                            // SAFETY: as in write_src
                            let lane = Lane { ptr: unsafe { base.add(ch * bytes) }, stride: channels * bytes };
                            decode_lane(format, lane, r as usize, |i, x| dst.put(channels, ch, done + i, x));
                        }
                    }
                    r
                }
                _ => {
                    // SAFETY: one plane per channel with room for n samples
                    let r = unsafe { ffi::snd_pcm_readn(pcm, setup.planes.as_mut_ptr(), n as _) };
                    if r > 0 {
                        for (ch, plane) in setup.planes.iter().enumerate() {
                            let lane = Lane { ptr: plane.cast(), stride: bytes };
                            decode_lane(format, lane, r as usize, |i, x| dst.put(channels, ch, done + i, x));
                        }
                    }
                    r
                }
            };
            let got = transferred(r)?;
            done += got;
            if got < n {
                break;
            }
        }
        Ok(done)
    }
}

impl Drop for Pcm {
    fn drop(&mut self) {
        // SAFETY: our handle and status object, released once; nothing else refers to them
        unsafe {
            if self.linked {
                ffi::snd_pcm_unlink(self.raw.as_ptr());
            }
            ffi::snd_pcm_close(self.raw.as_ptr());
            ffi::snd_pcm_status_free(self.status.as_ptr());
        }
    }
}

/// `Ok` for a non-negative return, the real-time error for a negative one
fn io(r: c_int) -> Result<c_int, PcmIoError> {
    if r < 0 { Err(PcmIoError::from_code(r)) } else { Ok(r) }
}

fn frames(r: ffi::snd_pcm_sframes_t) -> Result<u64, PcmIoError> {
    if r < 0 { Err(PcmIoError::from_code(r as c_int)) } else { Ok(r as u64) }
}

/// Frames a transfer moved: `EAGAIN` is 0 frames, not an error
fn transferred(r: ffi::snd_pcm_sframes_t) -> Result<usize, PcmIoError> {
    if r == -(errno::EAGAIN as ffi::snd_pcm_sframes_t) {
        Ok(0)
    } else if r < 0 {
        Err(PcmIoError::from_code(r as c_int))
    } else {
        Ok(r as usize)
    }
}

/// `snd_pcm_mmap_begin` for up to `want` frames, after `snd_pcm_avail_update`. `None` when no frame is available
fn mmap_begin(
    pcm: *mut ffi::snd_pcm_t,
    want: usize,
) -> Result<Option<(*const ffi::snd_pcm_channel_area_t, usize, usize)>, PcmIoError> {
    // SAFETY: an open handle of the caller
    let avail = unsafe { ffi::snd_pcm_avail_update(pcm) };
    let avail = transferred(avail)?;
    if avail == 0 {
        return Ok(None);
    }
    let mut areas = ptr::null();
    let mut offset = 0;
    let mut n = want.min(avail) as ffi::snd_pcm_uframes_t;
    // SAFETY: valid out pointers; the areas stay valid until the commit
    io(unsafe { ffi::snd_pcm_mmap_begin(pcm, &mut areas, &mut offset, &mut n) })?;
    if n == 0 || areas.is_null() {
        return Ok(None);
    }
    Ok(Some((areas, offset as usize, n as usize)))
}

/// The lane of channel `ch` at frame `offset` of the mmap areas.
///
/// # Safety
/// `areas` must be the array snd_pcm_mmap_begin returned, with more than `ch` entries
unsafe fn area_lane(areas: *const ffi::snd_pcm_channel_area_t, ch: usize, offset: usize) -> Lane {
    // SAFETY: the caller's contract
    let a = unsafe { *areas.add(ch) };
    let bit = a.first as usize + offset * a.step as usize;
    // SAFETY: inside the mapped buffer (ALSA's layout: first and step are in bits, whole bytes for our formats)
    Lane { ptr: unsafe { a.addr.cast::<u8>().add(bit / 8) }, stride: a.step as usize / 8 }
}

#[inline]
fn encode_lane(format: SampleFormat, lane: Lane, n: usize, mut sample: impl FnMut(usize) -> f32) {
    let bytes = format.bytes();
    for i in 0..n {
        // SAFETY: the lane points into a buffer with room for n samples of `bytes` at `stride` (callers above)
        let out = unsafe { std::slice::from_raw_parts_mut(lane.ptr.add(i * lane.stride), bytes) };
        format.encode(sample(i), out);
    }
}

#[inline]
fn decode_lane(format: SampleFormat, lane: Lane, n: usize, mut put: impl FnMut(usize, f32)) {
    let bytes = format.bytes();
    for i in 0..n {
        // SAFETY: as encode_lane
        let input = unsafe { std::slice::from_raw_parts(lane.ptr.add(i * lane.stride), bytes) };
        put(i, format.decode(input));
    }
}

/// The text libasound gives for an error code, for logs (`snd_strerror`)
pub fn strerror(code: i32) -> String {
    // SAFETY: snd_strerror returns a static string for any code
    text(unsafe { ffi::snd_strerror(code) })
}
