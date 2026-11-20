//! The compiled mapping table (interfaces.md, I-21.2): built off the audio thread with everything allocated (the
//! entries, each mapping's state, the 14-bit and NRPN state of every port and channel, the feedback limiters), then
//! fed raw messages on the audio thread without allocating. Generic over the target `T`, which the renderer makes a
//! parameter slot and the session a mapping id.

use crate::feedback::{Feedback, Limiter};
use crate::message::{Message, Raw};
use crate::transform::{Input, MapState, Output, Transform};

/// A source on any port
pub const ANY_PORT: u16 = u16::MAX;

/// Where a mapping listens: a port index (or [`ANY_PORT`]), a channel (or any) and a message
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub port: u16,
    pub channel: Option<u8>,
    pub message: Message,
}

impl Source {
    fn matches(&self, port: u16, channel: u8) -> bool {
        (self.port == ANY_PORT || self.port == port) && self.channel.is_none_or(|c| c == channel)
    }
}

/// One compiled mapping
#[derive(Clone, Debug, PartialEq)]
pub struct Entry<T> {
    pub source: Source,
    pub transform: Transform,
    /// The control is a button (a key; an on/off controller on a switch or an action): presses, not positions
    pub switch: bool,
    /// The message does not also reach the instruments
    pub consume: bool,
    /// Where the target's value goes back, if anywhere (needs a port, not [`ANY_PORT`])
    pub feedback: Option<Feedback>,
    /// A touch source (a motor fader's touch note): on while held
    pub touch: Option<Source>,
    pub target: T,
}

/// What a message did to one mapping
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A new target value (normalized), from entry `entry`
    Value { entry: usize, out: Output },
    /// The touch source of entry `entry` went down or up
    Touch { entry: usize, on: bool },
}

/// The running 14-bit and parameter-number state of one port and channel
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChannelState {
    /// The last MSB of each of CC 0..=31 (`NONE` = not seen)
    msb: [u8; 32],
    /// CC 99/101 seen, waiting for 98/100: (rpn, msb)
    select_msb: Option<(bool, u8)>,
    /// The selected parameter: (rpn, number)
    param: Option<(bool, u16)>,
    data_msb: u8,
}

const NONE: u8 = 0xFF;

impl Default for ChannelState {
    fn default() -> Self {
        ChannelState { msb: [NONE; 32], select_msb: None, param: None, data_msb: 0 }
    }
}

struct Readings<'a> {
    out: &'a mut [(Message, Input); MAX_READINGS],
    n: usize,
}

impl Readings<'_> {
    fn push(&mut self, m: Message, i: Input) {
        if self.n < MAX_READINGS {
            self.out[self.n] = (m, i);
            self.n += 1;
        }
    }
}

/// Up to this many readings of one raw message (CC n as itself, as a 14-bit half, as NRPN data)
const MAX_READINGS: usize = 4;

#[derive(Debug)]
pub struct MapTable<T> {
    entries: Box<[Entry<T>]>,
    states: Box<[MapState]>,
    limiters: Box<[Limiter]>,
    /// (message key, entry), sorted
    order: Box<[(u32, u32)]>,
    /// (touch message key, entry), sorted
    touches: Box<[(u32, u32)]>,
    /// `ports × 16`
    channels: Box<[ChannelState]>,
    ports: u16,
    interval: u64,
    no_echo: u64,
}

/// The feedback timing, in the caller's time unit
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// At most one value per control in this time (10 ms)
    pub interval: u64,
    /// A value that came from the source does not go back to it for this time (50 ms)
    pub no_echo: u64,
}

impl Timing {
    /// 10 ms and 50 ms in samples at `rate`
    pub fn samples(rate: u32) -> Timing {
        Timing { interval: u64::from(rate) / 100, no_echo: u64::from(rate) / 20 }
    }

    /// 10 ms and 50 ms in nanoseconds
    pub fn nanos() -> Timing {
        Timing { interval: 10_000_000, no_echo: 50_000_000 }
    }
}

impl<T> MapTable<T> {
    /// Builds the table (allocates). `ports` is how many port indices exist (each gets 16 channels of 14-bit and
    /// NRPN state); a message from a port at or above it still matches, without 14-bit pairing or NRPN
    pub fn new(entries: Vec<Entry<T>>, ports: u16, timing: Timing) -> MapTable<T> {
        let mut order: Vec<(u32, u32)> =
            entries.iter().enumerate().map(|(i, e)| (e.source.message.key(), i as u32)).collect();
        order.sort_unstable();
        let mut touches: Vec<(u32, u32)> =
            entries.iter().enumerate().filter_map(|(i, e)| e.touch.map(|t| (t.message.key(), i as u32))).collect();
        touches.sort_unstable();
        let n = entries.len();
        MapTable {
            entries: entries.into_boxed_slice(),
            states: vec![MapState::default(); n].into_boxed_slice(),
            limiters: vec![Limiter::default(); n].into_boxed_slice(),
            order: order.into_boxed_slice(),
            touches: touches.into_boxed_slice(),
            channels: vec![ChannelState::default(); usize::from(ports) * 16].into_boxed_slice(),
            ports,
            interval: timing.interval,
            no_echo: timing.no_echo,
        }
    }

    pub fn empty() -> MapTable<T> {
        MapTable::new(Vec::new(), 0, Timing { interval: 0, no_echo: 0 })
    }

    pub fn entries(&self) -> &[Entry<T>] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn state(&self, entry: usize) -> &MapState {
        &self.states[entry]
    }

    /// Carries the running state (pickup, toggles, 14-bit halves, feedback) of the entries that are the same mapping
    /// from an older table: `same(old, new)` says whether two targets are the same mapping
    pub fn inherit(&mut self, old: &MapTable<T>, mut same: impl FnMut(&Entry<T>, &Entry<T>) -> bool) {
        for (i, e) in self.entries.iter().enumerate() {
            if let Some(j) = old.entries.iter().position(|o| o.source == e.source && same(o, e)) {
                self.states[i] = old.states[j];
                self.limiters[i] = old.limiters[j];
            }
        }
        let n = self.channels.len().min(old.channels.len());
        self.channels[..n].copy_from_slice(&old.channels[..n]);
    }

    /// One raw message from `port` at time `now`. `current(target)` gives each target's normalized value now (for
    /// pickup, toggles and relative steps). Every result goes to `on`. Returns whether a consuming mapping (or a
    /// touch source) took the message. Never allocates
    pub fn feed(
        &mut self,
        port: u16,
        bytes: &[u8],
        now: u64,
        mut current: impl FnMut(&T) -> f32,
        mut on: impl FnMut(Event, &T),
    ) -> bool {
        let Some(raw) = Raw::parse(bytes) else { return false };
        let channel = raw.channel();
        let mut readings = [(Message::PitchBend, Input::Relative { steps: 0 }); MAX_READINGS];
        let count = self.read(port, raw, &mut readings);
        let mut consumed = false;

        // Touch sources first: a fader touched and moved in the same block must already count as touched
        let touch = match raw {
            Raw::NoteOn { key, .. } => Some((Message::Note { key }.key(), true)),
            Raw::NoteOff { key, .. } => Some((Message::Note { key }.key(), false)),
            Raw::Cc { number, value, .. } => Some((Message::Cc { number }.key(), value >= 64)),
            _ => None,
        };
        if let Some((key, down)) = touch {
            let start = self.touches.partition_point(|(k, _)| *k < key);
            for &(k, i) in &self.touches[start..] {
                if k != key {
                    break;
                }
                let i = i as usize;
                let e = &self.entries[i];
                if e.touch.is_some_and(|t| t.matches(port, channel)) {
                    consumed = true;
                    on(Event::Touch { entry: i, on: down }, &e.target);
                }
            }
        }

        for &(message, input) in &readings[..count] {
            let key = message.key();
            let start = self.order.partition_point(|(k, _)| *k < key);
            for &(k, i) in &self.order[start..] {
                if k != key {
                    break;
                }
                let i = i as usize;
                let e = &self.entries[i];
                if !e.source.matches(port, channel) {
                    continue;
                }
                consumed |= e.consume;
                let input = if e.switch {
                    match input {
                        Input::Absolute { value, .. } if message == Message::ProgramChange => {
                            // Every program change fires: a press without a release
                            self.states[i].pressed = false;
                            Input::Press { down: true, value }
                        }
                        Input::Absolute { value, max } => Input::Press { down: value > max / 2, value },
                        other => other,
                    }
                } else {
                    match input {
                        // A key on a continuous target: its velocity is the position, the release does nothing
                        Input::Press { down: true, value } => Input::Absolute { value, max: 127 },
                        Input::Press { down: false, .. } => continue,
                        other => other,
                    }
                };
                let cur = current(&e.target);
                if let Some(out) = self.states[i].apply(&e.transform, input, cur) {
                    if let Some(fb) = e.feedback {
                        let q = fb.quantize(e.transform.inverse(out.value));
                        self.limiters[i].input(now, q, self.no_echo);
                    }
                    on(Event::Value { entry: i, out }, &e.target);
                }
            }
        }
        consumed
    }

    /// The readings of one raw message, updating the port's channel state
    fn read(&mut self, port: u16, raw: Raw, out: &mut [(Message, Input); MAX_READINGS]) -> usize {
        let mut r = Readings { out, n: 0 };
        let a7 = |v: u8| Input::Absolute { value: u16::from(v), max: 127 };
        let a14 = |v: u16| Input::Absolute { value: v, max: 16383 };
        match raw {
            Raw::NoteOn { key, velocity, .. } => {
                r.push(Message::Note { key }, Input::Press { down: true, value: u16::from(velocity) })
            }
            Raw::NoteOff { key, velocity, .. } => {
                r.push(Message::Note { key }, Input::Press { down: false, value: u16::from(velocity) })
            }
            Raw::PolyPressure { key, value, .. } => r.push(Message::PolyPressure { key }, a7(value)),
            Raw::ProgramChange { program, .. } => r.push(Message::ProgramChange, a7(program)),
            Raw::ChannelPressure { value, .. } => r.push(Message::ChannelPressure, a7(value)),
            Raw::PitchBend { value, .. } => r.push(Message::PitchBend, a14(value)),
            Raw::Cc { channel, number, value } => {
                r.push(Message::Cc { number }, a7(value));
                if port >= self.ports {
                    return r.n;
                }
                let st = &mut self.channels[usize::from(port) * 16 + usize::from(channel)];
                match number {
                    0..=31 => {
                        st.msb[usize::from(number)] = value;
                        r.push(Message::Cc14 { msb: number }, a14(u16::from(value) << 7));
                    }
                    32..=63 => {
                        let msb = st.msb[usize::from(number - 32)];
                        if msb != NONE {
                            r.push(Message::Cc14 { msb: number - 32 }, a14((u16::from(msb) << 7) | u16::from(value)));
                        }
                    }
                    _ => {}
                }
                let param =
                    st.param.map(|(rpn, number)| if rpn { Message::Rpn { number } } else { Message::Nrpn { number } });
                match number {
                    99 | 101 => st.select_msb = Some((number == 101, value)),
                    98 | 100 => {
                        let rpn = number == 100;
                        let msb = match st.select_msb {
                            Some((r, m)) if r == rpn => m,
                            _ => 0,
                        };
                        st.select_msb = None;
                        st.param = if rpn && msb == 127 && value == 127 {
                            None // the null RPN deselects
                        } else {
                            Some((rpn, (u16::from(msb) << 7) | u16::from(value)))
                        };
                    }
                    6 => {
                        st.data_msb = value;
                        if let Some(m) = param {
                            r.push(m, a14(u16::from(value) << 7));
                        }
                    }
                    38 => {
                        if let Some(m) = param {
                            r.push(m, a14((u16::from(st.data_msb) << 7) | u16::from(value)));
                        }
                    }
                    96 | 97 => {
                        if let Some(m) = param {
                            r.push(m, Input::Relative { steps: if number == 96 { 1 } else { -1 } });
                        }
                    }
                    _ => {}
                }
            }
        }
        r.n
    }

    /// The target of entry `entry` has a new normalized value (from the hand, automation, undo, another control):
    /// queue it for feedback
    pub fn offer(&mut self, entry: usize, value: f32) {
        let e = &self.entries[entry];
        if let Some(fb) = e.feedback {
            self.limiters[entry].offer(fb.quantize(e.transform.inverse(value)));
        }
    }

    /// Queue every known value again (project open, bank change, reconnect). `port`: only the entries of that port
    pub fn resend(&mut self, port: Option<u16>) {
        for (e, l) in self.entries.iter().zip(self.limiters.iter_mut()) {
            if port.is_none_or(|p| e.source.port == p) {
                l.resend();
            }
        }
    }

    /// Forget what the controllers on `port` show (it was reconnected)
    pub fn reset_port(&mut self, port: u16) {
        for (e, l) in self.entries.iter().zip(self.limiters.iter_mut()) {
            if e.source.port == port {
                l.reset();
            }
        }
    }

    /// Sends what is due at `now`: `out(port, bytes, len)` for each message. Never allocates
    pub fn poll_feedback(&mut self, now: u64, mut out: impl FnMut(u16, [u8; 3], u8)) {
        for (e, l) in self.entries.iter().zip(self.limiters.iter_mut()) {
            if !l.has_pending() {
                continue;
            }
            let Some(fb) = e.feedback else { continue };
            if e.source.port == ANY_PORT {
                continue;
            }
            if let Some(q) = l.poll(now, self.interval) {
                fb.encode(q, |b, n| out(e.source.port, b, n));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::Ring;
    use crate::transform::{Button, Encoding, Mode, Takeover};

    fn entry(message: Message, target: u32) -> Entry<u32> {
        Entry {
            source: Source { port: 0, channel: Some(0), message },
            transform: Transform { takeover: Takeover::Jump, ..Transform::default() },
            switch: message.is_note(),
            consume: true,
            feedback: None,
            touch: None,
            target,
        }
    }

    fn run(t: &mut MapTable<u32>, msgs: &[[u8; 3]]) -> Vec<(u32, Event)> {
        let mut v = Vec::new();
        for m in msgs {
            let len = crate::message::channel_message_len(m[0]) as usize;
            t.feed(0, &m[..len], 0, |_| 0.0, |e, tg| v.push((*tg, e)));
        }
        v
    }

    fn values(evs: &[(u32, Event)]) -> Vec<(u32, i32)> {
        evs.iter()
            .filter_map(|(t, e)| match e {
                Event::Value { out, .. } => Some((*t, out.raw)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn fourteen_bit_pairs() {
        let mut t = MapTable::new(
            vec![entry(Message::Cc14 { msb: 1 }, 1), entry(Message::Cc { number: 1 }, 2)],
            1,
            Timing::nanos(),
        );
        let ev = run(&mut t, &[[0xB0, 1, 0x40], [0xB0, 33, 0x01]]);
        // The MSB alone gives the coarse value at once; the LSB refines it; the 7-bit mapping sees only the MSB
        assert_eq!(values(&ev), vec![(2, 0x40), (1, 0x2000), (1, 0x2001)]);
        let Event::Value { out, .. } = ev[2].1 else { panic!() };
        assert!((out.value - 0x2001 as f32 / 16383.0).abs() < 1e-6);
        // An LSB without an MSB seen is not a 14-bit value
        let mut t = MapTable::new(vec![entry(Message::Cc14 { msb: 2 }, 1)], 1, Timing::nanos());
        assert!(values(&run(&mut t, &[[0xB0, 34, 5]])).is_empty());
    }

    #[test]
    fn nrpn_and_rpn() {
        let mut t = MapTable::new(
            vec![entry(Message::Nrpn { number: 0x0102 }, 1), entry(Message::Rpn { number: 0 }, 2)],
            1,
            Timing::nanos(),
        );
        let ev = run(&mut t, &[[0xB0, 99, 2], [0xB0, 98, 2], [0xB0, 6, 0x10], [0xB0, 38, 0x05]]);
        assert_eq!(values(&ev), vec![(1, 0x10 << 7), (1, (0x10 << 7) | 5)]);
        // Increment/decrement on the selected parameter
        let mut t2 = MapTable::new(
            vec![Entry {
                transform: Transform { step: 0.25, ..Transform::default() },
                ..entry(Message::Nrpn { number: 0x0102 }, 1)
            }],
            1,
            Timing::nanos(),
        );
        let mut v = Vec::new();
        for m in [[0xB0u8, 99, 2], [0xB0, 98, 2], [0xB0, 96, 0]] {
            t2.feed(0, &m, 0, |_| 0.5, |e, _| v.push(e));
        }
        let Event::Value { out, .. } = v[0] else { panic!() };
        assert!((out.value - 0.75).abs() < 1e-6);
        // RPN 0 (pitch bend range), then the null RPN deselects
        let ev = run(
            &mut t,
            &[[0xB0, 101, 0], [0xB0, 100, 0], [0xB0, 6, 2], [0xB0, 101, 127], [0xB0, 100, 127], [0xB0, 6, 3]],
        );
        assert_eq!(values(&ev), vec![(2, 2 << 7)]);
        // A different port does not share the selection
        let mut v = Vec::new();
        t.feed(0, &[0xB0, 99, 2], 0, |_| 0.0, |_, _| {});
        t.feed(0, &[0xB0, 98, 2], 0, |_| 0.0, |_, _| {});
        t.feed(3, &[0xB0, 6, 1], 0, |_| 0.0, |e, _| v.push(e));
        assert!(v.is_empty());
    }

    #[test]
    fn ports_channels_and_consume() {
        let mut e = entry(Message::Cc { number: 7 }, 1);
        e.source.port = ANY_PORT;
        e.source.channel = None;
        let mut f = entry(Message::Cc { number: 7 }, 2);
        f.source.port = 1;
        f.consume = false;
        let mut t = MapTable::new(vec![e, f], 2, Timing::nanos());
        let mut seen = Vec::new();
        assert!(t.feed(1, &[0xB5, 7, 1], 0, |_| 0.0, |_, tg| seen.push(*tg)));
        assert_eq!(seen, vec![1]); // channel 5 is not f's channel 0
        seen.clear();
        assert!(t.feed(1, &[0xB0, 7, 1], 0, |_| 0.0, |_, tg| seen.push(*tg)));
        assert_eq!(seen, vec![1, 2]);
        let mut t = MapTable::new(
            vec![entry(Message::Cc { number: 7 }, 2)]
                .into_iter()
                .map(|mut e| {
                    e.consume = false;
                    e
                })
                .collect(),
            1,
            Timing::nanos(),
        );
        assert!(!t.feed(0, &[0xB0, 7, 1], 0, |_| 0.0, |_, _| {}));
        assert!(!t.feed(0, &[0xB0, 8, 1], 0, |_| 0.0, |_, _| {}));
    }

    #[test]
    fn switches_and_program_change() {
        let mut cc = entry(Message::Cc { number: 64 }, 1);
        cc.switch = true;
        cc.transform.button = Button::Toggle;
        let mut pc = entry(Message::ProgramChange, 2);
        pc.switch = true;
        pc.transform.button = Button::Trigger;
        let mut t = MapTable::new(vec![cc, pc], 1, Timing::nanos());
        let ev = run(&mut t, &[[0xB0, 64, 127], [0xB0, 64, 0], [0xB0, 64, 127], [0xC0, 5, 0], [0xC0, 9, 0]]);
        assert_eq!(values(&ev), vec![(1, 127), (1, 127), (2, 5), (2, 9)]);
        // A note on a continuous target: velocity is the position
        let mut note = entry(Message::Note { key: 60 }, 3);
        note.switch = false;
        let mut t = MapTable::new(vec![note], 1, Timing::nanos());
        assert_eq!(values(&run(&mut t, &[[0x90, 60, 64], [0x80, 60, 0]])), vec![(3, 64)]);
    }

    #[test]
    fn touch_sources() {
        let mut e = entry(Message::PitchBend, 1);
        e.touch = Some(Source { port: 0, channel: Some(0), message: Message::Note { key: 0x68 } });
        let mut t = MapTable::new(vec![e], 1, Timing::nanos());
        let ev = run(&mut t, &[[0x90, 0x68, 127], [0xE0, 0, 64], [0x90, 0x68, 0]]);
        assert!(matches!(ev[0].1, Event::Touch { entry: 0, on: true }));
        assert!(matches!(ev[1].1, Event::Value { .. }));
        assert!(matches!(ev[2].1, Event::Touch { entry: 0, on: false }));
    }

    #[test]
    fn feedback_with_echo_and_rate() {
        let mut e = entry(Message::Cc { number: 7 }, 1);
        e.feedback = Some(Feedback { message: Message::Cc { number: 7 }, channel: 0, ring: Ring::None });
        let mut t = MapTable::new(vec![e], 1, Timing { interval: 10, no_echo: 50 });
        let mut sent = Vec::new();
        // The hand moves the control at t=100; the renderer reports the same value: no echo
        t.feed(0, &[0xB0, 7, 100], 100, |_| 0.0, |_, _| {});
        t.offer(0, 100.0 / 127.0);
        t.poll_feedback(110, |p, b, n| sent.push((p, b, n)));
        assert!(sent.is_empty());
        // Automation moves it after the window
        t.offer(0, 0.0);
        t.poll_feedback(160, |p, b, n| sent.push((p, b, n)));
        assert_eq!(sent, vec![(0, [0xB0, 7, 0], 3)]);
        t.resend(None);
        t.poll_feedback(170, |p, b, n| sent.push((p, b, n)));
        assert_eq!(sent.len(), 2);
    }

    #[test]
    fn relative_on_a_table() {
        let mut e = entry(Message::Cc { number: 16 }, 1);
        e.transform.mode = Mode::Relative(Encoding::SignMagnitude);
        e.transform.step = 0.01;
        let mut t = MapTable::new(vec![e], 1, Timing::nanos());
        let mut v = Vec::new();
        t.feed(0, &[0xB0, 16, 0x43], 0, |_| 0.5, |ev, _| v.push(ev));
        let Event::Value { out, .. } = v[0] else { panic!() };
        assert!((out.value - 0.47).abs() < 1e-6);
    }
}
