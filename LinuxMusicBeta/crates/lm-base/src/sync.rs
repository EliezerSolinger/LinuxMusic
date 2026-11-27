//! The synchronisation types the crate uses, swapped for loom's under `--cfg loom` so the concurrency of the
//! operations can be model-checked (interfaces.md, Parte 4, step 1)

#[cfg(loom)]
pub(crate) use loom::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

#[cfg(not(loom))]
pub(crate) use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
