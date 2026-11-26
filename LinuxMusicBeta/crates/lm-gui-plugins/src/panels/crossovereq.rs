//! CrossoverEQ (legado/plugins/CrossoverEQ/CrossoverEQControlDialog.cpp): the 167×218 artwork, three crossover knobs
//! (`Bright26`, small-font labels "1/2", "2/3", "3/4") on a row, and under them four faders (band gains, dB, no unity
//! line) each with the square speaker button that mutes its band.
//!
//! The legacy dialog is a Qt layout (a vertical box with 9-pixel margins: the knob row, then a grid with margins
//! 4, 10, 4, 5 of four columns). The positions are that layout's result with the theme's font: the knob row 39 high
//! (27 of the pixmap and the label), the grid from the artwork's panel (y 54) to the layout's bottom margin, the
//! faders 120 high, the buttons 14. The speaker button shows its icon when the band plays (the legacy model is
//! "mute" = true when the band plays; the new parameter is `bandNActive`).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::spec::{Bind, PanelSpec, fader, knob, toggle};
use crate::widgets::{KnobStyle, draw_mute_button};

pub const ID: &str = "crossovereq";

static ARTWORK: Asset = asset!("crossovereq/artwork.svg");
pub static ASSETS: &[&Asset] = &[&ARTWORK];

const MUTES: [&str; 4] = ["band1Active", "band2Active", "band3Active", "band4Active"];
const GAINS: [&str; 4] = ["gain1", "gain2", "gain3", "gain4"];

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls = Vec::new();
    // the knob row: three equal cells of the 149 pixels between the margins
    let cell = (167.0 - 18.0) / 3.0;
    for (i, (param, hint, name, label)) in [
        ("crossover12", "Band 1/2 crossover", "Band 1/2 crossover", "1/2"),
        ("crossover23", "Band 2/3 crossover", "Band 2/3 crossover", "2/3"),
        ("crossover34", "Band 3/4 crossover", "Band 3/4 crossover", "3/4"),
    ]
    .into_iter()
    .enumerate()
    {
        let x = (9.0 + cell * (i as f32 + 0.5) - 13.5).round();
        controls.push(knob(param, x, 9.0, KnobStyle::Bright26, 1.0, name, hint, "Hz").label(label).log());
    }
    // the grid: four equal columns of the 141 pixels between its margins (9 + 4 on each side)
    let col = (167.0 - 18.0 - 8.0) / 4.0;
    let names = [
        ("Band 1 gain", "Mute band 1"),
        ("Band 2 gain", "Mute band 2"),
        ("Band 3 gain", "Mute band 3"),
        ("Band 4 gain", "Mute band 4"),
    ];
    for i in 0..4 {
        let cx = 13.0 + col * (i as f32 + 0.5);
        let (gain, mute) = names[i];
        controls.push(fader(GAINS[i], (cx - 11.5).round(), 64.0, 120.0, gain, gain));
        controls.push(
            toggle(
                Bind::Switch { param: MUTES[i] },
                MUTES[i],
                (cx - 7.0).round(),
                190.0,
                14.0,
                14.0,
                draw_mute_button,
                mute,
            )
            .tip(mute),
        );
    }
    PanelSpec {
        plugin: ID,
        size: (167.0, 218.0),
        background: Some(&ARTWORK),
        view_backgrounds: &[],
        fill: 0x101116,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});
