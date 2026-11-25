//! Nescaline (legado/plugins/Nes/Nes.cpp, `NesInstrumentView`): the 250×250 artwork, 29×29 styled knobs (the style
//! sheet's `NesInstrumentView Knob`: a red ring from radius 8 to 11), the red LEDs (`nesled`) and the duty cycle's
//! four buttons (`nesdc1`…`nesdc4`), which in the new state are one choice field (U30).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, PanelSpec, button, knob};
use crate::widgets::{KnobStyle, Styled};

pub const ID: &str = "nes";

static ARTWORK: Asset = asset!("nes/artwork.png");
static LED_ON: Asset = asset!("nes/nesled_on.png");
static LED_OFF: Asset = asset!("nes/nesled_off.png");
static DC1: Asset = asset!("nes/nesdc1_on.png");
static DC2: Asset = asset!("nes/nesdc2_on.png");
static DC3: Asset = asset!("nes/nesdc3_on.png");
static DC4: Asset = asset!("nes/nesdc4_on.png");
static DC_OFF: Asset = asset!("nes/nesdc_off.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK, &LED_ON, &LED_OFF, &DC1, &DC2, &DC3, &DC4, &DC_OFF];

fn nes_knob() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 29.0,
        h: 29.0,
        color: Rgba::hex(0xe7231b),
        outer: Some(Rgba::hex(0xffffff)),
        inner_radius: 8.0,
        outer_radius: 11.0,
        center: (14.5, 14.5),
        line_width: 2.0,
    })
}

const Y: [f32; 4] = [24.0, 81.0, 138.0, 195.0];
const X: [f32; 7] = [12.0, 46.0, 84.0, 117.0, 151.0, 172.0, 206.0];

/// A name built once, as an interned symbol owned by the symbol table (lm_base::intern)
fn leak(s: String) -> &'static str {
    lm_port::intern(&s)
}

/// A `makenesled`: a checkable red LED on a switch
fn nesled(param: &'static str, x: f32, y: f32, name: &'static str, tip: &'static str, ch: u32) -> Control {
    button(Bind::Switch { param }, param, x, y, &LED_ON, &LED_OFF, name).tip(tip).n(ch)
}

fn k(param: &'static str, x: f32, y: f32, step: f64, name: &'static str, hint: &'static str, ch: u32) -> Control {
    knob(param, x, y, nes_knob(), step, name, hint, "").n(ch)
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls = Vec::new();
    for ch in 1..=2u32 {
        let y = Y[ch as usize - 1];
        let p = |s: &str| leak(format!("ch{ch}{s}"));
        controls.extend([
            nesled(p("Enabled"), X[0], y - 12.0, "Channel %1 enable", "Enable channel %1", ch),
            k(p("Volume"), X[0], y, 1.0, "Channel %1 volume", "Volume", ch),
            k(p("Coarse"), X[1], y, 1.0, "Channel %1 coarse detune", "Coarse detune", ch),
            nesled(p("EnvEnabled"), X[2], y - 12.0, "Channel %1 envelope enable", "Enable envelope %1", ch),
            k(p("EnvLength"), X[2], y, 1.0, "Channel %1 envelope length", "Envelope length", ch),
            nesled(p("EnvLooped"), 129.0, y - 12.0, "Channel %1 envelope loop", "Enable envelope %1 loop", ch),
            nesled(p("SweepEnabled"), X[5], y - 12.0, "Channel %1 sweep enable", "Enable sweep %1", ch),
            k(p("SweepAmount"), X[5], y, 1.0, "Channel %1 sweep amount", "Sweep amount", ch),
            k(p("SweepRate"), X[6], y, 1.0, "Channel %1 sweep rate", "Sweep rate", ch),
        ]);
        let field = p("DutyCycle");
        let cycles: [(&str, &'static Asset, &str); 4] = [
            ("12.5", &DC1, "12.5% Duty cycle"),
            ("25", &DC2, "25% Duty cycle"),
            ("50", &DC3, "50% Duty cycle"),
            ("75", &DC4, "75% Duty cycle"),
        ];
        for (i, (key, on, tip)) in cycles.into_iter().enumerate() {
            controls.push(
                button(
                    Bind::Choice { field, key },
                    leak(format!("{field}.{key}")),
                    X[3] + 13.0 * i as f32,
                    y + 18.0,
                    on,
                    &DC_OFF,
                    "Channel %1 duty cycle",
                )
                .tip(tip)
                .n(ch),
            );
        }
    }
    let y = Y[2];
    controls.extend([
        nesled("ch3Enabled", X[0], y - 12.0, "Channel %1 enable", "Enable channel %1", 3),
        k("ch3Volume", X[0], y, 1.0, "Channel %1 volume", "Volume", 3),
        k("ch3Coarse", X[1], y, 1.0, "Channel %1 coarse detune", "Coarse detune", 3),
        k("masterVolume", X[3], y, 0.01, "Master volume", "Master volume", 3),
        k("vibrato", X[4], y, 1.0, "Vibrato", "Vibrato", 3),
    ]);
    let y = Y[3];
    controls.extend([
        nesled("ch4Enabled", X[0], y - 12.0, "Channel %1 enable", "Enable channel %1", 4),
        k("ch4Volume", X[0], y, 1.0, "Channel %1 volume", "Volume", 4),
        k("ch4NoiseFreq", X[1], y, 1.0, "Channel %1 noise frequency", "Noise Frequency", 4),
        nesled("ch4EnvEnabled", X[2], y - 12.0, "Channel %1 envelope enable", "Enable envelope %1", 4),
        k("ch4EnvLength", X[2], y, 1.0, "Channel %1 envelope length", "Envelope length", 4),
        nesled("ch4EnvLooped", 129.0, y - 12.0, "Channel %1 envelope loop", "Enable envelope %1 loop", 4),
        k("ch4Sweep", X[3], y, 1.0, "Channel %1 noise frequency sweep", "Frequency sweep", 4),
        nesled(
            "ch4NoiseQuantize",
            162.0,
            y - 12.0,
            "Channel %1 quantize",
            "Quantize noise frequency when using note frequency",
            4,
        ),
        nesled("ch4NoiseFreqMode", 148.0, 203.0, "Channel %1 frequency mode", "Use note frequency for noise", 4),
        nesled("ch4NoiseMode", 148.0, 224.0, "Channel %1 noise mode", "Noise mode", 4),
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
