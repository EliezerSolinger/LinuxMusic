//! What every prototype draws the same way: the theme as data, the layout sizes, the view math (scroll, zoom), the
//! knob mapping and the transport's text. Shared so the three differ only in what the library does.

use std::time::Instant;

/// Theme as data (spec item 12): the colors of design 02/03 (`design-sugestoes/fontes/base.css`), as 0xRRGGBB
#[derive(Debug, Clone)]
pub struct Theme {
    pub window: u32,
    pub bg1: u32,
    pub bg2: u32,
    pub bg3: u32,
    pub bg4: u32,
    pub line: u32,
    pub line2: u32,
    pub text: u32,
    pub text2: u32,
    pub text3: u32,
    pub green: u32,
    pub red: u32,
    pub orange: u32,
    pub yellow: u32,
    pub blue: u32,
    pub lcd_bg: u32,
    pub cursor: u32,
}

pub const THEME: Theme = Theme {
    window: 0x0d0f11,
    bg1: 0x1d2125,
    bg2: 0x262b30,
    bg3: 0x30363d,
    bg4: 0x3b424a,
    line: 0x0f1113,
    line2: 0x454d56,
    text: 0xd6dde6,
    text2: 0x98a2ae,
    text3: 0x6c7681,
    green: 0x2fd16f,
    red: 0xe8453c,
    orange: 0xf0a33a,
    yellow: 0xf2d73a,
    blue: 0x3d8be8,
    lcd_bg: 0x0b0d0f,
    cursor: 0xf2f5f8,
};

/// Track colors (design 02: the track number column takes the track's color)
pub const TRACK_COLORS: [u32; 8] = [0xe8453c, 0x9b6be8, 0x3d8be8, 0x2fd16f, 0xf0a33a, 0x3dc8d8, 0xf2d73a, 0xe86bb4];

// Layout, in logical pixels
pub const WINDOW_W: f32 = 1600.0;
pub const WINDOW_H: f32 = 980.0;
pub const TRANSPORT_H: f32 = 56.0;
pub const RULER_H: f32 = 22.0;
/// Design 02, normal height
pub const TRACK_H: f32 = 70.0;
pub const HEADER_W: f32 = 300.0;
pub const MIXER_H: f32 = 300.0;
pub const STRIP_W: f32 = 64.0;

pub const MIN_SPP: f64 = 16.0;
pub const MAX_SPP: f64 = 65_536.0;

/// The timeline's scroll and zoom. Lives in the GUI (the UI document of I-3.8 would hold it)
#[derive(Debug, Clone, Copy)]
pub struct TimelineView {
    pub scroll_samples: f64,
    pub samples_per_px: f64,
    /// pixels scrolled down in the track list
    pub scroll_y: f64,
}

impl Default for TimelineView {
    fn default() -> Self {
        TimelineView { scroll_samples: 0.0, samples_per_px: 1024.0, scroll_y: 0.0 }
    }
}

impl TimelineView {
    pub fn sample_to_x(&self, sample: f64) -> f64 {
        (sample - self.scroll_samples) / self.samples_per_px
    }

    /// Wheel or trackpad: horizontal pan, vertical scroll, or zoom around the pointer (Ctrl/Cmd held)
    pub fn wheel(&mut self, dx: f64, dy: f64, zoom: bool, pointer_x: f64, content_h: f64, view_h: f64) {
        if zoom {
            let factor = (dy * 0.01).exp(); // smooth for trackpads, a notch is ~1.6x
            let anchor = self.scroll_samples + pointer_x * self.samples_per_px;
            self.samples_per_px = (self.samples_per_px * factor).clamp(MIN_SPP, MAX_SPP);
            self.scroll_samples = (anchor - pointer_x * self.samples_per_px).max(0.0);
        } else {
            self.scroll_samples = (self.scroll_samples - dx * self.samples_per_px).max(0.0);
            self.scroll_y = (self.scroll_y - dy).clamp(0.0, (content_h - view_h).max(0.0));
        }
    }

    /// The first visible track and how many are visible
    pub fn visible_tracks(&self, view_h: f64, count: usize) -> (usize, usize) {
        let first = (self.scroll_y / TRACK_H as f64).floor().max(0.0) as usize;
        let n = (view_h / TRACK_H as f64).ceil() as usize + 1;
        (first.min(count), n.min(count.saturating_sub(first)))
    }
}

/// Volume knob: 0..1 of travel <-> dB, -60 dB (shown as -inf at the bottom) to +6 dB, 0 dB at 75 %
pub fn knob_to_db(t: f64) -> f64 {
    if t <= 0.0 { f64::NEG_INFINITY } else if t <= 0.75 { -60.0 + 60.0 * (t / 0.75) } else { (t - 0.75) / 0.25 * 6.0 }
}
pub fn db_to_knob(db: f64) -> f64 {
    if db <= -60.0 { 0.0 } else if db <= 0.0 { 0.75 * (db + 60.0) / 60.0 } else { 0.75 + 0.25 * db / 6.0 }
}
/// Dragging a knob: 200 px of vertical travel for the whole range; Shift for fine
pub fn knob_drag(start: f64, dy_px: f64, fine: bool) -> f64 {
    (start - dy_px / if fine { 2000.0 } else { 200.0 }).clamp(0.0, 1.0)
}
pub fn format_db(db: f64) -> String {
    if db.is_infinite() { "-inf dB".into() } else { format!("{db:+.1} dB") }
}

/// Meter value (linear peak 0..1) to the bar's fill 0..1, on a -60..0 dB scale
pub fn meter_fill(peak: f32) -> f32 {
    if peak <= 0.001 { 0.0 } else { ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0) }
}

/// The big display of design 03: bars.beats.ticks (4/4, 120 BPM fixed in the fake) and m:ss.mmm
pub fn format_bbt(sample: f64, sample_rate: f64) -> String {
    let beats = (sample.max(0.0) / sample_rate) * 2.0; // 120 BPM
    let bar = (beats / 4.0).floor() as i64 + 1;
    let beat = (beats % 4.0).floor() as i64 + 1;
    let tick = ((beats.fract()) * 192.0).floor() as i64;
    format!("{bar:03}.{beat}.{tick:03}")
}
pub fn format_clock(sample: f64, sample_rate: f64) -> String {
    let s = sample.max(0.0) / sample_rate;
    format!("{}:{:02}.{:03}", (s / 60.0).floor() as i64, (s % 60.0).floor() as i64, ((s.fract()) * 1000.0).floor() as i64)
}

/// The tiny translation table of the prototype (spec item 15): keys in English, Portuguese shown. Slint and Qt use
/// their own mechanism (`@tr`, `qsTr`) where they have one; GPUI has none and would use this kind of table.
pub fn tr(key: &str) -> &str {
    match key {
        "Stopped" => "PARADO",
        "Playing" => "TOCANDO",
        "Recording" => "GRAVANDO",
        "Loading" => "Carregando",
        "Mixer" => "Mesa",
        "Track" => "Faixa",
        "Reload" => "Recarregar take",
        other => other,
    }
}

/// Frame-to-frame smoothing for a meter's fall (the bar falls at ~20 dB/s, rises at once)
pub fn meter_ballistics(shown: f32, target: f32, dt: f32) -> f32 {
    if target >= shown { target } else { (shown - dt * 20.0 / 60.0).max(target) }
}

pub fn now() -> Instant {
    Instant::now()
}
