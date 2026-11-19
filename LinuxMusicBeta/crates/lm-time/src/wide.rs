//! A fixed-size signed 256-bit integer, just what the exact ramp arithmetic needs (no allocation, no `unsafe`).
//!
//! Sign and magnitude; the magnitude is four little-endian `u64` limbs. Every operation that would overflow 256
//! bits of magnitude panics (the crate's bounds keep the ramp values under 2²³⁰, so a panic means a bug).

use std::cmp::Ordering;
use std::ops::{Add, Div, Mul, Neg, Rem, Sub};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct U256([u64; 4]);

impl U256 {
    const ZERO: U256 = U256([0; 4]);

    fn from_u128(v: u128) -> Self {
        U256([v as u64, (v >> 64) as u64, 0, 0])
    }

    fn is_zero(&self) -> bool {
        self.0 == [0; 4]
    }

    fn bits(&self) -> u32 {
        for i in (0..4).rev() {
            if self.0[i] != 0 {
                return 64 * i as u32 + (64 - self.0[i].leading_zeros());
            }
        }
        0
    }

    fn checked_add(self, o: Self) -> Option<Self> {
        let mut out = [0_u64; 4];
        let mut carry = 0_u64;
        for (i, limb) in out.iter_mut().enumerate() {
            let (s1, c1) = self.0[i].overflowing_add(o.0[i]);
            let (s2, c2) = s1.overflowing_add(carry);
            *limb = s2;
            carry = u64::from(c1) + u64::from(c2);
        }
        (carry == 0).then_some(U256(out))
    }

    /// `self - o`, requires `self >= o`
    fn sub_smaller(self, o: Self) -> Self {
        debug_assert!(self >= o);
        let mut out = [0_u64; 4];
        let mut borrow = 0_u64;
        for (i, limb) in out.iter_mut().enumerate() {
            let (d1, b1) = self.0[i].overflowing_sub(o.0[i]);
            let (d2, b2) = d1.overflowing_sub(borrow);
            *limb = d2;
            borrow = u64::from(b1) + u64::from(b2);
        }
        U256(out)
    }

    fn checked_mul(self, o: Self) -> Option<Self> {
        let mut wide = [0_u64; 8];
        for i in 0..4 {
            if self.0[i] == 0 {
                continue;
            }
            let mut carry = 0_u128;
            for j in 0..4 {
                let t = u128::from(self.0[i]) * u128::from(o.0[j]) + u128::from(wide[i + j]) + carry;
                wide[i + j] = t as u64;
                carry = t >> 64;
            }
            wide[i + 4] = carry as u64;
        }
        (wide[4..] == [0; 4]).then_some(U256([wide[0], wide[1], wide[2], wide[3]]))
    }

    fn shl(self, n: u32) -> Self {
        debug_assert!(n < 256);
        let (limbs, bits) = ((n / 64) as usize, n % 64);
        let mut out = [0_u64; 4];
        for i in (limbs..4).rev() {
            let src = i - limbs;
            out[i] = self.0[src] << bits;
            if bits > 0 && src > 0 {
                out[i] |= self.0[src - 1] >> (64 - bits);
            }
        }
        U256(out)
    }

    fn shr1(self) -> Self {
        let mut out = [0_u64; 4];
        for (i, limb) in out.iter_mut().enumerate() {
            *limb = self.0[i] >> 1;
            if i < 3 {
                *limb |= self.0[i + 1] << 63;
            }
        }
        U256(out)
    }

    fn set_bit(&mut self, n: u32) {
        self.0[(n / 64) as usize] |= 1 << (n % 64);
    }

    /// Quotient and remainder; panics on division by zero
    fn div_rem(self, d: Self) -> (Self, Self) {
        assert!(!d.is_zero(), "division by zero");
        if self < d {
            return (U256::ZERO, self);
        }
        let shift = self.bits() - d.bits();
        let mut divisor = d.shl(shift);
        let mut rem = self;
        let mut quot = U256::ZERO;
        for i in (0..=shift).rev() {
            if rem >= divisor {
                rem = rem.sub_smaller(divisor);
                quot.set_bit(i);
            }
            divisor = divisor.shr1();
        }
        (quot, rem)
    }

    fn to_f64(self) -> f64 {
        self.0.iter().rev().fold(0.0, |acc, &limb| acc * 18_446_744_073_709_551_616.0 + limb as f64)
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for U256 {
    fn cmp(&self, other: &Self) -> Ordering {
        for i in (0..4).rev() {
            match self.0[i].cmp(&other.0[i]) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        Ordering::Equal
    }
}

/// A signed 256-bit integer (sign and magnitude; zero is never negative)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct I256 {
    negative: bool,
    magnitude: U256,
}

impl I256 {
    pub(crate) const ZERO: I256 = I256 { negative: false, magnitude: U256::ZERO };
    pub(crate) const ONE: I256 = I256 { negative: false, magnitude: U256([1, 0, 0, 0]) };

    fn new(negative: bool, magnitude: U256) -> Self {
        Self { negative: negative && !magnitude.is_zero(), magnitude }
    }

    pub(crate) fn abs(self) -> Self {
        Self::new(false, self.magnitude)
    }

    #[cfg(test)]
    fn is_negative(self) -> bool {
        self.negative
    }

    /// The value as an `i128`; panics if it does not fit
    pub(crate) fn as_i128(self) -> i128 {
        let m = self.magnitude;
        assert!(m.0[2] == 0 && m.0[3] == 0, "I256 does not fit in i128");
        let v = u128::from(m.0[0]) | (u128::from(m.0[1]) << 64);
        if self.negative {
            assert!(v <= 1 << 127, "I256 does not fit in i128");
            (v as i128).wrapping_neg()
        } else {
            i128::try_from(v).expect("I256 does not fit in i128")
        }
    }

    /// The nearest-ish `f64` (for search guesses and display only)
    pub(crate) fn as_f64(self) -> f64 {
        let v = self.magnitude.to_f64();
        if self.negative { -v } else { v }
    }
}

impl From<i128> for I256 {
    fn from(v: i128) -> Self {
        Self::new(v < 0, U256::from_u128(v.unsigned_abs()))
    }
}

impl Neg for I256 {
    type Output = I256;

    fn neg(self) -> I256 {
        Self::new(!self.negative, self.magnitude)
    }
}

impl Add for I256 {
    type Output = I256;

    fn add(self, o: I256) -> I256 {
        if self.negative == o.negative {
            let m = self.magnitude.checked_add(o.magnitude).expect("I256 overflow");
            return Self::new(self.negative, m);
        }
        if self.magnitude >= o.magnitude {
            Self::new(self.negative, self.magnitude.sub_smaller(o.magnitude))
        } else {
            Self::new(o.negative, o.magnitude.sub_smaller(self.magnitude))
        }
    }
}

impl Sub for I256 {
    type Output = I256;

    fn sub(self, o: I256) -> I256 {
        self + (-o)
    }
}

impl Mul for I256 {
    type Output = I256;

    fn mul(self, o: I256) -> I256 {
        let m = self.magnitude.checked_mul(o.magnitude).expect("I256 overflow");
        Self::new(self.negative != o.negative, m)
    }
}

impl Div for I256 {
    type Output = I256;

    /// Truncating division (toward zero), like the primitive integers
    fn div(self, o: I256) -> I256 {
        let (q, _) = self.magnitude.div_rem(o.magnitude);
        Self::new(self.negative != o.negative, q)
    }
}

impl Rem for I256 {
    type Output = I256;

    /// The remainder of the truncating division (sign of the dividend), like the primitive integers
    fn rem(self, o: I256) -> I256 {
        let (_, r) = self.magnitude.div_rem(o.magnitude);
        Self::new(self.negative, r)
    }
}

impl PartialOrd for I256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for I256 {
    fn cmp(&self, o: &Self) -> Ordering {
        match (self.negative, o.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => self.magnitude.cmp(&o.magnitude),
            (true, true) => o.magnitude.cmp(&self.magnitude),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        /// A random i128 with a random number of significant bits
        fn i128(&mut self) -> i128 {
            let v = (u128::from(self.next()) << 64 | u128::from(self.next())) >> (self.next() % 128);
            let v = (v >> 1) as i128;
            if self.next() & 1 == 0 { v } else { -v }
        }
    }

    #[test]
    fn agrees_with_i128_where_it_fits() {
        let mut rng = Rng(1);
        for _ in 0..200_000 {
            let (a, b) = (rng.i128(), rng.i128());
            let (wa, wb) = (I256::from(a), I256::from(b));
            assert_eq!(wa.cmp(&wb), a.cmp(&b));
            if let Some(s) = a.checked_add(b) {
                assert_eq!((wa + wb).as_i128(), s);
            }
            if let Some(s) = a.checked_sub(b) {
                assert_eq!((wa - wb).as_i128(), s);
            }
            if let Some(p) = a.checked_mul(b) {
                assert_eq!((wa * wb).as_i128(), p);
            }
            if b != 0 && a.checked_div(b).is_some() {
                assert_eq!((wa / wb).as_i128(), a / b);
                assert_eq!((wa % wb).as_i128(), a % b);
            }
        }
    }

    #[test]
    fn wide_products_divide_back() {
        let mut rng = Rng(2);
        for _ in 0..100_000 {
            let (a, b, c) = (rng.i128(), rng.i128(), rng.i128());
            if b == 0 {
                continue;
            }
            // (a·b + r) / b == a and % b == r, with |r| < |b| and r of the sign of the dividend
            let r = c % b;
            let r = if (a < 0) != (b < 0) { -r.abs() } else { r.abs() };
            let n = I256::from(a) * I256::from(b) + I256::from(r);
            assert_eq!((n / I256::from(b)).as_i128(), a, "a {a} b {b} r {r}");
            assert_eq!((n % I256::from(b)).as_i128(), r, "a {a} b {b} r {r}");
        }
    }

    #[test]
    fn edges() {
        let max = I256::from(i128::MAX);
        let min = I256::from(i128::MIN);
        assert_eq!(min.as_i128(), i128::MIN);
        assert_eq!((max * max) / max, max);
        assert_eq!((min * min) / min, min);
        assert!(min * max < I256::ZERO);
        assert_eq!(I256::from(-5) % I256::from(3), I256::from(-2));
        assert_eq!(-I256::ZERO, I256::ZERO);
        assert!(!(I256::from(3) - I256::from(3)).is_negative());
        assert_eq!((max * max).as_f64(), (i128::MAX as f64) * (i128::MAX as f64));
        let big = max * max * I256::from(2);
        assert!(big > max * max);
    }

    #[test]
    #[should_panic(expected = "I256 overflow")]
    fn overflow_panics() {
        let big = I256::from(i128::MAX) * I256::from(i128::MAX);
        let _ = big * I256::from(8);
    }
}
