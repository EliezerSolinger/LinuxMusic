//! The events a processor receives, with their offset in the block (arquitetura-nova.md, base item 7: like CLAP).
//! A processor only sees events (R3): parameter changes from the slots and the automation, notes from the items and
//! the MIDI inputs

use crate::ids::NoteId;

/// One event, `offset` samples into the block being processed
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub offset: u32,
    pub kind: EventKind,
}

/// What a note expression changes (F-NOT-010, P-24)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expression {
    /// −1 (left) to 1 (right), 0 the note's own place; applied as a balance (the legacy instrument track's law)
    Pan,
    /// Semitones added to the note's pitch (its bend curve), before the instrument's own bend range and glide
    Tuning,
    /// 0 to 1: the pressure of the note (MPE channel pressure), 0 until the player presses
    Pressure,
    /// 0 to 1: the timbre of the note (MPE CC 74), 0.5 is the middle
    Timbre,
}

/// What happened. Closed: no generic message (rule 10)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventKind {
    /// A note starts. `velocity` is 0 to 1, linear (U28)
    NoteOn { note: NoteId, channel: u8, key: u8, velocity: f32 },
    /// The note with this number ends (the same node always gets it, U25)
    NoteOff { note: NoteId, channel: u8, key: u8, velocity: f32 },
    /// The processor's parameter `index` takes `value` at this offset
    Param { index: u32, value: f32 },
    /// A channel message other than a note (control change, pitch bend, pressure, program change), as received
    Midi { bytes: [u8; 3] },
    /// One sounding note's expression goes linearly from its current value to `value` over `ramp` samples from this
    /// offset (0: at once). A note starts at pan 0 and tuning 0; a new event replaces a ramp in progress from where
    /// it got to. Its expressions come after its note-on at the same offset
    NoteExpression { note: NoteId, expression: Expression, value: f32, ramp: u32 },
    /// Every sounding note goes to its release: stop, locate, loop wrap, a processor that leaves for a tail node (U32)
    AllNotesRelease,
    /// Silence now, no release (the panic command)
    AllSoundOff,
}

impl EventKind {
    /// The order of events at the same offset (U26): whatever ends sound first, then parameters (so a note that
    /// starts at the same offset already hears the new value), then the other messages, then the note-ons. A note-off
    /// therefore always precedes the note-on of the same key at the same offset, and retriggering works
    pub fn rank(&self) -> u8 {
        match self {
            EventKind::AllSoundOff => 0,
            EventKind::AllNotesRelease => 1,
            EventKind::NoteOff { .. } => 2,
            EventKind::Param { .. } => 3,
            EventKind::Midi { .. } => 4,
            EventKind::NoteOn { .. } => 5,
            EventKind::NoteExpression { .. } => 6,
        }
    }

    /// Events that end sound are never refused while there is room: a part of a node's event list is kept for them, so an overflow can lose a note-on but never strand a note (U25)
    pub(crate) fn ends_sound(&self) -> bool {
        self.rank() <= 2
    }

    /// Whether this is part of a track's note stream: what a chain of note effects handles (notes, their
    /// expressions, the other channel messages and the releases). A [`EventKind::Param`] is the node's own
    pub fn is_note_stream(&self) -> bool {
        !matches!(self, EventKind::Param { .. })
    }
}

/// The events a note effect emits in one call (the output of a [`crate::Processor::process_notes`]): a fixed
/// capacity, so pushing never allocates. Like [`EventList`], a part of it is kept for the events that end sound, so
/// an overflow can lose a note-on but never strand a note (U25). The order of the pushes does not matter: the next
/// stage sorts by (offset, [`EventKind::rank`], arrival)
#[derive(Debug)]
pub struct NoteOut {
    events: Vec<Event>,
    soft: usize,
    dropped: u32,
}

impl NoteOut {
    /// Room for `capacity` events of any kind, plus a third more kept for the ones that end sound
    pub fn with_capacity(capacity: usize) -> Self {
        let soft = capacity.max(6);
        let hard = soft + soft / 3 + 2;
        Self { events: Vec::with_capacity(hard), soft, dropped: 0 }
    }

    /// Adds an event; `false` (and counted as dropped) when the list is full for this kind of event
    pub fn push(&mut self, offset: u32, kind: EventKind) -> bool {
        let limit = if kind.ends_sound() { self.events.capacity() } else { self.soft };
        if self.events.len() >= limit {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        self.events.push(Event { offset, kind });
        true
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn as_slice(&self) -> &[Event] {
        &self.events
    }

    /// The events refused since the last call
    pub fn take_dropped(&mut self) -> u32 {
        std::mem::take(&mut self.dropped)
    }
}

/// A node's events for one pass, with a fixed capacity given by the program. Pushing never allocates; the list is
/// sorted by (offset, [`EventKind::rank`], arrival) before the processor sees it, which is a total order, so the
/// result does not depend on the order the renderer gathered them in
#[derive(Debug)]
pub(crate) struct EventList {
    keyed: Vec<(u64, Event)>,
    sorted: Vec<Event>,
    seq: u32,
    /// Events that do not end sound are refused past this; the rest of the capacity is kept for those that do
    soft: usize,
    /// The declared range of each of the node's parameters, by index (empty: none known). Every parameter event
    /// enters the node through [`EventList::push`], so this is the one place a value is limited to its schema range,
    /// whether it came from the document, an automation curve, a modulator, a MIDI control or a hand
    limits: Box<[(f32, f32)]>,
}

impl EventList {
    /// Room for `capacity` events of any kind, plus a third more kept for the ones that end sound
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let soft = capacity.max(6);
        let hard = soft + soft / 3 + 2;
        Self { keyed: Vec::with_capacity(hard), sorted: Vec::with_capacity(hard), seq: 0, soft, limits: Box::default() }
    }

    /// The parameter ranges (index -> (min, max)); a parameter with no entry is not limited
    pub(crate) fn set_limits(&mut self, limits: Box<[(f32, f32)]>) {
        self.limits = limits;
    }

    pub(crate) fn clear(&mut self) {
        self.keyed.clear();
        self.sorted.clear();
        self.seq = 0;
    }

    /// Adds an event; `false` (and nothing added) when the list is full for this kind of event. A parameter value is
    /// limited to its declared range first; one that is not a number is dropped (it counts as accepted)
    pub(crate) fn push(&mut self, offset: u32, mut kind: EventKind) -> bool {
        if let EventKind::Param { index, value } = &mut kind {
            if !value.is_finite() {
                return true;
            }
            if let Some(&(lo, hi)) = self.limits.get(*index as usize) {
                *value = value.clamp(lo, hi);
            }
        }
        let limit = if kind.ends_sound() { self.keyed.capacity() } else { self.soft };
        if self.keyed.len() >= limit {
            return false;
        }
        let key = (u64::from(offset) << 32) | (u64::from(kind.rank()) << 28) | u64::from(self.seq & 0x0FFF_FFFF);
        self.seq = self.seq.wrapping_add(1);
        self.keyed.push((key, Event { offset, kind }));
        true
    }

    /// Sorts and exposes the events (in place; `sort_unstable` does not allocate, and the keys are unique)
    pub(crate) fn finish(&mut self) -> &[Event] {
        self.keyed.sort_unstable_by_key(|(k, _)| *k);
        self.sorted.clear();
        self.sorted.extend(self.keyed.iter().map(|(_, e)| *e));
        &self.sorted
    }

    pub(crate) fn sorted(&self) -> &[Event] {
        &self.sorted
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    fn on(key: u8) -> EventKind {
        EventKind::NoteOn { note: NoteId(u32::from(key)), channel: 0, key, velocity: 1.0 }
    }
    fn off(key: u8) -> EventKind {
        EventKind::NoteOff { note: NoteId(u32::from(key)), channel: 0, key, velocity: 0.0 }
    }

    #[test]
    fn parameter_values_are_limited_to_the_declared_range_on_push() {
        let mut l = EventList::with_capacity(8);
        l.set_limits(vec![(0.0, 1.0), (-1.0, 1.0)].into_boxed_slice());
        assert!(l.push(0, EventKind::Param { index: 0, value: 7.0 }));
        assert!(l.push(1, EventKind::Param { index: 1, value: -9.0 }));
        assert!(l.push(2, EventKind::Param { index: 2, value: 123.0 }), "no limit declared for index 2");
        assert!(l.push(3, EventKind::Param { index: 0, value: f32::NAN }));
        let e = l.finish();
        assert_eq!(e.len(), 3, "the NaN is dropped");
        assert_eq!(e[0].kind, EventKind::Param { index: 0, value: 1.0 });
        assert_eq!(e[1].kind, EventKind::Param { index: 1, value: -1.0 });
        assert_eq!(e[2].kind, EventKind::Param { index: 2, value: 123.0 });
    }

    #[test]
    fn note_off_before_note_on_at_the_same_offset() {
        let mut l = EventList::with_capacity(16);
        assert!(l.push(5, on(60)));
        assert!(l.push(5, off(60)));
        assert!(l.push(0, EventKind::Param { index: 1, value: 0.5 }));
        assert!(l.push(5, EventKind::Param { index: 2, value: 0.5 }));
        let e = l.finish();
        assert_eq!(e[0].offset, 0);
        assert!(matches!(e[1].kind, EventKind::NoteOff { .. }));
        assert!(matches!(e[2].kind, EventKind::Param { index: 2, .. }));
        assert!(matches!(e[3].kind, EventKind::NoteOn { .. }));
    }

    #[test]
    fn arrival_order_kept_among_equals() {
        let mut l = EventList::with_capacity(16);
        for i in 0..5 {
            assert!(l.push(3, EventKind::Param { index: i, value: 0.0 }));
        }
        let e = l.finish();
        for (i, ev) in e.iter().enumerate() {
            assert_eq!(ev.kind, EventKind::Param { index: i as u32, value: 0.0 });
        }
    }

    #[test]
    fn room_kept_for_note_offs() {
        let mut l = EventList::with_capacity(6);
        let mut accepted = 0;
        for k in 0..10 {
            if l.push(0, on(k)) {
                accepted += 1;
            }
        }
        assert_eq!(accepted, 6);
        for k in 0..4 {
            assert!(l.push(0, off(k)));
        }
        assert!(!l.push(0, off(5)));
    }
}
