//! GranularPitchShifter (legado/plugins/GranularPitchShifter/GranularPitchShifterControlDialog.cpp): the 305×180
//! artwork, nine Bright26 knobs, two Small17 knobs, the pitch and pitch-spread float LCDs, the prefilter button and
//! the ring buffer length combo box. The help button opens a text window in the legacy, not part of the panel.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::spec::{Bind, ComboItem, PanelSpec, Text, button, combo, knob, lcd_float};
use crate::widgets::{KnobStyle, LcdSheet};

pub const ID: &str = "granularpitchshifter";

static ARTWORK: Asset = asset!("granularpitchshifter/artwork.png");
static PREFILTER_ON: Asset = asset!("granularpitchshifter/prefilter_active.png");
static PREFILTER_OFF: Asset = asset!("granularpitchshifter/prefilter_inactive.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK, &PREFILTER_ON, &PREFILTER_OFF];

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let b = |p, x, y, step, name, hint, unit| knob(p, x, y, KnobStyle::Bright26, step, name, hint, unit);
    let s = |p, x, y, step, name, hint, unit| knob(p, x, y, KnobStyle::Small17, step, name, hint, unit);
    let item = |key, text| ComboItem { key, text: Text::new(text), icon: None };
    PanelSpec {
        plugin: ID,
        size: (305.0, 180.0),
        background: Some(&ARTWORK),
        fill: 0x000000,
        controls: vec![
            b("grainSize", 19.0, 78.0, 0.01, "Grain Size", "Grain Size:", " Hz").log(),
            b("spray", 116.0, 10.0, 0.001, "Spray", "Spray:", " seconds"),
            b("jitter", 158.0, 10.0, 0.001, "Jitter", "Jitter:", " octaves"),
            b("twitch", 200.0, 10.0, 0.001, "Twitch", "Twitch:", " octaves"),
            b("sprayStereo", 188.0, 60.0, 0.001, "Spray Stereo", "Spray Stereo Spread:", ""),
            b("shape", 135.0, 110.0, 0.001, "Shape", "Grain Shape:", ""),
            b("fadeLength", 188.0, 110.0, 0.001, "Fade Length", "Fade Length:", "").log(),
            b("feedback", 258.0, 45.0, 0.001, "Feedback", "Feedback:", ""),
            b(
                "minimumAllowedLatency",
                258.0,
                92.0,
                0.001,
                "Minimum Allowed Latency",
                "Minimum Allowed Latency:",
                " seconds",
            ),
            s("density", 66.0, 157.0, 0.001, "Density", "Density:", "x").log(),
            s("glide", 8.0, 157.0, 0.001, "Glide", "Glide:", " seconds"),
            lcd_float("pitch", 15.0, 41.0, LcdSheet::Green11, 3, 2, 0.01, "Pitch").tip("Pitch"),
            lcd_float("pitchStereoSpread", 133.0, 66.0, LcdSheet::Green11, 3, 2, 0.01, "Pitch Stereo Spread")
                .tip("Pitch Stereo Spread"),
            button(
                Bind::Switch { param: "prefilter" },
                "prefilter",
                8.0,
                133.0,
                &PREFILTER_ON,
                &PREFILTER_OFF,
                "Prefilter",
            )
            .tip("Prefilter"),
            combo(
                "ringBufferLength",
                189.0,
                155.0,
                80.0,
                "Ring Buffer Length",
                vec![
                    item("5s", "5 Seconds"),
                    item("10s-size", "10 Seconds (Size)"),
                    item("40s-size-pitch", "40 Seconds (Size and Pitch)"),
                    item("40s-size-spray-jitter", "40 Seconds (Size and Spray and Jitter)"),
                    item("120s-all", "120 Seconds (All of the above)"),
                ],
            ),
        ],
        view_backgrounds: &[],
        labels: Vec::new(),
        file_drop: None,
    }
});
