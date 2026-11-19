//! The parameter slots (interfaces.md, I-6.1, S8, S10, T15): fixed capacity, made by `engine()`, written by the main
//! thread and read by the renderer at the start of each block.
//!
//! Memory order (T15, model-checked with loom in `tests` below):
//! - the writer stores the value with `Relaxed`, then sets the dirty bit with `fetch_or(Release)`;
//! - the renderer swaps the bitset word for 0 with `swap(0, Acquire)` and only then reads the values of that word.
//!
//! The release/acquire pair on the bitset word makes every value stored before its bit was set visible to the reader
//! that took the bit. A write that lands after the swap sets the bit again and is read next block: nothing is lost,
//! and a value is never read before it is complete (an `f32` is one atomic word).
//!
//! The "touched" bitset (a gesture holds the parameter, so the hand wins over automation) has the same order, and
//! touching or releasing also sets the dirty bit, so the renderer notices the change in the next block

use crate::ids::ParamSlot;
use crate::sync::{Arc, AtomicU32, AtomicU64, Ordering};

#[derive(Debug)]
struct Inner {
    values: Box<[AtomicU32]>,
    dirty: Box<[AtomicU64]>,
    touched: Box<[AtomicU64]>,
}

/// The slots: an array of atomic `f32` bits and two bitsets of one `AtomicU64` per 64 slots. Cloning shares them
#[derive(Clone, Debug)]
pub struct ParamSlots {
    inner: Arc<Inner>,
}

impl ParamSlots {
    pub(crate) fn new(capacity: usize) -> Self {
        let words = capacity.div_ceil(64);
        Self {
            inner: Arc::new(Inner {
                values: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
                dirty: (0..words).map(|_| AtomicU64::new(0)).collect(),
                touched: (0..words).map(|_| AtomicU64::new(0)).collect(),
            }),
        }
    }

    pub fn capacity(&self) -> usize {
        self.inner.values.len()
    }

    fn index(&self, slot: ParamSlot) -> usize {
        let i = slot.0 as usize;
        assert!(i < self.inner.values.len(), "parameter slot {i} of {}", self.inner.values.len());
        i
    }

    /// Main thread: a new value. Store, then set the dirty bit
    pub fn write(&self, slot: ParamSlot, value: f32) {
        let i = self.index(slot);
        self.inner.values[i].store(value.to_bits(), Ordering::Relaxed);
        self.inner.dirty[i / 64].fetch_or(1 << (i % 64), Ordering::Release);
    }

    /// Main thread: a gesture started (`true`) or ended on this parameter
    pub fn touch(&self, slot: ParamSlot, touching: bool) {
        let i = self.index(slot);
        let bit = 1u64 << (i % 64);
        if touching {
            self.inner.touched[i / 64].fetch_or(bit, Ordering::Release);
        } else {
            self.inner.touched[i / 64].fetch_and(!bit, Ordering::Release);
        }
        self.inner.dirty[i / 64].fetch_or(bit, Ordering::Release);
    }

    /// The last value written (for the main thread and the renderer's `reset`/`adopt` reads)
    pub fn read(&self, slot: ParamSlot) -> f32 {
        f32::from_bits(self.inner.values[self.index(slot)].load(Ordering::Relaxed))
    }

    pub(crate) fn words(&self) -> usize {
        self.inner.dirty.len()
    }

    /// Renderer: takes the dirty bits of one word (the values of those slots may be read after this)
    pub(crate) fn take_dirty(&self, word: usize) -> u64 {
        self.inner.dirty[word].swap(0, Ordering::Acquire)
    }

    pub(crate) fn value_at(&self, index: usize) -> f32 {
        f32::from_bits(self.inner.values[index].load(Ordering::Relaxed))
    }

    pub(crate) fn touched_at(&self, index: usize) -> bool {
        self.inner.touched[index / 64].load(Ordering::Acquire) & (1 << (index % 64)) != 0
    }

    /// Whether a slot is touched (for the main thread and tests)
    pub fn is_touched(&self, slot: ParamSlot) -> bool {
        self.touched_at(self.index(slot))
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn write_sets_dirty_once() {
        let p = ParamSlots::new(130);
        p.write(ParamSlot(129), 0.25);
        p.write(ParamSlot(3), 1.0);
        assert_eq!(p.words(), 3);
        assert_eq!(p.take_dirty(0), 1 << 3);
        assert_eq!(p.take_dirty(0), 0);
        assert_eq!(p.take_dirty(2), 1 << 1);
        assert_eq!(p.value_at(129), 0.25);
        p.touch(ParamSlot(64), true);
        assert!(p.is_touched(ParamSlot(64)));
        assert_eq!(p.take_dirty(1), 1);
        p.touch(ParamSlot(64), false);
        assert!(!p.is_touched(ParamSlot(64)));
    }

    #[test]
    #[should_panic(expected = "parameter slot")]
    fn out_of_range_is_a_bug() {
        ParamSlots::new(4).write(ParamSlot(4), 0.0);
    }
}

#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    /// T15: whenever the renderer takes a dirty bit, the value it then reads is at least as new as the write that
    /// set the bit; and once both sides are done, the last value is always seen
    #[test]
    fn dirty_bit_publishes_the_value() {
        loom::model(|| {
            let slots = ParamSlots::new(2);
            let writer = slots.clone();
            let t = loom::thread::spawn(move || {
                writer.write(ParamSlot(1), 1.0);
                writer.write(ParamSlot(1), 2.0);
                writer.touch(ParamSlot(0), true);
            });
            let mut seen = 0.0f32;
            for _ in 0..2 {
                let bits = slots.take_dirty(0);
                if bits & 0b10 != 0 {
                    let v = slots.value_at(1);
                    assert!(v >= 1.0, "a dirty bit with an older value");
                    assert!(v >= seen, "values went back");
                    seen = v;
                }
                if bits & 0b01 != 0 {
                    assert!(slots.touched_at(0), "touch bit not visible with its dirty bit");
                }
            }
            t.join().expect("writer");
            let bits = slots.take_dirty(0);
            if bits & 0b10 != 0 || seen != 2.0 {
                assert_eq!(slots.value_at(1), 2.0);
            }
        });
    }
}
