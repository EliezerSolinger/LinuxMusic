//! FreeBoy (legado/plugins/FreeBoy/FreeBoy.cpp, `FreeBoyInstrumentView`): the 250×250 artwork, 30×30 styled knobs
//! (`FreeBoyKnob`: a line from radius 8 to 13, outer color `#F1FF93`; no style sheet rule, so the inner color is the
//! palette's window text), the on/off, direction and width buttons, and the wave pattern graph of channel 3
//! (32 steps of 0–15, drawn in steps).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, GraphSpec, GraphStyle, PanelSpec, button, graph, knob};
use crate::widgets::{KnobStyle, Styled, theme};

pub const ID: &str = "freeboy";

static ARTWORK: Asset = asset!("freeboy/artwork.png");
static ON: Asset = asset!("freeboy/btn_on.png");
static OFF: Asset = asset!("freeboy/btn_off.png");
static UP: Asset = asset!("freeboy/btn_up.png");
static DOWN: Asset = asset!("freeboy/btn_down.png");
static B7: Asset = asset!("freeboy/btn_7.png");
static B15: Asset = asset!("freeboy/btn_15.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK, &ON, &OFF, &UP, &DOWN, &B7, &B15];

fn fb_knob() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 30.0,
        h: 30.0,
        color: theme::WINDOW_TEXT,
        outer: Some(Rgba::hex(0xf1ff93)),
        inner_radius: 8.0,
        outer_radius: 13.0,
        center: (15.0, 15.0),
        line_width: 1.0,
    })
}

/// A `FreeBoyKnob`: the hint is the name with a colon, the tooltip the name
fn k(param: &'static str, x: f32, y: f32, name: &'static str, hint: &'static str) -> Control {
    knob(param, x, y, fb_knob(), 1.0, name, hint, "").tip(hint.trim_end_matches(':'))
}

fn sw(param: &'static str, x: f32, y: f32, on: &'static Asset, off: &'static Asset, name: &'static str) -> Control {
    button(Bind::Switch { param }, param, x, y, on, off, name).tip(name)
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls = vec![
        k("so1Volume", 5.0, 58.0, "Right output level", "SO1 volume (Right):"),
        k("so2Volume", 37.0, 58.0, "Left output level", "SO2 volume (Left):"),
        k("treble", 69.0, 58.0, "Treble", "Treble:"),
        k("bass", 101.0, 58.0, "Bass", "Bass:"),
    ];
    for n in 1..=4u32 {
        let y = 51.0 + 12.0 * (n - 1) as f32;
        let so2: &'static str = lm_port::intern(&format!("ch{n}ToSo2"));
        let so1: &'static str = lm_port::intern(&format!("ch{n}ToSo1"));
        controls.push(sw(so2, 148.0, y, &ON, &OFF, "Channel %1 to SO2 (Left)").n(n));
        controls.push(sw(so1, 208.0, y, &ON, &OFF, "Channel %1 to SO1 (Right)").n(n));
    }
    controls.extend([
        k("ch1Volume", 5.0, 106.0, "Channel 1 volume", "Square channel 1 volume:"),
        k("ch1SweepStepLength", 37.0, 106.0, "Length of each step in sweep", "Length of each step in sweep:"),
        k("ch1Duty", 69.0, 106.0, "Wave pattern duty cycle", "Wave pattern duty cycle:"),
        k("sweepRateShift", 101.0, 106.0, "Sweep rate shift amount", "Sweep rate shift amount:"),
        k("sweepTime", 133.0, 106.0, "Sweep time", "Sweep time:"),
        sw("sweepDirection", 167.0, 108.0, &DOWN, &UP, "Sweep direction"),
        sw("ch1VolumeSweepDirection", 207.0, 108.0, &UP, &DOWN, "Volume sweep direction"),
        k("ch2Volume", 5.0, 155.0, "Channel 2 volume", "Square channel 2 volume:"),
        k("ch2SweepStepLength", 37.0, 155.0, "Length of each step in sweep", "Length of each step in sweep:"),
        k("ch2Duty", 69.0, 155.0, "Wave pattern duty cycle", "Wave pattern duty cycle:"),
        sw("ch2VolumeSweepDirection", 102.0, 156.0, &UP, &DOWN, "Volume sweep direction"),
        k("ch4Volume", 144.0, 155.0, "Channel 4 volume", "Noise channel volume:"),
        k("ch4SweepStepLength", 176.0, 155.0, "Length of each step in sweep", "Length of each step in sweep:"),
        sw("ch4VolumeSweepDirection", 207.0, 157.0, &UP, &DOWN, "Volume sweep direction"),
        sw("ch4ShiftRegisterWidth", 207.0, 171.0, &B7, &B15, "Shift Register width").tip("Shift register width"),
        k("ch3Volume", 5.0, 204.0, "Channel 3 volume", "Wave pattern channel volume:"),
        graph(
            GraphSpec {
                field: "wave",
                w: 208.0,
                h: 47.0,
                min: 0.0,
                max: 15.0,
                length: 32,
                length_param: None,
                style: GraphStyle::Nearest,
                color: Rgba::hex(0x4e832b),
                background: None,
                foreground: None,
                step: 1.0,
            },
            37.0,
            199.0,
            "Wave pattern graph",
        )
        .tip("Wave pattern graph"),
    ]);
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
