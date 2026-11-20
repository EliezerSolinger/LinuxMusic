//! MIDI mapping in the renderer (interfaces.md, I-21.2, I-21.5, I-21.6): a mapped CC reaches the processor at its
//! sample with the fixed one-block delay, under the allocation trap; the slot is touched and the hand wins over
//! automation, comes back to the curve in 50 ms; Latch and Write; the map alone is replaced; fields and actions go
//! to the session; recording; a later stamp does not hold back the ring
#![cfg(not(loom))]

mod common;

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use common::*;
use lm_base::HostTime;
use lm_midimap::{Button, Entry, Message, Mode, Source, Takeover, Transform};
use lm_render::*;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// A fixed record of `Param` events, written by the render thread without a lock or an allocation
struct Record {
    frames: Box<[AtomicU64]>,
    values: Box<[AtomicU32]>,
    len: AtomicUsize,
}

impl Record {
    fn new(n: usize) -> Arc<Record> {
        Arc::new(Record {
            frames: (0..n).map(|_| AtomicU64::new(0)).collect(),
            values: (0..n).map(|_| AtomicU32::new(0)).collect(),
            len: AtomicUsize::new(0),
        })
    }

    fn all(&self) -> Vec<(u64, u32, f32)> {
        let n = self.len.load(Ordering::Acquire);
        (0..n)
            .map(|i| {
                (self.frames[i].load(Ordering::Relaxed), 0, f32::from_bits(self.values[i].load(Ordering::Relaxed)))
            })
            .collect()
    }
}

/// Records every `Param` event it gets, with the engine frame it lands on
struct Probe {
    seen: Seen,
}

impl Processor for Probe {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frame = cx.transport.frame;
        for e in cx.events {
            if let EventKind::Param { value, .. } = e.kind {
                let i = self.seen.len.load(Ordering::Relaxed);
                if i < self.seen.frames.len() {
                    self.seen.frames[i].store(frame + u64::from(e.offset), Ordering::Relaxed);
                    self.seen.values[i].store(value.to_bits(), Ordering::Relaxed);
                    self.seen.len.store(i + 1, Ordering::Release);
                }
            }
        }
        for ch in 0..cx.buffers.outputs() {
            cx.buffers.output(ch).fill(0.0);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

type Seen = Arc<Record>;

const SLOT: ParamSlot = ParamSlot(9);
const RANGE: ParamRange = ParamRange { min: 0.0, max: 100.0, scale: RangeScale::Linear };

fn cc_entry(id: u64, number: u8, target: MapTarget) -> Entry<MapTarget> {
    Entry {
        id,
        source: Source { port: 0, channel: None, message: Message::Cc { number } },
        transform: Transform { takeover: Takeover::Jump, ..Transform::default() },
        switch: false,
        consume: true,
        feedback: None,
        touch: None,
        target,
    }
}

/// A probe node with slot 9 bound to its parameter 0, optionally automated with `mode`
fn setup(curve: Option<(Curve, AutomationMode)>) -> (EngineControl, Dummy, Seen) {
    let (mut control, dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    b.node(NodeDesc::insert(n(1), 2));
    b.feed(Feed::new(n(1), m, 2));
    b.port(PortRoute { port: PortIndex(0), channel: None, node: n(1) });
    let automated = curve.as_ref().map(|(c, _)| c.clone());
    b.param(SLOT, n(1), 0, ParamInfo { rate: Rate::Block, discrete: false, initial: 0.0 }, automated);
    if let Some((_, mode)) = curve {
        b.automation_mode(SLOT, mode);
    }
    let seen: Seen = Record::new(1 << 16);
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), core(Probe { seen: seen.clone() }));
    i.set_map(MidiMap::new(vec![cc_entry(1, 7, MapTarget::Param { slot: SLOT, range: RANGE })], 4, spec()));
    install(&mut control, i);
    (control, dummy, seen)
}

fn send(control: &mut EngineControl, at: HostTime, bytes: [u8; 3]) {
    control.commands().send(Command::Midi { port: PortIndex(0), at, bytes }).expect("midi");
}

fn host_at(dummy: &Dummy, samples: u64) -> HostTime {
    HostTime(dummy.host_time().0 + samples * 1_000_000_000 / u64::from(RATE))
}

fn last_param(seen: &Seen) -> Option<(u64, u32, f32)> {
    seen.all().last().copied()
}

/// I-21.6: a mapped CC reaches the processor at the right sample, with the fixed one-block delay, under the trap
#[test]
fn a_mapped_cc_lands_at_its_sample_plus_one_block() {
    let (mut control, mut dummy, seen) = setup(None);
    block(&mut dummy, 256);
    let start = 256u64; // the engine frame of the next block
    let at = host_at(&dummy, 100);
    send(&mut control, at, [0xB0, 7, 127]);
    for _ in 0..8 {
        block(&mut dummy, 256);
    }
    let hits: Vec<_> = seen.all().into_iter().filter(|e| e.2 == 100.0).collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    let frame = hits[0].0;
    let expected = start + 100 + MAX_BLOCK as u64;
    assert!(frame.abs_diff(expected) <= 1, "at {frame}, expected {expected}");
    // The slot shows the value (the frame and the slot), and the session hears of it
    assert_eq!(control.params().read(SLOT), 100.0);
    let mut events = Vec::new();
    control.map_events(|e| events.push(e));
    assert!(matches!(events[0], MapEvent::Param { mapping: 1, slot: SLOT, value: 100.0, .. }), "{events:?}");
    // 500 ms after the last message the gesture ends (the session writes the document then)
    for _ in 0..(RATE as usize / 2 / 256 + 2) {
        block(&mut dummy, 256);
    }
    let mut events = Vec::new();
    control.map_events(|e| events.push(e));
    assert!(events.iter().any(|e| matches!(e, MapEvent::Released { slot: SLOT, value: 100.0, .. })), "{events:?}");
}

/// The hand wins over the curve while it moves, and the curve comes back in 50 ms, not at once
#[test]
fn the_hand_wins_and_the_curve_returns_in_50_ms() {
    let curve = Curve::new(vec![CurvePoint { sample: 0, value: 20.0, shape: 0.0 }]).unwrap();
    let (mut control, mut dummy, seen) = setup(Some((curve, AutomationMode::Read)));
    control.commands().send(Command::Play).expect("play");
    for _ in 0..4 {
        block(&mut dummy, 256);
    }
    assert_eq!(last_param(&seen).map(|e| e.2), Some(20.0));
    send(&mut control, host_at(&dummy, 0), [0xB0, 7, 127]);
    for _ in 0..8 {
        block(&mut dummy, 256);
    }
    assert_eq!(last_param(&seen).map(|e| e.2), Some(100.0), "the hand won");
    let mut touched = Vec::new();
    control.touched(|t| touched.push(t));
    assert!(touched.iter().any(|t| t.slot == SLOT && t.value == 100.0 && t.touching), "{touched:?}");
    // Wait for the release (500 ms), then watch the return: values between 100 and 20 for 50 ms
    let release_blocks = RATE as usize / 2 / 256 + 1;
    for _ in 0..release_blocks {
        block(&mut dummy, 256);
    }
    for _ in 0..20 {
        block(&mut dummy, 256);
    }
    let all = seen.all();
    let hand = all.iter().rposition(|e| e.2 == 100.0).expect("the hand's value");
    let after: Vec<(u64, u32, f32)> = all[hand..].to_vec();
    let between: Vec<_> = after.iter().filter(|e| e.2 > 20.5 && e.2 < 99.5).collect();
    assert!(!between.is_empty(), "no ramp: {after:?}");
    let first = between.first().unwrap().0;
    let last_mid = between.last().unwrap().0;
    let span = last_mid - first;
    assert!(span <= u64::from(RATE / 20) && span >= u64::from(RATE / 20) - 64, "ramp of {span} samples: {between:?}");
    assert_eq!(after.last().unwrap().2, 20.0, "back on the curve");
    let mut touched = Vec::new();
    control.touched(|t| touched.push(t));
    assert!(touched.iter().any(|t| t.slot == SLOT && !t.touching), "the release reaches the session");
}

/// Latch holds the last value until the transport stops; Write never plays the curve while playing
#[test]
fn latch_and_write() {
    let curve = || Curve::new(vec![CurvePoint { sample: 0, value: 20.0, shape: 0.0 }]).unwrap();
    let (mut control, mut dummy, seen) = setup(Some((curve(), AutomationMode::Latch)));
    control.commands().send(Command::Play).expect("play");
    block(&mut dummy, 256);
    send(&mut control, host_at(&dummy, 0), [0xB0, 7, 127]);
    for _ in 0..(RATE as usize / 256 * 2) {
        block(&mut dummy, 256);
    }
    assert_eq!(last_param(&seen).map(|e| e.2), Some(100.0), "latched long after the gesture ended");
    control.commands().send(Command::Stop).expect("stop");
    for _ in 0..4 {
        block(&mut dummy, 256);
    }
    assert_eq!(last_param(&seen).map(|e| e.2), Some(20.0), "the stop lets go");

    let (mut control, mut dummy, seen) = setup(Some((curve(), AutomationMode::Write)));
    control.param_write(SLOT, 55.0);
    block(&mut dummy, 256);
    control.commands().send(Command::Play).expect("play");
    for _ in 0..8 {
        block(&mut dummy, 256);
    }
    assert!(seen.all().iter().all(|e| e.2 != 20.0 || e.0 < 256), "the curve played in Write");
    let mut touched = Vec::new();
    control.touched(|t| touched.push(t));
    assert!(touched.iter().any(|t| t.slot == SLOT && t.touching), "a Write pass reports its start: {touched:?}");
}

/// Fields and actions go to the session; a consumed note does not play the instrument; the map alone is replaced
/// and comes back; toggles keep their state across the swap
#[test]
fn session_targets_and_map_swaps() {
    let (mut control, mut dummy, _seen) = setup(None);
    let mut note = cc_entry(5, 0, MapTarget::Session);
    note.source.message = Message::Note { key: 36 };
    note.switch = true;
    note.transform.button = Button::Toggle;
    install(&mut control, Install::map_only(ProgramGen(1), MidiMap::new(vec![note.clone()], 4, spec())));
    block(&mut dummy, 64);
    let returned = collect(&mut control);
    assert!(returned.iter().any(|r| r.map.is_some()), "the old map comes back");
    send(&mut control, host_at(&dummy, 0), [0x90, 36, 100]);
    for _ in 0..(MAX_BLOCK / 64 + 2) {
        block(&mut dummy, 64);
    }
    let mut events = Vec::new();
    control.map_events(|e| events.push(e));
    assert!(matches!(events[..], [MapEvent::Session { mapping: 5, press: Some(true), raw: 100, .. }]), "{events:?}");
    // The key is still held when the map is replaced: its release after the swap is not a new press
    let mut relative = cc_entry(6, 16, MapTarget::Session);
    relative.transform.mode = Mode::Relative(lm_midimap::Encoding::SignMagnitude);
    install(&mut control, Install::map_only(ProgramGen(1), MidiMap::new(vec![note, relative], 4, spec())));
    block(&mut dummy, 64);
    send(&mut control, host_at(&dummy, 0), [0x80, 36, 0]);
    send(&mut control, host_at(&dummy, 0), [0xB0, 16, 0x42]);
    for _ in 0..(MAX_BLOCK / 64 + 2) {
        block(&mut dummy, 64);
    }
    let mut events = Vec::new();
    control.map_events(|e| events.push(e));
    assert!(matches!(events[..], [MapEvent::Session { mapping: 6, raw: -2, .. }]), "{events:?}");
}

/// I-21.4: while recording, what reached the instruments is recorded at its timeline sample; mapped (consumed)
/// messages are not
#[test]
fn recording_keeps_the_played_messages() {
    let (mut control, mut dummy, _seen) = setup(None);
    control.commands().send(Command::Record { on: true }).expect("record");
    control.commands().send(Command::Play).expect("play");
    block(&mut dummy, 256);
    let position = 256i64;
    send(&mut control, host_at(&dummy, 10), [0x90, 60, 90]);
    send(&mut control, host_at(&dummy, 20), [0xB0, 7, 1]); // mapped: not recorded
    send(&mut control, host_at(&dummy, 30), [0xE0, 0, 64]);
    for _ in 0..8 {
        block(&mut dummy, 256);
    }
    let mut rec = Vec::new();
    control.recorded_midi(|r| rec.push(r));
    assert_eq!(rec.len(), 2, "{rec:?}");
    assert_eq!(rec[0].bytes, [0x90, 60, 90]);
    assert_eq!(rec[0].sample, position + 10 + MAX_BLOCK as i64);
    assert_eq!(rec[1].sample - rec[0].sample, 20);
    assert_eq!(rec[1].len, 3);
}

/// A message stamped later (a port with less latency subtracted) does not hold back the ring behind it
#[test]
fn a_later_stamp_does_not_hold_back_the_ring() {
    let (mut control, mut dummy, seen) = setup(None);
    block(&mut dummy, 256);
    send(&mut control, host_at(&dummy, 3000), [0xB0, 7, 64]);
    send(&mut control, host_at(&dummy, 10), [0xB0, 7, 127]);
    let mut frames = Vec::new();
    for _ in 0..24 {
        block(&mut dummy, 256);
    }
    for e in seen.all().iter() {
        frames.push((e.0, e.2));
    }
    let early = frames.iter().find(|e| e.1 == 100.0).expect("the early one played").0;
    assert!(early.abs_diff(256 + 10 + MAX_BLOCK as u64) <= 1, "{frames:?}");
    let late = frames.iter().find(|e| (e.1 - 64.0 / 127.0 * 100.0).abs() < 1e-3).expect("the late one played").0;
    assert!(late.abs_diff(256 + 3000 + MAX_BLOCK as u64) <= 1, "{frames:?}");
}
