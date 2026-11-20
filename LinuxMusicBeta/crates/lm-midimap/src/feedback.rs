//! MIDI feedback (interfaces.md, I-21.3): the target's value back to the controller, as CC, 14-bit pair, NRPN, note
//! velocity (LEDs), pitch bend (motor faders), or a Mackie Control LED ring. With a rate limit (one value per
//! control every 10 ms, always the latest) and no echo (a value that just came from the source does not go back to it
//! for 50 ms, so a motor fader does not fight the hand). Times are in the caller's unit (samples in the renderer,
//! nanoseconds in the session).

use crate::message::{Message, Raw};

/// The LED ring modes of a Mackie Control V-pot (CC 0x30 + strip, value `mode << 4 | position`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ring {
    /// Not a ring: the plain value of the message
    None,
    /// One LED at the position
    Dot,
    /// From the centre to the position (pan, EQ gain)
    BoostCut,
    /// From the left to the position (level)
    Wrap,
    /// Symmetric spread from the centre (width, Q)
    Spread,
}

impl Ring {
    pub const NAMES: &'static [&'static str] = &["none", "dot", "boostCut", "wrap", "spread"];

    pub fn name(self) -> &'static str {
        match self {
            Ring::None => "none",
            Ring::Dot => "dot",
            Ring::BoostCut => "boostCut",
            Ring::Wrap => "wrap",
            Ring::Spread => "spread",
        }
    }

    pub fn from_name(name: &str) -> Option<Ring> {
        Some(match name {
            "none" => Ring::None,
            "dot" => Ring::Dot,
            "boostCut" => Ring::BoostCut,
            "wrap" => Ring::Wrap,
            "spread" => Ring::Spread,
            _ => return None,
        })
    }

    fn mode(self) -> u8 {
        match self {
            Ring::None | Ring::Dot => 0,
            Ring::BoostCut => 1,
            Ring::Wrap => 2,
            Ring::Spread => 3,
        }
    }
}

/// Where and how a mapping's value goes back
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feedback {
    pub message: Message,
    pub channel: u8,
    pub ring: Ring,
}

impl Feedback {
    /// The resolution of the value sent (127, 16383, or the ring's 11 positions)
    pub fn max(&self) -> u16 {
        match (self.ring, self.message) {
            (Ring::None, m) => m.max_value(),
            _ => 11,
        }
    }

    /// The control-space value 0..=1 quantized to what this feedback can show
    pub fn quantize(&self, x: f32) -> u16 {
        let max = self.max();
        match self.ring {
            Ring::None => (x.clamp(0.0, 1.0) * f32::from(max)).round() as u16,
            // Positions 1..=11; 0 would be all LEDs off
            _ => 1 + (x.clamp(0.0, 1.0) * 10.0).round() as u16,
        }
    }

    /// The messages that show `q` (from [`Feedback::quantize`]); at most four (NRPN)
    pub fn encode(&self, q: u16, mut out: impl FnMut([u8; 3], u8)) {
        let ch = self.channel & 0x0F;
        let mut send = |raw: Raw| {
            let (b, n) = raw.encode();
            out(b, n);
        };
        if self.ring != Ring::None {
            let number = match self.message {
                Message::Cc { number } => number,
                _ => return,
            };
            let center = if self.ring == Ring::BoostCut && q == 6 { 0x40 } else { 0 };
            send(Raw::Cc { channel: ch, number, value: center | (self.ring.mode() << 4) | (q.min(11) as u8) });
            return;
        }
        let v7 = (q.min(127)) as u8;
        let hi = ((q >> 7) & 0x7F) as u8;
        let lo = (q & 0x7F) as u8;
        match self.message {
            Message::Cc { number } => send(Raw::Cc { channel: ch, number, value: v7 }),
            Message::Cc14 { msb } => {
                send(Raw::Cc { channel: ch, number: msb, value: hi });
                send(Raw::Cc { channel: ch, number: msb + 32, value: lo });
            }
            Message::Nrpn { number } | Message::Rpn { number } => {
                let (m, l) = if matches!(self.message, Message::Nrpn { .. }) { (99, 98) } else { (101, 100) };
                send(Raw::Cc { channel: ch, number: m, value: ((number >> 7) & 0x7F) as u8 });
                send(Raw::Cc { channel: ch, number: l, value: (number & 0x7F) as u8 });
                send(Raw::Cc { channel: ch, number: 6, value: hi });
                send(Raw::Cc { channel: ch, number: 38, value: lo });
            }
            // An LED: on with full velocity, off with a note-on of velocity 0 (what controllers expect)
            Message::Note { key } => send(Raw::NoteOn { channel: ch, key, velocity: v7 }),
            Message::PitchBend => send(Raw::PitchBend { channel: ch, value: q.min(16383) }),
            Message::ChannelPressure => send(Raw::ChannelPressure { channel: ch, value: v7 }),
            Message::PolyPressure { key } => send(Raw::PolyPressure { channel: ch, key, value: v7 }),
            Message::ProgramChange => send(Raw::ProgramChange { channel: ch, program: v7 }),
        }
    }
}

/// The rate limit and the echo window of one control
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limiter {
    /// The value waiting to go out
    pending: Option<u16>,
    /// What the controller shows (sent by us, or where the hand put it)
    shown: Option<u16>,
    /// The target's last known value, for a resend
    known: Option<u16>,
    sent_at: Option<u64>,
    echo_until: u64,
    force: bool,
}

impl Limiter {
    /// The target's value changed (from anywhere)
    pub fn offer(&mut self, q: u16) {
        self.known = Some(q);
        self.pending = Some(q);
    }

    /// A value just came in from the source itself: the controller is there, and nothing goes back for `no_echo`
    pub fn input(&mut self, now: u64, q: u16, no_echo: u64) {
        self.shown = Some(q);
        self.known = Some(q);
        self.echo_until = now.saturating_add(no_echo);
        if self.pending == Some(q) {
            self.pending = None;
        }
    }

    /// Send the last known value again, even if the controller should already show it (project open, bank change,
    /// reconnect)
    pub fn resend(&mut self) {
        if let Some(k) = self.known {
            self.pending = Some(k);
            self.force = true;
        }
    }

    /// Forget what the controller shows (it was reconnected or the mapping changed target)
    pub fn reset(&mut self) {
        *self = Limiter { known: self.known, ..Limiter::default() };
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// The value to send now, if any
    pub fn poll(&mut self, now: u64, interval: u64) -> Option<u16> {
        let p = self.pending?;
        if now < self.echo_until && !self.force {
            if self.shown == Some(p) {
                self.pending = None; // the echo of what the hand did
            }
            return None; // a different value waits for the end of the window
        }
        if self.sent_at.is_some_and(|t| now < t.saturating_add(interval)) {
            return None;
        }
        self.pending = None;
        if !self.force && self.shown == Some(p) {
            return None;
        }
        self.force = false;
        self.shown = Some(p);
        self.sent_at = Some(now);
        Some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(f: &Feedback, q: u16) -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        f.encode(q, |b, n| v.push(b[..usize::from(n)].to_vec()));
        v
    }

    #[test]
    fn encodes_each_kind() {
        let f = |message| Feedback { message, channel: 2, ring: Ring::None };
        assert_eq!(bytes(&f(Message::Cc { number: 7 }), 100), vec![vec![0xB2, 7, 100]]);
        assert_eq!(bytes(&f(Message::Cc14 { msb: 1 }), 0x3FFF), vec![vec![0xB2, 1, 127], vec![0xB2, 33, 127]]);
        assert_eq!(
            bytes(&f(Message::Nrpn { number: 0x81 }), 0x0102),
            vec![vec![0xB2, 99, 1], vec![0xB2, 98, 1], vec![0xB2, 6, 2], vec![0xB2, 38, 2]]
        );
        assert_eq!(bytes(&f(Message::Note { key: 0x10 }), 0), vec![vec![0x92, 0x10, 0]]);
        assert_eq!(bytes(&f(Message::PitchBend), 8192), vec![vec![0xE2, 0, 64]]);
        let ring = Feedback { message: Message::Cc { number: 0x30 }, channel: 0, ring: Ring::BoostCut };
        assert_eq!(ring.quantize(0.5), 6);
        assert_eq!(bytes(&ring, 6), vec![vec![0xB0, 0x30, 0x40 | 0x10 | 6]]);
        assert_eq!(bytes(&ring, ring.quantize(1.0)), vec![vec![0xB0, 0x30, 0x10 | 11]]);
        assert_eq!(f(Message::PitchBend).quantize(1.0), 16383);
    }

    #[test]
    fn rate_limit_sends_the_latest() {
        let mut l = Limiter::default();
        l.offer(10);
        assert_eq!(l.poll(0, 10), Some(10));
        l.offer(11);
        l.offer(12);
        assert_eq!(l.poll(5, 10), None, "within 10");
        l.offer(13);
        assert_eq!(l.poll(10, 10), Some(13), "the latest, never the intermediate ones");
        assert_eq!(l.poll(30, 10), None);
        l.offer(13);
        assert_eq!(l.poll(40, 10), None, "the controller shows it already");
    }

    #[test]
    fn no_echo_window() {
        let mut l = Limiter::default();
        l.input(100, 50, 50);
        l.offer(50); // the renderer's echo of the hand's own value
        assert_eq!(l.poll(110, 10), None);
        assert!(!l.has_pending());
        // A different value (automation) inside the window waits for its end
        l.offer(60);
        assert_eq!(l.poll(120, 10), None);
        assert_eq!(l.poll(149, 10), None);
        assert_eq!(l.poll(150, 10), Some(60));
        // Resend forces even an unchanged value, and even inside an echo window
        l.resend();
        assert_eq!(l.poll(200, 10), Some(60));
    }
}
