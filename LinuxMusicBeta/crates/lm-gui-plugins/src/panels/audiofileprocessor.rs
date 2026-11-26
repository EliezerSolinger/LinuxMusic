//! AudioFileProcessor (legado/plugins/AudioFileProcessor/AudioFileProcessorView.cpp, `AudioFileProcessorWaveView.cpp`):
//! the 250×250 artwork with the sample's file name (a text under the logo), the open button, the interpolation combo
//! box, the reverse, stutter, declick switches and the loop group (off, on, ping-pong), the amplify knob (a volume
//! knob in percent), the start, loop point and end knobs (the loop point only while it loops), and under them the
//! 245×75 wave view: the sample's peaks, the three marker lines (start and end white, loop cyan) that are dragged
//! with the mouse, the shaded played part, the "Sample length" header, and the wheel to zoom (up widens).
//!
//! Not in the panel: the cursor of the note playing (the core gives no playing position yet), dragging the wave
//! sideways (it slid the three points together) or vertically (zoom: the wheel does it), and the "reversed" mirroring of
//! the points that the legacy view did when the reverse button changed (the plugin reads the points as fractions of
//! the sample either way).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::{Canvas, Rgba};
use crate::spec::{
    Bind, ComboItem, PanelSpec, PeaksSpec, SurfText, SurfaceCx, SurfaceHandle, SurfaceSpec, Text, button, combo, knob,
    readout, surface,
};
use crate::widgets::KnobStyle;

pub const ID: &str = "audiofileprocessor";

static ARTWORK: Asset = asset!("audiofileprocessor/artwork.png");
static SELECT_FILE: Asset = asset!("audiofileprocessor/select_file.png");
static REVERSE_ON: Asset = asset!("audiofileprocessor/reverse_on.png");
static REVERSE_OFF: Asset = asset!("audiofileprocessor/reverse_off.png");
static LOOP_OFF_ON: Asset = asset!("audiofileprocessor/loop_off_on.png");
static LOOP_OFF_OFF: Asset = asset!("audiofileprocessor/loop_off_off.png");
static LOOP_ON_ON: Asset = asset!("audiofileprocessor/loop_on_on.png");
static LOOP_ON_OFF: Asset = asset!("audiofileprocessor/loop_on_off.png");
static LOOP_PP_ON: Asset = asset!("audiofileprocessor/loop_pingpong_on.png");
static LOOP_PP_OFF: Asset = asset!("audiofileprocessor/loop_pingpong_off.png");
static STUTTER_ON: Asset = asset!("audiofileprocessor/stutter_on.png");
static STUTTER_OFF: Asset = asset!("audiofileprocessor/stutter_off.png");
static DECLICK_ON: Asset = asset!("audiofileprocessor/declick_on.png");
static DECLICK_OFF: Asset = asset!("audiofileprocessor/declick_off.png");

pub static ASSETS: &[&Asset] = &[
    &ARTWORK,
    &SELECT_FILE,
    &REVERSE_ON,
    &REVERSE_OFF,
    &LOOP_OFF_ON,
    &LOOP_OFF_OFF,
    &LOOP_ON_ON,
    &LOOP_ON_OFF,
    &LOOP_PP_ON,
    &LOOP_PP_OFF,
    &STUTTER_ON,
    &STUTTER_OFF,
    &DECLICK_ON,
    &DECLICK_OFF,
];

/// The wave view's size, and its graph (2 pixels in on each side: `s_padding`)
const W: f32 = 245.0;
const H: f32 = 75.0;
const PAD: f32 = 2.0;
const GW: f32 = W - 2.0 * PAD;
const GH: f32 = H - 2.0 * PAD;

/// The marker x of a fraction of the sample in the window, in whole pixels (`m_startFrameX`)
fn marker_x(cx: &SurfaceCx<'_>, p: f64) -> f32 {
    let (from, to) = cx.window;
    let span = (to - from).max(1e-9);
    PAD + (((p - from) * f64::from(GW) / span) as i32) as f32
}

fn markers(cx: &SurfaceCx<'_>) -> [f32; 3] {
    [
        marker_x(cx, (cx.param)("startPoint")),
        marker_x(cx, (cx.param)("endPoint")),
        marker_x(cx, (cx.param)("loopPoint")),
    ]
}

fn handles(cx: &SurfaceCx<'_>) -> Vec<SurfaceHandle> {
    markers(cx).into_iter().map(|x| SurfaceHandle { x, y: H / 2.0 }).collect()
}

/// A marker dragged to local `x`: the fraction there in the window, kept in order with the others (the legacy
/// `checkBound`: start before end by at least 5 ms, the loop point between them)
fn drag(cx: &SurfaceCx<'_>, handle: usize, x: f32, _y: f32) -> Vec<(&'static str, f64)> {
    let (from, to) = cx.window;
    let v = (from + f64::from(x - PAD) * (to - from) / f64::from(GW)).clamp(0.0, 1.0);
    let (start, end) = ((cx.param)("startPoint"), (cx.param)("endPoint"));
    let gap = match cx.peaks {
        Some(b) if b.length > 0 => 0.005 * cx.rate / b.length as f64,
        _ => 0.0005,
    };
    match handle {
        0 => vec![("startPoint", v.min(end - gap).max(0.0))],
        1 => vec![("endPoint", v.max(start + gap).min(1.0))],
        _ => vec![("loopPoint", v.clamp(start, end.max(start)))],
    }
}

fn draw(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let (from, to) = cx.window;
    // the waveform: a vertical line per column from the peaks' minimum to their maximum, times the amplification
    if let Some(b) = cx.peaks
        && b.length > 0
        && !b.pairs.is_empty()
    {
        let amp = ((cx.param)("amplify") / 100.0) as f32;
        let len = b.length as f64;
        let (mid, half) = (PAD + GH / 2.0, GH / 2.0 - 1.0);
        for col in 0..GW as usize {
            let f0 = from + col as f64 / f64::from(GW) * (to - from);
            let f1 = from + (col + 1) as f64 / f64::from(GW) * (to - from);
            let p0 = ((f0 * len / b.samples_per_pair as f64).floor() as i64 - b.from as i64).max(0) as usize;
            let p1 = (((f1 * len / b.samples_per_pair as f64).ceil() as i64 - b.from as i64).max(p0 as i64 + 1)
                as usize)
                .min(b.pairs.len());
            if p0 >= b.pairs.len() {
                continue;
            }
            let (mut lo, mut hi) = (i16::MAX, i16::MIN);
            for &(a, z) in &b.pairs[p0..p1.max(p0 + 1).min(b.pairs.len())] {
                lo = lo.min(a);
                hi = hi.max(z);
            }
            let top = (mid - (f32::from(hi) / 32767.0 * amp).clamp(-1.0, 1.0) * half).floor();
            let bottom = (mid - (f32::from(lo) / 32767.0 * amp).clamp(-1.0, 1.0) * half).ceil();
            c.fill_rect(PAD + col as f32, top, 1.0, (bottom - top).max(1.0), Rgba::rgb(255, 255, 255));
        }
    }
    let [sx, ex, lx] = markers(cx);
    let (top, line_h) = (PAD, GH + 1.0);
    c.fill_rect(lx, top, 1.0, line_h, Rgba::rgb(0x7f, 0xff, 0xff));
    c.fill_rect(sx, top, 1.0, line_h, Rgba::rgb(255, 255, 255));
    c.fill_rect(ex, top, 1.0, line_h, Rgba::rgb(255, 255, 255));
    if ex - sx > 2.0 {
        c.fill_rect(sx + 1.0, top, ex - sx - 1.0, GH + PAD, Rgba::alpha(95, 175, 255, 50));
        if ex - lx > 2.0 {
            c.fill_rect(lx + 1.0, top, ex - lx - 1.0, GH + PAD, Rgba::alpha(95, 205, 255, 65));
        }
    }
    // the header's veil: (16, 111, 170, 180) to 40 % of 70 % of the width, then fading out
    let reach = W * 0.7;
    for col in 0..GW as usize {
        let t = (col as f32 + PAD) / reach;
        let a = if t <= 0.4 {
            180.0
        } else if t < 1.0 {
            180.0 * (1.0 - (t - 0.4) / 0.6)
        } else {
            0.0
        };
        if a > 0.0 {
            c.fill_rect(PAD + col as f32, PAD, 1.0, 14.0, Rgba::alpha(16, 111, 170, a as u8));
        }
    }
}

/// "Sample length: 3.2s": seconds above 2 s (a tenth of a second up to 20 s, whole seconds beyond), else milliseconds
fn texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
    let ms = cx.peaks.map_or(0.0, |b| b.length as f64 * 1000.0 / cx.rate.max(1.0)) as i64;
    let length = if ms > 20_000 {
        format!("{}s", ms / 1000)
    } else if ms > 2000 {
        format!("{}s", crate::widgets::qt_number(((ms / 100) as f64) / 10.0))
    } else {
        format!("{ms}ms")
    };
    vec![SurfText {
        x: PAD + 2.0,
        y: PAD + 10.0,
        px: crate::widgets::theme::SMALL_FONT_PX,
        text: format!("{} {}", Text::new("Sample length:").show(), length),
        color: 0xffffff,
        centered: false,
    }]
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let interp = |key, text| ComboItem { key, text: Text::new(text), icon: None };
    let group = |key, id, x, y, on, off, tip| {
        button(Bind::Choice { field: "loop", key }, id, x, y, on, off, "Loop mode").tip(tip)
    };
    PanelSpec {
        plugin: ID,
        size: (250.0, 250.0),
        background: Some(&ARTWORK),
        view_backgrounds: &[],
        fill: 0x000000,
        controls: vec![
            // the file's name, where the legacy paints it (8, 99), white
            readout("sample", 8.0, 90.0, 210.0, 0xffffff, "").small(),
            button(
                Bind::Open { field: "sample", action: None, extensions: &[] },
                "openSample",
                227.0,
                72.0,
                &SELECT_FILE,
                &SELECT_FILE,
                "Open sample",
            )
            .tip("Open sample"),
            button(
                Bind::Switch { param: "reverse" },
                "reverse",
                164.0,
                105.0,
                &REVERSE_ON,
                &REVERSE_OFF,
                "Reverse sample",
            )
            .tip("Reverse sample"),
            group("off", "loopOff", 190.0, 105.0, &LOOP_OFF_ON, &LOOP_OFF_OFF, "Disable loop"),
            group("on", "loopOn", 190.0, 124.0, &LOOP_ON_ON, &LOOP_ON_OFF, "Enable loop"),
            group("pingPong", "loopPingPong", 216.0, 124.0, &LOOP_PP_ON, &LOOP_PP_OFF, "Enable ping-pong loop"),
            button(Bind::Switch { param: "stutter" }, "stutter", 164.0, 124.0, &STUTTER_ON, &STUTTER_OFF, "Stutter")
                .tip("Continue sample playback across notes"),
            button(Bind::Switch { param: "declick" }, "declick", 216.0, 105.0, &DECLICK_ON, &DECLICK_OFF, "Declick")
                .tip("Declick: short fades at the start and the end, a crossfade where the loop wraps"),
            knob("amplify", 5.0, 108.0, KnobStyle::Bright26, 1.0, "Amplify", "Amplify:", "%").volume(),
            knob("startPoint", 50.0, 108.0, KnobStyle::Bright26, 0.0000001, "Start of sample", "Start point:", ""),
            knob("endPoint", 130.0, 108.0, KnobStyle::Bright26, 0.0000001, "End of sample", "End point:", ""),
            // the loop point means something only while it loops
            knob("loopPoint", 90.0, 108.0, KnobStyle::Bright26, 0.0000001, "Loopback point", "Loopback point:", "")
                .hidden_while("loop", "off"),
            combo(
                "interpolation",
                142.0,
                62.0,
                82.0,
                "Interpolation mode",
                vec![interp("none", "None"), interp("linear", "Linear"), interp("sinc", "Sinc")],
            ),
            surface(
                SurfaceSpec {
                    w: W,
                    h: H,
                    primary: "startPoint",
                    reads: &["startPoint", "endPoint", "loopPoint", "amplify", "reverse"],
                    field_reads: &["sample"],
                    draw,
                    texts: Some(texts),
                    handles: Some(handles),
                    drag: Some(drag),
                    grab: 4.0,
                    set_on_press: false,
                    live: false,
                    wheel: false,
                    peaks: Some(PeaksSpec { field: "sample", start: "startPoint", end: "endPoint", width: GW }),
                },
                "wave",
                2.0,
                172.0,
                "Wave view",
            ),
        ],
        labels: Vec::new(),
        file_drop: None,
    }
});
