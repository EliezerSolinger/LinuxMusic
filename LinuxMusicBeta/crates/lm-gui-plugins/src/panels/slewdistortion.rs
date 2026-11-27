//! SlewDistortion (legado/plugins/SlewDistortion/SlewDistortionControlDialog.cpp): the 638×271 artwork with its two
//! bands (the upper at y 26, the lower at y 147). Each band has a type combo box, the Slew Up/Down knobs with the
//! link button (linked, the Down knob turns Slew Up: the legacy swaps the knob's model), the dynamics, attack,
//! release, warp, crush and mix knobs, and three `Draggable` handles (drive at x 16, bias at x 416, output volume at
//! x 594, each on its own vertical track, the zero pixmap while the value is exactly 0). In the middle the two 100×100
//! transfer curves with their grid, drawn from the band's type, drive, bias, warp and crush exactly as the legacy
//! `paintEvent` does, and beside the handles the input and output meters (the dark cover over the artwork's meter,
//! from the live frame: four input peaks then four output peaks). Below: the DC and multiband switches, the split knob, the oversampling group.
//!
//! The help button (a window of text) is not in the panel.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::{Canvas, Cap, Paint, Rgba};
use crate::spec::{
    Bind, ComboItem, Control, PanelSpec, SurfaceCx, SurfaceSpec, Text, button, combo, knob, slider, surface,
};
use crate::widgets::KnobStyle;

pub const ID: &str = "slewdistortion";

static ARTWORK: Asset = asset!("slewdistortion/artwork.png");
static HANDLE: Asset = asset!("slewdistortion/handle.png");
static HANDLE_ZERO: Asset = asset!("slewdistortion/handle_zero.png");
static LINK_ON: Asset = asset!("slewdistortion/link_on.png");
static LINK_OFF: Asset = asset!("slewdistortion/link_off.png");
static DC_ON: Asset = asset!("slewdistortion/dc_on.png");
static DC_OFF: Asset = asset!("slewdistortion/dc_off.png");
static MB_ON: Asset = asset!("slewdistortion/mb_on.png");
static MB_OFF: Asset = asset!("slewdistortion/mb_off.png");
static OS_1_ON: Asset = asset!("slewdistortion/oversample_1x_on.png");
static OS_1_OFF: Asset = asset!("slewdistortion/oversample_1x_off.png");
static OS_2_ON: Asset = asset!("slewdistortion/oversample_2x_on.png");
static OS_2_OFF: Asset = asset!("slewdistortion/oversample_2x_off.png");
static OS_4_ON: Asset = asset!("slewdistortion/oversample_4x_on.png");
static OS_4_OFF: Asset = asset!("slewdistortion/oversample_4x_off.png");
static OS_8_ON: Asset = asset!("slewdistortion/oversample_8x_on.png");
static OS_8_OFF: Asset = asset!("slewdistortion/oversample_8x_off.png");
static OS_16_ON: Asset = asset!("slewdistortion/oversample_16x_on.png");
static OS_16_OFF: Asset = asset!("slewdistortion/oversample_16x_off.png");
static OS_32_ON: Asset = asset!("slewdistortion/oversample_32x_on.png");
static OS_32_OFF: Asset = asset!("slewdistortion/oversample_32x_off.png");

pub static ASSETS: &[&Asset] = &[
    &ARTWORK,
    &HANDLE,
    &HANDLE_ZERO,
    &LINK_ON,
    &LINK_OFF,
    &DC_ON,
    &DC_OFF,
    &MB_ON,
    &MB_OFF,
    &OS_1_ON,
    &OS_1_OFF,
    &OS_2_ON,
    &OS_2_OFF,
    &OS_4_ON,
    &OS_4_OFF,
    &OS_8_ON,
    &OS_8_OFF,
    &OS_16_ON,
    &OS_16_OFF,
    &OS_32_ON,
    &OS_32_OFF,
];

/// The shapes, in the legacy combo's order (`lm_dsp::slew_distortion::SlewShape::ALL`)
const SHAPES: [(&str, &str); 10] = [
    ("hard-clip", "Hard Clip"),
    ("tanh", "Tanh"),
    ("fast-soft-clip-1", "Fast Soft Clip 1"),
    ("fast-soft-clip-2", "Fast Soft Clip 2"),
    ("sinusoidal", "Sinusoidal"),
    ("foldback", "Foldback"),
    ("full-rectify", "Full Rectify"),
    ("half-rectify", "Half Rectify"),
    ("smooth-rectify", "Smooth Rectify"),
    ("bitcrush", "Bitcrush"),
];

const PI: f32 = std::f32::consts::PI;

/// The legacy's `SLEW_DISTORTION_MIN_FLOOR` (-72 dBFS)
const FLOOR: f32 = 0.0012589;

/// One shape's output for the input `d` (after the bias, warp and crush offset), as the dialog's `switch`
fn shape(key: &str, d: f32, drive: f32) -> f32 {
    match key {
        "hard-clip" => d.clamp(-1.0, 1.0),
        "tanh" => {
            let t = d.clamp(-40.0, 40.0);
            2.0 / (1.0 + (-2.0 * t).exp()) - 1.0
        }
        "fast-soft-clip-1" => {
            let t = d.clamp(-2.0, 2.0);
            t / (1.0 + 0.25 * t * t)
        }
        "fast-soft-clip-2" => {
            let t = d.clamp(-1.5, 1.5);
            t - (4.0 / 27.0) * t * t * t
        }
        "sinusoidal" => {
            let mut m = (d - PI * 0.5) % (2.0 * PI);
            if m < 0.0 {
                m += 2.0 * PI;
            }
            let x = (m - PI).abs() - PI * 0.5;
            let x2 = x * x;
            let x3 = x2 * x;
            let x5 = x3 * x2;
            x - x3 / 6.0 + x5 / 120.0
        }
        "foldback" => (((d - 1.0) % 4.0).abs() - 2.0).abs() - 1.0,
        "full-rectify" => d.abs(),
        "smooth-rectify" => (d * d + 0.04).sqrt() - 0.2,
        "half-rectify" => d.max(0.0),
        "bitcrush" => {
            let scale = 16.0 / drive;
            (d / drive * scale).round() / scale
        }
        _ => d,
    }
}

/// `drawCurve`: the polygon of the band's transfer curve in the rectangle (`x0`, `y0`, 100×100)
fn curve(c: &mut Canvas, cx: &SurfaceCx<'_>, band: usize, x0: f32, y0: f32) {
    let n = band + 1;
    let ty = (cx.field)(if band == 0 { "type1" } else { "type2" });
    let key = ty.as_text().unwrap_or("hard-clip").to_owned();
    let get = |k: &str| (cx.param)(&format!("{k}{n}")) as f32;
    let drive = 10f32.powf(get("drive") / 20.0);
    let bias = get("bias");
    let warp = get("warp");
    let crush = 10f32.powf(get("crush") / 20.0);
    let (w, h) = (100.0f32, 100.0f32);
    let pen = 2.0f32;
    let amplitude = (h - pen) / h;
    let steps = 200;
    let paint = Paint::Solid(Rgba::rgb(34, 226, 108));
    let mut last: Option<(f32, f32)> = None;
    for i in 0..=steps {
        let x = -1.0 + 2.0 * i as f32 / steps as f32;
        let biased = x * drive + bias;
        let dist_in = (biased - (warp / crush).copysign(biased)) / (1.0 - warp);
        let mut out = shape(&key, dist_in, drive);
        out = out * (1.0 - warp) + warp.copysign(biased);
        if biased.abs() < warp / crush {
            out = biased * crush;
        }
        out *= amplitude;
        let px = x0 + (x + 1.0) * 0.5 * w;
        let py = y0 + h - 1.0 - (out + 1.0) * 0.5 * h + pen / 2.0;
        if !(px.is_finite() && py.is_finite()) {
            last = None;
            continue;
        }
        if let Some((lx, ly)) = last
            && let Some((a, b)) = clip((lx, ly), (px, py), (x0, y0, x0 + w, y0 + h))
        {
            c.line(a, b, pen, Cap::Flat, &paint);
        }
        last = Some((px, py));
    }
}

/// Liang-Barsky: the part of the segment inside the rectangle (`setClipRect`)
fn clip(a: (f32, f32), b: (f32, f32), r: (f32, f32, f32, f32)) -> Option<((f32, f32), (f32, f32))> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, a.0 - r.0), (dx, r.2 - a.0), (-dy, a.1 - r.1), (dy, r.3 - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t1 {
                    return None;
                }
                t0 = t0.max(t);
            } else {
                if t < t0 {
                    return None;
                }
                t1 = t1.min(t);
            }
        }
    }
    Some(((a.0 + t0 * dx, a.1 + t0 * dy), (a.0 + t1 * dx, a.1 + t1 * dy)))
}

/// A dotted line of the grid (`Qt::DotLine`, one pixel of pen: one on, one off), antialiased
fn dotted(c: &mut Canvas, a: (f32, f32), b: (f32, f32), paint: &Paint) {
    let horizontal = a.1 == b.1;
    let len = if horizontal { b.0 - a.0 } else { b.1 - a.1 };
    let mut k = 0.0;
    while k < len {
        let (p, q) =
            if horizontal { ((a.0 + k, a.1), (a.0 + k + 1.0, a.1)) } else { ((a.0, a.1 + k), (a.0, a.1 + k + 1.0)) };
        c.line(p, q, 1.0, Cap::Flat, paint);
        k += 2.0;
    }
}

/// `paintEvent`: the meters' dark covers, the two grids and axes and the two curves
fn draw(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    // the covers: the part of each 8×75 meter above the level (-24 to 24 dBFS) is painted dark
    let meters = [
        (22.0, 31.0, 0, false),
        (30.0, 31.0, 1, false),
        (22.0, 152.0, 2, false),
        (30.0, 152.0, 3, false),
        (600.0, 31.0, 0, true),
        (608.0, 31.0, 1, true),
        (600.0, 152.0, 2, true),
        (608.0, 152.0, 3, true),
    ];
    for (x, y, i, out) in meters {
        let v = cx.live(i + if out { 4 } else { 0 }, FLOOR).max(FLOOR);
        let db = 20.0 * v.log10();
        let norm = ((db + 24.0) / 48.0).clamp(0.0, 1.0);
        let covered = (75.0 * (1.0 - norm)).trunc();
        c.fill_rect(x, y, 8.0, covered, Rgba::rgb(10, 10, 10));
    }
    let grid = Paint::Solid(Rgba::rgb(36, 40, 48));
    let axis = Paint::Solid(Rgba::rgb(62, 66, 75));
    for (i, (x0, y0)) in [(452.0f32, 10.0f32), (452.0, 131.0)].into_iter().enumerate() {
        let (right, bottom) = (x0 + 99.0, y0 + 99.0);
        for k in 1..8 {
            let x = x0 + (k * 100 / 8) as f32 + 1.0;
            dotted(c, (x, y0 + 1.0), (x, bottom), &grid);
            let y = y0 + (k * 100 / 8) as f32 + 1.0;
            dotted(c, (x0 + 1.0, y), (right, y), &grid);
        }
        let (mx, my) = (x0 + 49.0 + 2.0, y0 + 49.0 + 2.0);
        c.line((mx, y0 + 1.0), (mx, bottom), 2.0, Cap::Square, &axis);
        c.line((x0 + 1.0, my), (right, my), 2.0, Cap::Square, &axis);
        curve(c, cx, i, x0, y0);
    }
}

fn items() -> Vec<ComboItem> {
    SHAPES.iter().map(|(key, text)| ComboItem { key, text: Text::new(text), icon: None }).collect()
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls: Vec<Control> = Vec::new();
    // under everything: the curves, the grids and the meters' covers
    controls.push(surface(
        SurfaceSpec {
            w: 638.0,
            h: 271.0,
            primary: "drive1",
            reads: &["drive1", "bias1", "warp1", "crush1", "drive2", "bias2", "warp2", "crush2"],
            field_reads: &["type1", "type2"],
            draw,
            texts: None,
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
        "curves",
        0.0,
        0.0,
        "Transfer curves",
    ));
    let k = |p: &'static str, x, y, name, hint| knob(p, x, y, KnobStyle::Bright26, 0.0001, name, hint, "");
    for (band, dy) in [(1usize, 0.0f32), (2, 121.0)] {
        let key = |base: &str| -> &'static str { lm_port::intern(&format!("{base}{band}")) };
        let nm = |s: &str| -> &'static str { lm_port::intern(&format!("{s} {band}")) };
        let hint = |s: &str| -> &'static str { lm_port::intern(&format!("{s} {band}:")) };
        controls.push(combo(
            if band == 1 { "type1" } else { "type2" },
            85.0,
            26.0 + dy,
            115.0,
            if band == 1 { "Type 1" } else { "Type 2" },
            items(),
        ));
        // the handles' tracks: the minimum's y and the maximum's (the upper band's, then the lower's)
        let (drive, bias) = if band == 1 { ((108.0, 34.0), (112.0, 34.0)) } else { ((229.0, 155.0), (233.0, 155.0)) };
        let handle = |param, x, track: (f32, f32), name, h| {
            slider(param, x, track.0, track.1, &HANDLE, Some(&HANDLE_ZERO), name, h)
        };
        controls.push(handle(key("drive"), 16.0, drive, nm("Drive"), "Drive"));
        controls.push(handle(key("bias"), 416.0, bias, nm("Bias"), "Bias"));
        controls.push(handle(key("outVol"), 594.0, drive, nm("Out Vol"), "Out Vol"));
        controls.push(k(key("slewUp"), 96.0, 65.0 + dy, nm("Slew Up"), hint("Slew Up")));
        // linked, the Down knob is the Up one (`m_slewDown1Knob->setModel(&m_slewUp1Model)`)
        let link = key("slewLink");
        controls.push(k(key("slewDown"), 163.0, 65.0 + dy, nm("Slew Down"), hint("Slew Down")).when(link, false));
        let mut linked = k(key("slewUp"), 163.0, 65.0 + dy, nm("Slew Down"), hint("Slew Down")).when(link, true);
        linked.id = key("slewDownLinked");
        controls.push(linked);
        controls.push(k(key("warp"), 329.0, 26.0 + dy, nm("Warp"), hint("Warp")));
        controls.push(k(key("crush"), 371.0, 26.0 + dy, nm("Crush"), hint("Crush")));
        controls.push(k(key("attack"), 225.0, 65.0 + dy, nm("Attack"), hint("Attack")));
        controls.push(k(key("release"), 267.0, 65.0 + dy, nm("Release"), hint("Release")));
        controls.push(k(key("dynamics"), 225.0, 26.0 + dy, nm("Dynamics"), hint("Dynamics")));
        controls.push(k(key("dynamicSlew"), 267.0, 26.0 + dy, nm("Dynamic Slew"), hint("Dynamic Slew")));
        controls.push(k(key("mix"), 338.0, 78.0 + dy, nm("Mix"), hint("Mix")));
        controls.push(
            button(Bind::Switch { param: link }, link, 132.0, 70.0 + dy, &LINK_ON, &LINK_OFF, nm("Slew Link"))
                .tip(nm("Slew Link")),
        );
    }
    controls.push(
        button(Bind::Switch { param: "dcRemove" }, "dcRemove", 9.0, 248.0, &DC_ON, &DC_OFF, "DC Offset Removal")
            .tip("DC Offset Removal"),
    );
    controls.push(
        button(Bind::Switch { param: "multiband" }, "multiband", 99.0, 248.0, &MB_ON, &MB_OFF, "Multiband")
            .tip("Multiband"),
    );
    controls.push(knob("split", 190.0, 249.0, KnobStyle::Small17, 0.1, "Split", "Split:", "").log());
    let os: [(f64, &Asset, &Asset, f32, &'static str, &'static str); 6] = [
        (0.0, &OS_1_ON, &OS_1_OFF, 454.0, "oversample1x", "Disable Oversampling"),
        (1.0, &OS_2_ON, &OS_2_OFF, 479.0, "oversample2x", "2x Oversampling"),
        (2.0, &OS_4_ON, &OS_4_OFF, 504.0, "oversample4x", "4x Oversampling"),
        (3.0, &OS_8_ON, &OS_8_OFF, 529.0, "oversample8x", "8x Oversampling"),
        (4.0, &OS_16_ON, &OS_16_OFF, 554.0, "oversample16x", "16x Oversampling"),
        (5.0, &OS_32_ON, &OS_32_OFF, 579.0, "oversample32x", "32x Oversampling"),
    ];
    for (value, on, off, x, id, tip) in os {
        controls.push(button(Bind::Step { param: "oversample", value }, id, x, 248.0, on, off, "Oversample").tip(tip));
    }
    PanelSpec {
        plugin: ID,
        size: (638.0, 271.0),
        background: Some(&ARTWORK),
        view_backgrounds: &[],
        fill: 0x000000,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});
