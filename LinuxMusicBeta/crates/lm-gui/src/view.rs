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

pub const MIN_TPP: f64 = 0.25;
pub const MAX_TPP: f64 = 4096.0;

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
