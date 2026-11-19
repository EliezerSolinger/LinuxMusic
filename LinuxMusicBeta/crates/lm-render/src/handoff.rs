//! The install/return handshake (interfaces.md, I-5.3, S5, D2), generic so loom can check it alone.
//!
//! The main thread keeps a queue of its own; at most `capacity` messages are on the audio side at once (in the
//! forward ring, being applied, or in the return ring). Both rings have `capacity` slots, so:
//! - the forward push never fails (the main thread only pushes while fewer than `capacity` are out);
//! - the audio thread's return push never fails either (at most `capacity` messages exist on its side), so it never
//!   has to drop, and so never frees, anything.
//!
//! Nothing blocks: more messages than `capacity` wait in the main-thread queue and go out as returns come back

use std::collections::VecDeque;

use crate::sync::spsc::{Consumer, Producer, ring};

/// The main side
#[derive(Debug)]
pub(crate) struct HandoffSender<T> {
    forward: Producer<Box<T>>,
    back: Consumer<Box<T>>,
    queue: VecDeque<Box<T>>,
    in_flight: usize,
    capacity: usize,
}

/// The audio side
#[derive(Debug)]
pub(crate) struct HandoffReceiver<T> {
    forward: Consumer<Box<T>>,
    back: Producer<Box<T>>,
}

pub(crate) fn handoff<T>(capacity: usize) -> (HandoffSender<T>, HandoffReceiver<T>) {
    let capacity = capacity.max(1);
    let (ftx, frx) = ring(capacity);
    let (btx, brx) = ring(capacity);
    (
        HandoffSender { forward: ftx, back: brx, queue: VecDeque::new(), in_flight: 0, capacity },
        HandoffReceiver { forward: frx, back: btx },
    )
}

impl<T> HandoffSender<T> {
    /// Queues a message and sends what fits. Never blocks
    pub(crate) fn send(&mut self, message: Box<T>) {
        self.queue.push_back(message);
        self.flush();
    }

    fn flush(&mut self) {
        while self.in_flight < self.capacity {
            let Some(message) = self.queue.pop_front() else { break };
            match self.forward.push(message) {
                Ok(()) => self.in_flight += 1,
                Err(e) => {
                    // Impossible by the counting above; keep the order and try again later rather than lose it
                    self.queue.push_front(into_inner(e));
                    break;
                }
            }
        }
    }

    /// Takes every returned message, in order, then sends what waited in the queue
    pub(crate) fn collect(&mut self, mut f: impl FnMut(Box<T>)) -> usize {
        let mut n = 0;
        while let Ok(message) = self.back.pop() {
            self.in_flight -= 1;
            n += 1;
            f(message);
        }
        self.flush();
        n
    }

    /// Messages on the audio side
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Messages still in the main-thread queue
    pub(crate) fn queued(&self) -> usize {
        self.queue.len()
    }
}

impl<T> HandoffReceiver<T> {
    /// The next message, in the order sent
    pub(crate) fn receive(&mut self) -> Option<Box<T>> {
        self.forward.pop().ok()
    }

    /// Sends a message back. It always fits (see the module docs); if it ever did not, the message is leaked rather
    /// than freed on the audio thread
    pub(crate) fn give_back(&mut self, message: Box<T>) {
        if let Err(e) = self.back.push(message) {
            std::mem::forget(into_inner(e));
        }
    }
}

#[cfg(not(loom))]
fn into_inner<T>(e: lm_base::ring::PushError<T>) -> T {
    match e {
        lm_base::ring::PushError::Full(v) => v,
    }
}

#[cfg(loom)]
fn into_inner<T>(e: T) -> T {
    e
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn queue_beyond_capacity_never_blocks() {
        let (mut tx, mut rx) = handoff::<u32>(2);
        for i in 0..5 {
            tx.send(Box::new(i));
        }
        assert_eq!(tx.in_flight(), 2);
        assert_eq!(tx.queued(), 3);
        let mut got = Vec::new();
        while got.len() < 5 {
            while let Some(m) = rx.receive() {
                got.push(*m);
                rx.give_back(m);
            }
            tx.collect(|_| {});
        }
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
        tx.collect(|_| {});
        assert_eq!(tx.in_flight(), 0);
    }
}

#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    /// S5 and D2: every message is applied once, in order, and comes back; the audio side's return never fails
    #[test]
    fn every_install_applied_in_order_and_returned() {
        loom::model(|| {
            let (mut tx, mut rx) = handoff::<u32>(2);
            let audio = loom::thread::spawn(move || {
                let mut applied = Vec::new();
                while applied.len() < 3 {
                    match rx.receive() {
                        Some(m) => {
                            applied.push(*m);
                            rx.give_back(m);
                        }
                        None => loom::thread::yield_now(),
                    }
                }
                applied
            });
            for i in 0..3 {
                tx.send(Box::new(i));
                assert!(tx.in_flight() <= 2);
            }
            let mut returned = Vec::new();
            while returned.len() < 3 {
                if tx.collect(|m| returned.push(*m)) == 0 {
                    loom::thread::yield_now();
                }
            }
            assert_eq!(audio.join().expect("audio"), vec![0, 1, 2]);
            assert_eq!(returned, vec![0, 1, 2]);
            assert_eq!(tx.in_flight(), 0);
        });
    }
}
