//! ALSA's error codes as [`lm_base::Error`]s with translatable keys (interfaces.md, Parte 2, rule 7; U8), and the
//! small error the real-time calls return.
//!
//! Platform independent: the numbers are Linux's (`asm-generic/errno.h`, the same on x86-64 and aarch64), written
//! here so the mapping is tested on every platform.
//!
//! The keys (every one carries `device` when there is one, and `call` + `errno` for the logs):
//!
//! | key | when |
//! |---|---|
//! | `alsa.device_busy` | `EBUSY`: another program has the device (U8: ask with ReserveDevice1, `lm-sys-linux`) |
//! | `alsa.permission_denied` | `EACCES`/`EPERM`: no access to `/dev/snd/*` (the user is not in the `audio` group) |
//! | `alsa.device_not_found` | `ENOENT`/`ENXIO`/`ENODEV` when opening: no such device or card |
//! | `alsa.device_disconnected` | `ENODEV`/`ESHUTDOWN`/`EBADFD` on an open device: the card went away (U11) |
//! | `alsa.xrun` | `EPIPE`: an underrun (playback) or overrun (capture) |
//! | `alsa.suspended` | `ESTRPIPE`: the machine was suspended |
//! | `alsa.access_unsupported` | no access type (mmap or read/write) the request allows |
//! | `alsa.format_unsupported` | none of the requested sample formats; `formats` lists the device's |
//! | `alsa.channels_unsupported` | `requested`, `min`, `max` |
//! | `alsa.rate_unsupported` | `requested`, `min`, `max` |
//! | `alsa.period_unsupported` | the period size or count could not be set |
//! | `alsa.config_rejected` | the device refused the whole configuration |
//! | `alsa.link_unsupported` | `snd_pcm_link` refused: the two PCMs cannot share a clock (U7) |
//! | `alsa.not_configured` | a call that needs [`configure`](crate::Pcm::configure) first |
//! | `alsa.seq_unavailable` | no sequencer (`/dev/snd/seq` missing: the `snd-seq` module is not loaded) |
//! | `alsa.out_of_memory` | `ENOMEM` |
//! | `alsa.invalid_argument` | a bad argument from our side (an interior NUL in a name, a wrong slice length) |
//! | `alsa.error` | anything else |

use lm_base::Error;

/// Linux errno values (asm-generic), the ones ALSA returns negated
pub mod errno {
    pub const EPERM: i32 = 1;
    pub const ENOENT: i32 = 2;
    pub const EINTR: i32 = 4;
    pub const EIO: i32 = 5;
    pub const ENXIO: i32 = 6;
    pub const EAGAIN: i32 = 11;
    pub const ENOMEM: i32 = 12;
    pub const EACCES: i32 = 13;
    pub const EBUSY: i32 = 16;
    pub const ENODEV: i32 = 19;
    pub const EINVAL: i32 = 22;
    pub const ENOSPC: i32 = 28;
    pub const EPIPE: i32 = 32;
    pub const ENOSYS: i32 = 38;
    pub const EBADFD: i32 = 77;
    pub const ESTRPIPE: i32 = 86;
    pub const ESHUTDOWN: i32 = 108;
    pub const ETIMEDOUT: i32 = 110;
}

/// Where the code came from: the same `ENODEV` means "no such device" when opening and "the card went away" later
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Open,
    Running,
}

/// The error for a negative ALSA return `code` from `call` on `device` (empty when there is no device)
pub fn error_from_code(code: i32, call: &'static str, device: &str, phase: Phase) -> Error {
    let e = code.saturating_abs();
    let key = match e {
        errno::EBUSY => "alsa.device_busy",
        errno::EACCES | errno::EPERM => "alsa.permission_denied",
        errno::ENOENT | errno::ENXIO => "alsa.device_not_found",
        errno::ENODEV => match phase {
            Phase::Open => "alsa.device_not_found",
            Phase::Running => "alsa.device_disconnected",
        },
        errno::ESHUTDOWN | errno::EBADFD if phase == Phase::Running => "alsa.device_disconnected",
        errno::EPIPE => "alsa.xrun",
        errno::ESTRPIPE => "alsa.suspended",
        errno::ENOMEM => "alsa.out_of_memory",
        _ => "alsa.error",
    };
    with_context(Error::new(key), call, e, device)
}

pub(crate) fn with_context(error: Error, call: &'static str, errno: i32, device: &str) -> Error {
    let error = error.arg("call", call).arg("errno", i64::from(errno));
    if device.is_empty() { error } else { error.arg("device", device) }
}

/// The error of the real-time calls (read, write, avail, status, poll): `Copy`, no allocation, so it can be made and
/// matched on the audio thread. [`PcmIoError::to_error`] makes the full [`Error`] off the audio thread
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmIoError {
    /// `EPIPE`: underrun or overrun. Recover with `Pcm::recover`, then measure the lost frames (C2)
    Xrun,
    /// `ESTRPIPE`: the system was suspended
    Suspended,
    /// The card went away (`ENODEV`, `ESHUTDOWN`, or the PCM in the `DISCONNECTED` state) (U11)
    Disconnected,
    /// The PCM is in a state where the call makes no sense (`EBADFD`), e.g. not started
    BadState,
    /// Any other negative return, as the positive errno
    Other(i32),
}

impl PcmIoError {
    /// From a negative ALSA return. `EAGAIN` is not an error for the callers (they get 0 frames) and never comes here
    pub fn from_code(code: i32) -> Self {
        match code.saturating_abs() {
            errno::EPIPE => PcmIoError::Xrun,
            errno::ESTRPIPE => PcmIoError::Suspended,
            errno::ENODEV | errno::ESHUTDOWN => PcmIoError::Disconnected,
            errno::EBADFD => PcmIoError::BadState,
            e => PcmIoError::Other(e),
        }
    }

    /// The negative code ALSA uses for this error
    pub fn code(self) -> i32 {
        -match self {
            PcmIoError::Xrun => errno::EPIPE,
            PcmIoError::Suspended => errno::ESTRPIPE,
            PcmIoError::Disconnected => errno::ENODEV,
            PcmIoError::BadState => errno::EBADFD,
            PcmIoError::Other(e) => e,
        }
    }

    /// The full error, with its key. Allocates: not on the audio thread
    pub fn to_error(self, call: &'static str, device: &str) -> Error {
        match self {
            PcmIoError::BadState => with_context(Error::new("alsa.bad_state"), call, errno::EBADFD, device),
            other => error_from_code(other.code(), call, device, Phase::Running),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_and_denied_are_different_keys() {
        assert_eq!(error_from_code(-errno::EBUSY, "snd_pcm_open", "hw:0", Phase::Open).key(), "alsa.device_busy");
        assert_eq!(
            error_from_code(-errno::EACCES, "snd_pcm_open", "hw:0", Phase::Open).key(),
            "alsa.permission_denied"
        );
        assert_eq!(error_from_code(-errno::EPERM, "snd_seq_open", "", Phase::Open).key(), "alsa.permission_denied");
    }

    #[test]
    fn enodev_depends_on_the_phase() {
        assert_eq!(error_from_code(-errno::ENODEV, "x", "hw:9", Phase::Open).key(), "alsa.device_not_found");
        assert_eq!(error_from_code(-errno::ENODEV, "x", "hw:9", Phase::Running).key(), "alsa.device_disconnected");
    }

    #[test]
    fn errors_carry_their_context() {
        let e = error_from_code(-errno::EINVAL, "snd_pcm_hw_params", "hw:1", Phase::Open);
        assert_eq!(e.to_string(), "alsa.error call=\"snd_pcm_hw_params\" errno=22 device=\"hw:1\"");
    }

    #[test]
    fn io_errors_round_trip_their_codes() {
        for e in [
            PcmIoError::Xrun,
            PcmIoError::Suspended,
            PcmIoError::Disconnected,
            PcmIoError::BadState,
            PcmIoError::Other(5),
        ] {
            assert_eq!(PcmIoError::from_code(e.code()), e);
        }
        assert_eq!(PcmIoError::Xrun.to_error("snd_pcm_readi", "hw:0").key(), "alsa.xrun");
        assert_eq!(PcmIoError::Disconnected.to_error("snd_pcm_readi", "hw:0").key(), "alsa.device_disconnected");
    }
}
