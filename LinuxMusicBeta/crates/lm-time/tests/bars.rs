//! Bar:beat:tick with time-signature changes, and the musical position for display
mod common;

use common::{Rng, bpm, random_map, sig};
use lm_time::{BarBeatTick, Ppq, TempoMap, TempoPoint};

fn bbt(bar: i64, beat: u32, tick: u32) -> BarBeatTick {
    BarBeatTick { bar, beat, tick }
}

/// 4/4 for two bars, 7/8 for two bars, a tempo-only point inside a 7/8 bar, 3/4 starting mid-bar (it cuts a 7/8
/// bar short), then 6/8
fn changing_map() -> TempoMap {
    TempoMap::new(
        48_000,
        Ppq::DEFAULT,
        &[
            TempoPoint::new(0, bpm("120"), sig(4, 4)),
            TempoPoint::new(7680, bpm("120"), sig(7, 8)), // bar 3; 7/8 bar = 3360 ticks
            TempoPoint::new(7680 + 3360 + 480, bpm("90"), sig(7, 8)), // tempo only, inside bar 4
            TempoPoint::new(7680 + 2 * 3360 + 1000, bpm("90"), sig(3, 4)), // 1000 ticks into bar 5: cuts it short
            TempoPoint::new(7680 + 2 * 3360 + 1000 + 2 * 2880, bpm("100"), sig(6, 8)), // two 3/4 bars later
        ],
    )
    .unwrap()
}

#[test]
fn bar_beat_tick_across_signature_changes() {
    let map = changing_map();
    assert_eq!(map.bar_beat_tick(0), bbt(1, 1, 0));
    assert_eq!(map.bar_beat_tick(959), bbt(1, 1, 959));
    assert_eq!(map.bar_beat_tick(960), bbt(1, 2, 0));
    assert_eq!(map.bar_beat_tick(3839), bbt(1, 4, 959));
    assert_eq!(map.bar_beat_tick(3840), bbt(2, 1, 0));
    // 7/8: beats are eighths (480 ticks)
    assert_eq!(map.bar_beat_tick(7680), bbt(3, 1, 0));
    assert_eq!(map.bar_beat_tick(7680 + 480 * 6 + 479), bbt(3, 7, 479));
    assert_eq!(map.bar_beat_tick(7680 + 3360), bbt(4, 1, 0));
    // The tempo-only point does not restart the bar
    assert_eq!(map.bar_beat_tick(7680 + 3360 + 480), bbt(4, 2, 0));
    // Bar 5 is cut short after 1000 ticks (beat 3, tick 40) by the 3/4
    let bar5 = 7680 + 2 * 3360;
    assert_eq!(map.bar_beat_tick(bar5 + 999), bbt(5, 3, 39));
    assert_eq!(map.bar_beat_tick(bar5 + 1000), bbt(6, 1, 0));
    assert_eq!(map.bar_beat_tick(bar5 + 1000 + 2880), bbt(7, 1, 0));
    // 6/8 after two bars of 3/4
    let six_eight = bar5 + 1000 + 2 * 2880;
    assert_eq!(map.bar_beat_tick(six_eight), bbt(8, 1, 0));
    assert_eq!(map.bar_beat_tick(six_eight + 2880 * 10 + 481), bbt(18, 2, 1));
    // Before 0: the first signature continues backwards
    assert_eq!(map.bar_beat_tick(-1), bbt(0, 4, 959));
    assert_eq!(map.bar_beat_tick(-3840), bbt(0, 1, 0));
    assert_eq!(map.bar_beat_tick(-3841), bbt(-1, 4, 959));
    assert_eq!(map.time_signature_at(-5), sig(4, 4));
    assert_eq!(map.time_signature_at(bar5 + 999), sig(7, 8));
    assert_eq!(map.time_signature_at(bar5 + 1000), sig(3, 4));
}

#[test]
fn bar_starts_and_the_inverse() {
    let map = changing_map();
    let bar5 = 7680 + 2 * 3360;
    assert_eq!(map.bar_start(1).unwrap(), 0);
    assert_eq!(map.bar_start(3).unwrap(), 7680);
    assert_eq!(map.bar_start(5).unwrap(), bar5);
    assert_eq!(map.bar_start(6).unwrap(), bar5 + 1000);
    assert_eq!(map.bar_start(8).unwrap(), bar5 + 1000 + 5760);
    assert_eq!(map.bar_start(0).unwrap(), -3840);
    assert_eq!(map.bar_start(-9).unwrap(), -10 * 3840);
    assert_eq!(map.bar_start(i64::MIN).unwrap_err().key(), "time.position_out_of_range");
    assert_eq!(map.bar_start(i64::MAX).unwrap_err().key(), "time.position_out_of_range");

    assert_eq!(map.tick_of(bbt(5, 3, 39)).unwrap(), bar5 + 999);
    // Past the end of the shortened bar
    assert_eq!(map.tick_of(bbt(5, 3, 40)).unwrap_err().key(), "time.position_outside_bar");
    assert_eq!(map.tick_of(bbt(5, 7, 0)).unwrap_err().key(), "time.position_outside_bar");
    assert_eq!(map.tick_of(bbt(1, 0, 0)).unwrap_err().key(), "time.position_invalid");
    assert_eq!(map.tick_of(bbt(1, 5, 0)).unwrap_err().key(), "time.position_invalid");
    assert_eq!(map.tick_of(bbt(3, 8, 0)).unwrap_err().key(), "time.position_invalid");
    assert_eq!(map.tick_of(bbt(3, 1, 480)).unwrap_err().key(), "time.position_invalid");

    // Every tick round-trips through bar:beat:tick
    for t in -10_000..(bar5 + 30_000) {
        assert_eq!(map.tick_of(map.bar_beat_tick(t)).unwrap(), t, "tick {t}");
    }
}

#[test]
fn bar_beat_round_trip_on_random_maps() {
    for seed in 0..300 {
        let mut rng = Rng::new(60_000 + seed);
        let map = random_map(&mut rng, true, false).build();
        let last = map.points().last().unwrap().tick;
        for _ in 0..300 {
            let t = rng.range(-1_000_000, last.saturating_mul(2).max(10_000));
            let pos = map.bar_beat_tick(t);
            assert_eq!(map.tick_of(pos).unwrap(), t, "seed {seed} tick {t} {pos}");
            // Bars are in order and a bar's start is at or before the tick
            assert!(map.bar_start(pos.bar).unwrap() <= t);
            assert!(map.bar_start(pos.bar + 1).unwrap() > t);
            assert_eq!(map.bar_beat_tick(map.bar_start(pos.bar).unwrap()), bbt(pos.bar, 1, 0));
        }
        // A signature change always starts a new bar on beat 1
        let mut previous = None;
        for p in map.points() {
            if previous != Some(p.time_signature) {
                let pos = map.bar_beat_tick(p.tick);
                assert_eq!((pos.beat, pos.tick), (1, 0), "seed {seed}");
            }
            previous = Some(p.time_signature);
        }
    }
}

#[test]
fn musical_position_for_display() {
    let map = changing_map();
    let s = map.sample_at(7680 + 3360 + 480 + 10); // just after the tempo change to 90
    let pos = map.musical_position(s);
    assert_eq!(pos.sample, s);
    assert_eq!(pos.tick, 7680 + 3360 + 480 + 10);
    assert_eq!(pos.bar_beat_tick, bbt(4, 2, 10));
    assert_eq!(pos.time_signature, sig(7, 8));
    assert_eq!(pos.bpm, 90.0);
    assert_eq!(pos.bar_beat_tick.to_string(), "4:2:10");
    assert_eq!(map.musical_position(0).bpm, 120.0);
}
