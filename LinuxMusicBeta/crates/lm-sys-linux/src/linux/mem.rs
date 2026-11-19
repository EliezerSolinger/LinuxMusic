//! Memory locking (H1, revisão V6): a page fault on the audio thread is a click, so what the audio thread touches
//! stays in RAM. The rule:
//!
//! - **`mlockall(MCL_CURRENT | MCL_FUTURE)` only when `RLIMIT_MEMLOCK` is unlimited** ([`lock_all`]). With a limit,
//!   `MCL_FUTURE` makes every later allocation that crosses the limit fail (`mmap` returns `EAGAIN`, the allocator
//!   aborts): the whole program would die of an ordinary allocation.
//! - Otherwise, **selective locking**: the audio buffers in [`LockedBuffer`]s (their own pages, locked while they
//!   live) and each audio thread's stack ([`lock_current_stack`]), counted against the limit, failing with a clear
//!   error instead.
//! - Stacks are **pre-touched** ([`prefault_stack`]) so the first deep call of the audio thread does not fault.
//!
//! None of this is real-time safe: lock on the thread that prepares, before playing.

use super::rt::{Limit, get_limit, os_error};
use lm_base::{Error, Result};
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

fn page_size() -> usize {
    // SAFETY: plain call
    let p = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if p > 0 { p as usize } else { 4096 }
}

/// `RLIMIT_MEMLOCK` of the process
pub fn memlock_limit() -> Result<Limit> {
    get_limit(libc::RLIMIT_MEMLOCK)
}

fn lock_error(call: &'static str, bytes: usize) -> Error {
    let e = os_error("linux.memlock.denied", call).arg("bytes", bytes as i64);
    match memlock_limit() {
        Ok(limit) => e.arg("limit", limit.soft_arg()),
        Err(_) => e,
    }
}

/// Every page of the process locked, now and in the future (`mlockall`), undone on drop (`munlockall`). Only one
/// should exist at a time
#[must_use = "dropping it unlocks the memory at once"]
#[derive(Debug)]
pub struct LockAll(());

/// Locks all the process's memory, present and future, **only when `RLIMIT_MEMLOCK` is unlimited**; otherwise
/// `linux.memlock.limited` with the `limit`, and the caller locks selectively instead
pub fn lock_all() -> Result<LockAll> {
    let limit = memlock_limit()?;
    if limit.soft.is_some() {
        return Err(Error::new("linux.memlock.limited").arg("limit", limit.soft_arg()));
    }
    // SAFETY: plain system call
    if unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) } != 0 {
        return Err(lock_error("mlockall", 0));
    }
    Ok(LockAll(()))
}

impl Drop for LockAll {
    fn drop(&mut self) {
        // SAFETY: plain system call
        unsafe { libc::munlockall() };
    }
}

mod sealed {
    pub trait Sealed {}
}

/// The element types a [`LockedBuffer`] may hold: plain numbers, for which all-zero bytes are a valid value
pub trait Zeroable: Copy + sealed::Sealed + 'static {}

macro_rules! zeroable {
    ($($t:ty),*) => {$(
        impl sealed::Sealed for $t {}
        impl Zeroable for $t {}
    )*};
}
zeroable!(f32, f64, i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);

/// A buffer in pages of its own, locked in RAM while it lives (`mmap` + `mlock`; unmapped, so unlocked, on drop).
/// Its pages are never shared with other data, so unlocking one buffer never unlocks another's memory.
/// `Send + Sync` like a `Box<[T]>`. Making and dropping it is not real-time safe; reading and writing it is
#[must_use]
pub struct LockedBuffer<T: Zeroable> {
    ptr: NonNull<T>,
    len: usize,
    mapped: usize,
    _owns: PhantomData<T>,
}

// SAFETY: it owns its memory exclusively, like Box<[T]>, and T is a plain number
unsafe impl<T: Zeroable + Send> Send for LockedBuffer<T> {}
// SAFETY: shared access is read-only through &self
unsafe impl<T: Zeroable + Sync> Sync for LockedBuffer<T> {}

impl<T: Zeroable> fmt::Debug for LockedBuffer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockedBuffer").field("len", &self.len).field("bytes_locked", &self.mapped).finish()
    }
}

impl<T: Zeroable> LockedBuffer<T> {
    /// `len` zeroed elements, locked. `linux.memlock.denied` (with `bytes` and `limit`) when the limit does not allow
    /// it
    pub fn zeroed(len: usize) -> Result<Self> {
        let bytes = len
            .checked_mul(size_of::<T>())
            .ok_or_else(|| Error::new("linux.invalid_argument").arg("len", len as i64))?;
        if bytes == 0 {
            return Ok(Self { ptr: NonNull::dangling(), len, mapped: 0, _owns: PhantomData });
        }
        let page = page_size();
        let mapped = bytes.div_ceil(page) * page;
        // SAFETY: an anonymous private mapping; the result is checked
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                mapped,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if p == libc::MAP_FAILED {
            return Err(os_error("linux.out_of_memory", "mmap").arg("bytes", mapped as i64));
        }
        // SAFETY: the mapping just made; mlock faults every page in (zero-filled by the kernel)
        if unsafe { libc::mlock(p, mapped) } != 0 {
            let e = lock_error("mlock", mapped);
            // SAFETY: our mapping, unmapped once
            unsafe { libc::munmap(p, mapped) };
            return Err(e);
        }
        let ptr = NonNull::new(p.cast::<T>()).ok_or_else(|| Error::new("linux.out_of_memory"))?;
        Ok(Self { ptr, len, mapped, _owns: PhantomData })
    }

    /// Bytes locked (whole pages)
    pub fn bytes_locked(&self) -> usize {
        self.mapped
    }
}

impl<T: Zeroable> Deref for LockedBuffer<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: `len` initialised (zeroed) elements in our mapping, page-aligned so aligned for T
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl<T: Zeroable> DerefMut for LockedBuffer<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: as deref, with exclusive access through &mut self
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl<T: Zeroable> Drop for LockedBuffer<T> {
    fn drop(&mut self) {
        if self.mapped > 0 {
            // SAFETY: our mapping, unmapped once (which unlocks it)
            unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.mapped) };
        }
    }
}

/// Touches `bytes` of the calling thread's stack below the current frame, so those pages are mapped before the
/// real-time loop needs them. Keep `bytes` well inside the thread's stack size
pub fn prefault_stack(bytes: usize) {
    #[inline(never)]
    fn touch(pages: usize) {
        let mut page = [0u8; 4096];
        std::hint::black_box(&mut page);
        if pages > 1 {
            touch(pages - 1);
        }
        std::hint::black_box(&page);
    }
    touch(bytes.div_ceil(4096));
}

/// The calling thread's stack bounds (lowest address, size)
fn stack_bounds() -> Result<(usize, usize)> {
    // SAFETY: an all-zero attr is a valid out value for pthread_getattr_np, which initialises it; destroyed below
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return Err(Error::new("linux.stack_unknown"));
        }
        let mut addr = std::ptr::null_mut();
        let mut size = 0;
        let r = libc::pthread_attr_getstack(&attr, &mut addr, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        if r != 0 {
            return Err(Error::new("linux.stack_unknown"));
        }
        Ok((addr as usize, size))
    }
}

/// `bytes` of the calling thread's stack, from the current frame down, locked; unlocked on drop, on the same thread
/// (not `Send`)
#[must_use = "dropping it unlocks the stack at once"]
pub struct StackLock {
    start: usize,
    len: usize,
    _not_send: PhantomData<*const ()>,
}

impl fmt::Debug for StackLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StackLock").field("bytes", &self.len).finish()
    }
}

impl StackLock {
    pub fn bytes(&self) -> usize {
        self.len
    }
}

/// Pre-touches and locks `bytes` of the calling thread's stack below the current frame (clamped to the stack, one
/// page of margin above its guard). `linux.memlock.denied` when the limit does not allow it
pub fn lock_current_stack(bytes: usize) -> Result<StackLock> {
    prefault_stack(bytes);
    let page = page_size();
    let marker = 0u8;
    let sp = std::hint::black_box(&raw const marker) as usize;
    let (low, size) = stack_bounds()?;
    let top = (sp / page + 1) * page;
    let start = (sp.saturating_sub(bytes) / page * page).max(low + page);
    if start >= top || top > low + size + page {
        return Err(Error::new("linux.stack_unknown"));
    }
    let len = top - start;
    // SAFETY: a page-aligned range inside this thread's stack mapping, just touched
    if unsafe { libc::mlock(start as *const libc::c_void, len) } != 0 {
        return Err(lock_error("mlock", len));
    }
    Ok(StackLock { start, len, _not_send: PhantomData })
}

impl Drop for StackLock {
    fn drop(&mut self) {
        // SAFETY: the range locked above, on this same thread's stack
        unsafe { libc::munlock(self.start as *const libc::c_void, self.len) };
    }
}
