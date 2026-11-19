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
