//! What every page draws the same way: the layout sizes, the timeline's scroll and zoom (in ticks), the knobs'
//! mapping, the volume in dB, the meters' scale and fall, the transport's texts.

use lm_port::{ParamRange, RangeScale};
use lm_time::TempoMap;

pub const WINDOW_W: f32 = 1600.0;
pub const WINDOW_H: f32 = 980.0;
pub const TRANSPORT_H: f32 = 56.0;
pub const RULER_H: f32 = 22.0;
/// Design 02, normal height
pub const TRACK_H: f32 = 70.0;
pub const HEADER_W: f32 = 300.0;

/// 400 and 4 pixels per bar (3840 ticks): the zoom's limits (F-ARR-008)
pub const MIN_TPP: f64 = 9.6;
pub const MAX_TPP: f64 = 960.0;

/// The timeline's scroll and zoom, in ticks (the interface's document would hold it, I-3.8)
#[derive(Debug, Clone, Copy)]
pub struct TimelineView {
    pub scroll_ticks: f64,
    pub ticks_per_px: f64,
    /// pixels scrolled down in the track list
    pub scroll_y: f64,
}

impl Default for TimelineView {
    fn default() -> Self {
        TimelineView { scroll_ticks: 0.0, ticks_per_px: 24.0, scroll_y: 0.0 }
    }
}

impl TimelineView {
    pub fn x(&self, tick: f64) -> f32 {
        ((tick - self.scroll_ticks) / self.ticks_per_px) as f32
    }

    pub fn tick_at(&self, x: f32) -> f64 {
        self.scroll_ticks + x as f64 * self.ticks_per_px
    }

    /// Wheel or trackpad: sideways, down, or zoom around the pointer
    pub fn wheel(&mut self, dx: f64, dy: f64, zoom: bool, pointer_x: f64, content_h: f64, view_h: f64) {
        if zoom {
            let factor = (dy * 0.01).exp();
            let anchor = self.scroll_ticks + pointer_x * self.ticks_per_px;
            self.ticks_per_px = (self.ticks_per_px * factor).clamp(MIN_TPP, MAX_TPP);
            self.scroll_ticks = (anchor - pointer_x * self.ticks_per_px).max(0.0);
        } else {
            self.scroll_ticks = (self.scroll_ticks - dx * self.ticks_per_px).max(0.0);
            self.scroll_y = (self.scroll_y - dy).clamp(0.0, (content_h - view_h).max(0.0));
        }
    }
}

/// A linear gain (the document's `volume`, 0..4) as dB
pub fn gain_to_db(g: f64) -> f64 {
    if g <= 0.0 { f64::NEG_INFINITY } else { 20.0 * g.log10() }
}

pub fn db_to_gain(db: f64) -> f64 {
    if db <= -60.0 { 0.0 } else { 10f64.powf(db / 20.0) }
}

/// The fader's and the volume knob's law: the core's own (`RangeScale::Gain`, the legacy fader's
/// computeScaledRatio, 0 dB at ~75 %), so a knob on screen, a mapped controller and an automation lane agree
const FADER: ParamRange = ParamRange { min: 0.0, max: 4.0, scale: RangeScale::Gain };

/// Volume knob and fader: 0..1 of travel <-> dB (-inf at the bottom, +12 dB at the top)
pub fn travel_to_db(t: f64) -> f64 {
    gain_to_db(travel_to_gain(t))
}

pub fn db_to_travel(db: f64) -> f64 {
    gain_to_travel(if db == f64::NEG_INFINITY { 0.0 } else { 10f64.powf(db / 20.0) })
}

pub fn gain_to_travel(g: f64) -> f64 {
    FADER.normalize(g.max(0.0) as f32) as f64
}

pub fn travel_to_gain(t: f64) -> f64 {
    if t <= 0.0 { 0.0 } else { FADER.to_units(t.clamp(0.0, 1.0) as f32) as f64 }
}

/// Dragging a knob: 200 px of vertical travel for the whole range; Shift for fine
pub fn knob_drag(start: f64, dy_px: f64, fine: bool) -> f64 {
    (start - dy_px / if fine { 2000.0 } else { 200.0 }).clamp(0.0, 1.0)
}

pub fn format_db(db: f64) -> String {
    if db.is_infinite() || db <= -60.0 { "-inf dB".into() } else { format!("{db:+.1} dB") }
}

pub fn format_gain(g: f64) -> String {
    format_db(gain_to_db(g))
}

/// Meter value (linear peak) to the bar's fill 0..1, on a -60..0 dB scale
pub fn meter_fill(peak: f32) -> f32 {
    if peak <= 0.001 { 0.0 } else { ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0) }
}

/// What a channel's meter reads (the preference `interface.channelMeter`): the peak, or the RMS with the ballistics
/// of a VU meter or of a K-system meter (K-20, K-14, K-12)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeterMode {
    Peak,
    Rms,
    Vu,
    K20,
    K14,
    K12,
}

impl MeterMode {
    pub const ALL: [MeterMode; 6] =
        [MeterMode::Peak, MeterMode::Rms, MeterMode::Vu, MeterMode::K20, MeterMode::K14, MeterMode::K12];

    /// The mode a preference value names (anything else reads the peak)
    pub fn from_key(key: &str) -> MeterMode {
        Self::ALL.into_iter().find(|m| m.key() == key).unwrap_or(MeterMode::Peak)
    }

    pub fn key(self) -> &'static str {
        match self {
            MeterMode::Peak => "peak",
            MeterMode::Rms => "rms",
            MeterMode::Vu => "vu",
            MeterMode::K20 => "k20",
            MeterMode::K14 => "k14",
            MeterMode::K12 => "k12",
        }
    }

    /// The short name on the desk
    pub fn label(self) -> &'static str {
        match self {
            MeterMode::Peak => "PEAK",
            MeterMode::Rms => "RMS",
            MeterMode::Vu => "VU",
            MeterMode::K20 => "K-20",
            MeterMode::K14 => "K-14",
            MeterMode::K12 => "K-12",
        }
    }

    /// The next one in the cycle
    pub fn next(self) -> MeterMode {
        let at = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// Which smoothing of the mean square it reads (`Core::rms_sides`): 0 for RMS, 1 for VU, 2 for K
    pub fn window(self) -> usize {
        match self {
            MeterMode::Peak | MeterMode::Rms => 0,
            MeterMode::Vu => 1,
            _ => 2,
        }
    }

    /// Added to the level in dBFS to read the mode's unit: 0 VU is -18 dBFS; the K meters read the RMS of a sine
    /// (3 dB above its mean square) from 0 at -20, -14 or -12 dBFS
    pub fn offset(self) -> f32 {
        match self {
            MeterMode::Peak | MeterMode::Rms => 0.0,
            MeterMode::Vu => 18.0,
            MeterMode::K20 => 23.0,
            MeterMode::K14 => 17.0,
            MeterMode::K12 => 15.0,
        }
    }

    /// Whether the reading is past the mode's 0 (red): the peak at full scale, the VU and K meters above their zero
    pub fn over(self, shown_db: f32) -> bool {
        match self {
            MeterMode::Peak => shown_db > -0.1,
            MeterMode::Rms => shown_db > -3.0,
            _ => shown_db > 0.0,
        }
    }
}

/// The time constants (s) of the three mean-square smoothings: RMS (300 ms), VU (300 ms to 99 %) and K (600 ms)
pub const RMS_TAUS: [f32; 3] = [0.3, 0.065, 0.6];

/// The bar falls at ~20 dB/s and rises at once
pub fn meter_ballistics(shown: f32, target: f32, dt: f32) -> f32 {
    if target >= shown { target } else { (shown - dt * 20.0 / 60.0).max(target) }
}

/// The big display: bars.beats.ticks from the tempo map (ticks shown in 1/192 of a beat, as design 03)
pub fn format_bbt(map: &TempoMap, tick: i64) -> String {
    let b = map.bar_beat_tick(tick.max(0));
    let ppq = map.ppq().get() as i64;
    format!("{:03}.{}.{:03}", b.bar, b.beat, (b.tick as i64 * 192 / ppq.max(1)))
}

pub fn format_clock(samples: f64, rate: u32) -> String {
    let s = samples.max(0.0) / rate.max(1) as f64;
    format!("{}:{:02}.{:03}", (s / 60.0).floor() as i64, (s % 60.0).floor() as i64, (s.fract() * 1000.0).floor() as i64)
}

pub fn format_timecode(samples: f64, rate: u32) -> String {
    let s = samples.max(0.0) / rate.max(1) as f64;
    let f = (s.fract() * 25.0).floor();
    format!("{:02}:{:02}:{:02}:{:02}", (s / 3600.0) as i64, (s / 60.0) as i64 % 60, s as i64 % 60, f as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_modes_cycle_and_read_from_their_own_zero() {
        let mut m = MeterMode::Peak;
        for _ in 0..MeterMode::ALL.len() {
            assert_eq!(MeterMode::from_key(m.key()), m);
            m = m.next();
        }
        assert_eq!(m, MeterMode::Peak);
        assert_eq!(MeterMode::from_key("nonsense"), MeterMode::Peak);
        // -18 dBFS of RMS is 0 VU; -20 dBFS of sine RMS is 0 on K-20
        assert!(!MeterMode::Vu.over(-18.0 + MeterMode::Vu.offset()));
        assert!(MeterMode::K20.over(-17.0 + MeterMode::K20.offset() - 3.0 + 0.1));
    }

    #[test]
    fn the_knob_law_round_trips() {
        // the core's f32 law: round trips within a hundredth of a dB
        for db in [-48.0, -12.0, -3.0, 0.0, 6.0, 12.0] {
            assert!((travel_to_db(db_to_travel(db)) - db).abs() < 1e-2, "{db}");
        }
        // the legacy fader's numbers (Fader::computeScaledRatio, max 4)
        assert!((gain_to_travel(1.0) - 0.751).abs() < 1e-3, "0 dB at ~75 %");
        assert!((db_to_travel(-6.0) - 0.644).abs() < 1e-3);
        assert_eq!(travel_to_gain(0.0), 0.0);
    }

    #[test]
    fn zoom_keeps_the_pointer_still() {
        let mut v = TimelineView::default();
        let before = v.tick_at(300.0);
        v.wheel(0.0, -50.0, true, 300.0, 1000.0, 500.0);
        assert!((v.tick_at(300.0) - before).abs() < 1e-6);
    }
}
