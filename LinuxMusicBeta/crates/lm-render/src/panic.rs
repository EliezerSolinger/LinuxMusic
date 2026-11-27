//! Panics on the render threads (interfaces.md, I-9.3, S11, D10).
//!
//! Every `process` runs inside `catch_unwind`; the node that panicked is marked dead (silence, never called again)
//! until an install removes or replaces it. The panic's payload is not dropped on the audio thread: it goes to the
//! main thread in a [`crate::PanicReport`], which prints it.
//!
//! [`install_panic_hook`] replaces the process's panic hook once (the session calls it at start). On a render thread
//! the hook does no I/O and takes no lock: inside a catch site ([`CatchSite`], which reports the payload itself)
//! it does nothing; outside one, the panic is going to unwind through `render` (to the device's own catch) and
//! nobody would say why, so the hook writes the message into a fixed log of atomics ([`take_render_panics`] reads
//! it on the main thread, and the session tells the user). On any other thread the hook calls the previous one.
//! The panic machinery itself allocates the payload: the one exception to rule 5, and only after a bug

use std::cell::Cell;
use std::fmt::Write as _;
use std::sync::Once;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

thread_local! {
    static RENDER_THREAD: Cell<bool> = const { Cell::new(false) };
    static CATCH_DEPTH: Cell<u32> = const { Cell::new(0) };
    static HOOK_LOGGED: Cell<bool> = const { Cell::new(false) };
}

/// Marks the code under a `catch_unwind` whose `Err` payload goes to a [`crate::PanicReport`]: the hook leaves
/// panics there alone (the report carries the message)
pub(crate) struct CatchSite;

impl CatchSite {
    pub(crate) fn enter() -> Self {
        CATCH_DEPTH.with(|c| c.set(c.get() + 1));
        Self
    }
}

impl Drop for CatchSite {
    fn drop(&mut self) {
        CATCH_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

/// `catch_unwind` for the render path's catch sites: the hook knows the payload will be reported
pub(crate) fn catching<F: FnOnce() -> R + std::panic::UnwindSafe, R>(f: F) -> std::thread::Result<R> {
    let _site = CatchSite::enter();
    std::panic::catch_unwind(f)
}

const LOG_SLOTS: usize = 8;
const MESSAGE_BYTES: usize = 320;
const WORDS: usize = MESSAGE_BYTES / 8;

/// One message of the log: `state` is 0 free, 1 being written, 2 ready
struct LogSlot {
    state: AtomicU32,
    len: AtomicU32,
    words: [AtomicU64; WORDS],
}

#[allow(clippy::declare_interior_mutable_const)]
const FREE_SLOT: LogSlot =
    LogSlot { state: AtomicU32::new(0), len: AtomicU32::new(0), words: [const { AtomicU64::new(0) }; WORDS] };
static LOG: [LogSlot; LOG_SLOTS] = [FREE_SLOT; LOG_SLOTS];
static LOST: AtomicU32 = AtomicU32::new(0);

/// A stack buffer that takes what fits (cut at a character boundary)
struct Stack {
    bytes: [u8; MESSAGE_BYTES],
    len: usize,
}

impl std::fmt::Write for Stack {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let room = MESSAGE_BYTES - self.len;
        let mut take = s.len().min(room);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.bytes[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

/// Puts `message` in the log without allocating or locking. A full log counts the loss instead
fn log_message(message: &str) {
    let mut stack = Stack { bytes: [0; MESSAGE_BYTES], len: 0 };
    let _ = stack.write_str(message);
    log_stack(&stack);
}

fn log_stack(m: &Stack) {
    for slot in &LOG {
        if slot.state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            for (w, chunk) in slot.words.iter().zip(m.bytes.chunks_exact(8)) {
                let mut b = [0u8; 8];
                b.copy_from_slice(chunk);
                w.store(u64::from_le_bytes(b), Ordering::Relaxed);
            }
            slot.len.store(m.len as u32, Ordering::Relaxed);
            slot.state.store(2, Ordering::Release);
            return;
        }
    }
    LOST.fetch_add(1, Ordering::Relaxed);
}

/// Hands over, on the main thread, the messages of the panics that unwound through `render` outside a catch site
/// since the last call (oldest slot first is not promised), and returns how many were lost to a full log
pub fn take_render_panics(mut each: impl FnMut(&str)) -> u32 {
    for slot in &LOG {
        if slot.state.load(Ordering::Acquire) == 2 {
            let len = (slot.len.load(Ordering::Relaxed) as usize).min(MESSAGE_BYTES);
            let mut bytes = [0u8; MESSAGE_BYTES];
            for (w, chunk) in slot.words.iter().zip(bytes.chunks_exact_mut(8)) {
                chunk.copy_from_slice(&w.load(Ordering::Relaxed).to_le_bytes());
            }
            slot.state.store(0, Ordering::Release);
            each(&String::from_utf8_lossy(&bytes[..len]));
        }
    }
    LOST.swap(0, Ordering::Relaxed)
}

/// A device's `catch_unwind` caught a payload (the device thread is leaving, so it may allocate): if the hook did
/// not log that panic (it happened outside `render`), the message goes to the same log
pub fn note_caught_panic(payload: &(dyn std::any::Any + Send)) {
    if HOOK_LOGGED.with(|c| c.replace(false)) {
        return;
    }
    let message = if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "(non-string panic payload)"
    };
    log_message(message);
}

/// Marks the calling thread as a render thread for the hook while the guard lives (the renderer holds one for each
/// render call; a worker for its whole life)
pub(crate) struct RenderThreadGuard {
    previous: bool,
}

impl RenderThreadGuard {
    pub(crate) fn enter() -> Self {
        Self { previous: RENDER_THREAD.with(|c| c.replace(true)) }
    }
}

impl Drop for RenderThreadGuard {
    fn drop(&mut self) {
        RENDER_THREAD.with(|c| c.set(self.previous));
    }
}

/// Whether the calling thread renders
pub fn is_render_thread() -> bool {
    RENDER_THREAD.with(Cell::get)
}

/// Installs the hook (idempotent)
pub fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !is_render_thread() {
                previous(info);
            } else if CATCH_DEPTH.with(Cell::get) == 0 {
                let mut stack = Stack { bytes: [0; MESSAGE_BYTES], len: 0 };
                let _ = write!(stack, "{info}");
                log_stack(&stack);
                HOOK_LOGGED.with(|c| c.set(true));
            }
        }));
    });
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    /// R-4: a panic on a render thread outside a catch site leaves its message in the log (no allocation to write
    /// it); inside a catch site the payload's report is the only record
    #[test]
    fn a_panic_outside_a_catch_site_is_logged_and_one_inside_is_not() {
        install_panic_hook();
        let _render = RenderThreadGuard::enter();
        let seen = |mut v: Vec<String>| {
            take_render_panics(|m| v.push(m.to_owned()));
            v
        };
        assert!(seen(Vec::new()).is_empty());
        let r = catching(|| panic!("inside {}", 1));
        assert!(r.is_err());
        assert!(seen(Vec::new()).is_empty(), "the catch site reports the payload itself");
        let r = std::panic::catch_unwind(|| panic!("outside {}", 2));
        assert!(r.is_err());
        let v = seen(Vec::new());
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("outside 2") && v[0].contains("panic.rs"), "{v:?}");
        // The device's own catch finds the hook already logged it
        note_caught_panic(&*r.unwrap_err());
        assert!(seen(Vec::new()).is_empty());
        // A long message is cut, not dropped
        log_message(&"é".repeat(500));
        let v = seen(Vec::new());
        assert_eq!(v.len(), 1);
        assert!(v[0].len() <= MESSAGE_BYTES && v[0].chars().all(|c| c == 'é'));
    }
}
