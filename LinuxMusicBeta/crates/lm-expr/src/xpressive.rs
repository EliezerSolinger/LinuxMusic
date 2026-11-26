//! The pure helpers of the legacy Xpressive (`ExprSynth.cpp`): the wave functions, `semitone`, `cent`, the random
//! vector (`randv`, `randsv`), `integrate`'s per-call-site counters, `last(n)`'s history and the wave-table readers,
//! with the exact float arithmetic of the original. [`symbols`] registers the legacy's variables and functions in a
//! fixed order and [`Voice`] is the [`Host`] that dispatches them

use crate::symbols::{FnId, Symbols, VarId};
use crate::vm::Host;
use core::f32::consts::PI;

const RANDOM_DATA: [u32; 257] = [
    0xd76a33ec, 0x4a767724, 0xb34ebd08, 0xf4024196, 0x17b426e2, 0x8dc6389a, 0x1b5dcb93, 0xa771bd3f, 0x078d502e,
    0x8980988a, 0x1f64f846, 0xb5b48ed7, 0xf0742cfb, 0xe7c66303, 0xc9472876, 0x6c7494a5, 0x5c2203a1, 0x23986344,
    0x7d344fa0, 0x4f39474a, 0x28ac8b2b, 0x10f779b2, 0x6e79e659, 0x32e44c52, 0xf790aa55, 0x98b05083, 0xb5d44f1c,
    0xe553da04, 0xa884c6d2, 0x43274953, 0xbcb57404, 0x43f7d32a, 0xf1890f8b, 0x019f4dce, 0x5c4ede33, 0x2dec1a7e,
    0x0f3eab09, 0x2197c93c, 0xae933f42, 0x80d4b111, 0x6e5bd30a, 0x17139c70, 0xe15f7eb0, 0x1798f893, 0xe1c6be1c,
    0xe21edf9b, 0x4702e081, 0x8a2cb85a, 0xbf3c1f15, 0x147f4685, 0x9221d731, 0x3c7580f3, 0xc1c08498, 0x8e198b35,
    0xf821c15a, 0x4d3cd2d4, 0xad89a3b7, 0xd48f915f, 0xcaf893f0, 0xa64a4b8e, 0x20715f54, 0x1ba4de0a, 0x17ac6e91,
    0xd82ea8c0, 0x638a0ba5, 0xe7a76c0f, 0x486c5476, 0x334bbd0a, 0xffe29c55, 0x7247efaf, 0x15f98e83, 0x7a4a79ac,
    0x350cd175, 0xc7107908, 0xa85c67f7, 0x9c5002c4, 0x3cf27d2c, 0x314d8450, 0x05552886, 0x87a73642, 0x827832e4,
    0x9412cc67, 0x261979e6, 0xb31da27f, 0x3e6bbafb, 0x663f1968, 0xd84274e2, 0xdd91d982, 0xd25c4805, 0x9567f860,
    0xab99675c, 0x2254733b, 0x18799dd7, 0xee328916, 0xb9419a1b, 0x01b7a66f, 0xbcdc05e1, 0x788de4ae, 0x366e77cf,
    0x81a1ebd2, 0x97be859a, 0x17d4b533, 0x22dab3a9, 0xc99871ea, 0xc7502c91, 0x4474b65f, 0x655d059d, 0x0ddc1348,
    0x8325909b, 0x4873c155, 0x9fa30438, 0x7250b7a8, 0x90db2715, 0xf65e1cef, 0x41b74cf4, 0x38fba01c, 0xe9eefb40,
    0x9e5524ea, 0x1d3fc077, 0x04ec39db, 0x1c0d501c, 0xb93f26d9, 0xf9f910b9, 0x806fce99, 0x5691ffdf, 0x1e63b27a,
    0xf2035d42, 0xd3218a0b, 0x12eae6db, 0xeba372a9, 0x6f975260, 0xc514ae91, 0xebddb8ad, 0xc53207c0, 0xdbda57dc,
    0x8fb38ef4, 0xfaa4f1bc, 0x40caf49f, 0xcb394b41, 0x424fc698, 0xb79a9ece, 0x331202d6, 0xc604ed4d, 0x5e85819f,
    0x67222eda, 0xd976ba71, 0x7d083ec6, 0x040c819e, 0xc762c934, 0xa6684333, 0x2eaccc54, 0x69dc04b9, 0x0499cf36,
    0x6351f438, 0x6db2dc34, 0x787ae036, 0x11b5c6ac, 0x552b7227, 0x32a4c993, 0xf7f4c49d, 0x7ac9e2d9, 0xf3d32020,
    0x4ff01f89, 0x6f0e60bb, 0x3c6ed445, 0x7ca01986, 0x96901ecf, 0xe10df188, 0x62a6da6d, 0x8deee09f, 0x5347cb66,
    0x5249f452, 0x22704d4d, 0x6221555f, 0x6aa0ea90, 0xe1f7bae3, 0xd106626f, 0x6365a9db, 0x1989bb81, 0xfc2daa73,
    0x303c60b3, 0xcd867baa, 0x7c5787c2, 0x60082b30, 0xa68d3a81, 0x15a10f5d, 0x81b21c8a, 0x4bfb82e2, 0xff01c176,
    0xff3c8b65, 0x8cc6bd29, 0xc678d6ff, 0x99b86508, 0x3c47e314, 0x766ecc05, 0xba186cb0, 0x42f57199, 0x5ef524f4,
    0xb8419750, 0x6ae2a9d0, 0x291eaa18, 0x4e64b189, 0x506eb1d3, 0x78361d46, 0x6a2fcb7e, 0xbc0a46de, 0xb557badf,
    0xad3de958, 0xa2901279, 0x491decbf, 0x257383df, 0x94dd19d1, 0xd0cfbbe2, 0x9063d36d, 0x81e44c3b, 0x973e9cc9,
    0xfbe34690, 0x4eee3034, 0x1c413676, 0xf6735b8f, 0xf2991aca, 0x0ec85159, 0x6ce00ade, 0xad49de57, 0x025edf30,
    0x42722b67, 0x30cfa6b2, 0x32df8676, 0x387d4500, 0x97fa67fd, 0x027c994a, 0x77c71d0c, 0x478eb75a, 0x898370a6,
    0x73e7cca3, 0x34ace0ad, 0xc8ecb388, 0x5375c3aa, 0x9c194d87, 0x1b65246d, 0xca000bcf, 0x8a0fb13d, 0x81b957b0,
    0xac627bfb, 0xc0fe47e5, 0xf3db0ad8, 0x1c605c7d, 0x5f579884, 0x63e079b5, 0x3d96f7cf, 0x3edd46e9, 0xc347c61e,
    0x7b2b2a0e, 0x63dfcf51, 0x596781dd, 0x80304c4d, 0xa66f8b47,
];

/// `absFraction`: `x - floor(x)`
#[inline]
pub fn abs_fraction(x: f32) -> f32 {
    x - libm::floorf(x)
}

/// `fraction`: `x` minus its integer part (towards zero)
#[inline]
pub fn fraction(x: f32) -> f32 {
    x - (x as i32) as f32
}

/// `positiveFraction`: the fractional part in `[0, 1)`, 0 for NaN and infinity
#[inline]
pub fn positive_fraction(x: f32) -> f32 {
    if x.is_nan() || x.is_infinite() { 0.0 } else { abs_fraction(x) }
}

pub fn sinew(x: f32) -> f32 {
    let x = positive_fraction(x);
    libm::sinf(x * 2.0 * PI)
}

pub fn squarew(x: f32) -> f32 {
    if positive_fraction(x) >= 0.5 { -1.0 } else { 1.0 }
}

pub fn trianglew(x: f32) -> f32 {
    let x = positive_fraction(x);
    if x < 0.25 {
        x * 4.0
    } else if x < 0.75 {
        2.0 - x * 4.0
    } else {
        x * 4.0 - 4.0
    }
}

/// The legacy's `saww` (the sawtooth)
pub fn sawtoothw(x: f32) -> f32 {
    let x = positive_fraction(x);
    2.0 * x - 1.0
}

pub fn moogsaww(x: f32) -> f32 {
    let x = positive_fraction(x);
    if x < 0.5 { -1.0 + x * 4.0 } else { 1.0 - 2.0 * x }
}

pub fn moogw(x: f32) -> f32 {
    let mut x = positive_fraction(x);
    if x > 0.5 {
        x = 1.0 - x;
        return -1.0 + 4.0 * x * x;
    }
    -1.0 + 8.0 * x * x
}

pub fn expw(x: f32) -> f32 {
    let mut x = positive_fraction(x);
    if x > 0.5 {
        x = 1.0 - x;
    }
    -1.0 + 8.0 * x * x
}

pub fn expnw(x: f32) -> f32 {
    let x = positive_fraction(x);
    if x > 0.5 {
        return -1.0 + 8.0 * (1.0 - x) * x;
    }
    -1.0 + 8.0 * x * x
}

/// `cent(x)`: the frequency ratio of `x` cents
pub fn cent(x: f32) -> f32 {
    libm::exp2f(x / 1200.0)
}

/// `semitone(x)`: the frequency ratio of `x` semitones
pub fn semitone(x: f32) -> f32 {
    libm::exp2f(x / 12.0)
}

/// `W1`, `W2`, `W3` without interpolation: `table[int(frac(index) * len)]`
pub fn wave_value(table: &[f32], index: f32) -> f32 {
    let i = (positive_fraction(index) * table.len() as f32) as i32;
    usize::try_from(i).ok().and_then(|i| table.get(i)).copied().unwrap_or(0.0)
}

/// libstdc++'s `std::lerp(a, b, t)` for float
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    if (a <= 0.0 && b >= 0.0) || (a >= 0.0 && b <= 0.0) {
        return t * b + (1.0 - t) * a;
    }
    if t == 1.0 {
        return b;
    }
    let x = a + t * (b - a);
    if (t > 1.0) == (b > a) {
        if b < x { x } else { b }
    } else if x < b {
        x
    } else {
        b
    }
}

/// `W1`, `W2`, `W3` with the legacy's linear interpolation (wrapping at the end of the table)
pub fn wave_value_interpolated(table: &[f32], index: f32) -> f32 {
    let size = table.len();
    if size == 0 {
        return 0.0;
    }
    let x = positive_fraction(index) * size as f32;
    let ix = usize::try_from(x as i32).unwrap_or(0);
    let frac = fraction(x);
    let a = table.get(ix).copied().unwrap_or(0.0);
    let b = table.get((ix + 1) % size).copied().unwrap_or(0.0);
    lerp(a, b, frac)
}

fn rotate_left(x: u32, b: i32) -> u32 {
    if b > -32 && b < 32 && b != 0 {
        if b < 0 {
            let n = b.unsigned_abs();
            (x >> n) | (x << (32 - n))
        } else {
            let n = b as u32;
            (x << n) | (x >> (32 - n))
        }
    } else {
        x
    }
}

const DATA_SIZE: u32 = 257;

fn random_at(i: u32) -> u32 {
    RANDOM_DATA.get((i % DATA_SIZE) as usize).copied().unwrap_or(0)
}

/// The legacy's `RandomVectorSeedFunction::randv(index, seed)`: a fixed pseudo-random value in `[-1, 1)` per index
/// and seed (0 for a negative, NaN or infinite index)
pub fn randv_seeded(index: f32, seed: i32) -> f32 {
    if index < 0.0 || index.is_nan() || index.is_infinite() {
        return 0.0;
    }
    let xi = index as u32;
    // The legacy's seed is an int, non-negative where it matters; a negative one is taken as its magnitude's wrap
    let seed = seed.max(0) as u32;
    let si = seed % DATA_SIZE;
    let sa = seed / DATA_SIZE;
    let mut res = rotate_left(
        random_at(xi.wrapping_add(23_u32.wrapping_mul(si)).wrapping_add(1)) ^ random_at(xi / DATA_SIZE + sa),
        (sa % 31 + 1) as i32,
    );
    res ^= rotate_left(
        random_at(3_u32.wrapping_mul(xi).wrapping_add(si).wrapping_add(13)),
        (xi.wrapping_add(2_u32.wrapping_mul(si)) % 32) as i32,
    ) ^ rotate_left(random_at(xi / DATA_SIZE + 2 * sa), (xi % 31 + 1) as i32);
    // `(float)(1 << 31)` is -2^31 in the legacy
    (res as i32) as f32 / -2_147_483_648.0_f32
}

/// `randsv(index, seed)`: the seed is a float (negative, NaN or infinite gives 0)
pub fn randsv(index: f32, seed: f32) -> f32 {
    let s = if seed < 0.0 || seed.is_nan() || seed.is_infinite() { 0 } else { seed as i32 };
    randv_seeded(index, s)
}

/// `randv(index)`: [`randv_seeded`] with the expression's own random seed
pub fn randv(index: f32, random_seed: u32) -> f32 {
    randv_seeded(index, random_seed as i32)
}

/// The generator of `rand()`: uniform in `[-1, 1)`. The legacy drew from a per-thread mt19937; this is the same
/// distribution from a small seedable generator (the A/B reference draws from the same recurrence), so a note's
/// noise is reproducible
#[derive(Clone, Debug)]
pub struct Rand(u32);

impl Rand {
    pub fn new(seed: u32) -> Self {
        Self(seed)
    }

    pub fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 8_388_608.0 - 1.0
    }
}

/// `integrate(x)`: a running sum per call site, divided by the sample rate. The sums are doubles as the legacy's
#[derive(Clone, Debug)]
pub struct Integrate {
    counters: Vec<f64>,
    sample_rate: f32,
}

impl Integrate {
    /// `sites`: how many calls of `integrate` the expressions hold (`Expr::call_count`), summed over the expressions
    /// that share it; `sample_rate` in Hz
    pub fn new(sites: usize, sample_rate: f32) -> Self {
        Self { counters: vec![0.0; sites], sample_rate }
    }

    /// The value of the integral before this sample, then adds `x`. A site beyond the counters gives 0
    pub fn step(&mut self, site: u32, x: f32) -> f32 {
        match self.counters.get_mut(site as usize) {
            Some(c) => {
                let res = *c as f32;
                *c += f64::from(x);
                res / self.sample_rate
            }
            None => 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.counters.iter_mut().for_each(|c| *c = 0.0);
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }
}

/// `last(n)`: the output `n` samples ago (1 to the history size), 0 outside
#[derive(Clone, Debug)]
pub struct History {
    samples: Vec<f32>,
    pivot: usize,
}

impl History {
    pub fn new(size: usize) -> Self {
        let size = size.max(1);
        Self { samples: vec![0.0; size], pivot: size - 1 }
    }

    pub fn size(&self) -> usize {
        self.samples.len()
    }

    /// `last(x)`
    pub fn last(&self, x: f32) -> f32 {
        if !x.is_nan() && x >= 1.0 && x <= self.samples.len() as f32 {
            let i = (x as usize + self.pivot) % self.samples.len();
            return self.samples.get(i).copied().unwrap_or(0.0);
        }
        0.0
    }

    /// Records the sample the expression just returned (NaN and infinity are not stored, the position moves)
    pub fn push(&mut self, sample: f32) {
        if sample.is_finite() {
            if let Some(s) = self.samples.get_mut(self.pivot) {
                *s = sample;
            }
        }
        self.pivot = if self.pivot == 0 { self.samples.len() - 1 } else { self.pivot - 1 };
    }

    pub fn clear(&mut self) {
        self.samples.iter_mut().for_each(|s| *s = 0.0);
        self.pivot = self.samples.len() - 1;
    }
}

/// What a legacy function name stands for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rand,
    RandV,
    RandSv,
    Integrate,
    Semitone,
    Cent,
    Last,
    SineW,
    SquareW,
    TriangleW,
    /// The legacy's name `saww`
    SawW,
    /// The same function under the name `sawtoothw`
    SawtoothW,
    MoogSawW,
    MoogW,
    ExpW,
    ExpNW,
    W1,
    W2,
    W3,
}

/// The legacy's functions in registration order: name, arguments, side effects. `rand` and `integrate` have side
/// effects (the legacy left them out of its optimiser); the pure ones do not
const FUNCS: [(&str, u8, bool, Kind); 19] = [
    ("rand", 0, true, Kind::Rand),
    ("randv", 1, false, Kind::RandV),
    ("randsv", 2, false, Kind::RandSv),
    ("integrate", 1, true, Kind::Integrate),
    ("semitone", 1, false, Kind::Semitone),
    ("cent", 1, false, Kind::Cent),
    ("last", 1, false, Kind::Last),
    ("sinew", 1, false, Kind::SineW),
    ("squarew", 1, false, Kind::SquareW),
    ("trianglew", 1, false, Kind::TriangleW),
    ("saww", 1, false, Kind::SawW),
    ("sawtoothw", 1, false, Kind::SawtoothW),
    ("moogsaww", 1, false, Kind::MoogSawW),
    ("moogw", 1, false, Kind::MoogW),
    ("expw", 1, false, Kind::ExpW),
    ("expnw", 1, false, Kind::ExpNW),
    ("W1", 1, false, Kind::W1),
    ("W2", 1, false, Kind::W2),
    ("W3", 1, false, Kind::W3),
];

/// The variables of the legacy, written by the DSP before every sample (`bnote` and `tempo` of the old bundle are
/// not here: pass them as constants if wanted)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vars {
    pub t: VarId,
    pub f: VarId,
    pub key: VarId,
    pub rel: VarId,
    pub trel: VarId,
    pub a1: VarId,
    pub a2: VarId,
    pub a3: VarId,
    pub v: VarId,
    pub seed: VarId,
    pub srate: VarId,
}

impl Vars {
    /// Variables in the slice `eval` takes: `t, f, key, rel, trel, A1, A2, A3, v, seed, srate`
    pub const COUNT: usize = 11;
}

/// Registers the legacy's variables (in the order of [`Vars`], so `VarId(0)` is `t`) and functions (in the order of
/// [`Kind`]'s table, so [`kind`] maps an `FnId` back) in a fresh [`Symbols`]
pub fn symbols() -> (Symbols, Vars) {
    let mut s = Symbols::new();
    let mut var = |name: &str| s.add_var(name).unwrap_or(VarId(u32::MAX));
    let vars = Vars {
        t: var("t"),
        f: var("f"),
        key: var("key"),
        rel: var("rel"),
        trel: var("trel"),
        a1: var("A1"),
        a2: var("A2"),
        a3: var("A3"),
        v: var("v"),
        seed: var("seed"),
        srate: var("srate"),
    };
    for (name, arity, side_effects, _) in FUNCS {
        // The names are distinct and not reserved: registration cannot fail
        let _ = s.add_fn(name, arity, side_effects);
    }
    (s, vars)
}

/// What a function of a table made by [`symbols`] is
pub fn kind(f: FnId) -> Option<Kind> {
    FUNCS.get(f.index()).map(|entry| entry.3)
}

/// The id of a function of a table made by [`symbols`]
pub fn fn_id(kind: Kind) -> FnId {
    FUNCS.iter().position(|entry| entry.3 == kind).map_or(FnId(u32::MAX), |i| FnId(i as u32))
}

/// The host of one expression's evaluation: dispatches the legacy's functions over the voice's state
#[derive(Debug)]
pub struct Voice<'a> {
    pub integrate: &'a mut Integrate,
    pub history: &'a History,
    pub rand: &'a mut Rand,
    /// `randv`'s seed of this expression
    pub random_seed: u32,
    /// The wave tables W1, W2, W3
    pub waves: [&'a [f32]; 3],
    /// Whether each wave is read with interpolation
    pub interpolate: [bool; 3],
}

impl Voice<'_> {
    fn wave(&self, i: usize, x: f32) -> f32 {
        let table = self.waves.get(i).copied().unwrap_or(&[]);
        if self.interpolate.get(i).copied().unwrap_or(false) {
            wave_value_interpolated(table, x)
        } else {
            wave_value(table, x)
        }
    }
}

impl Host for Voice<'_> {
    fn call(&mut self, f: FnId, site: u32, args: &[f32]) -> f32 {
        let x = args.first().copied().unwrap_or(0.0);
        match kind(f) {
            Some(Kind::Rand) => self.rand.next_f32(),
            Some(Kind::RandV) => randv(x, self.random_seed),
            Some(Kind::RandSv) => randsv(x, args.get(1).copied().unwrap_or(0.0)),
            Some(Kind::Integrate) => self.integrate.step(site, x),
            Some(Kind::Semitone) => semitone(x),
            Some(Kind::Cent) => cent(x),
            Some(Kind::Last) => self.history.last(x),
            Some(Kind::SineW) => sinew(x),
            Some(Kind::SquareW) => squarew(x),
            Some(Kind::TriangleW) => trianglew(x),
            Some(Kind::SawW | Kind::SawtoothW) => sawtoothw(x),
            Some(Kind::MoogSawW) => moogsaww(x),
            Some(Kind::MoogW) => moogw(x),
            Some(Kind::ExpW) => expw(x),
            Some(Kind::ExpNW) => expnw(x),
            Some(Kind::W1) => self.wave(0, x),
            Some(Kind::W2) => self.wave(1, x),
            Some(Kind::W3) => self.wave(2, x),
            None => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Expr;

    #[test]
    fn waves_have_the_legacy_shapes() {
        assert_eq!(sinew(0.0), 0.0);
        assert!((sinew(0.25) - 1.0).abs() < 1e-6);
        assert_eq!(squarew(0.25), 1.0);
        assert_eq!(squarew(0.5), -1.0);
        assert_eq!(trianglew(0.25), 1.0);
        assert_eq!(trianglew(0.5), 0.0);
        assert_eq!(trianglew(0.75), -1.0);
        assert_eq!(sawtoothw(0.0), -1.0);
        assert_eq!(sawtoothw(0.5), 0.0);
        assert_eq!(moogsaww(0.25), 0.0);
        assert_eq!(moogsaww(0.5), 0.0);
        assert_eq!(expw(0.5), 1.0);
        assert_eq!(expw(0.0), -1.0);
        assert_eq!(moogw(0.0), -1.0);
        assert_eq!(expnw(0.5), 1.0);
        // The phase wraps, negative too, and NaN and infinity read as 0
        assert_eq!(squarew(1.25), squarew(0.25));
        assert_eq!(sawtoothw(-0.75), sawtoothw(0.25));
        assert_eq!(sinew(f32::NAN), 0.0);
        assert_eq!(sawtoothw(f32::INFINITY), -1.0);
    }

    #[test]
    fn semitone_and_cent() {
        assert_eq!(semitone(0.0), 1.0);
        assert_eq!(semitone(12.0), 2.0);
        assert_eq!(cent(1200.0), 2.0);
        assert!((semitone(7.0) - 1.498_307).abs() < 1e-5);
    }

    #[test]
    fn wave_tables() {
        let table = [0.0, 1.0, 2.0, 3.0];
        assert_eq!(wave_value(&table, 0.0), 0.0);
        assert_eq!(wave_value(&table, 0.3), 1.0);
        assert_eq!(wave_value(&table, 1.75), 3.0);
        assert_eq!(wave_value(&[], 0.3), 0.0);
        assert_eq!(wave_value_interpolated(&table, 0.125), 0.5);
        // Wraps from the last sample to the first
        assert_eq!(wave_value_interpolated(&table, 0.875), 1.5);
        assert_eq!(wave_value_interpolated(&[], 0.3), 0.0);
    }

    #[test]
    fn random_vector_is_fixed_and_bounded() {
        for i in 0..2000 {
            let v = randv_seeded(i as f32, 5);
            assert!((-1.0..=1.0).contains(&v), "{i}: {v}");
            assert_eq!(v, randv_seeded(i as f32, 5));
        }
        assert_ne!(randv_seeded(3.0, 1), randv_seeded(3.0, 2));
        assert_eq!(randv_seeded(-1.0, 0), 0.0);
        assert_eq!(randv_seeded(f32::NAN, 0), 0.0);
        assert_eq!(randsv(4.0, -3.0), randv_seeded(4.0, 0));
        assert_eq!(randsv(4.0, 9.0), randv_seeded(4.0, 9));
        // The first value of the legacy's table path, worked by hand for index 0, seed 0:
        // rotl(d[1] ^ d[0], 1) ^ rotl(d[13], 0) ^ rotl(d[0], 1)
        let d = |i: usize| RANDOM_DATA[i];
        let want = (d(1) ^ d(0)).rotate_left(1) ^ d(13) ^ d(0).rotate_left(1);
        assert_eq!(randv_seeded(0.0, 0), (want as i32) as f32 / -2_147_483_648.0);
    }

    #[test]
    fn integrate_counts_per_site() {
        let mut i = Integrate::new(2, 100.0);
        assert_eq!(i.step(0, 1.0), 0.0);
        assert_eq!(i.step(1, 10.0), 0.0);
        assert_eq!(i.step(0, 1.0), 0.01);
        assert_eq!(i.step(1, 10.0), 0.1);
        assert_eq!(i.step(5, 1.0), 0.0);
        i.reset();
        assert_eq!(i.step(0, 1.0), 0.0);
    }

    #[test]
    fn history_reads_back() {
        let mut h = History::new(4);
        for s in [1.0, 2.0, 3.0] {
            h.push(s);
        }
        assert_eq!(h.last(1.0), 3.0);
        assert_eq!(h.last(2.0), 2.0);
        assert_eq!(h.last(3.0), 1.0);
        assert_eq!(h.last(0.0), 0.0);
        assert_eq!(h.last(5.0), 0.0);
        assert_eq!(h.last(f32::NAN), 0.0);
        // NaN is not stored
        h.push(f32::NAN);
        assert_eq!(h.last(1.0), 0.0);
        assert_eq!(h.last(2.0), 3.0);
    }

    #[test]
    fn rand_is_in_range_and_reproducible() {
        let mut a = Rand::new(1);
        let mut b = Rand::new(1);
        for _ in 0..1000 {
            let x = a.next_f32();
            assert!((-1.0..1.0).contains(&x));
            assert_eq!(x, b.next_f32());
        }
    }

    #[test]
    fn the_legacy_table_registers_in_order() {
        let (s, v) = symbols();
        assert_eq!(v.t, VarId(0));
        assert_eq!(v.srate, VarId(10));
        assert_eq!(s.var_count(), Vars::COUNT);
        assert_eq!(s.fn_count(), 19);
        assert_eq!(kind(s.function("integrate").unwrap()), Some(Kind::Integrate));
        assert_eq!(fn_id(Kind::W3), s.function("w3").unwrap());
        assert_eq!(s.var("a2"), Some(v.a2));
    }

    #[test]
    fn a_whole_expression_through_the_voice() {
        let (s, v) = symbols();
        let expr = Expr::compile("sinew(t * f) * A1 + integrate(1) + W2(t) + semitone(key - 69)", &s).unwrap();
        assert!(expr.has_side_effects());
        assert_eq!(expr.call_count(fn_id(Kind::Integrate)), 1);
        let mut integrate = Integrate::new(1, 44100.0);
        let history = History::new(16);
        let mut rand = Rand::new(1);
        let w = [0.0, 0.5, 1.0, 0.5];
        let mut voice = Voice {
            integrate: &mut integrate,
            history: &history,
            rand: &mut rand,
            random_seed: 7,
            waves: [&w, &w, &w],
            interpolate: [false, true, false],
        };
        let mut vars = [0.0_f32; Vars::COUNT];
        vars[v.t.index()] = 0.1;
        vars[v.f.index()] = 2.5;
        vars[v.a1.index()] = 0.5;
        vars[v.key.index()] = 69.0;
        let mut stack = vec![0.0; expr.stack_len()];
        let y = expr.eval(&mut stack, &vars, &mut voice);
        // sinew(0.25) * 0.5 + 0 + W2(0.1) + 1
        let want = sinew(0.25) * 0.5 + 0.0 + wave_value_interpolated(&w, 0.1) + 1.0;
        assert!((y - want).abs() < 1e-5, "{y} vs {want}");
        // The integral advances by 1 per sample
        let y2 = expr.eval(&mut stack, &vars, &mut voice);
        assert!((y2 - y - 1.0 / 44100.0).abs() < 1e-6);
    }
}
