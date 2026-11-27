//! The "pronto quando" of lm-render (interfaces.md, Parte 4, step 5) and the behaviours the renderer owns. Every
//! render call runs under the allocation trap (`assert_no_alloc`, with its allocator for this binary), on the
//! calling thread and, through the render-thread hook, on the workers
#![cfg(not(loom))]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use common::*;
use lm_base::HostTime;
use lm_render::*;
use lm_time::{Bpm, Ppq, TempoMap, TempoPoint, TimeSignature};

// assert_no_alloc checks debug builds (in release its default feature makes the trap a no-op, as the crate intends)
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |p, v| p.max(v.abs()))
}

/// The hand-made program of step 5: a sine → a track strip → a desk channel → the master, heard on the Dummy
#[test]
fn hand_made_program_sounds() {
    let (mut control, mut dummy) = make(2);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(2), tempo(120));
    let m = master(&mut b);
    let osc = n(1);
    let chan = n(2);
    b.node(NodeDesc::processor(osc, 0, 1).with_strip(Strip { gain: HostParam::fixed(0.5), pan: None }));
    b.node(NodeDesc::sum(chan, 2).with_strip(Strip { gain: HostParam::fixed(1.0), pan: Some(HostParam::fixed(0.0)) }));
    b.feed(Feed::new(osc, chan, 1).mix(ChannelMix::UpMix { to: 2 }));
    b.feed(Feed::new(chan, m, 2));
    let mut i = Install::new(b.build().expect("program"));
    i.add(osc, core(Sine::new(1000.0, 1.0)));
    install(&mut control, i);

    let out = render(&mut dummy, RATE as usize, 256);
    let expected = 0.5 / 2f32.sqrt();
    for ch in &out {
        let p = peak(&ch[1000..]);
        assert!((p - expected).abs() < 1e-3, "peak {p}, expected {expected}");
        // 1 kHz: 2000 sign changes in a second
        let crossings = ch.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((1998..=2002).contains(&crossings), "{crossings} crossings");
    }
    assert_eq!(out[0], out[1]);
    let returned = collect(&mut control);
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0].generation, ProgramGen(1));
    assert!(returned[0].program.as_ref().is_some_and(|p| p.generation() == ProgramGen(0)));
    let frame = control.frame();
    assert!(frame.meters[osc.index as usize].peak[0] > 0.49);
    assert_eq!(frame.generation, ProgramGen(1));
    assert!(frame.periods > 0);
}

/// 1000 installs in a row, many more than `installs_in_flight` at once: everything comes back, in order, and
/// nothing leaks (each processor holds a clone of a token)
#[test]
fn thousand_installs_everything_returned() {
    let (mut control, mut dummy) = make(2);
    let token = Arc::new(());
    struct Holder(#[allow(dead_code)] Arc<()>, Sine);
    impl Processor for Holder {
        fn process(&mut self, cx: &mut ProcessContext<'_>) {
            self.1.process(cx);
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }
    let mut returned_programs = 0usize;
    let mut returned_processors = 0usize;
    let mut last_gen = 0u64;
    let mut present = false;
    for round in 0..1000u32 {
        let next_gen = u64::from(round) + 1;
        let mut b = ProgramBuilder::new(ProgramGen(next_gen), spec(), &limits(2), tempo(120));
        let m = master(&mut b);
        let osc = NodeIndex::new(1 + round % 8, round + 1);
        b.node(NodeDesc::processor(osc, 0, 2));
        b.feed(Feed::new(osc, m, 2));
        let mut i = Install::new(b.build().expect("program"));
        if present {
            i.remove(NodeIndex::new(1 + (round - 1) % 8, round));
        }
        i.add(osc, core(Holder(token.clone(), Sine::new(220.0, 0.1))));
        present = true;
        install(&mut control, i);
        // Every third round renders; the others pile up in the main-thread queue
        if round % 3 == 0 {
            block(&mut dummy, 64);
            block(&mut dummy, 64);
        }
        assert!(control.installs_in_flight() <= limits(2).installs_in_flight);
        control.collect(|r| {
            assert!(r.generation.0 > last_gen, "returns in order");
            last_gen = r.generation.0;
            returned_programs += usize::from(r.program.is_some());
            returned_processors += r.processors.len();
        });
    }
    while control.installs_in_flight() + control.installs_queued() > 0 {
        block(&mut dummy, 64);
        control.collect(|r| {
            assert!(r.generation.0 > last_gen);
            last_gen = r.generation.0;
            returned_programs += usize::from(r.program.is_some());
            returned_processors += r.processors.len();
        });
    }
    assert_eq!(last_gen, 1000);
    assert_eq!(returned_programs, 1000);
    assert_eq!(returned_processors, 999);
    // The last processor is still in the table: dropping the engine frees it, on this thread
    drop(dummy);
    drop(control);
    assert_eq!(Arc::strong_count(&token), 1, "a processor leaked");
}

fn render_big(workers: usize, total: usize, size: usize) -> Vec<Vec<f32>> {
    let (mut control, mut dummy) = make(workers);
    let (program, cores) = big_program(1, 64, true);
    install_new(&mut control, program, cores);
    control.commands().send(Command::Play).expect("play");
    render(&mut dummy, total, size)
}

/// Item 9: the same program twice gives the same bits, on 1, 2 and 4 workers
#[test]
fn deterministic_across_runs_and_workers() {
    let a = render_big(0, 20_000, 512);
    let b = render_big(0, 20_000, 512);
    assert!(peak(&a[0]) > 0.1);
    assert_eq!(a, b);
    for workers in [1, 2, 4] {
        assert_eq!(a, render_big(workers, 20_000, 512), "{workers} workers");
    }
}

/// PDC: two paths of different latency, compensated by a feed delay, put an impulse on the same sample
#[test]
fn pdc_aligns_two_paths() {
    const L: u32 = 1000;
    let (mut control, mut dummy) = make(1);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(1), tempo(120));
    let m = master(&mut b);
    let src = n(1);
    let slow = n(2);
    b.node(NodeDesc::processor(src, 0, 2));
    b.node(NodeDesc::insert(slow, 2));
    b.feed(Feed::new(src, slow, 2));
    b.feed(Feed::new(slow, m, 2));
    b.feed(Feed::new(src, m, 2).delay(L));
    let mut i = Install::new(b.build().expect("program"));
    i.add(src, core(Impulse { at: 300 }));
    i.add(slow, core(Latency::new(L as usize, 2)));
    install(&mut control, i);
    let out = render(&mut dummy, 4096, 256);
    for ch in &out {
        let hits: Vec<(usize, f32)> = ch.iter().copied().enumerate().filter(|(_, v)| *v != 0.0).collect();
        assert_eq!(hits, vec![(300 + L as usize, 2.0)]);
    }
}

/// A delay line keeps its memory when a recompilation brings an equal one
#[test]
fn pdc_delay_survives_a_recompilation() {
    let (mut control, mut dummy) = make(0);
    let program = |generation| {
        let mut b = ProgramBuilder::new(ProgramGen(generation), spec(), &limits(0), tempo(120));
        let m = master(&mut b);
        b.node(NodeDesc::processor(n(1), 0, 2));
        b.feed(Feed::new(n(1), m, 2).delay(500));
        b.build().expect("program")
    };
    let mut i = Install::new(program(1));
    i.add(n(1), core(Impulse { at: 100 }));
    install(&mut control, i);
    block(&mut dummy, 256);
    install(&mut control, Install::new(program(2)));
    let out = render(&mut dummy, 1024, 256);
    assert_eq!(out[0][600 - 256], 1.0, "the impulse crossed the program change");
}

fn automation_program(rate: Rate) -> (Box<Program>, Vec<(NodeIndex, Box<NodeCore>)>) {
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    let osc = n(1);
    let fx = n(2);
    let points = |v: &[(i64, f32, f32)]| {
        Curve::new(v.iter().map(|&(sample, value, shape)| CurvePoint { sample, value, shape }).collect())
            .expect("curve")
    };
    let gain_curve = points(&[(0, 0.0, 0.0), (7000, 1.0, 0.4), (13_000, 0.2, -0.7), (20_000, 0.9, 0.0)]);
    let fx_curve = points(&[(0, 1.0, 0.0), (9000, 0.1, 0.0), (19_000, 0.6, 0.5)]);
    b.node(NodeDesc::processor(osc, 0, 2).with_strip(Strip {
        gain: HostParam::slot(ParamSlot(5), 0.0).automated(gain_curve),
        pan: Some(HostParam::slot(ParamSlot(6), 0.0).automated(points(&[(0, -1.0, 0.0), (20_000, 1.0, 0.0)]))),
    }));
    b.node(NodeDesc::insert(fx, 2));
    b.feed(Feed::new(osc, fx, 2));
    b.feed(Feed::new(fx, m, 2));
    b.param(ParamSlot(7), fx, 0, ParamInfo { rate, discrete: false, initial: 1.0 }, Some(fx_curve));
    (b.build().expect("program"), vec![(osc, core(Sine::new(440.0, 1.0))), (fx, core(Gain(1.0)))])
}

fn render_automation(rate: Rate, size: usize) -> Vec<Vec<f32>> {
    let (mut control, mut dummy) = make(0);
    let (p, cores) = automation_program(rate);
    install_new(&mut control, p, cores);
    control.commands().send(Command::Play).expect("play");
    render(&mut dummy, 24_000, size)
}

/// V3: blocks of 32, 64, 256 and 1024 give the same output (bit for bit: sample-rate automation is per sample,
/// block-rate automation sits on a grid of the engine's frame count, not of the block)
#[test]
fn block_size_independence() {
    for rate in [Rate::Sample, Rate::Block] {
        let reference = render_automation(rate, 1024);
        assert!(peak(&reference[0]) > 0.2);
        for size in [32, 64, 256, 1000] {
            let other = render_automation(rate, size);
            let worst =
                reference.iter().flatten().zip(other.iter().flatten()).fold(0.0f32, |w, (a, b)| w.max((a - b).abs()));
            assert!(worst < 1e-6, "{rate:?}, block {size}: {worst}");
            assert_eq!(reference, other, "{rate:?}, block {size}");
        }
    }
}

/// U25: 1000 program swaps with notes playing (items in a loop and live MIDI), with the instrument kept,
/// adopted, replaced into a tail node, removed and re-added, and the item edited: no note is left hanging
#[test]
fn no_stuck_notes_over_a_thousand_swaps() {
    let (mut control, mut dummy) = make(2);
    let tpq = 960i64;
    let notes_item = |key: u64, len: i64, count: u32| {
        let mut ev = Vec::new();
        for k in 0..count {
            let t = i64::from(k) * 240;
            ev.push(ItemEvent {
                tick: t,
                kind: ItemEventKind::NoteOn { n: k, channel: 0, key: 40 + (k % 40) as u8, velocity: 0.8 },
            });
            ev.push(ItemEvent { tick: t + len, kind: ItemEventKind::NoteOff { n: k } });
        }
        Arc::new(MidiItem::new(key, ev).expect("item"))
    };
    let item_long = notes_item(7, 700, 16);
    let item_short = notes_item(7, 60, 16);
    let synth = NodeIndex::new(1, 1);
    let tail = NodeIndex::new(3, 1);
    let mut alive: Vec<(Arc<AtomicI64>, Arc<AtomicU64>)> = Vec::new();
    let new_counter = |alive: &mut Vec<(Arc<AtomicI64>, Arc<AtomicU64>)>, adopts: bool| {
        let open = Arc::new(AtomicI64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        alive.push((open.clone(), total.clone()));
        core(NoteCounter::new(open, total, adopts))
    };
    let mut generation = 0u64;
    let mut program = |item: &Arc<MidiItem>, with_synth: bool, with_tail: bool, synth: NodeIndex| {
        generation += 1;
        let mut b = ProgramBuilder::new(ProgramGen(generation), spec(), &limits(2), tempo(120));
        let m = master(&mut b);
        if with_synth {
            b.node(NodeDesc::processor(synth, 0, 2));
            b.feed(Feed::new(synth, m, 2));
            b.midi_items(synth, vec![item.clone()]);
            b.port(PortRoute { port: PortIndex(0), channel: None, node: synth, fixed_velocity: None });
        }
        if with_tail {
            b.node(NodeDesc::processor(tail, 0, 2).tail());
            b.feed(Feed::new(tail, m, 2));
        }
        b.transport(TransportSettings { loop_range: Some((0, 4 * tpq * 24)), ..Default::default() });
        b.build().expect("program")
    };
    let mut i = Install::new(program(&item_long, true, false, synth));
    i.add(synth, new_counter(&mut alive, true));
    install(&mut control, i);
    control.commands().send(Command::Play).expect("play");

    let mut present = true;
    let mut synth_now = synth;
    let mut tail_present = false;
    let mut held: Vec<u8> = Vec::new();
    let mut rng = Rng::new(99);
    for round in 0..1000u32 {
        // Live notes, some held across swaps
        let t = dummy.host_time();
        let key = 60 + (round % 24) as u8;
        control.commands().send(Command::Midi { port: PortIndex(0), at: t, bytes: [0x90, key, 100] }).expect("midi");
        held.push(key);
        if held.len() > 5 {
            let k = held.remove(0);
            control.commands().send(Command::Midi { port: PortIndex(0), at: t, bytes: [0x80, k, 0] }).expect("midi");
        }
        let item = if round % 5 == 0 { &item_short } else { &item_long };
        let mut tail_now = false;
        let inst = match rng.below(6) {
            // Same program again (an unrelated edit) or the item edited
            0 | 1 => Install::new(program(item, present, tail_present, synth_now)),
            // Replaced by an instrument that adopts
            2 if present => {
                let mut i = Install::new(program(item, true, tail_present, synth_now));
                i.replace(synth_now, new_counter(&mut alive, true), None);
                i
            }
            // Replaced by one that cannot adopt: the old one goes to the tail node
            3 if present && !tail_present => {
                let mut i = Install::new(program(item, true, true, synth_now));
                i.replace(synth_now, new_counter(&mut alive, false), Some(tail));
                tail_now = true;
                i
            }
            // Removed
            4 if present => {
                present = false;
                let mut i = Install::new(program(item, false, tail_present, synth_now));
                i.remove(synth_now);
                i
            }
            // Re-added (a new generation of the slot)
            _ if !present => {
                present = true;
                synth_now = NodeIndex::new(1, synth_now.generation + 1);
                let mut i = Install::new(program(item, true, tail_present, synth_now));
                i.add(synth_now, new_counter(&mut alive, true));
                i
            }
            _ => Install::new(program(item, present, tail_present, synth_now)),
        };
        let mut inst = inst;
        if tail_present && rng.below(3) == 0 && !tail_now {
            // The compiler removes a tail node after its time
            inst.remove(tail);
            tail_present = false;
        } else if tail_now {
            tail_present = true;
        }
        // Keep the tail flag in the program in step with the table
        install(&mut control, inst);
        block(&mut dummy, 1 + rng.below(700) as usize);
        collect(&mut control);
    }
    for k in held.drain(..) {
        let t = dummy.host_time();
        control.commands().send(Command::Midi { port: PortIndex(0), at: t, bytes: [0x80, k, 0] }).expect("midi");
    }
    control.commands().send(Command::Stop).expect("stop");
    for _ in 0..8 {
        block(&mut dummy, 512);
    }
    collect(&mut control);
    assert_eq!(dummy.renderer().active_notes(), 0, "the active-notes table is not empty");
    let mut started = 0;
    for (open, total) in &alive {
        started += total.load(Ordering::Relaxed);
        // A processor that left the table is gone with its notes; every other one must be silent of notes
        assert!(open.load(Ordering::Relaxed) == 0 || Arc::strong_count(open) == 1, "a stuck note");
    }
    assert!(started > 500, "only {started} notes played");
}

/// U20: NaN in a node silences it, marks it and reports it; the rest keeps playing
#[test]
fn nan_node_is_silenced_and_reported() {
    let (mut control, mut dummy) = make(1);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(1), tempo(120));
    let m = master(&mut b);
    b.node(NodeDesc::processor(n(1), 0, 2));
    b.node(NodeDesc::processor(n(2), 0, 2));
    b.feed(Feed::new(n(1), m, 2));
    b.feed(Feed::new(n(2), m, 2));
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), core(GoesNan { calls: 0, bad_from: 3 }));
    i.add(n(2), core(Constant(0.25)));
    install(&mut control, i);
    let out = render(&mut dummy, 2048, 256);
    assert!(out.iter().flatten().all(|x| x.is_finite()));
    assert_eq!(out[0][0], 0.75);
    assert_eq!(out[0][2047], 0.25);
    let mut reported = Vec::new();
    control.events(|e| reported.push(e));
    assert!(reported.contains(&Telemetry::NodeNonFinite { generation: ProgramGen(1), node: n(1) }));
    block(&mut dummy, 1024);
    assert!(control.frame().meters[1].dead);
}

/// I-9.3: a panicking processor is contained, silenced and reported, with its message; the engine goes on, and a
/// replacement brings the node back
#[test]
fn panic_is_contained() {
    install_panic_hook();
    // No allocation trap on these threads: the panic machinery allocates its payload (the one exception)
    let (mut control, renderer) = engine(spec(), limits(2));
    let mut dummy = Dummy::new(renderer, 0, 2);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(2), tempo(120));
    let m = master(&mut b);
    for k in 1..=3 {
        b.node(NodeDesc::processor(n(k), 0, 2));
        b.feed(Feed::new(n(k), m, 2));
    }
    let program = b.build().expect("program");
    let mut i = Install::new(program);
    i.add(n(1), core(Panics { calls: 0, panic_at: 4 }));
    i.add(n(2), core(Constant(0.5)));
    i.add(n(3), core(Constant(0.125)));
    control.install(i).expect("install");
    for _ in 0..10 {
        dummy.block(256);
    }
    assert_eq!(dummy.outputs()[0][255], 0.625);
    let mut reports = Vec::new();
    control.panics(|r| reports.push(r));
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].node, Some(n(1)));
    assert!(reports[0].message().contains("on purpose"));
    let mut events = Vec::new();
    control.events(|e| events.push(e));
    assert!(events.contains(&Telemetry::NodePanicked { generation: ProgramGen(1), node: n(1) }));
    // Recreated: a replace brings it back
    let mut b = ProgramBuilder::new(ProgramGen(2), spec(), &limits(2), tempo(120));
    let m = master(&mut b);
    for k in 1..=3 {
        b.node(NodeDesc::processor(n(k), 0, 2));
        b.feed(Feed::new(n(k), m, 2));
    }
    let mut i = Install::new(b.build().expect("program"));
    i.replace(n(1), core(Constant(0.25)), None);
    control.install(i).expect("install");
    dummy.block(256);
    assert_eq!(dummy.outputs()[0][100], 0.875);
    let mut returned = Vec::new();
    control.collect(|r| returned.push(r));
    assert_eq!(returned.iter().map(|r| r.processors.len()).sum::<usize>(), 1, "the dead processor came back");
}

/// U20: a +30 dBFS spike on the master cuts the output with a very short fade, and is reported
#[test]
fn master_protection_fires() {
    let (mut control, mut dummy) = make(0);
    let b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let _ = b;
    // An impulse of 1.0 through a gain of 31.62: +30 dBFS at frame 2000, in the pass 1024..2048
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    b.node(NodeDesc::processor(n(1), 0, 2));
    b.node(NodeDesc::processor(n(2), 0, 2).with_strip(Strip { gain: HostParam::fixed(31.62), pan: None }));
    b.feed(Feed::new(n(1), m, 2));
    b.feed(Feed::new(n(2), m, 2));
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), core(Constant(0.1)));
    i.add(n(2), core(Impulse { at: 2000 }));
    install(&mut control, i);
    let mut spike = Vec::new();
    let out = render(&mut dummy, 4096, 1024);
    // Before the spike: the constant
    assert_eq!(out[0][1000], 0.1);
    // From the pass with the spike on: faded within 32 samples, then silence
    let cut_from = 1024;
    for (k, v) in out[0][cut_from..].iter().enumerate() {
        if k >= 32 {
            assert_eq!(*v, 0.0, "sample {k} after the cut");
        }
        spike.push(*v);
    }
    assert!(spike[..32].iter().all(|v| v.abs() <= 0.1 + 1e-6), "the spike passed the protection");
    let mut events = Vec::new();
    control.events(|e| events.push(e));
    assert!(events.iter().any(|e| matches!(e, Telemetry::MasterProtection { peak, .. } if *peak > 31.0)));
    assert!(control.frame().protection);
}

/// C2: a capture xrun while recording reports the lost frames at the right sample, and the playhead keeps time
#[test]
fn capture_xrun_reports_lost_frames() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    master(&mut b);
    install(&mut control, Install::new(b.build().expect("program")));
    control.commands().send(Command::Record { on: true }).expect("record");
    control.commands().send(Command::Play).expect("play");
    block(&mut dummy, 512);
    let report = assert_no_alloc::assert_no_alloc(|| {
        let r = dummy.block_with(512, 77);
        (r.capture_gap, r.recorded().len())
    });
    assert_eq!(report.0, Some((512, 77)));
    assert_eq!(dummy.renderer().position(), 512 + 77 + 512);
    let mut events = Vec::new();
    control.transport_events(|e| events.push(e));
    assert_eq!(events[0], TransportEvent::RecordStarted { sample: 0, frame: 0 });
    assert_eq!(events[1], TransportEvent::CaptureGap { sample: 512, frames: 77 });
}

/// Loop and punch at the exact sample, whatever the block size
#[test]
fn loop_and_punch_at_exact_samples() {
    for size in [100usize, 256, 1024] {
        let (mut control, mut dummy) = make(0);
        // The probe goes straight to the device: through the master, its values (thousands) would trip the ear
        // protection
        let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
        b.node(NodeDesc::processor(n(1), 0, 2));
        b.output(OutputRoute { node: n(1), channel: 0, device_channel: 0 });
        b.transport(TransportSettings {
            loop_range: Some((5000, 8000)),
            punch: Some((6000, 7000)),
            midi: MidiTiming::FixedBlock,
            count_in: None,
            pre_roll_bars: 0,
        });
        let mut i = Install::new(b.build().expect("program"));
        i.add(n(1), core(Position));
        install(&mut control, i);
        control.commands().send(Command::Locate { sample: 4000 }).expect("locate");
        control.commands().send(Command::Record { on: true }).expect("record");
        control.commands().send(Command::Play).expect("play");
        let mut spans: Vec<(usize, usize, i64)> = Vec::new();
        let mut out = Vec::new();
        let mut done = 0usize;
        while done < 12_000 {
            let r = dummy.block(size);
            for s in r.recorded() {
                spans.push((done + s.offset, s.frames, s.sample));
            }
            out.extend_from_slice(&dummy.outputs()[0][..size]);
            done += size;
        }
        // 4000..8000, then 5000..8000 again and again
        for (k, v) in out.iter().enumerate() {
            let expected = if k < 4000 { 4000 + k } else { 5000 + (k - 4000) % 3000 };
            assert_eq!(*v as usize, expected, "block {size}, sample {k}");
        }
        // Merge spans that continue each other across blocks
        let mut merged: Vec<(usize, usize, i64)> = Vec::new();
        for s in spans {
            match merged.last_mut() {
                Some(l) if l.0 + l.1 == s.0 && l.2 + l.1 as i64 == s.2 => l.1 += s.1,
                _ => merged.push(s),
            }
        }
        assert_eq!(merged[0], (2000, 1000, 6000), "block {size}");
        assert_eq!(merged[1], (5000, 1000, 6000), "block {size}");
        assert_eq!(merged[2], (8000, 1000, 6000), "block {size}");
    }
}

/// H9: a tempo edit while playing keeps the musical position
#[test]
fn relocate_on_tempo_edit() {
    let (mut control, mut dummy) = make(0);
    let program = |generation, map: TempoMap| {
        let mut b = ProgramBuilder::new(ProgramGen(generation), spec(), &limits(0), map);
        master(&mut b);
        b.build().expect("program")
    };
    install(&mut control, Install::new(program(1, tempo(120))));
    control.commands().send(Command::Play).expect("play");
    for _ in 0..30 {
        block(&mut dummy, 1000);
    }
    let before = dummy.renderer().position();
    assert_eq!(before, 30_000);
    let old = tempo(120);
    let new = TempoMap::new(
        RATE,
        Ppq::DEFAULT,
        &[
            TempoPoint::new(0, Bpm::from_integer(120).expect("bpm"), TimeSignature::COMMON),
            TempoPoint::new(960, Bpm::from_integer(60).expect("bpm"), TimeSignature::COMMON),
        ],
    )
    .expect("map");
    let tick = old.tick_at(before - 1);
    let expected = lm_time::relocate(&old, &new, before);
    let mut i = Install::new(program(2, new.clone()));
    i.relocate(Relocate { tick });
    install(&mut control, i);
    block(&mut dummy, 100);
    assert_eq!(dummy.renderer().position(), expected + 100);
    assert_ne!(expected, before);
    assert_eq!(new.tick_at(expected - 1), tick);
    let mut events = Vec::new();
    control.transport_events(|e| events.push(e));
    assert!(events.contains(&TransportEvent::Relocated { from: before, to: expected }));
}

/// R14, C6: a MIDI note stamped at a known host time plays at its offset plus one block, within one sample
#[test]
fn midi_lands_at_its_time_plus_one_block() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    b.node(NodeDesc::processor(n(1), 0, 2));
    b.feed(Feed::new(n(1), m, 2));
    b.port(PortRoute { port: PortIndex(3), channel: None, node: n(1), fixed_velocity: None });
    let open = Arc::new(AtomicI64::new(0));
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), core(NoteCounter::new(open.clone(), Arc::new(AtomicU64::new(0)), false)));
    install(&mut control, i);
    block(&mut dummy, 256);
    let t0 = dummy.host_time();
    // 100 samples into the next block at 48 kHz
    let at = HostTime(t0.0 + 100 * 1_000_000_000 / u64::from(RATE));
    control.commands().send(Command::Midi { port: PortIndex(3), at, bytes: [0x90, 60, 64] }).expect("midi");
    // The note is due 100 + 1024 samples after t0: in the fifth block of 256 (1024..1280), at 100
    let mut first_sound = None;
    for k in 0..8 {
        block(&mut dummy, 256);
        if first_sound.is_none()
            && let Some(p) = dummy.outputs()[0][..256].iter().position(|v| *v > 0.0)
        {
            first_sound = Some(k * 256 + p);
        }
    }
    let at = first_sound.expect("the note played");
    assert!((1123..=1125).contains(&at), "note at {at}");
    assert_eq!(open.load(Ordering::Relaxed), 1);
}

/// I-6: a slot write reaches the processor as an event; a touched slot wins over automation and is sent back
/// on the lossless ring while playing; the frame shows automated values
#[test]
fn params_events_touch_and_telemetry() {
    let (mut control, mut dummy) = make(0);
    let (p, cores) = automation_program(Rate::Block);
    install_new(&mut control, p, cores);
    control.commands().send(Command::Play).expect("play");
    for _ in 0..20 {
        block(&mut dummy, 512);
    }
    let auto = control.frame().params[7];
    assert!(auto > 0.0 && auto < 1.0, "automated value {auto}");
    control.params().touch(ParamSlot(7), true);
    control.param_write(ParamSlot(7), 0.0);
    block(&mut dummy, 512);
    block(&mut dummy, 512);
    assert!(peak(&dummy.outputs()[0][..512]) == 0.0, "the hand did not win");
    let mut touched = Vec::new();
    control.touched(|t| touched.push(t));
    assert_eq!(touched.len(), 1);
    assert_eq!(touched[0].slot, ParamSlot(7));
    assert_eq!(touched[0].value, 0.0);
    control.params().touch(ParamSlot(7), false);
    block(&mut dummy, 512);
    block(&mut dummy, 512);
    assert!(peak(&dummy.outputs()[0][..512]) > 0.0, "the automation did not come back");
}

/// S20: a new slot starts from the program's initial value
#[test]
fn initial_param_values() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    b.node(NodeDesc::processor(n(1), 0, 2).with_strip(Strip { gain: HostParam::slot(ParamSlot(3), 0.5), pan: None }));
    b.feed(Feed::new(n(1), m, 2));
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), core(Constant(1.0)));
    install(&mut control, i);
    let out = render(&mut dummy, 512, 256);
    assert_eq!(out[0][0], 0.5);
    control.param_write(ParamSlot(3), 0.25);
    let out = render(&mut dummy, 256, 256);
    assert!(out[0][10] < 0.5 && out[0][10] > 0.25, "smoothed");
    assert_eq!(out[0][100], 0.25);
}

/// I-7.3: a sample preview plays on the preview bus and comes back whole
#[test]
fn preview_plays_and_returns() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    b.preview_bus(m);
    install(&mut control, Install::new(b.build().expect("program")));
    let data = Arc::new(SampleData::from_channels(RATE, vec![vec![0.5; 3000]]).expect("sample"));
    control
        .commands()
        .send(Command::Preview(PreviewCommand::Sample { id: PreviewId(9), data: data.clone(), gain: 0.5 }))
        .expect("preview");
    let out = render(&mut dummy, 4096, 1024);
    assert_eq!(out[0][10], 0.25);
    assert_eq!(out[1][2999], 0.25);
    assert_eq!(out[0][3000], 0.0);
    let mut ended = Vec::new();
    control.previews_ended(|e| ended.push(e));
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id, PreviewId(9));
    drop(ended);
    assert_eq!(Arc::strong_count(&data), 1);
}

/// I-5.5: the metronome clicks on every beat, the accent on the bar
#[test]
fn metronome_on_the_beats() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    let click = n(1);
    b.node(NodeDesc::sum(click, 1));
    b.feed(Feed::new(click, m, 1).mix(ChannelMix::UpMix { to: 2 }));
    let accent = Arc::new(SampleData::from_channels(RATE, vec![vec![1.0; 10]]).expect("sample"));
    let beat = Arc::new(SampleData::from_channels(RATE, vec![vec![0.5; 10]]).expect("sample"));
    b.metronome(click, Metronome { accent, beat, gain: 1.0, enabled: true, only_recording: false });
    install(&mut control, Install::new(b.build().expect("program")));
    control.commands().send(Command::Play).expect("play");
    let out = render(&mut dummy, 4 * 24_000 + 100, 333);
    let s = 1.0 / 2f32.sqrt();
    for beat in 0..5 {
        let at = beat * 24_000;
        let expected = if beat % 4 == 0 { s } else { 0.5 * s };
        assert!((out[0][at] - expected).abs() < 1e-6, "beat {beat}: {}", out[0][at]);
        assert_eq!(out[0][at + 10], 0.0);
        if at > 0 {
            assert_eq!(out[0][at - 1], 0.0);
        }
    }
}

/// U32: a processor that cannot adopt goes to a tail node with its notes released, ends, and is reported
#[test]
fn replace_without_adopt_goes_to_the_tail() {
    let (mut control, mut dummy) = make(0);
    let program = |generation, tail: bool| {
        let mut b = ProgramBuilder::new(ProgramGen(generation), spec(), &limits(0), tempo(120));
        let m = master(&mut b);
        b.node(NodeDesc::processor(n(1), 0, 2));
        b.feed(Feed::new(n(1), m, 2));
        b.port(PortRoute { port: PortIndex(0), channel: None, node: n(1), fixed_velocity: None });
        if tail {
            b.node(NodeDesc::processor(n(2), 0, 2).tail());
            b.feed(Feed::new(n(2), m, 2));
        }
        b.build().expect("program")
    };
    let old_open = Arc::new(AtomicI64::new(0));
    let mut i = Install::new(program(1, false));
    i.add(n(1), core(NoteCounter::new(old_open.clone(), Arc::new(AtomicU64::new(0)), false)));
    install(&mut control, i);
    let t = dummy.host_time();
    control.commands().send(Command::Midi { port: PortIndex(0), at: t, bytes: [0x90, 60, 100] }).expect("midi");
    for _ in 0..4 {
        block(&mut dummy, 512);
    }
    assert_eq!(old_open.load(Ordering::Relaxed), 1);
    let new_open = Arc::new(AtomicI64::new(0));
    let mut i = Install::new(program(2, true));
    i.replace(n(1), core(NoteCounter::new(new_open.clone(), Arc::new(AtomicU64::new(0)), false)), Some(n(2)));
    install(&mut control, i);
    block(&mut dummy, 512);
    assert_eq!(old_open.load(Ordering::Relaxed), 0, "the tail got all notes released");
    // The note-off of the old note goes nowhere: the new processor never had it
    let t = dummy.host_time();
    control.commands().send(Command::Midi { port: PortIndex(0), at: t, bytes: [0x80, 60, 0] }).expect("midi");
    for _ in 0..4 {
        block(&mut dummy, 512);
    }
    assert_eq!(new_open.load(Ordering::Relaxed), 0);
    let mut events = Vec::new();
    control.events(|e| events.push(e));
    assert!(events.contains(&Telemetry::TailEnded { generation: ProgramGen(2), node: n(2) }));
}

/// C-3: the hard end of a tail falls on an exact frame (2 s after it began), so a loud tail sounds the same
/// whatever the block size the device gives
#[test]
fn tail_hard_cap_is_the_same_for_any_block_size() {
    let run = |size: usize| {
        let (mut control, mut dummy) = make(0);
        let program = |generation, tail: bool| {
            let mut b = ProgramBuilder::new(ProgramGen(generation), spec(), &limits(0), tempo(120));
            let m = master(&mut b);
            b.node(NodeDesc::processor(n(1), 0, 2));
            b.feed(Feed::new(n(1), m, 2));
            if tail {
                b.node(NodeDesc::processor(n(2), 0, 2).tail());
                b.feed(Feed::new(n(2), m, 2));
            }
            b.build().expect("program")
        };
        let mut i = Install::new(program(1, false));
        i.add(n(1), core(Sine::new(440.0, 0.5)));
        install(&mut control, i);
        render(&mut dummy, 4800, 480);
        let mut i = Install::new(program(2, true));
        i.replace(n(1), core(Sine::new(0.0, 0.0)), Some(n(2)));
        install(&mut control, i);
        render(&mut dummy, 3 * RATE as usize, size)
    };
    let a = run(480);
    let b = run(777);
    let c = run(1024);
    assert_eq!(a, b);
    assert_eq!(a, c);
    // The tail is audible until its cap and silent after it
    let cap = 2 * RATE as usize;
    assert!(peak(&a[0][cap - 480..cap]) > 0.1);
    assert_eq!(peak(&a[0][cap + 480..]), 0.0);
}

/// A processor that hangs on a worker thread until `release` is set (it behaves on the audio thread)
struct Hang {
    release: Arc<std::sync::atomic::AtomicBool>,
}

impl Processor for Hang {
    fn process(&mut self, _cx: &mut ProcessContext<'_>) {
        let on_worker = std::thread::current().name().is_some_and(|n| n.starts_with("lm-render-"));
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while on_worker && !self.release.load(Ordering::Acquire) && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// R-8: a worker that never gives its node back does not freeze the callback: after the deadline the node is dead
/// and reported, the rest keeps sounding, and the late job goes home when the worker finally returns it
#[test]
fn a_hung_worker_is_given_up_after_the_deadline() {
    let (mut control, mut dummy) = make(1);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(1), tempo(120));
    let m = master(&mut b);
    b.node(NodeDesc::processor(n(1), 0, 2));
    b.node(NodeDesc::processor(n(2), 0, 2));
    b.feed(Feed::new(n(1), m, 2));
    b.feed(Feed::new(n(2), m, 2));
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), core(Hang { release: release.clone() }));
    i.add(n(2), core(Sine::new(1000.0, 0.5)));
    install(&mut control, i);
    let started = std::time::Instant::now();
    let out = render(&mut dummy, 512, 256);
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "the callback came back");
    assert!(peak(&out[0]) > 0.4, "the other node still sounds");
    let mut events = Vec::new();
    control.events(|e| events.push(e));
    assert!(events.iter().any(|e| matches!(e, Telemetry::NodeStuck { .. })), "{events:?}");
    // The worker returns the job late: it goes home, the node stays silent, and the engine goes on
    release.store(true, Ordering::Release);
    std::thread::sleep(std::time::Duration::from_millis(100));
    let out = render(&mut dummy, 1024, 256);
    assert!(peak(&out[0]) > 0.4);
}

/// Locate is refused while recording (I-7.2)
#[test]
fn locate_refused_while_recording() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    master(&mut b);
    install(&mut control, Install::new(b.build().expect("program")));
    control.commands().send(Command::Record { on: true }).expect("record");
    control.commands().send(Command::Play).expect("play");
    block(&mut dummy, 256);
    control.commands().send(Command::Locate { sample: 0 }).expect("locate");
    block(&mut dummy, 256);
    assert_eq!(dummy.renderer().position(), 512);
    let mut events = Vec::new();
    control.events(|e| events.push(e));
    assert!(events.contains(&Telemetry::LocateRefused));
}

/// Audio items play at their sample, with their fades and the explicit channel conversion
#[test]
fn audio_items_play_in_place() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    let track = n(1);
    b.node(NodeDesc::sum(track, 2));
    b.feed(Feed::new(track, m, 2));
    let data = Arc::new(SampleData::from_channels(RATE, vec![vec![1.0; 1000]]).expect("sample"));
    b.audio_items(
        track,
        vec![AudioItem {
            start: 700,
            length: 500,
            source: AudioSource::Memory(data),
            offset: 100,
            gain: 1.0,
            fade_in: 10,
            fade_out: 0,
        }],
    );
    install(&mut control, Install::new(b.build().expect("program")));
    control.commands().send(Command::Play).expect("play");
    let out = render(&mut dummy, 2048, 300);
    let s = 1.0 / 2f32.sqrt();
    assert_eq!(out[0][699], 0.0);
    assert_eq!(out[0][700], 0.0);
    assert!((out[0][705] - 0.5 * s).abs() < 1e-6);
    assert!((out[1][900] - s).abs() < 1e-6);
    assert_eq!(out[0][1200], 0.0);
}

/// A program the builder must refuse
#[test]
fn builder_refuses_bad_programs() {
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    b.node(NodeDesc::sum(n(1), 2));
    b.node(NodeDesc::sum(n(2), 2));
    b.feed(Feed::new(n(1), n(2), 2));
    b.feed(Feed::new(n(2), n(1), 2));
    assert_eq!(b.build().err().map(|e| e.key()), Some("render.program.cycle"));
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    b.node(NodeDesc::sum(n(1), 2));
    b.feed(Feed::new(n(1), n(9), 2));
    assert_eq!(b.build().err().map(|e| e.key()), Some("render.program.unknown_node"));
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    b.node(NodeDesc::sum(n(600), 2));
    assert_eq!(b.build().err().map(|e| e.key()), Some("render.program.node_out_of_range"));
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    b.transport(TransportSettings { loop_range: Some((0, 10)), ..Default::default() });
    assert_eq!(b.build().err().map(|e| e.key()), Some("render.program.loop_too_short"));
}

/// The render cost of a 64-track program, per block of 256 (printed; run with `--nocapture` in release)
#[test]
#[allow(clippy::print_stderr)]
fn cost_of_sixty_four_tracks() {
    // A timing budget means nothing under a sanitizer (TSan alone multiplies the cost several times)
    if std::env::var_os("LM_TEST_SANITIZER").is_some() {
        return;
    }
    for workers in [0usize, 2, 4] {
        let (mut control, mut dummy) = make(workers);
        let (program, cores) = big_program(1, 64, true);
        install_new(&mut control, program, cores);
        control.commands().send(Command::Play).expect("play");
        for _ in 0..50 {
            block(&mut dummy, 256);
        }
        let blocks = 2000;
        let start = std::time::Instant::now();
        for _ in 0..blocks {
            block(&mut dummy, 256);
        }
        let per_block = start.elapsed() / blocks;
        eprintln!("64 tracks, {workers} workers: {per_block:?} per block of 256 (budget 5.33 ms)");
        assert!(per_block.as_secs_f64() < 256.0 / f64::from(RATE), "slower than real time");
    }
}

/// Decays into the subnormal range: each sample is half the previous one, from 2^-120
struct Decay;

impl Processor for Decay {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        for ch in 0..cx.buffers.outputs() {
            let mut x = std::hint::black_box(f32::from_bits(0x0380_0000)); // 2^-120, normal
            for v in cx.buffers.output(ch).iter_mut() {
                *v = x;
                x = std::hint::black_box(x) * std::hint::black_box(0.5);
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn render_decay(workers: usize) -> Vec<Vec<f32>> {
    let (mut control, mut dummy) = make(workers);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(workers), tempo(120));
    // Straight to the device, two equal nodes in one level (so a worker takes one when there are workers)
    for k in 1..=2u32 {
        b.node(NodeDesc::processor(n(k), 0, 1));
        b.output(OutputRoute { node: n(k), channel: 0, device_channel: (k - 1) as u16 });
    }
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), core(Decay));
    i.add(n(2), core(Decay));
    install(&mut control, i);
    let out = render(&mut dummy, 2048, 256);
    assert!(control.frame().flushing_denormals, "the audio thread does not flush");
    out
}

/// H6: subnormals are flushed on every render thread, so the audio thread, the workers and an export (another
/// renderer, without a device) give the same bits
#[test]
fn subnormals_flushed_on_every_render_thread() {
    let inline = render_decay(0);
    for ch in &inline {
        // 2^-120 halved six times is 2^-126, the smallest normal; the next one would be subnormal
        assert_eq!(ch[6], f32::MIN_POSITIVE);
        assert!(ch[7..256].iter().all(|v| *v == 0.0), "a subnormal reached the output");
    }
    // The first block's nodes are unmeasured, so they go to the workers: block 0 is the worker path
    assert_eq!(inline, render_decay(3));
    assert!(!flushing_denormals(), "the flush leaked out of the render call");
}

/// Copies its sidechain into its output, plus its input
struct SidechainProbe;

impl Processor for SidechainProbe {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let (ins, mut outs, side) = cx.buffers.split();
        for ch in 0..outs.len() {
            let o = outs.get_mut(ch);
            for (k, x) in o.iter_mut().enumerate() {
                *x = ins.get(ch)[k] + if side.is_empty() { 0.0 } else { side.get(0)[k] * 10.0 };
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// H5 and base item 3: a device input reaches its node (monitoring, no delay) and a feed reaches a sidechain
#[test]
fn monitoring_input_and_sidechain() {
    let (mut control, mut dummy) = make(0);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    let input = n(1);
    let key = n(2);
    let fx = n(3);
    b.node(NodeDesc::sum(input, 1));
    b.input(input, vec![1]);
    b.node(NodeDesc::processor(key, 0, 1));
    b.node(NodeDesc::processor(fx, 1, 1).with_sidechain(1));
    b.feed(Feed::new(input, fx, 1));
    b.feed(Feed::new(key, fx, 1).to_sidechain());
    b.feed(Feed::new(fx, m, 1).mix(ChannelMix::UpMix { to: 2 }));
    let mut i = Install::new(b.build().expect("program"));
    i.add(key, core(Constant(0.01)));
    i.add(fx, core(SidechainProbe));
    install(&mut control, i);
    for (k, v) in dummy.inputs_mut()[1].iter_mut().enumerate() {
        *v = k as f32 * 1e-4;
    }
    block(&mut dummy, 512);
    let s = 1.0 / 2f32.sqrt();
    for k in [0usize, 1, 300, 511] {
        let expected = (k as f32 * 1e-4 + 0.1) * s;
        assert!((dummy.outputs()[0][k] - expected).abs() < 1e-6, "sample {k}");
    }
}

/// Records each parameter event it gets: (engine frame, value)
struct ParamLog(Arc<std::sync::Mutex<Vec<(u64, f32)>>>, Vec<(u64, f32)>);

impl Processor for ParamLog {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        for e in cx.events {
            if let EventKind::Param { value, .. } = e.kind
                && self.1.len() < self.1.capacity()
            {
                self.1.push((cx.transport.frame + u64::from(e.offset), value));
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl Drop for ParamLog {
    fn drop(&mut self) {
        if let Ok(mut v) = self.0.lock() {
            v.extend_from_slice(&self.1);
        }
    }
}

/// Base item 1: a discrete parameter is never interpolated and changes at the exact sample of each point
#[test]
fn discrete_automation_at_exact_samples() {
    for size in [64usize, 1000] {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        {
            let (mut control, mut dummy) = make(0);
            let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
            master(&mut b);
            b.node(NodeDesc::processor(n(1), 0, 1));
            let curve = Curve::new(vec![
                CurvePoint { sample: 0, value: 0.0, shape: 0.0 },
                CurvePoint { sample: 1234, value: 2.0, shape: 0.0 },
                CurvePoint { sample: 3001, value: 1.0, shape: 0.0 },
            ])
            .expect("curve");
            b.param(ParamSlot(1), n(1), 0, ParamInfo { rate: Rate::Block, discrete: true, initial: 0.0 }, Some(curve));
            let mut i = Install::new(b.build().expect("program"));
            i.add(n(1), core(ParamLog(log.clone(), Vec::with_capacity(64))));
            install(&mut control, i);
            control.commands().send(Command::Play).expect("play");
            render(&mut dummy, 4000, size);
        }
        let got = log.lock().expect("log").clone();
        assert_eq!(got, vec![(0, 0.0), (1234, 2.0), (3001, 1.0)], "block {size}");
    }
}
