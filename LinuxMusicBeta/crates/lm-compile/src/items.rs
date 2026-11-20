//! The flat structures of the items and the automation lanes (I-5.4, C1, I-6.2, V8), each kept in a cache by the
//! node's id and reused while the tree node is the very same one (`same_node`) and what else it depends on did not
//! change. Moving one note makes one new event list; every other item is the same `Arc` in the new program.
//!
//! - **MIDI items** are event lists in **ticks** (the renderer converts with the program's tempo map), so a tempo edit
//!   reuses them; only the transposition (P-37) and, for an item on the audio timebase, the tempo map enter the key.
//!   Notes come from the item's note map by number (C1) and are put in time order here; a note is cut at the item's
//!   end, and a note that starts at or after the end is left out.
//! - **Audio items** are in samples, with the comping flattened into one played segment per comp segment and a
//!   linear crossfade at each boundary (the segment's `fade`). They depend on the tempo map (a musical item's ticks)
//!   and on the loaded resources.
//! - **Automation curves** are points in samples through the tempo map; they depend on it.

use std::collections::HashMap;
use std::sync::Arc;

use lm_base::Result;
use lm_render::{
    AudioItem, AudioSource, Curve, CurvePoint, Expression, ItemEvent, ItemEventKind, MidiItem, SampleData, Segment,
    StreamFeeder, StreamSpec,
};
use lm_state::{NodeId, Value};
use lm_time::TempoMap;

use crate::catalog::{ResourceState, Resources};
use crate::read::{flag, id_of, int, items, real, ref_of, text};

/// Resources above this many seconds are streamed from the disk (P-32): the store says so with
/// [`ResourceState::OnDisk`] instead of decoding them
pub const STREAM_SECONDS: u32 = 30;

/// A stream the compiler made for a played segment of a long item: the session hands the feeder to `lm-io`'s
/// `DiskReader` with the resource's file (`serve(path, rate, feeder)`)
#[derive(Debug)]
pub struct StreamRegistration {
    pub resource: lm_state::Resource,
    pub feeder: StreamFeeder,
}

/// An item's played segments, the resources it waits for, and the streams made for it now
pub(crate) type AudioPlan = (Arc<[AudioItem]>, Vec<WaitingItem>, Vec<StreamRegistration>);

/// A lane's source, as loaded now
#[derive(Clone, Debug)]
enum LaneData {
    None,
    Memory(Arc<SampleData>),
    Disk { resource: lm_state::Resource, rate: u32, channels: usize, frames: u64 },
}

impl LaneData {
    fn same(&self, other: &LaneData) -> bool {
        match (self, other) {
            (LaneData::None, LaneData::None) => true,
            (LaneData::Memory(a), LaneData::Memory(b)) => Arc::ptr_eq(a, b),
            (
                LaneData::Disk { resource: a, rate: ra, channels: ca, frames: fa },
                LaneData::Disk { resource: b, rate: rb, channels: cb, frames: fb },
            ) => a == b && ra == rb && ca == cb && fa == fb,
            _ => false,
        }
    }
}

/// An item that cannot play yet, and why
#[derive(Clone, Debug, PartialEq)]
pub struct WaitingItem {
    pub item: NodeId,
    pub resource: lm_state::Resource,
    pub missing: bool,
}

#[derive(Debug)]
struct MidiEntry {
    item: Value,
    transpose: i32,
    tempo: Option<u64>,
    used: u64,
    out: Arc<MidiItem>,
}

#[derive(Debug)]
struct AudioEntry {
    item: Value,
    tempo: u64,
    data: Vec<LaneData>,
    used: u64,
    out: Arc<[AudioItem]>,
    waiting: Vec<WaitingItem>,
}

#[derive(Debug)]
struct CurveEntry {
    lane: Value,
    tempo: u64,
    used: u64,
    out: Option<Curve>,
}

/// The caches, kept by the compiler between compilations
#[derive(Debug, Default)]
pub(crate) struct Caches {
    midi: HashMap<NodeId, MidiEntry>,
    audio: HashMap<NodeId, AudioEntry>,
    curves: HashMap<NodeId, CurveEntry>,
    /// The current compilation's number (mark and sweep)
    pass: u64,
    /// Bumped whenever the tempo map changes
    pub(crate) tempo_version: u64,
    /// Counters of the last compilation: how many lists were made new
    pub(crate) made: Made,
}

/// How many flat structures the last compilation had to make (the rest were reused)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Made {
    pub midi_items: usize,
    pub audio_items: usize,
    pub curves: usize,
}

impl Caches {
    pub(crate) fn begin(&mut self) {
        self.pass += 1;
        self.made = Made::default();
    }

    /// Drops what the last compilation did not use
    pub(crate) fn sweep(&mut self) {
        let pass = self.pass;
        self.midi.retain(|_, e| e.used == pass);
        self.audio.retain(|_, e| e.used == pass);
        self.curves.retain(|_, e| e.used == pass);
    }

    /// The event list of a MIDI item (`None` for an item with nothing to play)
    pub(crate) fn midi(&mut self, id: NodeId, item: &Value, transpose: i32, tempo: &TempoMap) -> Result<Arc<MidiItem>> {
        let audio_time = text(item, "timebase", "musical") == "audio";
        let tempo_key = audio_time.then_some(self.tempo_version);
        if let Some(e) = self.midi.get_mut(&id)
            && e.item.same_node(item)
            && e.transpose == transpose
            && e.tempo == tempo_key
        {
            e.used = self.pass;
            return Ok(e.out.clone());
        }
        let out = Arc::new(midi_events(id, item, transpose, audio_time.then_some(tempo))?);
        self.made.midi_items += 1;
        self.midi.insert(
            id,
            MidiEntry { item: item.clone(), transpose, tempo: tempo_key, used: self.pass, out: out.clone() },
        );
        Ok(out)
    }

    /// The played segments of an audio item, the resources it waits for, and the streams made for it now (P-32).
    /// Audio at another rate than the project's (`rate`) is not played: the store converts it (base item 8)
    pub(crate) fn audio(
        &mut self,
        id: NodeId,
        item: &Value,
        tempo: &TempoMap,
        resources: &dyn Resources,
        rate: u32,
    ) -> Result<AudioPlan> {
        // The resources of the lanes, in lane order
        let mut data = Vec::new();
        let mut waiting = Vec::new();
        for lane in items(item, "lanes") {
            let Some(r) = lane.get("source").and_then(Value::as_resource) else {
                data.push(LaneData::None);
                continue;
            };
            let mut wait = |missing: bool| waiting.push(WaitingItem { item: id, resource: r.clone(), missing });
            data.push(match resources.audio(r) {
                ResourceState::Ready(d) if d.rate() == rate => LaneData::Memory(d),
                ResourceState::OnDisk { rate: r_rate, channels, frames }
                    if r_rate == rate && (1..=64).contains(&channels) =>
                {
                    LaneData::Disk { resource: r.clone(), rate, channels, frames }
                }
                ResourceState::Loading => {
                    wait(false);
                    LaneData::None
                }
                _ => {
                    wait(true);
                    LaneData::None
                }
            });
        }
        if let Some(e) = self.audio.get_mut(&id)
            && e.item.same_node(item)
            && e.tempo == self.tempo_version
            && e.data.len() == data.len()
            && e.data.iter().zip(&data).all(|(a, b)| a.same(b))
        {
            e.used = self.pass;
            return Ok((e.out.clone(), e.waiting.clone(), Vec::new()));
        }
        let (segments, streams) = audio_segments(item, tempo, &data)?;
        let out: Arc<[AudioItem]> = segments.into();
        self.made.audio_items += 1;
        self.audio.insert(
            id,
            AudioEntry {
                item: item.clone(),
                tempo: self.tempo_version,
                data,
                used: self.pass,
                out: out.clone(),
                waiting: waiting.clone(),
            },
        );
        Ok((out, waiting, streams))
    }

    /// Whether a streamed item of the last compilation still waits for the disk at its playhead (the export
    /// waits for this to be false, T8)
    pub(crate) fn streams_pending(&self) -> bool {
        self.audio.values().flat_map(|e| e.out.iter()).any(|it| match &it.source {
            AudioSource::Stream(s) => s.pending(),
            AudioSource::Memory(_) => false,
        })
    }

    /// The curve of an automation lane (`None`: off or without points)
    pub(crate) fn curve(&mut self, id: NodeId, lane: &Value, tempo: &TempoMap) -> Result<Option<Curve>> {
        if let Some(e) = self.curves.get_mut(&id)
            && e.lane.same_node(lane)
            && e.tempo == self.tempo_version
        {
            e.used = self.pass;
            return Ok(e.out.clone());
        }
        let out = lane_curve(lane, tempo)?;
        self.made.curves += 1;
        self.curves.insert(
            id,
            CurveEntry { lane: lane.clone(), tempo: self.tempo_version, used: self.pass, out: out.clone() },
        );
        Ok(out)
    }
}

/// The renderer's note number for a document note number: its low 32 bits, unless that collides inside the item
fn note_number(n: u64, taken: &mut std::collections::HashSet<u32>) -> u32 {
    let mut m = (n ^ (n >> 32)) as u32;
    while !taken.insert(m) {
        m = m.wrapping_add(0x9E37_79B9);
    }
    m
}

/// The time-ordered events of a MIDI item. `audio_time`: the item's start and length are in samples (its notes stay
/// in ticks from the tick where it starts)
pub(crate) fn midi_events(id: NodeId, item: &Value, transpose: i32, audio_time: Option<&TempoMap>) -> Result<MidiItem> {
    let (start, end) = match audio_time {
        Some(tempo) => {
            let s = int(item, "start", 0);
            let l = int(item, "length", 1);
            (tempo.tick_at(s), tempo.tick_at(s.saturating_add(l)))
        }
        None => {
            let s = int(item, "start", 0);
            (s, s.saturating_add(int(item, "length", 1)))
        }
    };
    // A pattern repeats its first `loop` ticks until the item ends; swing delays every second `step` (absent: no
    // repeat, no swing)
    let looped = item.get("loop").and_then(Value::as_int).filter(|l| *l >= 1);
    let span = end.saturating_sub(start).max(0);
    let reps = looped.map_or(1, |l| ((span + l - 1) / l).max(1));
    let swing = item.get("swing").and_then(Value::as_real).unwrap_or(0.0).clamp(0.0, 1.0);
    let step = item.get("step").and_then(Value::as_int).filter(|s| *s >= 1).unwrap_or(240);
    let swung = |rel: i64| -> i64 {
        if swing > 0.0 && rel.rem_euclid(2 * step) >= step {
            rel + (swing * step as f64 / 2.0).round() as i64
        } else {
            rel
        }
    };
    let mut events = Vec::new();
    let mut taken = std::collections::HashSet::new();
    if let Some(notes) = item.get("notes") {
        events.reserve(notes.len() * 2 * reps.min(64) as usize);
        for rep in 0..reps {
            for (k, note) in notes.entries() {
                let Some(n) = lm_state::parse_note_key(k) else { continue };
                let s = int(note, "start", 0);
                if looped.is_some_and(|l| s >= l) {
                    continue;
                }
                let on = start.saturating_add(swung(looped.map_or(0, |l| rep * l) + s));
                if on >= end {
                    continue;
                }
                let key = int(note, "key", 60) + i64::from(transpose);
                if !(0..=127).contains(&key) {
                    continue;
                }
                let off = on.saturating_add(int(note, "length", 1).max(1)).min(end);
                let num = note_number(n ^ ((rep as u64) << 44), &mut taken);
                let velocity = real(note, "velocity", 0.8).clamp(0.0, 1.0) as f32;
                events.push(ItemEvent {
                    tick: on,
                    kind: ItemEventKind::NoteOn { n: num, channel: 0, key: key as u8, velocity },
                });
                events.push(ItemEvent { tick: off, kind: ItemEventKind::NoteOff { n: num } });
                note_expressions(note, num, on, off, &mut events);
            }
        }
    }
    // The control lanes (I-21.4): each point at its tick, inside the item (looped like the notes)
    for lane in items(item, "controls") {
        let channel = (int(lane, "channel", 0).clamp(0, 15)) as u8;
        let number = (int(lane, "number", 0).clamp(0, 127)) as u8;
        let kind = text(lane, "kind", "cc");
        for rep in 0..reps {
            for p in items(lane, "points") {
                let t = int(p, "tick", 0);
                if looped.is_some_and(|l| t >= l) {
                    continue;
                }
                let at = start.saturating_add(looped.map_or(0, |l| rep * l) + t);
                if at >= end {
                    continue;
                }
                let v = int(p, "value", 0).clamp(0, 16383);
                let v7 = v.min(127) as u8;
                let bytes = match kind {
                    "pitchBend" => [0xE0 | channel, (v & 0x7F) as u8, ((v >> 7) & 0x7F) as u8],
                    "channelPressure" => [0xD0 | channel, v7, 0],
                    "polyPressure" => [0xA0 | channel, number, v7],
                    "programChange" => [0xC0 | channel, v7, 0],
                    _ => [0xB0 | channel, number, v7],
                };
                events.push(ItemEvent { tick: at, kind: ItemEventKind::Midi { bytes } });
            }
        }
    }
    MidiItem::new(crate::read::id_number(id), events)
}

/// A note's expressions (F-NOT-010, P-24): its pan at its start, and its bend curve as one ramp per segment
/// (linear between points, the first point's value before it), cut at the note's end
fn note_expressions(note: &Value, n: u32, on: i64, off: i64, events: &mut Vec<ItemEvent>) {
    let pan = real(note, "pan", 0.0).clamp(-1.0, 1.0) as f32;
    if pan != 0.0 {
        events.push(ItemEvent {
            tick: on,
            kind: ItemEventKind::Expression { n, expression: Expression::Pan, value: pan, until: on },
        });
    }
    let mut points: Vec<(i64, f32)> = items(note, "bend")
        .map(|p| (int(p, "at", 0).max(0), real(p, "semitones", 0.0).clamp(-24.0, 24.0) as f32))
        .collect();
    if points.is_empty() {
        return;
    }
    points.sort_by_key(|p| p.0);
    let tuning = |tick: i64, value: f32, until: i64| ItemEvent {
        tick,
        kind: ItemEventKind::Expression { n, expression: Expression::Tuning, value, until },
    };
    events.push(tuning(on, points[0].1, on));
    for w in points.windows(2) {
        let (a, b) = (on.saturating_add(w[0].0), on.saturating_add(w[1].0));
        if a >= off {
            break;
        }
        events.push(tuning(a, w[1].1, b));
    }
}

/// The played segments of an audio item (comping flattened), and a stream for each segment of a long source
/// (P-32)
fn audio_segments(
    item: &Value,
    tempo: &TempoMap,
    data: &[LaneData],
) -> Result<(Vec<AudioItem>, Vec<StreamRegistration>)> {
    let audio_time = text(item, "timebase", "musical") == "audio";
    let start = int(item, "start", 0);
    let length = int(item, "length", 1);
    // Positions inside the item (in its own unit) to timeline samples
    let at = |rel: i64| -> i64 {
        if audio_time { start.saturating_add(rel) } else { tempo.sample_at(start.saturating_add(rel)) }
    };
    let item_start = at(0);
    let item_end = at(length);
    let lanes: Vec<(Option<NodeId>, i64)> =
        items(item, "lanes").map(|l| (id_of(l), int(l, "sourceOffset", 0))).collect();
    let lane_index = |id: Option<NodeId>| id.and_then(|id| lanes.iter().position(|(l, _)| *l == Some(id)));

    // The segments: (from, to) in timeline samples, the lane, the crossfade into it
    let mut segs: Vec<(i64, i64, Option<usize>, u32)> = Vec::new();
    let comp: Vec<&Value> = items(item, "comp").collect();
    if comp.is_empty() {
        if !lanes.is_empty() {
            segs.push((item_start, item_end, Some(lanes.len() - 1), 0));
        }
    } else {
        let mut sorted: Vec<(i64, i64, Option<usize>, u32)> = comp
            .iter()
            .map(|c| {
                let from = int(c, "from", 0).clamp(0, length);
                let to = int(c, "to", 0).clamp(0, length);
                let fade = u32::try_from(int(c, "fade", 240).max(0)).unwrap_or(u32::MAX);
                (at(from), at(to), lane_index(ref_of(c, "lane")), fade)
            })
            .filter(|(a, b, _, _)| b > a)
            .collect();
        sorted.sort_by_key(|s| s.0);
        segs = sorted;
    }
    let fade_in = u32::try_from(int(item, "fadeIn", 0).max(0)).unwrap_or(u32::MAX);
    let fade_out = u32::try_from(int(item, "fadeOut", 0).max(0)).unwrap_or(u32::MAX);
    let gain = real(item, "gain", 1.0) as f32;
    let mut out = Vec::new();
    let mut streams = Vec::new();
    let n = segs.len();
    for i in 0..n {
        let (from, to, lane, fade) = segs[i];
        let Some(lane) = lane else { continue };
        let Some(d) = data.get(lane) else { continue };
        if matches!(d, LaneData::None) {
            continue;
        }
        // The crossfade into the next segment: this one goes on for the next one's fade
        let next_fade = if i + 1 < n && segs[i + 1].0 <= to { i64::from(segs[i + 1].3) } else { 0 };
        let end = (to + next_fade).min(item_end);
        let begin = from.max(item_start);
        if end <= begin {
            continue;
        }
        let offset = lanes[lane].1 + (begin - item_start);
        let mut fi = if i == 0 || begin == item_start { fade_in } else { fade };
        let mut fo = if end >= item_end { fade_out } else { u32::try_from(next_fade).unwrap_or(0) };
        let len = end - begin;
        let cap = u32::try_from(len).unwrap_or(u32::MAX);
        fi = fi.min(cap);
        fo = fo.min(cap);
        let source = match d {
            LaneData::Memory(d) => AudioSource::Memory(d.clone()),
            LaneData::Disk { resource, rate, channels, frames } => {
                let (stream, feeder) = lm_render::stream(StreamSpec::new(*rate, *channels, *frames))?;
                // The head is read before the program is even installed
                stream.set_playhead(u64::try_from(offset).ok());
                streams.push(StreamRegistration { resource: resource.clone(), feeder });
                AudioSource::Stream(stream)
            }
            LaneData::None => continue,
        };
        out.push(AudioItem { start: begin, length: len, source, offset, gain, fade_in: fi, fade_out: fo });
    }
    Ok((out, streams))
}

/// The curve of an automation lane, its ticks converted to samples
pub(crate) fn lane_curve(lane: &Value, tempo: &TempoMap) -> Result<Option<Curve>> {
    if !flag(lane, "on", true) {
        return Ok(None);
    }
    if lane.get("mode").and_then(Value::as_text) == Some("off") {
        return Ok(None);
    }
    // A point: its tick, the segment that ends at it (shape, bend, tension) and its value; a point with `out` is two
    // points at its sample (a jump, P-27), the second one leaving with `out`
    let mut points: Vec<(i64, CurvePoint, Segment)> = Vec::new();
    for p in items(lane, "points") {
        let tick = int(p, "tick", 0);
        let sample = tempo.sample_at(tick);
        let segment = match p.get("shape").and_then(Value::as_text) {
            Some("step") => Segment::Step,
            Some("smooth") => Segment::Smooth { tension: real(p, "tension", 0.5).clamp(0.0, 1.0) as f32 },
            _ => Segment::Curve,
        };
        let value = real(p, "value", 0.0) as f32;
        let shape = real(p, "curve", 0.0).clamp(-1.0, 1.0) as f32;
        points.push((tick, CurvePoint { sample, value, shape }, segment));
        if let Some(out) = p.get("out").and_then(Value::as_real) {
            points.push((tick, CurvePoint { sample, value: out as f32, shape: 0.0 }, Segment::Curve));
        }
    }
    if points.is_empty() {
        return Ok(None);
    }
    points.sort_by_key(|(t, _, _)| *t); // stable: two points at one tick stay in order (a jump, P-27)
    let (pts, segs): (Vec<CurvePoint>, Vec<Segment>) = points.into_iter().map(|(_, p, s)| (p, s)).unzip();
    Ok(Some(Curve::with_segments(pts, segs)?))
}
