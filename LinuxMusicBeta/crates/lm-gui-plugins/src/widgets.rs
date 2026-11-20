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
            KnobStyle::Styled(_) => None,
        }
    }

    /// The knob's drawn size (without its label)
    pub fn size(&self) -> (f32, f32) {
        match self {
            KnobStyle::Styled(s) => (s.w, s.h),
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
        let v = v.clamp(self.min, self.max);
        if self.step > 0.0 {
            // rounded at the step, then clamped again (the rounding may pass the end)
            ((v / self.step).round() * self.step).clamp(self.min, self.max)
        } else {
            v
        }
    }

    /// `inverseScaledValue`: a value to the linear travel (in the value's own range)
    pub fn inverse_scaled(&self, v: f64) -> f64 {
        if !self.log || self.span() == 0.0 {
            return v;
        }
        let v = v.clamp(self.min, self.max);
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
            c.pixmap(img, 0.0, 0.0, !enabled);
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
                KnobStyle::Styled(_) => {}
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

/// The LCD's size without its label (`LcdWidget::updateSize`)
pub fn lcd_size(sheet: LcdSheet, digits: usize) -> (f32, f32) {
    let (cw, ch) = sheet.cell();
    let margin_w = cw / 2;
    ((cw * digits as u32 + 2 * (1 + margin_w)) as f32, (ch + 2) as f32)
}

/// `LcdWidget::paintEvent` without the label: the left margin, the blank padding, the glyphs, the right margin, and
/// the sunken frame of `LmmsStyle::drawPrimitive(PE_Frame)`
pub fn draw_lcd(c: &mut Canvas, sheet: LcdSheet, digits: usize, text: &str, enabled: bool) {
    let img = assets::image(sheet.asset());
    let (cw, ch) = sheet.cell();
    let mw = cw / 2;
    let row = if enabled { 0 } else { ch };
    let mut x = 1.0;
    let y = 1.0;
    c.pixmap_part(img, (LCD_CELLS * cw, row, cw, ch), x, y, false);
    x += mw as f32;
    let shown: Vec<char> = text.chars().collect();
    for _ in shown.len()..digits {
        c.pixmap_part(img, (10 * cw, row, cw, ch), x, y, false);
        x += cw as f32;
    }
    for d in shown {
        let g = match d.to_digit(10) {
            Some(v) => v,
            None if d == '-' => 11,
            None => 10,
        };
        c.pixmap_part(img, (g * cw, row, cw, ch), x, y, false);
        x += cw as f32;
    }
    // the right margin: half a cell of the margin column, squeezed into marginWidth - 1 pixels
    let right_w = mw.saturating_sub(1);
    for i in 0..right_w {
        let sx = LCD_CELLS * cw + (i * (cw / 2)) / right_w.max(1);
        c.pixmap_part(img, (sx, row, 1, ch), x + i as f32, y, false);
    }
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
    let lastx = ((last_x - 2.0) as i32).clamp(0, g.w as i32 - 5);
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
    let v = v.clamp(g.min, g.max);
    if g.step > 0.0 { ((v / g.step).round() * g.step).clamp(g.min, g.max) } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lin(min: f64, max: f64, step: f64) -> Range {
        Range { min, max, step, center: min, log: false }
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
