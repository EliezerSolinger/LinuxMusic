//! The LMMS widgets, drawn as the legacy draws them (legado/src/gui/widgets): the knobs in their five types
//! (`Knob.cpp`: `Dark28`, `Bright26`, `Small17`, `Vintage32` on the theme's pixmaps, and `Styled`, a line painted
//! with the plugin's style-sheet values), the LED check box (`LedCheckBox.cpp`), the pixmap button
//! (`PixmapButton.cpp`) and the LCD spin box (`LcdWidget.cpp`). Each widget is rasterized per state into one image
//! (`raster`), cached by the state that changes its pixels (the knob's angle in whole degrees, as the legacy cache).
//!
//! The value model is the legacy `AutomatableModel`'s: range, step, center value, and the "logarithmic" scale,
//! which in the legacy is the power curve `linearToLogScale`/`logToLinearScale` (a knob's travel, not the value).

use crate::assets::{self, Asset};
use crate::raster::{Canvas, Cap, Paint, Rgba};

/// The default theme's palette and the knobs' style-sheet values (legado/data/themes/default/style.css)
pub mod theme {
    use crate::raster::Rgba;

    /// `LmmsPalette` windowText: the line and the arc of the pixmap knobs
    pub const WINDOW_TEXT: Rgba = Rgba::hex(0x1de276);
    /// `LmmsPalette` shadow: the line and the arc of the `Vintage32` knob
    pub const SHADOW: Rgba = Rgba::rgb(0, 0, 0);
    /// `Knob { qproperty-lineInactiveColor; qproperty-arcInactiveColor }`
    pub const LINE_INACTIVE: Rgba = Rgba::rgb(120, 120, 120);
    pub const ARC_INACTIVE: Rgba = Rgba::alpha(120, 120, 120, 70);
    /// `TextFloat, SimpleTextFloat { background; color }`
    pub const FLOAT_BG: u32 = 0x040506;
    pub const FLOAT_TEXT: u32 = 0xd1d8e4;
    /// QMenu (the controls' context menu)
    pub const MENU_BG: u32 = 0x262b30;
    pub const MENU_TEXT: u32 = 0xd1d8e4;
    pub const MENU_DISABLED: u32 = 0x6d7686;
    pub const MENU_HOVER: u32 = 0x17793b;
    pub const MENU_CAPTION: u32 = 0x3f4750;
    /// The value entry (QInputDialog on the default theme)
    pub const DIALOG_BG: u32 = 0x262b30;
    pub const DIALOG_FIELD: u32 = 0x101213;
    pub const DIALOG_BORDER: u32 = 0x3f4750;
    pub const DIALOG_TEXT: u32 = 0xd1d8e4;
    /// `DEFAULT_FONT_SIZE` and `SMALL_FONT_SIZE` (include/FontHelper.h), in pixels
    pub const DEFAULT_FONT_PX: f32 = 12.0;
    pub const SMALL_FONT_PX: f32 = 10.0;
}

/// A knob type (`KnobType`)
#[derive(Clone, Debug, PartialEq)]
pub enum KnobStyle {
    /// `knob01.png`, 29×29
    Dark28,
    /// `knob02.png`, 27×27
    Bright26,
    /// `knob03.png`, 19×19
    Small17,
    /// `knob05.png`, 32×32
    Vintage32,
    Styled(Styled),
    /// Guitarix's knobs, vector drawn with their LED ring (`gxwidgets`)
    Gx(crate::gxwidgets::GxKnob),
}

/// A `KnobType::Styled` knob: its size (set by the plugin) and the style sheet's properties
#[derive(Clone, Debug, PartialEq)]
pub struct Styled {
    pub w: f32,
    pub h: f32,
    /// `color`: the gradient's inner color (the widget's text color)
    pub color: Rgba,
    /// `qproperty-outerColor` (absent: a plain black pen)
    pub outer: Option<Rgba>,
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub center: (f32, f32),
    pub line_width: f32,
}

impl KnobStyle {
    fn pixmap(&self) -> Option<&'static Asset> {
        match self {
            KnobStyle::Dark28 => Some(&assets::lmms::KNOB01),
            KnobStyle::Bright26 => Some(&assets::lmms::KNOB02),
            KnobStyle::Small17 => Some(&assets::lmms::KNOB03),
            KnobStyle::Vintage32 => Some(&assets::lmms::KNOB05),
            KnobStyle::Styled(_) | KnobStyle::Gx(_) => None,
        }
    }

    /// The knob's drawn size (without its label)
    pub fn size(&self) -> (f32, f32) {
        match self {
            KnobStyle::Styled(s) => (s.w, s.h),
            KnobStyle::Gx(g) => g.size(),
            other => {
                let img = other.pixmap().map(assets::image);
                img.map_or((1.0, 1.0), |i| (i.width() as f32, i.height() as f32))
            }
        }
    }

    /// A number that names the style in the image cache
    pub fn cache_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        match self {
            KnobStyle::Dark28 => 1u8.hash(&mut h),
            KnobStyle::Bright26 => 2u8.hash(&mut h),
            KnobStyle::Small17 => 3u8.hash(&mut h),
            KnobStyle::Vintage32 => 4u8.hash(&mut h),
            KnobStyle::Gx(g) => {
                6u8.hash(&mut h);
                g.hash(&mut h);
            }
            KnobStyle::Styled(s) => {
                5u8.hash(&mut h);
                for v in [s.w, s.h, s.inner_radius, s.outer_radius, s.center.0, s.center.1, s.line_width] {
                    v.to_bits().hash(&mut h);
                }
                for c in [Some(s.color), s.outer].into_iter().flatten() {
                    for v in [c.r, c.g, c.b, c.a] {
                        v.to_bits().hash(&mut h);
                    }
                }
                s.outer.is_some().hash(&mut h);
            }
        }
        h.finish()
    }
}

/// `v` limited to `lo..=hi` without `f64::clamp`'s panic: bounds that come from a plugin's schema or from the
/// document may be inverted or NaN (a NaN bound is ignored, an inverted pair resolves to `hi`)
pub fn fit_between(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// The legacy model of a float control: what the knob, the wheel and the drag compute with
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// The arc's origin (`setCenterValue`; the legacy default is the minimum)
    pub center: f64,
    /// The legacy "logarithmic" scale (a power curve on the travel)
    pub log: bool,
}

const E: f64 = std::f64::consts::E;

fn signed_pow(v: f64, e: f64) -> f64 {
    v.abs().powf(e).copysign(v)
}

impl Range {
    pub fn span(&self) -> f64 {
        self.max - self.min
    }

    /// `AutomatableModel::fittedValue`: clamped, rounded to the step
    pub fn fit(&self, v: f64) -> f64 {
        let v = fit_between(v, self.min, self.max);
        if self.step > 0.0 {
            // rounded at the step, then clamped again (the rounding may pass the end)
            fit_between((v / self.step).round() * self.step, self.min, self.max)
        } else {
            v
        }
    }

    /// `inverseScaledValue`: a value to the linear travel (in the value's own range)
    pub fn inverse_scaled(&self, v: f64) -> f64 {
        if !self.log || self.span() == 0.0 {
            return v;
        }
        let v = fit_between(v, self.min, self.max);
        if self.min < 0.0 {
            let m = self.min.abs().max(self.max.abs());
            return signed_pow(v / m, 1.0 / E) * m;
        }
        ((v - self.min) / self.span()).powf(1.0 / E) * self.span() + self.min
    }

    /// `scaledValue`: the linear travel back to a value
    pub fn scaled(&self, t: f64) -> f64 {
        if !self.log || self.span() == 0.0 {
            return t;
        }
        let x = (t - self.min) / self.span();
        if self.min < 0.0 {
            let m = self.min.abs().max(self.max.abs());
            let val = x * self.span() + self.min;
            let r = signed_pow(val / m, E) * m;
            return if r.is_nan() { 0.0 } else { r };
        }
        let r = x.powf(E) * self.span() + self.min;
        if r.is_nan() { 0.0 } else { r }
    }

    /// `Knob::angleFromValue` on the travel, in whole degrees (the legacy truncates), for a total of 270°
    pub fn angle(&self, v: f64) -> i32 {
        if self.span() == 0.0 {
            return 0;
        }
        let t = self.inverse_scaled(v) as f32;
        let (min, max) = (self.min as f32, self.max as f32);
        (((t - 0.5 * (min + max)) / (max - min) * KNOB_TOTAL_ANGLE) as i32) % 360
    }

    /// `FloatModelEditorBase::setPosition`: the value after the mouse moved `dy` pixels down from the last
    /// position; `left_over` carries what did not reach a step. `slow`: Shift held
    pub fn drag(&self, value: f64, dy: f32, slow: bool, left_over: &mut f64) -> f64 {
        // getValue: 0.4 of a page per pixel; Shift: a quarter, at most 4 pages a move
        let mut page = 0.4 * dy as f64;
        if slow {
            page = (page / 4.0).clamp(-4.0, 4.0);
        }
        let offset = page * self.span() / 100.0 + *left_over;
        let scaled_offset = value - self.scaled(self.inverse_scaled(value) - offset);
        let step = if self.step > 0.0 { self.step } else { self.span() / 1.0e6 };
        let rounded = ((value - scaled_offset) / step).round() * step;
        if !approx(rounded, value) {
            *left_over = 0.0;
            self.fit(rounded)
        } else {
            *left_over = if offset > 0.0 && approx(value, self.min) { 0.0 } else { offset };
            value
        }
    }

    /// `FloatModelEditorBase::wheelEvent`: one notch; `steps` the notches of a full sweep (100, Shift 10,
    /// Ctrl 1000, Alt 2000)
    pub fn wheel(&self, value: f64, up: bool, steps: f64) -> f64 {
        let step = if self.step > 0.0 { self.step } else { self.span() / 1.0e6 };
        let offset = self.span() / steps;
        let scaled_offset = self.scaled(self.inverse_scaled(value) + offset) - value;
        let mult = (scaled_offset / step).max(1.0);
        let inc = (if up { 1.0 } else { -1.0 } * mult).trunc();
        self.fit(value + inc * step)
    }

    /// `FloatModel::getDigitCount`
    pub fn digits(&self) -> usize {
        if self.step > 0.0 { (-self.step.log10()).ceil().max(0.0) as usize } else { 2 }
    }

    /// `FloatModel::getRoundedValue`
    pub fn rounded(&self, v: f64) -> f64 {
        if self.step > 0.0 { (v / self.step).round() * self.step } else { v }
    }
}

/// `approximatelyEqual` (lmms_math.h)
fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1.0e-6 * (1.0f64).max(a.abs()).max(b.abs())
}

/// `Knob::initUi`: `setTotalAngle(270)`
pub const KNOB_TOTAL_ANGLE: f32 = 270.0;

/// `QString::number(v)`: the shortest of `%g` with 6 significant digits
pub fn qt_number(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return if v.is_nan() {
            "nan".into()
        } else if v.is_infinite() {
            "inf".into()
        } else {
            "0".into()
        };
    }
    let exp = v.abs().log10().floor() as i32;
    if !(-5..6).contains(&exp) {
        let s = format!("{:.5e}", v);
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = m.trim_end_matches('0').trim_end_matches('.');
        let e: i32 = e.parse().unwrap_or(0);
        return format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    let decimals = (5 - exp).max(0) as usize;
    let s = format!("{:.*}", decimals, v);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".into() } else { s }
}

/// A volume knob's value as dBFS (`VolumeKnob::currentValueToText`): `zero` is the value of 0 dBFS (100)
pub fn volume_text(rounded: f64, zero: f64) -> String {
    let ratio = rounded.abs() / zero;
    if ratio == 0.0 {
        return "-∞ dBFS".into();
    }
    let db = 20.0 * ratio.log10();
    if rounded > 0.0 { format!("{db:.2} dBFS") } else { format!("{db:.2} dBFS (inverted)") }
}

/// Draws a knob at `angle` (whole degrees, 0 straight up, clockwise positive) with its arc from `center_angle`,
/// as `Knob::drawKnob`. The canvas has the knob's size
pub fn draw_knob(c: &mut Canvas, style: &KnobStyle, angle: i32, center_angle: i32, enabled: bool) {
    match style {
        KnobStyle::Gx(g) => crate::gxwidgets::draw_knob(c, *g, angle, enabled),
        KnobStyle::Styled(s) => {
            // the styled knob draws only its line (the plugin's artwork has the knob's body)
            let (cx, cy) = s.center;
            let line = knob_line((cx, cy), angle, s.outer_radius, s.inner_radius);
            let paint = match s.outer {
                Some(outer) => {
                    Paint::Radial { cx, cy, radius: s.outer_radius, stops: vec![(0.4, s.color), (1.0, outer)] }
                }
                None => Paint::Solid(Rgba::rgb(0, 0, 0)),
            };
            // QPen::setWidth((int) lineWidth) when there is no outer color
            let w = if s.outer.is_some() { s.line_width } else { s.line_width.trunc() };
            c.line(line.0, line.1, w, Cap::Round, &paint);
        }
        _ => {
            let Some(asset) = style.pixmap() else { return };
            let img = assets::image(asset);
            let (pw, ph) = (img.width() as i32, img.height() as i32);
            let radius = pw as f32 / 2.0 - 1.0;
            // QPoint(width() / 2, pixmap height / 2): integer division
            let mid = ((pw / 2) as f32, (ph / 2) as f32);
            // the body is drawn at the screen's scale (the legacy strip is a 1x bitmap): the pixmap gives the size
            draw_knob_body(c, style, pw as f32, ph as f32, !enabled);
            let (line_color, arc_color) = match (style, enabled) {
                (_, false) => (theme::LINE_INACTIVE, theme::ARC_INACTIVE),
                (KnobStyle::Vintage32, true) => (theme::SHADOW, Rgba { a: 70.0 / 255.0, ..theme::SHADOW }),
                (_, true) => (theme::WINDOW_TEXT, Rgba { a: 70.0 / 255.0, ..theme::WINDOW_TEXT }),
            };
            let arc_size = (pw - 2) as f32;
            let arc_x = mid.0 - ((pw - 2) / 2) as f32;
            // the whole travel, faint: from 315° (Qt's angles) over 270°
            c.arc(arc_x, 1.0, arc_size, 315.0, KNOB_TOTAL_ANGLE, 2.0, Cap::Square, arc_color);
            let pen = Paint::Solid(line_color);
            match style {
                KnobStyle::Small17 => {
                    let l = knob_line(mid, angle, radius - 2.0, 1.0);
                    c.line(l.0, l.1, 2.0, Cap::Square, &pen);
                }
                KnobStyle::Bright26 => {
                    let l = knob_line(mid, angle, radius - 5.0, 1.0);
                    c.line(l.0, l.1, 2.0, Cap::Square, &pen);
                }
                KnobStyle::Dark28 => {
                    let rb = ((radius - 10.0) / 3.0).max(0.0);
                    let re = (radius - 4.0).max(0.0);
                    let l = knob_line(mid, angle, re, rb);
                    c.line((l.0.0 + 1.0, l.0.1 + 1.0), (l.1.0 + 1.0, l.1.1 + 1.0), 2.0, Cap::Square, &pen);
                }
                KnobStyle::Vintage32 => {
                    let l = knob_line(mid, angle, radius - 2.0, 2.0);
                    c.line(l.0, l.1, 2.0, Cap::Square, &pen);
                }
                KnobStyle::Styled(_) | KnobStyle::Gx(_) => {}
            }
            // the value's arc, from the center value to the value, in the line's color
            c.arc(
                arc_x,
                1.0,
                arc_size,
                (90 - center_angle) as f32,
                -((angle - center_angle) as f32),
                2.0,
                Cap::Square,
                line_color,
            );
        }
    }
}

/// `Knob::calculateLine`: from the inner to the outer radius at `angle`
fn knob_line(mid: (f32, f32), angle: i32, radius: f32, inner: f32) -> ((f32, f32), (f32, f32)) {
    let rarc = (angle as f32).to_radians();
    let ca = rarc.cos();
    let sa = -rarc.sin();
    ((mid.0 - sa * inner, mid.1 - ca * inner), (mid.0 - sa * radius, mid.1 - ca * radius))
}

/// An LED's color (`LedCheckBox::LedColor`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LedColor {
    Yellow,
    Green,
    Red,
}

impl LedColor {
    pub fn asset(self) -> &'static Asset {
        match self {
            LedColor::Yellow => &assets::lmms::LED_YELLOW,
            LedColor::Green => &assets::lmms::LED_GREEN,
            LedColor::Red => &assets::lmms::LED_RED,
        }
    }
}

/// An LCD's digit sheet (`LcdWidget`, `lcd_<style>.png`): 12 cells of glyphs (`0`–`9`, blank, `-`) and a margin
/// column, the enabled row above the disabled one
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LcdSheet {
    Green19,
    Red19,
    Pink21,
    Green11,
    Purple19,
}

/// The lit color of the sheet's segments
fn lcd_color(sheet: LcdSheet) -> Rgba {
    match sheet {
        LcdSheet::Green19 | LcdSheet::Green11 => Rgba::hex(0x4cff5e),
        LcdSheet::Red19 => Rgba::hex(0xff3b2f),
        LcdSheet::Pink21 => Rgba::hex(0xff5bd6),
        LcdSheet::Purple19 => Rgba::hex(0xb06bff),
    }
}

/// The segments (a to g, bit 0 to 6) lit for a glyph
fn lcd_segments(d: char) -> u8 {
    match d {
        '0' => 0b0111111,
        '1' => 0b0000110,
        '2' => 0b1011011,
        '3' => 0b1001111,
        '4' => 0b1100110,
        '5' => 0b1101101,
        '6' => 0b1111101,
        '7' => 0b0000111,
        '8' => 0b1111111,
        '9' => 0b1101111,
        '-' => 0b1000000,
        _ => 0,
    }
}

/// The LCD's dark ground under `w` pixels of cells
fn lcd_ground(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, enabled: bool) {
    let g = if enabled { Rgba::rgb(10, 14, 11) } else { Rgba::rgb(22, 24, 26) };
    c.fill_rect(x, y, w, h, g);
}

/// One glyph as seven segments at the screen's scale (the legacy's `lcd_*.png` strips redrawn as vectors): the lit
/// ones in the sheet's color, the others a faint trace of it
fn lcd_glyph(c: &mut Canvas, sheet: LcdSheet, d: char, x: f32, y: f32, enabled: bool) {
    let (cw, ch) = sheet.cell();
    let (cw, ch) = (cw as f32, ch as f32);
    lcd_ground(c, x, y, cw, ch, enabled);
    let lit = if enabled { lcd_color(sheet) } else { lcd_color(sheet).gray() };
    let (padx, pady) = (cw * 0.2, ch * 0.09);
    let t = (cw * 0.17).max(1.4);
    let (l, r) = (x + padx + t / 2.0, x + cw - padx - t / 2.0);
    let (top, mid, bot) = (y + pady + t / 2.0, y + ch / 2.0, y + ch - pady - t / 2.0);
    let segs: [((f32, f32), (f32, f32)); 7] = [
        ((l, top), (r, top)),
        ((r, top), (r, mid)),
        ((r, mid), (r, bot)),
        ((l, bot), (r, bot)),
        ((l, mid), (l, bot)),
        ((l, top), (l, mid)),
        ((l, mid), (r, mid)),
    ];
    let on = lcd_segments(d);
    for (i, (a, b)) in segs.iter().enumerate() {
        let lit_now = on & (1 << i) != 0;
        let color = if lit_now { lit } else { Rgba { a: if enabled { 0.1 } else { 0.05 }, ..lit } };
        c.line(*a, *b, t, Cap::Round, &Paint::Solid(color));
    }
}

/// `LcdWidget::charsPerPixmap`
const LCD_CELLS: u32 = 12;

impl LcdSheet {
    pub fn asset(self) -> &'static Asset {
        match self {
            LcdSheet::Green19 => &assets::lmms::LCD_19GREEN,
            LcdSheet::Red19 => &assets::lmms::LCD_19RED,
            LcdSheet::Pink21 => &assets::lmms::LCD_21PINK,
            LcdSheet::Green11 => &assets::lmms::LCD_11GREEN,
            LcdSheet::Purple19 => &assets::lmms::LCD_19PURPLE,
        }
    }

    /// One glyph's size (`LcdWidget::loadPixmap`)
    pub fn cell(self) -> (u32, u32) {
        let img = assets::image(self.asset());
        (img.width() / LCD_CELLS, img.height() / 2)
    }
}

impl LcdSheet {
    /// The decimal point of a `LcdFloatSpinBox` (`lcd_<style>_dot.png`)
    pub fn dot(self) -> &'static Asset {
        match self {
            LcdSheet::Green19 | LcdSheet::Pink21 => &assets::lmms::LCD_19GREEN_DOT,
            LcdSheet::Red19 => &assets::lmms::LCD_19RED_DOT,
            LcdSheet::Green11 => &assets::lmms::LCD_11GREEN_DOT,
            LcdSheet::Purple19 => &assets::lmms::LCD_19PURPLE_DOT,
        }
    }
}

/// The size of a `LcdFloatSpinBox` of `whole` + `frac` digits: the whole part with its left margin only, the dot, the
/// fraction with its right margin only (`LcdWidget::updateSize` with the seamless sides)
pub fn lcd_float_size(sheet: LcdSheet, whole: usize, frac: usize) -> (f32, f32) {
    let (cw, ch) = sheet.cell();
    let margin_w = cw / 2;
    let dot = assets::image(sheet.dot()).width();
    ((cw * (whole + frac) as u32 + 2 * (1 + margin_w) + dot) as f32, (ch + 2) as f32)
}

/// The two texts of a `LcdFloatSpinBox` for a value (`LcdFloatSpinBox::update`): the whole part (`-0` for values in
/// (-1, 0)) and the fraction with its leading zeros
pub fn lcd_float_texts(value: f64, frac: usize) -> (String, String) {
    let scale = 10f64.powi(frac as i32);
    let mut value = value;
    let mut fraction = ((value - value.trunc()) * scale).round().abs();
    if fraction == scale {
        value += 1f64.copysign(value);
        fraction = 0.0;
    }
    (lcd_text(value, 0, false), format!("{:0>frac$}", fraction as i64))
}

/// `LcdFloatSpinBox::paintEvent` with its two `LcdWidget`s and the dot between them
pub fn draw_lcd_float(
    c: &mut Canvas,
    sheet: LcdSheet,
    whole: usize,
    frac: usize,
    texts: &(String, String),
    enabled: bool,
) {
    let (cw, ch) = sheet.cell();
    let mw = cw / 2;
    let glyph = |c: &mut Canvas, d: char, x: f32| lcd_glyph(c, sheet, d, x, 1.0, enabled);
    // the whole part: left margin, padding, digits, no right margin
    let mut x = 1.0;
    lcd_ground(c, x, 1.0, mw as f32, ch as f32, enabled);
    x += mw as f32;
    let shown: Vec<char> = texts.0.chars().collect();
    for _ in shown.len()..whole {
        glyph(c, ' ', x);
        x += cw as f32;
    }
    for d in shown {
        glyph(c, d, x);
        x += cw as f32;
    }
    // the dot, centered in the height of the box
    let dot = assets::image(sheet.dot());
    let dot_h = dot.height() / 2;
    let dot_y = ((ch + 2) as f32 - dot_h as f32) / 2.0;
    lcd_ground(c, x, 1.0, dot.width() as f32, ch as f32, enabled);
    let dot_color = if enabled { lcd_color(sheet) } else { lcd_color(sheet).gray() };
    let dw = (dot.width() as f32 * 0.5).max(2.0);
    c.fill_rect(x + (dot.width() as f32 - dw) / 2.0, (dot_y + dot_h as f32 - dw - 1.0).floor(), dw, dw, dot_color);
    x += dot.width() as f32;
    // the fraction: digits from its left edge, then the right margin
    let shown: Vec<char> = texts.1.chars().collect();
    for _ in shown.len()..frac {
        glyph(c, '0', x);
        x += cw as f32;
    }
    for d in shown {
        glyph(c, d, x);
        x += cw as f32;
    }
    lcd_ground(c, x, 1.0, mw.saturating_sub(1) as f32, ch as f32, enabled);
    let (w, h) = lcd_float_size(sheet, whole, frac);
    draw_frame(c, w - 1.0, h);
}

/// The LCD's size without its label (`LcdWidget::updateSize`)
pub fn lcd_size(sheet: LcdSheet, digits: usize) -> (f32, f32) {
    let (cw, ch) = sheet.cell();
    let margin_w = cw / 2;
    ((cw * digits as u32 + 2 * (1 + margin_w)) as f32, (ch + 2) as f32)
}

/// `LcdWidget::paintEvent` without the label: the left margin, the blank padding, the glyphs, the right margin, and
/// the sunken frame of `LmmsStyle::drawPrimitive(PE_Frame)`
pub fn draw_lcd(c: &mut Canvas, sheet: LcdSheet, digits: usize, text: &str, enabled: bool) {
    let (cw, ch) = sheet.cell();
    let mw = cw / 2;
    let mut x = 1.0;
    let y = 1.0;
    lcd_ground(c, x, y, mw as f32, ch as f32, enabled);
    x += mw as f32;
    let shown: Vec<char> = text.chars().collect();
    for _ in shown.len()..digits {
        lcd_glyph(c, sheet, ' ', x, y, enabled);
        x += cw as f32;
    }
    for d in shown {
        lcd_glyph(c, sheet, d, x, y, enabled);
        x += cw as f32;
    }
    // the right margin
    lcd_ground(c, x, y, mw.saturating_sub(1) as f32, ch as f32, enabled);
    // the frame: rect (0, 0, w, h) with w = cells + margins - 1
    let w = (cw * digits as u32 + (1 + mw) * 2 - 1) as f32;
    let h = (ch + 2) as f32;
    draw_frame(c, w, h);
}

/// `LmmsStyle::drawPrimitive(PE_Frame)`: the sunken frame of LCDs, combo boxes and line edits, on the rectangle
/// (0, 0, `w`, `h`)
pub fn draw_frame(c: &mut Canvas, w: f32, h: f32) {
    let (l, t, r, b) = (0.0, 0.0, w - 1.0, h - 1.0);
    let black = |a: f32| Rgba { r: 0.0, g: 0.0, b: 0.0, a: a / 255.0 };
    let hl = |a: f32| Rgba { a: a / 255.0, ..Rgba::hex(0x262b30) };
    let (a100, a75, a50, a25) = (165.0, 123.0, 99.0, 54.0);
    let hline = |c: &mut Canvas, x0: f32, x1: f32, y: f32, col: Rgba| c.fill_rect(x0, y, x1 - x0 + 1.0, 1.0, col);
    let vline = |c: &mut Canvas, x: f32, y0: f32, y1: f32, col: Rgba| c.fill_rect(x, y0, 1.0, y1 - y0 + 1.0, col);
    let dot = |c: &mut Canvas, x: f32, y: f32, col: Rgba| c.fill_rect(x, y, 1.0, 1.0, col);
    hline(c, l + 2.0, r - 2.0, t + 1.0, black(a100));
    hline(c, l + 2.0, r - 2.0, b - 1.0, black(a100));
    vline(c, l + 1.0, t + 2.0, b - 2.0, black(a100));
    vline(c, r - 1.0, t + 2.0, b - 2.0, black(a100));
    for (px_, py_) in [(l + 2.0, t + 2.0), (l + 2.0, b - 2.0), (r - 2.0, t + 2.0), (r - 2.0, b - 2.0)] {
        dot(c, px_, py_, black(a50));
    }
    hline(c, l + 2.0, r - 2.0, t, black(a75));
    vline(c, l, t + 2.0, b - 2.0, black(a75));
    dot(c, l + 1.0, t + 1.0, black(a50));
    dot(c, r - 1.0, t + 1.0, black(a50));
    for (px_, py_) in [(l + 1.0, t), (l, t + 1.0), (r - 1.0, t), (l, b - 1.0)] {
        dot(c, px_, py_, black(a25));
    }
    hline(c, l + 2.0, r - 2.0, b, hl(a75));
    vline(c, r, t + 2.0, b - 2.0, hl(a75));
    dot(c, l + 1.0, b - 1.0, hl(a50));
    dot(c, r - 1.0, b - 1.0, hl(a50));
    for (px_, py_) in [(r - 1.0, b), (r, b - 1.0), (l + 1.0, b), (r, t + 1.0)] {
        dot(c, px_, py_, hl(a25));
    }
}

/// `ComboBox::DEFAULT_HEIGHT`
pub const COMBO_HEIGHT: f32 = 22.0;
/// `CB_ARROW_BTN_WIDTH`
pub const COMBO_ARROW_W: f32 = 18.0;

/// `ComboBox::paintEvent` without the text (drawn as text by the panel): the tiled `combobox_bg`, the button's
/// separator, the frame, the arrow, and the current item's icon (scaled to 16 pixels high when taller). Returns
/// where the text starts
pub fn draw_combo(c: &mut Canvas, w: f32, icon: Option<&image::RgbaImage>, pressed: bool) -> f32 {
    let h = COMBO_HEIGHT;
    let bg = assets::image(&assets::lmms::COMBOBOX_BG);
    // fillRect(2, 2, width - 2, height - 4) with the pixmap as a tiled brush
    let (bw, bh) = (bg.width().max(1), bg.height().max(1));
    let mut y = 2u32;
    while (y as f32) < h - 2.0 {
        let mut x = 2u32;
        while (x as f32) < w {
            let cw = bw.min(w as u32 - x);
            let chh = bh.min((h - 2.0) as u32 - y);
            c.pixmap_part(
                bg,
                ((x % bw), (y % bh), cw.min(bw - x % bw), chh.min(bh - y % bh)),
                x as f32,
                y as f32,
                false,
            );
            x += bw - x % bw;
        }
        y += bh - y % bh;
    }
    let shadow = Rgba::alpha(0, 0, 0, 124);
    let highlight = Rgba { a: 124.0 / 255.0, ..Rgba::hex(0x262b30) };
    c.fill_rect(w - COMBO_ARROW_W - 1.0, 1.0, 1.0, h - 3.0, shadow);
    c.fill_rect(w - COMBO_ARROW_W, 1.0, 1.0, h - 3.0, highlight);
    draw_frame(c, w, h);
    let arrow = if pressed { &assets::lmms::COMBOBOX_ARROW_SELECTED } else { &assets::lmms::COMBOBOX_ARROW };
    c.pixmap(assets::image(arrow), w - COMBO_ARROW_W + 3.0, 4.0, false);
    let mut tx = 5.0;
    if let Some(icon) = icon {
        if icon.height() > 16 {
            let k = 16.0 / icon.height() as f32;
            let scaled = image::imageops::resize(
                icon,
                (icon.width() as f32 * k).round() as u32,
                16,
                image::imageops::FilterType::Triangle,
            );
            c.pixmap(&scaled, tx, 3.0, false);
            tx += scaled.width() as f32 + 3.0;
        } else {
            c.pixmap(icon, tx, 3.0, false);
            tx += icon.width() as f32 + 3.0;
        }
    }
    tx
}

/// `LcdWidget::setValue(float)`: the text shown for a value (`-0` for values in (-1, 0)), with leading zeros
pub fn lcd_text(value: f64, digits: usize, leading_zero: bool) -> String {
    if -1.0 < value && value < 0.0 {
        return "-0".into();
    }
    let s = (value as i64).to_string();
    if leading_zero && s.len() < digits { format!("{s:0>digits$}") } else { s }
}

/// `Graph::paintEvent`: the curve in its style, the cross of the cursor while drawing, the foreground pixmap
pub fn draw_graph(c: &mut Canvas, g: &crate::spec::GraphSpec, samples: &[f64], cursor: Option<(f32, f32)>) {
    use crate::spec::GraphStyle;
    let (w, h) = (g.w, g.h);
    let n = samples.len().max(1);
    let xscale = (w - 4.0) / n as f32;
    let yscale = (h - 4.0) / (g.min - g.max) as f32;
    let ys = |v: f64| 2.0 + (((v - g.max) as f32 * yscale) as i32) as f32;
    let xs = |i: usize| 2.0 + ((i as f32 * xscale) as i32) as f32;
    let pen = Paint::Solid(g.color);
    let last = n - 1;
    if let Some(bg) = g.background {
        c.pixmap(assets::image(bg), 0.0, 0.0, false);
    }
    // a cosmetic pen of 1: the pixel row of the integer coordinate (Qt draws it at +0.5 without antialiasing)
    let crisp = |c: &mut Canvas, x0: f32, y0: f32, x1: f32, y1: f32| {
        c.line((x0 + 0.5, y0 + 0.5), (x1 + 0.5, y1 + 0.5), 1.0, Cap::Square, &pen);
    };
    match g.style {
        GraphStyle::Linear | GraphStyle::LinearNonCyclic => {
            for i in 0..last {
                c.line((xs(i), ys(samples[i])), (xs(i + 1), ys(samples[i + 1])), 1.0, Cap::Flat, &pen);
            }
            if g.style == GraphStyle::Linear {
                c.line((xs(last), ys(samples[last])), (w - 3.0, ys(samples[0])), 1.0, Cap::Flat, &pen);
            }
        }
        GraphStyle::Nearest => {
            for i in 0..last {
                crisp(c, xs(i), ys(samples[i]), xs(i + 1), ys(samples[i]));
                crisp(c, xs(i + 1), ys(samples[i]), xs(i + 1), ys(samples[i + 1]));
            }
            crisp(c, xs(last), ys(samples[last]), w - 3.0, ys(samples[last]));
        }
        GraphStyle::Bar => {
            let fill = Rgba { a: 100.0 / 255.0, ..g.color };
            let bottom = (((g.min - g.max) as f32 * yscale) as i32) as f32;
            for (i, v) in samples.iter().enumerate() {
                let top = ys(*v) - 2.0;
                c.fill_rect(xs(i), ys(*v), ((xscale as i32) - 1).max(1) as f32, (bottom - top).max(1.0), fill);
                let x1 = ((i as f32 * xscale) as i32 + xscale as i32).max(2 + (i as f32 * xscale) as i32) as f32;
                crisp(c, xs(i), ys(*v), x1, ys(*v));
            }
        }
    }
    if let Some((x, y)) = cursor {
        let cross = Paint::Solid(Rgba::hex(0x707c91));
        c.line((2.0, y.floor() + 0.5), (w - 2.0, y.floor() + 0.5), 1.0, Cap::Flat, &cross);
        c.line((x.floor() + 0.5, 2.0), (x.floor() + 0.5, h - 2.0), 1.0, Cap::Flat, &cross);
    }
    if let Some(fg) = g.foreground {
        c.pixmap(assets::image(fg), 0.0, 0.0, false);
    }
}

/// `Graph::changeSampleAt`: the sample under `x` takes the value of `y`
pub fn graph_set(g: &crate::spec::GraphSpec, samples: &mut [f64], x: f32, y: f32) {
    if g.w <= 4.0 || samples.is_empty() {
        return;
    }
    let xscale = samples.len() as f32 / (g.w - 4.0);
    let (x, y) = (x - 2.0, y - 2.0);
    let v = f64::from(y) * (g.min - g.max) / f64::from(g.h - 5.0) + g.max;
    let i = ((x * xscale) as isize).clamp(0, samples.len() as isize - 1) as usize;
    samples[i] = graph_fit(g, v);
}

/// `Graph::drawLineAt`: a straight line of samples from the last x to this one
pub fn graph_line(g: &crate::spec::GraphSpec, samples: &mut [f64], x: f32, y: f32, last_x: f32) {
    if g.w <= 4.0 || samples.is_empty() {
        return;
    }
    let len = samples.len();
    let xscale = len as f32 / (g.w - 4.0);
    let x = (x - 2.0) as i32;
    let y = y - 2.0;
    let lastx = ((last_x - 2.0) as i32).max(0).min(g.w as i32 - 5);
    let v = f64::from(y) * (g.min - g.max) / f64::from(g.h - 5.0) + g.max;
    let (begin, end, mut vb, ve) = if lastx > x {
        let b = ((x as f32 * xscale) as i32).max(0) as usize;
        let e = (((lastx + 1) as f32 * xscale).ceil() as usize).min(len);
        (b, e, v, samples[e.saturating_sub(1)])
    } else {
        let b = ((lastx as f32 * xscale) as i32).max(0) as usize;
        let e = (((x + 1) as f32 * xscale).ceil().max(0.0) as usize).min(len);
        (b.min(len - 1), e, samples[b.min(len - 1)], v)
    };
    let n = end.saturating_sub(begin).max(1);
    if n == 1 {
        vb = v;
    }
    let step = (ve - vb) / n as f64;
    for i in 0..n {
        if begin + i < len {
            samples[begin + i] = graph_fit(g, vb + i as f64 * step);
        }
    }
}

/// `graphModel::drawSampleAt`: clamped to the range, rounded to the step
fn graph_fit(g: &crate::spec::GraphSpec, v: f64) -> f64 {
    let v = fit_between(v, g.min, g.max);
    if g.step > 0.0 { fit_between((v / g.step).round() * g.step, g.min, g.max) } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lin(min: f64, max: f64, step: f64) -> Range {
        Range { min, max, step, center: min, log: false }
    }

    #[test]
    fn inverted_or_nan_bounds_do_not_panic() {
        assert_eq!(fit_between(0.5, 1.0, 0.0), 0.0);
        assert_eq!(fit_between(0.5, f64::NAN, 1.0), 0.5);
        assert_eq!(fit_between(f64::NAN, 0.0, 1.0), 0.0);
        let r = lin(10.0, 0.0, 1.0);
        assert!(r.fit(5.0).is_finite());
        assert!(r.inverse_scaled(5.0).is_finite());
    }

    #[test]
    fn the_angle_runs_from_minus_135_to_135() {
        let r = lin(0.0, 200.0, 1.0);
        assert_eq!(r.angle(0.0), -135);
        assert_eq!(r.angle(100.0), 0);
        assert_eq!(r.angle(200.0), 135);
        // truncated toward zero, like the legacy int cast
        assert_eq!(r.angle(0.5), -134);
    }

    #[test]
    fn a_drag_of_250_pixels_sweeps_the_whole_range() {
        let r = lin(0.0, 200.0, 1.0);
        let mut left = 0.0;
        let mut v = 200.0;
        for _ in 0..250 {
            v = r.drag(v, 1.0, false, &mut left);
        }
        assert_eq!(v, 0.0);
        // Shift: four times slower
        let mut left = 0.0;
        let mut v = 0.0;
        for _ in 0..100 {
            v = r.drag(v, -1.0, true, &mut left);
        }
        // 0.2 a pixel, rounded to the step of 1 with what is left carried: a step every 3 pixels (the legacy's)
        assert_eq!(v, 33.0);
    }

    #[test]
    fn the_wheel_takes_100_notches_or_10_with_shift() {
        let r = lin(0.0, 200.0, 1.0);
        assert_eq!(r.wheel(100.0, true, 100.0), 102.0);
        assert_eq!(r.wheel(100.0, false, 10.0), 80.0);
        // never less than one step
        assert_eq!(r.wheel(100.0, true, 2000.0), 101.0);
    }

    #[test]
    fn the_log_scale_is_the_legacy_power_curve() {
        let r = Range { min: 20.0, max: 20000.0, step: 0.0, center: 20.0, log: true };
        let mid = r.scaled(r.min + r.span() / 2.0);
        assert!((mid - (0.5f64.powf(E) * r.span() + r.min)).abs() < 1e-6);
        assert!((r.inverse_scaled(mid) - (r.min + r.span() / 2.0)).abs() < 1e-6);
    }

    #[test]
    fn numbers_print_as_qt_does() {
        assert_eq!(qt_number(20.0), "20");
        assert_eq!(qt_number(-12.0), "-12");
        assert_eq!(qt_number(0.5), "0.5");
        assert_eq!(qt_number(1234.5678), "1234.57");
        assert_eq!(volume_text(100.0, 100.0), "0.00 dBFS");
        assert_eq!(volume_text(0.0, 100.0), "-∞ dBFS");
    }

    #[test]
    fn every_knob_draws_its_line_at_its_angle() {
        for style in [KnobStyle::Dark28, KnobStyle::Bright26, KnobStyle::Small17, KnobStyle::Vintage32] {
            let (w, h) = style.size();
            let mut up = Canvas::new(w, h, 1.0);
            draw_knob(&mut up, &style, 0, -135, true);
            let mut right = Canvas::new(w, h, 1.0);
            draw_knob(&mut right, &style, 90, -135, true);
            // the line (green, black on the vintage knob) is above the middle at 0°, right of it at 90°
            let line = |c: &Canvas, x: usize, y: usize| {
                let p = c.pixel(x, y);
                if style == KnobStyle::Vintage32 {
                    p[3] > 0.5 && p[0] + p[1] + p[2] < 0.3
                } else {
                    p[1] > p[0] + 0.2 && p[1] > p[2]
                }
            };
            let (mx, my) = ((w as usize) / 2, (h as usize) / 2);
            assert!((0..3).any(|d| line(&up, mx + d - 1, my - 5)), "{style:?} up");
            assert!((0..3).any(|d| line(&right, mx + 5, my + d - 1)), "{style:?} right");
        }
    }
}

// --- Fader (legacy `Fader`, src/gui/widgets/Fader.cpp)

/// The fader knob's size (`fader_knob.svg`)
pub const FADER_KNOB_W: f32 = 23.0;
pub const FADER_KNOB_H: f32 = 30.0;
/// The dBFS below which a linear fader drops to -inf (`c_faderMinDb`)
pub const FADER_MIN_DB: f64 = -120.0;
const FADER_EXPONENT: f64 = 3.0;

pub fn amp_to_db(v: f64) -> f64 {
    20.0 * v.log10()
}

pub fn db_to_amp(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

fn fader_ratio(db: f64, max_db: f64) -> f64 {
    ((db - FADER_MIN_DB) / (max_db - FADER_MIN_DB)).clamp(0.0, 1.0).powf(FADER_EXPONENT)
}

/// `Fader::calculateKnobPosYFromModel`: the y of the knob's lower end for the model's `value` in `min..=max`
pub fn fader_knob_y(f: &crate::spec::FaderSpec, value: f64, min: f64, max: f64) -> f32 {
    let (h, k) = (f64::from(f.h), f64::from(FADER_KNOB_H));
    let ratio = if f.linear {
        if value - min <= 0.0 {
            return f.h;
        }
        fader_ratio(amp_to_db(value).max(FADER_MIN_DB), amp_to_db(max))
    } else if max > min {
        ((value.clamp(min, max) - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (h - (h - k) * ratio) as f32
}

/// `Fader::setVolumeByLocalPixelValue`: the model's value for the knob's center at local `y`
pub fn fader_value_at(f: &crate::spec::FaderSpec, y: f32, min: f64, max: f64) -> f64 {
    let (h, k) = (f64::from(f.h), f64::from(FADER_KNOB_H));
    let lower = (f64::from(y) + (k / 2.0).floor()).round();
    let clamped = fit_between(lower, k, h);
    if f.linear {
        if lower >= h {
            return 0.0;
        }
        // the inverse of `fader_knob_y`: ratio = (h - y) / (h - k), the cube of the position in the dB span
        let ratio = (h - clamped) / (h - k);
        let max_db = amp_to_db(max);
        let db = FADER_MIN_DB + ratio.powf(1.0 / FADER_EXPONENT) * (max_db - FADER_MIN_DB);
        if db < FADER_MIN_DB { 0.0 } else { db_to_amp(db.min(max_db)) }
    } else {
        min + (max - min) * ((h - clamped) / (h - k))
    }
}

/// The color a meter's level has at `db` (the legacy gradient: clip, warn between -0.01 and -6 dB, ok below -12 dB)
fn peak_color(db: f64) -> Rgba {
    let (ok, warn, clip) = (Rgba::rgb(10, 212, 92), Rgba::rgb(214, 236, 82), Rgba::rgb(193, 32, 56));
    let mix = |a: Rgba, b: Rgba, t: f32| Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    };
    if db >= 0.0 {
        clip
    } else if db > -0.01 {
        mix(clip, warn, (-db / 0.01) as f32)
    } else if db >= -6.0 {
        warn
    } else if db > -12.0 {
        mix(warn, ok, ((-6.0 - db) / 6.0) as f32)
    } else {
        ok
    }
}

/// `Fader::paintEvent`: the two meters (black wells, the levels in the gradient, the unity line) and the knob
pub fn draw_fader(
    c: &mut Canvas,
    f: &crate::spec::FaderSpec,
    value: f64,
    min: f64,
    max: f64,
    peaks: Option<(f32, f32)>,
) {
    let (w, h) = (f.w, f.h);
    // paintLevels: margin 1, 2 between, each meter (w - 2 - 2) / 2 wide
    let meter_w = ((w - 4.0) / 2.0).floor();
    let wells = [(1.0, 1.0, meter_w, h - 2.0), (w - 1.0 - meter_w, 1.0, meter_w, h - 2.0)];
    for (x, y, mw, mh) in wells {
        c.fill_rect(x, y + 1.0, mw, mh - 2.0, Rgba::rgb(0, 0, 0));
        c.fill_rect(x + 1.0, y, mw - 2.0, mh, Rgba::rgb(0, 0, 0));
    }
    // the levels are in dBFS from `min_peak` (-42) to `max_peak` (+9)
    let (lo_db, hi_db) = (-42.0f64, 9.0f64);
    let inner_top = 2.0f32;
    let inner_h = h - 4.0;
    let y_of = |db: f64| inner_top + (((hi_db - db) / (hi_db - lo_db)) as f32) * inner_h;
    let level_db = |v: f32| amp_to_db(f64::from(v).max(0.0001));
    if f.unity_line {
        let y = y_of(0.0).floor();
        for (x, _, mw, _) in wells {
            c.fill_rect(x + 1.0, y, mw - 2.0, 1.0, Rgba::rgb(63, 63, 63));
        }
    }
    if let Some((l, r)) = peaks {
        for ((x, _, mw, _), v) in wells.iter().zip([l, r]) {
            let db = level_db(v);
            if db <= lo_db {
                continue;
            }
            let top = y_of(db.min(hi_db)).floor();
            let bottom = inner_top + inner_h;
            let mut y = top;
            while y < bottom {
                let row_db = hi_db - f64::from((y - inner_top) / inner_h) * (hi_db - lo_db);
                c.fill_rect(x + 1.0, y, mw - 2.0, 1.0, peak_color(row_db));
                y += 1.0;
            }
        }
    }
    let lower = fader_knob_y(f, value, min, max);
    draw_fader_knob(c, (w - FADER_KNOB_W) / 2.0, lower - FADER_KNOB_H);
}

/// `fader_knob.svg` (23×30), drawn by hand: a black-edged rounded rectangle in a vertical gradient (#2c3138 on top,
/// #1c1f24 below), a green stripe with its shine, the top and bottom highlights
fn draw_fader_knob(c: &mut Canvas, x: f32, y: f32) {
    let (top, bottom) = (Rgba::hex(0x2c3138), Rgba::hex(0x1c1f24));
    let mix = |t: f32| Rgba {
        r: top.r + (bottom.r - top.r) * t,
        g: top.g + (bottom.g - top.g) * t,
        b: top.b + (bottom.b - top.b) * t,
        a: 1.0,
    };
    // the stroke: x 5..18, y 2..27 with 2 px round corners
    c.fill_rect(x + 5.0, y + 3.0, 13.0, 24.0, Rgba::rgb(0, 0, 0));
    c.fill_rect(x + 6.0, y + 2.0, 11.0, 26.0, Rgba::rgb(0, 0, 0));
    for row in 0..24 {
        c.fill_rect(x + 6.0, y + 3.0 + row as f32, 11.0, 1.0, mix(row as f32 / 23.0));
    }
    // the green stripe at y = 15, 2 px, reflected gradient #08a342 to #0bd556
    let (g0, g1) = (Rgba::hex(0x08a342), Rgba::hex(0x0bd556));
    for i in 0..11 {
        let t = (1.0 - ((i as f32 - 5.0) / 5.0).abs()).clamp(0.0, 1.0);
        let col =
            Rgba { r: g0.r + (g1.r - g0.r) * t, g: g0.g + (g1.g - g0.g) * t, b: g0.b + (g1.b - g0.b) * t, a: 1.0 };
        c.fill_rect(x + 6.0 + i as f32, y + 14.0, 1.0, 2.0, col);
    }
    c.fill_rect(x + 7.0, y + 3.0, 9.0, 1.0, Rgba::alpha(255, 255, 255, 26));
    c.fill_rect(x + 7.0, y + 25.0, 9.0, 1.0, Rgba::alpha(0, 0, 0, 77));
}

/// The style sheet's small square button (`QPushButton#btn-mute`, `#btn-mute-inv`: 14×14, a 1-pixel border darker
/// at the bottom, rounded by 2, a light-to-dark veil) with the speaker icon when `on` (the legacy shows the icon on
/// the checked state; the unchecked one is the bare button)
pub fn draw_mute_button(c: &mut Canvas, on: bool) {
    let (w, h) = (14.0f32, 14.0f32);
    let edge = Rgba::hex(0x080b12);
    // the border: the sides, the top (#0e1218) and the bottom (#03060b); the corners cut by the radius
    c.fill_rect(1.0, 0.0, w - 2.0, 1.0, Rgba::hex(0x0e1218));
    c.fill_rect(1.0, h - 1.0, w - 2.0, 1.0, Rgba::hex(0x03060b));
    c.fill_rect(0.0, 1.0, 1.0, h - 2.0, edge);
    c.fill_rect(w - 1.0, 1.0, 1.0, h - 2.0, edge);
    // the veil: white 20 % at the top to 2 % at 10 %, flat to 90 %, then black 2 % to 40 % at the bottom
    for row in 1..13 {
        let t = (row as f32 + 0.5) / h;
        let color = if t < 0.1 {
            Rgba { a: 0.20 + (0.02 - 0.20) * (t / 0.1), r: 1.0, g: 1.0, b: 1.0 }
        } else if t < 0.9 {
            Rgba { a: 0.02, r: 1.0, g: 1.0, b: 1.0 }
        } else {
            Rgba { a: 0.02 + (0.40 - 0.02) * ((t - 0.9) / 0.1), r: 0.0, g: 0.0, b: 0.0 }
        };
        c.fill_rect(1.0, row as f32, w - 2.0, 1.0, color);
    }
    if !on {
        return;
    }
    // `speaker.svg` (14×14) in the 10×10 the padding leaves: the body, the cone and the two waves
    let s = 10.0 / 14.0;
    let white = Rgba::rgb(255, 255, 255);
    let (ox, oy) = (2.0f32, 2.0f32);
    c.fill_rect(ox, oy + 4.5 * s, 3.5 * s, 5.0 * s, white);
    for i in 0..8 {
        let x = 3.5 + i as f32 * 0.5;
        let grow = (i as f32 + 1.0) * 0.5 * 0.75;
        c.fill_rect(ox + x * s, oy + (4.5 - grow) * s, 0.5 * s, (5.0 + 2.0 * grow) * s, white);
    }
    c.arc(ox + 4.0 * s, oy + 3.0 * s, 8.0 * s, -45.0, 90.0, 1.0, Cap::Flat, white);
    c.arc(ox + 2.5 * s, oy + 1.5 * s, 11.0 * s, -45.0, 90.0, 1.0, Cap::Flat, white);
}

/// A filled disc of diameter `d` centered at (`cx`, `cy`), with a radial gradient (the painter has no fill: a
/// zero-length round-capped line of that width is one)
fn disc(c: &mut Canvas, cx: f32, cy: f32, d: f32, stops: Vec<(f32, Rgba)>) {
    let paint = Paint::Radial { cx: cx - d * 0.12, cy: cy - d * 0.15, radius: d * 0.62, stops };
    c.line((cx, cy), (cx, cy), d, Cap::Round, &paint);
}

/// The knob's body as a vector (the legacy `knob01`...`knob05` strips redrawn at any scale): a dark rim and a
/// shaded disc, in the legacy's tones (dark, bright, small, vintage)
fn draw_knob_body(c: &mut Canvas, style: &KnobStyle, w: f32, h: f32, gray: bool) {
    let (hi, lo, rim) = match style {
        KnobStyle::Dark28 => (Rgba::rgb(88, 94, 104), Rgba::rgb(26, 29, 34), Rgba::rgb(8, 9, 11)),
        KnobStyle::Bright26 => (Rgba::rgb(150, 158, 170), Rgba::rgb(62, 68, 78), Rgba::rgb(10, 11, 13)),
        KnobStyle::Small17 => (Rgba::rgb(130, 138, 150), Rgba::rgb(50, 56, 66), Rgba::rgb(10, 11, 13)),
        _ => (Rgba::rgb(214, 196, 150), Rgba::rgb(112, 96, 68), Rgba::rgb(40, 32, 20)),
    };
    let (hi, lo, rim) = if gray { (hi.gray(), lo.gray(), rim.gray()) } else { (hi, lo, rim) };
    let (cx, cy) = ((w / 2.0).floor(), (h / 2.0).floor());
    let d = w.min(h) - 1.0;
    disc(c, cx, cy, d, vec![(0.0, rim), (1.0, rim)]);
    disc(c, cx, cy, d - 2.0, vec![(0.0, hi), (1.0, lo)]);
}

/// The LED lamp as a vector: a dark bezel, the lit color in a radial shine (or the dark glass when off), and a
/// small highlight. The size is the legacy pixmap's
pub fn draw_led(c: &mut Canvas, color: LedColor, on: bool) {
    let img = assets::image(color.asset());
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let d = w.min(h) - 1.0;
    let (hi, lo) = match (color, on) {
        (LedColor::Yellow, true) => (Rgba::rgb(255, 232, 120), Rgba::rgb(220, 140, 0)),
        (LedColor::Green, true) => (Rgba::rgb(150, 255, 170), Rgba::rgb(10, 150, 60)),
        (LedColor::Red, true) => (Rgba::rgb(255, 150, 140), Rgba::rgb(170, 10, 20)),
        (_, false) => (Rgba::rgb(70, 76, 84), Rgba::rgb(22, 25, 29)),
    };
    disc(c, cx, cy, d, vec![(0.0, Rgba::rgb(6, 7, 9)), (1.0, Rgba::rgb(6, 7, 9))]);
    disc(c, cx, cy, d - 2.5, vec![(0.0, hi), (1.0, lo)]);
    if on {
        disc(
            c,
            cx - d * 0.14,
            cy - d * 0.18,
            d * 0.22,
            vec![(0.0, Rgba::alpha(255, 255, 255, 190)), (1.0, Rgba::alpha(255, 255, 255, 0))],
        );
    }
}

/// The theme's wave icon (`<name>_wave_active.png` / `_inactive.png`, 14×14) of an asset path: its name and whether it
/// is the lit one
pub fn wave_icon_of(path: &str) -> Option<(&str, bool)> {
    let file = path.strip_prefix("lmms/")?.strip_suffix(".png")?;
    let (name, state) = file.rsplit_once("_wave_")?;
    Some((name, state == "active"))
}

/// A wave icon as a vector at the screen's scale: the small dark plate and the wave's line in it
pub fn draw_wave_icon(c: &mut Canvas, name: &str, on: bool, w: f32, h: f32) {
    let (plate, line) =
        if on { (Rgba::rgb(52, 59, 66), Rgba::hex(0xd1d8e4)) } else { (Rgba::rgb(30, 34, 39), Rgba::hex(0x7b8590)) };
    c.fill_rect(0.0, 0.0, w, h, Rgba::rgb(8, 10, 12));
    c.fill_rect(1.0, 1.0, w - 2.0, h - 2.0, plate);
    let (x0, x1, top, bot) = (2.5, w - 2.5, 3.0, h - 3.0);
    let (mid, amp) = ((top + bot) / 2.0, (bot - top) / 2.0);
    let at = |t: f32, v: f32| (x0 + (x1 - x0) * t, mid - amp * v);
    let pts: Vec<(f32, f32)> = match name {
        "sin" => (0..=24).map(|i| at(i as f32 / 24.0, (i as f32 / 24.0 * std::f32::consts::TAU).sin())).collect(),
        "triangle" => vec![at(0.0, 0.0), at(0.25, 1.0), at(0.75, -1.0), at(1.0, 0.0)],
        "saw" => vec![at(0.0, -1.0), at(0.5, 1.0), at(0.5, -1.0), at(1.0, 1.0)],
        "square" => vec![at(0.0, -1.0), at(0.0, 1.0), at(0.5, 1.0), at(0.5, -1.0), at(1.0, -1.0)],
        "round_square" => (0..=24)
            .map(|i| at(i as f32 / 24.0, (3.0 * (i as f32 / 24.0 * std::f32::consts::TAU).sin()).tanh()))
            .collect(),
        "moog_saw" => vec![at(0.0, -1.0), at(0.4, 0.6), at(0.5, 0.9), at(0.55, -1.0), at(1.0, 1.0)],
        "exp" => (0..=16).map(|i| at(i as f32 / 16.0, 1.0 - 2.0 * (1.0 - (-3.0 * i as f32 / 16.0).exp()))).collect(),
        "white_noise" => [0.2, -0.7, 0.9, -0.3, 0.6, -0.9, 0.1, 0.8, -0.5, 0.4, -0.8]
            .iter()
            .enumerate()
            .map(|(i, v)| at(i as f32 / 10.0, *v))
            .collect(),
        "usr" => (0..=24)
            .map(|i| {
                let t = i as f32 / 24.0;
                at(t, 0.7 * (t * std::f32::consts::TAU).sin() + 0.3 * (t * 3.0 * std::f32::consts::TAU).sin())
            })
            .collect(),
        _ => Vec::new(),
    };
    for pair in pts.windows(2) {
        c.line(pair[0], pair[1], 1.3, Cap::Round, &Paint::Solid(line));
    }
}
