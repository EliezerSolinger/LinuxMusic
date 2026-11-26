//! The chain of note effects (interfaces.md I-9, `Kind::NoteEffect`): what the sources make for an instrument goes
//! through the chain, and the instrument hears the result. Every render call runs under the allocation trap
#![cfg(not(loom))]

mod common;

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use common::*;
use lm_render::*;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// A note effect: every note an octave up (parameter 0 is the shift in semitones)
struct Shift(i32);

impl Processor for Shift {
    fn process(&mut self, _: &mut ProcessContext<'_>) {}

    fn process_notes(&mut self, cx: &mut NoteContext<'_>) {
        for e in cx.events {
            match e.kind {
                EventKind::Param { index: 0, value } => self.0 = value as i32,
                EventKind::NoteOn { note, channel, key, velocity } => {
                    let key = (i32::from(key) + self.0).clamp(0, 127) as u8;
                    cx.out.push(e.offset, EventKind::NoteOn { note, channel, key, velocity });
                }
                EventKind::NoteOff { note, channel, key, velocity } => {
                    let key = (i32::from(key) + self.0).clamp(0, 127) as u8;
                    cx.out.push(e.offset, EventKind::NoteOff { note, channel, key, velocity });
                }
                kind => {
                    cx.out.push(e.offset, kind);
                }
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// An instrument that keeps the key of the last note it was given, and counts the notes
struct Last {
    key: Arc<AtomicU32>,
    count: Arc<AtomicU32>,
}

impl Processor for Last {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        for e in cx.events {
            if let EventKind::NoteOn { key, .. } = e.kind {
                self.key.store(u32::from(key), Ordering::Relaxed);
                self.count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn program(generation: u64, chained: bool) -> Box<Program> {
    let mut b = ProgramBuilder::new(ProgramGen(generation), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    let (inst, fx) = (n(1), n(2));
    b.node(NodeDesc::processor(inst, 0, 2));
    b.feed(Feed::new(inst, m, 2));
    b.port(PortRoute { port: PortIndex(0), channel: None, node: inst, fixed_velocity: None });
    if chained {
        b.node(NodeDesc::note_effect(fx));
        b.note_chain(inst, vec![fx]);
    }
    b.build().expect("program")
}

#[test]
fn the_instrument_hears_what_the_chain_makes() {
    let (mut control, mut dummy) = make(0);
    let key = Arc::new(AtomicU32::new(0));
    let count = Arc::new(AtomicU32::new(0));
    let mut i = Install::new(program(1, true));
    i.add(n(1), core(Last { key: key.clone(), count: count.clone() }));
    i.add(n(2), core(Shift(12)));
    install(&mut control, i);
    let t = dummy.host_time();
    control.commands().send(Command::Midi { port: PortIndex(0), at: t, bytes: [0x90, 60, 100] }).expect("midi");
    render(&mut dummy, 4096, 256);
    assert_eq!(key.load(Ordering::Relaxed), 72, "an octave up");
    assert_eq!(count.load(Ordering::Relaxed), 1);
}

#[test]
fn a_node_without_a_chain_is_untouched() {
    let (mut control, mut dummy) = make(0);
    let key = Arc::new(AtomicU32::new(0));
    let count = Arc::new(AtomicU32::new(0));
    let mut i = Install::new(program(1, false));
    i.add(n(1), core(Last { key: key.clone(), count: count.clone() }));
    install(&mut control, i);
    let t = dummy.host_time();
    control.commands().send(Command::Midi { port: PortIndex(0), at: t, bytes: [0x90, 60, 100] }).expect("midi");
    render(&mut dummy, 4096, 256);
    assert_eq!(key.load(Ordering::Relaxed), 60);
}

#[test]
fn an_effect_not_built_yet_passes_the_stream_through() {
    let (mut control, mut dummy) = make(0);
    let key = Arc::new(AtomicU32::new(0));
    let count = Arc::new(AtomicU32::new(0));
    // (no core for the effect: its plugin is still being built)
    let mut i = Install::new(program(1, true));
    i.add(n(1), core(Last { key: key.clone(), count: count.clone() }));
    install(&mut control, i);
    let t = dummy.host_time();
    control.commands().send(Command::Midi { port: PortIndex(0), at: t, bytes: [0x90, 64, 100] }).expect("midi");
    render(&mut dummy, 4096, 256);
    assert_eq!(key.load(Ordering::Relaxed), 64);
}

#[test]
fn a_chain_that_is_not_made_of_note_effects_is_refused() {
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let _ = master(&mut b);
    b.node(NodeDesc::processor(n(1), 0, 2));
    b.node(NodeDesc::processor(n(2), 0, 2));
    b.note_chain(n(1), vec![n(2)]);
    assert!(b.build().is_err());
}
