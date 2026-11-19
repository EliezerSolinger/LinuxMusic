//! Panics on the render threads (interfaces.md, I-9.3, S11, D10).
//!
//! Every `process` runs inside `catch_unwind`; the node that panicked is marked dead (silence, never called again)
//! until an install removes or replaces it. The panic's payload is not dropped on the audio thread: it goes to the
//! main thread in a [`crate::PanicReport`], which prints it.
//!
//! [`install_panic_hook`] replaces the process's panic hook once (the session calls it at start): on a render thread
//! it does nothing at all (no I/O, no lock: the catch site records the node); on any other thread it calls the
//! previous hook. The panic machinery itself allocates the payload: the one exception to rule 5, and only after a bug

use std::cell::Cell;
use std::sync::Once;

thread_local! {
    static RENDER_THREAD: Cell<bool> = const { Cell::new(false) };
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
            }
        }));
    });
}
