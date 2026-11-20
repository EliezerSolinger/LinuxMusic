//! The composition's fake data (owner: agent A): track kinds and folders, several outputs per track, audio and
//! MIDI items, take layers with comping (U2), pre-roll and punch handles (U4), the tempo map with ramps, automation
//! lanes, loop, punch and markers. Coherent with the rest: the same tracks, channels and resources.

use std::sync::Arc;

use crate::{NodeId, ResourceId, SAMPLE_RATE};

/// Samples per bar in the fake (4/4 at 120 BPM; the tempo map changes only what the ruler and the lane show)
pub const BAR: i64 = 2 * SAMPLE_RATE as i64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Instrument,
    Folder,
}

/// One output of a track (I-3.9): a port of the instrument (or a split of the audio), sent to a desk channel
#[derive(Clone, Debug)]
pub struct Output {
    pub id: NodeId,
    /// stable key ("main", "kick", "oh")
    pub port: &'static str,
    pub channels: u8,
    /// the desk channel it sends to
    pub to: NodeId,
}

/// A take (one recorded pass) of an audio item
#[derive(Clone, Debug)]
pub struct Take {
    pub id: NodeId,
    pub resource: ResourceId,
}

/// A piece of the comp: from `start` (samples inside the item) on, take `take` plays; each boundary has a crossfade
#[derive(Clone, Debug)]
pub struct CompSegment {
    pub start: i64,
    pub take: NodeId,
    pub crossfade_ms: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct MiniNote {
    /// samples from the item's start
    pub start: i64,
    pub length: i64,
    pub key: u8,
    pub velocity: f32,
}

#[derive(Clone, Debug)]
pub enum ItemKind {
    Audio {
        resource: ResourceId,
        /// the take layers; empty when the item has a single take (`resource`)
        takes: Vec<Take>,
        comp: Vec<CompSegment>,
        /// recorded audio kept before the start and after the end (pre-roll and punch, U4), in samples
        handle_before: i64,
        handle_after: i64,
    },
    Midi {
        notes: Arc<Vec<MiniNote>>,
        /// a "pattern": the item repeats every `length` samples until `repeat_until`
        repeats: u32,
    },
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: NodeId,
    pub name: String,
    pub start: i64,
    pub length: i64,
    pub kind: ItemKind,
    /// fade in and out, in samples (the handles in the item's top 7 px, F-CLP-061), and their curve (P-30): 0 linear
    pub fades: (i64, i64),
    pub fade_curve: f32,
    /// the item's gain (the top edge's handle)
    pub gain_db: f64,
    /// played backwards (P-31)
    pub reversed: bool,
}

impl Item {
    pub fn resource(&self) -> Option<ResourceId> {
        match &self.kind {
            ItemKind::Audio { resource, .. } => Some(*resource),
            ItemKind::Midi { .. } => None,
        }
    }
}

/// An automation lane of a track's (or its instrument's) parameter
#[derive(Clone, Debug)]
pub struct Lane {
    pub id: NodeId,
    /// the parameter's label key
    pub param: &'static str,
    pub min: f64,
    pub max: f64,
    /// (sample, value, curve to the next point: 0 = linear, +-1 = bent)
    pub points: Vec<(i64, f64, f32)>,
}

#[derive(Clone, Copy, Debug)]
pub struct TempoPoint {
    pub bar: u32,
    pub bpm: f64,
    /// a ramp to the next point, or a step
    pub ramp: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Range {
    pub start: i64,
    pub end: i64,
    pub on: bool,
}

/// A region of the song (F-REG-011..013): a click makes it the loop; it can be exported alone
#[derive(Clone, Debug)]
pub struct Region {
    pub id: NodeId,
    pub start: i64,
    pub end: i64,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Marker {
    pub id: NodeId,
    pub at: i64,
    pub name: String,
}

/// The song-wide data of the composition
#[derive(Clone, Debug)]
pub struct Song {
    pub tempo: Vec<TempoPoint>,
    pub signature: (u8, u8),
    pub loop_range: Range,
    pub punch: Range,
    pub markers: Vec<Marker>,
    pub regions: Vec<Region>,
    /// the project's global transposition in semitones (P-37)
    pub transpose: i32,
    pub metronome: bool,
    pub count_in_bars: u8,
    pub pre_roll_bars: u8,
    /// the desk's mode (P-17): analog (the rack models) or digital (the rack clean)
    pub desk_analog: bool,
}

impl Default for Song {
    fn default() -> Self {
        Song {
            tempo: vec![
                TempoPoint { bar: 0, bpm: 120.0, ramp: false },
                TempoPoint { bar: 16, bpm: 120.0, ramp: true },
                TempoPoint { bar: 24, bpm: 132.0, ramp: false },
                TempoPoint { bar: 40, bpm: 96.0, ramp: false },
            ],
            signature: (4, 4),
            loop_range: Range { start: 4 * BAR, end: 12 * BAR, on: false },
            punch: Range { start: 6 * BAR, end: 8 * BAR, on: false },
            markers: vec![
                Marker { id: NodeId { kind: b'k', number: 1 }, at: 0, name: "Intro".into() },
                Marker { id: NodeId { kind: b'k', number: 2 }, at: 8 * BAR, name: "Verso".into() },
                Marker { id: NodeId { kind: b'k', number: 3 }, at: 24 * BAR, name: "Refrão".into() },
                Marker { id: NodeId { kind: b'k', number: 4 }, at: 40 * BAR, name: "Ponte".into() },
            ],
            regions: vec![
                Region { id: NodeId { kind: b'g', number: 1 }, start: 8 * BAR, end: 24 * BAR, name: "Verso".into() },
                Region { id: NodeId { kind: b'g', number: 2 }, start: 24 * BAR, end: 40 * BAR, name: "Refrão".into() },
            ],
            transpose: 0,
            metronome: true,
            count_in_bars: 1,
            pre_roll_bars: 1,
            desk_analog: true,
        }
    }
}

/// Deterministic notes for a MIDI item: a bass line, chords or a beat, by `seed`
pub fn fake_notes(seed: u64, length: i64) -> Arc<Vec<MiniNote>> {
    let step = BAR / 8;
    let mut v = Vec::new();
    let mut t = 0;
    let mut k = 0u64;
    while t < length {
        match seed % 3 {
            0 => v.push(MiniNote { start: t, length: step, key: [36, 38, 42, 42][k as usize % 4], velocity: 0.6 + 0.1 * (k % 4) as f32 }),
            1 => {
                for (j, key) in [60u8, 64, 67].iter().enumerate() {
                    if k % 4 == 0 {
                        v.push(MiniNote { start: t, length: step * 4, key: key + (k / 4 % 3) as u8 * 2, velocity: 0.5 + 0.1 * j as f32 });
                    }
                }
            }
            _ => v.push(MiniNote { start: t, length: step - 600, key: [40u8, 40, 43, 45, 47, 45, 43, 38][k as usize % 8], velocity: 0.8 }),
        }
        t += step;
        k += 1;
    }
    Arc::new(v)
}

/// Value of a tempo map at a bar (with ramps), for the tempo lane
pub fn tempo_at(map: &[TempoPoint], bar: f64) -> f64 {
    let mut prev = map[0];
    for p in &map[1..] {
        if (p.bar as f64) > bar {
            if prev.ramp {
                let f = (bar - prev.bar as f64) / (p.bar - prev.bar) as f64;
                return prev.bpm + (p.bpm - prev.bpm) * f;
            }
            return prev.bpm;
        }
        prev = *p;
    }
    prev.bpm
}

// ---------------------------------------------------------------------------------------------------------------
// The arrangement's actions (run through `CorePort::run_action`; each is one transaction, one undo step)

impl crate::Session {
    fn item_place(&self, item: NodeId) -> crate::Result<(usize, usize)> {
        for (t, tr) in self.doc.tracks.iter().enumerate() {
            if let Some(k) = tr.items.iter().position(|i| i.id == item) {
                return Ok((t, k));
            }
        }
        Err(crate::Error::NoSuchNode(item))
    }

    fn new_node(&mut self, kind: u8) -> NodeId {
        self.next_id += 1;
        NodeId { kind, number: 200_000 + self.next_id }
    }

    /// S: split the item at `at` (F-ARR-005); the right part is a new item
    pub fn split_item(&mut self, item: NodeId, at: i64) -> crate::Result<()> {
        let (t, k) = self.item_place(item)?;
        let it = self.doc.tracks[t].items[k].clone();
        if at <= it.start || at >= it.start + it.length {
            return Err(crate::Error::WrongType);
        }
        let id = self.new_node(b'i');
        self.transact("Split", |e| {
            let left = &mut e.doc.tracks[t].items[k];
            left.length = at - it.start;
            left.fades.1 = 0;
            let mut right = it.clone();
            right.id = id;
            right.start = at;
            right.length = it.start + it.length - at;
            right.fades.0 = 0;
            if let ItemKind::Audio { comp, .. } = &mut right.kind {
                let cut = at - it.start;
                for s in comp.iter_mut() {
                    s.start = (s.start - cut).max(0);
                }
            }
            e.doc.tracks[t].items.insert(k + 1, right);
            e.diff.changed.push((item, None));
            e.diff.changed.push((id, None));
            Ok(())
        })
        .map(|_| ())
    }

    /// Ctrl+D: a copy right after the item, on bar lines (F-ARR-007)
    pub fn duplicate_item(&mut self, item: NodeId) -> crate::Result<()> {
        let (t, k) = self.item_place(item)?;
        let mut copy = self.doc.tracks[t].items[k].clone();
        copy.id = self.new_node(b'i');
        let end = copy.start + copy.length;
        copy.start = (end + BAR - 1) / BAR * BAR;
        let id = copy.id;
        self.transact("Duplicate", |e| {
            e.doc.tracks[t].items.push(copy);
            e.diff.changed.push((id, None));
            Ok(())
        })
        .map(|_| ())
    }

    /// An empty audio item with one click (P-29), a bar long
    pub fn empty_item(&mut self, track: usize, at: i64) -> crate::Result<()> {
        let id = self.new_node(b'i');
        let res = self.doc.tracks.get(track).and_then(|t| t.main_resource()).unwrap_or(crate::ResourceId(0));
        self.transact("Empty item", |e| {
            let tr = e.doc.tracks.get_mut(track).ok_or(crate::Error::WrongType)?;
            tr.items.push(Item {
                id,
                name: "Vazio".into(),
                start: at,
                length: BAR,
                kind: ItemKind::Audio { resource: res, takes: Vec::new(), comp: Vec::new(), handle_before: 0, handle_after: 0 },
                fades: (0, 0),
                fade_curve: 0.0,
                gain_db: -120.0,
                reversed: false,
            });
            e.diff.changed.push((id, None));
            Ok(())
        })
        .map(|_| ())
    }

    /// M: a marker at `at` (F-ARR-006)
    pub fn add_marker(&mut self, at: i64) -> crate::Result<()> {
        let id = self.new_node(b'k');
        let n = self.doc.song.markers.len() + 1;
        self.transact("Marker", |e| {
            e.doc.song.markers.push(Marker { id, at, name: format!("{n}") });
            e.doc.song.markers.sort_by_key(|m| m.at);
            e.diff.changed.push((crate::ROOT, None));
            Ok(())
        })
        .map(|_| ())
    }

    /// A click on a region: it becomes the loop, switched on (F-REG-011)
    pub fn loop_from_region(&mut self, region: NodeId) -> crate::Result<()> {
        let r = self.doc.song.regions.iter().find(|r| r.id == region).cloned().ok_or(crate::Error::NoSuchNode(region))?;
        self.transact("Loop from region", |e| {
            e.doc.song.loop_range = Range { start: r.start, end: r.end, on: false };
            e.set(crate::ROOT, crate::Field::LoopOn, crate::Value::Bool(true))
        })
        .map(|_| ())
    }

    /// Shift+R: a region from the loop (F-ARR-006)
    pub fn region_from_loop(&mut self) -> crate::Result<()> {
        let id = self.new_node(b'g');
        let l = self.doc.song.loop_range;
        let n = self.doc.song.regions.len() + 1;
        self.transact("Region", |e| {
            e.doc.song.regions.push(Region { id, start: l.start, end: l.end, name: format!("Região {n}") });
            e.diff.changed.push((crate::ROOT, None));
            Ok(())
        })
        .map(|_| ())
    }
}

/// The arrangement's structural edits (port.rs routes them here): removing an item
pub(crate) fn apply_op(e: &mut crate::Edit, op: &crate::port::EditOp, _created: &[(u32, NodeId)]) -> Option<crate::Result<Option<(u32, NodeId)>>> {
    use crate::port::{EditOp, Ref};
    match op {
        EditOp::Remove { id: Ref::Id(id) } if id.kind == b'i' => {
            let t = e.doc.tracks.iter_mut().find(|t| t.items.iter().any(|i| i.id == *id))?;
            t.items.retain(|i| i.id != *id);
            let tid = t.id;
            e.diff.changed.push((*id, None));
            e.diff.changed.push((tid, None));
            Some(Ok(None))
        }
        _ => None,
    }
}
