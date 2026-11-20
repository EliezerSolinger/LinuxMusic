//! The block shared with a helper (I-9.6): one shared memory object per plugin instance, seen as `u32` words. A
//! header of control words, then the audio in, out and sidechain (`f32` bits, channel after channel, `max_block`
//! each), the events in (6 words each) and the events out (the plugin's own parameter changes, 2 words each).
//!
//! The handshake per block: the core writes the inputs, the events and the transport, then bumps `REQUEST` and wakes
//! it; the helper processes, writes the outputs, sets `DONE` to the request's number and wakes it. The core waits on
//! `DONE` until its deadline (live) or without one (export, S19).

use std::sync::atomic::{AtomicU32, Ordering};

use lm_render::{EventKind, Expression, NoteId};

pub const MAGIC: u32 = 0x4C4D_5048; // "LMPH"
/// The block's version: the core and the helper of one build must agree (the helper checks it)
pub const VERSION: u32 = 1;

pub mod w {
    pub const MAGIC: usize = 0;
    pub const VERSION: usize = 1;
    pub const MAX_BLOCK: usize = 2;
    pub const INPUTS: usize = 3;
    pub const OUTPUTS: usize = 4;
    pub const SIDECHAIN: usize = 5;
    pub const MAX_EVENTS: usize = 6;
    pub const MAX_OUT: usize = 7;
    /// Bumped by the core: a block to process
    pub const REQUEST: usize = 8;
    /// Set by the helper to the request it finished
    pub const DONE: usize = 9;
    pub const FRAMES: usize = 10;
    pub const EVENTS: usize = 11;
    /// Bit 0 playing, bit 1 recording
    pub const FLAGS: usize = 12;
    pub const POSITION_LO: usize = 13;
    pub const POSITION_HI: usize = 14;
    pub const TICK_LO: usize = 15;
    pub const TICK_HI: usize = 16;
    /// `f32` bits
    pub const BPM: usize = 17;
    /// The helper's: the plugin's latency in frames, kept current
    pub const LATENCY: usize = 18;
    /// The helper's: [`super::STATUS_READY`] once the plugin runs
    pub const STATUS: usize = 19;
    /// The helper's: how many events out this block
    pub const OUT_EVENTS: usize = 20;
    pub const RATE: usize = 21;
    /// Asked by the core: the helper ends its audio loop
    pub const QUIT: usize = 22;
    /// Set by the helper when the plugin says its own state changed (its window, a preset); the core clears it
    pub const STATE: usize = 23;
    /// Ticks per quarter note of `TICK`
    pub const PPQ: usize = 24;
    /// The time signature: numerator << 8 | denominator
    pub const SIGNATURE: usize = 25;
    pub const HEADER: usize = 32;
}

pub const STATUS_READY: u32 = 1;
pub const EVENT_WORDS: usize = 6;
pub const OUT_WORDS: usize = 2;

/// Where everything is in a block of this shape
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub max_block: usize,
    pub inputs: usize,
    pub outputs: usize,
    pub sidechain: usize,
    pub max_events: usize,
    pub max_out: usize,
}

impl Layout {
    pub fn new(max_block: usize, inputs: usize, outputs: usize, sidechain: usize) -> Self {
        Self { max_block, inputs, outputs, sidechain, max_events: 4096, max_out: 256 }
    }

    pub fn input(&self, ch: usize) -> usize {
        w::HEADER + ch * self.max_block
    }

    pub fn output(&self, ch: usize) -> usize {
        w::HEADER + (self.inputs + ch) * self.max_block
    }

    pub fn sidechain(&self, ch: usize) -> usize {
        w::HEADER + (self.inputs + self.outputs + ch) * self.max_block
    }

    pub fn events(&self) -> usize {
        w::HEADER + (self.inputs + self.outputs + self.sidechain) * self.max_block
    }

    pub fn out_events(&self) -> usize {
        self.events() + self.max_events * EVENT_WORDS
    }

    pub fn words(&self) -> usize {
        self.out_events() + self.max_out * OUT_WORDS
    }

    /// Writes the shape into a new block's header
    pub fn write_header(&self, words: &[AtomicU32], rate: u32) {
        let set = |i: usize, v: u32| words[i].store(v, Ordering::Relaxed);
        set(w::MAGIC, MAGIC);
        set(w::VERSION, VERSION);
        set(w::MAX_BLOCK, self.max_block as u32);
        set(w::INPUTS, self.inputs as u32);
        set(w::OUTPUTS, self.outputs as u32);
        set(w::SIDECHAIN, self.sidechain as u32);
        set(w::MAX_EVENTS, self.max_events as u32);
        set(w::MAX_OUT, self.max_out as u32);
        set(w::RATE, rate);
    }

    /// Reads the shape a block was made with (the helper), checking it
    pub fn read_header(words: &[AtomicU32]) -> Option<Layout> {
        let get = |i: usize| words.get(i).map(|x| x.load(Ordering::Relaxed) as usize);
        if get(w::MAGIC)? != MAGIC as usize || get(w::VERSION)? != VERSION as usize {
            return None;
        }
        let l = Layout {
            max_block: get(w::MAX_BLOCK)?,
            inputs: get(w::INPUTS)?,
            outputs: get(w::OUTPUTS)?,
            sidechain: get(w::SIDECHAIN)?,
            max_events: get(w::MAX_EVENTS)?,
            max_out: get(w::MAX_OUT)?,
        };
        (l.max_block > 0
            && l.max_block <= 1 << 16
            && l.inputs + l.outputs + l.sidechain <= 128
            && l.words() <= words.len())
        .then_some(l)
    }
}

/// An event as words: `[offset, kind, a, b, c, d]`
pub fn encode(offset: u32, kind: &EventKind) -> Option<[u32; EVENT_WORDS]> {
    let note = |n: NoteId| n.0;
    let ck = |channel: u8, key: u8| u32::from(channel) | (u32::from(key) << 8);
    Some(match *kind {
        EventKind::NoteOn { note: n, channel, key, velocity } => {
            [offset, 1, note(n), ck(channel, key), velocity.to_bits(), 0]
        }
        EventKind::NoteOff { note: n, channel, key, velocity } => {
            [offset, 2, note(n), ck(channel, key), velocity.to_bits(), 0]
        }
        EventKind::Param { index, value } => [offset, 3, index, value.to_bits(), 0, 0],
        EventKind::Midi { bytes } => [offset, 4, u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]), 0, 0, 0],
        EventKind::AllNotesRelease => [offset, 5, 0, 0, 0, 0],
        EventKind::AllSoundOff => [offset, 6, 0, 0, 0, 0],
        EventKind::NoteExpression { note: n, expression, value, ramp } => {
            let e = match expression {
                Expression::Pan => 0,
                Expression::Tuning => 1,
            };
            [offset, 7, note(n), e, value.to_bits(), ramp]
        }
    })
}

/// The words back as an event (the helper); `None` for a kind this build does not know
pub fn decode(x: [u32; EVENT_WORDS]) -> Option<(u32, EventKind)> {
    let key = |b: u32| ((b & 0xff) as u8, ((b >> 8) & 0x7f) as u8);
    let kind = match x[1] {
        1 => {
            let (channel, key) = key(x[3]);
            EventKind::NoteOn { note: NoteId(x[2]), channel, key, velocity: f32::from_bits(x[4]) }
        }
        2 => {
            let (channel, key) = key(x[3]);
            EventKind::NoteOff { note: NoteId(x[2]), channel, key, velocity: f32::from_bits(x[4]) }
        }
        3 => EventKind::Param { index: x[2], value: f32::from_bits(x[3]) },
        4 => {
            let b = x[2].to_le_bytes();
            EventKind::Midi { bytes: [b[0], b[1], b[2]] }
        }
        5 => EventKind::AllNotesRelease,
        6 => EventKind::AllSoundOff,
        7 => EventKind::NoteExpression {
            note: NoteId(x[2]),
            expression: if x[3] == 0 { Expression::Pan } else { Expression::Tuning },
            value: f32::from_bits(x[4]),
            ramp: x[5],
        },
        _ => return None,
    };
    Some((x[0], kind))
}

/// `f32`s into words
pub fn put(words: &[AtomicU32], at: usize, data: &[f32]) {
    for (w, x) in words[at..at + data.len()].iter().zip(data) {
        w.store(x.to_bits(), Ordering::Relaxed);
    }
}

/// Words into `f32`s
pub fn get(words: &[AtomicU32], at: usize, out: &mut [f32]) {
    let n = out.len();
    for (x, w) in out.iter_mut().zip(&words[at..at + n]) {
        *x = f32::from_bits(w.load(Ordering::Relaxed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip() {
        let kinds = [
            EventKind::NoteOn { note: NoteId(7), channel: 3, key: 64, velocity: 0.5 },
            EventKind::NoteOff { note: NoteId(7), channel: 3, key: 64, velocity: 0.0 },
            EventKind::Param { index: 9, value: -1.25 },
            EventKind::Midi { bytes: [0xb0, 64, 127] },
            EventKind::AllNotesRelease,
            EventKind::AllSoundOff,
            EventKind::NoteExpression { note: NoteId(2), expression: Expression::Tuning, value: 2.0, ramp: 480 },
        ];
        for k in kinds {
            let w = encode(17, &k).unwrap();
            assert_eq!(decode(w), Some((17, k)));
        }
        let l = Layout::new(256, 2, 2, 2);
        assert!(l.words() > l.out_events() && l.input(1) == w::HEADER + 256);
    }
}
