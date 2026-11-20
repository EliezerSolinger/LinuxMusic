//! The active-notes table (U25): every note that sounds, with where it came from and which node got its note-on.
//! A note-off always goes to that node, even if the program changed in between; a node that leaves the program, a
//! loop wrap, a locate, a stop and an item edit end the notes they concern. Fixed capacity
//! (`Limits::max_active_notes`); a full table refuses the note-on (counted), never a note-off

use crate::ids::{NodeIndex, NoteId, PortIndex};

/// Where a sounding note came from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoteSource {
    /// A live MIDI port
    Port(PortIndex),
    /// Note `n` of the MIDI item `item`
    Item { item: u64, n: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActiveNote {
    pub(crate) source: NoteSource,
    pub(crate) channel: u8,
    pub(crate) key: u8,
    pub(crate) node: NodeIndex,
    pub(crate) note: NoteId,
}

#[derive(Debug)]
pub(crate) struct ActiveNotes {
    notes: Vec<ActiveNote>,
    next: u32,
}

impl ActiveNotes {
    pub(crate) fn new(capacity: usize) -> Self {
        Self { notes: Vec::with_capacity(capacity.max(1)), next: 1 }
    }

    pub(crate) fn len(&self) -> usize {
        self.notes.len()
    }

    /// Registers a note-on for `node`; `None` when the table is full
    pub(crate) fn start(&mut self, source: NoteSource, channel: u8, key: u8, node: NodeIndex) -> Option<NoteId> {
        if self.notes.len() == self.notes.capacity() {
            return None;
        }
        let note = NoteId(self.next);
        self.next = self.next.wrapping_add(1).max(1);
        self.notes.push(ActiveNote { source, channel, key, node, note });
        Some(note)
    }

    /// Removes the notes that match and hands each to `f`, in table order
    pub(crate) fn take_where(&mut self, mut pred: impl FnMut(&ActiveNote) -> bool, mut f: impl FnMut(ActiveNote)) {
        let mut i = 0;
        while i < self.notes.len() {
            if pred(&self.notes[i]) {
                f(self.notes.remove(i));
            } else {
                i += 1;
            }
        }
    }

    /// Removes and returns the first note that matches
    pub(crate) fn take_one(&mut self, pred: impl FnMut(&ActiveNote) -> bool) -> Option<ActiveNote> {
        let i = self.notes.iter().position(pred)?;
        Some(self.notes.remove(i))
    }

    /// An item's note still sounding (for its expressions)
    pub(crate) fn item_note(&self, item: u64, n: u32) -> Option<&ActiveNote> {
        self.notes.iter().find(|a| a.source == NoteSource::Item { item, n })
    }

    /// An item's note (its note-off)
    pub(crate) fn take_item_note(&mut self, item: u64, n: u32) -> Option<ActiveNote> {
        let i = self.notes.iter().position(|a| a.source == NoteSource::Item { item, n })?;
        Some(self.notes.remove(i))
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn note_off_finds_the_note_on_node() {
        let mut t = ActiveNotes::new(4);
        let a = NodeIndex::new(1, 1);
        let b = NodeIndex::new(2, 1);
        let p = PortIndex(0);
        let n1 = t.start(NoteSource::Port(p), 0, 60, a).expect("room");
        let n2 = t.start(NoteSource::Port(p), 0, 60, b).expect("room");
        t.start(NoteSource::Item { item: 9, n: 3 }, 0, 62, a).expect("room");
        assert_ne!(n1, n2);
        let mut got = Vec::new();
        while let Some(n) = t.take_one(|a| a.source == NoteSource::Port(p) && a.key == 60) {
            got.push((n.node, n.note));
        }
        assert_eq!(got, vec![(a, n1), (b, n2)]);
        assert_eq!(t.take_item_note(9, 3).map(|n| n.node), Some(a));
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn full_refuses_note_on() {
        let mut t = ActiveNotes::new(1);
        assert!(t.start(NoteSource::Port(PortIndex(0)), 0, 1, NodeIndex::default()).is_some());
        assert!(t.start(NoteSource::Port(PortIndex(0)), 0, 2, NodeIndex::default()).is_none());
    }
}
