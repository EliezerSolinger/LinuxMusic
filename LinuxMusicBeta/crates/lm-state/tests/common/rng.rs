//! A small seeded generator for the randomised tests (the workspace takes no test-only crates, so no `proptest`).
//! Every run uses fixed seeds, and `LM_TEST_SEED=<n>` replays another; a failure prints the seed and the case.

#![allow(dead_code)]

use lm_state::{MAX_SAFE_INT, Resource, Value};

/// SplitMix64: tiny, fast, good enough to explore inputs, and the same sequence on every platform
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The seed of a test: `LM_TEST_SEED` if set, else `default`
    pub fn seed(default: u64) -> u64 {
        std::env::var("LM_TEST_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`)
    pub fn below(&mut self, n: usize) -> usize {
        let n64 = u64::try_from(n).unwrap_or(u64::MAX);
        usize::try_from(self.next_u64() % n64).unwrap_or(0)
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    pub fn byte(&mut self) -> u8 {
        self.next_u64().to_le_bytes()[0]
    }

    pub fn bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.below(max_len + 1);
        (0..len).map(|_| self.byte()).collect()
    }

    /// Any finite real, with the edge cases often: every bit pattern, -0.0, subnormals, extremes
    pub fn real(&mut self) -> f64 {
        match self.below(8) {
            0 => -0.0,
            1 => 0.0,
            2 => [f64::MIN_POSITIVE, f64::MAX, f64::MIN, 5e-324, -5e-324, 0.1, 1.0 / 3.0][self.below(7)],
            3 => (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 * 2e6 - 1e6,
            _ => loop {
                let v = f64::from_bits(self.next_u64());
                if v.is_finite() {
                    break v;
                }
            },
        }
    }

    pub fn int(&mut self) -> i64 {
        match self.below(4) {
            0 => [0, 1, -1, MAX_SAFE_INT, -MAX_SAFE_INT][self.below(5)],
            1 => i64::try_from(self.below(1000)).unwrap_or(0) - 500,
            _ => {
                let span = u64::try_from(MAX_SAFE_INT).unwrap_or(0) * 2 + 1;
                i64::try_from(self.next_u64() % span).unwrap_or(0) - MAX_SAFE_INT
            }
        }
    }

    /// Texts with what is hard to write: quotes, backslashes, control characters, non-ASCII, astral characters
    pub fn text(&mut self) -> String {
        let len = self.below(14);
        (0..len)
            .map(|_| match self.below(6) {
                0 => ['"', '\\', '/', '\n', '\t', '\u{0}', '\u{1f}', '\u{7f}', '\u{2028}'][self.below(9)],
                1 => ['é', 'ç', '😀', '𝄞', '\u{feff}', '~'][self.below(6)],
                2 => loop {
                    if let Some(c) = char::from_u32(u32::try_from(self.next_u64() % 0x11_0000).unwrap_or(0)) {
                        break c;
                    }
                },
                _ => char::from(b'a' + u8::try_from(self.below(26)).unwrap_or(0)),
            })
            .collect()
    }

    pub fn resource(&mut self) -> Resource {
        let kind = ["audio", "state", "soundfont", "drum-kit", "x_1"][self.below(5)];
        Resource::of_content(kind, &self.next_u64().to_le_bytes()).unwrap()
    }

    pub fn leaf(&mut self) -> Value {
        match self.below(7) {
            0 => Value::NULL,
            1 => Value::bool(self.chance(2)),
            2 => Value::int(self.int()),
            3 | 4 => Value::real(self.real()).unwrap(),
            5 => Value::text(self.text()),
            _ => Value::resource(self.resource()),
        }
    }

    /// A random tree up to `depth` levels and about `budget` nodes, lists and objects of up to 8 entries
    pub fn tree(&mut self, depth: usize) -> Value {
        let mut budget = 200usize;
        self.tree_within(depth, &mut budget)
    }

    fn tree_within(&mut self, depth: usize, budget: &mut usize) -> Value {
        if depth == 0 || *budget == 0 || self.chance(3) {
            *budget = budget.saturating_sub(1);
            return self.leaf();
        }
        let len = self.below(9).min(*budget);
        *budget = budget.saturating_sub(1);
        if self.chance(2) {
            Value::list((0..len).map(|_| self.tree_within(depth - 1, budget)).collect::<Vec<_>>())
        } else {
            Value::object((0..len).map(|_| (self.text(), self.tree_within(depth - 1, budget))).collect::<Vec<_>>())
        }
    }
}
