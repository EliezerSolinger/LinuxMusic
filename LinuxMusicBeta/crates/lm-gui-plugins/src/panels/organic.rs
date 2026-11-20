//! Organic (legado/plugins/Organic/Organic.cpp, `OrganicInstrumentView`): the 250×250 artwork, eight columns of five
//! small styled knobs (harmonic, waveform, volume, panning, detune; `x = 53 + 24 i`, rows from y 65 every 26), the
//! large distortion and volume knobs, and the "Randomise" button. The style sheet's `OrganicInstrumentView Knob`,
//! `#harmKnob`, `#fx1Knob` and `#volKnob` give the knobs' colors and sizes.
//!
//! The waveform was a number in the legacy and is a choice field here (`shape<n>`, U30): its knob turns the field,
//! one key per step, in the legacy's order.

use std::sync::LazyLock;

use lm_port::Value;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, PanelSpec, PressCx, Target, button, knob};
use crate::widgets::{KnobStyle, Styled};

pub const ID: &str = "organic";

static ARTWORK: Asset = asset!("organic/artwork.png");
static RANDOMISE: Asset = asset!("organic/randomise.png");
static RANDOMISE_PRESSED: Asset = asset!("organic/randomise_pressed.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK, &RANDOMISE, &RANDOMISE_PRESSED];

/// The shapes in the legacy knob's order (`WAVEFORM_NAMES`)
pub static SHAPES: [&str; 6] = ["sine", "saw", "square", "triangle", "moog-saw", "exponential"];
const SHAPE_NAMES: [&str; 6] =
    ["Sine wave", "Saw wave", "Square wave", "Triangle wave", "Moog saw wave", "Exponential wave"];
const HARMONIC_NAMES: [&str; 18] = [
    "Octave below",
    "Fifth below",
    "Fundamental",
    "2nd harmonic",
    "3rd harmonic",
    "4th harmonic",
    "5th harmonic",
    "6th harmonic",
    "7th harmonic",
    "8th harmonic",
    "9th harmonic",
    "10th harmonic",
    "11th harmonic",
    "12th harmonic",
    "13th harmonic",
    "14th harmonic",
    "15th harmonic",
    "16th harmonic",
];

fn harmonic_name(v: f64) -> String {
    format!(" ({})", crate::i18n::tr(HARMONIC_NAMES[(v.max(0.0) as usize).min(17)]))
}

fn shape_name(v: f64) -> String {
    format!(" ({})", crate::i18n::tr(SHAPE_NAMES[(v.max(0.0) as usize).min(5)]))
}

/// `OrganicKnob` (21×21) in the theme's green, or `#harmKnob`'s purple
fn small(harm: bool) -> KnobStyle {
    let (color, outer) = if harm {
        (Rgba::rgb(205, 98, 216), Rgba::rgb(18, 4, 18))
    } else {
        (Rgba::rgb(124, 207, 98), Rgba::rgb(13, 42, 4))
    };
    KnobStyle::Styled(Styled {
        w: 21.0,
        h: 21.0,
        color,
        outer: Some(outer),
        inner_radius: 2.0,
        outer_radius: 7.5,
        center: (10.5, 10.5),
        line_width: 1.5,
    })
}

/// `#fx1Knob` and `#volKnob`: resized to 37×47 after the 21×21
fn large() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 37.0,
        h: 47.0,
        color: Rgba::rgb(157, 157, 157),
        outer: Some(Rgba::rgb(37, 37, 37)),
        inner_radius: 4.0,
        outer_radius: 10.0,
        center: (18.5, 13.8),
        line_width: 2.0,
    })
}

/// `OrganicInstrument::randomiseSettings`: each oscillator's volume 0–100, detune ±5 cents, panning centered and a
/// random waveform
fn randomise(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let rand = &mut cx.rand;
    let mut out = Vec::new();
    for n in 1..=8 {
        let k = |s: &str| -> &'static str { Box::leak(format!("{s}{n}").into_boxed_str()) };
        let real = |v: f64| Value::real(v).unwrap_or_default();
        out.push((Target::Param(k("volume")), real((rand() * 100.0).round())));
        out.push((Target::Param(k("detune")), real((rand() * 10.0 - 5.0).round())));
        out.push((Target::Param(k("panning")), real(0.0)));
        out.push((Target::Field(k("shape")), Value::text(SHAPES[((rand() * 6.0) as usize).min(5)])));
    }
    out
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls: Vec<Control> = vec![
        knob("distortion", 15.0, 201.0, large(), 0.01, "Distortion", "Distortion:", ""),
        knob("volume", 60.0, 201.0, large(), 1.0, "Volume", "Volume:", "%").volume(),
        button(Bind::Press(randomise), "randomise", 148.0, 224.0, &RANDOMISE_PRESSED, &RANDOMISE, "Randomise"),
    ];
    for i in 0..8u32 {
        let n = i + 1;
        let x = 53.0 + 24.0 * i as f32;
        let k = |s: &str| -> &'static str { Box::leak(format!("{s}{n}").into_boxed_str()) };
        controls.extend([
            knob(k("harmonic"), x, 65.0, small(true), 1.0, "Osc %1 harmonic", "Osc %1 harmonic:", "")
                .unit_of(harmonic_name)
                .n(n),
            knob(k("shape"), x, 91.0, small(false), 1.0, "Osc %1 waveform", "Osc %1 waveform:", "")
                .choices(&SHAPES)
                .unit_of(shape_name)
                .n(n),
            knob(k("volume"), x, 117.0, small(false), 1.0, "Osc %1 volume", "Osc %1 volume:", "%").volume().n(n),
            knob(k("panning"), x, 143.0, small(false), 1.0, "Osc %1 panning", "Osc %1 panning:", "").n(n),
            knob(
                k("detune"),
                x,
                169.0,
                small(false),
                1.0,
                "Osc %1 stereo detuning",
                "Osc %1 stereo detuning",
                " cents",
            )
            .n(n),
        ]);
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
