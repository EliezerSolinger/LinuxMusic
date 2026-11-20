//! "MIDI learn" (interfaces.md, I-21.2; F-CTL-004): the first movement sets the source; in the next 300 ms the
//! detector looks for a 14-bit pair, an NRPN or RPN, or a relative encoder (values only near 1/127, 1/65 or 63/65).
//! Pure: the caller feeds stamped raw messages and asks for the result once the window has passed.

use crate::message::{Message, Raw};
use crate::transform::{Encoding, Mode};

/// How long after the first movement the detector keeps looking (ns)
pub const WINDOW_NS: u64 = 300_000_000;
/// The messages kept for the analysis (a 300 ms burst of one control)
const KEEP: usize = 128;

/// What learning found
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Learned {
    pub port: u16,
    pub channel: u8,
    pub message: Message,
    pub mode: Mode,
    /// A key or a controller that only sent 0 and 127: a button
    pub switch: bool,
}

#[derive(Clone, Copy, Debug)]
struct Seen {
    raw: Raw,
}

/// The learn detector of one learn request
#[derive(Debug)]
pub struct Detector {
    window: u64,
    start: Option<(u64, u16, u8)>,
    seen: Vec<Seen>,
}

impl Default for Detector {
    fn default() -> Self {
        Detector::new(WINDOW_NS)
    }
}

impl Detector {
    pub fn new(window: u64) -> Detector {
        Detector { window, start: None, seen: Vec::with_capacity(KEEP) }
    }

    /// Whether the first movement has come
    pub fn started(&self) -> bool {
        self.start.is_some()
    }

    /// One raw message from `port` at `at` (ns). Ignored: system messages, note-offs before anything, other ports
    /// and channels after the first movement, and anything after the window
    pub fn feed(&mut self, port: u16, at: u64, bytes: &[u8]) {
        let Some(raw) = Raw::parse(bytes) else { return };
        match self.start {
            None => {
                // A release or a data-entry continuation does not start learning
                if matches!(raw, Raw::NoteOff { .. }) {
                    return;
                }
                self.start = Some((at, port, raw.channel()));
            }
            Some((t0, p, ch)) => {
                if p != port || ch != raw.channel() || at > t0.saturating_add(self.window) {
                    return;
                }
            }
        }
        if self.seen.len() < KEEP {
            self.seen.push(Seen { raw });
        }
    }

    /// The result, once `now` is past the window (`None` before, or if nothing came)
    pub fn result(&self, now: u64) -> Option<Learned> {
        let (t0, port, channel) = self.start?;
        if now < t0.saturating_add(self.window) {
            return None;
        }
        Some(self.analyse(port, channel))
    }

    fn analyse(&self, port: u16, channel: u8) -> Learned {
        let first = self.seen[0].raw;
        let learned = |message, mode, switch| Learned { port, channel, message, mode, switch };
        let ccs = || {
            self.seen.iter().filter_map(|s| match s.raw {
                Raw::Cc { number, value, .. } => Some((number, value)),
                _ => None,
            })
        };
        match first {
            Raw::NoteOn { key, .. } | Raw::NoteOff { key, .. } => learned(Message::Note { key }, Mode::Absolute, true),
            Raw::PitchBend { .. } => learned(Message::PitchBend, Mode::Absolute, false),
            Raw::ChannelPressure { .. } => learned(Message::ChannelPressure, Mode::Absolute, false),
            Raw::PolyPressure { key, .. } => learned(Message::PolyPressure { key }, Mode::Absolute, false),
            Raw::ProgramChange { .. } => learned(Message::ProgramChange, Mode::Absolute, true),
            Raw::Cc { number, .. } => {
                // NRPN / RPN: a parameter selection followed by data
                let mut sel: [Option<u8>; 4] = [None; 4]; // 99, 98, 101, 100
                let mut param = None;
                for (n, v) in ccs() {
                    match n {
                        99 => sel[0] = Some(v),
                        98 => sel[1] = Some(v),
                        101 => sel[2] = Some(v),
                        100 => sel[3] = Some(v),
                        6 | 38 | 96 | 97 if param.is_none() => {
                            param = match sel {
                                [Some(m), Some(l), _, _] => {
                                    Some(Message::Nrpn { number: (u16::from(m) << 7) | u16::from(l) })
                                }
                                [_, _, Some(m), Some(l)] => {
                                    Some(Message::Rpn { number: (u16::from(m) << 7) | u16::from(l) })
                                }
                                _ => None,
                            };
                        }
                        _ => {}
                    }
                }
                if let Some(m) = param {
                    return learned(m, Mode::Absolute, false);
                }
                // 14-bit pair: CC n and CC n + 32 (whichever came first)
                let (lo, hi) = if number < 32 { (number, number + 32) } else { (number.wrapping_sub(32), number) };
                if (32..64).contains(&hi) && ccs().any(|(n, _)| n == lo) && ccs().any(|(n, _)| n == hi) {
                    return learned(Message::Cc14 { msb: lo }, Mode::Absolute, false);
                }
                let values: Vec<u8> = ccs().filter(|(n, _)| *n == number).map(|(_, v)| v).collect();
                let message = Message::Cc { number };
                if let Some(enc) = relative(&values) {
                    return learned(message, Mode::Relative(enc), false);
                }
                let switch = values.iter().all(|v| *v == 0 || *v == 127) && values.contains(&127);
                learned(message, Mode::Absolute, switch)
            }
        }
    }
}

/// A relative encoder sends small step values over and over; an absolute knob sends a run of different values.
/// The value sets: two's complement 1..=7 and 121..=127; sign-magnitude 1..=7 and 65..=71; offset 57..=63 and 65..=71
fn relative(values: &[u8]) -> Option<Encoding> {
    if values.len() < 2 {
        return None;
    }
    let small_up = |v: u8| (1..=7).contains(&v);
    let twos_down = |v: u8| (121..=127).contains(&v);
    let sm_down = |v: u8| (65..=71).contains(&v);
    let off_down = |v: u8| (57..=63).contains(&v);
    // Repeated values (or a direction change) tell an encoder from a knob being turned
    let mut distinct = values.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    let repeats = distinct.len() < values.len();
    if !repeats {
        return None;
    }
    if values.iter().all(|&v| small_up(v) || twos_down(v)) && !values.iter().all(|&v| v == 127) {
        return Some(Encoding::TwosComplement);
    }
    if values.iter().all(|&v| off_down(v) || sm_down(v)) && values.iter().all(|&v| v != 64) {
        return Some(Encoding::Offset64);
    }
    if values.iter().all(|&v| small_up(v) || sm_down(v)) {
        return Some(Encoding::SignMagnitude);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    fn learn(seq: &[&[u8]]) -> Learned {
        let mut d = Detector::default();
        for (i, m) in seq.iter().enumerate() {
            d.feed(2, 1000 + i as u64 * 5 * MS, m);
        }
        assert_eq!(d.result(1000 + 299 * MS), None, "not before the window");
        d.result(1000 + 300 * MS).expect("learned")
    }

    #[test]
    fn seven_bit_knob() {
        let seq: Vec<[u8; 3]> = (40..60).map(|v| [0xB3, 74, v]).collect();
        let refs: Vec<&[u8]> = seq.iter().map(|m| &m[..]).collect();
        let l = learn(&refs);
        assert_eq!(
            l,
            Learned { port: 2, channel: 3, message: Message::Cc { number: 74 }, mode: Mode::Absolute, switch: false }
        );
    }

    #[test]
    fn fourteen_bit_fader() {
        let mut seq = Vec::new();
        for v in 10..20u8 {
            seq.push([0xB0, 7, v]);
            seq.push([0xB0, 39, v * 3]);
        }
        let refs: Vec<&[u8]> = seq.iter().map(|m| &m[..]).collect();
        assert_eq!(learn(&refs).message, Message::Cc14 { msb: 7 });
        // Some controllers send the LSB first
        let mut seq = Vec::new();
        for v in 10..20u8 {
            seq.push([0xB0, 33, v]);
            seq.push([0xB0, 1, v]);
        }
        let refs: Vec<&[u8]> = seq.iter().map(|m| &m[..]).collect();
        assert_eq!(learn(&refs).message, Message::Cc14 { msb: 1 });
    }

    #[test]
    fn nrpn_and_rpn() {
        let seq: Vec<&[u8]> = vec![&[0xB0, 99, 3], &[0xB0, 98, 20], &[0xB0, 6, 64], &[0xB0, 38, 1], &[0xB0, 6, 65]];
        assert_eq!(learn(&seq).message, Message::Nrpn { number: (3 << 7) | 20 });
        let seq: Vec<&[u8]> = vec![&[0xB0, 101, 0], &[0xB0, 100, 2], &[0xB0, 6, 64]];
        assert_eq!(learn(&seq).message, Message::Rpn { number: 2 });
    }

    #[test]
    fn relative_encoders() {
        let twos: Vec<[u8; 3]> = [1, 1, 2, 1, 127, 127].iter().map(|v| [0xB0, 16, *v]).collect();
        let sm: Vec<[u8; 3]> = [1, 1, 65, 65, 66].iter().map(|v| [0xB0, 16, *v]).collect();
        let off: Vec<[u8; 3]> = [65, 65, 66, 63, 63].iter().map(|v| [0xB0, 16, *v]).collect();
        for (seq, enc) in [(twos, Encoding::TwosComplement), (sm, Encoding::SignMagnitude), (off, Encoding::Offset64)] {
            let refs: Vec<&[u8]> = seq.iter().map(|m| &m[..]).collect();
            assert_eq!(learn(&refs).mode, Mode::Relative(enc));
        }
        // A knob turned slowly near the bottom is absolute (no repeats)
        let seq: Vec<[u8; 3]> = (1..7).map(|v| [0xB0, 16, v]).collect();
        let refs: Vec<&[u8]> = seq.iter().map(|m| &m[..]).collect();
        assert_eq!(learn(&refs).mode, Mode::Absolute);
    }

    #[test]
    fn buttons_and_others() {
        let l = learn(&[&[0x91, 36, 100], &[0x81, 36, 0]]);
        assert_eq!((l.message, l.switch, l.channel), (Message::Note { key: 36 }, true, 1));
        let l = learn(&[&[0xB0, 64, 127], &[0xB0, 64, 0]]);
        assert!(l.switch);
        assert_eq!(learn(&[&[0xE0, 0, 64], &[0xE0, 0, 65]]).message, Message::PitchBend);
        assert_eq!(learn(&[&[0xC0, 5]]).message, Message::ProgramChange);
        // A release alone does not start; other channels and late messages are ignored
        let mut d = Detector::default();
        d.feed(0, 0, &[0x80, 1, 0]);
        assert!(!d.started());
        d.feed(0, 0, &[0xB0, 7, 1]);
        d.feed(0, 1, &[0xB1, 39, 1]);
        d.feed(1, 2, &[0xB0, 39, 1]);
        d.feed(0, 400 * MS, &[0xB0, 39, 1]);
        assert_eq!(d.result(400 * MS).unwrap().message, Message::Cc { number: 7 });
    }
}
