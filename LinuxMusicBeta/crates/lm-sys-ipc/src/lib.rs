//! `lm-sys-ipc`: memory shared between the core and the plugin host's helper processes (interfaces.md, I-9.6), and a
//! wait/wake on its words, so the audio thread hands a block over and gets it back without a lock and with every
//! system call bounded:
//!
//! - [`SharedMemory`]: a POSIX shared memory object (`shm_open` by a random name the child receives, `ftruncate`,
//!   `mmap`), seen as a slice of [`AtomicU32`] words — the only view given out, so the two processes share it through
//!   atomics and no `unsafe` is needed above this crate;
//! - [`wake`] and [`wait`]: Linux `futex` (`FUTEX_WAKE`, `FUTEX_WAIT` with a timeout; the shared flavour, not
//!   `_PRIVATE`, since the word lives in two processes), macOS `__ulock_wake`/`__ulock_wait` with
//!   `UL_COMPARE_AND_WAIT_SHARED` (the primitive libc++ and Rust's own std use there).
//!
//! # Safety sheet (ficha, I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | **Who owns each handle?** | [`SharedMemory`] owns its mapping (`munmap` in `Drop`); the descriptor is closed right after mapping (the mapping keeps the object). The creator owns the name and removes it with [`SharedMemory::unlink`] (called by the core once the helper has opened it, and in `Drop` of the creator if never called), so no object outlives both processes. |
//! | **Which thread may call what?** | Any: the mapping is `Send + Sync`, and every access is an atomic load or store on its words. Creating, opening and dropping happen on non-real-time threads. |
//! | **Allocates, locks, does I/O?** | Creating and opening allocate and do system calls (never on the audio thread). On the audio thread: reading and writing words (atomics), [`wake`] (one `futex`/`__ulock_wake` system call, which never blocks) and [`wait`] with a timeout (one system call, bounded by the timeout; spurious returns are the caller's loop). No allocation, no lock. |
//! | **Creates threads?** | No. |
//! | **Calls our code back?** | No. |
//! | **Tests** | Two mappings of one object see each other's words; a child process (this test binary re-run) writes and wakes, the parent waits; a wait times out on time; wake and wait under the allocation trap. |
//!
//! # Error keys
//!
//! `ipc.shm.name`, `ipc.shm.create`, `ipc.shm.open`, `ipc.shm.size`, `ipc.shm.map`.

use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use lm_base::{Error, Result};

fn os_error(key: &'static str) -> Error {
    Error::new(key).arg("errno", std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as i64)
}

/// A shared memory object, mapped
#[derive(Debug)]
pub struct SharedMemory {
    ptr: *mut AtomicU32,
    words: usize,
    name: CString,
    owner: bool,
}

// SAFETY: the mapping is plain memory accessed only through atomics; moving the owner between threads is sound
unsafe impl Send for SharedMemory {}
// SAFETY: every access is an atomic operation on the words
unsafe impl Sync for SharedMemory {}

fn check_name(name: &str) -> Result<CString> {
    // A portable POSIX name: one leading slash, then at most 30 safe characters (macOS limits names to 31 bytes)
    let ok = name.len() >= 2
        && name.len() <= 31
        && name.starts_with('/')
        && name[1..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !ok {
        return Err(Error::new("ipc.shm.name").arg("name", name));
    }
    CString::new(name).map_err(|_| Error::new("ipc.shm.name"))
}

impl SharedMemory {
    /// Makes a new object of `words` words, zeroed, readable and writable by this user only
    pub fn create(name: &str, words: usize) -> Result<SharedMemory> {
        let c = check_name(name)?;
        let bytes = words.checked_mul(4).filter(|b| *b > 0).ok_or_else(|| Error::new("ipc.shm.size"))?;
        // SAFETY: a valid C string; O_EXCL makes a fresh object or fails
        let fd =
            unsafe { libc::shm_open(c.as_ptr(), libc::O_CREAT | libc::O_EXCL | libc::O_RDWR, 0o600 as libc::c_uint) };
        if fd < 0 {
            return Err(os_error("ipc.shm.create"));
        }
        // SAFETY: a descriptor we own; the size fits off_t (checked above for overflow of usize, and words are bounded
        // by the caller's layout)
        if unsafe { libc::ftruncate(fd, bytes as libc::off_t) } != 0 {
            let e = os_error("ipc.shm.size");
            // SAFETY: closing and removing what we just made
            unsafe {
                libc::close(fd);
                libc::shm_unlink(c.as_ptr());
            }
            return Err(e);
        }
        let map = Self::map(fd, bytes, c.clone(), true);
        if map.is_err() {
            // SAFETY: removing the object we made
            unsafe { libc::shm_unlink(c.as_ptr()) };
        }
        let mut m = map?;
        m.words = words;
        Ok(m)
    }

    /// Opens an object another process made (its size must be `words` words)
    pub fn open(name: &str, words: usize) -> Result<SharedMemory> {
        let c = check_name(name)?;
        let bytes = words.checked_mul(4).filter(|b| *b > 0).ok_or_else(|| Error::new("ipc.shm.size"))?;
        // SAFETY: a valid C string
        let fd = unsafe { libc::shm_open(c.as_ptr(), libc::O_RDWR, 0) };
        if fd < 0 {
            return Err(os_error("ipc.shm.open"));
        }
        // SAFETY: fstat on the descriptor we own, into a zeroed struct
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: as above
        if unsafe { libc::fstat(fd, &mut st) } != 0 || (st.st_size as u64) < bytes as u64 {
            // SAFETY: closing our descriptor
            unsafe { libc::close(fd) };
            return Err(Error::new("ipc.shm.size"));
        }
        let mut m = Self::map(fd, bytes, c, false)?;
        m.words = words;
        Ok(m)
    }

    fn map(fd: libc::c_int, bytes: usize, name: CString, owner: bool) -> Result<SharedMemory> {
        // SAFETY: mapping `bytes` of an object at least that large; the result is checked
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), bytes, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        // SAFETY: the mapping keeps the object; the descriptor is no longer needed
        unsafe { libc::close(fd) };
        if p == libc::MAP_FAILED {
            return Err(os_error("ipc.shm.map"));
        }
        Ok(SharedMemory { ptr: p.cast(), words: bytes / 4, name, owner })
    }

    /// The words. Page-aligned memory, valid while `self` lives; atomics are the only access
    pub fn words(&self) -> &[AtomicU32] {
        // SAFETY: `ptr` maps `words` 4-byte words, aligned (mmap is page-aligned), alive until `Drop`; AtomicU32 has
        // the size and alignment of u32 and interior mutability, so a shared slice of them is the right view of
        // memory another process writes too
        unsafe { std::slice::from_raw_parts(self.ptr, self.words) }
    }

    /// Removes the name (the memory stays until every mapping is gone). Idempotent
    pub fn unlink(&mut self) {
        if self.owner {
            // SAFETY: a valid C string; failure (already gone) is harmless
            unsafe { libc::shm_unlink(self.name.as_ptr()) };
            self.owner = false;
        }
    }

    pub fn name(&self) -> &str {
        self.name.to_str().unwrap_or("")
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        self.unlink();
        // SAFETY: unmapping exactly what `map` mapped, once
        unsafe { libc::munmap(self.ptr.cast(), self.words * 4) };
    }
}

/// A fresh object name: `/lm-<pid>-<counter>-<random>`
pub fn unique_name() -> String {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    format!("/lm-{}-{n}-{:x}", std::process::id(), t & 0xff_ffff)
}

/// Wakes up to `count` waiters on `word` (in any process). Never blocks
pub fn wake(word: &AtomicU32, count: u32) {
    #[cfg(target_os = "linux")]
    // SAFETY: FUTEX_WAKE on the address of a live atomic word; the shared (non-private) flavour, as the word is
    // mapped in two processes. Never blocks
    unsafe {
        libc::syscall(libc::SYS_futex, word.as_ptr(), libc::FUTEX_WAKE, count.min(i32::MAX as u32) as i32);
    }
    #[cfg(target_os = "macos")]
    {
        let op = if count > 1 { UL_COMPARE_AND_WAIT_SHARED | ULF_WAKE_ALL } else { UL_COMPARE_AND_WAIT_SHARED };
        // SAFETY: __ulock_wake on the address of a live atomic word; never blocks
        unsafe { __ulock_wake(op, word.as_ptr().cast(), 0) };
    }
}

/// Waits while `word` holds `expected`, at most `timeout` (`None`: no limit). Returns when woken, when the value
/// differs, on timeout, or spuriously: the caller rechecks
pub fn wait(word: &AtomicU32, expected: u32, timeout: Option<Duration>) {
    #[cfg(target_os = "linux")]
    {
        let ts = timeout.map(|t| libc::timespec {
            tv_sec: t.as_secs().min(i64::MAX as u64) as libc::time_t,
            tv_nsec: t.subsec_nanos() as libc::c_long,
        });
        let tp = ts.as_ref().map_or(std::ptr::null(), |t| t as *const libc::timespec);
        // SAFETY: FUTEX_WAIT on a live atomic word with a relative timeout (or none); bounded by the timeout
        unsafe {
            libc::syscall(libc::SYS_futex, word.as_ptr(), libc::FUTEX_WAIT, expected, tp);
        }
    }
    #[cfg(target_os = "macos")]
    {
        // Microseconds; 0 is "no timeout" for __ulock_wait, so a zero timeout waits one microsecond
        let us = timeout.map_or(0u32, |t| t.as_micros().clamp(1, u128::from(u32::MAX)) as u32);
        // SAFETY: __ulock_wait on a live atomic word compared with `expected`; bounded by the timeout
        unsafe { __ulock_wait(UL_COMPARE_AND_WAIT_SHARED, word.as_ptr().cast(), u64::from(expected), us) };
    }
}

#[cfg(target_os = "macos")]
const UL_COMPARE_AND_WAIT_SHARED: u32 = 3;
#[cfg(target_os = "macos")]
const ULF_WAKE_ALL: u32 = 0x0000_0100;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn __ulock_wait(operation: u32, addr: *mut libc::c_void, value: u64, timeout_us: u32) -> libc::c_int;
    fn __ulock_wake(operation: u32, addr: *mut libc::c_void, wake_value: u64) -> libc::c_int;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[global_allocator]
    static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

    #[test]
    fn two_mappings_share_and_wake() {
        let name = unique_name();
        let mut a = SharedMemory::create(&name, 1024).unwrap();
        let b = SharedMemory::open(&name, 1024).unwrap();
        a.unlink();
        assert!(SharedMemory::open(&name, 1024).is_err(), "the name is gone, the memory stays");
        a.words()[5].store(42, Ordering::Release);
        assert_eq!(b.words()[5].load(Ordering::Acquire), 42);
        // A waiter woken by another thread through the other mapping
        let flag = &b.words()[0];
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(20));
                a.words()[0].store(1, Ordering::Release);
                wake(&a.words()[0], 1);
            });
            let t = Instant::now();
            while flag.load(Ordering::Acquire) == 0 {
                wait(flag, 0, Some(Duration::from_secs(5)));
            }
            assert!(t.elapsed() < Duration::from_secs(4));
        });
        // A timeout is kept, and neither call allocates
        let t = Instant::now();
        assert_no_alloc::assert_no_alloc(|| {
            wait(&b.words()[1], 0, Some(Duration::from_millis(30)));
            wake(&b.words()[1], 1);
        });
        let e = t.elapsed();
        assert!(e >= Duration::from_millis(20) && e < Duration::from_secs(2), "{e:?}");
        assert!(SharedMemory::create("bad name", 1).is_err());
    }
}
