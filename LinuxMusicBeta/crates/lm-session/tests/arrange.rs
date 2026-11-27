//! The arrangement's domain edits as actions (F-ARR-005, F-ARR-007, F-MIX-012)

mod common;

use common::*;
use lm_base::{Cancel, Label, Progress};
use lm_session::Session;
use lm_session::export::{ExportTarget, export};
use lm_state::{NodeId, PathStep, ROOT, Resource, Value};

fn render(s: &Session, dir: &TempDir, name: &str) -> Vec<f32> {
    let path = dir.join(name);
    export(s.export_job(), &ExportTarget::wav(path.clone()), &Cancel::new(), &Progress::new()).expect("export");
    let mut r = lm_sys_sndfile::Reader::open(&path).expect("open");
    let mut buf = vec![0.0f32; r.info().frames as usize * r.info().channels];
    let n = r.read_f32(&mut buf).expect("read");
    buf.truncate(n * r.info().channels);
    buf
}

fn item_ids(s: &Session, track: NodeId) -> Vec<NodeId> {
    s.document()
        .find(track)
        .and_then(|t| t.get("items").cloned())
        .map(|l| l.items().filter_map(|i| NodeId::parse(i.get("id")?.as_text()?).ok()).collect())
        .unwrap_or_default()
}

fn field(s: &Session, id: NodeId, k: &str) -> i64 {
    s.document().find(id).and_then(|v| v.get(k).and_then(Value::as_int)).unwrap_or(-1)
}

#[test]
fn split_keeps_each_note_on_its_side_and_is_one_undo_step() {
    let dir = TempDir::new("split-midi");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let t = add_synth_track(&mut s, "Synth");
    let it = add_notes(&mut s, t, &[(60, 0, 480), (62, 960, 480), (64, 1920, 480)]);
    let p = |tick: i64| Value::object([("item", Value::text(it.to_string())), ("tick", Value::int(tick))]);
    assert!(s.run_action("arrange.split", &p(0)).is_err(), "at the start: outside");
    s.run_action("arrange.split", &p(960)).expect("split");
    let ids = item_ids(&s, t);
    assert_eq!(ids.len(), 2);
    let (left, right) = (ids[0], ids[1]);
    assert_eq!((field(&s, left, "start"), field(&s, left, "length")), (0, 960));
    assert_eq!((field(&s, right, "start"), field(&s, right, "length")), (960, 1440));
    let starts = |id: NodeId| -> Vec<i64> {
        let mut v: Vec<i64> = s
            .document()
            .find(id)
            .and_then(|i| i.get("notes").cloned())
            .map(|m| m.entries().filter_map(|(_, n)| n.get("start").and_then(Value::as_int)).collect())
            .unwrap_or_default();
        v.sort();
        v
    };
    assert_eq!(starts(left), vec![0]);
    assert_eq!(starts(right), vec![0, 960]);
    s.document_mut().undo().expect("undo");
    assert_eq!(item_ids(&s, t), vec![it]);
    assert_eq!(field(&s, it, "length"), 2400);
}

#[test]
fn splitting_an_audio_item_does_not_change_the_sound() {
    let dir = TempDir::new("split-audio");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    // A second of a 220 Hz ramp-modulated sine, imported
    let wav = dir.join("source.wav");
    {
        let spec = lm_sys_sndfile::WriteSpec {
            format: lm_sys_sndfile::WriteFormat::Wav(lm_sys_sndfile::Sample::Float32),
            rate: 48_000,
            channels: 2,
        };
        let mut w = lm_sys_sndfile::Writer::create(&wav, spec, lm_sys_sndfile::Existing::Fail).expect("wav");
        let data: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let x = (i as f32 * 220.0 * std::f32::consts::TAU / 48_000.0).sin() * (i as f32 / 48_000.0);
                [x, -x]
            })
            .collect();
        w.write_f32(&data).expect("write");
        w.close().expect("close");
    }
    let imported =
        lm_io::import_file(&wav, s.project().folder.root(), 48_000, &Cancel::new(), &Progress::new()).expect("import");
    let mut item = None;
    let mut track = None;
    s.document_mut()
        .transact(Label::new("test"), |e| {
            let t = e.insert(ROOT, "tracks", 0, Value::object([("name", Value::text("Audio"))]))?;
            let lane = Value::object([("source", Value::resource(Resource::new("audio", &imported.key)?))]);
            let it = e.insert(
                t,
                "items",
                0,
                Value::object([
                    ("kind", Value::text("audio")),
                    ("start", Value::int(960)),
                    ("length", Value::int(1920)),
                    ("lanes", Value::list([lane])),
                ]),
            )?;
            item = Some(it);
            track = Some(t);
            Ok(())
        })
        .expect("item");
    settle(&mut s);
    let before = render(&s, &dir, "before.wav");
    let it = item.expect("item");
    let p = Value::object([("item", Value::text(it.to_string())), ("tick", Value::int(1500))]);
    s.run_action("arrange.split", &p).expect("split");
    settle(&mut s);
    let ids = item_ids(&s, track.expect("track"));
    assert_eq!(ids.len(), 2);
    let after = render(&s, &dir, "after.wav");
    assert_eq!(before.len(), after.len());
    let first = before.iter().zip(&after).position(|(a, b)| a.to_bits() != b.to_bits());
    assert_eq!(first, None, "the split item plays the same samples");
    assert!(before.iter().any(|x| x.abs() > 0.1), "and it plays");
}

#[test]
fn duplicate_lands_on_the_next_bar_line() {
    let dir = TempDir::new("duplicate");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let t = add_synth_track(&mut s, "Synth");
    // An item from 0 to 1.25 bars (4/4 at 960 ppq: a bar is 3840)
    let it = add_notes(&mut s, t, &[(60, 0, 4800)]);
    s.run_action("arrange.duplicate", &Value::object([("item", Value::text(it.to_string()))])).expect("dup");
    let ids = item_ids(&s, t);
    assert_eq!(ids.len(), 2);
    assert_eq!(field(&s, ids[1], "start"), 7680);
    assert_eq!(s.document().find(ids[1]).and_then(|v| v.get("notes").map(Value::len)), Some(1));
}

#[test]
fn channels_for_the_outputs_of_a_multi_output_instrument() {
    let dir = TempDir::new("outputs");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let t = add_synth_track(&mut s, "Drums");
    s.document_mut()
        .transact(Label::new("test"), |e| {
            let n = e.find(t).and_then(|v| v.get("outputs").map(Value::len)).unwrap_or(0);
            e.insert(t, "outputs", n, Value::object([("port", Value::text("kick")), ("channels", Value::int(1))]))?;
            Ok(())
        })
        .expect("output");
    let n0 = s.document().root().get("mixer").and_then(|m| m.get("channels")).map_or(0, Value::len);
    s.run_action("mixer.create_channels_for_outputs", &Value::object([("track", Value::text(t.to_string()))]))
        .expect("create");
    let channels = s.document().root().get("mixer").and_then(|m| m.get("channels")).cloned().expect("channels");
    let outputs = s.document().find(t).and_then(|v| v.get("outputs").cloned()).expect("outputs");
    assert_eq!(channels.len(), n0 + 1 + outputs.len());
    let group = channels.at(n0).and_then(|c| c.get("id")).and_then(Value::as_text).expect("group").to_owned();
    for (i, o) in outputs.items().enumerate() {
        let ch = channels.at(n0 + 1 + i).expect("channel");
        assert_eq!(o.get("to").and_then(Value::as_text), ch.get("id").and_then(Value::as_text));
        assert_eq!(ch.get("output").and_then(Value::as_text), Some(group.as_str()));
    }
    settle(&mut s);
    s.document_mut().undo().expect("undo: one step");
    assert_eq!(s.document().root().get("mixer").and_then(|m| m.get("channels")).map_or(0, Value::len), n0);
    let _ = PathStep::key("x");
}

#[test]
fn splitting_a_repeating_pattern_does_not_change_what_plays() {
    let dir = TempDir::new("split-pattern");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let t = add_synth_track(&mut s, "Synth");
    // A bar-long pattern of two notes repeated over three bars
    let it = add_notes(&mut s, t, &[(60, 0, 240), (67, 1920, 240)]);
    s.document_mut()
        .transact(Label::new("test"), |e| {
            e.set(it, &[PathStep::key("loop")], Value::int(3840))?;
            e.set(it, &[PathStep::key("length")], Value::int(3 * 3840))
        })
        .expect("pattern");
    settle(&mut s);
    let before = render(&s, &dir, "before.wav");
    s.run_action("arrange.split", &Value::object([("item", Value::text(it.to_string())), ("tick", Value::int(5000))]))
        .expect("split");
    settle(&mut s);
    let after = render(&s, &dir, "after.wav");
    assert_eq!(before.len(), after.len());
    assert!(before.iter().zip(&after).all(|(a, b)| (a - b).abs() < 1.0e-4), "the same notes at the same times");
}

fn ids_param(ids: &[NodeId]) -> Value {
    Value::object([("items", Value::list(ids.iter().map(|i| Value::text(i.to_string()))))])
}

#[test]
fn copy_cut_and_paste_keep_distances_and_tracks() {
    let dir = TempDir::new("clipboard");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let a = add_synth_track(&mut s, "A");
    let b = add_synth_track(&mut s, "B");
    let x = add_notes(&mut s, a, &[(60, 0, 480)]);
    let y = add_notes(&mut s, b, &[(62, 0, 480)]);
    set(&mut s, x, "start", Value::int(3840));
    set(&mut s, y, "start", Value::int(7680));
    assert!(s.run_action("arrange.paste", &Value::object([("at", Value::int(0))])).is_err(), "empty clipboard");
    s.run_action("arrange.copy", &ids_param(&[y, x])).expect("copy");
    // Pasted at bar 5 on track B: x (the first) goes to B, y one track further down: none, so it stays on B
    let at = Value::object([("at", Value::int(15360)), ("track", Value::text(b.to_string()))]);
    s.run_action("arrange.paste", &at).expect("paste");
    let on_b = item_ids(&s, b);
    assert_eq!(on_b.len(), 3);
    assert_eq!(field(&s, on_b[1], "start"), 15360);
    assert_eq!(field(&s, on_b[2], "start"), 19200);
    // Pasted with no track: each copy on its own track
    s.run_action("arrange.paste", &Value::object([("at", Value::int(0))])).expect("paste 2");
    assert_eq!(item_ids(&s, a).len(), 2);
    assert_eq!(field(&s, item_ids(&s, a)[1], "start"), 0);
    assert_eq!(field(&s, item_ids(&s, b)[3], "start"), 3840);
    // Cut: gone in one undo step, and on the clipboard
    s.run_action("arrange.cut", &ids_param(&[x])).expect("cut");
    assert_eq!(item_ids(&s, a).len(), 1);
    s.document_mut().undo().expect("undo");
    assert_eq!(item_ids(&s, a).len(), 2);
    s.run_action("arrange.cut", &ids_param(&[x])).expect("cut again");
    s.run_action("arrange.paste", &Value::object([("at", Value::int(960))])).expect("paste cut");
    assert_eq!(field(&s, *item_ids(&s, a).last().expect("pasted"), "start"), 960);
}

#[test]
fn insert_and_remove_bar_move_what_follows() {
    let dir = TempDir::new("bars");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let t = add_synth_track(&mut s, "Synth");
    let before = add_notes(&mut s, t, &[(60, 0, 480)]);
    let after = add_notes(&mut s, t, &[(62, 0, 480)]);
    set(&mut s, after, "start", Value::int(7680));
    s.run_action("arrange.insert_bar", &Value::object([("tick", Value::int(3840))])).expect("insert");
    assert_eq!((field(&s, before, "start"), field(&s, after, "start")), (0, 11520));
    s.document_mut().undo().expect("undo");
    assert_eq!(field(&s, after, "start"), 7680);
    // Removing the bar the item starts in: it goes to the removed bar's start
    s.run_action("arrange.remove_bar", &Value::object([("tick", Value::int(3840))])).expect("remove");
    assert_eq!(field(&s, after, "start"), 3840);
    s.run_action("arrange.remove_bar", &Value::object([("tick", Value::int(3840))])).expect("remove 2");
    assert_eq!((field(&s, before, "start"), field(&s, after, "start")), (0, 3840));
}

#[test]
fn split_and_duplicate_a_selection_in_one_undo_step() {
    let dir = TempDir::new("split-many");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let a = add_synth_track(&mut s, "A");
    let b = add_synth_track(&mut s, "B");
    let x = add_notes(&mut s, a, &[(60, 0, 3840)]);
    let y = add_notes(&mut s, b, &[(62, 0, 3840)]);
    set(&mut s, y, "start", Value::int(960));
    // An empty list: every item the tick is inside
    let p = Value::object([("items", Value::list([])), ("tick", Value::int(1920))]);
    s.run_action("arrange.split", &p).expect("split all");
    assert_eq!((item_ids(&s, a).len(), item_ids(&s, b).len()), (2, 2));
    s.document_mut().undo().expect("undo");
    assert_eq!((item_ids(&s, a).len(), item_ids(&s, b).len()), (1, 1));
    // The group x (0..3840) and y (960..4800) duplicated: it ends at 4800, the copies start on 7680 keeping 960
    s.run_action("arrange.duplicate", &ids_param(&[x, y])).expect("dup");
    assert_eq!(field(&s, item_ids(&s, a)[1], "start"), 7680);
    assert_eq!(field(&s, item_ids(&s, b)[1], "start"), 8640);
    s.document_mut().undo().expect("undo dup");
    assert_eq!((item_ids(&s, a).len(), item_ids(&s, b).len()), (1, 1));
    // As the legacy: whole bars from the start, so y (960..4800) lands on 4800, a quarter into its bar as it was
    s.run_action("arrange.duplicate", &ids_param(&[y])).expect("dup y");
    assert_eq!(field(&s, item_ids(&s, b)[1], "start"), 4800);
}

/// An imported constant sine of `amp` (a second, stereo) on a new audio track as one item; (item, lane, key)
fn audio_item(s: &mut Session, dir: &TempDir, amp: f32) -> (NodeId, NodeId, String) {
    let wav = dir.join("tone.wav");
    let spec = lm_sys_sndfile::WriteSpec {
        format: lm_sys_sndfile::WriteFormat::Wav(lm_sys_sndfile::Sample::Float32),
        rate: 48_000,
        channels: 2,
    };
    let mut w = lm_sys_sndfile::Writer::create(&wav, spec, lm_sys_sndfile::Existing::Fail).expect("wav");
    let data: Vec<f32> = (0..48_000)
        .flat_map(|i| {
            let x = (i as f32 * 220.0 * std::f32::consts::TAU / 48_000.0).sin() * amp;
            [x, -x]
        })
        .collect();
    w.write_f32(&data).expect("write");
    w.close().expect("close");
    let imported =
        lm_io::import_file(&wav, s.project().folder.root(), 48_000, &Cancel::new(), &Progress::new()).expect("import");
    let mut made = None;
    s.document_mut()
        .transact(Label::new("test"), |e| {
            let t = e.insert(ROOT, "tracks", 0, Value::object([("name", Value::text("Audio"))]))?;
            let lane = Value::object([("source", Value::resource(Resource::new("audio", &imported.key)?))]);
            let it = e.insert(
                t,
                "items",
                0,
                Value::object([
                    ("kind", Value::text("audio")),
                    ("start", Value::int(0)),
                    ("length", Value::int(1920)),
                    ("fadeIn", Value::int(100)),
                    ("lanes", Value::list([lane])),
                ]),
            )?;
            made = Some(it);
            Ok(())
        })
        .expect("item");
    let it = made.expect("item");
    let lane = s
        .document()
        .find(it)
        .and_then(|v| v.get("lanes").and_then(|l| l.items().next().cloned()))
        .and_then(|l| NodeId::parse(l.get("id")?.as_text()?).ok())
        .expect("lane");
    (it, lane, imported.key)
}

#[test]
fn normalize_sets_the_gain_that_brings_the_loudest_sample_to_the_target() {
    let dir = TempDir::new("normalize");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let (it, _, _) = audio_item(&mut s, &dir, 0.5);
    settle(&mut s);
    let p = ids_param(&[it]);
    s.run_action("clip.normalize", &p).expect("normalize");
    let gain = s.document().find(it).and_then(|v| v.get("gain").and_then(Value::as_real)).expect("gain");
    assert!((gain - 2.0).abs() < 0.01, "0.5 to full scale is a gain of 2, got {gain}");
    // -6 dB: half of that
    let p = Value::object([
        ("items", Value::list([Value::text(it.to_string())])),
        ("peak", Value::real(-6.0206).expect("real")),
    ]);
    s.run_action("clip.normalize", &p).expect("normalize");
    let gain = s.document().find(it).and_then(|v| v.get("gain").and_then(Value::as_real)).expect("gain");
    assert!((gain - 1.0).abs() < 0.01, "got {gain}");
    s.document_mut().undo().expect("undo");
    s.document_mut().undo().expect("undo");
    let back = s.document().find(it).and_then(|v| v.get("gain").and_then(Value::as_real)).expect("gain");
    assert!((back - 1.0).abs() < 1e-9, "one undo step each");
}

#[test]
fn reverse_writes_a_new_resource_and_mirrors_the_item() {
    let dir = TempDir::new("reverse");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let (it, lane, key) = audio_item(&mut s, &dir, 0.5);
    settle(&mut s);
    let before = render(&s, &dir, "before.wav");
    let p = ids_param(&[it]);
    s.run_action("clip.reverse", &p).expect("reverse");
    settle(&mut s);
    let lane_v = s.document().find(lane).expect("lane");
    let new = lane_v.get("source").and_then(Value::as_resource).expect("source").key().to_owned();
    assert_ne!(new, key, "a new resource");
    assert_eq!(lane_v.get("sourceOffset").and_then(Value::as_int), Some(0));
    assert_eq!(field(&s, it, "fadeIn"), 0, "the fade in became the fade out");
    assert_eq!(field(&s, it, "fadeOut"), 100);
    // Same energy, other order: reversing twice plays what the first one played
    s.run_action("clip.reverse", &p).expect("reverse");
    settle(&mut s);
    let twice = render(&s, &dir, "twice.wav");
    assert_eq!(before.len(), twice.len());
    let worst = before.iter().zip(&twice).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-5, "reversed twice is the original, off by {worst}");
}
