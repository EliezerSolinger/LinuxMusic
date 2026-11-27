//! The plugin as the helper's two threads share it, without the audio thread ever waiting.
//!
//! A plugin instance is not safe to call from two threads at once (`lm-sys-clap`'s and `lm-sys-vst3`'s instances take
//! `&mut self`), and the formats forbid some main-thread calls (state, restart, window) from running with `process`.
//! So the instance has one holder at a time, and the two threads are not equals:
//!
//! - the audio thread only ever *tries* ([`Shared::audio`]): one atomic compare-and-swap, never a wait, never a
//!   system call. When the main thread holds the plugin the block is answered with silence and the loop goes on;
//! - the main thread waits for the plugin ([`Shared::main`], at most one block's time when the audio thread has it:
//!   it polls, so the audio thread's release never has a waiter to wake) and holds it for its call. A call that
//!   may take long (state, window, a restart) is therefore a stop of processing, bounded by the core's own timeout
//!   for that request; the core's blocks answer silence meanwhile and no helper is declared hung for it.
//!
//! Every call that CLAP and VST3 put on the main thread stays on the helper's main thread, and `process` on the
//! audio thread: the thread that calls is the one that holds the plugin.

use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use lm_base::{Error, Result};

use crate::backend::Plugin;

/// How long the main thread sleeps between tries
const POLL: Duration = Duration::from_micros(100);

/// A plugin and its single holder
pub struct Shared {
    plugin: Mutex<Box<dyn Plugin>>,
}

impl Shared {
    pub fn new(plugin: Box<dyn Plugin>) -> Shared {
        Shared { plugin: Mutex::new(plugin) }
    }

    /// The audio thread's turn: the plugin if nobody holds it, otherwise `None` at once. Never waits
    pub fn audio(&self) -> Option<MutexGuard<'_, Box<dyn Plugin>>> {
        match self.plugin.try_lock() {
            Ok(g) => Some(g),
            // A plugin that panicked in an earlier call is still the plugin (a panic is the helper's to survive)
            Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// The main thread's turn, waiting up to `wait` for the audio thread's block to end; `None` when it did not
    pub fn main_for(&self, wait: Duration) -> Option<MutexGuard<'_, Box<dyn Plugin>>> {
        let until = Instant::now() + wait;
        loop {
            match self.plugin.try_lock() {
                Ok(g) => return Some(g),
                Err(TryLockError::Poisoned(p)) => return Some(p.into_inner()),
                Err(TryLockError::WouldBlock) => {}
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(POLL);
        }
    }

    /// The main thread's turn for a request that must be answered: `plugin_host.busy` when the plugin stayed in the
    /// audio thread's hands for `wait` (a block that never ends: the core's timeout will kill the helper)
    pub fn main(&self, wait: Duration) -> Result<MutexGuard<'_, Box<dyn Plugin>>> {
        self.main_for(wait).ok_or_else(|| Error::new("plugin_host.busy"))
    }
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_plugins::Test;

    fn shared() -> Shared {
        Shared::new(Box::new(Test::open("synth", 64).unwrap()))
    }

    #[test]
    fn the_audio_thread_never_waits_for_the_main_thread() {
        let s = shared();
        let held = s.main(Duration::from_secs(1)).unwrap();
        let t = Instant::now();
        assert!(s.audio().is_none(), "answered at once, without the plugin");
        assert!(t.elapsed() < Duration::from_millis(50));
        drop(held);
        assert!(s.audio().is_some());
    }

    #[test]
    fn the_main_thread_gets_the_plugin_when_a_block_ends() {
        let s = shared();
        std::thread::scope(|scope| {
            let audio = s.audio().unwrap();
            let waiter = scope.spawn(|| s.main(Duration::from_secs(5)).is_ok());
            std::thread::sleep(Duration::from_millis(30));
            drop(audio);
            assert!(waiter.join().unwrap());
        });
        // A plugin that never comes free is `busy`, not a hang
        let audio = s.audio().unwrap();
        assert!(s.main(Duration::from_millis(20)).is_err());
        assert!(s.main_for(Duration::from_millis(5)).is_none());
        drop(audio);
    }
}
