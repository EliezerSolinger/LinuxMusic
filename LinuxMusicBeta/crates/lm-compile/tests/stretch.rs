//! Takes that follow the tempo (F-REC-090): the plan of a lane with a timing (the original at the same tempo, a
//! rendering of exactly the item's length otherwise, its key a function of what makes it), and the compiled item
//! playing the original until its rendering is loaded, then the rendering

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use lm_compile::stretch::{RENDER_KIND, lane_play, recorded_timing};
use lm_compile::{ResourceState, Resources};
use lm_render::SampleData;
use lm_state::{ROOT, Resource, Value};
use lm_time::{Bpm, Ppq, TempoMap, TempoPoint, TimeSignature};

fn map(points: &[(i64, u32, bool)]) -> TempoMap {
    let pts: Vec<TempoPoint> = points
        .iter()
        .map(|(t, b, ramp)| {
            let p = TempoPoint::new(*t, Bpm::from_integer(*b).unwrap(), TimeSignature::COMMON);
            if *ramp { p.linear() } else { p }
        })
        .collect();
    TempoMap::new(RATE, Ppq::DEFAULT, &pts).unwrap()
}

fn take() -> Resource {
    Resource::of_content("audio", b"a take").unwrap()
}

#[test]
fn at_the_tempo_it_was_played_the_original_plays_where_it_was_played() {
    let m = map(&[(0, 120, false), (7680, 90, false)]);
    // Recorded from a sample inside a tick: the item starts at the tick before, the take a few samples later
    let start_sample = m.sample_at(3000) + 7;
    let start_tick = m.tick_at(start_sample);
    let frames = 300_000;
    let timing = recorded_timing(&m, start_tick, start_sample, frames);
    assert!(timing.iter().any(|p| p.0 == 7680 - start_tick), "the tempo change is a point");
    let length = m.tick_at(start_sample + frames) - start_tick + 1;
    let play = lane_play(start_tick, length, &take(), 0, frames as u64, &timing, &m).unwrap();
    assert_eq!(play.render, None, "no rendering at the same tempo");
    assert_eq!(play.offset, -(start_sample - m.sample_at(start_tick)), "frame 0 where it was played");
}

#[test]
fn at_another_tempo_the_rendering_is_exactly_the_item_long_and_each_tick_lands_on_its_sample() {
    let recorded = map(&[(0, 120, false)]);
    let frames = 96_000i64; // 2 s: 4 beats at 120
    let timing = recorded_timing(&recorded, 0, 0, frames);
    let length = 3840;
    for (now, name) in [
        (map(&[(0, 60, false)]), "slower"),
        (map(&[(0, 240, false)]), "faster"),
        (map(&[(0, 120, false), (1920, 80, false)]), "a change inside"),
        (map(&[(0, 100, true), (3840, 140, false)]), "a ramp"),
    ] {
        let play = lane_play(960, length, &take(), 0, frames as u64, &timing, &now).unwrap();
        let plan = play.render.expect(name);
        let want = now.sample_at(960 + length) - now.sample_at(960);
        assert_eq!(plan.output_frames, want as u64, "{name}: exactly the item's length");
        let (first, last) = (plan.key_frames[0], *plan.key_frames.last().unwrap());
        assert_eq!(first, (0, 0), "{name}");
        assert_eq!(last.0, frames as u64, "{name}: the whole take");
        assert_eq!(last.1, want as u64, "{name}: to the item's end");
        // Each beat of the take lands on the beat's sample now
        for beat in 1..4 {
            let src = (beat * 24_000) as u64;
            let at = now.sample_at(960 + beat * 960) - now.sample_at(960);
            let i = plan.key_frames.iter().position(|k| k.0 >= src).unwrap();
            if plan.key_frames[i].0 == src {
                assert_eq!(plan.key_frames[i].1 as i64, at, "{name}: beat {beat}");
            }
        }
        assert!(plan.key_frames.windows(2).all(|w| w[1].0 > w[0].0 && w[1].1 > w[0].1), "{name}: increasing");
    }
}

#[test]
fn the_key_names_what_makes_the_rendering() {
    let recorded = map(&[(0, 120, false)]);
    let timing = recorded_timing(&recorded, 0, 0, 96_000);
    let slower = map(&[(0, 60, false)]);
    let plan =
        |m: &TempoMap, start: i64| lane_play(start, 3840, &take(), 0, 96_000, &timing, m).unwrap().render.unwrap();
    let a = plan(&slower, 0);
    assert_eq!(a, plan(&slower, 0), "the same twice");
    assert_eq!(a.key, plan(&slower, 3840).key, "moved to a place of the same tempo: the same rendering");
    assert_ne!(a.key, plan(&map(&[(0, 61, false)]), 0).key, "another tempo, another rendering");
    let other = lane_play(0, 3840, &Resource::of_content("audio", b"b").unwrap(), 0, 96_000, &timing, &slower);
    assert_ne!(a.key, other.unwrap().render.unwrap().key, "another original");
    assert_eq!(a.resource().kind(), RENDER_KIND);
    // A timing that cannot be read: as a lane without one
    assert!(lane_play(0, 3840, &take(), 0, 96_000, &[(0, 0.0)], &slower).is_none());
    assert!(lane_play(0, 3840, &take(), 0, 96_000, &[(0, 0.0), (10, -5.0)], &slower).is_none());
}

struct Loaded(BTreeMap<(String, String), Arc<SampleData>>);

impl Resources for Loaded {
    fn audio(&self, r: &Resource) -> ResourceState {
        match self.0.get(&(r.kind().to_owned(), r.key().to_owned())) {
            Some(d) => ResourceState::Ready(d.clone()),
            None => ResourceState::Loading,
        }
    }
}

#[test]
fn an_item_plays_its_original_until_its_rendering_is_loaded() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    digital(&mut doc);
    let t = add_track(&mut doc, None);
    edit(&mut doc, |e| e.set(t, &[k("channels")], Value::int(1)));
    let o = outputs(&doc, t)[0];
    edit(&mut doc, |e| e.set(o, &[k("channels")], Value::int(1)));
    let item = add_item(&mut doc, t, "audio", 0, 3840);
    // Recorded at 120 (4 beats, 2 s), played at 60
    let recorded = map(&[(0, 120, false)]);
    let timing = recorded_timing(&recorded, 0, 0, 96_000);
    let points =
        Value::list(timing.iter().map(|(t, f)| Value::object([("tick", Value::int(*t)), ("frame", real(*f))])));
    edit(&mut doc, |e| {
        e.insert(item, "lanes", 0, Value::object([("source", Value::resource(take())), ("timing", points.clone())]))?;
        e.set(ROOT, &[k("tempoMap")], Value::list([Value::object([("tick", Value::int(0)), ("bpm", text("60"))])]))
    });
    let original = Arc::new(SampleData::from_channels(RATE, vec![vec![0.5; 96_000]]).unwrap());
    let mut h = Harness::new(&plugins);
    h.resources = Arc::new(Loaded(BTreeMap::from([(("audio".to_owned(), take().key().to_owned()), original.clone())])));
    let c = h.compile(doc.root());
    assert_eq!(c.stretches.len(), 1, "the rendering is asked for");
    assert!(!c.complete(), "an export waits for it");
    let plan = c.stretches[0].clone();
    assert_eq!(plan.output_frames, 192_000);
    let rendered = Arc::new(SampleData::from_channels(RATE, vec![vec![0.25; 192_000]]).unwrap());
    h.resources = Arc::new(Loaded(BTreeMap::from([
        (("audio".to_owned(), take().key().to_owned()), original),
        ((RENDER_KIND.to_owned(), plan.key.clone()), rendered.clone()),
    ])));
    let c = h.settle(doc.root());
    assert!(c.stretches.is_empty() && c.complete(), "the rendering plays");
    h.play();
    let out = h.render(200_000 / BLOCK);
    for (pos, l, _) in out {
        let mono = l * std::f32::consts::SQRT_2;
        let want = if pos < 192_000 { 0.25 } else { 0.0 };
        assert!((mono - want).abs() < 1e-5, "at {pos}: {mono} (want {want}: the rendering, the item's length)");
    }
}
