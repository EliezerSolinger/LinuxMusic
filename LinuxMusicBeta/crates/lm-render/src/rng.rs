//! The per-node random generator (C13): `xoshiro256**`, seeded from the project seed and the node's id by the
//! compiler, so a render is the same every time (arquitetura-nova.md, base item 9)

/// `xoshiro256**` (Blackman and Vigna). Never seeded from the clock
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// The generator for a seed; the four words of state come from `splitmix64`, as the authors recommend, so any
    /// seed (zero too) gives a good state
    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        let mut next = || {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Self { s: [next(), next(), next(), next()] }
    }

    /// The seed of a node from the project seed and a stable number of the node (the compiler hashes the `NodeId`)
    pub fn seed_for(project_seed: u64, node: u64) -> u64 {
        let mut r = Rng::new(project_seed ^ node.rotate_left(32));
        r.next_u64()
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in `[0, 1)`, 24 bits
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / (1u64 << 24) as f32)
    }

    /// Uniform in `[0, 1)`, 53 bits
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[-1, 1)`
    pub fn bipolar(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }

    /// Uniform in `0..n` (`n > 0`), without modulo bias (Lemire's method)
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "Rng::below(0)");
        loop {
            let m = u128::from(self.next_u64()) * u128::from(n);
            let low = m as u64;
            if low >= n.wrapping_neg() % n {
                return (m >> 64) as u64;
            }
        }
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn reference_sequence() {
        // The reference implementation with state {1, 2, 3, 4}: 11520, 0, 1509978240, 1215971899390074240
        let mut r = Rng { s: [1, 2, 3, 4] };
        assert_eq!(r.next_u64(), 11520);
        assert_eq!(r.next_u64(), 0);
        assert_eq!(r.next_u64(), 1_509_978_240);
        assert_eq!(r.next_u64(), 1_215_971_899_390_074_240);
    }

    #[test]
    fn same_seed_same_numbers() {
        let mut a = Rng::new(271_828);
        let mut b = Rng::new(271_828);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_ne!(Rng::seed_for(1, 2), Rng::seed_for(1, 3));
    }

    #[test]
    fn ranges() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let f = r.next_f32();
            assert!((0.0..1.0).contains(&f));
            let b = r.bipolar();
            assert!((-1.0..1.0).contains(&b));
            assert!(r.below(10) < 10);
        }
    }
}
