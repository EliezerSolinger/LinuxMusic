//! MIDI messages and their translation to and from the ALSA sequencer's events. Platform independent (tested
//! everywhere); the sequencer itself is in the Linux part.
//!
//! The engine speaks raw MIDI bytes (`Command::Midi { bytes: [u8; 3] }`, I-7.1), the sequencer speaks typed events.
//! Only short messages cross (no sysex in F1, T14): channel voice messages, system common and real-time. 14-bit
//! controllers and (N)RPNs, which a client may send as one event, become the two or four controller messages a
//! hardware port would have sent.

// The translation is used by the Linux sequencer; elsewhere only its tests use it
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// A sequencer address: client and port
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Addr {
    pub client: u8,
    pub port: u8,
}

impl Addr {
    pub const fn new(client: u8, port: u8) -> Self {
        Self { client, port }
    }
}

/// One short MIDI message: up to three bytes, the status first
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MidiMessage {
    pub bytes: [u8; 3],
    pub len: u8,
}

impl MidiMessage {
    pub const fn new1(a: u8) -> Self {
        Self { bytes: [a, 0, 0], len: 1 }
    }

    pub const fn new2(a: u8, b: u8) -> Self {
        Self { bytes: [a, b & 0x7f, 0], len: 2 }
    }

    pub const fn new3(a: u8, b: u8, c: u8) -> Self {
        Self { bytes: [a, b & 0x7f, c & 0x7f], len: 3 }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    /// The message from raw bytes: a status byte and the right number of data bytes. `None` for sysex, running
    /// status, a data byte first, or a wrong length
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let status = *bytes.first()?;
        let len = expected_len(status)?;
        if bytes.len() != len || bytes[1..].iter().any(|b| b & 0x80 != 0) {
            return None;
        }
        let mut out = [0u8; 3];
        out[..len].copy_from_slice(bytes);
        Some(Self { bytes: out, len: len as u8 })
    }
}

/// Length of a message with this status byte, `None` for data bytes, sysex and its end, and the undefined statuses
pub(crate) fn expected_len(status: u8) -> Option<usize> {
    match status {
        0x80..=0xbf | 0xe0..=0xef => Some(3),
        0xc0..=0xdf => Some(2),
        0xf1 | 0xf3 => Some(2),
        0xf2 => Some(3),
        0xf6 | 0xf8 | 0xfa | 0xfb | 0xfc | 0xfe | 0xff => Some(1),
        _ => None,
    }
}

/// ALSA sequencer event types (`snd_seq_event_type`) the translation knows
pub(crate) mod ev {
    pub const NOTEON: u8 = 6;
    pub const NOTEOFF: u8 = 7;
    pub const KEYPRESS: u8 = 8;
    pub const CONTROLLER: u8 = 10;
    pub const PGMCHANGE: u8 = 11;
    pub const CHANPRESS: u8 = 12;
    pub const PITCHBEND: u8 = 13;
    pub const CONTROL14: u8 = 14;
    pub const NONREGPARAM: u8 = 15;
    pub const REGPARAM: u8 = 16;
    pub const SONGPOS: u8 = 20;
    pub const SONGSEL: u8 = 21;
    pub const QFRAME: u8 = 22;
    pub const START: u8 = 30;
    pub const CONTINUE: u8 = 31;
    pub const STOP: u8 = 32;
    pub const CLOCK: u8 = 36;
    pub const TUNE_REQUEST: u8 = 40;
    pub const RESET: u8 = 41;
    pub const SENSING: u8 = 42;
    pub const CLIENT_START: u8 = 60;
    pub const CLIENT_EXIT: u8 = 61;
    pub const CLIENT_CHANGE: u8 = 62;
    pub const PORT_START: u8 = 63;
    pub const PORT_EXIT: u8 = 64;
    pub const PORT_CHANGE: u8 = 65;
    pub const PORT_SUBSCRIBED: u8 = 66;
    pub const PORT_UNSUBSCRIBED: u8 = 67;
}

/// The fields of a sequencer event that carry a short message: the note fields (`data.note`) or the control fields
/// (`data.control`), depending on the type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct SeqFields {
    pub kind: u8,
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
    pub param: u32,
    pub value: i32,
}

/// Up to four messages, made from one event without allocating
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Messages {
    items: [Option<MidiMessage>; 4],
}

impl Messages {
    fn push(&mut self, m: MidiMessage) {
        if let Some(slot) = self.items.iter_mut().find(|s| s.is_none()) {
            *slot = Some(m);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = MidiMessage> + '_ {
        self.items.iter().flatten().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.items[0].is_none()
    }
}

fn clamp7(v: i32) -> u8 {
    v.clamp(0, 127) as u8
}

/// The MIDI messages of one sequencer event; empty for events that are not short MIDI messages (sysex, queue and
/// system events), which the reader counts as dropped
pub(crate) fn seq_to_midi(f: SeqFields) -> Messages {
    let ch = f.channel & 0x0f;
    let mut out = Messages::default();
    let cc = |n: u32, v: i32| MidiMessage::new3(0xb0 | ch, (n & 0x7f) as u8, clamp7(v));
    match f.kind {
        // A note-on with velocity 0 stays as it came: it is a note-off by the MIDI rules, and the engine knows that
        ev::NOTEON => out.push(MidiMessage::new3(0x90 | ch, f.note, f.velocity)),
        ev::NOTEOFF => out.push(MidiMessage::new3(0x80 | ch, f.note, f.velocity)),
        ev::KEYPRESS => out.push(MidiMessage::new3(0xa0 | ch, f.note, f.velocity)),
        ev::CONTROLLER => out.push(cc(f.param, f.value)),
        ev::PGMCHANGE => out.push(MidiMessage::new2(0xc0 | ch, clamp7(f.value))),
        ev::CHANPRESS => out.push(MidiMessage::new2(0xd0 | ch, clamp7(f.value))),
        ev::PITCHBEND => {
            let v = (f.value + 8192).clamp(0, 16383);
            out.push(MidiMessage::new3(0xe0 | ch, (v & 0x7f) as u8, (v >> 7) as u8));
        }
        ev::CONTROL14 => {
            let v = f.value.clamp(0, 16383);
            if f.param < 32 {
                out.push(cc(f.param, v >> 7));
                out.push(cc(f.param + 32, v & 0x7f));
            } else {
                // Not a 14-bit pair: the value is a plain 7-bit controller
                out.push(cc(f.param, v));
            }
        }
        ev::NONREGPARAM | ev::REGPARAM => {
            let (msb, lsb) = if f.kind == ev::NONREGPARAM { (99, 98) } else { (101, 100) };
            let p = (f.param & 0x3fff) as i32;
            let v = f.value.clamp(0, 16383);
            out.push(cc(msb, p >> 7));
            out.push(cc(lsb, p & 0x7f));
            out.push(cc(6, v >> 7));
            out.push(cc(38, v & 0x7f));
        }
        ev::SONGPOS => {
            let v = f.value.clamp(0, 16383);
            out.push(MidiMessage::new3(0xf2, (v & 0x7f) as u8, (v >> 7) as u8));
        }
        ev::SONGSEL => out.push(MidiMessage::new2(0xf3, clamp7(f.value))),
        ev::QFRAME => out.push(MidiMessage::new2(0xf1, clamp7(f.value))),
        ev::TUNE_REQUEST => out.push(MidiMessage::new1(0xf6)),
        ev::CLOCK => out.push(MidiMessage::new1(0xf8)),
        ev::START => out.push(MidiMessage::new1(0xfa)),
        ev::CONTINUE => out.push(MidiMessage::new1(0xfb)),
        ev::STOP => out.push(MidiMessage::new1(0xfc)),
        ev::SENSING => out.push(MidiMessage::new1(0xfe)),
        ev::RESET => out.push(MidiMessage::new1(0xff)),
        _ => {}
    }
    out
}

/// The sequencer event for one MIDI message (the inverse of [`seq_to_midi`] for single messages)
pub(crate) fn midi_to_seq(m: &MidiMessage) -> Option<SeqFields> {
    let b = m.bytes;
    let status = b[0];
    let ch = status & 0x0f;
    let note = |kind| SeqFields { kind, channel: ch, note: b[1], velocity: b[2], ..SeqFields::default() };
    let ctrl = |kind, param: u32, value: i32| SeqFields { kind, channel: ch, param, value, ..SeqFields::default() };
    let sys = |kind, value: i32| SeqFields { kind, value, ..SeqFields::default() };
    if expected_len(status)? != usize::from(m.len) {
        return None;
    }
    Some(match status {
        0x80..=0x8f => note(ev::NOTEOFF),
        0x90..=0x9f => note(ev::NOTEON),
        0xa0..=0xaf => note(ev::KEYPRESS),
        0xb0..=0xbf => ctrl(ev::CONTROLLER, u32::from(b[1]), i32::from(b[2])),
        0xc0..=0xcf => ctrl(ev::PGMCHANGE, 0, i32::from(b[1])),
        0xd0..=0xdf => ctrl(ev::CHANPRESS, 0, i32::from(b[1])),
        0xe0..=0xef => ctrl(ev::PITCHBEND, 0, (i32::from(b[1]) | (i32::from(b[2]) << 7)) - 8192),
        0xf1 => sys(ev::QFRAME, i32::from(b[1])),
        0xf2 => sys(ev::SONGPOS, i32::from(b[1]) | (i32::from(b[2]) << 7)),
        0xf3 => sys(ev::SONGSEL, i32::from(b[1])),
        0xf6 => sys(ev::TUNE_REQUEST, 0),
        0xf8 => sys(ev::CLOCK, 0),
        0xfa => sys(ev::START, 0),
        0xfb => sys(ev::CONTINUE, 0),
        0xfc => sys(ev::STOP, 0),
        0xfe => sys(ev::SENSING, 0),
        0xff => sys(ev::RESET, 0),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(m: Messages) -> Vec<Vec<u8>> {
        m.iter().map(|m| m.as_slice().to_vec()).collect()
    }

    #[test]
    fn every_short_message_round_trips() {
        let messages: [&[u8]; 17] = [
            &[0x81, 60, 0],
            &[0x92, 64, 100],
            &[0xa3, 1, 2],
            &[0xb4, 7, 127],
            &[0xc5, 12],
            &[0xd6, 99],
            &[0xe7, 0, 0],
            &[0xe7, 0x7f, 0x7f],
            &[0xf1, 5],
            &[0xf2, 3, 4],
            &[0xf3, 9],
            &[0xf6],
            &[0xf8],
            &[0xfa],
            &[0xfb],
            &[0xfc],
            &[0xfe],
        ];
        for bytes in messages {
            let m = MidiMessage::from_bytes(bytes).expect("valid");
            let fields = midi_to_seq(&m).expect("translatable");
            assert_eq!(all(seq_to_midi(fields)), vec![bytes.to_vec()], "{bytes:02x?}");
        }
    }

    #[test]
    fn pitch_bend_centre_is_8192() {
        let f = midi_to_seq(&MidiMessage::new3(0xe0, 0, 0x40)).expect("pitch bend");
        assert_eq!(f.value, 0);
    }

    #[test]
    fn fourteen_bit_events_become_controller_pairs() {
        let f = SeqFields { kind: ev::CONTROL14, channel: 2, param: 7, value: 0x1234, ..SeqFields::default() };
        assert_eq!(all(seq_to_midi(f)), vec![vec![0xb2, 7, 0x24], vec![0xb2, 39, 0x34]]);
        let f = SeqFields { kind: ev::NONREGPARAM, channel: 0, param: 0x0101, value: 0x0081, ..SeqFields::default() };
        assert_eq!(
            all(seq_to_midi(f)),
            vec![vec![0xb0, 99, 2], vec![0xb0, 98, 1], vec![0xb0, 6, 1], vec![0xb0, 38, 1]]
        );
    }

    #[test]
    fn invalid_bytes_are_refused() {
        assert!(MidiMessage::from_bytes(&[]).is_none());
        assert!(MidiMessage::from_bytes(&[0x40]).is_none());
        assert!(MidiMessage::from_bytes(&[0xf0, 1, 0xf7]).is_none());
        assert!(MidiMessage::from_bytes(&[0x90, 60]).is_none());
        assert!(MidiMessage::from_bytes(&[0x90, 60, 0x80]).is_none());
    }

    #[test]
    fn other_events_make_no_message() {
        assert!(seq_to_midi(SeqFields { kind: ev::PORT_START, ..SeqFields::default() }).is_empty());
    }
}
