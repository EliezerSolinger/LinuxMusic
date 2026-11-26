//! Guitarix's own widgets (legado/plugins/Guitarix/GuitarixWidgets.cpp), drawn as vectors at the screen's scale
//! (26/11 decision 6) in place of the upstream style's bitmap strips: the knobs with their LED rings, the switches,
//! the selector, the LED meters, the sliders, the value display, and the rack unit's skin (the brushed panel, the
//! handles and the screws). The numbers are the ones listed in docs/plugins-visual.md ("Widgets próprios").

use crate::raster::{Canvas, Cap, Paint, Rgba};
use crate::spec::{SurfText, SurfaceCx, SurfaceHandle};

/// `ControlColor`
pub const ORANGE: Rgba = Rgba::hex(0xff9000);
/// The unlit ring and the disabled control
pub const RING_OFF: Rgba = Rgba::hex(0x211006);
pub const ORANGE_HEX: u32 = 0xff9000;

/// The knob types (`GxSmallKnob`, `GxSmallKnobR`, `GxMidKnob`, `GxBigKnob`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GxKnob {
    Small,
    SmallR,
    Mid,
    Big,
}

struct KnobDims {
    img: (f32, f32),
    ring_r: f32,
    ring_w: f32,
    /// Dash and gap of the LED ring, absent: solid
    led: Option<(f32, f32)>,
    ind_r: f32,
    ind_w: f32,
    ind_len: f32,
}

impl GxKnob {
    fn dims(self) -> KnobDims {
        match self {
            GxKnob::Small => KnobDims {
                img: (38.0, 38.0),
                ring_r: 15.0,
                ring_w: 2.0,
                led: Some((3.0, 2.0)),
                ind_r: 11.0,
                ind_w: 2.0,
                ind_len: 5.0,
            },
            GxKnob::SmallR => KnobDims {
                img: (68.0, 68.0),
                ring_r: 26.0,
                ring_w: 3.0,
                led: Some((3.0, 2.0)),
                ind_r: 12.0,
                ind_w: 3.0,
                ind_len: 3.0,
            },
            GxKnob::Mid => KnobDims {
                img: (90.0, 90.0),
                ring_r: 35.0,
                ring_w: 6.0,
                led: Some((2.0, 2.0)),
                ind_r: 27.0,
                ind_w: 2.0,
                ind_len: 6.0,
            },
            GxKnob::Big => KnobDims {
                img: (114.0, 134.0),
                ring_r: 50.0,
                ring_w: 3.0,
                led: None,
                ind_r: 46.0,
                ind_w: 3.0,
                ind_len: 6.0,
            },
        }
    }

    /// The widget's size: the image and 2×8 / 2×4 around it
    pub fn size(self) -> (f32, f32) {
        let d = self.dims();
        (d.img.0 + 16.0, d.img.1 + 8.0)
    }
}

fn disc(c: &mut Canvas, cx: f32, cy: f32, d: f32, stops: Vec<(f32, Rgba)>, off: (f32, f32)) {
    let paint = Paint::Radial { cx: cx + off.0, cy: cy + off.1, radius: d * 0.62, stops };
    c.line((cx, cy), (cx, cy), d, Cap::Round, &paint);
}

/// A filled rounded rectangle (rows of half a pixel, each inset by the corner's circle)
pub fn rrect(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32, color: Rgba) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r <= 0.0 {
        c.fill_rect(x, y, w, h, color);
        return;
    }
    let rows = (h * 2.0).ceil() as i32;
    for n in 0..rows {
        let ry = y + n as f32 * 0.5;
        let mid = (ry + 0.25 - y).min(h - (ry + 0.25 - y));
        let inset = if mid < r {
            let dy = r - mid;
            r - (r * r - dy * dy).max(0.0).sqrt()
        } else {
            0.0
        };
        c.fill_rect(x + inset, ry, w - 2.0 * inset, 0.5, color);
    }
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    Rgba { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}

/// A Guitarix knob at `angle` (whole degrees from -135 to +135 on the legacy's 270° travel): the state is the
/// position on that travel, 0 to 1; Guitarix's own dead zone is 40° at the bottom on each side
pub fn draw_knob(c: &mut Canvas, kind: GxKnob, angle: i32, enabled: bool) {
    let d = kind.dims();
    let (w, h) = kind.size();
    let (cx, cy) = (w / 2.0, h / 2.0);
    let t = ((angle as f32 + 135.0) / 270.0).clamp(0.0, 1.0);
    let (hi, lo, rim) = if enabled {
        (Rgba::rgb(78, 78, 80), Rgba::rgb(22, 22, 24), Rgba::rgb(6, 6, 7))
    } else {
        (Rgba::rgb(52, 52, 52), Rgba::rgb(20, 20, 20), Rgba::rgb(6, 6, 6))
    };
    let cap = d.ring_r - d.ring_w / 2.0 - 1.0;
    // the cap: a dark rim, a shaded disc, and the dome in the middle
    disc(
        c,
        cx + 0.5,
        cy + 1.5,
        cap * 2.0 + 3.0,
        vec![(0.0, Rgba::alpha(0, 0, 0, 90)), (1.0, Rgba::alpha(0, 0, 0, 0))],
        (0.0, 0.0),
    );
    disc(c, cx, cy, cap * 2.0, vec![(0.0, rim), (1.0, rim)], (0.0, 0.0));
    disc(c, cx, cy, cap * 2.0 - 2.0, vec![(0.0, hi), (1.0, lo)], (-cap * 0.18, -cap * 0.22));
    disc(c, cx, cy, cap * 1.1, vec![(0.0, mix(hi, lo, 0.5)), (1.0, mix(lo, rim, 0.6))], (-cap * 0.08, -cap * 0.1));
    let on = if enabled { ORANGE } else { RING_OFF };
    // the ring: Qt's angles are the Cairo's negated; the travel starts at 130° (Cairo), lower left, and runs clockwise
    let (rx, ry, rd) = (cx - d.ring_r, cy - d.ring_r, d.ring_r * 2.0);
    let total = 280.0f32;
    let lit = t * total;
    match d.led {
        Some((dash, gap)) => {
            let pitch = (dash + gap) / d.ring_r;
            let (pitch_deg, dash_deg) = (pitch.to_degrees(), (dash / d.ring_r).to_degrees());
            let mut a = 0.0f32;
            while a + dash_deg * 0.5 <= total {
                let span = dash_deg.min(total - a);
                c.arc(rx, ry, rd, 230.0 - a, -span, d.ring_w, Cap::Flat, RING_OFF);
                if a < lit {
                    c.arc(rx, ry, rd, 230.0 - a, -span.min(lit - a), d.ring_w, Cap::Flat, on);
                }
                a += pitch_deg;
            }
        }
        None => {
            c.arc(rx, ry, rd, 230.0, -total, d.ring_w, Cap::Flat, RING_OFF);
            if lit > 0.0 {
                c.arc(rx, ry, rd, 230.0, -lit, d.ring_w, Cap::Flat, on);
            }
        }
    }
    // the indicator
    let a = (40.0 + t * total).to_radians();
    let at = |r: f32| (cx - a.sin() * r, cy + a.cos() * r);
    c.line(at(d.ind_r), at(d.ind_r - d.ind_len), d.ind_w, Cap::Flat, &Paint::Solid(on));
}

/// The switch types' widget sizes (`GxSwitch`: the image and 4 around it)
pub const SWITCH_SIZE: f32 = 24.0;
pub const MINITOGGLE_SIZE: f32 = 18.0;
pub const FRBUTTON_W: f32 = 29.0;
pub const FRBUTTON_H: f32 = 27.0;

/// `switch` (20×20): a lamp button, orange when on
pub fn draw_switch(c: &mut Canvas, on: bool) {
    let s = SWITCH_SIZE;
    rrect(c, 2.0, 2.0, s - 4.0, s - 4.0, 4.0, Rgba::rgb(14, 14, 14));
    rrect(c, 3.0, 3.0, s - 6.0, s - 6.0, 3.0, Rgba::rgb(52, 52, 54));
    let (a, b) = if on {
        (Rgba::rgb(255, 190, 90), Rgba::rgb(210, 110, 0))
    } else {
        (Rgba::rgb(56, 34, 18), Rgba::rgb(26, 14, 6))
    };
    disc(c, s / 2.0, s / 2.0, 11.0, vec![(0.0, a), (1.0, b)], (-2.0, -2.0));
}

/// `switch_mid_*` (14×14)
pub fn draw_switch_mid(c: &mut Canvas, on: bool) {
    let s = MINITOGGLE_SIZE;
    rrect(c, 2.0, 2.0, s - 4.0, s - 4.0, 3.0, Rgba::rgb(14, 14, 14));
    let (a, b) = if on {
        (Rgba::rgb(255, 190, 90), Rgba::rgb(210, 110, 0))
    } else {
        (Rgba::rgb(56, 34, 18), Rgba::rgb(26, 14, 6))
    };
    disc(c, s / 2.0, s / 2.0, 8.0, vec![(0.0, a), (1.0, b)], (-1.5, -1.5));
}

/// `minitoggle_*` (14×14): a small lever, up when on
pub fn draw_minitoggle(c: &mut Canvas, on: bool) {
    let s = MINITOGGLE_SIZE;
    rrect(c, 3.0, 1.0, s - 6.0, s - 2.0, 3.0, Rgba::rgb(14, 14, 14));
    let y = if on { 2.0 } else { s / 2.0 };
    let col = if on { ORANGE } else { Rgba::rgb(96, 62, 30) };
    rrect(c, 4.0, y, s - 8.0, s / 2.0 - 1.0, 2.0, col);
}

/// `frbutton_*` (25×23): a wide button with its lamp
pub fn draw_frbutton(c: &mut Canvas, on: bool) {
    let (w, h) = (FRBUTTON_W, FRBUTTON_H);
    rrect(c, 2.0, 2.0, w - 4.0, h - 4.0, 4.0, Rgba::rgb(14, 14, 14));
    let col = if on { Rgba::rgb(230, 130, 10) } else { Rgba::rgb(48, 30, 16) };
    rrect(c, 4.0, 4.0, w - 8.0, h - 8.0, 3.0, col);
    if on {
        rrect(c, 6.0, 5.0, w - 12.0, 3.0, 1.5, Rgba::alpha(255, 220, 140, 120));
    }
}

/// `GxSelector`: the dark frame, the black text area (the value is drawn as text over it), the arrow
pub fn draw_selector(c: &mut Canvas, w: f32) {
    let h = crate::widgets::COMBO_HEIGHT;
    rrect(c, 0.0, 0.0, w, h, 3.0, Rgba::hex(0x272727));
    rrect(c, 8.0, 3.0, w - 32.0, h - 6.0, 1.5, Rgba::rgb(0, 0, 0));
    // the inset's shade and the glass on top
    c.fill_rect(8.0, 3.0, w - 32.0, 1.0, Rgba::alpha(0, 0, 0, 120));
    c.fill_rect(8.0, h - 4.0, w - 32.0, 1.0, Rgba::alpha(255, 255, 255, 26));
    for k in 0..3 {
        c.fill_rect(8.0, 4.0 + k as f32, w - 32.0, 1.0, Rgba::alpha(0, 0, 0, 90 - k * 30));
    }
    // `selector_icon`: a little up-down arrow centered on the right border
    let (ax, ay) = (w - 19.5, h / 2.0);
    let col = Paint::Solid(Rgba::hex(0xb0b0b0));
    c.line((ax - 4.0, ay - 1.0), (ax, ay - 4.5), 1.5, Cap::Round, &col);
    c.line((ax, ay - 4.5), (ax + 4.0, ay - 1.0), 1.5, Cap::Round, &col);
    c.line((ax - 4.0, ay + 1.0), (ax, ay + 4.5), 1.5, Cap::Round, &col);
    c.line((ax, ay + 4.5), (ax + 4.0, ay + 1.0), 1.5, Cap::Round, &col);
}

/// The text of a selector's value: where it goes in the widget (its baseline and center) and its color
pub fn selector_text_pos(w: f32) -> (f32, f32) {
    (8.0 + (w - 32.0) / 2.0, 14.0)
}

/// A notebook tab (`GtkNotebook`'s style sheet: transparent, text `#995600`, selected `#ff9000`)
pub fn draw_tab(c: &mut Canvas, on: bool) {
    let w = c.width as f32 / c.scale;
    let h = c.height as f32 / c.scale;
    let col = if on { Rgba::hex(0xff9000) } else { Rgba::hex(0x995600) };
    c.fill_rect(0.0, h - 2.0, w, 2.0, col);
    if on {
        c.fill_rect(0.0, 0.0, w, h - 2.0, Rgba::alpha(255, 144, 0, 28));
    }
}

/// `box_uni_2_expose`: the unit's dark panel (`background2`)
pub fn draw_dark(c: &mut Canvas, _on: bool) {
    let w = c.width as f32 / c.scale;
    let h = c.height as f32 / c.scale;
    c.fill_rect(0.0, 0.0, w, h, Rgba::hex(0x1c1c1c));
    for k in 0..6 {
        let x = w * (k as f32 + 0.5) / 6.0;
        c.fill_rect(x - w / 12.0, 0.0, w / 6.0, h, Rgba::alpha(255, 255, 255, if k % 2 == 0 { 5 } else { 0 }));
    }
    c.fill_rect(0.0, 0.0, w, 1.0, Rgba::alpha(0, 0, 0, 120));
    c.fill_rect(0.0, h - 1.0, w, 1.0, Rgba::alpha(255, 255, 255, 18));
}

/// The rack unit: `paintSkin` (the brushed panel, the bevel, the shadow), the two handles and the screws
pub fn draw_skin(c: &mut Canvas, _on: bool) {
    let w = c.width as f32 / c.scale;
    let h = c.height as f32 / c.scale;
    let hp = h - 4.0;
    // the brushed panel: vertical bands of light, as `background1`
    c.fill_rect(0.0, 0.0, w, hp, Rgba::hex(0x7a9a58));
    let bands = (w / 6.0).ceil().max(1.0) as i32;
    for k in 0..bands {
        let x = k as f32 * 6.0;
        let u = (x / 200.0).sin() * 0.5 + (x / 57.0).sin() * 0.3;
        let col = if u > 0.0 {
            Rgba::alpha(255, 255, 255, (u * 26.0) as u8)
        } else {
            Rgba::alpha(0, 0, 0, (-u * 30.0) as u8)
        };
        c.fill_rect(x, 0.0, 6.0, hp, col);
    }
    // the bevel: white 0.055 at the top to black 0.11 at the bottom
    let rows = hp.ceil() as i32;
    for y in 0..rows {
        let t = y as f32 / hp.max(1.0);
        let top = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.055 * (1.0 - t) };
        let bottom = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.11 * t };
        c.fill_rect(0.0, y as f32, w, 1.0, top);
        c.fill_rect(0.0, y as f32, w, 1.0, bottom);
    }
    // the shadow under the panel: black 0.8 fading over the last 4 pixels
    for k in 0..4 {
        c.fill_rect(0.0, hp + k as f32, w, 1.0, Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.8 * (1.0 - k as f32 / 4.0) });
    }
    // the handles
    let handle = Rgba::hex(0x1d1d1e);
    c.fill_rect(0.0, 0.0, 32.0, hp, handle);
    c.fill_rect(w - 32.0, 0.0, 32.0, hp, handle);
    for k in 0..(hp / 4.0).ceil() as i32 {
        let y = k as f32 * 4.0;
        c.fill_rect(0.0, y, 32.0, 1.0, Rgba::alpha(255, 255, 255, 8));
        c.fill_rect(w - 32.0, y, 32.0, 1.0, Rgba::alpha(255, 255, 255, 8));
    }
    c.fill_rect(31.0, 0.0, 1.0, hp, Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.8 });
    c.fill_rect(32.0, 0.0, 1.0, hp, Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.15 });
    c.fill_rect(w - 33.0, 0.0, 1.0, hp, Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.5 });
    c.fill_rect(w - 32.0, 0.0, 1.0, hp, Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.05 });
    // the screws (32×32 each): four when the panel is tall, else two at the middle
    let hs = h - 3.0;
    let spots: Vec<(f32, f32)> = if hs > 64.0 {
        vec![(0.0, 0.0), (0.0, hs - 32.0), (w - 32.0, hs - 32.0), (w - 32.0, 0.0)]
    } else if hs > 32.0 {
        vec![(0.0, hs / 2.0 - 16.0), (w - 32.0, hs / 2.0 - 16.0)]
    } else {
        Vec::new()
    };
    for (sx, sy) in spots {
        let (cx, cy) = (sx + 16.0, sy + 16.0);
        disc(c, cx, cy, 15.0, vec![(0.0, Rgba::rgb(58, 58, 60)), (1.0, Rgba::rgb(10, 10, 11))], (-2.0, -2.0));
        c.line((cx - 4.5, cy + 4.5), (cx + 4.5, cy - 4.5), 2.0, Cap::Round, &Paint::Solid(Rgba::rgb(4, 4, 4)));
    }
}

// ---- the surfaces: the meters, the sliders and the value display. `SurfaceSpec::tag` tells each what it is.

/// A meter's tag: the output's place in the frame, horizontal, logarithmic, and the range of a linear one
pub fn meter_tag(index: u32, horizontal: bool, log: bool, lo: i32, hi: i32) -> u32 {
    index
        | (u32::from(horizontal) << 4)
        | (u32::from(log) << 5)
        | (((lo & 0xfff) as u32) << 6)
        | (((hi & 0xfff) as u32) << 18)
}

fn signed12(v: u32) -> f32 {
    let v = (v & 0xfff) as i32;
    (if v >= 0x800 { v - 0x1000 } else { v }) as f32
}

/// Guitarix's `log_meter`: dB to a deflection of 0 to 115 (the IEC 60268 scale)
fn log_meter(db: f32) -> f32 {
    if db < -70.0 {
        0.0
    } else if db < -60.0 {
        (db + 70.0) * 0.25
    } else if db < -50.0 {
        (db + 60.0) * 0.5 + 2.5
    } else if db < -40.0 {
        (db + 50.0) * 0.75 + 7.5
    } else if db < -30.0 {
        (db + 40.0) * 1.5 + 15.0
    } else if db < -20.0 {
        (db + 30.0) * 2.0 + 30.0
    } else {
        (db + 20.0) * 2.5 + 50.0
    }
    .min(115.0)
}

/// `GxFastMeter`: 2×2 LEDs on a 3-pixel pitch from the bottom (or the left), the lit ones in the gradient
/// brown, yellow, red, the others the same under black at 0.8
pub fn draw_meter(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let tag = cx.tag;
    let index = (tag & 0xf) as usize;
    let horizontal = tag & 0x10 != 0;
    let log = tag & 0x20 != 0;
    let (lo, hi) = (signed12(tag >> 6), signed12(tag >> 18));
    let v = cx.live(index, 0.0);
    let level = if log {
        let db = 20.0 * v.max(1.0e-9).log10();
        log_meter(db) / 115.0
    } else if hi > lo {
        (v - lo) / (hi - lo)
    } else {
        v
    }
    .clamp(0.0, 1.0);
    let len = if horizontal { cx.w } else { cx.h };
    let n = ((len - 1.0) / 3.0).floor().max(1.0) as i32;
    let thick = if horizontal { cx.h } else { cx.w };
    c.fill_rect(0.0, 0.0, cx.w, cx.h, Rgba::hex(0x222222));
    let lit = (n as f32 * level).round() as i32;
    let stops =
        [(0.0, Rgba::hex(0xb96901)), (0.717, Rgba::hex(0xd4cd00)), ((n as f32 - 2.0) / n as f32, Rgba::hex(0xff0500))];
    for k in 0..n {
        let f = (k as f32 + 0.5) / n as f32;
        let col = if f >= stops[2].0 {
            stops[2].1
        } else if f >= stops[1].0 {
            mix(stops[1].1, stops[2].1, (f - stops[1].0) / (stops[2].0 - stops[1].0).max(0.001))
        } else {
            mix(stops[0].1, stops[1].1, f / stops[1].0)
        };
        let col = if k < lit { col } else { mix(col, Rgba::rgb(0, 0, 0), 0.8) };
        let along = 1.0 + k as f32 * 3.0;
        let leds = ((thick - 1.0) / 3.0).floor().max(1.0) as i32;
        for j in 0..leds {
            let across = 1.0 + j as f32 * 3.0;
            if horizontal {
                c.fill_rect(along, across, 2.0, 2.0, col);
            } else {
                c.fill_rect(across, cx.h - along - 2.0, 2.0, 2.0, col);
            }
        }
    }
}

const SLIDER_SPAN: f32 = 98.0;
const SLIDER_CURSOR: f32 = 36.0;

fn slider_state(cx: &SurfaceCx<'_>) -> f32 {
    cx.unit(cx.primary) as f32
}

/// `GxVSlider` and `GxHSlider` (bit 1 of the tag: horizontal): the groove and the cursor
pub fn draw_slider(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let horizontal = cx.tag & 2 != 0;
    let t = slider_state(cx);
    let groove = Rgba::rgb(14, 14, 14);
    if horizontal {
        rrect(c, 1.0, 9.0, cx.w - 2.0, 6.0, 3.0, groove);
        rrect(c, 2.0, 10.0, cx.w - 4.0, 2.0, 1.0, Rgba::rgb(40, 40, 40));
        let x = t * SLIDER_SPAN;
        rrect(c, x, 1.0, SLIDER_CURSOR, 22.0, 4.0, Rgba::rgb(10, 10, 10));
        rrect(c, x + 1.0, 2.0, SLIDER_CURSOR - 2.0, 20.0, 3.0, Rgba::rgb(70, 70, 72));
        c.fill_rect(x + SLIDER_CURSOR / 2.0 - 1.0, 4.0, 2.0, 16.0, ORANGE);
    } else {
        rrect(c, 9.0, 1.0, 6.0, cx.h - 2.0, 3.0, groove);
        rrect(c, 10.0, 2.0, 2.0, cx.h - 4.0, 1.0, Rgba::rgb(40, 40, 40));
        let y = SLIDER_SPAN - t * SLIDER_SPAN;
        rrect(c, 1.0, y, 22.0, SLIDER_CURSOR, 4.0, Rgba::rgb(10, 10, 10));
        rrect(c, 2.0, y + 1.0, 20.0, SLIDER_CURSOR - 2.0, 3.0, Rgba::rgb(70, 70, 72));
        c.fill_rect(4.0, y + SLIDER_CURSOR / 2.0 - 1.0, 16.0, 2.0, ORANGE);
    }
}

pub fn slider_handles(cx: &SurfaceCx<'_>) -> Vec<SurfaceHandle> {
    let t = slider_state(cx);
    if cx.tag & 2 != 0 {
        vec![SurfaceHandle { x: t * SLIDER_SPAN + SLIDER_CURSOR / 2.0, y: cx.h / 2.0 }]
    } else {
        vec![SurfaceHandle { x: cx.w / 2.0, y: SLIDER_SPAN - t * SLIDER_SPAN + SLIDER_CURSOR / 2.0 }]
    }
}

/// The parameter's value for the cursor dragged to a local point
pub fn slider_drag(cx: &SurfaceCx<'_>, _handle: usize, x: f32, y: f32) -> Vec<(&'static str, f64)> {
    let t = if cx.tag & 2 != 0 {
        (x - SLIDER_CURSOR / 2.0) / SLIDER_SPAN
    } else {
        1.0 - (y - SLIDER_CURSOR / 2.0) / SLIDER_SPAN
    }
    .clamp(0.0, 1.0) as f64;
    let (lo, hi) = (cx.range)(cx.primary);
    let v = if cx.tag & 1 != 0 && lo > 0.0 { lo * (hi / lo).powf(t) } else { lo + (hi - lo) * t };
    vec![(cx.primary, v)]
}

/// `GxValueDisplay`: a black rounded field with the value in orange (the text is drawn over it)
pub fn draw_value(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    rrect(c, 0.0, 0.0, cx.w, cx.h, 6.0, Rgba::rgb(0, 0, 0));
    for k in 0..4 {
        c.fill_rect(5.0, 1.0 + k as f32, cx.w - 10.0, 1.0, Rgba::alpha(255, 255, 255, 14 - k as u8 * 3));
    }
}

pub fn value_texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
    let digits = (cx.tag & 0xf) as usize;
    let v = (cx.param)(cx.primary);
    vec![SurfText {
        x: cx.w / 2.0,
        y: cx.h / 2.0 + 4.0,
        px: 10.0,
        text: format!("{v:.digits$}"),
        color: ORANGE_HEX,
        centered: true,
    }]
}

/// The skin as a surface (the panel's whole area)
pub fn skin_surface(c: &mut Canvas, _cx: &SurfaceCx<'_>) {
    draw_skin(c, false);
}

/// The dark box as a surface
pub fn dark_surface(c: &mut Canvas, _cx: &SurfaceCx<'_>) {
    draw_dark(c, false);
}
