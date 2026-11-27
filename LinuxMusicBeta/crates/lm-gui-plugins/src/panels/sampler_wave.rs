//! The wave view of an audio file (the AudioFileProcessor's, legado/plugins/AudioFileProcessor/
//! AudioFileProcessorWaveView.cpp): the sample's peaks, the three marker lines (start and end white, loop cyan) that are
//! dragged with the mouse, the shaded played part, the "Sample length" header, and the wheel to zoom (up widens). The
//! Sampler's panel shows it for an audio file.
//!
//! Not in the view: the cursor of the note playing (the core gives no playing position yet), dragging the wave sideways
//! or vertically, and the "reversed" mirroring of the points that the legacy view did when the reverse button changed
//! (the plugin reads the points as fractions of the sample either way).

use crate::raster::{Canvas, Rgba};
use crate::spec::{SurfText, SurfaceCx, SurfaceHandle, Text};

/// The wave view's size, and its graph (2 pixels in on each side: `s_padding`)
pub(super) const W: f32 = 245.0;
pub(super) const H: f32 = 75.0;
const PAD: f32 = 2.0;
pub(super) const GW: f32 = W - 2.0 * PAD;
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

pub(super) fn handles(cx: &SurfaceCx<'_>) -> Vec<SurfaceHandle> {
    markers(cx).into_iter().map(|x| SurfaceHandle { x, y: H / 2.0 }).collect()
}

/// A marker dragged to local `x`: the fraction there in the window, kept in order with the others (the legacy
/// `checkBound`: start before end by at least 5 ms, the loop point between them)
pub(super) fn drag(cx: &SurfaceCx<'_>, handle: usize, x: f32, _y: f32) -> Vec<(&'static str, f64)> {
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

/// The wave view with its markers, the waveform scaled by `amp`
pub(super) fn draw_wave(c: &mut Canvas, cx: &SurfaceCx<'_>, amp: f32) {
    let (from, to) = cx.window;
    // the waveform: a vertical line per column from the peaks' minimum to their maximum, times the amplification
    if let Some(b) = cx.peaks
        && b.length > 0
        && !b.pairs.is_empty()
    {
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
pub(super) fn texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
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
