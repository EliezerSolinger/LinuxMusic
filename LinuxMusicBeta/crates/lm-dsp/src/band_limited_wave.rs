//! The legacy `BandLimitedWave`: saw, square, triangle and Moog saw as mipmapped tables of additive harmonics up to
//! Nyquist, one table for each wavelength of 2 or 3 times a power of two (2 to 6144 samples), read with a 4-point
//! optimal interpolation. The legacy program ships them as files made by this same generation (they null against
//! it); they are generated here, once.

use std::sync::OnceLock;

use crate::math::{f32c, sin};

/// The tables' lengths: 2 and 3 times the powers of two, up to 3 << 11
pub const TABLE_LENGTHS: [usize; 24] = {
    let mut t = [0usize; 24];
    let mut k = 0;
    while k < 24 {
        t[k] = (if k % 2 == 0 { 2 } else { 3 }) << (k / 2);
        k += 1;
    }
    t
};
/// The last table
pub const MAX_TABLE: usize = 23;

/// The band-limited waves
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandLimitedWaveform {
    Saw,
    Square,
    Triangle,
    Moog,
}

/// One wave's tables, longest last
type MipMap = Vec<Vec<f32>>;

fn generate(odd_only: bool, amp: impl Fn(i32) -> f64, offset: impl Fn(i32) -> f64) -> MipMap {
    // (the angle's 2 pi in single precision, as the legacy program wrote it)
    let two_pi = 2.0 * f64::from(f32c::PI);
    TABLE_LENGTHS
        .iter()
        .map(|&len| {
            let sums: Vec<f64> = (0..len)
                .map(|ph| {
                    let mut harm = 1i32;
                    let mut s = 0.0f64;
                    loop {
                        let hlen = len as f64 / f64::from(harm);
                        s += amp(harm) * sin((f64::from(ph as i32 * harm) / len as f64 + offset(harm)) * two_pi);
                        harm += if odd_only { 2 } else { 1 };
                        if hlen <= 2.0 {
                            break;
                        }
                    }
                    s
                })
                .collect();
            // Normalized by the largest sum (in double), the samples stored in single precision first
            let max = sums.iter().fold(0.0f64, |m, s| m.max(s.abs()));
            let table: Vec<f32> = sums.iter().map(|&s| (f64::from(s as f32) / max) as f32).collect();
            table
        })
        .collect()
}

fn waves() -> &'static [MipMap; 4] {
    static WAVES: OnceLock<[MipMap; 4]> = OnceLock::new();
    WAVES.get_or_init(|| {
        let saw = generate(false, |h| -1.0 / f64::from(h), |_| 0.0);
        let square = generate(true, |h| 1.0 / f64::from(h), |_| 0.0);
        let triangle = generate(true, |h| 1.0 / f64::from(h * h), |h| if (h + 1) % 4 == 0 { 0.5 } else { 0.0 });
        // A triangle plus a saw a quarter period on
        let moog = TABLE_LENGTHS
            .iter()
            .enumerate()
            .map(|(i, &len)| {
                (0..len)
                    .map(|ph| {
                        let saw_ph = (ph + (len as f64 * 0.75) as usize) % len;
                        (saw[i][saw_ph] + triangle[i][ph]) * 0.5
                    })
                    .collect()
            })
            .collect();
        [saw, square, triangle, moog]
    })
}

/// Table @p t (of [`TABLE_LENGTHS`]) of @p wave
pub fn table(wave: BandLimitedWaveform, t: usize) -> &'static [f32] {
    &waves()[wave as usize][t]
}

/// The table a wave of @p wavelength samples is read from
#[inline]
pub fn table_for(wavelength: f32) -> usize {
    let mut t = 0;
    while t < MAX_TABLE && wavelength >= TABLE_LENGTHS[t + 1] as f32 {
        t += 1;
    }
    t
}

/// Builds the tables now (they are built on first use otherwise): away from the audio thread
pub fn prepare() {
    let _ = waves();
}

/// The legacy `optimal4pInterpolate` (Olli Niemitalo's)
#[inline]
pub fn optimal_4p(v0: f32, v1: f32, v2: f32, v3: f32, x: f32) -> f32 {
    let z = x - 0.5;
    let even1 = v2 + v1;
    let odd1 = v2 - v1;
    let even2 = v3 + v0;
    let odd2 = v3 - v0;
    let c0 = (f64::from(even1) * 0.45868970870461956 + f64::from(even2) * 0.04131401926395584) as f32;
    let c1 = (f64::from(odd1) * 0.48068024766578432 + f64::from(odd2) * 0.17577925564495955) as f32;
    let c2 = (f64::from(even1) * -0.246185007019907091 + f64::from(even2) * 0.24614027139700284) as f32;
    let c3 = (f64::from(odd1) * -0.36030925263849456 + f64::from(odd2) * 0.10174985775982505) as f32;
    ((c3 * z + c2) * z + c1) * z + c0
}

/// The wave at phase @p phase (periods) for a wavelength of @p wavelength samples
#[inline]
pub fn oscillate(phase: f32, wavelength: f32, wave: BandLimitedWaveform) -> f32 {
    // The next longer table
    let t = table_for(wavelength);
    let len = TABLE_LENGTHS[t];
    let ph = phase - phase.trunc();
    let lookup_f = ph * len as f32;
    let lookup = lookup_f as i32;
    let ip = lookup_f - lookup_f.trunc();
    let table = &waves()[wave as usize][t];
    let at = |i: i32| table[i.rem_euclid(len as i32) as usize];
    let s1 = at(lookup);
    let s2 = at((lookup + 1) % len as i32);
    let s0 = at(if lookup == 0 { len as i32 - 1 } else { lookup - 1 });
    let s3 = at((lookup + 2) % len as i32);
    optimal_4p(s0, s1, s2, s3, ip)
}
