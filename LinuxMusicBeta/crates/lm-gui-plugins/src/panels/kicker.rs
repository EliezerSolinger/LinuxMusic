//! Kicker (legado/plugins/Kicker/Kicker.cpp, `KickerInstrumentView`): the 250×250 artwork, two large knobs for the
//! frequencies, nine small ones and the two "note" LEDs. The knobs are the style sheet's `Knob#largeKnob` and
//! `Knob#smallKnob`. The length knob is a `TempoSyncKnob` in the legacy; the new Kicker has no tempo sync for it, so
//! it is a plain knob (the sync entries of its menu are not there).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, PanelSpec, knob, led};
use crate::widgets::{KnobStyle, LedColor, Styled};

pub const ID: &str = "kicker";

static ARTWORK: Asset = asset!("kicker/artwork.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK];

/// `KickerKnob`/`KickerEnvKnob`: 29×29, `Knob#smallKnob`
fn small() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 29.0,
        h: 29.0,
        color: Rgba::hex(0x595959),
        outer: Some(Rgba::rgb(0, 0, 0)),
        inner_radius: 3.0,
        outer_radius: 11.0,
        center: (14.5, 14.5),
        line_width: 2.0,
    })
}

/// `KickerLargeKnob`: 34×34, `Knob#largeKnob`
fn large() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 34.0,
        h: 34.0,
        color: Rgba::hex(0x0c3b89),
        outer: Some(Rgba::hex(0x519fff)),
        inner_radius: 12.0,
        outer_radius: 14.5,
        center: (17.0, 17.0),
        line_width: 3.0,
    })
}

const ROW1: f32 = 14.0;
const ROW2: f32 = ROW1 + 56.0;
const ROW3: f32 = ROW2 + 56.0;
const LED_ROW: f32 = 63.0;
const COL1: f32 = 14.0;
const COL2: f32 = COL1 + 56.0;
const COL3: f32 = COL2 + 56.0;
const COL4: f32 = COL3 + 41.0;
const COL5: f32 = COL4 + 41.0;
const END_COL: f32 = COL1 + 48.0;

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| PanelSpec {
    plugin: ID,
    size: (250.0, 250.0),
    background: Some(&ARTWORK),
    fill: 0x000000,
    controls: vec![
        knob("startFrequency", COL1, ROW1, large(), 1.0, "Start frequency", "Start frequency:", "Hz"),
        knob("endFrequency", END_COL, ROW1, large(), 1.0, "End frequency", "End frequency:", "Hz"),
        knob("frequencySlope", COL3, ROW1, small(), 0.001, "Frequency slope", "Frequency slope:", ""),
        knob("gain", COL1, ROW3, small(), 0.05, "Gain", "Gain:", ""),
        knob("length", COL2, ROW3, small(), 1.0, "Length", "Envelope length:", "ms"),
        knob("envelopeSlope", COL3, ROW3, small(), 0.001, "Envelope slope", "Envelope slope:", ""),
        knob("click", COL5, ROW1, small(), 0.05, "Click", "Click:", ""),
        knob("attack", COL4 + 2.0, ROW3, small(), 0.1, "Attack", "Attack:", "ms"),
        knob("noise", COL5, ROW3, small(), 0.01, "Noise", "Noise:", ""),
        knob("startDistortion", COL4, ROW2, small(), 0.1, "Start distortion", "Start distortion:", ""),
        knob("endDistortion", COL5, ROW2, small(), 0.1, "End distortion", "End distortion:", ""),
        led(
            Bind::Switch { param: "startFromNote" },
            "startFromNote",
            COL1 + 8.0,
            LED_ROW,
            LedColor::Green,
            "",
            "Start from note",
        ),
        led(
            Bind::Switch { param: "endToNote" },
            "endToNote",
            END_COL + 8.0,
            LED_ROW,
            LedColor::Green,
            "",
            "End to note",
        ),
    ],
    view_backgrounds: &[],
    labels: Vec::new(),
    file_drop: None,
});
