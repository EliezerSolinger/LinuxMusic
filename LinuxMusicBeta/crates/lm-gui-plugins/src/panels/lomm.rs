//! LOMM (legado/plugins/LOMM/LOMMControlDialog.cpp): the 400×256 artwork, the knobs, the float LCDs for the splits,
//! thresholds, ratios and lookahead, and the checkable pixmap buttons. Every number is in docs/plugins-visual.md.
//!
//! Not ported yet: the live level meters and the draggable threshold zones painted by the legacy `paintEvent` (they
//! need the plugin's display frame in the panel), and the two buttons the legacy hides depending on other switches
//! (Feedback with Lookahead, Suppress with Mid/Side): here both are always shown.

use std::sync::LazyLock;

use lm_port::Value;

use crate::asset;
use crate::assets::Asset;
use crate::spec::{Bind, Control, PanelSpec, PressCx, Target, button, knob, lcd_float};
use crate::widgets::{KnobStyle, LcdSheet};

pub const ID: &str = "lomm";

static ARTWORK: Asset = asset!("lomm/artwork.png");
static XOVER_ON: Asset = asset!("lomm/crossover_led_green.png");
static XOVER_OFF: Asset = asset!("lomm/crossover_led_off.png");
static HIGH_ON: Asset = asset!("lomm/high_band_active.png");
static HIGH_OFF: Asset = asset!("lomm/high_band_inactive.png");
static MID_ON: Asset = asset!("lomm/mid_band_active.png");
static MID_OFF: Asset = asset!("lomm/mid_band_inactive.png");
static LOW_ON: Asset = asset!("lomm/low_band_active.png");
static LOW_OFF: Asset = asset!("lomm/low_band_inactive.png");
static DEPTH_ON: Asset = asset!("lomm/depthScaling_active.png");
static DEPTH_OFF: Asset = asset!("lomm/depthScaling_inactive.png");
static LINK_ON: Asset = asset!("lomm/stereoLink_active.png");
static LINK_OFF: Asset = asset!("lomm/stereoLink_inactive.png");
static INIT_ON: Asset = asset!("lomm/init_active.png");
static INIT_OFF: Asset = asset!("lomm/init_inactive.png");
static FEEDBACK_ON: Asset = asset!("lomm/feedback_active.png");
static FEEDBACK_OFF: Asset = asset!("lomm/feedback_inactive.png");
static MIDSIDE_ON: Asset = asset!("lomm/midside_active.png");
static MIDSIDE_OFF: Asset = asset!("lomm/midside_inactive.png");
static SUPPRESS_ON: Asset = asset!("lomm/lowSideUpwardSuppress_active.png");
static SUPPRESS_OFF: Asset = asset!("lomm/lowSideUpwardSuppress_inactive.png");
static LOOKAHEAD_ON: Asset = asset!("lomm/lookahead_active.png");
static LOOKAHEAD_OFF: Asset = asset!("lomm/lookahead_inactive.png");

pub static ASSETS: &[&Asset] = &[
    &ARTWORK,
    &XOVER_ON,
    &XOVER_OFF,
    &HIGH_ON,
    &HIGH_OFF,
    &MID_ON,
    &MID_OFF,
    &LOW_ON,
    &LOW_OFF,
    &DEPTH_ON,
    &DEPTH_OFF,
    &LINK_ON,
    &LINK_OFF,
    &INIT_ON,
    &INIT_OFF,
    &FEEDBACK_ON,
    &FEEDBACK_OFF,
    &MIDSIDE_ON,
    &MIDSIDE_OFF,
    &SUPPRESS_ON,
    &SUPPRESS_OFF,
    &LOOKAHEAD_ON,
    &LOOKAHEAD_OFF,
];

const KEYS: [&str; 50] = [
    "depth",
    "time",
    "inputVolume",
    "outputVolume",
    "upwardDepth",
    "downwardDepth",
    "highMidSplit",
    "midLowSplit",
    "highMidSplitEnabled",
    "midLowSplitEnabled",
    "highBandEnabled",
    "midBandEnabled",
    "lowBandEnabled",
    "highInputVolume",
    "midInputVolume",
    "lowInputVolume",
    "highOutputVolume",
    "midOutputVolume",
    "lowOutputVolume",
    "aboveThresholdHigh",
    "aboveThresholdMid",
    "aboveThresholdLow",
    "aboveRatioHigh",
    "aboveRatioMid",
    "aboveRatioLow",
    "belowThresholdHigh",
    "belowThresholdMid",
    "belowThresholdLow",
    "belowRatioHigh",
    "belowRatioMid",
    "belowRatioLow",
    "attackHigh",
    "attackMid",
    "attackLow",
    "releaseHigh",
    "releaseMid",
    "releaseLow",
    "rmsTime",
    "knee",
    "range",
    "balance",
    "depthScaling",
    "stereoLink",
    "autoTime",
    "mix",
    "feedback",
    "midSide",
    "lookahead",
    "lookaheadLength",
    "lowSideUpwardSuppress",
];

/// `resetAllParameters`: every parameter to its default
fn init(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    KEYS.iter().map(|key| (Target::Param(key), Value::real((cx.default)(key)).unwrap_or_default())).collect()
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let big =
        |p, x, y, step, name, hint, unit, tip| knob(p, x, y, KnobStyle::Bright26, step, name, hint, unit).tip(tip);
    let small =
        |p, x, y, step, name, hint, unit, tip| knob(p, x, y, KnobStyle::Small17, step, name, hint, unit).tip(tip);
    let sw = |p, x, y, on, off, name: &'static str, tip: &'static str| {
        button(Bind::Switch { param: p }, p, x, y, on, off, name).tip(tip)
    };
    let f = |p, x, y, whole, frac, step, name, tip: &'static str| {
        lcd_float(p, x, y, LcdSheet::Green11, whole, frac, step, name).tip(tip)
    };
    let mut c: Vec<Control> = vec![
        big("depth", 10.0, 4.0, 0.01, "Depth", "Depth:", "", "Compression amount for all bands"),
        big("time", 10.0, 41.0, 0.01, "Time", "Time:", "", "Attack/release scaling for all bands"),
        big("inputVolume", 10.0, 220.0, 0.1, "Input Volume", "Input Volume:", " dB", "Input volume"),
        big("outputVolume", 363.0, 220.0, 0.1, "Output Volume", "Output Volume:", " dB", "Output volume"),
        big(
            "upwardDepth",
            10.0,
            179.0,
            0.01,
            "Upward Depth",
            "Upward Depth:",
            "",
            "Upward compression amount for all bands",
        ),
        big(
            "downwardDepth",
            363.0,
            179.0,
            0.01,
            "Downward Depth",
            "Downward Depth:",
            "",
            "Downward compression amount for all bands",
        ),
        big("mix", 363.0, 4.0, 0.01, "Mix", "Mix:", "", "Wet/Dry of all bands"),
        f("highMidSplit", 352.0, 76.0, 5, 2, 0.01, "High/Mid Split", "High/Mid Crossover"),
        f("midLowSplit", 352.0, 156.0, 5, 2, 0.01, "Mid/Low Split", "Mid/Low Crossover"),
        sw("highMidSplitEnabled", 369.0, 104.0, &XOVER_ON, &XOVER_OFF, "Enable High/Mid Split", "High/mid band split"),
        sw("midLowSplitEnabled", 369.0, 126.0, &XOVER_ON, &XOVER_OFF, "Enable Mid/Low Split", "Mid/low band split"),
        sw("highBandEnabled", 143.0, 66.0, &HIGH_ON, &HIGH_OFF, "Enable High Band", "Enable High Band"),
        sw("midBandEnabled", 143.0, 146.0, &MID_ON, &MID_OFF, "Enable Mid Band", "Enable Mid Band"),
        sw("lowBandEnabled", 143.0, 226.0, &LOW_ON, &LOW_OFF, "Enable Low Band", "Enable Low Band"),
    ];
    // the three bands: high, mid, low, each 80 pixels below the one before
    let bands = [("High", "high", 0.0f32), ("Mid", "mid", 80.0), ("Low", "low", 160.0)];
    for (cap, low, dy) in bands {
        let k = |s: &str| -> &'static str { lm_port::intern(&format!("{s}{cap}")) };
        let kl = |s: &str| -> &'static str { lm_port::intern(&format!("{low}{s}")) };
        let txt = |s: String| -> &'static str { lm_port::intern(&s) };
        let tipv = |s: String| -> &'static str { lm_port::intern(&s) };
        c.push(big(
            kl("InputVolume"),
            53.0,
            43.0 + dy,
            0.1,
            txt(format!("{cap} Input Volume")),
            txt(format!("{cap} Input Volume:")),
            " dB",
            tipv(format!("Input volume for {low} band")),
        ));
        c.push(big(
            kl("OutputVolume"),
            320.0,
            43.0 + dy,
            0.1,
            txt(format!("{cap} Output Volume")),
            txt(format!("{cap} Output Volume:")),
            " dB",
            tipv(format!("Output volume for {low} band")),
        ));
        c.push(f(
            k("aboveThreshold"),
            300.0,
            13.0 + dy,
            3,
            3,
            0.001,
            txt(format!("Above Threshold {cap}")),
            tipv(format!("Downward compression threshold for {low} band")),
        ));
        c.push(f(
            k("aboveRatio"),
            284.0,
            44.0 + dy,
            2,
            2,
            0.01,
            txt(format!("Above Ratio {cap}")),
            tipv(format!("Downward compression ratio for {low} band")),
        ));
        c.push(f(
            k("belowThreshold"),
            59.0,
            13.0 + dy,
            3,
            3,
            0.001,
            txt(format!("Below Threshold {cap}")),
            tipv(format!("Upward compression threshold for {low} band")),
        ));
        c.push(f(
            k("belowRatio"),
            87.0,
            44.0 + dy,
            2,
            2,
            0.01,
            txt(format!("Below Ratio {cap}")),
            tipv(format!("Upward compression ratio for {low} band")),
        ));
        c.push(small(
            k("attack"),
            120.0,
            61.0 + dy,
            0.01,
            txt(format!("Attack {cap}")),
            txt(format!("Attack {cap}:")),
            " ms",
            tipv(format!("Attack time for {low} band")),
        ));
        c.push(small(
            k("release"),
            261.0,
            61.0 + dy,
            0.01,
            txt(format!("Release {cap}")),
            txt(format!("Release {cap}:")),
            " ms",
            tipv(format!("Release time for {low} band")),
        ));
    }
    c.extend([
        small(
            "rmsTime",
            380.0,
            42.0,
            0.01,
            "RMS Time",
            "RMS Time:",
            " ms",
            "RMS size for sidechain signal (set to 0 for Peak mode)",
        ),
        small("knee", 356.0, 42.0, 0.01, "Knee", "Knee:", " dB", "Knee size for all compressors"),
        small("range", 24.0, 146.0, 0.01, "Range", "Range:", " dB", "Maximum gain increase for all bands"),
        small("balance", 13.0, 114.0, 0.01, "Balance", "Balance:", " dB", "Bias input volume towards one channel"),
        small(
            "autoTime",
            24.0,
            80.0,
            0.01,
            "Auto Time",
            "Auto Time:",
            "",
            "Speed up attack and release times when transients occur",
        ),
        sw(
            "depthScaling",
            51.0,
            0.0,
            &DEPTH_ON,
            &DEPTH_OFF,
            "Scale output volume with Depth",
            "Scale output volume with Depth parameter",
        ),
        sw("stereoLink", 52.0, 237.0, &LINK_ON, &LINK_OFF, "Stereo Link", "Apply same gain change to both channels"),
        sw(
            "feedback",
            317.0,
            238.0,
            &FEEDBACK_ON,
            &FEEDBACK_OFF,
            "Feedback",
            "Use output as sidechain signal instead of input",
        ),
        sw(
            "midSide",
            285.0,
            238.0,
            &MIDSIDE_ON,
            &MIDSIDE_OFF,
            "Mid/Side",
            "Compress mid/side channels instead of left/right",
        ),
        sw(
            "lowSideUpwardSuppress",
            106.0,
            180.0,
            &SUPPRESS_ON,
            &SUPPRESS_OFF,
            "Suppress upward compression for side band",
            "Suppress upward compression for side band",
        ),
        sw(
            "lookahead",
            147.0,
            0.0,
            &LOOKAHEAD_ON,
            &LOOKAHEAD_OFF,
            "Lookahead",
            "Enable lookahead with fixed 20 ms latency",
        ),
        f("lookaheadLength", 214.0, 2.0, 2, 2, 0.01, "Lookahead Length", "Lookahead length"),
        button(Bind::Press(init), "init", 84.0, 237.0, &INIT_ON, &INIT_OFF, "Clear all parameters")
            .tip("Clear all parameters"),
    ]);
    PanelSpec {
        plugin: ID,
        size: (400.0, 256.0),
        background: Some(&ARTWORK),
        fill: 0x000000,
        controls: c,
        labels: Vec::new(),
        file_drop: None,
    }
});
