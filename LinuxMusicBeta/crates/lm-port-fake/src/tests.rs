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
