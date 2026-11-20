//! LB302 (legado/plugins/Lb302/Lb302.cpp, `Lb302SynthView`): the 250×250 artwork, six `Bright26` knobs, three
//! yellow LEDs (24 dB, slide, dead; the accent LED was commented out in the legacy) and the twelve wave buttons with
//! the theme's wave icons (the four band-limited ones reuse the icons of their shapes).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::{Asset, lmms};
use crate::spec::{Bind, Control, PanelSpec, button, knob, led};
use crate::widgets::{KnobStyle, LedColor};

pub const ID: &str = "lb302";

static ARTWORK: Asset = asset!("lb302/artwork.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK];

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let k = |p, x, y, step, name, hint| knob(p, x, y, KnobStyle::Bright26, step, name, hint, "");
    let mut controls = vec![
        k("vcfCutoffFrequency", 75.0, 130.0, 0.005, "VCF Cutoff Frequency", "Cutoff Freq:"),
        k("vcfResonance", 120.0, 130.0, 0.005, "VCF Resonance", "Resonance:"),
        k("vcfEnvelopeMod", 165.0, 130.0, 0.005, "VCF Envelope Mod", "Env Mod:"),
        k("vcfEnvelopeDecay", 210.0, 130.0, 0.005, "VCF Envelope Decay", "Decay:"),
        led(Bind::Switch { param: "slide" }, "slide", 10.0, 180.0, LedColor::Yellow, "", "Slide"),
        led(Bind::Switch { param: "dead" }, "dead", 10.0, 200.0, LedColor::Yellow, "", "Dead"),
        led(Bind::Switch { param: "db24" }, "db24", 10.0, 150.0, LedColor::Yellow, "", "24dB/oct Filter")
            .tip("303-es-que, 24dB/octave, 3 pole filter"),
        k("slideDecay", 210.0, 75.0, 0.005, "Slide Decay", "Slide Decay:"),
        k("distortion", 210.0, 190.0, 0.01, "Distortion", "DIST:"),
    ];
    // (key, active, inactive, x, tooltip): x = 10 + 16 i, the band-limited ones after a gap of 8
    let waves: [(&str, &'static Asset, &'static Asset, f32, &str); 12] = [
        ("saw", &lmms::SAW_WAVE_ACTIVE, &lmms::SAW_WAVE_INACTIVE, 10.0, "Click here for a sawtooth wave."),
        (
            "triangle",
            &lmms::TRIANGLE_WAVE_ACTIVE,
            &lmms::TRIANGLE_WAVE_INACTIVE,
            26.0,
            "Click here for a triangle wave.",
        ),
        ("square", &lmms::SQUARE_WAVE_ACTIVE, &lmms::SQUARE_WAVE_INACTIVE, 42.0, "Click here for a square wave."),
        (
            "round-square",
            &lmms::ROUND_SQUARE_WAVE_ACTIVE,
            &lmms::ROUND_SQUARE_WAVE_INACTIVE,
            58.0,
            "Click here for a square wave with a rounded end.",
        ),
        ("moog", &lmms::MOOG_SAW_WAVE_ACTIVE, &lmms::MOOG_SAW_WAVE_INACTIVE, 74.0, "Click here for a moog-like wave."),
        ("sine", &lmms::SIN_WAVE_ACTIVE, &lmms::SIN_WAVE_INACTIVE, 90.0, "Click for a sine wave."),
        ("exponential", &lmms::EXP_WAVE_ACTIVE, &lmms::EXP_WAVE_INACTIVE, 106.0, "Click here for an exponential wave."),
        (
            "white-noise",
            &lmms::WHITE_NOISE_WAVE_ACTIVE,
            &lmms::WHITE_NOISE_WAVE_INACTIVE,
            122.0,
            "Click here for white noise.",
        ),
        (
            "bl-saw",
            &lmms::SAW_WAVE_ACTIVE,
            &lmms::SAW_WAVE_INACTIVE,
            146.0,
            "Click here for bandlimited sawtooth wave.",
        ),
        (
            "bl-square",
            &lmms::SQUARE_WAVE_ACTIVE,
            &lmms::SQUARE_WAVE_INACTIVE,
            162.0,
            "Click here for bandlimited square wave.",
        ),
        (
            "bl-triangle",
            &lmms::TRIANGLE_WAVE_ACTIVE,
            &lmms::TRIANGLE_WAVE_INACTIVE,
            178.0,
            "Click here for bandlimited triangle wave.",
        ),
        (
            "bl-moog",
            &lmms::MOOG_SAW_WAVE_ACTIVE,
            &lmms::MOOG_SAW_WAVE_INACTIVE,
            194.0,
            "Click here for bandlimited moog-like wave.",
        ),
    ];
    for (key, on, off, x, tip) in waves {
        let id: &'static str = Box::leak(format!("waveform.{key}").into_boxed_str());
        let c: Control = button(Bind::Choice { field: "waveform", key }, id, x, 96.0, on, off, "Waveform").tip(tip);
        controls.push(c);
    }
    PanelSpec {
        plugin: ID,
        size: (250.0, 250.0),
        background: Some(&ARTWORK),
        fill: 0x000000,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});
