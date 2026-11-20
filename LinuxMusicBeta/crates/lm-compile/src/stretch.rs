//! Takes that follow the tempo (F-REC-046, F-REC-090): what a lane with a `timing` plays.
//!
//! A lane's `timing` is the tempo its take was played at: the source frame (counted from the lane's `sourceOffset`)
//! at each tick from its item's start. On a musical item, the item's ticks fall on samples by the project's tempo map
//! now; when that puts every timing point where it was played (the same tempo), the original plays as it is, its
//! frames where the timing says. Otherwise the lane plays a **rendering**: the original stretched so that each tick
//! lands on its sample now, rendered off the audio thread from the original (always the original: a rendering is
//! never stretched again) into a file of exactly the item's length in samples, frame 0 at the item's start.
//!
//! The rendering is named by a hash of what makes it ([`StretchPlan::key`]): the original, the key frames, the
//! length, the rate and the version of the rendering code. Undoing a tempo change, or moving the item to a place of
//! the same tempo, finds the rendering already made.
//!
//! The key frames: the timing's points, the tempo map's points inside the item, and a point every
//! [`RAMP_STEP`] ticks inside a ramp (the stretch is even between two key frames, and a ramp's samples are not even
//! in ticks); each is (the source frame there, the item's sample there), rounded, inside the source and the item.

use lm_state::Resource;
use lm_time::TempoMap;

/// The version of the rendering (its algorithm and its settings): a change makes new keys, never reusing renderings
/// made by another
pub const RENDER_VERSION: u32 = 1;
/// The resource kind of a rendering (`render/<key>` in the project, a cache that can be emptied)
pub const RENDER_KIND: &str = "render";
/// Ticks between two key frames inside a tempo ramp (a sixteenth at 960 per quarter)
pub const RAMP_STEP: i64 = 240;

/// A rendering to make: from `source`, exactly `output_frames` frames at `rate`, each key frame's source frame at its
/// output frame (silence outside the first and the last)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StretchPlan {
    pub source: Resource,
    /// 64 hex digits: the hash of the plan
    pub key: String,
    /// (source frame, output frame), both strictly increasing; empty when the source is outside the item
    pub key_frames: Vec<(u64, u64)>,
    pub output_frames: u64,
    pub rate: u32,
}

impl StretchPlan {
    /// The rendering's resource
    pub fn resource(&self) -> Resource {
        Resource::new(RENDER_KIND, &self.key).unwrap_or_else(|_| unreachable!("a hash is a valid key"))
    }

    /// Seconds of audio the rendering makes (for the estimate)
    pub fn seconds(&self) -> f64 {
        self.output_frames as f64 / f64::from(self.rate.max(1))
    }
}

/// What a lane plays: the original, its frame `offset` at the item's start (negative: the source starts that much
/// later), or the rendering `render` (frame 0 at the item's start). Until the rendering is ready, the original plays
/// from `offset` as it is
#[derive(Clone, Debug, PartialEq)]
pub struct LanePlay {
    pub offset: i64,
    pub render: Option<StretchPlan>,
}

/// The source frame at each tick, from the timing points: linear between two, and on from the first and the last
/// pair outside them
fn frame_at(points: &[(i64, f64)], tick: i64) -> f64 {
    let seg = match points.iter().position(|p| p.0 > tick) {
        Some(0) => 0,
        Some(i) => i - 1,
        None => points.len() - 2,
    };
    let (a, b) = (points[seg], points[seg + 1]);
    a.1 + (tick - a.0) as f64 * (b.1 - a.1) / (b.0 - a.0) as f64
}

/// What a lane plays: its item at `start` ticks for `length`, its `source` (of `source_frames` frames) from
/// `source_offset`, played at `timing` (ticks from the item's start, frames from the offset), the tempo map now.
/// `None` when the timing cannot be read (fewer than two points, ticks or frames not increasing): the lane plays as
/// one without timing
pub fn lane_play(
    start: i64,
    length: i64,
    source: &Resource,
    source_offset: i64,
    source_frames: u64,
    timing: &[(i64, f64)],
    tempo: &TempoMap,
) -> Option<LanePlay> {
    if timing.len() < 2 || timing.windows(2).any(|w| w[1].0 <= w[0].0 || w[1].1 <= w[0].1 || !w[1].1.is_finite()) {
        return None;
    }
    let length = length.max(1);
    let t0 = tempo.sample_at(start);
    let output = tempo.sample_at(start.saturating_add(length)) - t0;
    let f = |t: i64| source_offset as f64 + frame_at(timing, t);
    let g = |t: i64| tempo.sample_at(start.saturating_add(t)) - t0;

    // The breakpoints: the item's ends, the timing's points, the map's points and ramps inside the item
    let mut ticks: Vec<i64> = vec![0, length];
    ticks.extend(timing.iter().map(|p| p.0).filter(|t| (1..length).contains(t)));
    for seg in tempo.segments() {
        let (a, b) = (seg.start_tick().saturating_sub(start), seg.end_tick().saturating_sub(start));
        if a > 0 && a < length {
            ticks.push(a);
        }
        if seg.is_ramp() {
            let mut t = a.max(0) + RAMP_STEP;
            while t < b.min(length) {
                ticks.push(t);
                t += RAMP_STEP;
            }
        }
    }
    ticks.sort_unstable();
    ticks.dedup();

    // The same tempo: every point where it was played
    let first = f(0);
    let offset = first.round() as i64;
    if first == first.round() && ticks.iter().all(|t| f(*t) - first == g(*t) as f64) {
        return Some(LanePlay { offset, render: None });
    }

    // The points inside the source, rounded and strictly increasing
    let n = source_frames as f64;
    let mut frames: Vec<(f64, f64)> = Vec::with_capacity(ticks.len() + 2);
    for w in ticks.windows(2) {
        let (fa, fb) = (f(w[0]), f(w[1]));
        let (ga, gb) = (g(w[0]) as f64, g(w[1]) as f64);
        let at = |x: f64| ga + (x - fa) * (gb - ga) / (fb - fa);
        let (lo, hi) = (fa.max(0.0), fb.min(n));
        if lo >= hi {
            continue;
        }
        frames.push((lo, at(lo)));
        frames.push((hi, at(hi)));
    }
    let mut key_frames: Vec<(u64, u64)> = Vec::with_capacity(frames.len());
    for (x, y) in frames {
        let (x, y) = (x.round().max(0.0) as u64, y.round().clamp(0.0, output as f64) as u64);
        if key_frames.last().is_none_or(|l| x > l.0 && y > l.1) {
            key_frames.push((x, y));
        }
    }
    if key_frames.len() < 2 {
        key_frames.clear();
    }
    let output_frames = u64::try_from(output).unwrap_or(0);
    let rate = tempo.sample_rate();
    let key = plan_key(source, &key_frames, output_frames, rate);
    Some(LanePlay {
        offset,
        render: Some(StretchPlan { source: source.clone(), key, key_frames, output_frames, rate }),
    })
}

fn plan_key(source: &Resource, key_frames: &[(u64, u64)], output_frames: u64, rate: u32) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"lm-stretch\0");
    h.update(&RENDER_VERSION.to_le_bytes());
    h.update(source.kind().as_bytes());
    h.update(b"\0");
    h.update(source.key().as_bytes());
    h.update(b"\0");
    h.update(&rate.to_le_bytes());
    h.update(&output_frames.to_le_bytes());
    h.update(&(key_frames.len() as u64).to_le_bytes());
    for (a, b) in key_frames {
        h.update(&a.to_le_bytes());
        h.update(&b.to_le_bytes());
    }
    h.finalize().to_hex().to_string()
}

/// The timing of a take recorded from `start_sample` (its frame 0 there) for `frames` frames, into an item at
/// `start_tick`: the frame at the item's start, at each point of the map inside the take, and at its end tick
/// (`commit_recording`'s `timing`)
pub fn recorded_timing(tempo: &TempoMap, start_tick: i64, start_sample: i64, frames: i64) -> Vec<(i64, f64)> {
    let end_tick = tempo.tick_at(start_sample + frames) + 1;
    let mut ticks = vec![0, (end_tick - start_tick).max(1)];
    for seg in tempo.segments() {
        let a = seg.start_tick().saturating_sub(start_tick);
        if a > 0 && a < ticks[1] {
            ticks.push(a);
        }
        if seg.is_ramp() {
            let mut t = a.max(0) + RAMP_STEP;
            while t < seg.end_tick().saturating_sub(start_tick).min(ticks[1]) {
                ticks.push(t);
                t += RAMP_STEP;
            }
        }
    }
    ticks.sort_unstable();
    ticks.dedup();
    ticks.into_iter().map(|t| (t, (tempo.sample_at(start_tick + t) - start_sample) as f64)).collect()
}
