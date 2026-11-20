//! Sid (legado/plugins/Sid/SidInstrument.cpp, `SidInstrumentView`): the 250×250 artwork, 16×16 styled knobs
//! (`sidKnob` with the style sheet's `SidInstrumentView Knob`: radius 2 to 7, centered at 7.5), and text buttons that
//! are red when on. The legacy view rewrote the hints of the time and frequency knobs on every change (the attack's
//! milliseconds, the cutoff's hertz): here those units are worked out from the value.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, PanelSpec, button, knob};
use crate::widgets::{KnobStyle, Styled};

pub const ID: &str = "sid";

macro_rules! pair {
    ($on:ident, $off:ident, $name:literal) => {
        static $on: Asset = asset!(concat!("sid/", $name, "red.png"));
        static $off: Asset = asset!(concat!("sid/", $name, ".png"));
    };
}

static ARTWORK: Asset = asset!("sid/artwork.png");
pair!(HP_ON, HP_OFF, "hp");
pair!(BP_ON, BP_OFF, "bp");
pair!(LP_ON, LP_OFF, "lp");
pair!(OFF3_ON, OFF3_OFF, "3off");
pair!(M6581_ON, M6581_OFF, "6581");
pair!(M8580_ON, M8580_OFF, "8580");
pair!(PULSE_ON, PULSE_OFF, "pulse");
pair!(TRI_ON, TRI_OFF, "triangle");
pair!(SAW_ON, SAW_OFF, "saw");
pair!(NOISE_ON, NOISE_OFF, "noise");
pair!(SYNC_ON, SYNC_OFF, "sync");
pair!(RING_ON, RING_OFF, "ring");
pair!(FILTER_ON, FILTER_OFF, "filter");
pair!(TEST_ON, TEST_OFF, "test");

pub static ASSETS: &[&Asset] = &[
    &ARTWORK,
    &HP_ON,
    &HP_OFF,
    &BP_ON,
    &BP_OFF,
    &LP_ON,
    &LP_OFF,
    &OFF3_ON,
    &OFF3_OFF,
    &M6581_ON,
    &M6581_OFF,
    &M8580_ON,
    &M8580_OFF,
    &PULSE_ON,
    &PULSE_OFF,
    &TRI_ON,
    &TRI_OFF,
    &SAW_ON,
    &SAW_OFF,
    &NOISE_ON,
    &NOISE_OFF,
    &SYNC_ON,
    &SYNC_OFF,
    &RING_ON,
    &RING_OFF,
    &FILTER_ON,
    &FILTER_OFF,
    &TEST_ON,
    &TEST_OFF,
];

fn sid_knob() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 16.0,
        h: 16.0,
        color: Rgba::rgb(113, 95, 80),
        outer: Some(Rgba::rgb(255, 255, 255)),
        inner_radius: 2.0,
        outer_radius: 7.0,
        center: (7.5, 7.5),
        line_width: 2.0,
    })
}

/// `attackTime` (the SID's attack rates)
const ATTACK: [&str; 16] = [
    "2 ms", "8 ms", "16 ms", "24 ms", "38 ms", "56 ms", "68 ms", "80 ms", "100 ms", "250 ms", "500 ms", "800 ms",
    "1 s", "3 s", "5 s", "8 s",
];
/// `decRelTime` (the decay and release rates)
const DEC_REL: [&str; 16] = [
    "6 ms", "24 ms", "48 ms", "72 ms", "114 ms", "168 ms", "204 ms", "240 ms", "300 ms", "750 ms", "1.5 s", "2.4 s",
    "3 s", "9 s", "15 s", "24 s",
];

fn attack_time(v: f64) -> String {
    format!(" ({})", ATTACK[(v as usize).min(15)])
}

fn dec_rel_time(v: f64) -> String {
    format!(" ({})", DEC_REL[(v as usize).min(15)])
}

fn pulse_percent(v: f64) -> String {
    format!(" ({}%)", crate::widgets::qt_number(v / 40.95))
}

fn cutoff_hz(v: f64) -> String {
    format!(" ({} Hz)", (9970.0 / 2047.0 * v + 30.0) as i64)
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let k = |p, x, y, name, hint| knob(p, x, y, sid_knob(), 1.0, name, hint, "");
    let mut controls: Vec<Control> = vec![
        k("volume", 7.0, 64.0, "Volume", "Volume:"),
        k("filterResonance", 35.0, 64.0, "Resonance", "Resonance:"),
        k("filterCutoff", 63.0, 64.0, "Cutoff frequency", "Cutoff frequency:").unit_of(cutoff_hz),
        button(
            Bind::Choice { field: "filterMode", key: "highpass" },
            "filterMode.highpass",
            140.0,
            77.0,
            &HP_ON,
            &HP_OFF,
            "Filter type",
        )
        .tip("High-pass filter"),
        button(
            Bind::Choice { field: "filterMode", key: "bandpass" },
            "filterMode.bandpass",
            164.0,
            77.0,
            &BP_ON,
            &BP_OFF,
            "Filter type",
        )
        .tip("Band-pass filter"),
        button(
            Bind::Choice { field: "filterMode", key: "lowpass" },
            "filterMode.lowpass",
            185.0,
            77.0,
            &LP_ON,
            &LP_OFF,
            "Filter type",
        )
        .tip("Low-pass filter"),
        button(Bind::Switch { param: "voice3Off" }, "voice3Off", 207.0, 77.0, &OFF3_ON, &OFF3_OFF, "Voice 3 off")
            .tip("Voice 3 off"),
        button(
            Bind::Choice { field: "chipModel", key: "mos6581" },
            "chipModel.mos6581",
            170.0,
            59.0,
            &M6581_ON,
            &M6581_OFF,
            "Chip model",
        )
        .tip("MOS6581 SID"),
        button(
            Bind::Choice { field: "chipModel", key: "mos8580" },
            "chipModel.mos8580",
            207.0,
            59.0,
            &M8580_ON,
            &M8580_OFF,
            "Chip model",
        )
        .tip("MOS8580 SID"),
    ];
    for i in 0..3u32 {
        let n = i + 1;
        let dy = 50.0 * i as f32;
        let p = |s: &str| leak(format!("{s}{n}"));
        controls.extend([
            k(p("attack"), 7.0, 114.0 + dy, "Voice %1 attack", "Attack:").unit_of(attack_time).n(n),
            k(p("decay"), 35.0, 114.0 + dy, "Voice %1 decay", "Decay:").unit_of(dec_rel_time).n(n),
            k(p("sustain"), 63.0, 114.0 + dy, "Voice %1 sustain", "Sustain:").n(n),
            k(p("release"), 91.0, 114.0 + dy, "Voice %1 release", "Release:").unit_of(dec_rel_time).n(n),
            k(p("pulseWidth"), 119.0, 114.0 + dy, "Voice %1 pulse width", "Pulse width:").unit_of(pulse_percent).n(n),
            knob(p("coarse"), 147.0, 114.0 + dy, sid_knob(), 1.0, "Voice %1 coarse detuning", "Coarse:", " semitones")
                .n(n),
        ]);
        let wave = p("wave");
        let waves: [(&str, f32, &'static Asset, &'static Asset, &str); 4] = [
            ("square", 187.0, &PULSE_ON, &PULSE_OFF, "Pulse wave"),
            ("triangle", 168.0, &TRI_ON, &TRI_OFF, "Triangle wave"),
            ("saw", 207.0, &SAW_ON, &SAW_OFF, "Saw wave"),
            ("noise", 226.0, &NOISE_ON, &NOISE_OFF, "Noise"),
        ];
        for (key, x, on, off, tip) in waves {
            controls.push(
                button(
                    Bind::Choice { field: wave, key },
                    leak(format!("{wave}.{key}")),
                    x,
                    101.0 + dy,
                    on,
                    off,
                    "Voice %1 wave shape",
                )
                .tip(tip)
                .n(n),
            );
        }
        let switches: [(&str, f32, f32, &'static Asset, &'static Asset, &str, &str); 4] = [
            ("sync", 207.0, 134.0, &SYNC_ON, &SYNC_OFF, "Voice %1 sync", "Sync"),
            ("ringMod", 170.0, 116.0, &RING_ON, &RING_OFF, "Voice %1 ring modulate", "Ring modulation"),
            ("filtered", 207.0, 116.0, &FILTER_ON, &FILTER_OFF, "Voice %1 filtered", "Filtered"),
            ("test", 170.0, 134.0, &TEST_ON, &TEST_OFF, "Voice %1 test", "Test"),
        ];
        for (param, x, y, on, off, name, tip) in switches {
            let param = p(param);
            controls.push(button(Bind::Switch { param }, param, x, y + dy, on, off, name).tip(tip).n(n));
        }
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
