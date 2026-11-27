//! MPE on a live MIDI port (MIDI Polyphonic Expression, `decisoes-autonomas.md`, 26/11, item 5): in a zone, every note
//! has a channel of its own, and the pitch bend, the channel pressure and the CC 74 of that channel belong to that one
//! note. The renderer turns them into per-note expressions ([`crate::Expression::Tuning`], `Pressure`, `Timbre`).
//!
//! Everything here is plain data and arithmetic: nothing allocates and nothing locks (the audio path calls it)

use crate::event::Expression;
use crate::ids::PortIndex;

/// The member channels' default bend range of the MPE specification, in semitones
pub const DEFAULT_BEND_SEMITONES: f32 = 48.0;
/// The most member channels of a zone
pub const MAX_MEMBERS: u8 = 15;

/// Which end of the channels the zone's master channel is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// Master channel 1 (0), members 2 and up (1, 2, ...)
    Lower,
    /// Master channel 16 (15), members 15 and down (14, 13, ...)
    Upper,
}

/// The MPE setting of a port, and the last expression values seen on each channel (a bend sent before the
/// note-on, as the specification asks, is applied to the note that starts)
#[derive(Clone, Debug, PartialEq)]
pub struct MpePort {
    pub port: PortIndex,
    pub zone: Zone,
    /// The member channels of the zone, 1..=15
    pub members: u8,
    /// The semitones of a full pitch bend of a member channel
    pub bend_semitones: f32,
    /// By channel: tuning (semitones), pressure and timbre (0..1); `NaN` until the channel sent one
    pub last: [[f32; 3]; 16],
}

impl MpePort {
    pub fn new(port: PortIndex, zone: Zone, members: u8) -> Self {
        Self {
            port,
            zone,
            members: members.clamp(1, MAX_MEMBERS),
            bend_semitones: DEFAULT_BEND_SEMITONES,
            last: [[f32::NAN; 3]; 16],
        }
    }

    /// The zone's master channel
    pub fn master(&self) -> u8 {
        match self.zone {
            Zone::Lower => 0,
            Zone::Upper => 15,
        }
    }

    /// Whether `channel` is one of the zone's member channels
    pub fn is_member(&self, channel: u8) -> bool {
        match self.zone {
            Zone::Lower => channel >= 1 && channel <= self.members,
            Zone::Upper => channel < 15 && channel >= 15 - self.members,
        }
    }

    /// The expression a channel message of a member channel carries: pitch bend (semitones), channel pressure or
    /// CC 74 (0..1). `None` for anything else, which goes to the instruments as it came
    pub fn expression(&self, bytes: [u8; 3]) -> Option<(Expression, f32)> {
        match bytes[0] & 0xF0 {
            0xE0 => {
                let v = (i32::from(bytes[2] & 0x7F) << 7) | i32::from(bytes[1] & 0x7F);
                Some((Expression::Tuning, (v - 8192) as f32 / 8192.0 * self.bend_semitones))
            }
            0xD0 => Some((Expression::Pressure, f32::from(bytes[1] & 0x7F) / 127.0)),
            0xB0 if bytes[1] & 0x7F == 74 => Some((Expression::Timbre, f32::from(bytes[2] & 0x7F) / 127.0)),
            _ => None,
        }
    }

    fn slot(expression: Expression) -> Option<usize> {
        match expression {
            Expression::Tuning => Some(0),
            Expression::Pressure => Some(1),
            Expression::Timbre => Some(2),
            Expression::Pan => None,
        }
    }

    /// Remembers a value of a channel
    pub fn remember(&mut self, channel: u8, expression: Expression, value: f32) {
        if let Some(s) = Self::slot(expression) {
            self.last[usize::from(channel & 0x0F)][s] = value;
        }
    }

    /// The values of a channel that a starting note takes: `(expression, value)` for every one the channel sent
    pub fn pending(&self, channel: u8) -> [Option<(Expression, f32)>; 3] {
        let v = &self.last[usize::from(channel & 0x0F)];
        let get = |i: usize, e: Expression| (!v[i].is_nan()).then_some((e, v[i]));
        [get(0, Expression::Tuning), get(1, Expression::Pressure), get(2, Expression::Timbre)]
    }
}

/// The MPE zone of an output port: each sounding note gets a member channel of its own, and the note's expressions
/// go out on that channel (pitch bend, channel pressure, CC 74)
#[derive(Clone, Debug, PartialEq)]
pub struct MpeOut {
    zone: Zone,
    members: u8,
    bend_semitones: f32,
    /// By member index: the note that holds the channel (0 = free) and the key it was sent on
    slots: [(u32, u8); MAX_MEMBERS as usize],
    /// The member index the next search for a free channel starts at
    next: u8,
}

impl MpeOut {
    pub fn new(p: &MpePort) -> Self {
        Self {
            zone: p.zone,
            members: p.members.clamp(1, MAX_MEMBERS),
            bend_semitones: p.bend_semitones,
            slots: [(0, 0); MAX_MEMBERS as usize],
            next: 0,
        }
    }

    /// The zone's master channel (the messages that are not a note's go there)
    pub fn master(&self) -> u8 {
        match self.zone {
            Zone::Lower => 0,
            Zone::Upper => 15,
        }
    }

    fn channel_at(&self, index: u8) -> u8 {
        match self.zone {
            Zone::Lower => 1 + index,
            Zone::Upper => 14 - index,
        }
    }

    /// The member channels, in order
    pub fn channels(&self) -> impl Iterator<Item = u8> + '_ {
        (0..self.members).map(|i| self.channel_at(i))
    }

    /// A channel for a note that starts: a free one, round robin; with none free, the one held the longest
    /// (the next in the rotation) is taken over, as the specification suggests. `key` is the key it is sent on.
    /// Gives the channel and, when one was taken over, the key of the note that had it: its note-off must go out
    /// first, or the receiver keeps that note sounding
    pub fn start(&mut self, note: u32, key: u8) -> (u8, Option<u8>) {
        let n = self.members;
        let index =
            (0..n).map(|k| (self.next + k) % n).find(|i| self.slots[usize::from(*i)].0 == 0).unwrap_or(self.next);
        let (held, held_key) = self.slots[usize::from(index)];
        self.slots[usize::from(index)] = (note.max(1), key);
        self.next = (index + 1) % n;
        (self.channel_at(index), (held != 0).then_some(held_key))
    }

    fn index_of(&self, note: u32) -> Option<u8> {
        (0..self.members).find(|i| self.slots[usize::from(*i)].0 == note.max(1))
    }

    /// The channel a sounding note was given
    pub fn channel(&self, note: u32) -> Option<u8> {
        self.index_of(note).map(|i| self.channel_at(i))
    }

    /// A note ends: its channel and the key it was sent on, and the channel is free
    pub fn end(&mut self, note: u32) -> Option<(u8, u8)> {
        let i = self.index_of(note)?;
        let key = self.slots[usize::from(i)].1;
        self.slots[usize::from(i)] = (0, 0);
        Some((self.channel_at(i), key))
    }

    /// Every channel is free
    pub fn clear(&mut self) {
        self.slots = [(0, 0); MAX_MEMBERS as usize];
        self.next = 0;
    }

    /// The pitch bend message of a member channel for an offset in semitones (clamped to the range)
    pub fn bend(&self, channel: u8, semitones: f32) -> [u8; 3] {
        let v = (8192.0 + semitones / self.bend_semitones.max(0.01) * 8192.0).round().clamp(0.0, 16383.0) as u16;
        [0xE0 | (channel & 0x0F), (v & 0x7F) as u8, (v >> 7) as u8]
    }

    /// The message of an expression of a note on its channel, and its length; `None` for the pan (MPE has none)
    pub fn expression(&self, channel: u8, expression: Expression, value: f32) -> Option<([u8; 3], u8)> {
        let seven = |v: f32| (v.clamp(0.0, 1.0) * 127.0).round() as u8;
        match expression {
            Expression::Tuning => Some((self.bend(channel, value), 3)),
            Expression::Pressure => Some(([0xD0 | (channel & 0x0F), seven(value), 0], 2)),
            Expression::Timbre => Some(([0xB0 | (channel & 0x0F), 74, seven(value)], 3)),
            Expression::Pan => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_members_of_each_zone() {
        let lower = MpePort::new(PortIndex(0), Zone::Lower, 4);
        assert_eq!(lower.master(), 0);
        assert!(!lower.is_member(0) && lower.is_member(1) && lower.is_member(4) && !lower.is_member(5));
        let upper = MpePort::new(PortIndex(0), Zone::Upper, 3);
        assert_eq!(upper.master(), 15);
        assert!(!upper.is_member(15) && upper.is_member(14) && upper.is_member(12) && !upper.is_member(11));
    }

    #[test]
    fn bend_pressure_and_timbre_become_expressions() {
        let p = MpePort::new(PortIndex(0), Zone::Lower, 15);
        // The bend's centre is 0; the top of the 14 bits is the whole range
        assert_eq!(p.expression([0xE1, 0x00, 0x40]), Some((Expression::Tuning, 0.0)));
        let (e, v) = p.expression([0xE1, 0x7F, 0x7F]).unwrap();
        assert_eq!(e, Expression::Tuning);
        assert!((v - 48.0 * 8191.0 / 8192.0).abs() < 1e-3);
        assert_eq!(p.expression([0xD1, 127, 0]), Some((Expression::Pressure, 1.0)));
        assert_eq!(p.expression([0xB1, 74, 0]), Some((Expression::Timbre, 0.0)));
        assert_eq!(p.expression([0xB1, 7, 100]), None, "another controller goes to the instrument as it came");
        assert_eq!(p.expression([0x91, 60, 100]), None);
    }

    #[test]
    fn a_value_sent_before_the_note_is_kept_for_it() {
        let mut p = MpePort::new(PortIndex(0), Zone::Lower, 15);
        assert_eq!(p.pending(2), [None, None, None]);
        p.remember(2, Expression::Tuning, 1.5);
        p.remember(2, Expression::Timbre, 0.25);
        assert_eq!(p.pending(2), [Some((Expression::Tuning, 1.5)), None, Some((Expression::Timbre, 0.25))]);
        assert_eq!(p.pending(3), [None, None, None]);
    }

    #[test]
    fn an_output_zone_gives_each_note_a_channel() {
        let p = MpePort::new(PortIndex(1), Zone::Lower, 2);
        let mut out = MpeOut::new(&p);
        assert_eq!(out.start(10, 60), (1, None));
        assert_eq!(out.start(11, 64), (2, None));
        assert_eq!(out.start(12, 67), (1, Some(60)), "none free: the one held the longest is taken over, its key back");
        assert_eq!(out.end(10), None, "the taken-over note no longer holds a channel");
        assert_eq!(out.channel(11), Some(2));
        assert_eq!(out.end(11), Some((2, 64)));
        assert_eq!(out.channel(11), None);
        assert_eq!(out.channels().collect::<Vec<_>>(), vec![1, 2]);
        let up = MpeOut::new(&MpePort::new(PortIndex(1), Zone::Upper, 2));
        assert_eq!((up.master(), up.channels().collect::<Vec<_>>()), (15, vec![14, 13]));
    }

    #[test]
    fn an_output_expression_is_the_message_of_the_channel() {
        let out = MpeOut::new(&MpePort::new(PortIndex(0), Zone::Lower, 15));
        assert_eq!(out.bend(3, 0.0), [0xE3, 0x00, 0x40]);
        assert_eq!(out.bend(3, 48.0), [0xE3, 0x7F, 0x7F]);
        assert_eq!(out.expression(3, Expression::Pressure, 1.0), Some(([0xD3, 127, 0], 2)));
        assert_eq!(out.expression(3, Expression::Timbre, 0.0), Some(([0xB3, 74, 0], 3)));
        assert_eq!(out.expression(3, Expression::Pan, 0.5), None);
    }
}
