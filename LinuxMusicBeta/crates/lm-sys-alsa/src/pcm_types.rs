//! The plain data of the PCM: what is asked, what was negotiated, the state and the status. Platform independent, so
//! `lm-io` can keep its configuration types on every platform and the arithmetic (C2) is tested everywhere.

use crate::SampleFormat;
use lm_base::HostTime;

/// The direction of a PCM
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stream {
    Playback,
    Capture,
}

impl Stream {
    pub const fn name(self) -> &'static str {
        match self {
            Stream::Playback => "playback",
            Stream::Capture => "capture",
        }
    }
}

/// How the samples move between the program and the device
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    /// Straight into the device's buffer, frames interleaved (the usual for `hw` devices)
    MmapInterleaved,
    /// Straight into the device's buffer, one area per channel (common on multichannel cards, e.g. RME)
    MmapNonInterleaved,
    /// `snd_pcm_writei`/`readi`: through a copy, frames interleaved
    ReadWriteInterleaved,
    /// `snd_pcm_writen`/`readn`: through a copy, one buffer per channel
    ReadWriteNonInterleaved,
}

impl Access {
    pub const fn is_mmap(self) -> bool {
        matches!(self, Access::MmapInterleaved | Access::MmapNonInterleaved)
    }

    pub const fn is_interleaved(self) -> bool {
        matches!(self, Access::MmapInterleaved | Access::ReadWriteInterleaved)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Access::MmapInterleaved => "mmap_interleaved",
            Access::MmapNonInterleaved => "mmap_noninterleaved",
            Access::ReadWriteInterleaved => "rw_interleaved",
            Access::ReadWriteNonInterleaved => "rw_noninterleaved",
        }
    }
}

/// Which access types the configuration may pick, tried in the order listed
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum AccessChoice {
    /// mmap if the device has it (no copy, the lowest latency), else read/write
    #[default]
    MmapPreferred,
    /// mmap only: fails with `alsa.access_unsupported` if the device has none
    MmapOnly,
    /// read/write only
    ReadWriteOnly,
}

impl AccessChoice {
    /// The access types to try, in order
    pub fn candidates(self) -> &'static [Access] {
        const MMAP_FIRST: [Access; 4] = [
            Access::MmapInterleaved,
            Access::MmapNonInterleaved,
            Access::ReadWriteInterleaved,
            Access::ReadWriteNonInterleaved,
        ];
        match self {
            AccessChoice::MmapPreferred => &MMAP_FIRST,
            AccessChoice::MmapOnly => &MMAP_FIRST[..2],
            AccessChoice::ReadWriteOnly => &MMAP_FIRST[2..],
        }
    }
}

/// What to ask of a device. Rate and channels are exact (an `hw` device in exclusive mode, U7, U10); the period size
/// and count are a wish, and the device answers with what it can do (C7)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HwRequest {
    pub rate: u32,
    pub channels: u32,
    /// Sample formats in order of preference; the first the device has is used
    pub formats: Vec<SampleFormat>,
    /// Frames per period (the block the device wakes the program for)
    pub period_frames: u32,
    /// Periods in the buffer (2 or 3 for low latency)
    pub periods: u32,
    pub access: AccessChoice,
    /// Let ALSA resample when the device does not have the rate (only `plughw`/`default` do; `hw` never does)
    pub allow_resample: bool,
}

impl HwRequest {
    /// Every format, mmap preferred, no resampling
    pub fn new(rate: u32, channels: u32, period_frames: u32, periods: u32) -> Self {
        Self {
            rate,
            channels,
            formats: SampleFormat::PREFERENCE.to_vec(),
            period_frames,
            periods,
            access: AccessChoice::default(),
            allow_resample: false,
        }
    }
}

/// What the device agreed to: the configuration in use. `period_frames` is the negotiated period, the
/// `ProcessSpec::max_block` of the engine (C7)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HwConfig {
    pub rate: u32,
    pub channels: u32,
    pub format: SampleFormat,
    pub access: Access,
    pub period_frames: u32,
    pub periods: u32,
    pub buffer_frames: u32,
}

impl HwConfig {
    /// The buffer's length in host time: the worst-case latency of the device's buffer
    pub fn buffer_nanos(&self) -> u64 {
        u64::from(self.buffer_frames) * 1_000_000_000 / u64::from(self.rate.max(1))
    }
}

/// The state of a PCM (`snd_pcm_state_t`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PcmState {
    Open,
    Setup,
    Prepared,
    Running,
    Xrun,
    Draining,
    Paused,
    Suspended,
    Disconnected,
    /// A value this binding does not know (a newer alsa-lib)
    Unknown(i32),
}

impl PcmState {
    pub fn from_raw(v: i32) -> Self {
        match v {
            0 => PcmState::Open,
            1 => PcmState::Setup,
            2 => PcmState::Prepared,
            3 => PcmState::Running,
            4 => PcmState::Xrun,
            5 => PcmState::Draining,
            6 => PcmState::Paused,
            7 => PcmState::Suspended,
            8 => PcmState::Disconnected,
            other => PcmState::Unknown(other),
        }
    }
}

/// A snapshot of `snd_pcm_status`, with the time stamps on the host clock (`CLOCK_MONOTONIC`, C6)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmStatus {
    pub state: PcmState,
    /// When the status was taken (the last hardware pointer update)
    pub now: HostTime,
    /// When the stream was last started or stopped (for a stream stopped by an xrun: the moment of the xrun)
    pub trigger: HostTime,
    /// Frames ready to be read (capture) or written (playback)
    pub avail: u64,
    /// The largest `avail` since the last status
    pub avail_max: u64,
    /// Frames between the program and the converter: what was written and not yet heard, or heard and not yet read
    pub delay: i64,
}

/// What an xrun left, taken with the status before recovering (C2)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XrunInfo {
    /// When the stream stopped
    pub at: HostTime,
    /// Frames between the program's position and the hardware's when it stopped: for a capture stream, frames that
    /// were captured and will never be read; for playback, how far the hardware ran past the written data
    pub pending: u64,
}

/// Capture frames lost to an xrun (C2): the frames captured and never read when the stream stopped, plus the frames
/// of the gap between the stop and the new start (`restarted`, the trigger time stamp after `start`). The `DiskWriter`
/// writes exactly this much silence, so the take stays on the grid
pub fn lost_capture_frames(xrun: XrunInfo, restarted: HostTime, rate: u32) -> u64 {
    xrun.pending + HostTime::nanos_to_samples(restarted.since(xrun.at), rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_frames_count_the_buffer_and_the_gap() {
        let xrun = XrunInfo { at: HostTime(1_000_000_000), pending: 256 };
        // 10 ms later at 48 kHz: 480 frames of gap
        assert_eq!(lost_capture_frames(xrun, HostTime(1_010_000_000), 48_000), 256 + 480);
        // a restart time before the xrun (clock noise) adds nothing
        assert_eq!(lost_capture_frames(xrun, HostTime(999_000_000), 48_000), 256);
    }

    #[test]
    fn access_choices_try_in_order() {
        assert_eq!(AccessChoice::MmapPreferred.candidates()[0], Access::MmapInterleaved);
        assert!(AccessChoice::MmapOnly.candidates().iter().all(|a| a.is_mmap()));
        assert!(AccessChoice::ReadWriteOnly.candidates().iter().all(|a| !a.is_mmap()));
    }

    #[test]
    fn states_map_from_alsa() {
        assert_eq!(PcmState::from_raw(4), PcmState::Xrun);
        assert_eq!(PcmState::from_raw(8), PcmState::Disconnected);
        assert_eq!(PcmState::from_raw(42), PcmState::Unknown(42));
    }

    #[test]
    fn buffer_time() {
        let config = HwConfig {
            rate: 48_000,
            channels: 2,
            format: SampleFormat::S32,
            access: Access::MmapInterleaved,
            period_frames: 128,
            periods: 2,
            buffer_frames: 256,
        };
        assert_eq!(config.buffer_nanos(), 5_333_333);
    }
}
