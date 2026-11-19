//! Time without loss of precision (arquitetura-nova.md, base item 1; interfaces.md, I-18).
//!
//! Two units and one bridge:
//!
//! - the **musical position** in ticks (`i64`), [`Ppq`] ticks per quarter note (960 by default, per project);
//! - the **audio position** in samples (`i64`). The transport's canonical position is in samples; the tick is
//!   always recomputed from the map ([`TempoMap::tick_at`]), never accumulated block by block;
//! - the [`TempoMap`] is the only conversion between them. It is built from points `{tick, bpm, time signature,
//!   shape}`; the tempo jumps at each point ([`TempoShape::Constant`]) or ramps linearly in time to the next point
//!   ([`TempoShape::Linear`]). [`Bpm`] is an exact decimal rational.
//!
//! # The rounding rules
//!
//! Each segment (from one point to the next) is anchored at an exact integer pair (tick, sample). The first point
//! is at (0, 0); each next point's sample is that point's [`TempoMap::sample_at`] through the previous segment.
//!
//! - **`sample_at(tick)`**: the exact real sample offset of the tick from the segment's anchor, **rounded half to
//!   even**, plus the anchor sample. "Where the tick starts".
//! - **`tick_at(sample)`**: the **last tick whose start sample is at or before `sample`**:
//!   `tick_at(s) = max { t : sample_at(t) <= s }`. It is the floor of the inverse of `sample_at`'s staircase, so
//!   the two conversions can never disagree: an event at tick `t` is played at `sample_at(t)`, and at that
//!   sample the transport already reads tick `t` (a plain `floor(s · ticks_per_sample)` would read `t - 1` there
//!   whenever `sample_at` rounded down).
//!
//! All the arithmetic is integer and exact: `i128` for constant tempo, 256-bit for ramps (whose inverse needs an
//! exact search). No float takes part in any conversion, so results are identical on every machine.
//!
//! # Guarantees
//!
//! For every map and every input whose result fits in `i64`:
//!
//! 1. `sample_at` and `tick_at` are non-decreasing.
//! 2. `sample_at(tick_at(s)) <= s < sample_at(tick_at(s) + 1)` (the Galois relation above).
//! 3. **Tick → sample → tick is exact** — `tick_at(sample_at(t)) == t` — for every tick `t` in a segment where
//!    ticks are no finer than samples (the segment's fastest tempo has `bpm · ppq <= 60 · sample_rate`). With
//!    960 PPQ that is every tempo up to 1000 BPM at 16 kHz or more, so in practice always.
//! 4. **Sample → tick → sample is exact** — `sample_at(tick_at(s)) == s` — for every sample `s` in a segment where
//!    ticks are no coarser than samples (`bpm · ppq >= 60 · sample_rate` at its slowest tempo). Where ticks are
//!    coarser, the samples inside a tick map to that tick, and only the tick's first sample comes back.
//! 5. Nothing accumulates: a position is converted from the map in one step, whatever path led to it. The only
//!    rounding steps are the anchors, one per point, each at most half a sample from the ideal real-valued map
//!    and fixed when the map is built (editing a point never moves the samples of the points before it).
//!
//! # Ramps
//!
//! A [`TempoShape::Linear`] point ramps the tempo linearly in time (BPM against seconds) to the next point's
//! tempo, reaching it exactly at the next point's tick. Linear-in-time is the law whose integral stays rational:
//! the ramp lasts exactly `Y = 120 · sr · L / (ppq · (bpm0 + bpm1))` samples for `L` ticks, and the tick at real
//! sample offset `y` is the quadratic `ppq · (bpm0 · y + (bpm1 − bpm0) · y² / (2Y)) / (60 · sr)`. Both
//! conversions are exact under the same rules as constant segments (the quadratic is evaluated in 256-bit
//! integers; the inverse is the exact integer root, rounded half to even by evaluating the quadratic at the
//! half sample). Guarantees 1–5 hold for ramps too.
//!
//! # Bounds
//!
//! [`Bpm`] 1–1000 with six decimal places, [`Ppq`] up to 100 000, sample rate up to 4 MHz, points within
//! [`MAX_POSITION`]. Inside these bounds no intermediate value overflows (`i128` for constant segments, under 230
//! bits for ramps).

#![forbid(unsafe_code)]

mod bpm;
mod kernel;
mod map;
mod meter;
mod wide;

pub use bpm::Bpm;
pub use map::{
    MAX_POSITION, MAX_SAMPLE_RATE, Segment, SegmentShape, Span, Spans, TempoMap, TempoPoint, TempoShape, relocate,
};
pub use meter::{BarBeatTick, MusicalPosition, Ppq, TimeSignature};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_crosses_threads() {
        fn send_sync<T: Send + Sync + Clone>() {}
        send_sync::<TempoMap>();
        send_sync::<Segment>();
        send_sync::<Bpm>();
        send_sync::<TimeSignature>();
        send_sync::<Ppq>();
    }
}
