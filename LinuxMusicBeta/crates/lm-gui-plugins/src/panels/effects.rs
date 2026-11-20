//! The legacy effects whose dialogs were knobs on their own artwork (`artwork.svg`): Bitcrush, BassBooster,
//! ReverbSC, Flanger, Amplifier, StereoMatrix and DualFilter (legado/plugins/<Effect>/<Effect>ControlDialog.cpp;
//! docs/plugins-visual.md has every number).
//!
//! Dialogs placed by a Qt layout (BassBooster, ReverbSC, Flanger, Amplifier, StereoMatrix): the legacy positions
//! came from the layout with the system's font, so they were never fixed numbers. Here the panel has the artwork's
//! size (the size the artwork was drawn for) and each knob is centered in its equal cell inside the layout's 9-pixel
//! margins, which is the layout's result when the knobs are as wide as their pixmaps; StereoMatrix puts its knobs on
//! the circles its artwork draws for them (docs/plugins-visual.md says so). Knob labels are at `SMALL_FONT_SIZE`.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::{Asset, lmms};
use crate::spec::{Bind, ComboItem, Control, PanelSpec, Text, combo, knob, label, led};
use crate::widgets::{KnobStyle, LedColor};

static BITCRUSH: Asset = asset!("bitcrush/artwork.svg");
static BASSBOOSTER: Asset = asset!("bassbooster/artwork.svg");
static REVERBSC: Asset = asset!("reverbsc/artwork.svg");
static FLANGER: Asset = asset!("flanger/artwork.svg");
static AMPLIFIER: Asset = asset!("amplifier/artwork.svg");
static STEREOMATRIX: Asset = asset!("stereomatrix/artwork.svg");
static DUALFILTER: Asset = asset!("dualfilter/artwork.svg");

pub static ASSETS: &[&Asset] = &[&BITCRUSH, &BASSBOOSTER, &REVERBSC, &FLANGER, &AMPLIFIER, &STEREOMATRIX, &DUALFILTER];

/// The fill under the effects' artwork: the bottom color of their gradient (`#08090c`)
const FILL: u32 = 0x08090c;
const MARGIN: f32 = 9.0;
/// `knob02.png`
const KNOB_W: f32 = 27.0;

/// The x of the `i`-th of `n` knobs, centered in equal cells across `width` inside the layout margins
fn cell(i: usize, n: usize, width: f32) -> f32 {
    let w = (width - 2.0 * MARGIN) / n as f32;
    (MARGIN + w * (i as f32 + 0.5) - KNOB_W / 2.0).round()
}

fn b26(
    param: &'static str,
    x: f32,
    y: f32,
    step: f64,
    name: &'static str,
    hint: &'static str,
    unit: &'static str,
) -> Control {
    knob(param, x, y, KnobStyle::Bright26, step, name, hint, unit)
}

pub static BITCRUSH_PANEL: LazyLock<PanelSpec> = LazyLock::new(|| PanelSpec {
    plugin: "bitcrush",
    size: (181.0, 128.0),
    background: Some(&BITCRUSH),
    fill: FILL,
    controls: vec![
        b26("inputGain", 16.0, 32.0, 0.1, "Input gain", "Input gain:", " dBFS").label("GAIN"),
        b26("inputNoise", 14.0, 76.0, 0.1, "Input noise", "Input noise:", "%").label("NOISE"),
        b26("outputGain", 138.0, 32.0, 0.1, "Output gain", "Output gain:", " dBFS").label("GAIN"),
        b26("outputClip", 138.0, 76.0, 0.1, "Output clip", "Output clip:", " dBFS").label("CLIP"),
        led(Bind::Switch { param: "rateEnabled" }, "rateEnabled", 64.0, 14.0, LedColor::Green, "", "Rate enabled")
            .tip("Enable sample-rate crushing"),
        led(Bind::Switch { param: "depthEnabled" }, "depthEnabled", 101.0, 14.0, LedColor::Green, "", "Depth enabled")
            .tip("Enable bit-depth crushing"),
        b26("sampleRate", 59.0, 32.0, 1.0, "Sample rate", "Sample rate:", " Hz").label("FREQ"),
        b26("stereoDifference", 72.0, 76.0, 0.1, "Stereo difference", "Stereo difference:", "%").label("STEREO"),
        b26("levels", 92.0, 32.0, 0.01, "Levels", "Levels:", "").label("QUANT"),
    ],
    labels: vec![label(24.0, 15.0, "IN"), label(139.0, 15.0, "OUT")],
    file_drop: None,
});

pub static BASSBOOSTER_PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let (w, y) = (128.0, MARGIN + 4.0);
    PanelSpec {
        plugin: "bassbooster",
        size: (w, 64.0),
        background: Some(&BASSBOOSTER),
        fill: FILL,
        controls: vec![
            b26("frequency", cell(0, 3, w), y, 1.0, "Frequency", "Frequency:", "Hz").label("FREQ"),
            b26("gain", cell(1, 3, w), y, 0.05, "Gain", "Gain:", "").label("GAIN"),
            b26("ratio", cell(2, 3, w), y, 0.1, "Ratio", "Ratio:", "").label("RATIO"),
        ],
        labels: Vec::new(),
        file_drop: None,
    }
});

pub static REVERBSC_PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let w = 185.0;
    PanelSpec {
        plugin: "reverbsc",
        size: (w, 55.0),
        background: Some(&REVERBSC),
        fill: FILL,
        controls: vec![
            b26("inputGain", cell(0, 4, w), MARGIN, 0.1, "Input gain", "Input gain:", "dB").label("Input"),
            b26("size", cell(1, 4, w), MARGIN, 0.01, "Size", "Size:", "").label("Size"),
            b26("color", cell(2, 4, w), MARGIN, 0.1, "Color", "Color:", "").label("Color"),
            b26("outputGain", cell(3, 4, w), MARGIN, 0.1, "Output gain", "Output gain:", "dB").label("Output"),
        ],
        labels: Vec::new(),
        file_drop: None,
    }
});

pub static FLANGER_PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let w = 195.0;
    let k = |i| cell(i, 6, w);
    PanelSpec {
        plugin: "flanger",
        size: (w, 75.0),
        background: Some(&FLANGER),
        fill: FILL,
        controls: vec![
            b26("delayTime", k(0), MARGIN, 0.0001, "Delay samples", "Delay time:", "s").label("DELAY"),
            b26("lfoPeriod", k(1), MARGIN, 0.0001, "LFO frequency", "Period:", " Sec").label("RATE"),
            b26("lfoAmount", k(2), MARGIN, 0.0001, "Amount", "Amount:", "").label("AMNT"),
            b26("stereoPhase", k(3), MARGIN, 0.0001, "Stereo phase", "Phase:", " degrees").label("PHASE"),
            b26("feedback", k(4), MARGIN, 0.0001, "Feedback", "Feedback amount:", "")
                .label("FDBK")
                .center(0.0)
                .volume_at(1.0),
            b26("noise", k(5), MARGIN, 0.0001, "Noise", "White noise amount:", "").label("NOISE").volume_at(1.0),
            led(Bind::Switch { param: "invert" }, "invert", MARGIN, 57.0, LedColor::Yellow, "Invert", "Invert"),
        ],
        labels: Vec::new(),
        file_drop: None,
    }
});

pub static AMPLIFIER_PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    // a 2×2 grid: two columns of (82 − 6) / 2 = 38 pixels, rows of the knob's 39 pixels and the spacing of 6
    let (x0, x1) = (MARGIN + 19.0 - KNOB_W / 2.0, MARGIN + 38.0 + 6.0 + 19.0 - KNOB_W / 2.0);
    let (y0, y1) = (MARGIN, MARGIN + 39.0 + 6.0);
    PanelSpec {
        plugin: "amplifier",
        size: (100.0, 110.0),
        background: Some(&AMPLIFIER),
        fill: FILL,
        controls: vec![
            b26("volume", x0.round(), y0, 0.1, "Volume", "Volume:", "%").label("VOL").volume(),
            b26("panning", x1.round(), y0, 0.1, "Panning", "Panning:", "%").label("PAN"),
            b26("leftGain", x0.round(), y1, 0.1, "Left gain", "Left gain:", "%").label("LEFT").volume(),
            b26("rightGain", x1.round(), y1, 0.1, "Right gain", "Right gain:", "%").label("RIGHT").volume(),
        ],
        labels: Vec::new(),
        file_drop: None,
    }
});

pub static STEREOMATRIX_PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    // on the four circles of the artwork, centers 24.2, 61.4, 98.6, 135.8 at y 92.5
    let at = |cx: f32| cx - KNOB_W / 2.0;
    let y = 92.5 - KNOB_W / 2.0;
    PanelSpec {
        plugin: "stereomatrix",
        size: (160.0, 185.0),
        background: Some(&STEREOMATRIX),
        fill: FILL,
        controls: vec![
            b26("leftToLeft", at(24.2), y, 0.01, "Left to Left", "Left to Left Vol:", "").center(0.0),
            b26("leftToRight", at(61.4), y, 0.01, "Left to Right", "Left to Right Vol:", "").center(0.0),
            b26("rightToLeft", at(98.6), y, 0.01, "Right to Left", "Right to Left Vol:", "").center(0.0),
            b26("rightToRight", at(135.8), y, 0.01, "Right to Right", "Right to Right Vol:", "").center(0.0),
        ],
        labels: Vec::new(),
        file_drop: None,
    }
});

/// The filter types in the legacy's order, with their texts and icons (`DualFilterControls`)
fn filter_types() -> Vec<ComboItem> {
    let item = |key, text, icon: &'static Asset| ComboItem { key, text: Text::new(text), icon: Some(icon) };
    vec![
        item("low-pass", "Low-pass", &lmms::FILTER_LP),
        item("high-pass", "Hi-pass", &lmms::FILTER_HP),
        item("band-pass-csg", "Band-pass csg", &lmms::FILTER_BP),
        item("band-pass-czpg", "Band-pass czpg", &lmms::FILTER_BP),
        item("notch", "Notch", &lmms::FILTER_NOTCH),
        item("all-pass", "All-pass", &lmms::FILTER_AP),
        item("moog", "Moog", &lmms::FILTER_LP),
        item("double-low-pass", "2x Low-pass", &lmms::FILTER_2LP),
        item("rc-low-pass-12", "RC Low-pass 12 dB/oct", &lmms::FILTER_LP),
        item("rc-band-pass-12", "RC Band-pass 12 dB/oct", &lmms::FILTER_BP),
        item("rc-high-pass-12", "RC High-pass 12 dB/oct", &lmms::FILTER_HP),
        item("rc-low-pass-24", "RC Low-pass 24 dB/oct", &lmms::FILTER_LP),
        item("rc-band-pass-24", "RC Band-pass 24 dB/oct", &lmms::FILTER_BP),
        item("rc-high-pass-24", "RC High-pass 24 dB/oct", &lmms::FILTER_HP),
        item("formant", "Vocal Formant", &lmms::FILTER_HP),
        item("double-moog", "2x Moog", &lmms::FILTER_2LP),
        item("sv-low-pass", "SV Low-pass", &lmms::FILTER_LP),
        item("sv-band-pass", "SV Band-pass", &lmms::FILTER_BP),
        item("sv-high-pass", "SV High-pass", &lmms::FILTER_HP),
        item("sv-notch", "SV Notch", &lmms::FILTER_NOTCH),
        item("fast-formant", "Fast Formant", &lmms::FILTER_HP),
        item("tripole", "Tripole", &lmms::FILTER_LP),
    ]
}

pub static DUALFILTER_PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let side = |n: u32, x: f32| -> Vec<Control> {
        let k = |s: &str| -> &'static str { Box::leak(format!("{s}{n}").into_boxed_str()) };
        let enabled: &'static str = Box::leak(format!("filter{n}Enabled").into_boxed_str());
        let kind: &'static str = Box::leak(format!("filter{n}Type").into_boxed_str());
        vec![
            led(Bind::Switch { param: enabled }, enabled, x + 3.0, 11.0, LedColor::Green, "", "Filter %1 enabled")
                .tip("Enable/disable filter %1")
                .n(n),
            b26(k("cutoff"), x + 15.0, 26.0, 1.0, "Cutoff frequency %1", "Cutoff frequency", "Hz").label("FREQ").n(n),
            b26(k("resonance"), x + 65.0, 26.0, 0.01, "Q/Resonance %1", "Resonance", "").label("RESO").n(n),
            b26(k("gain"), x + 115.0, 26.0, 0.1, "Gain %1", "Gain", "%").label("GAIN").volume().n(n),
            combo(kind, x + 10.0, 70.0, 137.0, "Filter %1 type", filter_types()).n(n),
        ]
    };
    let mut controls = side(1, 9.0);
    controls.push(b26("mix", 173.0, 37.0, 0.01, "Mix", "Mix", "").label("MIX").center(0.0));
    controls.extend(side(2, 207.0));
    PanelSpec {
        plugin: "dualfilter",
        size: (373.0, 109.0),
        background: Some(&DUALFILTER),
        fill: FILL,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});
