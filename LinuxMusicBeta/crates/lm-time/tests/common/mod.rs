//! Shared by the integration tests: a seeded generator, random maps, and an independent reference for maps of
//! constant segments
#![allow(dead_code)]

use lm_time::{Bpm, Ppq, TempoMap, TempoPoint, TempoShape, TimeSignature};

/// splitmix64: small, seeded, reproducible (a failing case prints its seed)
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `lo..=hi`
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (i128::from(hi) - i128::from(lo) + 1) as u128;
        let v = (u128::from(self.next_u64()) << 64 | u128::from(self.next_u64())) % span;
        (i128::from(lo) + v as i128) as i64
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }

    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.range(0, items.len() as i64 - 1) as usize]
    }
}

pub fn bpm(text: &str) -> Bpm {
    Bpm::parse(text).unwrap()
}

pub fn sig(n: u8, d: u8) -> TimeSignature {
    TimeSignature::new(n, d).unwrap()
}

pub fn random_bpm(rng: &mut Rng) -> Bpm {
    let micro = match rng.range(0, 3) {
        0 => rng.range(40, 240) * 1_000_000,
        1 => rng.range(40_000, 240_000) * 1000,
        2 => rng.range(1_000_000, 1_000_000_000),
        _ => rng.pick(&[1_000_000, 1_000_000_000, 119_880_000, 120_000_000, 999_999_999, 1_000_001]),
    };
    Bpm::from_micro(micro as u64).unwrap()
}

/// A random valid signature for this PPQ
pub fn random_signature(rng: &mut Rng, ppq: Ppq) -> TimeSignature {
    loop {
        let s = sig(rng.range(1, 15) as u8, rng.pick(&[1, 2, 4, 8, 16, 32, 64]));
        if s.beat_ticks(ppq).is_some() {
            return s;
        }
    }
}

pub struct MapSpec {
    pub sample_rate: u32,
    pub ppq: Ppq,
    pub points: Vec<TempoPoint>,
}

impl MapSpec {
    pub fn build(&self) -> TempoMap {
        TempoMap::new(self.sample_rate, self.ppq, &self.points).unwrap()
    }
}

/// A random map. `ramps` allows linear segments; `realistic` keeps common rates and 960 PPQ
pub fn random_map(rng: &mut Rng, ramps: bool, realistic: bool) -> MapSpec {
    let sample_rate = if realistic {
        rng.pick(&[44_100, 48_000, 88_200, 96_000, 192_000])
    } else {
        match rng.range(0, 3) {
            0 => rng.pick(&[8_000, 11_025, 22_050, 44_100, 48_000, 96_000, 192_000, 384_000]),
            1 => rng.range(1, 4_000_000) as u32,
            2 => rng.range(1, 2_000) as u32,
            _ => 4_000_000,
        }
    };
    let ppq = if realistic {
        Ppq::DEFAULT
    } else {
        let any = rng.range(1, 100_000) as u32;
        Ppq::new(rng.pick(&[960, 960, 480, 96, 24, 1, 7, 1001, 100_000, any])).unwrap()
    };
    let count = rng.range(1, 12);
    let mut points = Vec::new();
    let mut tick = 0_i64;
    for i in 0..count {
        if i > 0 {
            tick += match rng.range(0, 3) {
                0 => rng.range(1, 4),
                1 => rng.range(1, 4 * 3840),
                2 => rng.range(1, 3840 * 400),
                _ => rng.range(1, 3840 * 40_000),
            };
        }
        let shape = if ramps && i + 1 < count && rng.chance(50) { TempoShape::Linear } else { TempoShape::Constant };
        let time_signature = if i == 0 || rng.chance(30) {
            random_signature(rng, ppq)
        } else {
            points.last().map_or(TimeSignature::COMMON, |p: &TempoPoint| p.time_signature)
        };
        let bpm = if i > 0 && rng.chance(10) {
            points.last().map_or(Bpm::default(), |p: &TempoPoint| p.bpm)
        } else {
            random_bpm(rng)
        };
        points.push(TempoPoint { tick, bpm, time_signature, shape });
    }
    MapSpec { sample_rate, ppq, points }
}

/// `round_half_even(n / d)`, `d > 0`, written independently of the crate (via truncating division)
pub fn ref_round_half_even(n: i128, d: i128) -> i128 {
    let mut q = n / d;
    let mut r = n % d;
    if r < 0 {
        q -= 1;
        r += d;
    }
    if 2 * r > d || (2 * r == d && q % 2 != 0) { q + 1 } else { q }
}

/// An independent reference for maps of constant segments: the anchors summed from the points, then
/// `sample = anchor + round_half_even(dt · 60 · sr · 10⁶ / (µbpm · ppq))`, and `tick_at` by its definition
/// (`max t : sample_at(t) <= s`) found by bisection
pub struct Reference {
    sample_rate: i128,
    ppq: i128,
    ticks: Vec<i64>,
    micro: Vec<i128>,
    anchors: Vec<i128>,
}

impl Reference {
    pub fn new(spec: &MapSpec) -> Self {
        assert!(spec.points.iter().all(|p| p.shape == TempoShape::Constant));
        let sample_rate = i128::from(spec.sample_rate);
        let ppq = i128::from(spec.ppq.get());
        let ticks: Vec<i64> = spec.points.iter().map(|p| p.tick).collect();
        let micro: Vec<i128> = spec.points.iter().map(|p| i128::from(p.bpm.micro())).collect();
        let mut anchors = vec![0_i128];
        for i in 1..ticks.len() {
            let dt = i128::from(ticks[i] - ticks[i - 1]);
            let prev = anchors[i - 1];
            anchors.push(prev + ref_round_half_even(dt * 60 * sample_rate * 1_000_000, micro[i - 1] * ppq));
        }
        Self { sample_rate, ppq, ticks, micro, anchors }
    }

    pub fn sample_at(&self, tick: i64) -> i128 {
        let i = self.ticks.partition_point(|&t| t <= tick).saturating_sub(1);
        let dt = i128::from(tick) - i128::from(self.ticks[i]);
        self.anchors[i] + ref_round_half_even(dt * 60 * self.sample_rate * 1_000_000, self.micro[i] * self.ppq)
    }

    pub fn tick_at(&self, sample: i64) -> i64 {
        let s = i128::from(sample);
        // Bracket: sample_at is non-decreasing and unbounded both ways
        let (mut lo, mut hi) = (-1_i64, 1_i64);
        while self.sample_at(lo) > s {
            lo *= 2;
        }
        while self.sample_at(hi) <= s {
            hi *= 2;
        }
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if self.sample_at(mid) <= s {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }
}
