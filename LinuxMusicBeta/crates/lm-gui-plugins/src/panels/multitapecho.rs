//! MultitapEcho (legado/plugins/MultitapEcho/MultitapEchoControlDialog.cpp): the 245×300 artwork, two 204×105 bar
//! graphs over `graph_bg.png` (the echoes' amplitudes and low-pass amounts; as many bars as the Steps LCD says), the
//! Steps LCD, three knobs and the swap switch.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, GraphSpec, GraphStyle, PanelSpec, graph, knob, lcd, led};
use crate::widgets::{KnobStyle, LcdSheet, LedColor};

pub const ID: &str = "multitapecho";

static ARTWORK: Asset = asset!("multitapecho/artwork.png");
static GRAPH_BG: Asset = asset!("multitapecho/graph_bg.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK, &GRAPH_BG];

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let bars = |field, min, max, color: Rgba, step, y, name| {
        graph(
            GraphSpec {
                field,
                w: 204.0,
                h: 105.0,
                min,
                max,
                length: 32,
                length_param: Some("steps"),
                style: GraphStyle::Bar,
                color,
                background: Some(&GRAPH_BG),
                foreground: None,
                step,
            },
            30.0,
            y,
            name,
        )
    };
    PanelSpec {
        plugin: ID,
        size: (245.0, 300.0),
        background: Some(&ARTWORK),
        fill: 0x000000,
        controls: vec![
            bars("amplitude", -60.0, 0.0, Rgba::rgb(11, 213, 86), 0.0, 10.0, "Amplitude"),
            bars("lowpass", 0.0, 3.0, Rgba::rgb(0, 200, 187), 0.0, 125.0, "Low-pass"),
            lcd("steps", 20.0, 245.0, LcdSheet::Green19, 2, "Steps"),
            knob("stepLength", 100.0, 245.0, KnobStyle::Bright26, 0.1, "Step length", "Step length:", " ms")
                .label("Length"),
            knob("dryGain", 150.0, 245.0, KnobStyle::Bright26, 0.1, "Dry gain", "Dry gain:", " dBFS").label("Dry"),
            knob("lowpassStages", 200.0, 245.0, KnobStyle::Bright26, 1.0, "Lowpass stages", "Low-pass stages:", "x")
                .label("Stages"),
            led(
                Bind::Switch { param: "swapInputs" },
                "swapInputs",
                20.0,
                275.0,
                LedColor::Green,
                "Swap inputs",
                "Swap inputs",
            )
            .tip("Swap left and right input channels for reflections"),
        ],
        labels: Vec::new(),
        file_drop: None,
    }
});
