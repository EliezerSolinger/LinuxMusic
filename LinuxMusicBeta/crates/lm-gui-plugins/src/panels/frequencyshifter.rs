//! FrequencyShifter (legado/plugins/FrequencyShifter/FrequencyShifterControlDialog.cpp): the 288×360 artwork, white
//! `Styled` knobs of four sizes (`setupKnobGeometry`: centered, outer radius `cx − 3`, inner radius by size), the big
//! red float LCD of the shift, the pixmap buttons and the route group. The help button opens a text window in the
//! legacy, not part of the panel.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, PanelSpec, button, knob, lcd_float};
use crate::widgets::{KnobStyle, LcdSheet, Styled};

pub const ID: &str = "frequencyshifter";

static ARTWORK: Asset = asset!("frequencyshifter/artwork.png");
static ANTIREFLECT_ON: Asset = asset!("frequencyshifter/antireflect_on.png");
static ANTIREFLECT_OFF: Asset = asset!("frequencyshifter/antireflect_off.png");
static SEND_ON: Asset = asset!("frequencyshifter/send_on.png");
static SEND_OFF: Asset = asset!("frequencyshifter/send_off.png");
static PASS_ON: Asset = asset!("frequencyshifter/pass_on.png");
static PASS_OFF: Asset = asset!("frequencyshifter/pass_off.png");
static MUTE_ON: Asset = asset!("frequencyshifter/mute_on.png");
static MUTE_OFF: Asset = asset!("frequencyshifter/mute_off.png");
static RESET_SHIFTER_ON: Asset = asset!("frequencyshifter/reset_shifter_on.png");
static RESET_SHIFTER_OFF: Asset = asset!("frequencyshifter/reset_shifter_off.png");
static RESET_LFO_ON: Asset = asset!("frequencyshifter/reset_lfo_on.png");
static RESET_LFO_OFF: Asset = asset!("frequencyshifter/reset_lfo_off.png");

pub static ASSETS: &[&Asset] = &[
    &ARTWORK,
    &ANTIREFLECT_ON,
    &ANTIREFLECT_OFF,
    &SEND_ON,
    &SEND_OFF,
    &PASS_ON,
    &PASS_OFF,
    &MUTE_ON,
    &MUTE_OFF,
    &RESET_SHIFTER_ON,
    &RESET_SHIFTER_OFF,
    &RESET_LFO_ON,
    &RESET_LFO_OFF,
];

/// `setupKnobGeometry(k, w, h)` with the style sheet's white pointer: `outer = max(1, cx − 3)`, `inner = max(1,
/// outer − (w ≥ 40 ? 16 : w ≥ 24 ? 10 : 6))`, `lineWidth` 3 (2 for the glide knobs and the phase)
fn styled(size: f32, line_width: f32) -> KnobStyle {
    let c = (size / 2.0).floor();
    let outer = (c - 3.0).max(1.0);
    let inner = (outer
        - if size >= 40.0 {
            16.0
        } else if size >= 24.0 {
            10.0
        } else {
            6.0
        })
    .max(1.0);
    KnobStyle::Styled(Styled {
        w: size,
        h: size,
        color: Rgba::hex(0xffffff),
        outer: Some(Rgba::hex(0xffffff)),
        inner_radius: inner,
        outer_radius: outer,
        center: (c, c),
        line_width,
    })
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let k = |p, x, y, size, lw, step, name, hint, unit| -> Control {
        knob(p, x, y, styled(size, lw), step, name, hint, unit)
    };
    let route = |key, id, y, on, off, tip: &'static str| {
        button(Bind::Choice { field: "routeMode", key }, id, 188.0, y, on, off, "Route Mode").tip(tip)
    };
    PanelSpec {
        plugin: ID,
        size: (288.0, 360.0),
        background: Some(&ARTWORK),
        fill: 0x000000,
        controls: vec![
            lcd_float("frequencyShift", 100.0, 43.0, LcdSheet::Red19, 6, 3, 0.001, "Frequency Shift"),
            k("mix", 18.0, 30.0, 60.0, 3.0, 0.01, "Mix", "Mix", ""),
            k("spreadShift", 235.0, 24.0, 24.0, 3.0, 0.01, "Spread Shift", "Spread", "Hz"),
            k("phase", 235.0, 72.0, 24.0, 2.0, 0.01, "Phase", "Phase", ""),
            k("ring", 24.0, 115.0, 36.0, 3.0, 0.01, "Ring", "Ring", ""),
            k("harmonics", 72.0, 115.0, 36.0, 3.0, 0.01, "Harmonics", "Harmonics", ""),
            k("tone", 120.0, 115.0, 36.0, 3.0, 1.0, "Tone", "Tone", "Hz").log(),
            k("glide", 200.0, 147.0, 19.0, 2.0, 0.001, "Glide", "Glide", ""),
            k("lfoAmount", 18.0, 200.0, 36.0, 3.0, 0.01, "LFO Amount", "LFO", "Hz"),
            k("lfoRate", 66.0, 200.0, 36.0, 3.0, 0.01, "LFO Rate", "LFO Rate", "Hz"),
            k("lfoStereoPhase", 114.0, 200.0, 36.0, 3.0, 0.001, "LFO StereoPhase", "LFO Stereo Phase", ""),
            k("delayLength", 18.0, 282.0, 36.0, 3.0, 0.01, "Delay Length", "Delay Length", "ms"),
            k("feedback", 114.0, 282.0, 36.0, 3.0, 0.001, "Feedback", "Feedback", ""),
            k("fineDelayLength", 24.0, 324.0, 24.0, 3.0, 0.001, "Fine Delay Length", "Delay Length (fine)", "ms"),
            k("delayDamping", 120.0, 324.0, 24.0, 3.0, 1.0, "Delay Damping", "Delay Damping", "Hz").log(),
            k("delayGlide", 245.0, 315.0, 19.0, 2.0, 0.001, "Delay Glide", "Delay Glide", ""),
            button(
                Bind::Switch { param: "antireflect" },
                "antireflect",
                188.0,
                122.0,
                &ANTIREFLECT_ON,
                &ANTIREFLECT_OFF,
                "Antireflect",
            )
            .tip("Anti-reflect"),
            route("series", "route.series", 199.0, &SEND_ON, &SEND_OFF, "Route: Send"),
            route("parallel-add", "route.parallel-add", 217.0, &PASS_ON, &PASS_OFF, "Route: Pass"),
            route("parallel-mix", "route.parallel-mix", 235.0, &MUTE_ON, &MUTE_OFF, "Route: Mute"),
            button(
                Bind::Momentary { param: "resetShifter" },
                "resetShifter",
                77.0,
                5.0,
                &RESET_SHIFTER_ON,
                &RESET_SHIFTER_OFF,
                "Reset Shifter",
            )
            .tip("Reset the shifter's oscillator phases to 0 (automatable)"),
            button(
                Bind::Momentary { param: "resetLfo" },
                "resetLfo",
                60.0,
                179.0,
                &RESET_LFO_ON,
                &RESET_LFO_OFF,
                "Reset LFO",
            )
            .tip("Reset the LFO phase to 0 (automatable)"),
        ],
        view_backgrounds: &[],
        labels: Vec::new(),
        file_drop: None,
    }
});
