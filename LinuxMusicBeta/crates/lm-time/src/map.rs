//! The tempo map: points, segments, the two conversions, block splitting, bars and beats

use std::ops::Range;
use std::sync::Arc;

use lm_base::{Error, Result};

use crate::Bpm;
use crate::kernel::{self, Kernel};
use crate::meter::{BarBeatTick, MusicalPosition, Ppq, TimeSignature};

/// The largest absolute tick or sample a tempo point may sit at: 2⁶² (about 3 million years at 48 kHz). The
/// conversions accept any `i64`; the guarantees of the crate docs hold for every input whose result is inside
/// `i64` (results outside it saturate)
pub const MAX_POSITION: i64 = 1 << 62;

/// The fastest sample rate accepted (it bounds the integer sizes of the conversions)
pub const MAX_SAMPLE_RATE: u32 = 4_000_000;

/// How the tempo goes from one point to the next
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TempoShape {
    /// The tempo holds until the next point, then jumps
    #[default]
    Constant,
    /// The tempo changes linearly in time (BPM against seconds) from this point's tempo to the next point's,
    /// arriving exactly at the next point's tick. The last point cannot be a ramp
    Linear,
}

/// A point of the map, as the document stores it (`{tick, bpm, timeSig}`, plus the shape)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TempoPoint {
    pub tick: i64,
    pub bpm: Bpm,
    pub time_signature: TimeSignature,
    pub shape: TempoShape,
}

impl TempoPoint {
    /// A point whose tempo holds until the next one
    pub fn new(tick: i64, bpm: Bpm, time_signature: TimeSignature) -> Self {
        Self { tick, bpm, time_signature, shape: TempoShape::Constant }
    }

    /// The same point, ramping linearly to the next point's tempo
    #[must_use]
    pub fn linear(mut self) -> Self {
        self.shape = TempoShape::Linear;
        self
    }
}

/// What the tempo does inside a segment
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SegmentShape {
    Constant(Bpm),
    /// Linear in time from `from` (at the segment's start) to `to` (at its end)
    Linear {
        from: Bpm,
        to: Bpm,
    },
}

/// A stretch of the map with one tempo law, from one point to the next.
///
/// `start_*` is inclusive and `end_*` exclusive. The first segment starts at `i64::MIN` (positions before tick 0
/// continue the first tempo backwards) and the last one ends at `i64::MAX`. A segment can be empty in samples
/// (when several ticks share a sample, see the crate docs); [`TempoMap::segments_in`] skips those
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    start_tick: i64,
    end_tick: i64,
    start_sample: i64,
    end_sample: i64,
    anchor_tick: i64,
    anchor_sample: i64,
    time_signature: TimeSignature,
    /// The tempo at the segment's start
    bpm: Bpm,
    kernel: Kernel,
}

impl Segment {
    pub fn start_tick(&self) -> i64 {
        self.start_tick
    }

    pub fn end_tick(&self) -> i64 {
        self.end_tick
    }

    pub fn start_sample(&self) -> i64 {
        self.start_sample
    }

    pub fn end_sample(&self) -> i64 {
        self.end_sample
    }

    pub fn time_signature(&self) -> TimeSignature {
        self.time_signature
    }

    pub fn shape(&self) -> SegmentShape {
        match &self.kernel {
            Kernel::Constant(_) => SegmentShape::Constant(self.constant_bpm()),
            Kernel::Ramp(r) => SegmentShape::Linear { from: r.from(), to: r.to() },
        }
    }

    pub fn is_ramp(&self) -> bool {
        matches!(self.kernel, Kernel::Ramp(_))
    }

    /// The tempo at a sample of this segment, for display and plugins' host-tempo fields
    pub fn bpm_at_sample(&self, sample: i64) -> f64 {
        match &self.kernel {
            Kernel::Constant(_) => self.constant_bpm().to_f64(),
            Kernel::Ramp(r) => r.bpm_at((i128::from(sample) - i128::from(self.anchor_sample)) as f64),
        }
    }

    fn constant_bpm(&self) -> Bpm {
        self.bpm
    }

    fn ticks_are_coarser(&self, sample_rate: u32, ppq: Ppq) -> bool {
        match &self.kernel {
            Kernel::Constant(k) => k.ticks_are_coarser(),
            Kernel::Ramp(r) => r.ticks_are_coarser(sample_rate, ppq.get()),
        }
    }

    fn ticks_are_finer(&self, sample_rate: u32, ppq: Ppq) -> bool {
        match &self.kernel {
            Kernel::Constant(k) => k.ticks_are_finer(),
            Kernel::Ramp(r) => r.ticks_are_finer(sample_rate, ppq.get()),
        }
    }
}

/// A piece of a block that lies inside one segment: `start_sample..end_sample`
#[derive(Clone, Copy, Debug)]
pub struct Span<'a> {
    pub start_sample: i64,
    pub end_sample: i64,
    pub segment: &'a Segment,
}

#[derive(Debug)]
struct Meter {
    start_tick: i64,
    /// Bar index (0-based: bar 1 is index 0) of the bar that starts at `start_tick`
    start_bar: i64,
    signature: TimeSignature,
    beat_ticks: i64,
    bar_ticks: i64,
}

#[derive(Debug)]
struct Inner {
    sample_rate: u32,
    ppq: Ppq,
    points: Box<[TempoPoint]>,
    segments: Box<[Segment]>,
    meters: Box<[Meter]>,
}

/// The tempo map: the only bridge between ticks (musical position) and samples (audio position).
///
/// Built on the main thread from the document's points, the project's PPQ and the sample rate, then moved or
/// cloned into the render program. Immutable; a clone is an `Arc` clone. The conversions, [`TempoMap::segments_in`]
/// and the bar functions never allocate, so they can run on the audio thread (dropping the last clone frees
/// memory: the render program sends its old map back to the main thread to be dropped).
#[derive(Clone, Debug)]
pub struct TempoMap {
    inner: Arc<Inner>,
}

impl PartialEq for TempoMap {
    /// Two maps are equal when they were built from the same rate, PPQ and points
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
            || (self.inner.sample_rate == other.inner.sample_rate
                && self.inner.ppq == other.inner.ppq
                && self.inner.points == other.inner.points)
    }
}

impl Eq for TempoMap {}

impl TempoMap {
    /// Builds the map.
    ///
    /// Rules (error keys): the sample rate is `1..=4_000_000` (`time.sample_rate_invalid`); there is at least one
    /// point (`time.map_empty`); the first point is at tick 0 (`time.map_first_point_not_zero`); ticks strictly
    /// increase (`time.map_points_not_increasing`); no point beyond [`MAX_POSITION`] in ticks or samples
    /// (`time.map_too_long`); every time signature's beat is a whole number of ticks at this PPQ
    /// (`time.signature_not_on_tick_grid`); the last point is not a ramp (`time.ramp_without_end`).
    ///
    /// Each point's sample is computed through the previous segment and rounded half to even; that integer pair
    /// (tick, sample) anchors the next segment, so the map is exactly self-consistent (see the crate docs).
    pub fn new(sample_rate: u32, ppq: Ppq, points: &[TempoPoint]) -> Result<Self> {
        if sample_rate == 0 || sample_rate > MAX_SAMPLE_RATE {
            return Err(Error::new("time.sample_rate_invalid")
                .arg("sample_rate", i64::from(sample_rate))
                .arg("max", i64::from(MAX_SAMPLE_RATE)));
        }
        let Some(first) = points.first() else {
            return Err(Error::new("time.map_empty"));
        };
        if first.tick != 0 {
            return Err(Error::new("time.map_first_point_not_zero").arg("tick", first.tick));
        }
        for (i, pair) in points.windows(2).enumerate() {
            if pair[1].tick <= pair[0].tick {
                return Err(Error::new("time.map_points_not_increasing")
                    .arg("index", (i + 1) as i64)
                    .arg("tick", pair[1].tick));
            }
        }
        for (i, p) in points.iter().enumerate() {
            if p.tick > MAX_POSITION {
                return Err(Error::new("time.map_too_long").arg("index", i as i64).arg("tick", p.tick));
            }
            if p.time_signature.beat_ticks(ppq).is_none() {
                return Err(Error::new("time.signature_not_on_tick_grid")
                    .arg("index", i as i64)
                    .arg("numerator", i64::from(p.time_signature.numerator()))
                    .arg("denominator", i64::from(p.time_signature.denominator()))
                    .arg("ppq", i64::from(ppq.get())));
            }
        }
        if let Some(last) = points.last()
            && last.shape == TempoShape::Linear
        {
            return Err(Error::new("time.ramp_without_end").arg("tick", last.tick));
        }

        let mut segments = Vec::with_capacity(points.len() + 1);
        let constant = |bpm| Kernel::Constant(kernel::Constant::new(sample_rate, ppq.get(), bpm));
        let mut anchor_sample = 0_i64;
        for (i, p) in points.iter().enumerate() {
            let next = points.get(i + 1);
            let kernel = match (p.shape, next) {
                (TempoShape::Linear, Some(n)) if n.bpm != p.bpm => {
                    Kernel::Ramp(kernel::Ramp::new(sample_rate, ppq.get(), p.bpm, n.bpm, n.tick - p.tick))
                }
                _ => constant(p.bpm),
            };
            if i == 0 && matches!(kernel, Kernel::Ramp(_)) {
                // Before tick 0 the first tempo continues, constant, anchored at (0, 0)
                segments.push(Segment {
                    start_tick: i64::MIN,
                    end_tick: 0,
                    start_sample: i64::MIN,
                    end_sample: 0,
                    anchor_tick: 0,
                    anchor_sample: 0,
                    time_signature: p.time_signature,
                    bpm: p.bpm,
                    kernel: constant(p.bpm),
                });
            }
            let (end_tick, end_sample) = match next {
                Some(n) => {
                    let end = i128::from(anchor_sample) + kernel.sample_offset(i128::from(n.tick - p.tick));
                    if end > i128::from(MAX_POSITION) {
                        return Err(Error::new("time.map_too_long").arg("index", (i + 1) as i64).arg("tick", n.tick));
                    }
                    (n.tick, end as i64)
                }
                None => (i64::MAX, i64::MAX),
            };
            let open_start = segments.is_empty();
            segments.push(Segment {
                start_tick: if open_start { i64::MIN } else { p.tick },
                end_tick,
                start_sample: if open_start { i64::MIN } else { anchor_sample },
                end_sample,
                anchor_tick: p.tick,
                anchor_sample,
                time_signature: p.time_signature,
                bpm: p.bpm,
                kernel,
            });
            anchor_sample = end_sample;
        }

        let mut meters: Vec<Meter> = Vec::new();
        for p in points {
            if meters.last().is_some_and(|m| m.signature == p.time_signature) {
                continue;
            }
            let beat_ticks = p.time_signature.beat_ticks(ppq).unwrap_or(1);
            let bar_ticks = beat_ticks * i64::from(p.time_signature.numerator());
            let start_bar = match meters.last() {
                // A new signature starts a new bar; a bar it cuts short stays a (shorter) bar
                Some(m) => m.start_bar + ceil_div(p.tick - m.start_tick, m.bar_ticks),
                None => 0,
            };
            meters.push(Meter { start_tick: p.tick, start_bar, signature: p.time_signature, beat_ticks, bar_ticks });
        }

        Ok(Self {
            inner: Arc::new(Inner {
                sample_rate,
                ppq,
                points: points.into(),
                segments: segments.into(),
                meters: meters.into(),
            }),
        })
    }

    /// A map with one tempo and one signature
    pub fn constant(sample_rate: u32, ppq: Ppq, bpm: Bpm, time_signature: TimeSignature) -> Result<Self> {
        Self::new(sample_rate, ppq, &[TempoPoint::new(0, bpm, time_signature)])
    }

    pub fn sample_rate(&self) -> u32 {
        self.inner.sample_rate
    }

    pub fn ppq(&self) -> Ppq {
        self.inner.ppq
    }

    /// The points the map was built from
    pub fn points(&self) -> &[TempoPoint] {
        &self.inner.points
    }

    /// All the segments, in order. When the first point is a ramp, a constant lead-in segment covers the
    /// positions before 0
    pub fn segments(&self) -> &[Segment] {
        &self.inner.segments
    }

    /// The segment containing the sample (the last one starting at or before it)
    pub fn segment_at_sample(&self, sample: i64) -> &Segment {
        let segs = &self.inner.segments;
        &segs[segs.partition_point(|s| s.start_sample <= sample).saturating_sub(1)]
    }

    /// The segment containing the tick
    pub fn segment_at_tick(&self, tick: i64) -> &Segment {
        let segs = &self.inner.segments;
        &segs[segs.partition_point(|s| s.start_tick <= tick).saturating_sub(1)]
    }

    /// The tick reached at `sample`: the last tick whose start sample is `<= sample`
    /// (`max { t : sample_at(t) <= sample }`). Exact integer arithmetic; see the crate docs for the guarantees
    pub fn tick_at(&self, sample: i64) -> i64 {
        let seg = self.segment_at_sample(sample);
        let r = i128::from(sample) - i128::from(seg.anchor_sample);
        saturate(i128::from(seg.anchor_tick) + seg.kernel.tick_offset(r))
    }

    /// The sample where `tick` starts: the exact real position rounded half to even (relative to the segment's
    /// integer anchor). Exact integer arithmetic
    pub fn sample_at(&self, tick: i64) -> i64 {
        let seg = self.segment_at_tick(tick);
        let dt = i128::from(tick) - i128::from(seg.anchor_tick);
        saturate(i128::from(seg.anchor_sample) + seg.kernel.sample_offset(dt))
    }

    /// The ticks that start inside the samples `from..to`: `t` with `from <= sample_at(t) < to`. This is the range
    /// of musical events a block `from..to` must play
    pub fn ticks_in(&self, from: i64, to: i64) -> Range<i64> {
        if to <= from {
            let t = self.tick_at(from.saturating_sub(1)).saturating_add(1);
            return t..t;
        }
        let first = if from == i64::MIN { i64::MIN } else { self.tick_at(from - 1).saturating_add(1) };
        first..self.tick_at(to - 1).saturating_add(1)
    }

    /// The pieces of the block `from..to` (samples), cut at every segment boundary inside it, in order. They tile
    /// the block exactly; empty segments are skipped. Empty when `to <= from`. Does not allocate
    pub fn segments_in(&self, from: i64, to: i64) -> Spans<'_> {
        let segs = &self.inner.segments;
        let index = segs.partition_point(|s| s.start_sample <= from).saturating_sub(1);
        Spans { segments: segs, index, position: from, to }
    }

    /// The tempo at a sample, for display and plugins' host-tempo fields (never for a conversion)
    pub fn bpm_at_sample(&self, sample: i64) -> f64 {
        self.segment_at_sample(sample).bpm_at_sample(sample)
    }

    /// The time signature in force at a tick
    pub fn time_signature_at(&self, tick: i64) -> TimeSignature {
        self.meter_at_tick(tick).signature
    }

    /// Whether guarantee 3 of the crate docs holds in the segment of this tick (tick → sample → tick is exact)
    pub fn ticks_coarser_at_tick(&self, tick: i64) -> bool {
        self.segment_at_tick(tick).ticks_are_coarser(self.sample_rate(), self.ppq())
    }

    /// Whether guarantee 4 of the crate docs holds in the segment of this sample (sample → tick → sample is exact)
    pub fn ticks_finer_at_sample(&self, sample: i64) -> bool {
        self.segment_at_sample(sample).ticks_are_finer(self.sample_rate(), self.ppq())
    }

    fn meter_at_tick(&self, tick: i64) -> &Meter {
        let m = &self.inner.meters;
        &m[m.partition_point(|m| m.start_tick <= tick).saturating_sub(1)]
    }

    /// The bar:beat:tick of a tick. A time-signature change starts a new bar; when it falls inside a bar, that
    /// bar ends early (it is shorter). Ticks before 0 continue the first signature backwards (bar 0, -1, ...)
    pub fn bar_beat_tick(&self, tick: i64) -> BarBeatTick {
        let m = self.meter_at_tick(tick);
        let offset = i128::from(tick) - i128::from(m.start_tick);
        let bars = offset.div_euclid(i128::from(m.bar_ticks));
        let within = offset.rem_euclid(i128::from(m.bar_ticks)) as i64;
        BarBeatTick {
            bar: saturate(i128::from(m.start_bar) + bars + 1),
            beat: (within / m.beat_ticks) as u32 + 1,
            tick: (within % m.beat_ticks) as u32,
        }
    }

    /// The tick where a bar (numbered from 1) starts. Key `time.position_out_of_range` if it is not an `i64`
    pub fn bar_start(&self, bar: i64) -> Result<i64> {
        let out = || Error::new("time.position_out_of_range").arg("bar", bar);
        let index = bar.checked_sub(1).ok_or_else(out)?;
        let meters = &self.inner.meters;
        let m = &meters[meters.partition_point(|m| m.start_bar <= index).saturating_sub(1)];
        let tick = i128::from(m.start_tick) + (i128::from(index) - i128::from(m.start_bar)) * i128::from(m.bar_ticks);
        i64::try_from(tick).map_err(|_| out())
    }

    /// The tick of a bar:beat:tick. Keys: `time.position_invalid` (beat outside `1..=numerator` or tick outside
    /// the beat), `time.position_outside_bar` (past the end of a bar cut short by a signature change),
    /// `time.position_out_of_range`
    pub fn tick_of(&self, position: BarBeatTick) -> Result<i64> {
        let start = self.bar_start(position.bar)?;
        let m = self.meter_at_tick(start);
        if position.beat == 0
            || position.beat > u32::from(m.signature.numerator())
            || i64::from(position.tick) >= m.beat_ticks
        {
            return Err(Error::new("time.position_invalid")
                .arg("bar", position.bar)
                .arg("beat", i64::from(position.beat))
                .arg("tick", i64::from(position.tick)));
        }
        let tick =
            i128::from(start) + i128::from(position.beat - 1) * i128::from(m.beat_ticks) + i128::from(position.tick);
        let tick =
            i64::try_from(tick).map_err(|_| Error::new("time.position_out_of_range").arg("bar", position.bar))?;
        let next_bar = position.bar.checked_add(1).map(|b| self.bar_start(b));
        if let Some(Ok(end)) = next_bar
            && tick >= end
        {
            return Err(Error::new("time.position_outside_bar")
                .arg("bar", position.bar)
                .arg("beat", i64::from(position.beat))
                .arg("tick", i64::from(position.tick)));
        }
        Ok(tick)
    }

    /// Everything the interface shows for a sample position
    pub fn musical_position(&self, sample: i64) -> MusicalPosition {
        let tick = self.tick_at(sample);
        MusicalPosition {
            sample,
            tick,
            bar_beat_tick: self.bar_beat_tick(tick),
            time_signature: self.time_signature_at(tick),
            bpm: self.bpm_at_sample(sample),
        }
    }
}

/// The iterator of [`TempoMap::segments_in`]
#[derive(Clone, Debug)]
pub struct Spans<'a> {
    segments: &'a [Segment],
    index: usize,
    position: i64,
    to: i64,
}

impl<'a> Iterator for Spans<'a> {
    type Item = Span<'a>;

    fn next(&mut self) -> Option<Span<'a>> {
        if self.position >= self.to {
            return None;
        }
        // Skip segments that end at or before the position (the empty ones, and the one just finished)
        while self.segments[self.index].end_sample <= self.position {
            self.index += 1;
        }
        let segment = &self.segments[self.index];
        let end = segment.end_sample.min(self.to);
        let span = Span { start_sample: self.position, end_sample: end, segment };
        self.position = end;
        Some(span)
    }
}

impl std::iter::FusedIterator for Spans<'_> {}

/// H9: where to continue playing after the tempo map changed under the playhead, so the musical position is kept.
///
/// `sample` is the current position in `old` (the next sample to be played). The ticks that had started before
/// it, `t <= τ` with `τ = old.tick_at(sample - 1)`, are exactly the ones that have started before the returned
/// position in `new`, so no event is played twice or skipped; inside tick `τ` the fraction already played is kept
/// (rounded half to even, to the samples of `new`). Returns `sample` unchanged when the maps are equal. The one
/// exception: if `new` starts ticks `τ` and `τ + 1` on the same sample (more ticks than samples per second), the
/// result is one sample after it.
///
/// The `Relocate { tick }` of I-7.2 carries `τ`; the renderer calls this with its old and new maps at install time.
pub fn relocate(old: &TempoMap, new: &TempoMap, sample: i64) -> i64 {
    if old == new {
        return sample;
    }
    let tau = old.tick_at(sample.saturating_sub(1));
    let a = i128::from(old.sample_at(tau));
    let b = i128::from(old.sample_at(tau.saturating_add(1)));
    let a2 = i128::from(new.sample_at(tau));
    let b2 = i128::from(new.sample_at(tau.saturating_add(1)));
    let s = i128::from(sample);
    if b2 <= a2 || b <= a {
        return saturate(a2 + 1);
    }
    // a < s <= b in the old map; keep the fraction (s - a) / (b - a) inside (a2, b2]
    let offset = kernel::round_half_even_div((s - a) * (b2 - a2), b - a).clamp(1, b2 - a2);
    saturate(a2 + offset)
}

fn saturate(v: i128) -> i64 {
    v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

fn ceil_div(n: i64, d: i64) -> i64 {
    -((-n).div_euclid(d))
}
