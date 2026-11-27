//! The legacy graph tools (`graphModel` in legado/src/gui/widgets/Graph.cpp) as functions over a graph's samples:
//! the preset shapes (`setWaveToSine`…, from `Oscillator`'s wave functions), smoothing, normalizing, inverting and
//! shifting the phase. The panels' push buttons call them on the graph's field (BitInvader, Watsyn, Vibed…).

/// `absFraction`: the wrapped fraction in [0, 1)
fn abs_fraction(x: f64) -> f64 {
    x - x.floor()
}

/// `Oscillator::sinSample`
pub fn sine(x: f64) -> f64 {
    (x * 2.0 * std::f64::consts::PI).sin()
}

/// `Oscillator::triangleSample`
pub fn triangle(x: f64) -> f64 {
    let ph = abs_fraction(x);
    if ph <= 0.25 {
        ph * 4.0
    } else if ph <= 0.75 {
        2.0 - ph * 4.0
    } else {
        ph * 4.0 - 4.0
    }
}

/// `Oscillator::sawSample`
pub fn saw(x: f64) -> f64 {
    -1.0 + abs_fraction(x) * 2.0
}

/// `Oscillator::squareSample`
pub fn square(x: f64) -> f64 {
    if abs_fraction(x) > 0.5 { -1.0 } else { 1.0 }
}

/// `Oscillator::moogSawSample`
pub fn moog_saw(x: f64) -> f64 {
    let ph = abs_fraction(x);
    if ph < 0.5 { -1.0 + ph * 4.0 } else { 1.0 - 2.0 * ph }
}

/// `Oscillator::expSample`
pub fn exponential(x: f64) -> f64 {
    let mut ph = abs_fraction(x);
    if ph > 0.5 {
        ph = 1.0 - ph;
    }
    -1.0 + 8.0 * ph * ph
}

/// `graphModel::setWaveTo<shape>`: the first `n` samples from a wave function of the phase `i / n`
pub fn fill(samples: &mut [f64], n: usize, wave: fn(f64) -> f64) {
    let n = n.min(samples.len());
    for (i, s) in samples.iter_mut().take(n).enumerate() {
        *s = wave(i as f64 / n as f64);
    }
}

/// `graphModel::setWaveToNoise`: `fastRandInc(-1, 1)` per sample
pub fn noise(samples: &mut [f64], n: usize, rand: &mut dyn FnMut() -> f64) {
    for s in samples.iter_mut().take(n) {
        *s = rand() * 2.0 - 1.0;
    }
}

/// `graphModel::smooth`: [1 2 1] / 4, cyclic
pub fn smooth(samples: &mut [f64], n: usize) {
    let n = n.min(samples.len());
    if n < 2 {
        return;
    }
    let t = samples[..n].to_vec();
    samples[0] = (t[n - 1] + t[0] * 2.0 + t[1]) * 0.25;
    for i in 1..n - 1 {
        samples[i] = (t[i - 1] + t[i] * 2.0 + t[i + 1]) * 0.25;
    }
    samples[n - 1] = (t[n - 2] + t[n - 1] * 2.0 + t[0]) * 0.25;
}

/// `graphModel::smoothNonCyclic`: the ends kept
pub fn smooth_non_cyclic(samples: &mut [f64], n: usize) {
    let n = n.min(samples.len());
    if n < 3 {
        return;
    }
    let t = samples[..n].to_vec();
    for i in 1..n - 1 {
        samples[i] = (t[i - 1] + t[i] * 2.0 + t[i + 1]) * 0.25;
    }
}

/// `graphModel::normalize`: the DC removed, then the peak to the range's edge
pub fn normalize(samples: &mut [f64], n: usize, min: f64, max: f64) {
    let n = n.min(samples.len());
    if n == 0 {
        return;
    }
    let avg = samples[..n].iter().sum::<f64>() / n as f64;
    for s in samples.iter_mut().take(n) {
        *s -= avg;
    }
    let peak = samples[..n].iter().fold(0.0001f64, |m, s| m.max(s.abs()));
    for s in samples.iter_mut().take(n) {
        *s = crate::widgets::fit_between(*s / peak, min, max);
    }
}

/// `graphModel::invert`: mirrored in the range
pub fn invert(samples: &mut [f64], n: usize, min: f64, max: f64) {
    let range = max - min;
    for s in samples.iter_mut().take(n) {
        *s = min + (range - (*s - min));
    }
}

/// `graphModel::shiftPhase`: by `deg` degrees (the offset in whole samples, multiplied first as the legacy)
pub fn shift_phase(samples: &mut [f64], n: usize, deg: i32) {
    let n = n.min(samples.len());
    if n == 0 {
        return;
    }
    let offset = (deg as i64 * n as i64) / 360;
    let t = samples[..n].to_vec();
    for (i, s) in samples.iter_mut().take(n).enumerate() {
        let o = (i as i64 + offset).rem_euclid(n as i64) as usize;
        *s = t[o];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shapes_are_the_oscillators() {
        let mut s = vec![0.0; 8];
        fill(&mut s, 8, sine);
        assert!((s[2] - 1.0).abs() < 1e-12 && s[0].abs() < 1e-12);
        fill(&mut s, 8, square);
        assert_eq!(s, vec![1.0, 1.0, 1.0, 1.0, 1.0, -1.0, -1.0, -1.0]);
        fill(&mut s, 8, triangle);
        assert_eq!(s[2], 1.0);
        assert_eq!(s[6], -1.0);
    }

    #[test]
    fn smooth_normalize_invert_shift() {
        let mut s = vec![0.0, 1.0, 0.0, -1.0];
        smooth(&mut s, 4);
        assert_eq!(s, vec![0.0, 0.5, 0.0, -0.5]);
        normalize(&mut s, 4, -1.0, 1.0);
        assert_eq!(s, vec![0.0, 1.0, 0.0, -1.0]);
        invert(&mut s, 4, -1.0, 1.0);
        assert_eq!(s, vec![0.0, -1.0, 0.0, 1.0]);
        shift_phase(&mut s, 4, 90);
        assert_eq!(s, vec![-1.0, 0.0, 1.0, 0.0]);
    }
}
