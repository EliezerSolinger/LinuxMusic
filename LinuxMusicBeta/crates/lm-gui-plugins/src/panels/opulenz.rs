//! OpulenZ (legado/plugins/OpulenZ/OpulenZ.cpp, `OpulenzInstrumentView`): the 250×250 artwork, 22×22 styled knobs
//! (the style sheet's `OpulenzInstrumentView Knob`, centered at 11), LED switches and each operator's four wave
//! buttons. The times and the multiplier's semitones are worked out from the value, as `updateKnobHints` did.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, PanelSpec, button, knob};
use crate::widgets::{KnobStyle, Styled};

pub const ID: &str = "opulenz";

static ARTWORK: Asset = asset!("opulenz/artwork.png");
static LED_ON: Asset = asset!("opulenz/led_on.png");
static LED_OFF: Asset = asset!("opulenz/led_off.png");
static W1_ON: Asset = asset!("opulenz/wave1_on.png");
static W1_OFF: Asset = asset!("opulenz/wave1_off.png");
static W2_ON: Asset = asset!("opulenz/wave2_on.png");
static W2_OFF: Asset = asset!("opulenz/wave2_off.png");
static W3_ON: Asset = asset!("opulenz/wave3_on.png");
static W3_OFF: Asset = asset!("opulenz/wave3_off.png");
static W4_ON: Asset = asset!("opulenz/wave4_on.png");
static W4_OFF: Asset = asset!("opulenz/wave4_off.png");
pub static ASSETS: &[&Asset] =
    &[&ARTWORK, &LED_ON, &LED_OFF, &W1_ON, &W1_OFF, &W2_ON, &W2_OFF, &W3_ON, &W3_OFF, &W4_ON, &W4_OFF];

fn op_knob() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 22.0,
        h: 22.0,
        color: Rgba::rgb(128, 128, 128),
        outer: Some(Rgba::rgb(255, 255, 255)),
        inner_radius: 2.0,
        outer_radius: 9.0,
        center: (11.0, 11.0),
        line_width: 2.0,
    })
}

const ATTACK_MS: [f64; 16] =
    [0.0, 0.2, 0.4, 0.9, 1.8, 3.7, 7.4, 15.0, 30.0, 60.0, 120.0, 240.0, 480.0, 950.0, 1900.0, 3800.0];
const DEC_REL_MS: [f64; 16] =
    [0.0, 1.2, 2.5, 5.0, 10.0, 20.0, 40.0, 80.0, 160.0, 320.0, 640.0, 1300.0, 2600.0, 5200.0, 10000.0, 20000.0];
const MULT_SEMITONES: [i32; 16] = [-12, 0, 12, 19, 24, 28, 31, 34, 36, 38, 40, 40, 43, 43, 47, 47];

/// `knobHintHelper`
fn time(ms: f64) -> String {
    if ms > 1000.0 {
        format!(" ({:.0} s)", ms / 1000.0)
    } else if ms > 10.0 {
        format!(" ({ms:.0} ms)")
    } else {
        format!(" ({ms:.1} ms)")
    }
}

fn attack(v: f64) -> String {
    time(ATTACK_MS[(v.max(0.0) as usize).min(15)])
}

fn dec_rel(v: f64) -> String {
    time(DEC_REL_MS[(v.max(0.0) as usize).min(15)])
}

fn semitones(v: f64) -> String {
    format!(" ({} {})", MULT_SEMITONES[(v.max(0.0) as usize).min(15)], crate::i18n::tr("semitones"))
}

/// A name built once, as an interned symbol owned by the symbol table (lm_base::intern)
fn leak(s: String) -> &'static str {
    lm_port::intern(&s)
}

fn led(param: &'static str, x: f32, y: f32, name: &'static str, tip: &'static str, op: u32) -> Control {
    button(Bind::Switch { param }, param, x, y, &LED_ON, &LED_OFF, name).tip(tip).n(op)
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls = Vec::new();
    for (op, ky, by) in [(1u32, 48.0, 87.0), (2, 138.0, 177.0)] {
        let p = |s: &str| leak(format!("op{op}{s}"));
        let k = |param, x, name, hint| knob(param, x, ky, op_knob(), 1.0, name, hint, "").n(op);
        controls.extend([
            k(p("Attack"), 6.0, "Op %1 attack", "Attack").unit_of(attack),
            k(p("Decay"), 34.0, "Op %1 decay", "Decay").unit_of(dec_rel),
            k(p("Sustain"), 62.0, "Op %1 sustain", "Sustain"),
            k(p("Release"), 90.0, "Op %1 release", "Release").unit_of(dec_rel),
            k(p("Level"), 166.0, "Op %1 level", "Level"),
            k(p("LevelScaling"), 194.0, "Op %1 level scaling", "Scale"),
            k(p("Multiplier"), 222.0, "Op %1 frequency multiplier", "Frequency multiplier").unit_of(semitones),
            led(p("KeyScalingRate"), 9.0, by, "Op %1 key scaling rate", "Keyboard scaling rate", op),
            led(p("Percussive"), 36.0, by, "Op %1 percussive envelope", "Percussive envelope", op),
            led(p("Tremolo"), 65.0, by, "Op %1 tremolo", "Tremolo", op),
            led(p("Vibrato"), 93.0, by, "Op %1 vibrato", "Vibrato", op),
        ]);
        if op == 1 {
            controls.push(knob("feedback", 128.0, ky, op_knob(), 1.0, "Op 1 feedback", "Feedback", ""));
        }
        let field = p("Waveform");
        let waves: [(&str, f32, &'static Asset, &'static Asset, &str); 4] = [
            ("sine", 154.0, &W1_ON, &W1_OFF, "Sine"),
            ("half-sine", 178.0, &W2_ON, &W2_OFF, "Half sine"),
            ("abs-sine", 199.0, &W3_ON, &W3_OFF, "Absolute sine"),
            ("quarter-sine", 220.0, &W4_ON, &W4_OFF, if op == 1 { "Quarter sine" } else { "Quarter Sine" }),
        ];
        for (key, x, on, off, tip) in waves {
            controls.push(
                button(
                    Bind::Choice { field, key },
                    leak(format!("{field}.{key}")),
                    x,
                    by - 1.0,
                    on,
                    off,
                    "Op %1 waveform",
                )
                .tip(tip)
                .n(op),
            );
        }
    }
    controls.extend([
        led("fm", 9.0, 220.0, "FM", "FM", 0),
        led("vibratoDepth", 65.0, 220.0, "Vibrato depth", "Vibrato depth", 0),
        led("tremoloDepth", 93.0, 220.0, "Tremolo depth", "Tremolo depth", 0),
        knob("volume", 222.0, 219.0, op_knob(), 0.1, "Volume", "Volume", " dB"),
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
