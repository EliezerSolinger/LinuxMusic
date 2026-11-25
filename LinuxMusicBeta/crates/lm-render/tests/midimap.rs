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
    b.port(PortRoute { port: PortIndex(0), channel: None, node: n(1), fixed_velocity: None });
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

/// I-21.6: recording and playing at once on 64 tracks (64 automated parameters playing their curves, 64 MIDI
/// controls moving others and their touched values going to the session), without allocating on the audio thread
#[test]
fn sixty_four_tracks_play_and_record_at_once_without_allocating() {
    let (mut control, dummy) = make(2);
    let mut dummy = dummy;
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(2), tempo(120));
    let m = master(&mut b);
    let mut entries = Vec::new();
    let mut cores = Vec::new();
    for i in 0..64u32 {
        let node = n(10 + i);
        b.node(NodeDesc::insert(node, 2));
        b.feed(Feed::new(node, m, 2));
        let curve = Curve::new(vec![
            CurvePoint { sample: 0, value: 0.0, shape: 0.0 },
            CurvePoint { sample: 48_000, value: 1.0, shape: 0.3 },
        ])
        .unwrap();
        let automated = ParamSlot(100 + 2 * i);
        let mapped = ParamSlot(101 + 2 * i);
        let info = ParamInfo { rate: Rate::Block, discrete: false, initial: 0.0 };
        b.param(automated, node, 0, info, Some(curve));
        b.param(mapped, node, 1, info, None);
        cores.push((node, core(Probe { seen: Record::new(1 << 12) })));
        let mut e = cc_entry(u64::from(i), (i % 120) as u8, MapTarget::Param { slot: mapped, range: RANGE });
        e.source.channel = Some((i / 120) as u8);
        entries.push(e);
    }
    let mut install_msg = Install::new(b.build().expect("program"));
    for (node, c) in cores {
        install_msg.add(node, c);
    }
    install_msg.set_map(MidiMap::new(entries, 4, spec()));
    install(&mut control, install_msg);
    control.commands().send(Command::Record { on: true }).expect("record");
    control.commands().send(Command::Play).expect("play");
    let mut touched = 0usize;
    for round in 0..40u8 {
        for i in 0..64u8 {
            send(&mut control, host_at(&dummy, 0), [0xB0, i, round.wrapping_mul(3) & 0x7F]);
        }
        for _ in 0..4 {
            block(&mut dummy, 256);
        }
        control.touched(|_| touched += 1);
        control.map_events(|_| {});
    }
    assert!(touched >= 64 * 30, "touched values reached the session: {touched}");
    assert!(control.frame().playing);
}

/// Records every MIDI channel message it gets (CC from the rack), packed as `b0 << 16 | b1 << 8 | b2`
struct MidiProbe {
    seen: Seen,
}

impl Processor for MidiProbe {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frame = cx.transport.frame;
        for e in cx.events {
            if let EventKind::Midi { bytes } = e.kind {
                let i = self.seen.len.load(Ordering::Relaxed);
                if i < self.seen.frames.len() {
                    self.seen.frames[i].store(frame + u64::from(e.offset), Ordering::Relaxed);
                    let packed = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
                    self.seen.values[i].store(packed, Ordering::Relaxed);
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

fn midi_seen(seen: &Seen) -> Vec<(u64, [u8; 3])> {
    let n = seen.len.load(Ordering::Acquire);
    (0..n)
        .map(|i| {
            let v = seen.values[i].load(Ordering::Relaxed);
            (seen.frames[i].load(Ordering::Relaxed), [(v >> 16) as u8, (v >> 8) as u8, v as u8])
        })
        .collect()
}

/// F-IJN-025: a CC of the track's rack reaches the instrument as a control change, at its sample, only when the
/// rounded value changes; its automation does too
#[test]
fn the_cc_rack_sends_control_changes() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    b.node(NodeDesc::processor(n(1), 0, 2));
    b.feed(Feed::new(n(1), m, 2));
    let ramp = Curve::new(vec![
        CurvePoint { sample: 0, value: 0.0, shape: 0.0 },
        CurvePoint { sample: 12_700, value: 127.0, shape: 0.0 },
    ])
    .unwrap();
    b.cc_param(ParamSlot(30), n(1), 2, 74, 10.0, None);
    b.cc_param(ParamSlot(31), n(1), 2, 1, 0.0, Some(ramp));
    let seen: Seen = Record::new(1 << 12);
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), core(MidiProbe { seen: seen.clone() }));
    install(&mut control, i);
    block(&mut dummy, 256);
    // The rack's initial value goes once; a new value goes; the same rounded value does not
    control.param_write(ParamSlot(30), 99.6);
    block(&mut dummy, 256);
    control.param_write(ParamSlot(30), 100.2);
    block(&mut dummy, 256);
    let cc74: Vec<u8> =
        midi_seen(&seen).iter().filter(|(_, b)| b[0] == 0xB2 && b[1] == 74).map(|(_, b)| b[2]).collect();
    assert_eq!(cc74, vec![10, 100]);
    // The automated CC climbs one step at a time while playing
    control.commands().send(Command::Play).expect("play");
    for _ in 0..60 {
        block(&mut dummy, 256);
    }
    let cc1: Vec<u8> = midi_seen(&seen).iter().filter(|(_, b)| b[0] == 0xB2 && b[1] == 1).map(|(_, b)| b[2]).collect();
    assert!(cc1.len() > 100, "{} steps", cc1.len());
    assert!(cc1.windows(2).all(|w| w[1] > w[0]), "each one a new value: {cc1:?}");
    assert_eq!(*cc1.last().unwrap(), 127);
}

/// P-38 and F-IJN-017: what a track plays goes out to its device at its exact time, on the route's channel, with
/// the fixed note (its note-off ends the same key) and the fixed input velocity of the port route
#[test]
fn midi_out_to_a_device() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    b.node(NodeDesc::processor(n(1), 0, 2));
    b.feed(Feed::new(n(1), m, 2));
    b.port(PortRoute { port: PortIndex(0), channel: None, node: n(1), fixed_velocity: Some(99) });
    b.midi_out(MidiOutRoute { node: n(1), port: PortIndex(5), channel: 3, fixed_velocity: None, fixed_note: Some(50) });
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), core(MidiProbe { seen: Record::new(16) }));
    install(&mut control, i);
    let mut out = control.take_midi_out().expect("the output ring");
    block(&mut dummy, 256);
    let t0 = dummy.host_time();
    send(&mut control, host_at(&dummy, 100), [0x90, 60, 20]);
    send(&mut control, host_at(&dummy, 200), [0xB0, 7, 33]);
    send(&mut control, host_at(&dummy, 300), [0x80, 60, 0]);
    for _ in 0..8 {
        block(&mut dummy, 256);
    }
    let mut got = Vec::new();
    while let Ok(e) = out.pop() {
        got.push(e);
    }
    let bytes: Vec<[u8; 3]> = got.iter().map(|e| e.bytes).collect();
    assert_eq!(bytes, vec![[0x93, 50, 99], [0xB3, 7, 33], [0x83, 50, 0]], "{got:?}");
    assert!(got.iter().all(|e| e.port == PortIndex(5)));
    // At the exact time: the stamp plus the one block the live MIDI waits
    let ns = |samples: u64| samples * 1_000_000_000 / u64::from(RATE);
    let want = t0.0 + ns(100 + MAX_BLOCK as u64);
    assert!(got[0].at.0.abs_diff(want) <= ns(1) + 1, "{} vs {want}", got[0].at.0);
    assert_eq!(got[2].at.0 - got[0].at.0, ns(200));
}
