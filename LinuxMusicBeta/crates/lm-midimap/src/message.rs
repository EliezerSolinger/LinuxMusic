//! What a mapping listens to (interfaces.md, I-21.2): the message kinds, and the raw channel messages they come from.

/// The message of a mapping's source. 14-bit kinds (`Cc14`, `Nrpn`, `Rpn`, `PitchBend`) carry values 0..=16383
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Message {
    /// A 7-bit controller
    Cc {
        number: u8,
    },
    /// A 14-bit pair: `msb` (0..=31) and `msb + 32`
    Cc14 {
        msb: u8,
    },
    /// A non-registered parameter (CC 99/98 select, 6/38 data, 96/97 increment/decrement)
    Nrpn {
        number: u16,
    },
    /// A registered parameter (CC 101/100 select)
    Rpn {
        number: u16,
    },
    Note {
        key: u8,
    },
    PitchBend,
    ChannelPressure,
    PolyPressure {
        key: u8,
    },
    ProgramChange,
}

impl Message {
    /// The permanent name of the kind, as in the document (`midi.source.message`)
    pub fn kind_name(self) -> &'static str {
        match self {
            Message::Cc { .. } => "cc",
            Message::Cc14 { .. } => "cc14",
            Message::Nrpn { .. } => "nrpn",
            Message::Rpn { .. } => "rpn",
            Message::Note { .. } => "note",
            Message::PitchBend => "pitchBend",
            Message::ChannelPressure => "channelPressure",
            Message::PolyPressure { .. } => "polyPressure",
            Message::ProgramChange => "programChange",
        }
    }

    /// The number stored with the kind (CC number, msb, parameter number, key; 0 for the others)
    pub fn number(self) -> u16 {
        match self {
            Message::Cc { number } => u16::from(number),
            Message::Cc14 { msb } => u16::from(msb),
            Message::Nrpn { number } | Message::Rpn { number } => number,
            Message::Note { key } | Message::PolyPressure { key } => u16::from(key),
            Message::PitchBend | Message::ChannelPressure | Message::ProgramChange => 0,
        }
    }

    /// The message from the document's kind name and number; `None` when the name is unknown or the number is out of
    /// range for the kind
    pub fn from_parts(kind: &str, number: u16) -> Option<Message> {
        let b7 = u8::try_from(number).ok().filter(|n| *n < 128);
        Some(match kind {
            "cc" => Message::Cc { number: b7? },
            "cc14" => Message::Cc14 { msb: b7.filter(|n| *n < 32)? },
            "nrpn" if number < 16384 => Message::Nrpn { number },
            "rpn" if number < 16384 => Message::Rpn { number },
            "note" => Message::Note { key: b7? },
            "pitchBend" => Message::PitchBend,
            "channelPressure" => Message::ChannelPressure,
            "polyPressure" => Message::PolyPressure { key: b7? },
            "programChange" => Message::ProgramChange,
            _ => return None,
        })
    }

    /// The largest value of the kind (127 or 16383)
    pub fn max_value(self) -> u16 {
        if self.is_14bit() { 16383 } else { 127 }
    }

    pub fn is_14bit(self) -> bool {
        matches!(self, Message::Cc14 { .. } | Message::Nrpn { .. } | Message::Rpn { .. } | Message::PitchBend)
    }

    /// Keys and on/off messages: what a button sends
    pub fn is_note(self) -> bool {
        matches!(self, Message::Note { .. })
    }

    /// A sort key, for the compiled table's index
    pub(crate) fn key(self) -> u32 {
        let k = match self {
            Message::Cc { .. } => 0,
            Message::Cc14 { .. } => 1,
            Message::Nrpn { .. } => 2,
            Message::Rpn { .. } => 3,
            Message::Note { .. } => 4,
            Message::PitchBend => 5,
            Message::ChannelPressure => 6,
            Message::PolyPressure { .. } => 7,
            Message::ProgramChange => 8,
        };
        (k << 16) | u32::from(self.number())
    }
}

/// One raw channel message, parsed. System messages (status 0xF0 and up) are not channel messages
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raw {
    NoteOff {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOn {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    PolyPressure {
        channel: u8,
        key: u8,
        value: u8,
    },
    Cc {
        channel: u8,
        number: u8,
        value: u8,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    ChannelPressure {
        channel: u8,
        value: u8,
    },
    /// 0..=16383, 8192 at the centre
    PitchBend {
        channel: u8,
        value: u16,
    },
}

impl Raw {
    /// Parses a complete channel message (no running status). `None` for system messages and malformed bytes
    pub fn parse(bytes: &[u8]) -> Option<Raw> {
        let status = *bytes.first()?;
        if !(0x80..0xF0).contains(&status) {
            return None;
        }
        let channel = status & 0x0F;
        let d1 = bytes.get(1).copied().unwrap_or(0);
        let d2 = bytes.get(2).copied().unwrap_or(0);
        if d1 >= 0x80 || d2 >= 0x80 {
            return None;
        }
        let two = |n: usize| bytes.len() >= n;
        Some(match status & 0xF0 {
            0x80 if two(3) => Raw::NoteOff { channel, key: d1, velocity: d2 },
            0x90 if two(3) => {
                if d2 == 0 {
                    Raw::NoteOff { channel, key: d1, velocity: 0 }
                } else {
                    Raw::NoteOn { channel, key: d1, velocity: d2 }
                }
            }
            0xA0 if two(3) => Raw::PolyPressure { channel, key: d1, value: d2 },
            0xB0 if two(3) => Raw::Cc { channel, number: d1, value: d2 },
            0xC0 if two(2) => Raw::ProgramChange { channel, program: d1 },
            0xD0 if two(2) => Raw::ChannelPressure { channel, value: d1 },
            0xE0 if two(3) => Raw::PitchBend { channel, value: u16::from(d1) | (u16::from(d2) << 7) },
            _ => return None,
        })
    }

    pub fn channel(self) -> u8 {
        match self {
            Raw::NoteOff { channel, .. }
            | Raw::NoteOn { channel, .. }
            | Raw::PolyPressure { channel, .. }
            | Raw::Cc { channel, .. }
            | Raw::ProgramChange { channel, .. }
            | Raw::ChannelPressure { channel, .. }
            | Raw::PitchBend { channel, .. } => channel,
        }
    }

    /// The bytes of the message and how many of them count
    pub fn encode(self) -> ([u8; 3], u8) {
        match self {
            Raw::NoteOff { channel, key, velocity } => ([0x80 | channel, key, velocity], 3),
            Raw::NoteOn { channel, key, velocity } => ([0x90 | channel, key, velocity], 3),
            Raw::PolyPressure { channel, key, value } => ([0xA0 | channel, key, value], 3),
            Raw::Cc { channel, number, value } => ([0xB0 | channel, number, value], 3),
            Raw::ProgramChange { channel, program } => ([0xC0 | channel, program, 0], 2),
            Raw::ChannelPressure { channel, value } => ([0xD0 | channel, value, 0], 2),
            Raw::PitchBend { channel, value } => {
                ([0xE0 | channel, (value & 0x7F) as u8, ((value >> 7) & 0x7F) as u8], 3)
            }
        }
    }
}

/// The length of a channel message from its status byte (0 for system messages)
pub fn channel_message_len(status: u8) -> u8 {
    match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 3,
        0xC0 | 0xD0 => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_encode_round_trip() {
        for raw in [
            Raw::NoteOn { channel: 3, key: 60, velocity: 100 },
            Raw::NoteOff { channel: 0, key: 1, velocity: 64 },
            Raw::PolyPressure { channel: 15, key: 2, value: 3 },
            Raw::Cc { channel: 1, number: 7, value: 127 },
            Raw::ProgramChange { channel: 2, program: 99 },
            Raw::ChannelPressure { channel: 9, value: 5 },
            Raw::PitchBend { channel: 4, value: 16383 },
            Raw::PitchBend { channel: 4, value: 8192 },
        ] {
            let (b, n) = raw.encode();
            assert_eq!(Raw::parse(&b[..usize::from(n)]), Some(raw));
            assert_eq!(channel_message_len(b[0]), n);
        }
        assert_eq!(Raw::parse(&[0x90, 60, 0]), Some(Raw::NoteOff { channel: 0, key: 60, velocity: 0 }));
        assert_eq!(Raw::parse(&[0xF8]), None);
        assert_eq!(Raw::parse(&[0x90, 0x80, 1]), None);
        assert_eq!(Raw::parse(&[0x90, 1]), None);
    }

    #[test]
    fn kinds_round_trip() {
        for m in [
            Message::Cc { number: 7 },
            Message::Cc14 { msb: 31 },
            Message::Nrpn { number: 16383 },
            Message::Rpn { number: 0 },
            Message::Note { key: 127 },
            Message::PitchBend,
            Message::ChannelPressure,
            Message::PolyPressure { key: 3 },
            Message::ProgramChange,
        ] {
            assert_eq!(Message::from_parts(m.kind_name(), m.number()), Some(m));
        }
        assert_eq!(Message::from_parts("cc14", 32), None);
        assert_eq!(Message::from_parts("cc", 128), None);
        assert_eq!(Message::from_parts("x", 0), None);
    }
}
