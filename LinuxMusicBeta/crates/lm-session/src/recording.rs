//! The pieces of recording that need no session (I-11, I-5.5): the metronome's sounds, the growing waveform of the
//! takes being written (U3), and how a closed take becomes items (F-REC-022, F-REC-040, F-REC-041, F-REC-046).

use std::collections::BTreeMap;
use std::sync::Arc;

use lm_io::{LivePeak, Peaks, PeaksBuilder, Take};
use lm_render::SampleData;
use lm_state::{NodeId, Resource, Value};
use lm_time::TempoMap;

// ---------------------------------------------------------------------------------------------------------------
// The metronome's sounds (I-5.5)

/// The accent and the beat of the metronome at `rate`: a short sine burst each, the accent higher and louder. Made
/// here, so the metronome sounds with no file to find
pub fn metronome_sounds(rate: u32) -> Option<(Arc<SampleData>, Arc<SampleData>)> {
    let burst = |freq: f64, gain: f32| {
        let frames = (f64::from(rate) * 0.035) as usize;
        let tau = f64::from(rate) * 0.007;
        let samples: Vec<f32> = (0..frames)
            .map(|i| {
                let t = i as f64;
                let phase = 2.0 * std::f64::consts::PI * freq * t / f64::from(rate);
                // a 1 ms attack keeps the start from clicking, then a decay
                let attack = (t / (f64::from(rate) * 0.001)).min(1.0);
                (phase.sin() * (-t / tau).exp() * attack) as f32 * gain
            })
            .collect();
        SampleData::from_channels(rate, vec![samples]).ok().map(Arc::new)
    };
    Some((burst(1600.0, 0.9)?, burst(1000.0, 0.6)?))
}

// ---------------------------------------------------------------------------------------------------------------
// The waveform that grows (U3)

/// A take being written: its pairs as they come from the disk thread
#[derive(Debug)]
struct Live {
    track: NodeId,
    resource: Resource,
    /// Timeline sample of the file's first frame, the latency and the PDC taken off
    start: i64,
    builder: PeaksBuilder,
    /// The pyramid of what came, rebuilt when asked for after news
    cached: Option<Arc<Peaks>>,
    dirty: bool,
}

/// The takes being written, by (input, take number)
#[derive(Debug, Default)]
pub struct LiveTakes {
    takes: BTreeMap<(u32, u32), Live>,
    /// The track and the channels of each input of the recording
    inputs: BTreeMap<u32, (NodeId, usize)>,
    /// The timeline sample of the first frame of the newest take of each input
    starts: BTreeMap<u32, i64>,
    /// The latency and the PDC the takes are placed back by
    shift: i64,
    serial: u64,
}

impl LiveTakes {
    /// A recording starts: the track and the channels each input records
    pub fn plan(&mut self, inputs: impl IntoIterator<Item = (u32, NodeId, usize)>, shift: i64) {
        self.takes.clear();
        self.starts.clear();
        self.inputs = inputs.into_iter().map(|(i, t, c)| (i, (t, c))).collect();
        self.shift = shift;
        self.serial += 1;
    }

    /// The resource a live take is drawn from: no file has this key, so it never meets a real one
    fn resource(&self, input: u32, take: u32) -> Option<Resource> {
        let name = format!("live-take/{}/{input}/{take}", self.serial);
        Resource::of_content("audio", name.as_bytes()).ok()
    }

    /// The disk thread created the file of a take of `input`
    pub fn started(&mut self, input: u32, start: i64) {
        self.starts.insert(input, start);
    }

    /// One level-0 pair of a take: the first of a take opens it
    pub fn pair(&mut self, p: &LivePeak) {
        let key = (p.input.0, p.take);
        if !self.takes.contains_key(&key) {
            let (Some((track, channels)), Some(resource)) =
                (self.inputs.get(&key.0).copied(), self.resource(key.0, key.1))
            else {
                return;
            };
            let start = self.starts.get(&key.0).copied().unwrap_or(0) - self.shift;
            let builder = PeaksBuilder::new(channels.clamp(1, lm_io::LIVE_PEAK_CHANNELS));
            self.takes.insert(key, Live { track, resource, start, builder, cached: None, dirty: false });
        }
        let Some(live) = self.takes.get_mut(&key) else { return };
        let n = usize::from(p.channels).clamp(1, lm_io::LIVE_PEAK_CHANNELS);
        live.builder.push_pair(&p.min[..n], &p.max[..n]);
        live.dirty = true;
    }

    /// What the interface draws: each take with the length it has now
    pub fn views(&self) -> Vec<lm_port::RecordingTake> {
        self.takes
            .values()
            .filter(|l| l.builder.frames() > 0)
            .map(|l| lm_port::RecordingTake {
                track: l.track,
                resource: l.resource.clone(),
                start: l.start.max(0),
                length: l.builder.frames() as i64,
            })
            .collect()
    }

    /// The peaks of a live take, if `key` is one; the pyramid is rebuilt when pairs came since the last ask
    pub fn peaks(&mut self, key: &str) -> Option<Arc<Peaks>> {
        let live = self.takes.values_mut().find(|l| l.resource.key() == key)?;
        if live.dirty || live.cached.is_none() {
            live.cached = Some(Arc::new(live.builder.snapshot()));
            live.dirty = false;
        }
        live.cached.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.takes.is_empty()
    }

    /// The recording is in the tree: the live waveforms go away
    pub fn clear(&mut self) {
        self.takes.clear();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// From a take to items

/// An item to make from a take: its place on the timeline in samples (the latency not yet taken off), and one
/// lane per layer, each with the source frame at the item's start
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedItem {
    pub start: i64,
    pub frames: i64,
    pub lanes: Vec<i64>,
}

#[derive(Clone, Copy, Debug)]
struct Pass {
    sample: i64,
    file_frame: i64,
    frames: i64,
}

impl Pass {
    fn end(&self) -> i64 {
        self.sample + self.frames
    }
}

/// How a closed take lands on the timeline.
///
/// The item covers only the recorded spans, never the handles the file keeps beyond them (the pre-roll and what was
/// outside the punch region, F-REC-022). A take with loop cuts is one pass per lap: each lap is a layer of one item,
/// the newest on top (it plays; F-REC-040, F-REC-041), and a lap shorter than 90% of the longest (stopped in the
/// middle) is thrown away. Without `layered` (the simple recorder, F-REC-042) only the last good lap is kept.
/// Spans a punch left apart (record, stop recording, record again) are items of their own.
pub fn plan_items(take: &Take, layered: bool) -> Vec<PlannedItem> {
    let whole = [lm_io::TakeSpan { file_frame: 0, sample: take.start_sample, frames: take.frames.max(0) as u64 }];
    let spans = if take.spans.is_empty() { &whole[..] } else { &take.spans[..] };
    let mut cuts: Vec<u64> = take.cuts.iter().map(|c| c.file_frame).collect();
    cuts.sort_unstable();
    if cuts.is_empty() {
        return spans
            .iter()
            .filter(|s| s.frames > 0)
            .map(|s| PlannedItem { start: s.sample, frames: s.frames as i64, lanes: vec![s.file_frame as i64] })
            .collect();
    }
    // The laps: the spans between two cuts
    let mut passes: Vec<Pass> = Vec::new();
    let mut lap_of_last = usize::MAX;
    for s in spans.iter().filter(|s| s.frames > 0) {
        let lap = cuts.partition_point(|c| *c <= s.file_frame);
        let end = s.sample + s.frames as i64;
        if lap == lap_of_last {
            if let Some(p) = passes.last_mut() {
                p.frames = (end - p.sample).max(p.frames);
            }
        } else {
            passes.push(Pass { sample: s.sample, file_frame: s.file_frame as i64, frames: s.frames as i64 });
            lap_of_last = lap;
        }
    }
    let Some(longest) = passes.iter().map(|p| p.frames).max() else { return Vec::new() };
    let mut kept: Vec<Pass> = passes.into_iter().filter(|p| p.frames * 10 >= longest * 9).collect();
    if !layered {
        kept = kept.split_off(kept.len().saturating_sub(1));
    }
    // Every layer must cover the item: from the latest start to the earliest end
    let start = kept.iter().map(|p| p.sample).max().unwrap_or(0);
    let end = kept.iter().map(Pass::end).min().unwrap_or(0);
    if end <= start {
        let Some(best) = kept.iter().max_by_key(|p| p.frames).copied() else { return Vec::new() };
        return vec![PlannedItem { start: best.sample, frames: best.frames, lanes: vec![best.file_frame] }];
    }
    vec![PlannedItem {
        start,
        frames: end - start,
        lanes: kept.iter().map(|p| p.file_frame + (start - p.sample)).collect(),
    }]
}

/// The take's tempo (F-REC-046): the source frame at the item's first tick, at every tempo point inside it, and at
/// its last, as the tempo map now puts them. A later tempo change stretches the take from the original (F-REC-090)
pub fn timing_points(map: &TempoMap, from_tick: i64, to_tick: i64) -> Vec<(i64, f64)> {
    if to_tick <= from_tick {
        return Vec::new();
    }
    let base = map.sample_at(from_tick);
    let mut ticks = vec![from_tick];
    ticks.extend(map.points().iter().map(|p| p.tick).filter(|t| *t > from_tick && *t < to_tick));
    ticks.push(to_tick);
    let mut out: Vec<(i64, f64)> = Vec::with_capacity(ticks.len());
    for t in ticks {
        let frame = (map.sample_at(t) - base) as f64;
        if out.last().is_none_or(|(pt, pf)| t > *pt && frame > *pf) {
            out.push((t - from_tick, frame));
        }
    }
    out
}

/// The audio item the plan makes: on the timeline at `shift` samples (the latency and the PDC) before where it was
/// heard, `resource` the take's file
pub fn audio_item(
    map: Option<&TempoMap>,
    planned: &PlannedItem,
    resource: &Resource,
    shift: i64,
    gaps: &[lm_io::TakeGap],
) -> Value {
    let mut start = planned.start - shift;
    let mut frames = planned.frames;
    let mut clipped = 0;
    if start < 0 {
        clipped = -start;
        frames -= clipped;
        start = 0;
    }
    let frames = frames.max(1);
    let (start_tick, length) = match map {
        Some(m) => {
            let a = m.tick_at(start);
            (a, (m.tick_at(start + frames) - a).max(1))
        }
        None => (0, 1),
    };
    let timing = map.map(|m| timing_points(m, start_tick, start_tick + length)).unwrap_or_default();
    let lanes = planned.lanes.iter().map(|offset| {
        let mut lane = vec![
            ("source", Value::resource(resource.clone())),
            ("sourceOffset", Value::int((offset + clipped).max(0))),
        ];
        // The xrun gaps that fall inside what this lane plays
        let from = (offset + clipped).max(0);
        let to = from + frames;
        let marked: Vec<Value> = gaps
            .iter()
            .filter_map(|g| {
                let (a, b) = (i64::try_from(g.file_frame).ok()?, i64::try_from(g.file_frame.saturating_add(g.frames)).ok()?);
                (b > from && a < to && b > a)
                    .then(|| Value::object([("frame", Value::int(a)), ("frames", Value::int(b - a))]))
            })
            .collect();
        if !marked.is_empty() {
            lane.push(("gaps", Value::list(marked)));
        }
        if timing.len() >= 2 {
            let points = timing.iter().filter_map(|(tick, frame)| {
                Some(Value::object([("tick", Value::int(*tick)), ("frame", Value::real(*frame).ok()?)]))
            });
            lane.push(("timing", Value::list(points)));
        }
        Value::object(lane)
    });
    Value::object([
        ("kind", Value::text("audio")),
        ("name", Value::text("")),
        ("start", Value::int(start_tick)),
        ("length", Value::int(length)),
        ("lanes", Value::list(lanes)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use lm_io::{InputIndex, LoopCut, TakeSpan};
    use std::path::PathBuf;

    fn take(spans: Vec<TakeSpan>, cuts: Vec<LoopCut>, frames: i64) -> Take {
        Take {
            input: InputIndex(0),
            path: PathBuf::new(),
            record: PathBuf::new(),
            start_sample: 1000,
            frames,
            channels: 1,
            rate: 48_000,
            spans,
            cuts,
            gaps: Vec::new(),
            stopped: None,
            latency: 0,
            pdc: 0,
            tracks: Vec::new(),
        }
    }

    fn span(file_frame: u64, sample: i64, frames: u64) -> TakeSpan {
        TakeSpan { file_frame, sample, frames }
    }

    fn cut(file_frame: u64, from: i64, to: i64) -> LoopCut {
        LoopCut { file_frame, from_sample: from, to_sample: to }
    }

    #[test]
    fn the_metronome_sounds_have_the_rate_and_the_accent_is_louder() {
        let (accent, beat) = metronome_sounds(48_000).expect("sounds");
        assert_eq!(accent.rate(), 48_000);
        assert_eq!(beat.rate(), 48_000);
        let peak = |d: &SampleData| d.channel(0).iter().fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(peak(&accent) > peak(&beat));
        assert!(peak(&beat) > 0.1);
        assert_eq!(accent.frames(), 1680);
    }

    #[test]
    fn the_item_covers_the_punch_region_and_not_the_handles() {
        // the file starts 500 frames before the region (a handle) and runs 400 past it
        let t = take(vec![span(500, 1500, 2000)], vec![], 2900);
        let items = plan_items(&t, true);
        assert_eq!(items, vec![PlannedItem { start: 1500, frames: 2000, lanes: vec![500] }]);
    }

    #[test]
    fn a_take_without_spans_is_the_whole_file() {
        let t = take(vec![], vec![], 3000);
        assert_eq!(plan_items(&t, true), vec![PlannedItem { start: 1000, frames: 3000, lanes: vec![0] }]);
    }

    #[test]
    fn spans_a_punch_left_apart_are_two_items() {
        let t = take(vec![span(0, 1000, 500), span(900, 1900, 500)], vec![], 1400);
        let items = plan_items(&t, true);
        assert_eq!(items.len(), 2);
        assert_eq!(items[1], PlannedItem { start: 1900, frames: 500, lanes: vec![900] });
    }

    #[test]
    fn each_lap_of_a_loop_is_a_layer_and_a_short_lap_is_dropped() {
        // loop 1000..2000: three full laps, then a stop 300 frames into the fourth
        let spans = vec![span(0, 1000, 1000), span(1000, 1000, 1000), span(2000, 1000, 1000), span(3000, 1000, 300)];
        let cuts = vec![cut(1000, 2000, 1000), cut(2000, 2000, 1000), cut(3000, 2000, 1000)];
        let t = take(spans, cuts, 3300);
        let items = plan_items(&t, true);
        assert_eq!(items, vec![PlannedItem { start: 1000, frames: 1000, lanes: vec![0, 1000, 2000] }]);
        // the simple recorder keeps the last good lap only
        let simple = plan_items(&t, false);
        assert_eq!(simple, vec![PlannedItem { start: 1000, frames: 1000, lanes: vec![2000] }]);
    }

    #[test]
    fn a_first_lap_that_started_late_is_cut_to_what_every_layer_has() {
        // recording started 40 frames into the loop: the first lap is 960 long (96%: kept)
        let spans = vec![span(0, 1040, 960), span(960, 1000, 1000)];
        let cuts = vec![cut(960, 2000, 1000)];
        let t = take(spans, cuts, 1960);
        let items = plan_items(&t, true);
        assert_eq!(items, vec![PlannedItem { start: 1040, frames: 960, lanes: vec![0, 1000] }]);
    }

    #[test]
    fn the_item_is_moved_back_by_the_latency_and_clipped_at_zero() {
        let planned = PlannedItem { start: 300, frames: 1000, lanes: vec![50] };
        let resource = Resource::of_content("audio", b"x").expect("resource");
        let item = audio_item(None, &planned, &resource, 500, &[]);
        assert_eq!(item.get("start").and_then(Value::as_int), Some(0));
        let lane = item.get("lanes").and_then(|l| l.items().next().cloned()).expect("lane");
        // 200 frames fell before the start of the song: the source starts that much later
        assert_eq!(lane.get("sourceOffset").and_then(Value::as_int), Some(250));
    }

    #[test]
    fn xrun_gaps_inside_the_lane_are_marked_on_it() {
        let planned = PlannedItem { start: 0, frames: 1000, lanes: vec![100] };
        let resource = Resource::of_content("audio", b"x").expect("resource");
        let gaps = [
            lm_io::TakeGap { file_frame: 400, frames: 64 },
            // before the part the lane plays
            lm_io::TakeGap { file_frame: 10, frames: 20 },
            // after it
            lm_io::TakeGap { file_frame: 5000, frames: 20 },
        ];
        let item = audio_item(None, &planned, &resource, 0, &gaps);
        let lane = item.get("lanes").and_then(|l| l.items().next().cloned()).expect("lane");
        let marked: Vec<_> = lane.get("gaps").expect("gaps").items().cloned().collect();
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].get("frame").and_then(Value::as_int), Some(400));
        assert_eq!(marked[0].get("frames").and_then(Value::as_int), Some(64));
    }

    #[test]
    fn the_timing_follows_the_tempo_map() {
        let map = lm_time::TempoMap::constant(
            48_000,
            lm_time::Ppq::DEFAULT,
            lm_time::Bpm::from_integer(120).expect("bpm"),
            lm_time::TimeSignature::COMMON,
        )
        .expect("map");
        let points = timing_points(&map, 0, 3840);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0], (0, 0.0));
        assert_eq!(points[1].0, 3840);
        assert!((points[1].1 - 96_000.0).abs() < 2.0);
        assert!(timing_points(&map, 10, 10).is_empty());
    }

    #[test]
    fn live_takes_grow_and_vanish() {
        let node = NodeId::parse("t:1").expect("id");
        let mut live = LiveTakes::default();
        live.plan([(0, node, 1)], 100);
        live.started(0, 5000);
        let mut p = LivePeak {
            input: InputIndex(0),
            take: 1,
            pair: 0,
            channels: 1,
            min: [0; lm_io::LIVE_PEAK_CHANNELS],
            max: [0; lm_io::LIVE_PEAK_CHANNELS],
        };
        p.min[0] = -1000;
        p.max[0] = 2000;
        live.pair(&p);
        live.pair(&p);
        let views = live.views();
        assert_eq!(views.len(), 1);
        assert_eq!((views[0].start, views[0].length), (4900, 128));
        let peaks = live.peaks(views[0].resource.key()).expect("peaks");
        assert_eq!(peaks.pairs(0), 2);
        assert_eq!(peaks.pair(0, 0, 1), Some((-1000, 2000)));
        live.clear();
        assert!(live.views().is_empty());
        assert!(live.peaks(views[0].resource.key()).is_none());
    }
}
