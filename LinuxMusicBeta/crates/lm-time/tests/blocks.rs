//! 24 hours of simulated audio blocks: the transport keeps its position in samples, recomputes the tick from the map
//! every block, and cuts every block at the tempo changes inside it. Nothing may drift
mod common;

use common::{MapSpec, Reference, Rng, random_bpm, sig};
use lm_time::{Ppq, TempoMap, TempoPoint, TempoShape};

const DAY_SECONDS: i64 = 24 * 60 * 60;

/// A map with tempo points spread over more than a day of audio
fn day_map(rng: &mut Rng, sample_rate: u32, ramps: bool) -> MapSpec {
    let day = DAY_SECONDS * i64::from(sample_rate);
    let mut points = vec![TempoPoint::new(0, random_bpm(rng), sig(4, 4))];
    let mut tick = 0;
    loop {
        let spec = MapSpec { sample_rate, ppq: Ppq::DEFAULT, points: points.clone() };
        if spec.build().sample_at(tick) > day + i64::from(sample_rate) {
            return spec;
        }
        // Mostly long stretches, sometimes changes a few ticks apart (several inside one block)
        tick += if rng.chance(20) { rng.range(1, 50) } else { rng.range(1, 400_000) };
        if ramps && rng.chance(50) {
            points.last_mut().unwrap().shape = TempoShape::Linear;
        }
        let s = if rng.chance(20) { sig(rng.range(1, 12) as u8, rng.pick(&[2, 4, 8, 16])) } else { sig(4, 4) };
        points.push(TempoPoint::new(tick, random_bpm(rng), s));
    }
}

/// Runs a day of blocks of random sizes; `reference` checks each recomputed tick against a direct computation
fn run_day(seed: u64, map: &TempoMap, reference: Option<&Reference>) -> u64 {
    let mut rng = Rng::new(seed);
    let day = DAY_SECONDS * i64::from(map.sample_rate());
    let segments = map.segments();
    let mut next_boundary = 0; // index into `segments` of the next segment start to expect as a cut
    let mut position: i64 = 0; // the canonical transport position, in samples
    let mut started: i64 = 0; // ticks started so far, counted block by block from `ticks_in`
    let ticks_before_start = map.tick_at(-1);
    let mut blocks = 0_u64;
    while position < day {
        let size = match rng.range(0, 9) {
            0 => rng.range(1, 16),
            1..=7 => rng.range(16, 4096),
            _ => rng.range(4096, 16_384),
        };
        let (from, to) = (position, position + size);

        // The tick is recomputed from the position, never accumulated
        let tick = map.tick_at(from);
        match reference {
            Some(r) => assert_eq!(tick, r.tick_at(from), "seed {seed}: block at {from}"),
            None => assert!(map.sample_at(tick) <= from && from < map.sample_at(tick + 1), "seed {seed} at {from}"),
        }

        // The block is cut at every segment start inside it, and only there; the pieces tile it
        while next_boundary < segments.len() && segments[next_boundary].start_sample() <= from {
            next_boundary += 1;
        }
        let mut cursor = from;
        for span in map.segments_in(from, to) {
            assert_eq!(span.start_sample, cursor, "seed {seed}: gap or overlap in the block at {from}");
            assert!(span.end_sample > span.start_sample);
            assert!(span.segment.start_sample() <= span.start_sample && span.end_sample <= span.segment.end_sample());
            if span.start_sample > from {
                // A cut: it must be the next non-empty segment's start
                while segments[next_boundary].end_sample() <= segments[next_boundary].start_sample() {
                    next_boundary += 1;
                }
                assert_eq!(span.start_sample, segments[next_boundary].start_sample(), "seed {seed}: cut at {from}");
                next_boundary += 1;
            }
            cursor = span.end_sample;
        }
        assert_eq!(cursor, to, "seed {seed}: the pieces do not reach the end of the block at {from}");
        while next_boundary < segments.len()
            && segments[next_boundary].end_sample() <= segments[next_boundary].start_sample()
        {
            next_boundary += 1;
        }
        assert!(
            next_boundary >= segments.len() || segments[next_boundary].start_sample() >= to,
            "seed {seed}: a tempo change inside the block at {from} was not cut"
        );

        // Counting the ticks block by block gives the same total as the direct computation
        let ticks = map.ticks_in(from, to);
        started += ticks.end - ticks.start;
        assert_eq!(started, map.tick_at(to - 1) - ticks_before_start, "seed {seed}: tick count drifted at {to}");

        position = to;
        blocks += 1;
    }
    // After a day, the position is exact and the tick is the direct one
    assert!(position >= day);
    if let Some(r) = reference {
        assert_eq!(map.tick_at(position), r.tick_at(position));
    }
    blocks
}

#[test]
fn a_day_of_blocks_with_tempo_jumps_has_no_accumulated_error() {
    let mut rng = Rng::new(0xDA7);
    let spec = day_map(&mut rng, 48_000, false);
    let map = spec.build();
    let reference = Reference::new(&spec);
    let blocks = run_day(1, &map, Some(&reference));
    assert!(blocks > 500_000, "{blocks} blocks");
}

#[test]
fn a_day_of_blocks_with_ramps_has_no_accumulated_error() {
    let mut rng = Rng::new(0xDA8);
    let spec = day_map(&mut rng, 44_100, true);
    let map = spec.build();
    assert!(
        map.segments().iter().filter(|s| s.is_ramp()).count() > 20,
        "{} ramps",
        map.segments().iter().filter(|s| s.is_ramp()).count()
    );
    let blocks = run_day(2, &map, None);
    assert!(blocks > 500_000, "{blocks} blocks");
}

#[test]
fn a_float_accumulator_would_drift_where_the_map_does_not() {
    // The reason for the rule: 119.88 BPM, 48 kHz, blocks of 64. Adding the tick increment per block in f64
    // drifts; recomputing from the sample position does not
    let map = TempoMap::constant(48_000, Ppq::DEFAULT, common::bpm("119.88"), sig(4, 4)).unwrap();
    let per_sample = 119.88 * 960.0 / 60.0 / 48_000.0;
    let mut float_ticks = 0.0_f32;
    let mut position = 0_i64;
    while position < DAY_SECONDS * 48_000 {
        float_ticks += (per_sample * 64.0) as f32;
        position += 64;
    }
    let exact = map.tick_at(position);
    assert!(map.sample_at(exact) <= position && position < map.sample_at(exact + 1));
    let ideal = position as f64 * per_sample;
    assert!((exact as f64 - ideal).abs() <= 1.0);
    assert!((f64::from(float_ticks) - exact as f64).abs() > 1000.0);
}

#[test]
fn spans_cover_several_changes_in_one_block_and_skip_empty_segments() {
    let map = TempoMap::new(
        48_000,
        Ppq::DEFAULT,
        &[
            TempoPoint::new(0, common::bpm("120"), sig(4, 4)),
            TempoPoint::new(4, common::bpm("60"), sig(4, 4)),
            TempoPoint::new(6, common::bpm("90"), sig(4, 4)).linear(),
            TempoPoint::new(10, common::bpm("180"), sig(4, 4)),
        ],
    )
    .unwrap();
    // 120: 25 samples per tick → tick 4 at 100; 60: 50 per tick → tick 6 at 200; ramp to tick 10
    let cuts: Vec<(i64, i64)> = map.segments_in(-10, 1000).map(|s| (s.start_sample, s.end_sample)).collect();
    let ramp_end = map.sample_at(10);
    assert_eq!(cuts, vec![(-10, 100), (100, 200), (200, ramp_end), (ramp_end, 1000)]);
    assert_eq!(map.segments_in(5, 5).count(), 0);
    assert_eq!(map.segments_in(5, 4).count(), 0);
    assert_eq!(map.segments_in(150, 151).count(), 1);
    assert_eq!(map.segments_in(i64::MIN, i64::MIN + 3).count(), 1);
    assert_eq!(map.segments_in(i64::MAX - 3, i64::MAX).count(), 1);

    // More ticks than samples: 1 kHz, 1000 BPM, 960 PPQ → 16 ticks per sample. Points at ticks 1 and 2 start on
    // sample 0, so the segments between them are empty in samples and never come out
    let dense = TempoMap::new(
        1_000,
        Ppq::DEFAULT,
        &[
            TempoPoint::new(0, common::bpm("1000"), sig(4, 4)),
            TempoPoint::new(1, common::bpm("1000"), sig(4, 4)),
            TempoPoint::new(2, common::bpm("500"), sig(4, 4)),
        ],
    )
    .unwrap();
    let spans: Vec<(i64, i64, i64)> =
        dense.segments_in(-5, 5).map(|s| (s.start_sample, s.end_sample, s.segment.start_tick())).collect();
    assert_eq!(spans, vec![(-5, 0, i64::MIN), (0, 5, 2)]);
    // At 500 BPM a sample holds 8 ticks: tick 2 + dt starts at round_half_even(dt / 8), so sample 1 starts
    // dt = 5..=11 (4/8 = ½ rounds to the even 0)
    assert_eq!(dense.tick_at(1), 13);
    assert_eq!(dense.ticks_in(1, 2), 7..14);
}

#[test]
fn ticks_in_gives_the_events_of_a_block() {
    let map = TempoMap::constant(48_000, Ppq::DEFAULT, common::bpm("120"), sig(4, 4)).unwrap();
    // Ticks start every 25 samples: block 0..25 holds tick 0 only, 1..26 holds tick 1 only
    assert_eq!(map.ticks_in(0, 25), 0..1);
    assert_eq!(map.ticks_in(1, 26), 1..2);
    assert_eq!(map.ticks_in(1, 25), 1..1);
    assert_eq!(map.ticks_in(0, 1000), 0..40);
    assert_eq!(map.ticks_in(-50, 0), -2..0);
    assert!(map.ticks_in(10, 10).is_empty());
    assert!(map.ticks_in(i64::MIN, i64::MIN).is_empty());
}
