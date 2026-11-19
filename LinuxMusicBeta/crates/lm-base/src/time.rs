//! The host clock (interfaces.md, C6): nanoseconds of the system's monotonic clock (`CLOCK_MONOTONIC` on Linux, the
//! same clock on macOS), the same one ALSA is configured to stamp its PCM status with. Never the wall clock.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A point of the monotonic host clock, in nanoseconds
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostTime(pub u64);

impl HostTime {
    /// The time now, from the system's monotonic clock
    pub fn now() -> Self {
        let ts = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        let nanos = (ts.tv_sec as u64).saturating_mul(1_000_000_000).saturating_add(ts.tv_nsec as u64);
        Self(nanos)
    }

    pub fn nanos(self) -> u64 {
        self.0
    }

    /// `self - earlier`, in nanoseconds, never negative
    pub fn since(self, earlier: HostTime) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    /// Converts a span of host time to samples at `rate`, rounding to the nearest sample
    pub fn nanos_to_samples(nanos: u64, rate: u32) -> u64 {
        ((u128::from(nanos) * u128::from(rate) + 500_000_000) / 1_000_000_000) as u64
    }
}

impl fmt::Display for HostTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:09}", self.0 / 1_000_000_000, self.0 % 1_000_000_000)
    }
}

/// Where "now" comes from: the system in the program, a hand-moved clock in tests, so that timing rules (the 200 ms of
/// the loading states, T10) are tested deterministically
pub trait Clock: Send + Sync + fmt::Debug {
    fn now(&self) -> HostTime;
}

/// The system's monotonic clock
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> HostTime {
        HostTime::now()
    }
}

/// A clock that only moves when a test moves it
#[derive(Clone, Debug, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn new(start: HostTime) -> Self {
        Self(Arc::new(AtomicU64::new(start.0)))
    }

    pub fn advance_nanos(&self, nanos: u64) {
        self.0.fetch_add(nanos, Ordering::SeqCst);
    }

    pub fn advance_millis(&self, millis: u64) {
        self.advance_nanos(millis.saturating_mul(1_000_000));
    }

    pub fn set(&self, time: HostTime) {
        self.0.store(time.0, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> HostTime {
        HostTime(self.0.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_never_goes_back() {
        let a = HostTime::now();
        let b = HostTime::now();
        assert!(b >= a);
    }

    #[test]
    fn manual_clock_moves_only_by_hand() {
        let clock = ManualClock::new(HostTime(1_000));
        assert_eq!(clock.now(), HostTime(1_000));
        clock.advance_millis(200);
        assert_eq!(clock.now(), HostTime(200_001_000));
    }

    #[test]
    fn nanos_to_samples_rounds() {
        assert_eq!(HostTime::nanos_to_samples(1_000_000_000, 48_000), 48_000);
        assert_eq!(HostTime::nanos_to_samples(10_416, 48_000), 0); // 0.49999 sample
        assert_eq!(HostTime::nanos_to_samples(10_417, 48_000), 1);
    }
}
