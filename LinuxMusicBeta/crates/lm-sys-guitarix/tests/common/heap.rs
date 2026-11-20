//! A trap for the C heap, beside `assert_no_alloc` (which sees only Rust's allocator): the C libraries allocate with
//! `malloc` directly.
//!
//! - **Under a sanitizer** (ASan, LSan, TSan replace the allocator): the sanitizer allocator's own count of the
//!   bytes in use (`__sanitizer_get_current_allocated_bytes`); a change means something was kept allocated.
//! - **macOS:** every `malloc`, `calloc`, `realloc` and `free` of every zone goes through `malloc_logger` when it is
//!   set (the hook of the system's MallocStackLogging). [`measure`] counts the calls made while armed: an allocation
//!   that is freed again inside the measured code is still caught.
//! - **Linux (glibc):** `mallinfo2` gives the bytes in use; a change means something was kept allocated. An
//!   allocation freed inside the measured code is not seen here; the macOS run catches it (same library code).
//!
//! Only meaningful with one thread running: these tests are `harness = false` and measure on the main thread.

#![allow(dead_code)]

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[cfg(target_os = "linux")]
const RTLD_DEFAULT: *mut c_void = std::ptr::null_mut();
#[cfg(target_os = "macos")]
const RTLD_DEFAULT: *mut c_void = -2_isize as *mut c_void;

type InUse = unsafe extern "C" fn() -> usize;

/// The sanitizer allocator's count of the bytes in use, when the binary runs under one
fn sanitizer_in_use() -> Option<InUse> {
    static FOUND: OnceLock<Option<usize>> = OnceLock::new();
    let address = *FOUND.get_or_init(|| {
        // SAFETY: dlsym with RTLD_DEFAULT and a NUL-terminated name only looks the symbol up
        let p = unsafe { dlsym(RTLD_DEFAULT, c"__sanitizer_get_current_allocated_bytes".as_ptr()) };
        (!p.is_null()).then_some(p as usize)
    });
    // SAFETY: the address is the sanitizer runtime's `size_t __sanitizer_get_current_allocated_bytes(void)`
    address.map(|a| unsafe { std::mem::transmute::<usize, InUse>(a) })
}

fn measure_in_use(read: impl Fn() -> u64, f: impl FnOnce()) -> u64 {
    let before = read();
    f();
    read().abs_diff(before)
}

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
        super::measure_in_use(in_use, f)
    }
}

/// Prepares the trap (once, at the start of main)
pub fn install() {
    if sanitizer_in_use().is_none() {
        imp::install();
    }
}

/// Runs `f` and returns how many C-heap events it caused (0 = it neither allocated nor freed)
pub fn measure(f: impl FnOnce()) -> u64 {
    match sanitizer_in_use() {
        // SAFETY: the sanitizer runtime's function, found by its name and signature
        Some(read) => measure_in_use(|| unsafe { read() } as u64, f),
        None => imp::measure(f),
    }
}

/// Whether the trap works here: an allocation kept across the measured code must be seen. It is freed afterwards (a
/// leak sanitizer would report it otherwise). Under a sanitizer whose allocator this cannot read, the trap is off and
/// the caller says so; anywhere else a trap that sees nothing is a failure
pub fn self_test() -> bool {
    let mut kept: Option<Vec<u8>> = None;
    let seen = measure(|| kept = Some(std::hint::black_box(vec![1_u8; 1 << 16])));
    drop(kept);
    if seen > 0 {
        return true;
    }
    match std::env::var("LM_TEST_SANITIZER") {
        Ok(sanitizer) => {
            #[allow(clippy::print_stderr)]
            {
                eprintln!(
                    "the C heap trap cannot read the {sanitizer} sanitizer's allocator: only assert_no_alloc checks"
                );
            }
            false
        }
        Err(_) => panic!("the heap trap does not see a malloc on this system"),
    }
}
