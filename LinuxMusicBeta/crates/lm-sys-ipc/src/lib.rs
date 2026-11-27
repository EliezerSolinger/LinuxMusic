//! `lm-sys-ipc`: memory shared between the core and the plugin host's helper processes (interfaces.md, I-9.6), and a
//! wait/wake on its words, so the audio thread hands a block over and gets it back without a lock and with every
//! system call bounded:
//!
//! - [`SharedMemory`]: an anonymous shared memory object the CORE makes and sizes (Linux: `memfd_create`, sealed with
//!   `F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL`; macOS: `shm_open` by a random name that is unlinked at once,
//!   `ftruncate`), mapped as a slice of [`AtomicU32`] words — the only view given out, so the two processes share it
//!   through atomics and no `unsafe` is needed above this crate. The helper never opens anything by name: it receives
//!   the descriptor over an [`FdChannel`] (`SCM_RIGHTS`) and maps it with [`SharedMemory::from_fd`];
//! - [`FdChannel`], [`pass_to_child`]: the socket pair that carries those descriptors, and its end handed to the helper
//!   as descriptor [`HELPER_FD`];
//! - [`exit_when_parent_dies`], [`exit_now`]: the helper ends itself when the core is gone;
//! - [`wake`] and [`wait`]: Linux `futex` (`FUTEX_WAKE`, `FUTEX_WAIT` with a timeout; the shared flavour, not
//!   `_PRIVATE`, since the word lives in two processes), macOS `__ulock_wake`/`__ulock_wait` with
//!   `UL_COMPARE_AND_WAIT_SHARED` (the primitive libc++ and Rust's own std use there).
//!
//! # Safety sheet (ficha, I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | **Who owns each handle?** | [`SharedMemory`] owns its mapping (`munmap` in `Drop`) and, in the core, the object's descriptor until [`SharedMemory::close_fd`]. There is no name to own: the object exists only while a descriptor or a mapping does, so none outlives both processes. [`FdChannel`] owns its socket. |
//! | **Can the helper hurt the core through the object?** | It can write any word (the core reads them as untrusted numbers). It cannot shrink the object, which is what would make the core's next read a `SIGBUS`: on Linux the seals refuse `ftruncate` (and the helper refuses an unsealed object); on macOS a shared memory object's size can be set only once (a second `ftruncate` fails with `EINVAL`), which is the kernel's behaviour rather than a documented guarantee — there are no seals, so this is the residual risk there. The name is unlinked at creation, so no third process can open the object. |
//! | **Which thread may call what?** | Any: the mapping is `Send + Sync`, and every access is an atomic load or store on its words. Creating, opening and dropping happen on non-real-time threads. |
//! | **Allocates, locks, does I/O?** | Creating, mapping and passing allocate and do system calls (never on the audio thread). On the audio thread: reading and writing words (atomics), [`wake`] (one `futex`/`__ulock_wake` system call, which never blocks) and [`wait`] with a timeout (one system call, bounded by the timeout; spurious returns are the caller's loop). No allocation, no lock. |
//! | **Creates threads?** | No. |
//! | **Calls our code back?** | No. |
//! | **Tests** | Two mappings of one object (the second from a passed descriptor) see each other's words; the helper's side refuses a wrong size and, on Linux, an unsealed object and cannot resize a sealed one; descriptors travel with their tag; a wait times out on time; wake and wait under the allocation trap. |
//!
//! # Error keys
//!
//! `ipc.shm.name`, `ipc.shm.create`, `ipc.shm.size`, `ipc.shm.seal`, `ipc.shm.map`, `ipc.fd.socket`, `ipc.fd.send`, `ipc.fd.recv`, `ipc.fd.timeout`.

use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::atomic::AtomicU32;
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
    /// The descriptor of the object: kept until it was handed to the helper (it is what the helper maps)
    fd: Option<OwnedFd>,
}

// SAFETY: the mapping is plain memory accessed only through atomics; moving the owner between threads is sound
unsafe impl Send for SharedMemory {}
// SAFETY: every access is an atomic operation on the words
unsafe impl Sync for SharedMemory {}

fn set_cloexec(fd: RawFd) {
    // SAFETY: fcntl on a descriptor we own; a failure leaves the flag as it was
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }
}

/// Makes the object: an anonymous `memfd` sealed against any later resize (Linux)
#[cfg(target_os = "linux")]
fn make_object(bytes: usize) -> Result<OwnedFd> {
    // SAFETY: memfd_create with a valid C string; the result is checked
    let raw = unsafe {
        libc::syscall(libc::SYS_memfd_create, c"lm-shm".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
    };
    if raw < 0 {
        return Err(os_error("ipc.shm.create"));
    }
    // SAFETY: a fresh descriptor nobody else owns
    let fd = unsafe { OwnedFd::from_raw_fd(raw as RawFd) };
    // SAFETY: ftruncate on the descriptor we own; the size fits off_t (a layout's words are bounded by the caller)
    if unsafe { libc::ftruncate(fd.as_raw_fd(), bytes as libc::off_t) } != 0 {
        return Err(os_error("ipc.shm.size"));
    }
    // The size is final for every holder of the object, the helper included: a shrink would make the core's next
    // read of the mapping a SIGBUS
    let seals = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
    // SAFETY: fcntl on the descriptor we own
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, seals) } != 0 {
        return Err(os_error("ipc.shm.seal"));
    }
    Ok(fd)
}

/// Makes the object: a POSIX shared memory object whose name is removed at once, so no other process can open it
/// and only the descriptors that are passed on reach it (macOS has no seals)
#[cfg(not(target_os = "linux"))]
fn make_object(bytes: usize) -> Result<OwnedFd> {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    // macOS limits a name to 31 bytes
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let name = format!("/lm-{:x}-{:x}-{:x}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed), t);
    let c = std::ffi::CString::new(name).map_err(|_| Error::new("ipc.shm.name"))?;
    // SAFETY: a valid C string; O_EXCL makes a fresh object or fails
    let raw = unsafe { libc::shm_open(c.as_ptr(), libc::O_CREAT | libc::O_EXCL | libc::O_RDWR, 0o600 as libc::c_uint) };
    if raw < 0 {
        return Err(os_error("ipc.shm.create"));
    }
    // SAFETY: a fresh descriptor nobody else owns
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    set_cloexec(raw);
    // The name goes whatever happens next: the object lives while a descriptor or a mapping does
    // SAFETY: removing the name just made
    unsafe { libc::shm_unlink(c.as_ptr()) };
    // SAFETY: ftruncate on the descriptor we own. macOS allows the size of a shared memory object to be set once,
    // so the helper cannot shrink it afterwards
    if unsafe { libc::ftruncate(raw, bytes as libc::off_t) } != 0 {
        return Err(os_error("ipc.shm.size"));
    }
    Ok(fd)
}

/// Whether the object is of the size and (on Linux) sealed as [`make_object`] made it
fn check_object(fd: &OwnedFd, bytes: usize) -> Result<()> {
    // SAFETY: fstat on the descriptor we own, into a zeroed struct
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: as above
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } != 0 || st.st_size as u64 != bytes as u64 {
        return Err(Error::new("ipc.shm.size"));
    }
    #[cfg(target_os = "linux")]
    {
        // An object whose size the other side could still change is refused
        // SAFETY: fcntl on the descriptor we own
        let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
        let need = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW;
        if seals < 0 || seals & need != need {
            return Err(Error::new("ipc.shm.seal"));
        }
    }
    Ok(())
}

impl SharedMemory {
    /// Makes a new anonymous object of `words` words, zeroed, readable and writable by this user only. The core
    /// makes it and sizes it; the helper gets the descriptor ([`FdChannel::send`]) and calls [`SharedMemory::from_fd`]
    pub fn create(words: usize) -> Result<SharedMemory> {
        let bytes = words.checked_mul(4).filter(|b| *b > 0).ok_or_else(|| Error::new("ipc.shm.size"))?;
        let fd = make_object(bytes)?;
        Self::map(fd, words)
    }

    /// Maps an object another process made and handed over (its size must be exactly `words` words; on Linux it
    /// must be sealed against resizing)
    pub fn from_fd(fd: OwnedFd, words: usize) -> Result<SharedMemory> {
        let bytes = words.checked_mul(4).filter(|b| *b > 0).ok_or_else(|| Error::new("ipc.shm.size"))?;
        check_object(&fd, bytes)?;
        let mut m = Self::map(fd, words)?;
        // The helper does not pass the object on
        m.fd = None;
        Ok(m)
    }

    fn map(fd: OwnedFd, words: usize) -> Result<SharedMemory> {
        let bytes = words * 4;
        // SAFETY: mapping `bytes` of an object of exactly that size; the result is checked
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), bytes, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0)
        };
        if p == libc::MAP_FAILED {
            return Err(os_error("ipc.shm.map"));
        }
        Ok(SharedMemory { ptr: p.cast(), words, fd: Some(fd) })
    }

    /// The object's descriptor, to hand to the helper (`None` once [`SharedMemory::close_fd`] was called)
    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.fd.as_ref().map(std::os::fd::AsFd::as_fd)
    }

    /// Closes the core's descriptor (the mapping stays): once it was sent, so a helper started later never inherits
    /// it by a race. Idempotent
    pub fn close_fd(&mut self) {
        self.fd = None;
    }

    /// The words. Page-aligned memory, valid while `self` lives; atomics are the only access
    pub fn words(&self) -> &[AtomicU32] {
        // SAFETY: `ptr` maps `words` 4-byte words, aligned (mmap is page-aligned), alive until `Drop`; AtomicU32 has
        // the size and alignment of u32 and interior mutability, so a shared slice of them is the right view of
        // memory another process writes too
        unsafe { std::slice::from_raw_parts(self.ptr, self.words) }
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        // SAFETY: unmapping exactly what `map` mapped, once
        unsafe { libc::munmap(self.ptr.cast(), self.words * 4) };
    }
}

/// Passes descriptors between the core and a helper over a Unix datagram socket pair (`SCM_RIGHTS`): the core sends
/// the shared block's descriptor, tagged with the instance's number, and the helper receives it. One way (the core
/// sends), so the helper's side only ever reads from it
#[derive(Debug)]
pub struct FdChannel {
    fd: OwnedFd,
}

/// The descriptor number the helper's end of the channel has in the helper
pub const HELPER_FD: RawFd = 3;

impl FdChannel {
    /// A connected pair: the core's end, and the helper's end to give to [`pass_to_child`]
    pub fn pair() -> Result<(FdChannel, OwnedFd)> {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: socketpair into a two-element array; the result is checked
        if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_DGRAM, 0, fds.as_mut_ptr()) } != 0 {
            return Err(os_error("ipc.fd.socket"));
        }
        // SAFETY: two fresh descriptors nobody else owns
        let (mine, theirs) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        // Neither survives an exec by accident (`pass_to_child` gives the helper's end its number)
        set_cloexec(mine.as_raw_fd());
        set_cloexec(theirs.as_raw_fd());
        #[cfg(target_os = "macos")]
        {
            let on: libc::c_int = 1;
            // SAFETY: setsockopt with a c_int on a socket we own: a dead helper must be an error, not a SIGPIPE
            unsafe {
                libc::setsockopt(
                    mine.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_NOSIGPIPE,
                    std::ptr::from_ref(&on).cast(),
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }
        Ok((FdChannel { fd: mine }, theirs))
    }

    /// The helper's end, which the core started it with as descriptor [`HELPER_FD`]. Once per process (a second call
    /// is an error, as the descriptor is owned by the first); an error too when that descriptor is not a datagram
    /// socket (the helper was started by hand)
    pub fn inherited() -> Result<FdChannel> {
        use std::sync::atomic::{AtomicBool, Ordering};
        static TAKEN: AtomicBool = AtomicBool::new(false);
        let mut kind: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: getsockopt into a c_int of the stated length; on a descriptor that is not open, or not a socket, it
        // only fails
        let ok = unsafe {
            libc::getsockopt(HELPER_FD, libc::SOL_SOCKET, libc::SO_TYPE, std::ptr::from_mut(&mut kind).cast(), &mut len)
        } == 0
            && kind == libc::SOCK_DGRAM;
        if !ok || TAKEN.swap(true, Ordering::AcqRel) {
            return Err(Error::new("ipc.fd.socket"));
        }
        // SAFETY: the descriptor is an open socket (just checked) and the flag above makes this the only owner
        Ok(FdChannel { fd: unsafe { OwnedFd::from_raw_fd(HELPER_FD) } })
    }

    /// The helper's end of a pair made in this process (the tests)
    #[cfg(test)]
    fn from_owned(fd: OwnedFd) -> FdChannel {
        FdChannel { fd }
    }

    /// Sends `object` with `tag`. Never blocks (a full socket is an error)
    pub fn send(&self, object: BorrowedFd<'_>, tag: u64) -> Result<()> {
        let mut data = tag.to_le_bytes();
        let mut iov = libc::iovec { iov_base: data.as_mut_ptr().cast(), iov_len: data.len() };
        // Aligned room for one control message carrying one descriptor
        let mut control = [0u64; 8];
        // SAFETY: a zeroed msghdr is valid; the fields are set below
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        // SAFETY: CMSG_SPACE is a pure size computation
        let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
        debug_assert!(space <= std::mem::size_of_val(&control));
        msg.msg_controllen = space as _;
        // SAFETY: `msg_control` points at `control`, large enough for one header; the header and its data are
        // written inside that space
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(c).cast::<RawFd>(), object.as_raw_fd());
        }
        #[cfg(target_os = "linux")]
        let flags = libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT;
        #[cfg(not(target_os = "linux"))]
        let flags = libc::MSG_DONTWAIT;
        // SAFETY: sendmsg with a fully built header over buffers that outlive the call
        let n = unsafe { libc::sendmsg(self.fd.as_raw_fd(), &msg, flags) };
        if n < 0 { Err(os_error("ipc.fd.send")) } else { Ok(()) }
    }

    /// Receives one descriptor and its tag, waiting at most `timeout`. The descriptor is close-on-exec
    pub fn recv(&self, timeout: Duration) -> Result<(OwnedFd, u64)> {
        let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
        // SAFETY: poll on one valid pollfd
        let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
        if ready == 0 {
            return Err(Error::new("ipc.fd.timeout"));
        }
        if ready < 0 {
            return Err(os_error("ipc.fd.recv"));
        }
        let mut data = [0u8; 8];
        let mut iov = libc::iovec { iov_base: data.as_mut_ptr().cast(), iov_len: data.len() };
        let mut control = [0u64; 8];
        // SAFETY: a zeroed msghdr is valid; the fields are set below
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = std::mem::size_of_val(&control) as _;
        #[cfg(target_os = "linux")]
        let flags = libc::MSG_CMSG_CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let flags = 0;
        // SAFETY: recvmsg into buffers that outlive the call
        let n = unsafe { libc::recvmsg(self.fd.as_raw_fd(), &mut msg, flags) };
        if n < 0 {
            return Err(os_error("ipc.fd.recv"));
        }
        // Every descriptor that arrived is owned (and so closed) here, whatever else is wrong with the message
        let mut got: Vec<OwnedFd> = Vec::new();
        // SAFETY: the kernel filled `msg_control` with `msg_controllen` bytes of well-formed control messages; the
        // CMSG macros walk exactly that
        unsafe {
            let mut c = libc::CMSG_FIRSTHDR(&msg);
            while !c.is_null() {
                if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                    let len = (*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                    let base = libc::CMSG_DATA(c).cast::<RawFd>();
                    for i in 0..len / std::mem::size_of::<RawFd>() {
                        let raw = std::ptr::read_unaligned(base.add(i));
                        set_cloexec(raw);
                        got.push(OwnedFd::from_raw_fd(raw));
                    }
                }
                c = libc::CMSG_NXTHDR(&msg, c);
            }
        }
        if n as usize != data.len() || msg.msg_flags & libc::MSG_CTRUNC != 0 || got.len() != 1 {
            return Err(Error::new("ipc.fd.recv"));
        }
        Ok((got.remove(0), u64::from_le_bytes(data)))
    }
}

/// Makes `fd` the descriptor number `target` in the child `command` starts (and no other descriptor of the core
/// reaches it: everything else is close-on-exec). `fd` must stay open until the command has been spawned
pub fn pass_to_child(command: &mut Command, fd: &OwnedFd, target: RawFd) {
    let raw = fd.as_raw_fd();
    // SAFETY: the closure runs between fork and exec and calls only async-signal-safe functions (dup2, fcntl); it
    // allocates nothing and takes no lock
    unsafe {
        command.pre_exec(move || {
            if raw == target {
                // Already the right number: only the close-on-exec flag has to go
                let flags = libc::fcntl(raw, libc::F_GETFD);
                if flags < 0 || libc::fcntl(raw, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::dup2(raw, target) < 0 {
                // dup2 leaves the new descriptor without close-on-exec
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Ends this process at once, without running destructors or `atexit` handlers (a plugin's own can hang): for the
/// helper whose core is gone
pub fn exit_now(code: i32) -> ! {
    // SAFETY: _exit never returns and touches nothing of ours
    unsafe { libc::_exit(code) }
}

/// For the helper: ends the process (see [`exit_now`]) as soon as its parent is gone, by polling the parent's id
/// every 200 ms in a thread of its own. `PR_SET_PDEATHSIG` is not used on purpose: it fires when the parent
/// *thread* that spawned the helper ends, and the core spawns helpers from short-lived operation threads. When the
/// parent process dies the helper is re-parented, so the id changes
pub fn exit_when_parent_dies() {
    // SAFETY: getppid has no preconditions
    let parent = unsafe { libc::getppid() };
    let _ = std::thread::Builder::new().name("lm-parent-watch".into()).spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(200));
            // SAFETY: getppid has no preconditions
            if unsafe { libc::getppid() } != parent {
                exit_now(0);
            }
        }
    });
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

    use std::sync::atomic::Ordering;

    /// A second mapping of the same object, through a duplicate of its descriptor (what the helper gets)
    fn second(a: &SharedMemory, words: usize) -> Result<SharedMemory> {
        let fd = a.fd().unwrap().try_clone_to_owned().unwrap();
        SharedMemory::from_fd(fd, words)
    }

    #[test]
    fn the_helper_refuses_an_object_of_another_size() {
        let a = SharedMemory::create(1024).unwrap();
        assert!(second(&a, 1025).is_err());
        assert!(second(&a, 512).is_err());
        assert!(second(&a, 0).is_err());
        assert!(second(&a, 1024).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_sealed_object_cannot_be_resized_and_an_unsealed_one_is_refused() {
        let a = SharedMemory::create(1024).unwrap();
        let fd = a.fd().unwrap();
        // SAFETY: ftruncate on a descriptor the test owns; it must fail on the seals
        let r = unsafe { libc::ftruncate(fd.as_raw_fd(), 16) };
        assert_eq!(r, -1, "a sealed object cannot shrink");
        // SAFETY: as above, growing
        let r = unsafe { libc::ftruncate(fd.as_raw_fd(), 1 << 20) };
        assert_eq!(r, -1, "nor grow");
        // An object made without seals is refused by the helper's side
        // SAFETY: memfd_create with a valid C string, then ftruncate on the fresh descriptor
        let raw = unsafe { libc::syscall(libc::SYS_memfd_create, c"t".as_ptr(), libc::MFD_CLOEXEC) } as RawFd;
        assert!(raw >= 0);
        // SAFETY: a fresh descriptor nobody else owns
        let loose = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: ftruncate on the descriptor we own
        assert_eq!(unsafe { libc::ftruncate(loose.as_raw_fd(), 4096) }, 0);
        assert!(SharedMemory::from_fd(loose, 1024).is_err());
    }

    #[test]
    fn descriptors_travel_with_their_tag() {
        let (core, helper_end) = FdChannel::pair().unwrap();
        let helper = FdChannel::from_owned(helper_end);
        let a = SharedMemory::create(256).unwrap();
        core.send(a.fd().unwrap(), 7).unwrap();
        let (fd, tag) = helper.recv(Duration::from_secs(2)).unwrap();
        assert_eq!(tag, 7);
        let b = SharedMemory::from_fd(fd, 256).unwrap();
        a.words()[3].store(99, Ordering::Release);
        assert_eq!(b.words()[3].load(Ordering::Acquire), 99);
        // Nothing more is waiting: a receive times out
        assert!(helper.recv(Duration::from_millis(20)).is_err());
    }

    #[test]
    fn two_mappings_share_and_wake() {
        let mut a = SharedMemory::create(1024).unwrap();
        let b = second(&a, 1024).unwrap();
        // The core's descriptor can go: the mappings keep the object
        a.close_fd();
        assert!(a.fd().is_none());
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
        assert!(SharedMemory::create(0).is_err());
    }
}
