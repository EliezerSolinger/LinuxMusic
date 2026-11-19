//! A trap for the C heap, beside `assert_no_alloc` (which sees only Rust's allocator): the C libraries allocate with
//! `malloc` directly.
//!
//! - **macOS:** every `malloc`, `calloc`, `realloc` and `free` of every zone goes through `malloc_logger` when it is
//!   set (the hook of the system's MallocStackLogging). [`count`] counts the calls made while armed: an allocation
//!   that is freed again inside the measured code is still caught.
//! - **Linux (glibc):** `mallinfo2` gives the bytes in use; a change means something was kept allocated. An
//!   allocation freed inside the measured code is not seen here; the macOS run catches it (same library code).
//!
//! Only meaningful with one thread running: these tests are `harness = false` and measure on the main thread.

#![allow(dead_code)]

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    type Logger = unsafe extern "C" fn(u32, usize, usize, usize, usize, u32);

    unsafe extern "C" {
        static mut malloc_logger: Option<Logger>;
    }

    const ALLOCATE: u32 = 2;
    const DEALLOCATE: u32 = 4;

    static ARMED: AtomicBool = AtomicBool::new(false);
    static CALLS: AtomicU64 = AtomicU64::new(0);

    unsafe extern "C" fn logger(kind: u32, _: usize, _: usize, _: usize, _: usize, _: u32) {
        if ARMED.load(Ordering::Relaxed) && kind & (ALLOCATE | DEALLOCATE) != 0 {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn install() {
        // SAFETY: the hook is written once, on the main thread, before any measured code, and the logger only touches
        // atomics (it never allocates, which would recurse)
        unsafe { *std::ptr::addr_of_mut!(malloc_logger) = Some(logger) };
    }

    pub fn measure(f: impl FnOnce()) -> u64 {
        let before = CALLS.load(Ordering::SeqCst);
        ARMED.store(true, Ordering::SeqCst);
        f();
        ARMED.store(false, Ordering::SeqCst);
        CALLS.load(Ordering::SeqCst) - before
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod imp {
    #[repr(C)]
    struct Mallinfo2 {
        arena: usize,
        ordblks: usize,
        smblks: usize,
        hblks: usize,
        hblkhd: usize,
        usmblks: usize,
        fsmblks: usize,
        uordblks: usize,
        fordblks: usize,
        keepcost: usize,
    }

    unsafe extern "C" {
        safe fn mallinfo2() -> Mallinfo2;
    }

    fn in_use() -> u64 {
        let info = mallinfo2();
        (info.uordblks + info.hblkhd) as u64
    }

    pub fn install() {}

    pub fn measure(f: impl FnOnce()) -> u64 {
        let before = in_use();
        f();
        in_use().abs_diff(before)
    }
}

/// Prepares the trap (once, at the start of main)
pub fn install() {
    imp::install();
}

/// Runs `f` and returns how many C-heap events it caused (0 = it neither allocated nor freed)
pub fn measure(f: impl FnOnce()) -> u64 {
    imp::measure(f)
}

/// Checks the trap works on this system: a real allocation must be seen
pub fn self_test() {
    let seen = measure(|| {
        let v = std::hint::black_box(vec![0_u8; 1 << 16]);
        std::mem::forget(v);
    });
    assert!(seen > 0, "the heap trap does not see a malloc on this system");
}
