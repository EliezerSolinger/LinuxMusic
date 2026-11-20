//! The fake as a CorePort: the document is real (transactions, refusals, undo, redo, gestures, feeds), the engine
//! is made up but coherent

use std::time::Duration;

use super::*;

fn small() -> FakePort {
    FakePort::new(Config { tracks: 12, slow_delay: None, slow_track: 2 }).expect("the demo builds")
}

fn track(port: &FakePort, i: usize) -> NodeId {
    NodeId::parse(port.root().get("tracks").unwrap().at(i).unwrap().get("id").unwrap().as_text().unwrap()).unwrap()
}

#[test]
fn the_demo_opens_with_nothing_to_undo_and_unmodified() {
    let p = small();
    assert!(!p.can_undo());
    assert!(!p.modified());
    assert_eq!(p.root().get("tracks").unwrap().len(), 12);
    assert!(p.root().get("mixer").unwrap().get("channels").unwrap().len() >= 7);
}

#[test]
fn apply_is_one_step_and_redo_works() {
    let mut p = small();
    let (a, b) = (track(&p, 1), track(&p, 2));
    let r = p.apply(
        Label::new("Two"),
        vec![EditOp::set(a, &["volume"], Value::real(0.5).unwrap()), EditOp::set(b, &["mute"], Value::bool(true))],
    );
    assert!(r.ok().is_some());
    p.pump();
    assert!(p.modified());
    assert_eq!(p.find(a).unwrap().get("volume").unwrap().as_real(), Some(0.5));
    assert!(p.undo().ok().is_some());
    assert_eq!(p.find(a).unwrap().get("volume").unwrap().as_real(), Some(1.0));
    assert_eq!(p.find(b).unwrap().get("mute").unwrap().as_bool(), Some(false));
    assert!(p.can_redo());
    assert!(p.redo().ok().is_some());
    assert_eq!(p.find(b).unwrap().get("mute").unwrap().as_bool(), Some(true));
    let _ = p.run_action(ActionId::from("project.save"), Value::NULL);
    assert!(!p.modified());
}

#[test]
fn a_bad_list_changes_nothing() {
    let mut p = small();
    let a = track(&p, 1);
    let r = p.apply(
        Label::new("Bad"),
        vec![EditOp::set(a, &["volume"], Value::real(0.5).unwrap()), EditOp::set(a, &["volume"], Value::text("loud"))],
    );
    assert!(matches!(r, Pending::Ready(Err(_))));
    assert_eq!(p.find(a).unwrap().get("volume").unwrap().as_real(), Some(1.0));
}

#[test]
fn temporary_refs_name_what_the_same_list_created() {
    let mut p = small();
    let t = track(&p, 3);
    let ops = vec![
        EditOp::Insert {
            temp: 1,
            parent: Ref::Id(t),
            list: "items".into(),
            index: 0,
            node: Value::object([("kind", Value::text("midi")), ("length", Value::int(3840))]),
        },
        EditOp::Set { id: Ref::Temp(1), path: vec![PathStep::key("name")], value: Value::text("Novo") },
        EditOp::AddNote { temp: 7, item: Ref::Temp(1), note: Value::object([("key", Value::int(64))]) },
        EditOp::SetNote {
            item: Ref::Temp(1),
            n: NoteRef::Temp(7),
            path: vec![PathStep::key("velocity")],
            value: Value::real(0.5).unwrap(),
        },
    ];
    let applied = p.apply(Label::new("New item"), ops).ok().expect("applied");
    let item = applied.created[0].1;
    let n = applied.notes[0].1;
    let v = p.find(item).unwrap();
    assert_eq!(v.get("name").unwrap().as_text(), Some("Novo"));
    assert_eq!(v.get("notes").unwrap().note(n).unwrap().get("velocity").unwrap().as_real(), Some(0.5));
}

#[test]
fn a_gesture_is_one_undo_step_and_feeds_see_it() {
    let mut p = small();
    let a = track(&p, 1);
    let mut feed = p.feed(Topic::Node(a));
    let g = p.begin_gesture(Label::new("Volume"));
    for v in [0.9, 0.8, 0.7] {
        p.gesture_apply(&g, vec![EditOp::set(a, &["volume"], Value::real(v).unwrap())]);
    }
    p.gesture_commit(g);
    p.pump();
    assert!(p.drain(&mut feed).is_some());
    assert!(p.drain(&mut feed).is_none());
    assert!(p.undo().ok().is_some());
    assert_eq!(p.find(a).unwrap().get("volume").unwrap().as_real(), Some(1.0));
}

#[test]
fn loading_follows_the_200_ms_rule_and_peaks_wait() {
    let mut p =
        FakePort::new(Config { tracks: 6, slow_delay: Some(Duration::from_millis(400)), slow_track: 2 }).unwrap();
    let item = NodeId::parse(
        p.root()
            .get("tracks")
            .unwrap()
            .at(2)
            .unwrap()
            .get("items")
            .unwrap()
            .at(0)
            .unwrap()
            .get("id")
            .unwrap()
            .as_text()
            .unwrap(),
    )
    .unwrap();
    let res = p.first_lane_of_track(2).unwrap();
    assert_eq!(p.loading(&Target::Node(item)), Loading::Pending);
    assert!(p.peaks(&res, 2, 0, 10).is_waiting());
    std::thread::sleep(Duration::from_millis(250));
    p.pump();
    assert!(matches!(p.loading(&Target::Node(item)), Loading::Visible { .. }));
    let deadline = Instant::now() + Duration::from_secs(10);
    while p.loading(&Target::Node(item)) != Loading::Ready {
        assert!(Instant::now() < deadline, "never ready");
        std::thread::sleep(Duration::from_millis(10));
        p.pump();
    }
    assert_eq!(p.peaks(&res, 2, 0, 10).ok().unwrap().pairs.len(), 10);
}

#[test]
fn playing_moves_the_cursor_and_the_meters() {
    let mut p = small();
    let _ = p.run_action(ActionId::from("transport.locate"), Value::int(demo::BAR));
    let _ = p.run_action(ActionId::from("transport.play"), Value::NULL);
    std::thread::sleep(Duration::from_millis(60));
    p.pump();
    let t = p.telemetry();
    assert!(t.position.playing);
    assert!(t.position.samples > 0);
    assert!(t.meters.values().any(|m| m.peak[0] > 0.0), "something sounds");
    let _ = p.run_action(ActionId::from("transport.stop"), Value::NULL);
    p.pump();
    assert!(!p.telemetry().position.playing);
}

#[test]
fn recording_makes_an_item_on_the_armed_track() {
    let mut p = small();
    let armed = track(&p, 2);
    let before = p.find(armed).unwrap().get("items").unwrap().len();
    let _ = p.run_action(ActionId::from("transport.locate"), Value::int(8 * demo::BAR));
    let _ = p.run_action(ActionId::from("transport.play"), Value::NULL);
    std::thread::sleep(Duration::from_millis(20));
    p.pump();
    let _ = p.run_action(ActionId::from("transport.record"), Value::NULL); // playing: a punch-in
    std::thread::sleep(Duration::from_millis(80));
    p.pump();
    assert!(!p.telemetry().recording.is_empty(), "the take grows");
    let _ = p.run_action(ActionId::from("transport.record"), Value::NULL); // punch-out
    p.pump();
    assert_eq!(p.find(armed).unwrap().get("items").unwrap().len(), before + 1);
}

#[test]
fn the_browser_lists_searches_and_keeps_favorites() {
    let mut p = small();
    let top = p.browse(BrowsePage::Samples, None, false).ok().unwrap();
    assert_eq!(top[0].name.as_ref(), "Minhas amostras", "the user's before the factory's");
    assert!(!top.iter().any(|e| e.hidden));
    let found = p.search("\"loop funk\"").ok().unwrap();
    assert_eq!(found.len(), 1);
    assert!(p.search("\"funk loop\"").ok().unwrap().is_empty(), "a phrase is whole");
    let id = found[0].id;
    assert!(p.set_favorite(id, true).ok().is_some());
    assert_eq!(p.browse(BrowsePage::Favorites, None, false).ok().unwrap().len(), 1);
    assert!(p.prepare_preview(id).ok().is_some());
}

#[test]
fn preferences_change_by_path() {
    let mut p = small();
    assert!(p.set_preference(vec![PathStep::key("audio"), PathStep::key("block")], Value::int(512)).ok().is_some());
    assert_eq!(p.preferences().get("audio").unwrap().get("block").unwrap().as_int(), Some(512));
    assert_eq!(p.status().get("project").unwrap().get("name").unwrap().as_text(), Some("Demo"));
}

#[test]
fn removing_a_channel_sends_its_sources_to_the_master() {
    let mut p = small();
    let chans = p.root().get("mixer").unwrap().get("channels").unwrap().clone();
    let group = NodeId::parse(chans.at(0).unwrap().get("id").unwrap().as_text().unwrap()).unwrap();
    let piece = NodeId::parse(chans.at(1).unwrap().get("id").unwrap().as_text().unwrap()).unwrap();
    assert!(p.apply(Label::new("Remove"), vec![EditOp::Remove { id: Ref::Id(group) }]).ok().is_some());
    assert_eq!(p.find(piece).unwrap().get("output").unwrap().as_text(), Some("m:0"));
}

#[test]
fn split_and_duplicate_keep_notes_and_comps_coherent() {
    let mut p = small();
    let t = p.root().get("tracks").unwrap().at(3).unwrap().clone(); // an instrument track: MIDI items
    let item = t.get("items").unwrap().at(0).unwrap().clone();
    let id = item.get("id").unwrap().as_text().unwrap().to_string();
    let start = item.get("start").unwrap().as_int().unwrap();
    let notes = item.get("notes").unwrap().len();
    let params = Value::object([("item", Value::text(id.clone())), ("tick", Value::int(start + 2 * demo::BAR))]);
    assert!(p.run_action(ActionId::from("arrange.split"), params).ok().is_some());
    let track = NodeId::parse(t.get("id").unwrap().as_text().unwrap()).unwrap();
    let items = p.find(track).unwrap().get("items").unwrap().clone();
    let (left, right) = (items.at(0).unwrap(), items.at(1).unwrap());
    assert_eq!(left.get("length").unwrap().as_int(), Some(2 * demo::BAR));
    assert_eq!(right.get("start").unwrap().as_int(), Some(start + 2 * demo::BAR));
    assert_eq!(left.get("notes").unwrap().len() + right.get("notes").unwrap().len(), notes, "every note on one side");
    assert!(
        right.get("notes").unwrap().entries().all(|(_, n)| n.get("start").unwrap().as_int().unwrap() < 2 * demo::BAR)
    );
    // the audio item with three lanes: the comp follows the cut
    let voice = p.root().get("tracks").unwrap().at(2).unwrap().get("items").unwrap().at(0).unwrap().clone();
    let vid = voice.get("id").unwrap().as_text().unwrap().to_string();
    let vstart = voice.get("start").unwrap().as_int().unwrap();
    let params = Value::object([("item", Value::text(vid.clone())), ("tick", Value::int(vstart + 10 * demo::BAR))]);
    assert!(p.run_action(ActionId::from("arrange.split"), params).ok().is_some());
    let v = p.find(NodeId::parse(&vid).unwrap()).unwrap();
    assert_eq!(v.get("comp").unwrap().len(), 2, "two segments before the cut");
    assert!(
        p.run_action(ActionId::from("arrange.duplicate"), Value::object([("item", Value::text(vid))])).ok().is_some()
    );
    assert!(p.undo().ok().is_some() && p.undo().ok().is_some(), "two steps");
    let back = p.find(NodeId::parse(&id).unwrap()).unwrap();
    assert_eq!(back.get("notes").unwrap().len(), notes - right_count(&p, track));
}

fn right_count(p: &FakePort, track: NodeId) -> usize {
    // after two undos the MIDI split stays (it was the first step): its right part's notes
    p.find(track).unwrap().get("items").unwrap().at(1).unwrap().get("notes").unwrap().len()
}

#[test]
fn creating_channels_for_the_outputs_is_one_step() {
    let mut p = small();
    let drums = track(&p, 1);
    let outs = p.find(drums).unwrap().get("outputs").unwrap().len();
    let before = p.root().get("mixer").unwrap().get("channels").unwrap().len();
    let r = p.run_action(
        ActionId::from("mixer.create_channels_for_outputs"),
        Value::object([("track", Value::text(drums.to_string()))]),
    );
    assert!(r.ok().is_some());
    assert_eq!(p.root().get("mixer").unwrap().get("channels").unwrap().len(), before + 1 + outs);
    assert!(p.undo().ok().is_some());
    assert_eq!(p.root().get("mixer").unwrap().get("channels").unwrap().len(), before);
}

#[test]
fn every_plugin_has_its_schema_and_its_browser_entry() {
    let mut port = FakePort::new(Config::default()).unwrap();
    let plugins = port.plugins();
    assert!(!plugins.is_empty());
    let mut listed = Vec::new();
    for page in [BrowsePage::Instruments, BrowsePage::Effects] {
        let Pending::Ready(Ok(entries)) = port.browse(page, None, false) else { panic!("the fake answers at once") };
        listed.extend(entries.iter().filter_map(|e| e.info.get("plugin").and_then(Value::as_text).map(str::to_string)));
    }
    for p in &plugins {
        let schema = port.plugin_schema(&p.id).unwrap_or_else(|| panic!("{} has a schema", p.id));
        assert!(!schema.params.is_empty() || !schema.fields.is_empty());
        assert!(listed.iter().any(|l| **l == *p.id), "{} is on the browser's plugin pages", p.id);
    }
    assert!(port.plugin_schema("nonexistent").is_none());
}

#[test]
fn presets_load_as_one_step_and_save_to_the_library() {
    let mut port = FakePort::new(Config::default()).unwrap();
    let root = port.root().clone();
    let inst = root.get("tracks").unwrap().items().find_map(|t| {
        let i = t.get("instrument").filter(|i| !i.is_null())?;
        (i.get("plugin").and_then(Value::as_text) == Some("tripleoscillator"))
            .then(|| i.get("id").unwrap().as_text().unwrap().to_string())
    });
    let inst = inst.expect("a tripleoscillator");
    let Pending::Ready(Ok(presets)) = port.browse(BrowsePage::Presets, None, false) else { panic!() };
    let fat =
        presets.iter().find(|e| e.info.get("plugin").and_then(Value::as_text) == Some("tripleoscillator")).unwrap();
    let params = Value::object([("node", Value::text(inst.as_str())), ("entry", Value::int(fat.id.0 as i64))]);
    assert!(port.run_action("plugin.load_preset".into(), params).ok().is_some());
    assert!(port.can_undo());
    let save = Value::object([("node", Value::text(inst.as_str())), ("name", Value::text("Meu baixo"))]);
    assert!(port.run_action("plugin.save_preset".into(), save).ok().is_some());
    let Pending::Ready(Ok(after)) = port.browse(BrowsePage::Presets, None, false) else { panic!() };
    assert!(after.iter().any(|e| &*e.name == "Meu baixo" && !e.factory));
}

/// MIDI learn (I-21.2): the first message after `midi_learn` becomes a mapping in the project, in one undo step; the
/// activity shows it as mapped; `unmap` removes it; a mapping to an action reaches the interface's feed
#[test]
fn learning_mapping_and_unmapping() {
    use lm_port::{MapScope, MapTarget, MidiMessage, NodeRef};
    let mut p = small();
    let t = track(&p, 1);
    let target = MapTarget::Param { node: NodeRef::Id(t), key: "volume".into() };
    let id = match p.midi_learn(target.clone(), MapScope::Project) {
        Pending::Ready(Ok(id)) => id,
        other => panic!("{other:?}"),
    };
    assert_eq!(p.learning(), Some(id));
    p.midi_in("Controlador 1", 0, MidiMessage::Cc { number: 7 }, 100);
    assert_eq!(p.learning(), None, "learned");
    let maps = p.mappings(MapScope::Project);
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0].target, target);
    assert_eq!(maps[0].source.message, MidiMessage::Cc { number: 7 });
    assert!(p.notices().iter().any(|n| n.key == "midi.learned"));
    let (n, act) = p.midi_activity(0);
    assert!(n >= 1 && act.last().is_some_and(|a| a.mapped), "the activity shows it");
    p.midi_in("Controlador 1", 0, MidiMessage::Cc { number: 7 }, 20);
    assert!(p.midi_activity(n).1[0].mapped, "the next one is mapped too");
    assert!(p.can_undo());
    assert!(matches!(p.unmap(&target, MapScope::Project), Pending::Ready(Ok(()))));
    assert!(p.mappings(MapScope::Project).is_empty(), "unmapped");
    p.undo().ok().unwrap();
    assert_eq!(p.mappings(MapScope::Project).len(), 1, "undo brings it back");
    // an action, in the machine's mappings: the interface gets it
    let zoom = MapTarget::Action { id: "arrange.zoom_in".into(), params: Value::NULL, value_into: None };
    let _ = p.midi_learn(zoom, MapScope::Machine);
    p.midi_in("Controlador 1", 0, MidiMessage::Note { key: 36 }, 127);
    p.midi_in("Controlador 1", 0, MidiMessage::Note { key: 36 }, 127);
    let mut ui = Vec::new();
    p.drain_midi_ui(&mut ui);
    assert_eq!(ui.len(), 1);
    assert_eq!(ui[0].action.as_str(), "arrange.zoom_in");
    // a learn can be cancelled
    let id = p.midi_learn(target, MapScope::Project).ok().unwrap();
    p.cancel_learn(id);
    assert_eq!(p.learning(), None);
}

/// "The hand won" (I-21.5) shows in the telemetry until `automation.rearm`
#[test]
fn the_hand_won_until_rearmed() {
    let mut p = small();
    let t = track(&p, 1);
    p.hand_wins(t, "volume");
    p.hand_wins(t, "pan");
    assert_eq!(p.telemetry().hand_won.len(), 2);
    p.run_action("automation.rearm".into(), Value::empty_object()).ok().unwrap();
    assert!(p.telemetry().hand_won.is_empty());
}

/// `items.move` takes the automation under the items along (I-21.5), in one undo step; `automation.thin` drops
/// the points a line already gives
#[test]
fn moving_items_moves_their_automation_and_thinning() {
    let mut p = small();
    let t = track(&p, 2);
    let item = p.find(t).unwrap().get("items").unwrap().items().next().unwrap().clone();
    let item_id = NodeId::parse(item.get("id").unwrap().as_text().unwrap()).unwrap();
    let start = item.get("start").unwrap().as_int().unwrap();
    let pt = |tick: i64, v: f64| Value::object([("tick", Value::int(tick)), ("value", Value::real(v).unwrap())]);
    let lane = Value::object([
        ("target", Value::object([("node", Value::text(t.to_string())), ("param", Value::text("volume"))])),
        ("points", Value::list([pt(start, 0.5), pt(start + 100, 0.6), pt(start + 200, 0.7), pt(start + 300, 0.2)])),
    ]);
    let n = p.find(t).unwrap().get("automation").map_or(0, Value::len);
    let made = p
        .apply(
            Label::new("lane"),
            vec![EditOp::Insert { temp: 1, parent: Ref::Id(t), list: "automation".into(), index: n, node: lane }],
        )
        .ok()
        .unwrap();
    let lane_id = made.created[0].1;
    p.run_action(
        "items.move".into(),
        Value::object([("items", Value::list([Value::text(item_id.to_string())])), ("delta", Value::int(960))]),
    )
    .ok()
    .unwrap();
    let ticks = |p: &FakePort| -> Vec<i64> {
        p.find(lane_id)
            .unwrap()
            .get("points")
            .unwrap()
            .items()
            .map(|v| v.get("tick").unwrap().as_int().unwrap())
            .collect()
    };
    assert_eq!(p.find(item_id).unwrap().get("start").unwrap().as_int(), Some(start + 960));
    assert_eq!(ticks(&p)[0], start + 960, "the points under the item moved");
    p.undo().ok().unwrap();
    assert_eq!(ticks(&p)[0], start, "one undo step");
    p.run_action("automation.thin".into(), Value::object([("lane", Value::text(lane_id.to_string()))])).ok().unwrap();
    assert_eq!(ticks(&p).len(), 3, "the middle point of the straight part went: {:?}", ticks(&p));
}

#[test]
fn sidechains_tuning_files_reveal_and_open() {
    let mut port = FakePort::new(Config::default()).unwrap();
    let plugins = port.plugins();
    assert!(plugins.iter().any(|p| &*p.id == "compressor" && p.sidechain));
    assert!(plugins.iter().any(|p| &*p.id == "reverb" && !p.sidechain));
    let dir = std::env::temp_dir().join(format!("lm-fake-tuning-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scl = dir.join("meantone.scl");
    std::fs::write(&scl, "! x\nMeantone\n2\n193.157\n2/1\n").unwrap();
    let params = Value::object([("slot", Value::int(3)), ("path", Value::text(scl.to_str().unwrap()))]);
    assert!(port.run_action("tuning.import_scale".into(), params).ok().is_some());
    let scales = port.root().get("tuning").and_then(|t| t.get("scales")).unwrap().clone();
    assert_eq!(scales.items().count(), 10);
    assert_eq!(scales.at(3).and_then(|s| s.get("description")).and_then(Value::as_text), Some("Meantone"));
    let bad = dir.join("bad.scl");
    std::fs::write(&bad, "x\n2\n100.0\nnope\n").unwrap();
    let params = Value::object([("slot", Value::int(4)), ("path", Value::text(bad.to_str().unwrap()))]);
    assert!(port.run_action("tuning.import_scale".into(), params).ok().is_none(), "refused whole");
    let _ = std::fs::remove_dir_all(&dir);
    let Pending::Ready(Ok(projects)) = port.browse(BrowsePage::Projects, None, false) else { panic!() };
    let p = projects.iter().find(|e| e.kind == lm_port::EntryKind::Project).unwrap().id;
    assert!(port.run_action("browser.reveal".into(), Value::int(p.0 as i64)).ok().is_some());
    assert!(port.notices().iter().any(|n| n.key.starts_with("Shown in the file manager")));
    assert!(port.run_action("project.open_entry".into(), Value::int(p.0 as i64)).ok().is_some());
    assert_ne!(port.loading(&lm_port::Target::Project), Loading::Ready, "the opening runs on the project");
}

/// The desk's live readings (`trees::rack_display`): the rack layout for every channel in the analog desk, the bus
/// compressor and the scope on the master only, nothing in the digital desk
#[test]
fn the_rack_display() {
    use lm_port::trees::rack_display as d;
    let mut p = small();
    let master = lm_doc::MASTER;
    let ch = NodeId::parse(
        p.root()
            .get("mixer")
            .unwrap()
            .get("channels")
            .unwrap()
            .items()
            .next()
            .unwrap()
            .get("id")
            .unwrap()
            .as_text()
            .unwrap(),
    )
    .unwrap();
    let m = p.display(master).expect("the master has its readings");
    assert_eq!((&*m.kind, m.frame.len()), (d::KIND, d::FLOATS));
    assert!(m.frame[d::CORRELATION].is_finite() && m.frame[d::GATE_GR].is_nan());
    let c = p.display(ch).expect("a channel has its readings");
    assert!(c.frame[d::BUS_GR].is_nan() && c.frame[d::COMP_GR].is_finite());
    p.apply(
        Label::new("digital"),
        vec![EditOp::Set {
            id: Ref::Id(ROOT),
            path: vec![PathStep::key("mixer"), PathStep::key("mode")],
            value: Value::text("digital"),
        }],
    )
    .ok()
    .unwrap();
    assert!(p.display(ch).is_none(), "the digital desk has no rack");
}
