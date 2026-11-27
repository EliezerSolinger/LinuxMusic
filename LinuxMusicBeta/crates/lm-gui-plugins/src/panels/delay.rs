//! Delay (legado/plugins/Delay/DelayControlsDialog.cpp): the 300×208 artwork, four `Bright26` knobs on the left
//! (delay time, feedback as a volume knob whose 0 dBFS is 1, LFO rate and amount), the 200×200 XY pad in the middle
//! (feedback across, delay time down: the legacy `XyPad`, a round dot of 8 pixels that follows the two models and
//! sets them while the mouse is held), and the output fader on the right (`EqFader`, 23×116, with the two peak
//! meters of the output, which the plugin's live frame carries as its first two numbers).
//!
//! The delay and LFO knobs are `TempoSyncKnob`s in the legacy; the new Delay has no tempo sync, so they are plain
//! knobs (as Kicker's).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::{Canvas, Cap, Paint, Rgba};
use crate::spec::{PanelSpec, SurfaceCx, SurfaceHandle, SurfaceSpec, fader, knob, surface};
use crate::widgets::KnobStyle;

pub const ID: &str = "delay";

static ARTWORK: Asset = asset!("delay/artwork.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK];

const PAD: f32 = 200.0;

/// The dot's place: each model's position in its range, over the pad's width and height (whole pixels, as the legacy)
fn dot(cx: &SurfaceCx<'_>) -> (f32, f32) {
    ((cx.unit("feedback") as f32 * cx.w).floor(), (cx.unit("delayTime") as f32 * cx.h).floor())
}

fn draw_pad(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let (x, y) = dot(cx);
    c.line((x, y), (x, y), 8.0, Cap::Round, &Paint::Solid(Rgba::alpha(200, 200, 200, 200)));
}

fn pad_handles(cx: &SurfaceCx<'_>) -> Vec<SurfaceHandle> {
    let (x, y) = dot(cx);
    vec![SurfaceHandle { x, y }]
}

/// `XyPad::mouseMoveEvent`: inside the pad the models take `min + pos * range / size`
fn pad_drag(cx: &SurfaceCx<'_>, _handle: usize, x: f32, y: f32) -> Vec<(&'static str, f64)> {
    if x < 0.0 || y < 0.0 || x >= cx.w || y >= cx.h {
        return Vec::new();
    }
    let (fmin, fmax) = (cx.range)("feedback");
    let (dmin, dmax) = (cx.range)("delayTime");
    vec![
        ("feedback", fmin + f64::from(x) * (fmax - fmin) / f64::from(cx.w)),
        ("delayTime", dmin + f64::from(y) * (dmax - dmin) / f64::from(cx.h)),
    ]
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let b26 = |param, x, y, step, name, hint, label| {
        knob(param, x, y, KnobStyle::Bright26, step, name, hint, " s").label(label)
    };
    PanelSpec {
        plugin: ID,
        size: (300.0, 208.0),
        background: Some(&ARTWORK),
        view_backgrounds: &[],
        fill: 0x000000,
        controls: vec![
            b26("delayTime", 10.0, 14.0, 0.0001, "Delay samples", "Delay time:", "DELAY"),
            knob("feedback", 11.0, 58.0, KnobStyle::Bright26, 0.01, "Feedback", "Feedback amount:", "")
                .volume_at(1.0)
                .label("FDBK"),
            b26("lfoTime", 11.0, 119.0, 0.0001, "LFO frequency", "LFO frequency:", "RATE"),
            b26("lfoAmount", 11.0, 159.0, 0.0001, "LFO amount", "LFO amount:", "AMNT"),
            surface(
                SurfaceSpec {
                    w: PAD,
                    h: PAD,
                    primary: "feedback",
                    reads: &["feedback", "delayTime"],
                    field_reads: &[],
                    draw: draw_pad,
                    texts: None,
                    handles: Some(pad_handles),
                    drag: Some(pad_drag),
                    grab: f32::INFINITY,
                    set_on_press: false,
                    live: false,
                    wheel: false,
                    wheel_edit: None,
                    peaks: None,

                    markers: None,
                    tag: 0,
                },
                "pad",
                50.0,
                5.0,
                "Feedback and delay time",
            ),
            fader("outGain", 263.0, 45.0, 116.0, "Output gain", "Gain:").meters(0, 1).unity_line(),
        ],
        labels: Vec::new(),
        file_drop: None,
    }
});
