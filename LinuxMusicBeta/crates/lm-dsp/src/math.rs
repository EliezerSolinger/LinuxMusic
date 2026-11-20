//! The math the DSP uses: the pure `libm` crate, so that a render gives the same samples on every distribution
//! (interfaces.md, V2), and the level conversions.

pub use core::f32::consts as f32c;
pub use core::f64::consts as f64c;

// Single precision
#[inline]
pub fn expf(x: f32) -> f32 {
    libm::expf(x)
}
#[inline]
pub fn logf(x: f32) -> f32 {
    libm::logf(x)
}
#[inline]
pub fn log10f(x: f32) -> f32 {
    libm::log10f(x)
}
#[inline]
pub fn log1pf(x: f32) -> f32 {
    libm::log1pf(x)
}
#[inline]
pub fn log2f(x: f32) -> f32 {
    libm::log2f(x)
}
#[inline]
pub fn exp2f(x: f32) -> f32 {
    libm::exp2f(x)
}
#[inline]
pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}
#[inline]
pub fn tanhf(x: f32) -> f32 {
    libm::tanhf(x)
}
#[inline]
pub fn sinf(x: f32) -> f32 {
    libm::sinf(x)
}
#[inline]
pub fn cosf(x: f32) -> f32 {
    libm::cosf(x)
}
#[inline]
pub fn tanf(x: f32) -> f32 {
    libm::tanf(x)
}

// Double precision
#[inline]
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}
#[inline]
pub fn exp2(x: f64) -> f64 {
    libm::exp2(x)
}
#[inline]
pub fn log(x: f64) -> f64 {
    libm::log(x)
}
#[inline]
pub fn log1p(x: f64) -> f64 {
    libm::log1p(x)
}
#[inline]
pub fn pow(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}
#[inline]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}
#[inline]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}
#[inline]
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}
#[inline]
pub fn cosh(x: f64) -> f64 {
    libm::cosh(x)
}
#[inline]
pub fn hypot(x: f64, y: f64) -> f64 {
    libm::hypot(x, y)
}

/// Linear amplitude to dBFS, 1.0 being 0 dBFS (the legacy `ampToDbfs`: `ln(x) / ln 10 * 20` in single precision)
#[inline]
pub fn amp_to_dbfs(amplitude: f32) -> f32 {
    logf(amplitude) * (1.0 / f64c::LN_10) as f32 * 20.0
}

/// dBFS to linear amplitude (the legacy `dbfsToAmp`: `e^(ln 10 * dB / 20)` in single precision)
#[inline]
pub fn dbfs_to_amp(dbfs: f32) -> f32 {
    expf(f32c::LN_10 * (dbfs * 0.05))
}

/// The one-pole smoothing coefficient for a time constant of @p seconds at @p rate: `1 - e^(-1 / (s rate))`, or 1
/// (no smoothing) for no time
#[inline]
pub fn one_pole(seconds: f32, rate: f32) -> f32 {
    if seconds > 0.0 { 1.0 - expf(-1.0 / (seconds * rate)) } else { 1.0 }
}

/// A complex number, enough for evaluating filter responses
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const ONE: Complex = Complex { re: 1.0, im: 0.0 };

    pub fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    /// `e^(i angle)` times @p magnitude
    pub fn polar(magnitude: f64, angle: f64) -> Self {
        Self { re: magnitude * cos(angle), im: magnitude * sin(angle) }
    }
    pub fn abs(self) -> f64 {
        hypot(self.re, self.im)
    }
}

impl core::ops::Add for Complex {
    type Output = Complex;
    fn add(self, o: Complex) -> Complex {
        Complex::new(self.re + o.re, self.im + o.im)
    }
}
impl core::ops::Add<f64> for Complex {
    type Output = Complex;
    fn add(self, o: f64) -> Complex {
        Complex::new(self.re + o, self.im)
    }
}
impl core::ops::Sub<f64> for Complex {
    type Output = Complex;
    fn sub(self, o: f64) -> Complex {
        Complex::new(self.re - o, self.im)
    }
}
impl core::ops::AddAssign for Complex {
    fn add_assign(&mut self, o: Complex) {
        *self = *self + o;
    }
}
impl core::ops::Mul for Complex {
    type Output = Complex;
    fn mul(self, o: Complex) -> Complex {
        Complex::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}
impl core::ops::Mul<Complex> for f64 {
    type Output = Complex;
    fn mul(self, o: Complex) -> Complex {
        Complex::new(self * o.re, self * o.im)
    }
}
impl core::ops::Div for Complex {
    type Output = Complex;
    fn div(self, o: Complex) -> Complex {
        let d = o.re * o.re + o.im * o.im;
        Complex::new((self.re * o.re + self.im * o.im) / d, (self.im * o.re - self.re * o.im) / d)
    }
}

/// 4-point Hermite interpolation between @p x1 and @p x2 (the legacy `hermiteInterpolate`)
#[inline]
pub fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, frac: f32) -> f32 {
    let frsq = frac * frac;
    let frsq2 = 2.0 * frsq;
    ((x2 - x0) * 0.5) * (frac * (frsq + 1.0) - frsq2)
        + (frsq2 * frac - 3.0 * frsq) * (x1 - x2)
        + frsq2 * (frac - 1.0) * ((x3 - x1) * 0.25)
        + x1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_round_trip() {
        for db in [-60.0f32, -18.0, -6.0, 0.0, 6.0, 24.0] {
            assert!((amp_to_dbfs(dbfs_to_amp(db)) - db).abs() < 1e-4);
        }
        assert!((dbfs_to_amp(-6.0) - 0.501_187).abs() < 1e-5);
    }
}
