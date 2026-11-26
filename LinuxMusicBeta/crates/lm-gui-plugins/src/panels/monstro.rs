//! Monstro (legado/plugins/Monstro/Monstro.cpp, `MonstroView`): two views of the same 250×250 window, switched by the
//! tab buttons at the top (operators at 0, 0; matrix at 125, 0: the panel's own view, not saved in the plugin).
//!
//! The operators view (`artwork_op.png`): three oscillator rows of `Knob`s on the style sheet's `MonstroView Knob`
//! (Styled 20×20, white with a grey ring), the two LFOs' wave combos and knobs, the two envelopes' seven knobs each,
//! the tiny sync LEDs, the waves' combo boxes (42 wide, with the wave's picture) and the osc 2 - osc 3 mix selector
//! (mix, AM, FM, PM). The matrix view (`artwork_mat.png`): the modulation amounts, a knob for each of the four
//! sources (envelopes 1 and 2, LFOs 1 and 2) on each target (volume, phase, pitch of the three oscillators, the
//! pulse width and the sub mix).
//!
//! The LFOs' and envelopes' time knobs are `TempoSyncKnob`s in the legacy; their sync menu is not in the panel yet.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, ComboItem, Control, PanelSpec, Text, button, combo, knob};
use crate::widgets::{KnobStyle, Styled};

pub const ID: &str = "monstro";

static ART_OP: Asset = asset!("monstro/artwork_op.png");
static ART_MAT: Asset = asset!("monstro/artwork_mat.png");
static OPVIEW_ON: Asset = asset!("monstro/opview_active.png");
static OPVIEW_OFF: Asset = asset!("monstro/opview_inactive.png");
static MATVIEW_ON: Asset = asset!("monstro/matview_active.png");
static MATVIEW_OFF: Asset = asset!("monstro/matview_inactive.png");
static MIX_ON: Asset = asset!("monstro/mix_active.png");
static MIX_OFF: Asset = asset!("monstro/mix_inactive.png");
static AM_ON: Asset = asset!("monstro/am_active.png");
static AM_OFF: Asset = asset!("monstro/am_inactive.png");
static FM_ON: Asset = asset!("monstro/fm_active.png");
static FM_OFF: Asset = asset!("monstro/fm_inactive.png");
static PM_ON: Asset = asset!("monstro/pm_active.png");
static PM_OFF: Asset = asset!("monstro/pm_inactive.png");
static LED_ON: Asset = asset!("monstro/tinyled_on.png");
static LED_OFF: Asset = asset!("monstro/tinyled_off.png");
static W_SIN: Asset = asset!("monstro/sin.png");
static W_TRI: Asset = asset!("monstro/tri.png");
static W_SAW: Asset = asset!("monstro/saw.png");
static W_RAMP: Asset = asset!("monstro/ramp.png");
static W_SQR: Asset = asset!("monstro/sqr.png");
static W_MOOG: Asset = asset!("monstro/moog.png");
static W_SQRSOFT: Asset = asset!("monstro/sqrsoft.png");
static W_SINABS: Asset = asset!("monstro/sinabs.png");
static W_EXP: Asset = asset!("monstro/exp.png");
static W_NOISE: Asset = asset!("monstro/noise.png");
static W_RAND: Asset = asset!("monstro/rand.png");

pub static ASSETS: &[&Asset] = &[
    &ART_OP,
    &ART_MAT,
    &OPVIEW_ON,
    &OPVIEW_OFF,
    &MATVIEW_ON,
    &MATVIEW_OFF,
    &MIX_ON,
    &MIX_OFF,
    &AM_ON,
    &AM_OFF,
    &FM_ON,
    &FM_OFF,
    &PM_ON,
    &PM_OFF,
    &LED_ON,
    &LED_OFF,
    &W_SIN,
    &W_TRI,
    &W_SAW,
    &W_RAMP,
    &W_SQR,
    &W_MOOG,
    &W_SQRSOFT,
    &W_SINABS,
    &W_EXP,
    &W_NOISE,
    &W_RAND,
];

static VIEW_BACKGROUNDS: [&Asset; 2] = [&ART_OP, &ART_MAT];

const O1ROW: f32 = 22.0;
const O2ROW: f32 = 22.0 + 39.0;
const O3ROW: f32 = 22.0 + 39.0 * 2.0;
const LFOROW: f32 = 22.0 + 39.0 * 3.0;
const E1ROW: f32 = 22.0 + 39.0 * 4.0;
const E2ROW: f32 = 22.0 + 39.0 * 5.0;

const fn col(i: f32) -> f32 {
    16.0 + 30.0 * i
}

const MATCOL: [f32; 8] = [32.0, 57.0, 82.0, 107.0, 149.0, 174.0, 199.0, 224.0];

/// `MonstroView Knob`: 20×20, white with a grey ring, radii 6 and 9, center (10, 10), line 2.5
fn style() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 20.0,
        h: 20.0,
        color: Rgba::hex(0xffffff),
        outer: Some(Rgba::hex(0xaaaaaa)),
        inner_radius: 6.0,
        outer_radius: 9.0,
        center: (10.0, 10.0),
        line_width: 2.5,
    })
}

/// The oscillators' waves in the legacy menu's order: key, text, picture
fn osc_waves() -> Vec<ComboItem> {
    let w = |key, text, icon: &'static Asset| ComboItem { key, text: Text::new(text), icon: Some(icon) };
    vec![
        w("sine", "Sine wave", &W_SIN),
        w("triangle", "Bandlimited Triangle wave", &W_TRI),
        w("saw", "Bandlimited Saw wave", &W_SAW),
        w("ramp", "Bandlimited Ramp wave", &W_RAMP),
        w("square", "Bandlimited Square wave", &W_SQR),
        w("moogSaw", "Bandlimited Moog saw wave", &W_MOOG),
        w("softSquare", "Soft square wave", &W_SQRSOFT),
        w("absSine", "Absolute sine wave", &W_SINABS),
        w("exponential", "Exponential wave", &W_EXP),
        w("noise", "White noise", &W_NOISE),
        w("digitalTriangle", "Digital Triangle wave", &W_TRI),
        w("digitalSaw", "Digital Saw wave", &W_SAW),
        w("digitalRamp", "Digital Ramp wave", &W_RAMP),
        w("digitalSquare", "Digital Square wave", &W_SQR),
        w("digitalMoogSaw", "Digital Moog saw wave", &W_MOOG),
    ]
}

/// The LFOs' waves
fn lfo_waves() -> Vec<ComboItem> {
    let w = |key, text, icon: &'static Asset| ComboItem { key, text: Text::new(text), icon: Some(icon) };
    vec![
        w("sine", "Sine wave", &W_SIN),
        w("triangle", "Triangle wave", &W_TRI),
        w("saw", "Saw wave", &W_SAW),
        w("ramp", "Ramp wave", &W_RAMP),
        w("square", "Square wave", &W_SQR),
        w("moogSaw", "Moog saw wave", &W_MOOG),
        w("softSquare", "Soft square wave", &W_SQRSOFT),
        w("absSine", "Abs. sine wave", &W_SINABS),
        w("exponential", "Exponential wave", &W_EXP),
        w("random", "Random", &W_RAND),
        w("randomSmooth", "Random smooth", &W_RAND),
    ]
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut c: Vec<Control> = Vec::new();
    let k = |p: &str, x: f32, y: f32, step: f64, name: &'static str, hint: &'static str, unit: &'static str| {
        knob(lm_port::intern(p), x, y, style(), step, name, hint, unit).in_view_n(0)
    };
    let led = |param: &str, x: f32, y: f32, name: &'static str| {
        button(Bind::Switch { param: lm_port::intern(param) }, lm_port::intern(param), x, y, &LED_ON, &LED_OFF, name)
            .tip(name)
            .in_view_n(0)
    };
    // the tabs, always shown
    c.push(
        button(Bind::View(0), "viewOperators", 0.0, 0.0, &OPVIEW_ON, &OPVIEW_OFF, "Operators view")
            .tip("Operators view"),
    );
    c.push(
        button(Bind::View(1), "viewMatrix", 125.0, 0.0, &MATVIEW_ON, &MATVIEW_OFF, "Matrix view").tip("Matrix view"),
    );

    // oscillator 1
    for (i, (suffix, name, hint, unit, step)) in [
        ("Volume", "Volume", "Volume", "%", 1.0),
        ("Panning", "Panning", "Panning", "", 1.0),
        ("Coarse", "Coarse detune", "Coarse detune", " semitones", 1.0),
        ("FineLeft", "Fine tune left", "Fine tune left", " cents", 1.0),
        ("FineRight", "Fine tune right", "Fine tune right", " cents", 1.0),
        ("StereoPhase", "Stereo phase offset", "Stereo phase offset", " deg", 1.0),
        ("PulseWidth", "Pulse width", "Pulse width", "%", 0.01),
    ]
    .into_iter()
    .enumerate()
    {
        let mut ctl = k(&format!("osc1{suffix}"), col(i as f32), O1ROW, step, name, hint, unit);
        if suffix == "Volume" {
            ctl = ctl.volume();
        }
        c.push(ctl);
    }
    c.push(led("osc1SyncRise", 230.0, 34.0, "Send sync on pulse rise"));
    c.push(led("osc1SyncFall", 230.0, 44.0, "Send sync on pulse fall"));
    // oscillator 2
    for (i, (suffix, name, hint, unit)) in [
        ("Volume", "Volume", "Volume", "%"),
        ("Panning", "Panning", "Panning", ""),
        ("Coarse", "Coarse detune", "Coarse detune", " semitones"),
        ("FineLeft", "Fine tune left", "Fine tune left", " cents"),
        ("FineRight", "Fine tune right", "Fine tune right", " cents"),
        ("StereoPhase", "Stereo phase offset", "Stereo phase offset", " deg"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut ctl = k(&format!("osc2{suffix}"), col(i as f32), O2ROW, 1.0, name, hint, unit);
        if suffix == "Volume" {
            ctl = ctl.volume();
        }
        c.push(ctl);
    }
    c.push(combo("osc2Wave", 204.0, O2ROW + 7.0, 42.0, "Osc 2 waveform", osc_waves()).in_view_n(0));
    c.push(led("osc2SyncHard", 212.0, O2ROW - 3.0, "Hard sync oscillator 2"));
    c.push(led("osc2SyncReverse", 191.0, O2ROW - 3.0, "Reverse sync oscillator 2"));
    // oscillator 3
    for (i, (suffix, name, hint, unit)) in [
        ("Volume", "Volume", "Volume", "%"),
        ("Panning", "Panning", "Panning", ""),
        ("Coarse", "Coarse detune", "Coarse detune", " semitones"),
        ("StereoPhase", "Stereo phase offset", "Stereo phase offset", " deg"),
        ("Sub", "Sub-osc mix", "Sub-osc mix", ""),
    ]
    .into_iter()
    .enumerate()
    {
        let mut ctl = k(&format!("osc3{suffix}"), col(i as f32), O3ROW, 1.0, name, hint, unit);
        if suffix == "Volume" {
            ctl = ctl.volume();
        }
        c.push(ctl);
    }
    c.push(combo("osc3Wave1", 160.0, O3ROW + 7.0, 42.0, "Osc 3 waveform 1", osc_waves()).in_view_n(0));
    c.push(combo("osc3Wave2", 204.0, O3ROW + 7.0, 42.0, "Osc 3 waveform 2", osc_waves()).in_view_n(0));
    c.push(led("osc3SyncHard", 212.0, O3ROW - 3.0, "Hard sync oscillator 3"));
    c.push(led("osc3SyncReverse", 191.0, O3ROW - 3.0, "Reverse sync oscillator 3"));
    // the LFOs
    for (n, (wave_x, cols)) in
        [(1, (2.0, [col(1.0), col(1.0) + 26.0, col(1.0) + 52.0])), (2, (127.0, [171.0, 197.0, 223.0]))].into_iter()
    {
        let wave = lm_port::intern(&format!("lfo{n}Wave"));
        c.push(
            combo(
                wave,
                wave_x,
                LFOROW + 7.0,
                42.0,
                if n == 1 { "LFO 1 waveform" } else { "LFO 2 waveform" },
                lfo_waves(),
            )
            .in_view_n(0),
        );
        c.push(k(&format!("lfo{n}Attack"), cols[0], LFOROW, 1.0, "LFO attack", "Attack", " ms"));
        c.push(k(&format!("lfo{n}Rate"), cols[1], LFOROW, 0.01, "LFO rate", "Rate", " ms"));
        c.push(k(&format!("lfo{n}Phase"), cols[2], LFOROW, 1.0, "LFO phase", "Phase", " deg"));
    }
    // the envelopes
    for (n, row) in [(1, E1ROW), (2, E2ROW)] {
        for (i, (suffix, name, hint, unit, step)) in [
            ("PreDelay", "Pre-delay", "Pre-delay", " ms", 1.0),
            ("Attack", "Attack", "Attack", " ms", 1.0),
            ("Hold", "Hold", "Hold", " ms", 1.0),
            ("Decay", "Decay", "Decay", " ms", 1.0),
            ("Sustain", "Sustain", "Sustain", "", 0.001),
            ("Release", "Release", "Release", " ms", 1.0),
            ("Slope", "Slope", "Slope", "", 0.001),
        ]
        .into_iter()
        .enumerate()
        {
            c.push(k(&format!("env{n}{suffix}"), col(i as f32), row, step, name, hint, unit));
        }
    }
    // the osc 2 - osc 3 modulation
    for (i, (key, on, off, tip)) in [
        ("mix", &MIX_ON, &MIX_OFF, "Mix osc 2 with osc 3"),
        ("am", &AM_ON, &AM_OFF, "Modulate amplitude of osc 3 by osc 2"),
        ("fm", &FM_ON, &FM_OFF, "Modulate frequency of osc 3 by osc 2"),
        ("pm", &PM_ON, &PM_OFF, "Modulate phase of osc 3 by osc 2"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = lm_port::intern(&format!("o23{key}"));
        c.push(
            button(
                Bind::Choice { field: "o23Mod", key },
                id,
                225.0,
                185.0 + 15.0 * i as f32,
                on,
                off,
                "Osc 2 - osc 3 modulation",
            )
            .tip(tip)
            .in_view_n(0),
        );
    }

    // the matrix: rows of targets, columns of sources (envelope 1, 2, LFO 1, 2) in two groups of four
    let sources = ["Env1", "Env2", "Lfo1", "Lfo2"];
    let rows = |y: f32, left: Option<&'static str>, right: Option<&'static str>, c: &mut Vec<Control>| {
        for (group, target) in [(0usize, left), (1, right)] {
            let Some(target) = target else { continue };
            for (s, src) in sources.iter().enumerate() {
                let param = lm_port::intern(&format!("{target}{src}"));
                c.push(
                    knob(param, MATCOL[group * 4 + s], y, style(), 0.001, "Modulation amount", "Modulation amount", "")
                        .in_view_n(1),
                );
            }
        }
    };
    let r = |i: f32| 22.0 + 39.0 * i;
    rows(r(0.0), Some("osc1Vol"), Some("osc1Pit"), &mut c);
    rows(r(1.0), Some("osc1Phs"), Some("osc1Pw"), &mut c);
    rows(r(2.0), Some("osc2Vol"), Some("osc2Pit"), &mut c);
    rows(r(3.0), Some("osc2Phs"), None, &mut c);
    rows(r(4.0), Some("osc3Vol"), Some("osc3Pit"), &mut c);
    rows(r(5.0), Some("osc3Phs"), Some("osc3Sub"), &mut c);

    PanelSpec {
        plugin: ID,
        size: (250.0, 250.0),
        background: Some(&ART_OP),
        view_backgrounds: &VIEW_BACKGROUNDS,
        fill: 0x000000,
        controls: c,
        labels: Vec::new(),
        file_drop: None,
    }
});
