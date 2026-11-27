//! Waiting: the blocking wait of the audio thread on its PCMs ([`PcmPoll`]) and the wake-up any thread can send to a
//! waiting thread ([`Waker`]), so a thread blocked in a wait is stopped by a message, never killed (plano do ciclo
//! de vida: no `terminate()`).

use super::ffi;
use crate::error::PcmIoError;
use crate::{Pcm, Stream};
use lm_base::{Error, Result};
use std::ffi::c_uint;
use std::fmt;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::time::Duration;

/// A wake-up for a thread blocked in [`PcmPoll::wait`] or in the MIDI reader: an `eventfd`. `Send + Sync`; share it
/// with an `Arc`. [`wake`](Waker::wake) is real-time safe (one `write`) and never blocks
#[derive(Debug)]
pub struct Waker {
    fd: OwnedFd,
}

impl Waker {
    /// Not real-time safe (a system call that allocates a kernel object)
    pub fn new() -> Result<Self> {
        // SAFETY: plain system call; on success the fd is ours
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(Error::new("alsa.waker_failed").arg("errno", i64::from(last_errno())));
        }
        // SAFETY: a fresh descriptor nobody else owns
        Ok(Self { fd: unsafe { OwnedFd::from_raw_fd(fd) } })
    }

    /// Wakes the waiting thread (or the next wait, if nobody is waiting now)
    pub fn wake(&self) {
        let one: u64 = 1;
        // SAFETY: writes 8 bytes from a valid u64 to our eventfd. A full counter (EAGAIN) already means "woken"
        unsafe { libc::write(self.fd.as_raw_fd(), (&raw const one).cast(), 8) };
    }

    /// Clears a pending wake-up; true if there was one
    pub(crate) fn clear(&self) -> bool {
        let mut v: u64 = 0;
        // SAFETY: reads 8 bytes into a valid u64 from our non-blocking eventfd
        let r = unsafe { libc::read(self.fd.as_raw_fd(), (&raw mut v).cast(), 8) };
        r == 8 && v > 0
    }

    pub(crate) fn pollfd(&self) -> libc::pollfd {
        libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 }
    }
}

pub(crate) fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// What one [`PcmPoll::wait`] saw. Bit `i` of `ready` and `errors` is the `i`-th PCM given to [`PcmPoll::new`]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollOutcome {
    /// PCMs ready: a playback with room for a period, a capture with a period to read
    pub ready: u32,
    /// PCMs in error (`POLLERR`/`POLLHUP`/`POLLNVAL`): an xrun, a suspend or a removed card. The next transfer or
    /// [`Pcm::state`] says which
    pub errors: u32,
    /// The [`Waker`] was woken (and is now cleared)
    pub woken: bool,
}

impl PollOutcome {
    pub fn is_ready(&self, index: usize) -> bool {
        self.ready & (1 << index) != 0
    }

    pub fn has_error(&self, index: usize) -> bool {
        self.errors & (1 << index) != 0
    }

    /// Nothing happened before the timeout (or a signal interrupted the wait)
    pub fn is_empty(&self) -> bool {
        self.ready == 0 && self.errors == 0 && !self.woken
    }
}

/// The poll set of up to 32 PCMs and an optional [`Waker`]: built once off the audio thread (it allocates), then
/// [`wait`](PcmPoll::wait) is real-time safe (one `poll` and one `revents` per PCM, no allocation). `Send`, not
/// `Sync`; moves to the audio thread with the PCMs
pub struct PcmPoll {
    fds: Vec<libc::pollfd>,
    /// For each PCM: first descriptor, count, direction
    spans: Vec<(usize, usize, Stream)>,
    waker: Option<Arc<Waker>>,
}

impl fmt::Debug for PcmPoll {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PcmPoll").field("descriptors", &self.fds.len()).field("pcms", &self.spans.len()).finish()
    }
}

impl PcmPoll {
    /// The poll set of `pcms` (configured), in this order, plus the waker. Not real-time safe
    pub fn new(pcms: &[&Pcm], waker: Option<Arc<Waker>>) -> Result<Self> {
        if pcms.len() > 32 {
            return Err(Error::new("alsa.invalid_argument").arg("argument", "pcms").arg("count", pcms.len() as i64));
        }
        let mut fds = Vec::new();
        let mut spans = Vec::with_capacity(pcms.len());
        for pcm in pcms {
            // SAFETY: an open handle
            let count = unsafe { ffi::snd_pcm_poll_descriptors_count(pcm.raw()) };
            let count = super::check(count, "snd_pcm_poll_descriptors_count", pcm.name(), crate::error::Phase::Running)?
                as usize;
            let first = fds.len();
            fds.resize(first + count, libc::pollfd { fd: -1, events: 0, revents: 0 });
            // SAFETY: room for `count` descriptors at `first`
            let got = unsafe { ffi::snd_pcm_poll_descriptors(pcm.raw(), fds.as_mut_ptr().add(first), count as c_uint) };
            let got = super::check(got, "snd_pcm_poll_descriptors", pcm.name(), crate::error::Phase::Running)? as usize;
            if got > count {
                return Err(Error::new("alsa.invalid_argument").arg("argument", "poll_descriptors").arg("count", got as i64));
            }
            fds.truncate(first + got);
            spans.push((first, got, pcm.stream()));
        }
        if let Some(w) = &waker {
            fds.push(w.pollfd());
        }
        Ok(Self { fds, spans, waker })
    }

    /// Waits until a PCM is ready or in error, the waker is woken, or the timeout passes (`None`: no timeout).
    /// `pcms` must be the same PCMs, in the same order, as given to [`new`](PcmPoll::new). Real-time safe
    pub fn wait(&mut self, pcms: &[&Pcm], timeout: Option<Duration>) -> Result<PollOutcome, PcmIoError> {
        // A different set would poll the descriptors of other PCMs: refused in release too (EINVAL), without allocating
        if pcms.len() != self.spans.len() {
            return Err(PcmIoError::Other(libc::EINVAL));
        }
        for fd in &mut self.fds {
            fd.revents = 0;
        }
        let ms = timeout.map_or(-1, |t| i32::try_from(t.as_millis()).unwrap_or(i32::MAX));
        // SAFETY: a valid array of `len` pollfds
        let r = unsafe { libc::poll(self.fds.as_mut_ptr(), self.fds.len() as libc::nfds_t, ms) };
        let mut out = PollOutcome::default();
        if r < 0 {
            let e = last_errno();
            return if e == libc::EINTR { Ok(out) } else { Err(PcmIoError::Other(e)) };
        }
        if r == 0 {
            return Ok(out);
        }
        for (i, (pcm, &(first, count, stream))) in pcms.iter().zip(&self.spans).enumerate() {
            let mut revents: u16 = 0;
            // SAFETY: the descriptors this PCM gave, in our array
            let r = unsafe {
                ffi::snd_pcm_poll_descriptors_revents(
                    pcm.raw(),
                    self.fds.as_mut_ptr().add(first),
                    count as c_uint,
                    &mut revents,
                )
            };
            if r < 0 {
                return Err(PcmIoError::from_code(r));
            }
            let revents = revents as i16;
            if revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                out.errors |= 1 << i;
            }
            let want = match stream {
                Stream::Playback => libc::POLLOUT,
                Stream::Capture => libc::POLLIN,
            };
            if revents & want != 0 {
                out.ready |= 1 << i;
            }
        }
        if let Some(w) = &self.waker {
            let last = self.fds[self.fds.len() - 1];
            if last.revents & libc::POLLIN != 0 {
                out.woken = w.clear();
            }
        }
        Ok(out)
    }
}
