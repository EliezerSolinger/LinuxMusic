//! The fixed-size rings that cross threads (interfaces.md, Parte 2, rule 3 and 4): one producer, one consumer,
//! allocated when made, never growing. Pushing and popping never allocate, lock or wait, so the audio thread can do
//! both. Every ring is sized in time (C4): use [`capacity_for`] to turn "N events per second for S seconds" into a size.
//!
//! The policy when a ring is full belongs to whoever uses it: `push` hands the value back, and the caller either counts
//! it as dropped (and reports it) or stops the operation with an explicit error. Rings that carry ownership must never
//! be allowed to fill (T9): size them for the maximum that can be in flight.

pub use rtrb::{chunks, Consumer, PopError, Producer, PushError};

/// A ring of `capacity` slots
pub fn ring<T>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    rtrb::RingBuffer::new(capacity.max(1))
}

/// The capacity for `per_second` items over `seconds`, rounded up, with at least `minimum` slots
pub fn capacity_for(per_second: f64, seconds: f64, minimum: usize) -> usize {
    let needed = (per_second.max(0.0) * seconds.max(0.0)).ceil();
    // A capacity that does not fit a usize is a configuration bug; saturate rather than wrap
    let needed = if needed >= usize::MAX as f64 { usize::MAX } else { needed as usize };
    needed.max(minimum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_ring_hands_the_value_back() {
        let (mut tx, mut rx) = ring::<u32>(2);
        assert!(tx.push(1).is_ok());
        assert!(tx.push(2).is_ok());
        assert!(matches!(tx.push(3), Err(PushError::Full(3))));
        assert_eq!(rx.pop().ok(), Some(1));
        assert!(tx.push(3).is_ok());
    }

    #[test]
    fn capacity_in_time() {
        // 16 MIDI ports at 1000 events per second, 10 s of a stopped pump (C4)
        assert_eq!(capacity_for(16_000.0, 10.0, 64), 160_000);
        assert_eq!(capacity_for(0.5, 1.0, 8), 8);
    }

    #[test]
    fn crosses_threads_in_order() {
        let (mut tx, mut rx) = ring::<u32>(64);
        let producer = std::thread::spawn(move || {
            for i in 0..10_000 {
                while tx.push(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        });
        let mut next = 0;
        while next < 10_000 {
            if let Ok(v) = rx.pop() {
                assert_eq!(v, next);
                next += 1;
            }
        }
        producer.join().expect("producer");
    }
}
