//! Compressor (legado/plugins/Compressor/CompressorControlDialog.cpp): the 800×560 window (the legacy dialog is
//! resizable; this is its default size) on the dark `#070809`: the level graph with its 3 dB grid over the whole
//! window and the transfer ("knee") view in the square 560×560 at the left, and the 720×280 controls box
//! (`controlsBox.png`) at (40, 240) with the large threshold, ratio, attack and release knobs, the small ones, the
//! two faders with peak meters, and the switches and selector groups. The knobs are `Styled` on the dialog's style
//! sheet (green `#2fcc71`, 56 or 30 pixels); the "enabled" rings sit behind the knobs that only act in some modes
//! (RMS size, blend, lookahead length, ratio), and those knobs show only then.
//!
//! The transfer view draws the threshold, ratio (infinite as a limiter) and knee exactly as `redrawKnee`, with the
//! part up to the present input level (from the plugin's live frame) in full color, and the dotted threshold lines.
//! The scrolling history of the input, output and gain reduction is drawn from the panel's frames of the last seconds,
//! and the wheel over the graph changes the dB range (3 to 96, in steps of 3). Not in the panel: the double click that
//! hides the controls.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::{Canvas, Cap, Paint, Rgba};
use crate::spec::{Bind, Control, PanelSpec, SurfText, SurfaceCx, SurfaceSpec, button, fader, knob, surface};
use crate::widgets::{KnobStyle, Styled};

pub const ID: &str = "compressor";

static BOX: Asset = asset!("compressor/controlsBox.png");
static RING_SMALL: Asset = asset!("compressor/knob_enabled.png");
static RING_LARGE: Asset = asset!("compressor/knob_enabled_large.png");
static RMS_ON: Asset = asset!("compressor/rms_sel.png");
static RMS_OFF: Asset = asset!("compressor/rms_unsel.png");
static PEAK_ON: Asset = asset!("compressor/peak_sel.png");
static PEAK_OFF: Asset = asset!("compressor/peak_unsel.png");
static LEFTRIGHT_ON: Asset = asset!("compressor/leftright_sel.png");
static LEFTRIGHT_OFF: Asset = asset!("compressor/leftright_unsel.png");
static MIDSIDE_ON: Asset = asset!("compressor/midside_sel.png");
static MIDSIDE_OFF: Asset = asset!("compressor/midside_unsel.png");
static COMPRESSOR_ON: Asset = asset!("compressor/compressor_sel.png");
static COMPRESSOR_OFF: Asset = asset!("compressor/compressor_unsel.png");
static LIMITER_ON: Asset = asset!("compressor/limiter_sel.png");
static LIMITER_OFF: Asset = asset!("compressor/limiter_unsel.png");
static UNLINKED_ON: Asset = asset!("compressor/unlinked_sel.png");
static UNLINKED_OFF: Asset = asset!("compressor/unlinked_unsel.png");
static MAXIMUM_ON: Asset = asset!("compressor/maximum_sel.png");
static MAXIMUM_OFF: Asset = asset!("compressor/maximum_unsel.png");
static AVERAGE_ON: Asset = asset!("compressor/average_sel.png");
static AVERAGE_OFF: Asset = asset!("compressor/average_unsel.png");
static MINIMUM_ON: Asset = asset!("compressor/minimum_sel.png");
static MINIMUM_OFF: Asset = asset!("compressor/minimum_unsel.png");
static BLEND_ON: Asset = asset!("compressor/blend_sel.png");
static BLEND_OFF: Asset = asset!("compressor/blend_unsel.png");
static AUTOGAIN_ON: Asset = asset!("compressor/autogain_sel.png");
static AUTOGAIN_OFF: Asset = asset!("compressor/autogain_unsel.png");
static AUDITION_ON: Asset = asset!("compressor/audition_sel.png");
static AUDITION_OFF: Asset = asset!("compressor/audition_unsel.png");
static FEEDBACK_ON: Asset = asset!("compressor/feedback_sel.png");
static FEEDBACK_OFF: Asset = asset!("compressor/feedback_unsel.png");
static LOOKAHEAD_ON: Asset = asset!("compressor/lookahead_sel.png");
static LOOKAHEAD_OFF: Asset = asset!("compressor/lookahead_unsel.png");
pub static ASSETS: &[&Asset] = &[
    &BOX,
    &RING_SMALL,
    &RING_LARGE,
    &RMS_ON,
    &RMS_OFF,
    &PEAK_ON,
    &PEAK_OFF,
    &LEFTRIGHT_ON,
    &LEFTRIGHT_OFF,
    &MIDSIDE_ON,
    &MIDSIDE_OFF,
    &COMPRESSOR_ON,
    &COMPRESSOR_OFF,
    &LIMITER_ON,
    &LIMITER_OFF,
    &UNLINKED_ON,
    &UNLINKED_OFF,
    &MAXIMUM_ON,
    &MAXIMUM_OFF,
    &AVERAGE_ON,
    &AVERAGE_OFF,
    &MINIMUM_ON,
    &MINIMUM_OFF,
    &BLEND_ON,
    &BLEND_OFF,
    &AUTOGAIN_ON,
    &AUTOGAIN_OFF,
    &AUDITION_ON,
    &AUDITION_OFF,
    &FEEDBACK_ON,
    &FEEDBACK_OFF,
    &LOOKAHEAD_ON,
    &LOOKAHEAD_OFF,
];

const W: f32 = 800.0;
const H: f32 = 560.0;
/// The controls box's corner
const BX: f32 = 40.0;
const BY: f32 = 240.0;
const GRID: f32 = 3.0;

/// The level's row for a dB range (36 at the start; the wheel changes it by 3, from 3 to 96)
fn y_of(db: f32, range: f32) -> f32 {
    -db / range * H
}

fn x_of(db: f32, range: f32) -> f32 {
    H - y_of(db, range)
}

fn dotted(c: &mut Canvas, a: (f32, f32), b: (f32, f32), paint: &Paint) {
    let horizontal = a.1 == b.1;
    let len = if horizontal { b.0 - a.0 } else { b.1 - a.1 };
    let mut k = 0.0;
    while k < len {
        let (p, q) = if horizontal {
            ((a.0 + k, a.1), (a.0 + (k + 2.0).min(len), a.1))
        } else {
            ((a.0, a.1 + k), (a.0, a.1 + (k + 2.0).min(len)))
        };
        c.line(p, q, 2.0, Cap::Flat, paint);
        k += 6.0;
    }
}

/// A segment of the knee's polyline, drawn in `paint`, cut at x = `until` when given
fn seg(c: &mut Canvas, a: (f32, f32), b: (f32, f32), color: Rgba, until: Option<f32>) {
    let (mut a, mut b) = (a, b);
    if let Some(u) = until {
        if a.0 > u && b.0 > u {
            return;
        }
        if b.0 > u && (b.0 - a.0).abs() > f32::EPSILON {
            let t = (u - a.0) / (b.0 - a.0);
            b = (u, a.1 + (b.1 - a.1) * t);
        } else if a.0 > u {
            a = b;
        }
    }
    c.line(a, b, 3.0, Cap::Flat, &Paint::Solid(color));
}

/// The scrolling history (`drawVisPixmap`): the input level as a line over a faint area, the output (the input plus the
/// gain reduction, in pixels) the same, and the gain reduction in 2 pixels of red from the top; a pixel is 6 ms
/// (`COMP_MILLI_PER_PIXEL`), the newest at the right edge. The frame is the input peaks, the output peaks and the gains
fn history(c: &mut Canvas, cx: &SurfaceCx<'_>, range: f32) {
    let ys = |f: &[f32]| -> (f32, f32) {
        let avg = |a: f32, b: f32| (a + b) / 2.0;
        let db = |v: f32| 20.0 * v.max(1.0e-6).log10();
        let input = y_of(db(avg(f.first().copied().unwrap_or(0.0), f.get(1).copied().unwrap_or(0.0))), range);
        let gain = y_of(db(avg(f.get(4).copied().unwrap_or(1.0), f.get(5).copied().unwrap_or(1.0))), range);
        (input.clamp(0.0, H), gain)
    };
    let mut last: Option<(f32, (f32, f32))> = None;
    for (age, frame) in cx.history {
        let x = W - age / 6.0;
        let (y, g) = ys(frame);
        if let Some((lx, (ly, lg))) = last {
            let (x0, x1) = (lx.min(x), lx.max(x));
            let step = |col: f32, a: f32, b: f32| a + (b - a) * ((col - lx) / (x - lx).max(0.001)).clamp(0.0, 1.0);
            let mut col = x0.floor();
            while col < x1 {
                let (iy, og) = (step(col, ly, y), step(col, lg, g));
                c.fill_rect(col, iy, 1.0, H - iy, Rgba::alpha(209, 216, 228, 17));
                c.fill_rect(
                    col,
                    (iy + og).clamp(0.0, H),
                    1.0,
                    H - (iy + og).clamp(0.0, H),
                    Rgba::alpha(209, 216, 228, 30),
                );
                col += 1.0;
            }
            let pen = |a: u8| Paint::Solid(Rgba::alpha(209, 216, 228, a));
            c.line((lx, ly), (x, y), 1.0, Cap::Flat, &pen(100));
            c.line((lx, (ly + lg).clamp(0.0, H)), (x, (y + g).clamp(0.0, H)), 1.0, Cap::Flat, &pen(240));
            c.line((lx, lg), (x, g), 2.0, Cap::Flat, &Paint::Solid(Rgba::alpha(180, 100, 100, 210)));
        }
        last = Some((x, (y, g)));
    }
}

fn draw(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let range = cx.zoom;
    history(c, cx, range);
    let grid = Paint::Solid(Rgba::alpha(209, 216, 228, 50));
    let mut i = 0.0;
    while i < range / GRID + 1.0 {
        let db = -GRID * i;
        let (y, x) = (y_of(db, range).round(), x_of(db, range).round());
        c.line((0.0, y + 0.5), (W, y + 0.5), 1.0, Cap::Flat, &grid);
        c.line((x + 0.5, 0.0), (x + 0.5, H), 1.0, Cap::Flat, &grid);
        i += 1.0;
    }
    let thr = (cx.param)("threshold") as f32;
    let knee = (cx.param)("knee") as f32;
    let limiter = (cx.param)("limiter") >= 0.5;
    let ratio = if limiter { 0.0 } else { 1.0 / ((cx.param)("ratio") as f32).max(1.0) };
    let k1 = thr - knee;
    let k2x = thr + knee;
    let k2y = thr + knee * ratio;
    let rp = thr + (-thr * ratio);
    // the input level now, the louder side, for the part of the curve that is lit
    let level = cx.live(0, 0.0).max(cx.live(1, 0.0));
    let lit = (level > 0.0).then(|| x_of(20.0 * level.log10(), range));
    let green = Rgba::rgb(39, 171, 95);
    let teal = Rgba::rgb(9, 171, 160);
    let dim = |col: Rgba| Rgba { a: 0.25, ..col };
    let mut lines: Vec<((f32, f32), (f32, f32), Rgba)> = vec![((0.0, H), (x_of(k1, range), y_of(k1, range)), green)];
    if x_of(k2x, range) < H {
        lines.push(((x_of(k2x, range), y_of(k2y, range)), (H, y_of(rp, range)), green));
    }
    if knee > 0.0 {
        let mut prev = (k1, k1);
        for n in 0..20 {
            let nx = k1 + (k2x - k1) * (n + 1) as f32 / 20.0;
            let t = nx - thr + knee;
            let ny = nx + (ratio - 1.0) * t * t / (4.0 * knee);
            lines.push(((x_of(prev.0, range), y_of(prev.1, range)), (x_of(nx, range), y_of(ny, range)), teal));
            prev = (nx, ny);
        }
    }
    for (a, b, col) in &lines {
        seg(c, *a, *b, dim(*col), None);
        if let Some(u) = lit {
            seg(c, *a, *b, *col, Some(u));
        }
    }
    let thresh = Paint::Solid(Rgba::alpha(39, 171, 95, 100));
    dotted(c, (0.0, y_of(thr, range)), (W, y_of(thr, range)), &thresh);
    dotted(c, (H - y_of(thr, range), 0.0), (H - y_of(thr, range), H), &thresh);
}

fn texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
    let range = cx.zoom;
    let mut out = Vec::new();
    let mut i = 0.0;
    while i < range / GRID + 1.0 {
        let t = crate::widgets::qt_number(f64::from(-GRID * i));
        out.push(SurfText {
            x: W - 4.0 - 7.0 * t.len() as f32,
            y: y_of(-GRID * i, range) + 24.0,
            px: 12.0,
            text: t,
            color: 0x2f3134,
            centered: false,
        });
        i += 1.0;
    }
    out
}

/// `makeLargeKnob`: 56×56, radii 15 and 23, center (28, 28); `makeSmallKnob`: 30×30, radii 4 and 10, center (15, 15)
fn style(large: bool) -> KnobStyle {
    let (s, inner, outer, c) = if large { (56.0, 15.0, 23.0, 28.0) } else { (30.0, 4.0, 10.0, 15.0) };
    KnobStyle::Styled(Styled {
        w: s,
        h: s,
        color: Rgba::hex(0x2fcc71),
        outer: Some(Rgba::hex(0x2fcc71)),
        inner_radius: inner,
        outer_radius: outer,
        center: (c, c),
        line_width: 2.0,
    })
}

/// A decoration (the ring behind a knob): a surface with no handles, drawing the image
fn ring(id: &'static str, x: f32, y: f32, large: bool) -> Control {
    fn small(c: &mut Canvas, _: &SurfaceCx<'_>) {
        c.pixmap(crate::assets::image(&RING_SMALL), 0.0, 0.0, false);
    }
    fn big(c: &mut Canvas, _: &SurfaceCx<'_>) {
        c.pixmap(crate::assets::image(&RING_LARGE), 0.0, 0.0, false);
    }
    let s = if large { 56.0 } else { 30.0 };
    surface(
        SurfaceSpec {
            w: s,
            h: s,
            primary: "ratio",
            reads: &[],
            field_reads: &[],
            draw: if large { big } else { small },
            texts: None,
            handles: None,
            drag: None,
            grab: 0.0,
            set_on_press: false,
            live: false,
            wheel: false,
            wheel_edit: None,
            peaks: None,

            markers: None,
            tag: 0,
        },
        id,
        x,
        y,
        "Knob ring",
    )
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut c: Vec<Control> = Vec::new();
    c.push(surface(
        SurfaceSpec {
            w: W,
            h: H,
            primary: "threshold",
            reads: &["threshold", "knee", "ratio", "limiter"],
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
            // the dB range: 36 at first, 3 dB a notch, from 3 to 96
            markers: None,
            tag: crate::spec::SURFACE_ZOOM | 36 | (3 << 8) | (96 << 16),
        },
        "graph",
        0.0,
        0.0,
        "Compressor graph",
    ));
    c.push(surface(
        SurfaceSpec {
            w: 720.0,
            h: 280.0,
            primary: "threshold",
            reads: &[],
            field_reads: &[],
            draw: |cv, _| cv.pixmap(crate::assets::image(&BOX), 0.0, 0.0, false),
            texts: None,
            handles: None,
            drag: None,
            grab: 0.0,
            set_on_press: false,
            live: false,
            wheel: false,
            wheel_edit: None,
            peaks: None,

            markers: None,
            tag: 0,
        },
        "box",
        BX,
        BY,
        "Controls box",
    ));
    let k = |p: &'static str,
             x: f32,
             y: f32,
             large: bool,
             step: f64,
             name: &'static str,
             hint: &'static str,
             unit: &'static str,
             tip: &'static str| { knob(p, BX + x, BY + y, style(large), step, name, hint, unit).tip(tip) };
    c.push(ring("ringRms", BX + 429.0, BY + 209.0, false).when("peakMode", false));
    c.push(ring("ringBlend", BX + 587.0, BY + 197.0, false).only_while("stereoLink", "blend"));
    c.push(ring("ringLookahead", BX + 221.0, BY + 135.0, false).when("lookahead", true));
    c.push(ring("ringRatio", BX + 267.0, BY + 21.0, true).when("limiter", false));
    c.push(k(
        "threshold",
        137.0,
        21.0,
        true,
        0.001,
        "Threshold",
        "Threshold:",
        " dBFS",
        "Volume at which the compression begins to take place",
    ));
    c.push(
        k(
            "ratio",
            267.0,
            21.0,
            true,
            0.001,
            "Ratio",
            "Ratio:",
            ":1",
            "How far the compressor must turn the volume down after crossing the threshold",
        )
        .when("limiter", false),
    );
    c.push(k(
        "attack",
        397.0,
        21.0,
        true,
        0.001,
        "Attack",
        "Attack:",
        " ms",
        "Speed at which the compressor starts to compress the audio",
    ));
    c.push(k(
        "release",
        527.0,
        21.0,
        true,
        0.001,
        "Release",
        "Release:",
        " ms",
        "Speed at which the compressor ceases to compress the audio",
    ));
    c.push(k(
        "knee",
        97.0,
        135.0,
        false,
        0.0001,
        "Knee",
        "Knee:",
        " dB",
        "Smooth out the gain reduction curve around the threshold",
    ));
    c.push(k("range", 159.0, 135.0, false, 0.0001, "Range", "Range:", " dBFS", "Maximum gain reduction"));
    c.push(
        k(
            "lookaheadLength",
            221.0,
            135.0,
            false,
            0.0001,
            "Lookahead Length",
            "Lookahead Length:",
            " ms",
            "How long the compressor has to react to the sidechain signal ahead of time",
        )
        .when("lookahead", true),
    );
    c.push(k("hold", 283.0, 135.0, false, 0.0001, "Hold", "Hold:", " ms", "Delay between attack and release stages"));
    c.push(
        k("rmsSize", 429.0, 209.0, false, 0.0001, "RMS Size", "RMS Size:", "", "Size of the RMS buffer")
            .when("peakMode", false),
    );
    c.push(k(
        "inputBalance",
        27.0,
        219.0,
        false,
        0.0001,
        "Input Balance",
        "Input Balance:",
        "",
        "Bias the input audio to the left/right or mid/side",
    ));
    c.push(k(
        "outputBalance",
        662.0,
        219.0,
        false,
        0.0001,
        "Output Balance",
        "Output Balance:",
        "",
        "Bias the output audio to the left/right or mid/side",
    ));
    c.push(k(
        "stereoBalance",
        522.0,
        137.0,
        false,
        0.0001,
        "Stereo Balance",
        "Stereo Balance:",
        "",
        "Bias the sidechain signal to the left/right or mid/side",
    ));
    c.push(
        k(
            "blend",
            587.0,
            197.0,
            false,
            0.0001,
            "Stereo Link Blend",
            "Stereo Link Blend:",
            "",
            "Blend between unlinked/maximum/average/minimum stereo linking modes",
        )
        .only_while("stereoLink", "blend"),
    );
    c.push(k(
        "tilt",
        364.0,
        138.0,
        false,
        0.0001,
        "Tilt Gain",
        "Tilt Gain:",
        " dB",
        "Bias the sidechain signal to the low or high frequencies.  -6 db is lowpass, 6 db is highpass.",
    ));
    c.push(k(
        "tiltFrequency",
        415.0,
        138.0,
        false,
        0.0001,
        "Tilt Frequency",
        "Tilt Frequency:",
        " Hz",
        "Center frequency of sidechain tilt filter",
    ));
    c.push(k("mix", 27.0, 13.0, false, 0.0001, "Mix", "Mix:", "%", "Balance between wet and dry signals"));
    c.push(k(
        "autoAttack",
        460.0,
        38.0,
        false,
        0.0001,
        "Auto Attack",
        "Auto Attack:",
        "%",
        "Automatically control attack value depending on crest factor",
    ));
    c.push(k(
        "autoRelease",
        590.0,
        38.0,
        false,
        0.0001,
        "Auto Release",
        "Auto Release:",
        "%",
        "Automatically control release value depending on crest factor",
    ));
    c.push(
        fader("outputGain", BX + 666.0, BY + 91.0, 116.0, "Output gain", "Gain")
            .fader_unit("dBFS")
            .meters(2, 3)
            .unity_line()
            .tip("Output volume"),
    );
    c.push(
        fader("inputGain", BX + 31.0, BY + 91.0, 116.0, "Input gain", "Gain")
            .fader_unit("dBFS")
            .meters(0, 1)
            .unity_line()
            .tip("Input volume"),
    );
    let step =
        |param: &'static str,
         value: f64,
         id: &'static str,
         x: f32,
         y: f32,
         on: &'static Asset,
         off: &'static Asset,
         name: &'static str,
         tip: &'static str| { button(Bind::Step { param, value }, id, BX + x, BY + y, on, off, name).tip(tip) };
    c.push(step("peakMode", 0.0, "rms", 337.0, 231.0, &RMS_ON, &RMS_OFF, "Root Mean Square", "Use RMS of the input"));
    c.push(step("peakMode", 1.0, "peak", 337.0, 248.0, &PEAK_ON, &PEAK_OFF, "Peak", "Use absolute value of the input"));
    c.push(step(
        "midSide",
        0.0,
        "leftRight",
        220.0,
        231.0,
        &LEFTRIGHT_ON,
        &LEFTRIGHT_OFF,
        "Left/Right",
        "Compress left and right audio",
    ));
    c.push(step(
        "midSide",
        1.0,
        "midSide",
        220.0,
        248.0,
        &MIDSIDE_ON,
        &MIDSIDE_OFF,
        "Mid/Side",
        "Compress mid and side audio",
    ));
    c.push(step(
        "limiter",
        0.0,
        "compress",
        98.0,
        231.0,
        &COMPRESSOR_ON,
        &COMPRESSOR_OFF,
        "Compressor",
        "Compress the audio",
    ));
    c.push(step(
        "limiter",
        1.0,
        "limit",
        98.0,
        248.0,
        &LIMITER_ON,
        &LIMITER_OFF,
        "Limiter",
        "Set Ratio to infinity (is not guaranteed to limit audio volume)",
    ));
    for (i, (key, on, off, name, tip)) in [
        ("unlinked", &UNLINKED_ON, &UNLINKED_OFF, "Unlinked", "Compress each channel separately"),
        ("maximum", &MAXIMUM_ON, &MAXIMUM_OFF, "Maximum", "Compress based on the loudest channel"),
        ("average", &AVERAGE_ON, &AVERAGE_OFF, "Average", "Compress based on the averaged channel volume"),
        ("minimum", &MINIMUM_ON, &MINIMUM_OFF, "Minimum", "Compress based on the quietest channel"),
        ("blend", &BLEND_ON, &BLEND_OFF, "Blend", "Blend between stereo linking modes"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = lm_port::intern(&format!("link{key}"));
        c.push(
            button(
                Bind::Choice { field: "stereoLink", key },
                id,
                BX + 495.0,
                BY + 180.0 + 17.0 * i as f32,
                on,
                off,
                name,
            )
            .tip(tip),
        );
    }
    let sw =
        |param: &'static str,
         x: f32,
         y: f32,
         on: &'static Asset,
         off: &'static Asset,
         name: &'static str,
         tip: &'static str| { button(Bind::Switch { param }, param, BX + x, BY + y, on, off, name).tip(tip) };
    c.push(sw(
        "autoMakeupGain",
        220.0,
        206.0,
        &AUTOGAIN_ON,
        &AUTOGAIN_OFF,
        "Auto Makeup Gain",
        "Automatically change makeup gain depending on threshold, knee, and ratio settings",
    ));
    c.push(sw("audition", 658.0, 14.0, &AUDITION_ON, &AUDITION_OFF, "Audition", "Play the delta signal"));
    c.push(
        sw(
            "feedback",
            98.0,
            206.0,
            &FEEDBACK_ON,
            &FEEDBACK_OFF,
            "Feedback",
            "Use the compressor's output as the sidechain input",
        )
        .when("lookahead", false),
    );
    c.push(sw(
        "lookahead",
        202.0,
        171.0,
        &LOOKAHEAD_ON,
        &LOOKAHEAD_OFF,
        "Lookahead Enabled",
        "Enable Lookahead, which introduces 20 milliseconds of latency",
    ));
    PanelSpec {
        plugin: ID,
        size: (W, H),
        background: None,
        view_backgrounds: &[],
        fill: 0x070809,
        controls: c,
        labels: Vec::new(),
        file_drop: None,
    }
});
