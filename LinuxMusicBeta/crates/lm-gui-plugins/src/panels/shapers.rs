//! DynamicsProcessor and WaveShaper (legado/plugins/DynamicsProcessor/DynamicsProcessorControlDialog.cpp,
//! legado/plugins/WaveShaper/WaveShaperControlDialog.cpp): the artwork, a 204×205 drawable graph over its own
//! `wavegraph.png`, two volume knobs and the push buttons that reset, smooth and scale the curve by ±1 dB (the
//! `*Controls::resetClicked/smoothClicked/addOneClicked/subOneClicked` slots).

use std::sync::LazyLock;

use lm_port::Value;

use crate::asset;
use crate::assets::Asset;
use crate::graphs;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, GraphSpec, GraphStyle, PanelSpec, PressCx, Target, button, graph, knob, led};
use crate::widgets::{KnobStyle, LedColor};

pub const DYNAMICS_ID: &str = "dynamicsprocessor";
pub const WAVESHAPER_ID: &str = "waveshaper";

static DP_ARTWORK: Asset = asset!("dynamicsprocessor/artwork.png");
static DP_GRAPH: Asset = asset!("dynamicsprocessor/wavegraph.png");
static DP_RESET_ON: Asset = asset!("dynamicsprocessor/reset_active.png");
static DP_RESET_OFF: Asset = asset!("dynamicsprocessor/reset_inactive.png");
static DP_SMOOTH_ON: Asset = asset!("dynamicsprocessor/smooth_active.png");
static DP_SMOOTH_OFF: Asset = asset!("dynamicsprocessor/smooth_inactive.png");
static DP_ADD_ON: Asset = asset!("dynamicsprocessor/add1_active.png");
static DP_ADD_OFF: Asset = asset!("dynamicsprocessor/add1_inactive.png");
static DP_SUB_ON: Asset = asset!("dynamicsprocessor/sub1_active.png");
static DP_SUB_OFF: Asset = asset!("dynamicsprocessor/sub1_inactive.png");
static DP_MAX_ON: Asset = asset!("dynamicsprocessor/max_active.png");
static DP_MAX_OFF: Asset = asset!("dynamicsprocessor/max_inactive.png");
static DP_AVG_ON: Asset = asset!("dynamicsprocessor/avg_active.png");
static DP_AVG_OFF: Asset = asset!("dynamicsprocessor/avg_inactive.png");
static DP_UNL_ON: Asset = asset!("dynamicsprocessor/unl_active.png");
static DP_UNL_OFF: Asset = asset!("dynamicsprocessor/unl_inactive.png");

static WS_ARTWORK: Asset = asset!("waveshaper/artwork.png");
static WS_GRAPH: Asset = asset!("waveshaper/wavegraph.png");
static WS_RESET_ON: Asset = asset!("waveshaper/reset_active.png");
static WS_RESET_OFF: Asset = asset!("waveshaper/reset_inactive.png");
static WS_SMOOTH_ON: Asset = asset!("waveshaper/smooth_active.png");
static WS_SMOOTH_OFF: Asset = asset!("waveshaper/smooth_inactive.png");
static WS_ADD_ON: Asset = asset!("waveshaper/add1_active.png");
static WS_ADD_OFF: Asset = asset!("waveshaper/add1_inactive.png");
static WS_SUB_ON: Asset = asset!("waveshaper/sub1_active.png");
static WS_SUB_OFF: Asset = asset!("waveshaper/sub1_inactive.png");

pub static DYNAMICS_ASSETS: &[&Asset] = &[
    &DP_ARTWORK,
    &DP_GRAPH,
    &DP_RESET_ON,
    &DP_RESET_OFF,
    &DP_SMOOTH_ON,
    &DP_SMOOTH_OFF,
    &DP_ADD_ON,
    &DP_ADD_OFF,
    &DP_SUB_ON,
    &DP_SUB_OFF,
    &DP_MAX_ON,
    &DP_MAX_OFF,
    &DP_AVG_ON,
    &DP_AVG_OFF,
    &DP_UNL_ON,
    &DP_UNL_OFF,
];

pub static WAVESHAPER_ASSETS: &[&Asset] = &[
    &WS_ARTWORK,
    &WS_GRAPH,
    &WS_RESET_ON,
    &WS_RESET_OFF,
    &WS_SMOOTH_ON,
    &WS_SMOOTH_OFF,
    &WS_ADD_ON,
    &WS_ADD_OFF,
    &WS_SUB_ON,
    &WS_SUB_OFF,
];

/// `onedB`
const ONE_DB: f64 = 1.122_018_454_301_963_3;

/// The curve of a field as samples
fn samples(cx: &PressCx<'_>, field: &str) -> Vec<f64> {
    let mut v: Vec<f64> = (cx.field)(field).items().map(|x| x.as_real().unwrap_or(0.0)).collect();
    v.resize(200, 0.0);
    v
}

fn list(samples: &[f64]) -> Value {
    Value::list(samples.iter().map(|v| Value::real(*v).unwrap_or_default()))
}

/// `setDefaultShape` / `resetClicked`: `(i + 1) / 200`
fn reset_to(field: &'static str) -> Vec<(Target, Value)> {
    let s: Vec<f64> = (0..200).map(|i| (i as f64 + 1.0) / 200.0).collect();
    vec![(Target::Field(field), list(&s))]
}

fn scaled(cx: &PressCx<'_>, field: &'static str, f: f64) -> Vec<(Target, Value)> {
    let s: Vec<f64> = samples(cx, field).iter().map(|v| (v * f).clamp(0.0, 1.0)).collect();
    vec![(Target::Field(field), list(&s))]
}

fn smoothed(cx: &PressCx<'_>, field: &'static str) -> Vec<(Target, Value)> {
    let mut s = samples(cx, field);
    graphs::smooth_non_cyclic(&mut s, 200);
    vec![(Target::Field(field), list(&s))]
}

fn dp_reset(_: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    reset_to("curve")
}
fn dp_smooth(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    smoothed(cx, "curve")
}
fn dp_add(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    scaled(cx, "curve", ONE_DB)
}
fn dp_sub(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    scaled(cx, "curve", 1.0 / ONE_DB)
}
fn ws_reset(_: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    reset_to("shape")
}
fn ws_smooth(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    smoothed(cx, "shape")
}
fn ws_add(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    scaled(cx, "shape", ONE_DB)
}
fn ws_sub(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    scaled(cx, "shape", 1.0 / ONE_DB)
}

fn wave_graph(field: &'static str, bg: &'static Asset) -> Control {
    graph(
        GraphSpec {
            field,
            w: 204.0,
            h: 205.0,
            min: 0.0,
            max: 1.0,
            length: 200,
            length_param: None,
            style: GraphStyle::LinearNonCyclic,
            color: Rgba::rgb(85, 204, 145),
            background: Some(bg),
            foreground: None,
            step: 0.0,
        },
        10.0,
        6.0,
        "Wavegraph",
    )
}

pub static DYNAMICS_PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let b = |f, id, x, y, on, off, tip: &'static str| button(Bind::Press(f), id, x, y, on, off, tip).tip(tip);
    let mode = |key, id, y, on, off, tip: &'static str| {
        button(Bind::Choice { field: "stereoMode", key }, id, 131.0, y, on, off, "Stereo mode").tip(tip)
    };
    PanelSpec {
        plugin: DYNAMICS_ID,
        size: (224.0, 319.0),
        background: Some(&DP_ARTWORK),
        fill: 0x000000,
        controls: vec![
            wave_graph("curve", &DP_GRAPH),
            knob("inputGain", 26.0, 223.0, KnobStyle::Bright26, 0.01, "Input gain", "Input gain:", "")
                .label("INPUT")
                .volume_at(1.0),
            knob("outputGain", 76.0, 223.0, KnobStyle::Bright26, 0.01, "Output gain", "Output gain:", "")
                .label("OUTPUT")
                .volume_at(1.0),
            knob("attackTime", 24.0, 268.0, KnobStyle::Bright26, 1.0, "Attack time", "Peak attack time:", "ms")
                .label("ATTACK"),
            knob("releaseTime", 74.0, 268.0, KnobStyle::Bright26, 1.0, "Release time", "Peak release time:", "ms")
                .label("RELEASE"),
            b(dp_reset, "reset", 162.0, 223.0, &DP_RESET_ON, &DP_RESET_OFF, "Reset wavegraph"),
            b(dp_smooth, "smooth", 162.0, 239.0, &DP_SMOOTH_ON, &DP_SMOOTH_OFF, "Smooth wavegraph"),
            b(dp_add, "add1", 131.0, 223.0, &DP_ADD_ON, &DP_ADD_OFF, "Increase wavegraph amplitude by 1 dB"),
            b(dp_sub, "sub1", 131.0, 239.0, &DP_SUB_ON, &DP_SUB_OFF, "Decrease wavegraph amplitude by 1 dB"),
            mode(
                "maximum",
                "stereoMode.maximum",
                257.0,
                &DP_MAX_ON,
                &DP_MAX_OFF,
                "Process based on the maximum of both stereo channels",
            ),
            mode(
                "average",
                "stereoMode.average",
                274.0,
                &DP_AVG_ON,
                &DP_AVG_OFF,
                "Process based on the average of both stereo channels",
            ),
            mode(
                "unlinked",
                "stereoMode.unlinked",
                290.0,
                &DP_UNL_ON,
                &DP_UNL_OFF,
                "Process each stereo channel independently",
            ),
        ],
        view_backgrounds: &[],
        labels: Vec::new(),
        file_drop: None,
    }
});

pub static WAVESHAPER_PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let b = |f, id, x, y, on, off, tip: &'static str| button(Bind::Press(f), id, x, y, on, off, tip).tip(tip);
    PanelSpec {
        plugin: WAVESHAPER_ID,
        size: (224.0, 274.0),
        background: Some(&WS_ARTWORK),
        fill: 0x000000,
        controls: vec![
            wave_graph("shape", &WS_GRAPH),
            knob("inputGain", 26.0, 225.0, KnobStyle::Bright26, 0.01, "Input gain", "Input gain:", "")
                .label("INPUT")
                .volume_at(1.0),
            knob("outputGain", 76.0, 225.0, KnobStyle::Bright26, 0.01, "Output gain", "Output gain:", "")
                .label("OUTPUT")
                .volume_at(1.0),
            b(ws_reset, "reset", 162.0, 221.0, &WS_RESET_ON, &WS_RESET_OFF, "Reset wavegraph"),
            b(ws_smooth, "smooth", 162.0, 237.0, &WS_SMOOTH_ON, &WS_SMOOTH_OFF, "Smooth wavegraph"),
            b(ws_add, "add1", 131.0, 221.0, &WS_ADD_ON, &WS_ADD_OFF, "Increase wavegraph amplitude by 1 dB"),
            b(ws_sub, "sub1", 131.0, 237.0, &WS_SUB_ON, &WS_SUB_OFF, "Decrease wavegraph amplitude by 1 dB"),
            led(
                Bind::Switch { param: "clipInput" },
                "clipInput",
                131.0,
                252.0,
                LedColor::Green,
                "Clip input",
                "Clip input",
            )
            .tip("Clip input signal to 0 dB"),
        ],
        view_backgrounds: &[],
        labels: Vec::new(),
        file_drop: None,
    }
});
