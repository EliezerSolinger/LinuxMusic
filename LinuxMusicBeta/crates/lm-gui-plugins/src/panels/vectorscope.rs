//! Vectorscope (legado/plugins/Vectorscope/VecControlsDialog.cpp, VectorView.cpp): the legacy's size hint, 275×300:
//! the black view (a square of 275 px, its center at half the width and half the smaller side, `scale = side / 4.1`)
//! and under it the row with the two LEDs, "Log. scale" on the left and "Lines" on the right.
//!
//! The view draws the plugin's live frame (`lm-plugins-effects`' `vectorscope`: the last frames, oldest first)
//! as the legacy's `paintEvent`: each frame is a point at (left − right, −(left + right)) in the units where a full
//! scale mono signal reaches 2 (on the logarithmic scale each frame's distance from the origin is replaced by
//! `log10(1 + 9 d)`), joined to the one before with a line, or alone as a point, 2 px wide; the older the frame the
//! brighter: frame `i` is the trace color darkened by `100 + i` percent, as the legacy. Over it the grid: a circle of
//! radius 2, the two dotted diagonals, and the L and R labels (26 px; the legacy rotates them, here they sit where
//! the rotation puts them). The colors are the Linux theme's (`VectorView`).
//!
//! Not ported: the wheel's zoom (20 to 1000 %, not saved by the legacy either) and the double click's color chooser.

use std::sync::LazyLock;

use crate::raster::{Canvas, Cap, Paint, Rgba};
use crate::spec::{Bind, PanelSpec, SurfText, SurfaceCx, SurfaceSpec, led, surface};
use crate::widgets::LedColor;

pub const ID: &str = "vectorscope";

const W: f32 = 275.0;
const VIEW_H: f32 = 278.0;
const TRACE: (u8, u8, u8) = (43, 212, 168);
const GRID: Rgba = Rgba::alpha(76, 80, 84, 128);
const LABELS: u32 = 0x4c5054;
/// The frames of the plugin's frame (`lm_plugins_effects::vectorscope::POINTS`)
const POINTS: usize = 1024;

/// `VectorView`'s geometry: the center and the scale of the grid
fn geometry(w: f32, h: f32) -> (f32, f32, f32) {
    let side = w.min(h);
    (w / 2.0, side / 2.0, side / 4.1)
}

fn dotted(c: &mut Canvas, a: (f32, f32), b: (f32, f32), width: f32, paint: &Paint) {
    // `Qt::DotLine`: a pen's width on, a pen's width off
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    let (ux, uy) = (dx / len, dy / len);
    let mut k = 0.0;
    while k < len {
        let e = (k + width).min(len);
        c.line((a.0 + ux * k, a.1 + uy * k), (a.0 + ux * e, a.1 + uy * e), width, Cap::Flat, paint);
        k += 2.0 * width;
    }
}

fn draw(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let (w, h) = (cx.w, cx.h);
    c.fill_rect(0.0, 0.0, w, h, Rgba::rgb(0, 0, 0));
    let (mx, my, scale) = geometry(w, h);
    let log = (cx.param)("logarithmic") >= 0.5;
    let lines = (cx.param)("linesMode") >= 0.5;
    let count = (cx.live(0, 0.0) as usize).min(POINTS);
    // the newest `count` frames are the last ones of the frame
    let first = POINTS - count;
    let mut last: Option<(f32, f32)> = None;
    for i in 0..count {
        let (mut l, mut r) = (cx.live(1 + 2 * (first + i), 0.0), cx.live(2 + 2 * (first + i), 0.0));
        if log {
            let distance = (l * l + r * r).sqrt();
            if distance != 0.0 {
                let factor = (1.0 + 9.0 * distance.abs()).log10() / distance;
                l *= factor;
                r *= factor;
            }
        }
        // side across, mid up (the legacy's y is flipped, so "−mid" is down on the screen)
        let (side, mid) = (l - r, l + r);
        let p = (mx + scale * side, my + scale * mid);
        // `QColor::darker(100 + i)`: the value divided by (100 + i) / 100
        let k = 100.0 / (100.0 + i as f32);
        let paint = Paint::Solid(Rgba::rgb(
            (f32::from(TRACE.0) * k) as u8,
            (f32::from(TRACE.1) * k) as u8,
            (f32::from(TRACE.2) * k) as u8,
        ));
        match last {
            Some(q) if lines && q != p => c.line(q, p, 2.0, Cap::Round, &paint),
            _ => c.line(p, p, 2.0, Cap::Round, &paint),
        }
        last = Some(p);
    }
    // the grid over the trace: the circle of radius 2 and the two dotted diagonals
    let grid = Paint::Solid(GRID);
    c.arc(mx - 2.0 * scale, my - 2.0 * scale, 4.0 * scale, 0.0, 360.0, 2.5, Cap::Round, GRID);
    let root = 2.1f32.sqrt() * scale;
    dotted(c, (mx, my), (mx - root, my - root), 2.5, &grid);
    dotted(c, (mx, my), (mx + root, my - root), 2.5, &grid);
}

/// The L and R, where the legacy's rotated text lands (unrotated, at its baseline)
fn texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
    let (mx, my, _) = geometry(cx.w, cx.h);
    let side = cx.w.min(cx.h);
    let up = side / 2.0 + 10.0;
    let k = std::f32::consts::FRAC_1_SQRT_2;
    // half the glyph's width, as the legacy centers it before rotating
    let half = 9.0;
    let (lx, ly) = (mx - k * (half + up), my - k * (up - half));
    let (rx, ry) = (mx + k * (up - half), my - k * (up + half));
    vec![
        SurfText { x: lx, y: ly, px: 26.0, text: "L".to_owned(), color: LABELS, centered: false },
        SurfText { x: rx, y: ry, px: 26.0, text: "R".to_owned(), color: LABELS, centered: false },
    ]
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| PanelSpec {
    plugin: ID,
    size: (W, 300.0),
    background: None,
    view_backgrounds: &[],
    fill: 0x101116,
    controls: vec![
        surface(
            SurfaceSpec {
                w: W,
                h: VIEW_H,
                primary: "logarithmic",
                reads: &["logarithmic", "linesMode"],
                field_reads: &[],
                draw,
                texts: Some(texts),
                handles: None,
                drag: None,
                grab: 0.0,
                set_on_press: false,
                live: true,
                wheel: false,
                wheel_edit: None,
                peaks: None,
                markers: None,
                tag: 0,
            },
            "view",
            0.0,
            0.0,
            "Vectorscope",
        ),
        led(
            Bind::Switch { param: "logarithmic" },
            "logarithmic",
            5.0,
            283.0,
            LedColor::Green,
            "Log. scale",
            "Logarithmic scale",
        )
        .tip("Display amplitude on logarithmic scale to better see small values."),
        led(
            Bind::Switch { param: "linesMode" },
            "linesMode",
            215.0,
            283.0,
            LedColor::Green,
            "Lines",
            "Lines rendering",
        )
        .tip("Render with lines."),
    ],
    labels: Vec::new(),
    file_drop: None,
});
