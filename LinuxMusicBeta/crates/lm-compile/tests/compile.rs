//! The "pronto quando" of step 7 (interfaces.md, Parte 4) and the translation rules: random roots compile and play,
//! unchanged nodes are reused, an index is never reused before its return, solo, PDC, monitoring, the slots, the
//! builds (waiting, replacing, tails), the limits, and a project rendered through `lm-render`'s `Dummy`.

mod common;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use common::*;
use lm_compile::{
    Compiled, CompilerCore, Environment, InsertPoint, NodeKey, ResourceState, Resources, Route, Stage, rack_params,
};
use lm_render::{NodeIndex, PortIndex, Returned, SampleData};
use lm_state::{NodeId, ROOT, Resource, Value};

fn route(c: &Compiled, from: NodeKey, to: NodeKey) -> Option<Route> {
    c.routes.iter().find(|r| r.from == from && r.to == to).copied()
}

fn muted(c: &Compiled, key: NodeKey) -> bool {
    c.nodes.iter().find(|n| n.key == key).is_some_and(|n| n.muted)
}

fn master() -> NodeId {
    lm_doc::MASTER
}

#[test]
fn an_empty_project_compiles_and_installs() {
    let plugins = plugins();
    let doc = new_doc(&plugins);
    let mut h = Harness::new(&plugins);
    let c = h.settle(doc.root());
    assert!(c.complete());
    let t = c.table.clone();
    for k in [NodeKey::Metronome, NodeKey::Preview, NodeKey::ChannelIn(master()), NodeKey::Fader(master())] {
        assert!(t.node(k).is_some(), "{k:?}");
    }
    // The master's rack in its three places (analog by default)
    for s in [Stage::MasterDrive, Stage::MasterBus, Stage::MasterTransformer] {
        assert!(t.node(NodeKey::Rack(master(), s)).is_some(), "{s:?}");
    }
    // The master's volume and its rack's real fields have slots
    assert_eq!(t.slots_of(master(), "volume").len(), 1);
    for p in rack_params(&schemas(&plugins), true) {
        assert_eq!(t.slots_of(master(), &p.key).len(), 1, "{}", p.key);
    }
}

#[test]
fn a_note_plays_at_its_exact_samples() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let t = add_track(&mut doc, Some(dc()));
    let item = add_item(&mut doc, t, "midi", 1920, 3840);
    // 120 BPM at 960 PPQ and 48 kHz: 25 samples per tick. The note: ticks 1920 + 480 .. + 960
    edit(&mut doc, |e| e.add_note(item, note(60, 480, 960, 0.5)).map(|_| ()));
    let mut h = Harness::new(&plugins);
    h.settle(doc.root());
    h.play();
    let out = h.render(48_000 * 2 / BLOCK + 2);
    assert!(out.len() > 80_000);
    let on = (1920 + 480) * 25;
    let off = on + 960 * 25;
    for (pos, l, r) in out {
        let want = if (on..off).contains(&pos) { 0.5 } else { 0.0 };
        assert_eq!((l, r), (want, want), "at sample {pos}");
    }
}

#[test]
fn an_audition_plays_one_item_alone_in_its_own_loop() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let t = add_track(&mut doc, Some(dc()));
    let a = add_item(&mut doc, t, "midi", 0, 3840);
    let b = add_item(&mut doc, t, "midi", 3840, 3840);
    edit(&mut doc, |e| e.add_note(a, note(60, 0, 960, 0.5)).map(|_| ()));
    edit(&mut doc, |e| e.add_note(b, note(60, 0, 960, 0.25)).map(|_| ()));
    let mut h = Harness::new(&plugins);
    // 25 samples per tick: the second item starts at sample 96_000
    h.env.audition = Some(lm_compile::Audition { item: b, from: 3840, to: 3840 + 1920, repeat: true });
    h.settle(doc.root());
    h.play();
    let out = h.render(48_000 * 3 / BLOCK + 2);
    let first_item = out.iter().filter(|(pos, ..)| *pos < 96_000).any(|(_, l, _)| *l != 0.0);
    assert!(!first_item, "the other item is silent");
    // the loop (1920 ticks = 48_000 samples) starts the transport again at its start: the note is heard in the second turn
    assert!(out.iter().any(|(_, l, _)| *l == 0.25), "the audition's item plays");
    assert!(out.iter().all(|(_, l, _)| *l != 0.5), "never the other item");
}

#[test]
fn unchanged_nodes_keep_their_index_processor_and_slots() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let t1 = add_track(&mut doc, Some(dc()));
    let t2 = add_track(&mut doc, Some(dc()));
    let i1 = add_item(&mut doc, t1, "midi", 0, 3840);
    let i2 = add_item(&mut doc, t2, "midi", 0, 3840);
    let mut n1 = 0;
    edit(&mut doc, |e| {
        n1 = e.add_note(i1, note(60, 0, 100, 0.5))?;
        e.add_note(i2, note(62, 0, 100, 0.5)).map(|_| ())
    });
    let ch = add_channel(&mut doc);
    insert(&mut doc, ch, delay(0));
    let mut h = Harness::new(&plugins);
    let first = h.settle(doc.root());
    let before = first.table.clone();

    // A parameter edit only writes a slot: compiling the new root changes nothing at all
    edit(&mut doc, |e| e.set_param(t1, "volume", 0.5));
    let c = h.compile(doc.root());
    assert!(c.builds.is_empty() && c.cancelled.is_empty() && c.complete());
    assert_eq!(c.table.nodes, before.nodes);
    assert_eq!(c.table.slots, before.slots);
    assert_eq!(c.made.midi_items, 0);
    h.pump();

    // Moving one note makes one event list and nothing else
    edit(&mut doc, |e| e.set_note(i1, n1, &[k("start")], Value::int(200)));
    let c = h.compile(doc.root());
    assert_eq!(c.made.midi_items, 1);
    assert_eq!(c.made.audio_items + c.made.curves, 0);
    assert!(c.builds.is_empty());
    assert_eq!(c.table.nodes, before.nodes);
    assert_eq!(c.table.processors.len(), before.processors.len());
    h.pump();

    // Changing an instrument's non-parameter state asks for one build and keeps the old one playing (T7)
    edit(&mut doc, |e| e.set(t2, &[k("instrument"), k("bypass")], Value::bool(true)));
    let c = h.compile(doc.root());
    assert!(c.builds.is_empty(), "bypass is not the plugin's state");
    h.pump();
    let p2 =
        NodeId::parse(doc.find(t2).unwrap().get("instrument").unwrap().get("id").unwrap().as_text().unwrap()).unwrap();
    edit(&mut doc, |e| e.set(p2, &[k("version")], Value::int(1)));
    let c = h.compile(doc.root());
    assert!(c.builds.is_empty(), "the same value is the same processor");
    h.pump();
}

#[test]
fn an_index_is_never_reused_before_its_return() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let mut core = CompilerCore::new(config(&plugins));
    let env = Environment::default();
    let t = add_track(&mut doc, Some(dc()));
    let a = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    edit(&mut doc, |e| e.remove(t));
    let b = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    let removed: Vec<NodeIndex> =
        a.table.nodes.iter().filter(|(k, _)| b.table.node(**k).is_none()).map(|(_, i)| *i).collect();
    assert_eq!(removed.len(), 3, "instrument, gain, output: {removed:?}");
    assert_eq!(b.table.pending_nodes.len(), removed.len());
    // New tracks while the removal is on its way: none of the pending indices
    for _ in 0..3 {
        add_track(&mut doc, Some(dc()));
    }
    let c = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    let used: HashSet<u32> = c.table.nodes.values().map(|i| i.index).collect();
    for r in &removed {
        assert!(!used.contains(&r.index), "index {} reused before its return", r.index);
    }
    // The return of b frees them: the next compilation may take them (with a new generation)
    core.returned(&Returned { generation: b.generation, freed: removed.clone(), ..Returned::default() });
    add_track(&mut doc, Some(dc()));
    let d = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    let reused: Vec<NodeIndex> =
        d.table.nodes.values().filter(|i| removed.iter().any(|r| r.index == i.index)).copied().collect();
    assert!(!reused.is_empty());
    for i in reused {
        assert!(!removed.contains(&i), "a reused index has a new generation");
    }
}

#[test]
fn slots_wait_for_their_return_too() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let mut core = CompilerCore::new(config(&plugins));
    let env = Environment::default();
    let t = add_track(&mut doc, Some(dc()));
    let a = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    let old: BTreeSet<u32> = a.table.slots_of(t, "volume").iter().map(|s| s.0).collect();
    edit(&mut doc, |e| e.remove(t));
    let b = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    assert!(b.table.pending_slots.iter().any(|(_, s)| old.contains(&s.0)));
    let t2 = add_track(&mut doc, Some(dc()));
    let c = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    assert!(c.table.slots_of(t2, "volume").iter().all(|s| !old.contains(&s.0)));
}

/// Solo: a test graph — tracks A→ch1, B→ch2, C→ch3; ch1 sends to the return R; ch2 → ch4 (a group) → master
#[test]
fn solo_rules() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let ta = add_track(&mut doc, Some(dc()));
    let tb = add_track(&mut doc, Some(dc()));
    let tc = add_track(&mut doc, Some(dc()));
    let c1 = add_channel(&mut doc);
    let c2 = add_channel(&mut doc);
    let c3 = add_channel(&mut doc);
    let c4 = add_channel(&mut doc);
    let ret = add_channel(&mut doc);
    for (t, c) in [(ta, c1), (tb, c2), (tc, c3)] {
        let o = outputs(&doc, t)[0];
        route_output(&mut doc, o, c);
    }
    edit(&mut doc, |e| {
        e.set(c2, &[k("output")], text(&c4.to_string()))?;
        e.insert(c1, "sends", 0, Value::object([("to", text(&ret.to_string())), ("amount", real(0.5))])).map(|_| ())
    });
    let mut core = CompilerCore::new(config(&plugins));
    let env = Environment::default();
    let compile = |core: &mut CompilerCore, doc: &lm_doc::Document| {
        core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap()
    };
    // The node where a channel's solo-mute sits: before the fader (the rack's last segment here)
    let pre = |c: NodeId| NodeKey::Rack(c, Stage::Gain);

    // No solo: nothing muted
    let c = compile(&mut core, &doc);
    assert!(c.nodes.iter().all(|n| !n.muted));

    // Track solo: the other tracks are silenced, the desk stays whole (the return keeps playing)
    edit(&mut doc, |e| e.set(ta, &[k("solo")], Value::bool(true)));
    let c = compile(&mut core, &doc);
    assert!(!muted(&c, NodeKey::TrackGain(ta)));
    assert!(muted(&c, NodeKey::TrackGain(tb)) && muted(&c, NodeKey::TrackGain(tc)));
    for ch in [c1, c2, c3, c4, ret] {
        assert!(!muted(&c, pre(ch)), "{ch}");
    }

    // A folder's solo covers its children
    let folder = add_track(&mut doc, None);
    edit(&mut doc, |e| {
        e.set(folder, &[k("folder")], Value::bool(true))?;
        e.set(ta, &[k("solo")], Value::bool(false))?;
        e.set(tb, &[k("parent")], text(&folder.to_string()))?;
        e.set(folder, &[k("solo")], Value::bool(true))
    });
    let c = compile(&mut core, &doc);
    assert!(!muted(&c, NodeKey::TrackGain(tb)));
    assert!(muted(&c, NodeKey::TrackGain(ta)) && muted(&c, NodeKey::TrackGain(tc)));
    edit(&mut doc, |e| e.set(folder, &[k("solo")], Value::bool(false)));

    // Channel solo on ch2: its sources and its path (ch4, master) play; ch1, ch3 and the return are silenced
    edit(&mut doc, |e| e.set(c2, &[k("solo")], Value::bool(true)));
    let c = compile(&mut core, &doc);
    assert!(c.nodes.iter().filter(|n| matches!(n.key, NodeKey::TrackGain(_))).all(|n| !n.muted));
    assert!(!muted(&c, pre(c2)) && !muted(&c, pre(c4)));
    assert!(muted(&c, pre(c1)) && muted(&c, pre(c3)) && muted(&c, pre(ret)));
    assert!(!c.nodes.iter().any(|n| n.key.owner() == Some(master()) && n.muted));

    // Channel solo on ch1: the return it sends to is on its path
    edit(&mut doc, |e| {
        e.set(c2, &[k("solo")], Value::bool(false))?;
        e.set(c1, &[k("solo")], Value::bool(true))
    });
    let c = compile(&mut core, &doc);
    assert!(!muted(&c, pre(c1)) && !muted(&c, pre(ret)));
    assert!(muted(&c, pre(c2)) && muted(&c, pre(c3)) && muted(&c, pre(c4)));

    // Soloing the group ch4 keeps its source ch2 audible
    edit(&mut doc, |e| {
        e.set(c1, &[k("solo")], Value::bool(false))?;
        e.set(c4, &[k("solo")], Value::bool(true))
    });
    let c = compile(&mut core, &doc);
    assert!(!muted(&c, pre(c4)) && !muted(&c, pre(c2)));
    assert!(muted(&c, pre(c1)) && muted(&c, pre(c3)));

    // Both: the intersection — track B soloed and ch4 soloed: A and C silenced as tracks, ch1/ch3 as channels
    edit(&mut doc, |e| e.set(tb, &[k("solo")], Value::bool(true)));
    let c = compile(&mut core, &doc);
    assert!(!muted(&c, NodeKey::TrackGain(tb)));
    assert!(muted(&c, NodeKey::TrackGain(ta)) && muted(&c, NodeKey::TrackGain(tc)));
    assert!(muted(&c, pre(c1)) && muted(&c, pre(c3)) && !muted(&c, pre(c2)));

    // A sidechain-only source keeps feeding: silenced after its fader, not before
    let duck = insert(&mut doc, c4, plugin("duck", Value::empty_object()));
    edit(&mut doc, |e| {
        e.insert(
            c3,
            "sends",
            0,
            Value::object([("to", text(&duck.to_string())), ("port", text("sidechain")), ("pre", Value::bool(true))]),
        )
        .map(|_| ())
    });
    let c = compile(&mut core, &doc);
    assert!(!muted(&c, pre(c3)), "the sidechain source is not silenced before its fader");
    assert!(muted(&c, NodeKey::Fader(c3)), "but after it");
    let sc = route(&c, pre(c3), NodeKey::Insert(duck)).expect("the sidechain route");
    assert!(sc.sidechain && sc.channels == 2);
}

#[test]
fn pdc_aligns_every_path() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    digital(&mut doc);
    let ta = add_track(&mut doc, Some(dc()));
    let tb = add_track(&mut doc, Some(dc()));
    let ca = add_channel(&mut doc);
    let cb = add_channel(&mut doc);
    let oa = outputs(&doc, ta)[0];
    let ob = outputs(&doc, tb)[0];
    route_output(&mut doc, oa, ca);
    route_output(&mut doc, ob, cb);
    let d1 = insert(&mut doc, ca, delay(1000));
    let mut h = Harness::new(&plugins);
    let c = h.settle(doc.root());
    // ca's path is 1000 late: cb's output into the master waits for it
    assert_eq!(route(&c, NodeKey::Fader(cb), NodeKey::ChannelIn(master())).unwrap().delay, 1000);
    assert_eq!(route(&c, NodeKey::Fader(ca), NodeKey::ChannelIn(master())).unwrap().delay, 0);

    // A second latency inside cb's chain, and a pre-fader send from cb to ca
    let d2 = insert(&mut doc, cb, delay(300));
    edit(&mut doc, |e| {
        e.insert(cb, "sends", 0, Value::object([("to", text(&ca.to_string())), ("pre", Value::bool(true))])).map(|_| ())
    });
    let c = h.settle(doc.root());
    // ca's input: the track (0) and cb's send (300): the track's output waits 300, hoisted before the output node
    assert_eq!(route(&c, NodeKey::Insert(d2), NodeKey::ChannelIn(ca)).unwrap().delay, 0);
    assert_eq!(route(&c, NodeKey::Output(oa), NodeKey::ChannelIn(ca)).unwrap().delay, 0);
    assert_eq!(route(&c, NodeKey::TrackGain(ta), NodeKey::Output(oa)).unwrap().delay, 300);
    // At the master: ca arrives at 1300, cb at 300
    assert_eq!(route(&c, NodeKey::Fader(ca), NodeKey::ChannelIn(master())).unwrap().delay, 0);
    assert_eq!(route(&c, NodeKey::Fader(cb), NodeKey::ChannelIn(master())).unwrap().delay, 1000);
    let _ = d1;

    // And the audio: two notes at the same tick arrive together, 1300 samples late
    for t in [ta, tb] {
        let item = add_item(&mut doc, t, "midi", 0, 3840);
        edit(&mut doc, |e| e.add_note(item, note(60, 480, 480, 0.25)).map(|_| ()));
    }
    // ta plays 0.25 through ca (1300); tb plays 0.25 through cb (300 + 1000) and through the send (300 → ca → 1000)
    h.settle(doc.root());
    h.play();
    let out = h.render(48_000 / BLOCK + 2);
    let on = 480 * 25 + 1300;
    let off = on + 480 * 25;
    for (pos, l, _) in out {
        let want = if (on..off).contains(&pos) { 0.75 } else { 0.0 };
        assert!((l - want).abs() < 1e-6, "at {pos}: {l} (want {want})");
    }
}

#[test]
fn monitoring_is_outside_the_pdc() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    digital(&mut doc);
    let voice = add_track(&mut doc, None);
    let other = add_track(&mut doc, Some(dc()));
    let ch = add_channel(&mut doc);
    let ov = outputs(&doc, voice)[0];
    let oo = outputs(&doc, other)[0];
    route_output(&mut doc, ov, ch);
    route_output(&mut doc, oo, ch);
    let late = add_channel(&mut doc);
    insert(&mut doc, late, delay(500));
    edit(&mut doc, |e| {
        e.set(late, &[k("output")], text(&ch.to_string()))?;
        e.set(
            voice,
            &[k("input")],
            Value::object([("device", text("in")), ("channels", Value::list([Value::int(1)]))]),
        )?;
        e.set(voice, &[k("armed")], Value::bool(true))
    });
    let mut h = Harness::new(&plugins);
    let c = h.settle(doc.root());
    let m = route(&c, NodeKey::Monitor(voice), NodeKey::Output(ov)).expect("monitoring into the main output");
    assert!(m.monitor && m.delay == 0);
    // The channel waits 500 for `late`: the track's share goes before the output node, so the monitoring that
    // joins at the output is not delayed
    assert_eq!(route(&c, NodeKey::Output(ov), NodeKey::ChannelIn(ch)).unwrap().delay, 0);
    assert_eq!(route(&c, NodeKey::TrackAudio(voice), NodeKey::Output(ov)).unwrap().delay, 500);
    assert_eq!(route(&c, NodeKey::Output(oo), NodeKey::ChannelIn(ch)).unwrap().delay, 0);
    assert_eq!(route(&c, NodeKey::TrackGain(other), NodeKey::Output(oo)).unwrap().delay, 500);
    // The monitor has its own slot of the track's volume
    assert_eq!(c.table.slots_of(voice, "volume").len(), 2);
    // Disarmed (auto): no monitoring
    edit(&mut doc, |e| e.set(voice, &[k("armed")], Value::bool(false)));
    let c = h.settle(doc.root());
    assert!(c.table.node(NodeKey::Monitor(voice)).is_none());
    assert_eq!(c.table.slots_of(voice, "volume").len(), 1);
}

#[test]
fn live_midi_reaches_an_armed_instrument() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let t = add_track(&mut doc, Some(dc()));
    edit(&mut doc, |e| e.set(t, &[k("armed")], Value::bool(true)));
    let env = Environment { midi_ports: vec![PortIndex(0)], ..Environment::default() };
    let mut core = CompilerCore::new(config(&plugins));
    let c = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    assert_eq!(c.ports.len(), 1);
    assert!(matches!(c.ports[0], (PortIndex(0), NodeKey::Instrument(_))));
    edit(&mut doc, |e| e.set(t, &[k("armed")], Value::bool(false)));
    let c = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    assert!(c.ports.is_empty());
}

#[test]
fn a_kit_routes_each_port_and_keeps_playing_while_it_changes() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let t = add_track(&mut doc, Some(plugin("kit", Value::object([("kit", text("small"))]))));
    let outs = outputs(&doc, t);
    assert_eq!(outs.len(), 2);
    let kick = add_channel(&mut doc);
    route_output(&mut doc, outs[0], kick);
    let mut h = Harness::new(&plugins);
    let c = h.settle(doc.root());
    let inst = c.table.processors.iter().find(|p| matches!(p.node, NodeKey::Instrument(_))).unwrap().node;
    assert_eq!(route(&c, NodeKey::TrackGain(t), NodeKey::Output(outs[0])).unwrap().channels, 1);
    let oh = route(&c, NodeKey::TrackGain(t), NodeKey::Output(outs[1])).unwrap();
    assert_eq!((oh.from_channel, oh.channels), (1, 2));
    // The kick (mono) into a stereo channel: one channel, up-mixed
    assert_eq!(route(&c, NodeKey::Output(outs[0]), NodeKey::ChannelIn(kick)).unwrap().channels, 1);

    // Another kit: a new port, a build; meanwhile the old kit keeps its shape and plays (T7)
    let p =
        NodeId::parse(doc.find(t).unwrap().get("instrument").unwrap().get("id").unwrap().as_text().unwrap()).unwrap();
    edit(&mut doc, |e| e.set(p, &[k("state"), k("kit")], text("big")));
    assert_eq!(outputs(&doc, t).len(), 3);
    let c = h.compile(doc.root());
    h.pump();
    assert_eq!(c.builds.len(), 1);
    assert_eq!(c.replacing, vec![inst]);
    assert!(c.waiting.is_empty());
    let room = outputs(&doc, t)[2];
    assert!(route(&c, NodeKey::TrackGain(t), NodeKey::Output(room)).is_none(), "the old kit has no room port");
    // Built: replaced in the next install, with a tail node for the old one
    let b = c.builds[0].clone();
    assert!(h.core.ready(b.ticket, build(&plugins, &b)));
    let c = h.compile(doc.root());
    h.pump();
    assert!(c.complete());
    assert_eq!(c.table.tails.len(), 1);
    assert!(route(&c, NodeKey::TrackGain(t), NodeKey::Output(room)).is_some());
    // The tail ends: it is removed and its index comes back
    let tail = c.table.tails[0].index;
    assert!(h.core.tail_ended(tail));
    let c = h.compile(doc.root());
    h.pump();
    assert!(c.table.tails.is_empty());
    assert!(c.table.pending_nodes.contains(&tail) || h.returns > 0);
}

/// C-3: a tail whose `TailEnded` was lost is dropped by the audio clock, never by the wall clock, and the same
/// way however the clock is stepped
#[test]
fn a_tail_without_its_end_event_leaves_by_the_audio_clock() {
    let run = |steps: &[u64]| {
        let plugins = plugins();
        let mut doc = new_doc(&plugins);
        let t = add_track(&mut doc, Some(plugin("kit", Value::object([("kit", text("small"))]))));
        let mut h = Harness::new(&plugins);
        h.settle(doc.root());
        let p = NodeId::parse(doc.find(t).unwrap().get("instrument").unwrap().get("id").unwrap().as_text().unwrap())
            .unwrap();
        edit(&mut doc, |e| e.set(p, &[k("state"), k("kit")], text("big")));
        let c = h.compile(doc.root());
        h.pump();
        let b = c.builds[0].clone();
        assert!(h.core.ready(b.ticket, build(&plugins, &b)));
        let c = h.compile(doc.root());
        h.pump();
        assert_eq!(c.table.tails.len(), 1);
        let net = lm_compile::TAIL_NET_SECONDS * u64::from(RATE);
        let mut at = 0;
        for s in steps {
            at += s;
            h.core.observe_frames(at);
        }
        assert_eq!(at, net + 1);
        assert!(h.core.has_pending_work());
        let c = h.compile(doc.root());
        h.pump();
        c.table.tails.len()
    };
    let net = lm_compile::TAIL_NET_SECONDS * u64::from(RATE);
    assert_eq!(run(&[net + 1]), 0);
    assert_eq!(run(&[1, 480, 777, net - 1257]), 0);
}

#[test]
fn replacing_a_note_effect_installs_it_without_a_tail() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let t = add_track(&mut doc, Some(plugin("probe", Value::empty_object())));
    let mut fx = None;
    edit(&mut doc, |e| {
        fx = Some(e.insert(t, "noteEffects", 0, plugin("nfx", Value::object([("mode", Value::int(0))])))?);
        Ok(())
    });
    let fx = fx.unwrap();
    let mut h = Harness::new(&plugins);
    h.settle(doc.root());
    // Another state: a new build replaces the installed one; a note effect leaves no sound, so no tail was planned
    edit(&mut doc, |e| e.set(fx, &[k("state"), k("mode")], Value::int(1)));
    let c = h.settle(doc.root());
    assert!(c.complete());
    assert!(c.table.tails.is_empty());
}

#[test]
fn a_stale_build_is_refused() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let ch = add_channel(&mut doc);
    let d = insert(&mut doc, ch, delay(10));
    let mut core = CompilerCore::new(config(&plugins));
    let env = Environment::default();
    let a = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    let first = a.builds.iter().find(|b| b.node == NodeKey::Insert(d)).unwrap().clone();
    assert!(a.waiting.contains(&NodeKey::Insert(d)));
    edit(&mut doc, |e| e.set(d, &[k("state"), k("latency")], Value::int(20)));
    let b = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    assert!(b.cancelled.contains(&first.ticket));
    assert!(!core.ready(first.ticket, build(&plugins, &first)), "superseded");
    let second = b.builds.iter().find(|x| x.node == NodeKey::Insert(d)).unwrap().clone();
    assert!(core.ready(second.ticket, build(&plugins, &second)));
    let c = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    // Built: its latency counts now
    assert_eq!(c.nodes.iter().find(|n| n.key == NodeKey::Insert(d)).unwrap().latency, 20);
}

#[test]
fn inserts_sit_at_their_points() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let ch = add_channel(&mut doc);
    let with_point = |p: &str| plugin("delay", Value::object([("latency", Value::int(0))])).with("point", text(p));
    let a = insert(&mut doc, ch, with_point("post-gain"));
    let b = insert(&mut doc, ch, with_point("post-dynamics"));
    let c_ = insert(&mut doc, ch, with_point("pre-pan"));
    let d = insert(&mut doc, ch, plugin("delay", Value::object([("latency", Value::int(0))])));
    let mut core = CompilerCore::new(config(&plugins));
    let c = core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    let chain = [
        NodeKey::ChannelIn(ch),
        NodeKey::Rack(ch, Stage::Gain),
        NodeKey::Insert(a),
        NodeKey::Rack(ch, Stage::Filter),
        NodeKey::Insert(b),
        NodeKey::Rack(ch, Stage::Eq),
        NodeKey::Insert(d),
        NodeKey::Fader(ch),
        NodeKey::Insert(c_),
        NodeKey::Pan(ch),
    ];
    for w in chain.windows(2) {
        assert!(route(&c, w[0], w[1]).is_some(), "{:?} → {:?}", w[0], w[1]);
    }
    // The filter segment runs the filters and the dynamics; its parameters are bound to it
    let rack = c.table.processors.len();
    let _ = rack;
    assert_eq!(InsertPoint::parse(Some("post-gain")), InsertPoint::PostGain);
    // The slots of the rack: one per real field
    for p in rack_params(&schemas(&plugins), false).iter().filter(|p| p.section != "channel") {
        assert_eq!(c.table.slots_of(ch, &p.key).len(), 1, "{}", p.key);
    }
    // No channel-level controls: no input nor polarity node
    assert!(
        c.table.node(NodeKey::Rack(ch, Stage::Input)).is_none()
            && c.table.node(NodeKey::Rack(ch, Stage::Polarity)).is_none()
    );
    // Digital: no rack nodes and no rack slots (P-17)
    digital(&mut doc);
    let c = core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    assert!(c.table.nodes.keys().all(|k| !matches!(k, NodeKey::Rack(..))));
    assert!(c.table.slots_of(ch, "rack.tube.drive").is_empty());
    assert!(route(&c, NodeKey::ChannelIn(ch), NodeKey::Insert(a)).is_some());
    // The legacy strip's TRIM and MONO before everything, phase and swap before the fader: in the digital desk too
    doc.transact(lm_base::Label::new("test"), |e| {
        e.set(ch, &[lm_state::PathStep::key("trim")], Value::real(-6.0)?)?;
        e.set(ch, &[lm_state::PathStep::key("swap")], Value::bool(true))
    })
    .unwrap();
    let c = core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    let chain = [
        NodeKey::ChannelIn(ch),
        NodeKey::Rack(ch, Stage::Input),
        NodeKey::Insert(a),
        NodeKey::Insert(b),
        NodeKey::Insert(d),
        NodeKey::Rack(ch, Stage::Polarity),
        NodeKey::Fader(ch),
    ];
    for w in chain.windows(2) {
        assert!(route(&c, w[0], w[1]).is_some(), "{:?} → {:?}", w[0], w[1]);
    }
    assert_eq!(c.table.slots_of(ch, "trim").len(), 1, "TRIM has its slot");
    // The monitoring: only when the Environment has it (the session's live program, never an export)
    let env = Environment { monitor: Some(Value::object([("dim", Value::bool(true))])), ..Environment::default() };
    let c = core.compile(doc.root(), &lm_compile::NoResources, &env).unwrap();
    assert!(c.table.node(NodeKey::Rack(master(), Stage::MasterTransformer)).is_some());
    assert_eq!(c.table.slots_of(master(), "monitor.level").len(), 1);
}

#[test]
fn the_limits_are_refused_whole() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    for _ in 0..40 {
        add_track(&mut doc, Some(dc()));
    }
    let mut cfg = config(&plugins);
    cfg.limits.max_nodes = 100;
    let mut core = CompilerCore::new(cfg);
    let e = core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap_err();
    assert_eq!(e.key(), "compile.limits.nodes");
    assert!(core.usage(doc.root()).unwrap().nodes > 100);
    // Nothing changed: a smaller project still compiles at generation 1
    let small = new_doc(&plugins);
    let c = core.compile(small.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    assert_eq!(c.generation.0, 1);
}

struct Loaded(BTreeMap<String, Arc<SampleData>>);

impl Resources for Loaded {
    fn audio(&self, r: &Resource) -> ResourceState {
        match self.0.get(r.key()) {
            Some(d) => ResourceState::Ready(d.clone()),
            None => ResourceState::Loading,
        }
    }
}

#[test]
fn audio_items_play_their_comp_with_fades() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    digital(&mut doc);
    let t = add_track(&mut doc, None);
    edit(&mut doc, |e| e.set(t, &[k("channels")], Value::int(1)));
    let o = outputs(&doc, t)[0];
    edit(&mut doc, |e| e.set(o, &[k("channels")], Value::int(1)));
    let take_a = Resource::of_content("audio", b"take a").unwrap();
    let take_b = Resource::of_content("audio", b"take b").unwrap();
    let item = add_item(&mut doc, t, "audio", 0, 960);
    let mut lanes = Vec::new();
    edit(&mut doc, |e| {
        for r in [&take_a, &take_b] {
            lanes.push(e.insert(
                item,
                "lanes",
                lanes.len(),
                Value::object([("source", Value::resource(r.clone()))]),
            )?);
        }
        // Lane a for ticks 0..480, lane b for 480..960, with a 100-sample crossfade
        let comp = Value::list([
            Value::object([("from", Value::int(0)), ("to", Value::int(480)), ("lane", text(&lanes[0].to_string()))]),
            Value::object([
                ("from", Value::int(480)),
                ("to", Value::int(960)),
                ("lane", text(&lanes[1].to_string())),
                ("fade", Value::int(100)),
            ]),
        ]);
        e.set(item, &[k("comp")], comp)
    });
    let mut h = Harness::new(&plugins);
    let c = h.compile(doc.root());
    h.pump();
    assert_eq!(c.waiting_items.len(), 2, "both takes are loading");
    let ones = Arc::new(SampleData::from_channels(RATE, vec![vec![0.5; 48_000]]).unwrap());
    let twos = Arc::new(SampleData::from_channels(RATE, vec![vec![0.25; 48_000]]).unwrap());
    h.resources = Arc::new(Loaded(BTreeMap::from([(take_a.key().to_owned(), ones), (take_b.key().to_owned(), twos)])));
    let c = h.settle(doc.root());
    assert!(c.waiting_items.is_empty());
    h.play();
    let out = h.render(48_000 / BLOCK);
    let edge = 480 * 25;
    for (pos, l, r) in out {
        assert_eq!(l, r, "a mono output up-mixed into the stereo master");
        let mono = l * std::f32::consts::SQRT_2;
        let want = if pos < edge {
            0.5
        } else if pos < edge + 100 {
            // Crossfade: a fades out over 100 samples, b fades in
            let x = (pos - edge) as f32;
            0.5 * ((99.0 - x).max(0.0) / 100.0) + 0.25 * (x / 100.0)
        } else if pos < 960 * 25 {
            0.25
        } else {
            0.0
        };
        assert!((mono - want).abs() < 1e-5, "at {pos}: {mono} (want {want})");
    }
}

#[test]
fn tempo_ramps_reach_the_curves_at_exact_samples() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let t = add_track(&mut doc, Some(dc()));
    edit(&mut doc, |e| {
        e.set(
            ROOT,
            &[k("tempoMap")],
            Value::list([
                Value::object([("tick", Value::int(0)), ("bpm", text("60")), ("ramp", Value::bool(true))]),
                Value::object([("tick", Value::int(3840)), ("bpm", text("180"))]),
            ]),
        )?;
        e.insert(
            t,
            "automation",
            0,
            Value::object([
                ("target", Value::object([("node", text(&t.to_string())), ("param", text("volume"))])),
                (
                    "points",
                    Value::list([1000, 2000, 3840, 5000].iter().map(|tick| {
                        Value::object([("tick", Value::int(*tick)), ("value", real(f64::from(*tick as i32) / 5000.0))])
                    })),
                ),
            ]),
        )
        .map(|_| ())
    });
    let mut core = CompilerCore::new(config(&plugins));
    let c = core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    assert_eq!(c.made.curves, 1);
    let map = lm_time::TempoMap::new(
        RATE,
        lm_time::Ppq::DEFAULT,
        &[
            lm_time::TempoPoint::new(0, lm_time::Bpm::from_integer(60).unwrap(), lm_time::TimeSignature::COMMON)
                .linear(),
            lm_time::TempoPoint::new(3840, lm_time::Bpm::from_integer(180).unwrap(), lm_time::TimeSignature::COMMON),
        ],
    )
    .unwrap();
    let curve = core.curve(t, "volume").expect("the curve").clone();
    let samples: Vec<i64> = curve.points().iter().map(|p| p.sample).collect();
    let want: Vec<i64> = [1000, 2000, 3840, 5000].iter().map(|t| map.sample_at(*t)).collect();
    assert_eq!(samples, want);
    // A ramp is not a constant tempo: the samples are not proportional to the ticks
    assert_ne!(want[1] - want[0], want[0]);
    // A tempo edit makes the curve again, and relocates
    edit(&mut doc, |e| e.set(ROOT, &[k("tempoMap"), lm_state::PathStep::Index(1), k("bpm")], text("120")));
    let c = core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    assert_eq!(c.made.curves, 1);
}

#[test]
fn a_compiler_rebuilt_from_the_table_goes_on() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    add_track(&mut doc, Some(dc()));
    let mut h = Harness::new(&plugins);
    let c = h.settle(doc.root());
    let table = (*c.table).clone();
    // The old compiler dies; the new one continues from the table without touching the installed nodes
    h.core = CompilerCore::from_table(config(&plugins), &table);
    add_track(&mut doc, Some(dc()));
    let c = h.compile(doc.root());
    h.pump();
    for (k, i) in &table.nodes {
        assert_eq!(c.table.node(*k), Some(*i), "{k:?}");
    }
    assert!(c.generation > table.generation);
    // Only the new track waits for a build
    assert_eq!(c.builds.len(), 1);
}

#[test]
fn random_projects_compile_and_play() {
    let plugins = plugins();
    let mut rng = Rng(std::env::var("LM_TEST_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x5eed));
    for round in 0..6 {
        let mut doc = new_doc(&plugins);
        let mut h = Harness::new(&plugins);
        let mut seen: BTreeMap<u32, (NodeIndex, bool)> = BTreeMap::new();
        for step in 0..40 {
            random_edit(&mut doc, &mut rng);
            let c = h.compile(doc.root());
            // Each live index is used by one node, and an index that was removed came back before its reuse
            let live: Vec<NodeIndex> =
                c.table.nodes.values().copied().chain(c.table.tails.iter().map(|t| t.index)).collect();
            let unique: HashSet<u32> = live.iter().map(|i| i.index).collect();
            assert_eq!(unique.len(), live.len(), "round {round} step {step}");
            for i in &live {
                if let Some((old, _)) = seen.get(&i.index)
                    && *old != *i
                {
                    assert!(!c.table.pending_nodes.iter().any(|p| p.index == i.index));
                }
                seen.insert(i.index, (*i, true));
            }
            h.build_some(|| rng.below(3) > 0);
            h.pump();
            if step % 7 == 0 {
                h.play();
            }
        }
        h.settle(doc.root());
        h.render(4);
    }
}

#[derive(Debug)]
pub struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn ids(doc: &lm_doc::Document, list: &[&str]) -> Vec<NodeId> {
    let mut v = doc.root().clone();
    for k in list {
        v = v.get(k).cloned().unwrap_or_default();
    }
    v.items().filter_map(|x| NodeId::parse(x.get("id")?.as_text()?).ok()).collect()
}

fn random_edit(doc: &mut lm_doc::Document, rng: &mut Rng) {
    let tracks = ids(doc, &["tracks"]);
    let channels = ids(doc, &["mixer", "channels"]);
    let pick = |rng: &mut Rng, v: &[NodeId]| (!v.is_empty()).then(|| v[rng.below(v.len())]);
    let _ = doc.transact(lm_base::Label::new("test.random"), |e| {
        match rng.below(14) {
            0 | 1 => {
                let inst = match rng.below(3) {
                    0 => None,
                    1 => Some(dc()),
                    _ => Some(plugin(
                        "kit",
                        Value::object([("kit", text(if rng.below(2) == 0 { "small" } else { "big" }))]),
                    )),
                };
                let mut node = Value::object([("name", text("r"))]);
                if let Some(i) = inst {
                    node = node.with("instrument", i);
                }
                e.insert(ROOT, "tracks", tracks.len(), node)?;
            }
            2 => {
                e.insert(ROOT, "mixer.channels", channels.len(), Value::object([("name", text("c"))]))?;
            }
            3 => {
                if let Some(t) = pick(rng, &tracks) {
                    e.remove(t)?;
                }
            }
            4 => {
                if let Some(c) = pick(rng, &channels) {
                    e.remove(c)?;
                }
            }
            5 => {
                if let (Some(t), Some(c)) = (pick(rng, &tracks), pick(rng, &channels)) {
                    let outs = e.find(t).and_then(|t| t.get("outputs").cloned()).unwrap_or_default();
                    if let Some(o) = outs.items().next().and_then(|o| NodeId::parse(o.get("id")?.as_text()?).ok()) {
                        e.set(o, &[k("to")], text(&c.to_string()))?;
                    }
                }
            }
            6 => {
                if let Some(c) = pick(rng, &channels) {
                    let points = ["pre-gain", "post-gain", "post-filter", "post-dynamics", "post-eq", "pre-pan"];
                    let p = delay(rng.below(3) as i64 * 64).with("point", text(points[rng.below(points.len())]));
                    let n = e.find(c).and_then(|c| c.get("inserts")).map_or(0, Value::len);
                    e.insert(c, "inserts", n, p)?;
                }
            }
            7 => {
                if let (Some(a), Some(b)) = (pick(rng, &channels), pick(rng, &channels)) {
                    e.insert(
                        a,
                        "sends",
                        0,
                        Value::object([("to", text(&b.to_string())), ("pre", Value::bool(rng.below(2) == 0))]),
                    )?;
                }
            }
            8 => {
                if let Some(t) = pick(rng, &tracks) {
                    let v = !e.find(t).and_then(|t| t.get("solo")).and_then(Value::as_bool).unwrap_or(false);
                    e.set(t, &[k("solo")], Value::bool(v))?;
                }
            }
            9 => {
                if let Some(c) = pick(rng, &channels) {
                    let v = !e.find(c).and_then(|c| c.get("solo")).and_then(Value::as_bool).unwrap_or(false);
                    e.set(c, &[k("solo")], Value::bool(v))?;
                }
            }
            10 | 11 => {
                if let Some(t) = pick(rng, &tracks) {
                    let n = e.find(t).and_then(|t| t.get("items")).map_or(0, Value::len);
                    let item = e.insert(
                        t,
                        "items",
                        n,
                        Value::object([("kind", text("midi")), ("start", Value::int(rng.below(4000) as i64))]),
                    )?;
                    for _ in 0..rng.below(20) {
                        e.add_note(
                            item,
                            note(36 + rng.below(60) as i64, rng.below(3000) as i64, 1 + rng.below(500) as i64, 0.7),
                        )?;
                    }
                }
            }
            12 => {
                if let Some(t) = pick(rng, &tracks) {
                    e.set(t, &[k("armed")], Value::bool(rng.below(2) == 0))?;
                    e.set(t, &[k("input")], Value::object([("channels", Value::list([Value::int(1)]))]))?;
                }
            }
            _ => {
                if let Some(c) = pick(rng, &channels) {
                    e.set(c, &[k("volume")], real(rng.below(100) as f64 / 50.0))?;
                }
            }
        }
        Ok(())
    });
}

#[test]
fn the_usage_matches_the_program() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let mut rng = Rng(7);
    for _ in 0..30 {
        random_edit(&mut doc, &mut rng);
    }
    let mut core = CompilerCore::new(config(&plugins));
    let u = core.usage(doc.root()).unwrap();
    let c = core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    assert_eq!(u.nodes, c.nodes.len());
    assert_eq!(u.params, c.table.slots.values().map(Vec::len).sum::<usize>());
}

#[test]
fn the_compile_thread_keeps_only_the_newest_root() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let mut compiler = lm_compile::Compiler::new(config(&plugins));
    let res: Arc<dyn Resources> = Arc::new(lm_compile::NoResources);
    let mut roots = Vec::new();
    for _ in 0..20 {
        add_track(&mut doc, Some(dc()));
        roots.push(doc.root().clone());
    }
    for r in &roots {
        compiler.submit(r.clone(), res.clone(), Environment::default());
    }
    let mut last = None;
    let mut builds = Vec::new();
    while let Some(r) = compiler.wait(std::time::Duration::from_millis(500)) {
        let r = r.expect("compiled");
        builds.extend(r.builds.iter().cloned());
        builds.retain(|b: &lm_compile::BuildRequest| !r.cancelled.contains(&b.ticket));
        last = Some(r);
    }
    let last = last.expect("at least one result");
    // The last result is the newest root, and fewer compilations than roots ran
    assert_eq!(last.table.nodes.keys().filter(|k| matches!(k, NodeKey::Instrument(_))).count(), 20);
    assert!(last.generation.0 <= 20);
    // Builds handed back on the thread: a recompilation installs them without a new root
    for b in &builds {
        compiler.ready(b.ticket, build(&plugins, b));
    }
    // The cores arrive in batches: the recompilations go on until nothing waits
    let mut next = compiler.wait(wait()).expect("a recompilation").expect("compiled");
    while !next.complete() {
        next = compiler.wait(wait()).expect("a recompilation").expect("compiled");
    }
    assert_eq!(next.waiting.len(), 0);
}

/// The raw device output from the next block on (channel 0), `blocks` blocks
fn raw(h: &mut Harness, blocks: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(blocks * BLOCK);
    for _ in 0..blocks {
        h.pump();
        out.extend_from_slice(&h.dummy.outputs()[0][..BLOCK]);
    }
    out
}

#[test]
fn the_count_in_clicks_on_exact_samples_and_the_song_starts_after_it() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    digital(&mut doc);
    let t = add_track(&mut doc, Some(dc()));
    let item = add_item(&mut doc, t, "midi", 0, 3840);
    edit(&mut doc, |e| {
        e.add_note(item, note(60, 0, 3840, 0.5))?;
        // One bar of 3/4 at 120 BPM: three beats of 24,000 samples
        e.set(
            ROOT,
            &[k("tempoMap"), lm_state::PathStep::Index(0), k("timeSig")],
            Value::list([Value::int(3), Value::int(4)]),
        )?;
        e.set(ROOT, &[k("transport"), k("metronome"), k("countIn")], Value::int(1))
    });
    let accent = Arc::new(SampleData::from_channels(RATE, vec![vec![1.0; 10]]).unwrap());
    let beat = Arc::new(SampleData::from_channels(RATE, vec![vec![0.5; 10]]).unwrap());
    let mut h = Harness::new(&plugins);
    h.env.metronome_sounds = Some((accent, beat));

    // Playing without recording: no count-in unless the preference says so
    h.settle(doc.root());
    h.play();
    let out = raw(&mut h, 4);
    assert_eq!(out[0], 0.5, "the song starts at once");
    h.control.commands().send(lm_render::Command::Stop).unwrap();
    h.control.commands().send(lm_render::Command::Locate { sample: 0 }).unwrap();
    h.pump();

    h.env.count_in_on_play = true;
    h.settle(doc.root());
    h.play();
    let total = 3 * 24_000;
    let out = raw(&mut h, (total + 4_000) / BLOCK);
    let gain = 0.8; // the metronome's default volume
    for (i, v) in out.iter().enumerate() {
        let want = if i >= total {
            0.5
        } else {
            let beat_at = i / 24_000 * 24_000;
            if i - beat_at < 10 { if beat_at == 0 { gain } else { 0.5 * gain } } else { 0.0 }
        };
        assert!((v - want).abs() < 1e-6, "at {i}: {v} (want {want})");
    }
    // Recording armed starts with the count-in too
    h.control.commands().send(lm_render::Command::Stop).unwrap();
    h.control.commands().send(lm_render::Command::Locate { sample: 0 }).unwrap();
    h.pump();
    h.env.count_in_on_play = false;
    h.settle(doc.root());
    h.control.commands().send(lm_render::Command::Record { on: true }).unwrap();
    h.play();
    let out = raw(&mut h, (total + 1_000) / BLOCK);
    assert_eq!(out[0], gain);
    assert_eq!(out[total - 1], 0.0);
    assert_eq!(out[total], 0.5);
}

#[test]
fn low_latency_monitoring_bypasses_the_inserts_with_latency_on_the_armed_path() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    digital(&mut doc);
    let voice = add_track(&mut doc, None);
    let other = add_track(&mut doc, Some(dc()));
    let vox = add_channel(&mut doc);
    let bus = add_channel(&mut doc);
    let drums = add_channel(&mut doc);
    let (ov, oo) = (outputs(&doc, voice)[0], outputs(&doc, other)[0]);
    route_output(&mut doc, ov, vox);
    route_output(&mut doc, oo, drums);
    edit(&mut doc, |e| e.set(vox, &[k("output")], text(&bus.to_string())));
    let slow_vox = insert(&mut doc, vox, delay(256));
    let none_vox = insert(&mut doc, vox, delay(0));
    let slow_bus = insert(&mut doc, bus, delay(128));
    let slow_drums = insert(&mut doc, drums, delay(512));
    let slow_master = insert(&mut doc, lm_doc::MASTER, delay(64));
    edit(&mut doc, |e| {
        e.set(voice, &[k("input")], Value::object([("channels", Value::list([Value::int(1)]))]))?;
        e.set(voice, &[k("armed")], Value::bool(true))
    });
    let mut h = Harness::new(&plugins);
    h.env.low_latency_monitoring = true;
    let c = h.settle(doc.root());
    let mut bypassed = c.low_latency_bypassed.clone();
    bypassed.sort();
    let mut want = vec![NodeKey::Insert(slow_vox), NodeKey::Insert(slow_bus), NodeKey::Insert(slow_master)];
    want.sort();
    assert_eq!(bypassed, want);
    // Out of the chain (still built, so turning the preference off does not wait), the others in it
    assert!(route(&c, NodeKey::ChannelIn(vox), NodeKey::Insert(slow_vox)).is_none());
    assert!(route(&c, NodeKey::ChannelIn(vox), NodeKey::Insert(none_vox)).is_some());
    assert!(route(&c, NodeKey::ChannelIn(drums), NodeKey::Insert(slow_drums)).is_some());
    assert!(c.table.node(NodeKey::Insert(slow_vox)).is_some());
    // The path has no PDC left from them: the bus and vox paths add no delay at the master
    assert_eq!(route(&c, NodeKey::Fader(bus), NodeKey::ChannelIn(lm_doc::MASTER)).unwrap().delay, 512);
    // Disarmed: everything back, nothing to build
    edit(&mut doc, |e| e.set(voice, &[k("armed")], Value::bool(false)));
    let c = h.compile(doc.root());
    assert!(c.low_latency_bypassed.is_empty() && c.builds.is_empty());
    assert!(route(&c, NodeKey::ChannelIn(vox), NodeKey::Insert(slow_vox)).is_some());
    // The preference off: nothing bypassed even when armed
    h.pump();
    edit(&mut doc, |e| e.set(voice, &[k("armed")], Value::bool(true)));
    h.env.low_latency_monitoring = false;
    let c = h.compile(doc.root());
    assert!(c.low_latency_bypassed.is_empty());
}

/// A store that keeps one long resource on the disk (P-32)
struct Long {
    resource: Resource,
    frames: u64,
}

impl Resources for Long {
    fn audio(&self, r: &Resource) -> ResourceState {
        if *r == self.resource {
            ResourceState::OnDisk { rate: RATE, channels: 1, frames: self.frames }
        } else {
            ResourceState::Loading
        }
    }
}

#[test]
fn a_long_item_streams_from_the_disk() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    digital(&mut doc);
    let t = add_track(&mut doc, None);
    edit(&mut doc, |e| e.set(t, &[k("channels")], Value::int(1)));
    let o = outputs(&doc, t)[0];
    edit(&mut doc, |e| e.set(o, &[k("channels")], Value::int(1)));
    let frames = 40 * RATE as usize;
    let data: Vec<f32> = (0..frames).map(|i| ((i % 1000) as f32 / 1000.0) - 0.5).collect();
    let file = SampleData::from_channels(RATE, vec![data]).unwrap();
    let resource = Resource::of_content("audio", b"a long take").unwrap();
    // 40 s at 120 BPM: 1920 ticks per second... the item is 30 s long, from 1 s into the take
    let item = add_item(&mut doc, t, "audio", 0, 30 * 1920);
    edit(&mut doc, |e| {
        e.insert(
            item,
            "lanes",
            0,
            Value::object([("source", Value::resource(resource.clone())), ("sourceOffset", Value::int(48_000))]),
        )
        .map(|_| ())
    });
    let mut h = Harness::new(&plugins);
    h.resources = Arc::new(Long { resource: resource.clone(), frames: frames as u64 });
    let mut c = h.settle(doc.root());
    assert_eq!(c.streams.len(), 1);
    let reg = c.streams.pop().unwrap();
    assert_eq!(reg.resource, resource);
    let feeder = reg.feeder;
    // The disk, synchronously: everything wanted, now
    let serve = |f: &lm_render::StreamFeeder| {
        let chunk = f.chunk_frames();
        while let Some(ch) = f.next_request() {
            let from = ch as usize * chunk;
            let to = (from + chunk).min(file.frames());
            f.write(ch, &file.channel(0)[from..to]).unwrap();
        }
    };
    serve(&feeder);
    assert!(!h.core.streams_pending());
    h.play();
    for _ in 0..200 {
        serve(&feeder);
        let pos = h.dummy.renderer().position();
        let playing = h.dummy.renderer().is_playing();
        h.pump();
        if playing {
            for (i, v) in h.dummy.outputs()[0][..BLOCK].iter().enumerate() {
                let p = pos as usize + i;
                let want = file.channel(0)[48_000 + p] * (1.0 / 2f32.sqrt());
                assert_eq!(*v, want, "at {p}");
            }
        }
    }
    // An unrelated edit keeps the stream (no new registration, the feeder still wanted)
    add_channel(&mut doc);
    let c = h.compile(doc.root());
    h.pump();
    assert!(c.streams.is_empty());
    assert!(!feeder.is_abandoned());
    // The item goes: once the old program is back, the feeder is abandoned
    edit(&mut doc, |e| e.remove(item));
    h.compile(doc.root());
    h.pump();
    h.pump();
    assert!(feeder.is_abandoned());
}

#[test]
fn a_bypassed_plugin_is_neither_built_nor_run() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    digital(&mut doc);
    let t = add_track(&mut doc, Some(dc()));
    let item = add_item(&mut doc, t, "midi", 0, 3840);
    edit(&mut doc, |e| e.add_note(item, note(60, 0, 960, 0.5)).map(|_| ()));
    let ch = add_channel(&mut doc);
    let o = outputs(&doc, t)[0];
    route_output(&mut doc, o, ch);
    let fx = insert(&mut doc, ch, delay(100));
    let p =
        NodeId::parse(doc.find(t).unwrap().get("instrument").unwrap().get("id").unwrap().as_text().unwrap()).unwrap();
    edit(&mut doc, |e| {
        e.set(p, &[k("bypass")], Value::bool(true))?;
        e.set(fx, &[k("bypass")], Value::bool(true))
    });
    let mut h = Harness::new(&plugins);
    let c = h.compile(doc.root());
    h.pump();
    assert!(c.builds.is_empty(), "nothing to build");
    assert!(c.table.node(NodeKey::Instrument(p)).is_none() && c.table.node(NodeKey::Insert(fx)).is_none());
    assert!(c.table.slots_of(p, "level").is_empty());
    // The chain goes past the insert; the track's output stays (silent)
    assert!(route(&c, NodeKey::ChannelIn(ch), NodeKey::Fader(ch)).is_some());
    assert!(c.table.node(NodeKey::Output(o)).is_some());
    h.play();
    assert!(h.render(20).iter().all(|(_, l, r)| *l == 0.0 && *r == 0.0));
    // Back: built, with the loading of any new node
    edit(&mut doc, |e| {
        e.set(p, &[k("bypass")], Value::bool(false))?;
        e.set(fx, &[k("bypass")], Value::bool(false))
    });
    let c = h.compile(doc.root());
    h.pump();
    assert_eq!(c.builds.len(), 2);
    assert!(c.waiting.contains(&NodeKey::Instrument(p)) && c.waiting.contains(&NodeKey::Insert(fx)));
}
