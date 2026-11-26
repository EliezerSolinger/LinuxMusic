//! Tap Tempo (F-JAN-023, F-JAN-046): the tempo from the user's taps, the average of the last eight intervals; a
//! pause of more than 2 s starts again. Pure: the caller gives the clock (seconds), so it is tested without one.

/// How many intervals are averaged
const WINDOW: usize = 8;
/// A pause longer than this starts a new count (seconds)
const RESET: f64 = 2.0;

#[derive(Clone, Debug, Default)]
pub struct TapTempo {
    taps: Vec<f64>,
}

impl TapTempo {
    /// A tap at `now` seconds: the tempo so far, once there are two taps
    pub fn tap(&mut self, now: f64) -> Option<f64> {
        if self.taps.last().is_some_and(|&last| now - last > RESET || now < last) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > WINDOW + 1 {
            self.taps.remove(0);
        }
        let n = self.taps.len();
        if n < 2 {
            return None;
        }
        let mean = (self.taps[n - 1] - self.taps[0]) / (n - 1) as f64;
        (mean > 0.0).then(|| (60.0 / mean).clamp(20.0, 400.0))
    }

    /// How many taps count now (the button shows it)
    pub fn count(&self) -> usize {
        self.taps.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_tap_says_nothing_then_the_mean_of_the_intervals() {
        let mut t = TapTempo::default();
        assert_eq!(t.tap(0.0), None);
        let bpm = t.tap(0.5).unwrap();
        assert!((bpm - 120.0).abs() < 1e-9);
        let bpm = t.tap(1.1).unwrap();
        assert!((bpm - 60.0 / 0.55).abs() < 1e-9);
    }

    #[test]
    fn a_long_pause_starts_again() {
        let mut t = TapTempo::default();
        t.tap(0.0);
        t.tap(0.5);
        assert_eq!(t.tap(10.0), None);
        assert_eq!(t.count(), 1);
    }

    #[test]
    fn only_the_last_eight_intervals_count() {
        let mut t = TapTempo::default();
        t.tap(0.0);
        t.tap(1.5);
        let mut bpm = 0.0;
        for i in 0..9 {
            bpm = t.tap(1.5 + 0.5 * (i + 1) as f64).unwrap();
        }
        assert!((bpm - 120.0).abs() < 1e-9);
        assert_eq!(t.count(), 9);
    }

    #[test]
    fn the_tempo_stays_in_range() {
        let mut t = TapTempo::default();
        t.tap(0.0);
        assert_eq!(t.tap(1.9).map(|b| b.round()), Some(32.0));
        let mut t = TapTempo::default();
        t.tap(0.0);
        assert_eq!(t.tap(0.01), Some(400.0));
    }
}
