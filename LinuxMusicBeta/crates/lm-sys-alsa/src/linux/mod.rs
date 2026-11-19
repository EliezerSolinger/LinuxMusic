//! The Linux part: everything that calls libasound.

pub mod enumerate;
pub(crate) mod ffi;
pub mod pcm;
pub mod poll;
pub mod seq;

use crate::error::{Phase, error_from_code, with_context};
use lm_base::{Error, HostTime, Result};
use std::ffi::{CStr, CString, c_char};

/// A C string for libasound, or `alsa.invalid_argument` if `s` has a NUL inside
pub(crate) fn c_string(s: &str, what: &'static str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::new("alsa.invalid_argument").arg("argument", what).arg("value", s))
}

/// The text of a C string libasound owns, or empty for a null pointer
pub(crate) fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: libasound returns NUL-terminated strings that stay valid while the object they belong to lives; the
    // callers read them before freeing that object
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// `Ok(code)` for a non-negative return, the error for a negative one
pub(crate) fn check(code: i32, call: &'static str, device: &str, phase: Phase) -> Result<i32> {
    if code < 0 { Err(error_from_code(code, call, device, phase)) } else { Ok(code) }
}

/// An error with its own key and the call's context
pub(crate) fn keyed(key: &'static str, call: &'static str, code: i32, device: &str) -> Error {
    with_context(Error::new(key), call, code.saturating_abs(), device)
}

/// A `timespec` of the monotonic clock as host time (negative fields, which ALSA never gives, become 0)
pub(crate) fn host_time(ts: &libc::timespec) -> HostTime {
    let secs = u64::try_from(ts.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(ts.tv_nsec).unwrap_or(0);
    HostTime(secs.saturating_mul(1_000_000_000).saturating_add(nanos))
}

pub(crate) const fn zero_timespec() -> libc::timespec {
    libc::timespec { tv_sec: 0, tv_nsec: 0 }
}
