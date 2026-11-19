//! The synchronisation types of the two protocols that are model-checked with loom (interfaces.md, I-17): the
//! parameter slots with their dirty bitset (T15) and the install/return handshake (S5, D2). Under `--cfg loom` they
//! are loom's, everywhere else the standard library's

#[cfg(loom)]
pub(crate) use loom::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, AtomicU64, Ordering},
};

#[cfg(not(loom))]
pub(crate) use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicU64, Ordering},
};

/// The single-producer single-consumer ring of the handshake: rtrb in the program, a loom-visible model under loom
/// (the model is a mutex-guarded deque with the same capacity rule; only the model checker ever runs it)
pub(crate) mod spsc {
    #[cfg(not(loom))]
    pub(crate) use lm_base::ring::{Consumer, Producer, ring};

    #[cfg(loom)]
    pub(crate) use model::{Consumer, Producer, ring};

    #[cfg(loom)]
    mod model {
        use super::super::{Arc, Mutex};
        use std::collections::VecDeque;

        #[derive(Debug)]
        pub(crate) struct Producer<T> {
            shared: Arc<Mutex<VecDeque<T>>>,
            capacity: usize,
        }

        #[derive(Debug)]
        pub(crate) struct Consumer<T> {
            shared: Arc<Mutex<VecDeque<T>>>,
        }

        pub(crate) fn ring<T>(capacity: usize) -> (Producer<T>, Consumer<T>) {
            let shared = Arc::new(Mutex::new(VecDeque::with_capacity(capacity)));
            (Producer { shared: shared.clone(), capacity: capacity.max(1) }, Consumer { shared })
        }

        impl<T> Producer<T> {
            pub(crate) fn push(&mut self, value: T) -> Result<(), T> {
                let mut q = self.shared.lock().expect("model ring");
                if q.len() >= self.capacity {
                    return Err(value);
                }
                q.push_back(value);
                Ok(())
            }
        }

        impl<T> Consumer<T> {
            pub(crate) fn pop(&mut self) -> Result<T, ()> {
                self.shared.lock().expect("model ring").pop_front().ok_or(())
            }
        }
    }
}
