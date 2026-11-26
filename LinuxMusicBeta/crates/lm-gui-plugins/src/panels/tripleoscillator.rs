//! TripleOscillator (legado/plugins/TripleOscillator/TripleOscillator.cpp, `TripleOscillatorView`): the 250×250
//! artwork, the two rows of modulation buttons, and per oscillator seven styled knobs, the wavetable switch and the
//! eight wave buttons. The knobs' look is the style sheet's `TripleOscillatorView Knob` (white line, radius 2 to 7,
//! centered at 13, 14, width 2).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, ButtonSpec, Control, Kind, KnobSpec, PanelSpec, Text};
use crate::widgets::{KnobStyle, Styled};

pub const ID: &str = "tripleoscillator";

static ARTWORK: Asset = asset!("tripleoscillator/artwork.png");
static PM_ON: Asset = asset!("tripleoscillator/pm_active.png");
static PM_OFF: Asset = asset!("tripleoscillator/pm_inactive.png");
static AM_ON: Asset = asset!("tripleoscillator/am_active.png");
static AM_OFF: Asset = asset!("tripleoscillator/am_inactive.png");
static MIX_ON: Asset = asset!("tripleoscillator/mix_active.png");
static MIX_OFF: Asset = asset!("tripleoscillator/mix_inactive.png");
static SYNC_ON: Asset = asset!("tripleoscillator/sync_active.png");
static SYNC_OFF: Asset = asset!("tripleoscillator/sync_inactive.png");
static FM_ON: Asset = asset!("tripleoscillator/fm_active.png");
static FM_OFF: Asset = asset!("tripleoscillator/fm_inactive.png");
static SIN_ON: Asset = asset!("tripleoscillator/sin_shape_active.png");
static SIN_OFF: Asset = asset!("tripleoscillator/sin_shape_inactive.png");
static TRI_ON: Asset = asset!("tripleoscillator/triangle_shape_active.png");
static TRI_OFF: Asset = asset!("tripleoscillator/triangle_shape_inactive.png");
static SAW_ON: Asset = asset!("tripleoscillator/saw_shape_active.png");
static SAW_OFF: Asset = asset!("tripleoscillator/saw_shape_inactive.png");
static SQR_ON: Asset = asset!("tripleoscillator/square_shape_active.png");
static SQR_OFF: Asset = asset!("tripleoscillator/square_shape_inactive.png");
static MOOG_ON: Asset = asset!("tripleoscillator/moog_saw_shape_active.png");
static MOOG_OFF: Asset = asset!("tripleoscillator/moog_saw_shape_inactive.png");
static EXP_ON: Asset = asset!("tripleoscillator/exp_shape_active.png");
static EXP_OFF: Asset = asset!("tripleoscillator/exp_shape_inactive.png");
static NOISE_ON: Asset = asset!("tripleoscillator/white_noise_shape_active.png");
static NOISE_OFF: Asset = asset!("tripleoscillator/white_noise_shape_inactive.png");
static USR_ON: Asset = asset!("tripleoscillator/usr_shape_active.png");
static USR_OFF: Asset = asset!("tripleoscillator/usr_shape_inactive.png");
static TABLE_ON: Asset = asset!("tripleoscillator/wavetable_active.png");
static TABLE_OFF: Asset = asset!("tripleoscillator/wavetable_inactive.png");

/// Every image of the panel (the asset test decodes them all)
pub static ASSETS: &[&Asset] = &[
    &ARTWORK, &PM_ON, &PM_OFF, &AM_ON, &AM_OFF, &MIX_ON, &MIX_OFF, &SYNC_ON, &SYNC_OFF, &FM_ON, &FM_OFF, &SIN_ON,
    &SIN_OFF, &TRI_ON, &TRI_OFF, &SAW_ON, &SAW_OFF, &SQR_ON, &SQR_OFF, &MOOG_ON, &MOOG_OFF, &EXP_ON, &EXP_OFF,
    &NOISE_ON, &NOISE_OFF, &USR_ON, &USR_OFF, &TABLE_ON, &TABLE_OFF,
];

/// `lmms--gui--TripleOscillatorView lmms--gui--Knob` with `setFixedSize(28, 35)`
fn knob() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 28.0,
        h: 35.0,
        color: Rgba::rgb(255, 255, 255),
        outer: Some(Rgba::rgb(255, 255, 255)),
        inner_radius: 2.0,
        outer_radius: 7.0,
        center: (13.0, 14.0),
        line_width: 2.0,
    })
}

const MOD_X: f32 = 66.0;
const MOD1_Y: f32 = 58.0;
const MOD2_Y: f32 = 75.0;
const OSC_Y: f32 = 109.0;
const OSC_H: f32 = 52.0;

/// The ids of the controls, per oscillator: `<param>` for the knobs and switches, `wave<n>.<key>`,
/// `modulation<n>.<key>` for the groups' buttons
pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls = Vec::new();
    // modulation: oscillator 1 by 2, 2 by 3
    let mods: [(&str, &'static Asset, &'static Asset, [&str; 2]); 5] = [
        (
            "pm",
            &PM_ON,
            &PM_OFF,
            ["Modulate phase of oscillator 1 by oscillator 2", "Modulate phase of oscillator 2 by oscillator 3"],
        ),
        (
            "am",
            &AM_ON,
            &AM_OFF,
            [
                "Modulate amplitude of oscillator 1 by oscillator 2",
                "Modulate amplitude of oscillator 2 by oscillator 3",
            ],
        ),
        ("mix", &MIX_ON, &MIX_OFF, ["Mix output of oscillators 1 & 2", "Mix output of oscillators 2 & 3"]),
        (
            "sync",
            &SYNC_ON,
            &SYNC_OFF,
            ["Synchronize oscillator 1 with oscillator 2", "Synchronize oscillator 2 with oscillator 3"],
        ),
        (
            "fm",
            &FM_ON,
            &FM_OFF,
            [
                "Modulate frequency of oscillator 1 by oscillator 2",
                "Modulate frequency of oscillator 2 by oscillator 3",
            ],
        ),
    ];
    for (row, (field, y)) in [("modulation1", MOD1_Y), ("modulation2", MOD2_Y)].into_iter().enumerate() {
        for (i, (key, on, off, tips)) in mods.iter().enumerate() {
            controls.push(Control {
                id: leak(format!("{field}.{key}")),
                x: MOD_X + 35.0 * i as f32,
                y,
                kind: Kind::Button(ButtonSpec {
                    active: on,
                    inactive: off,
                    bind: Bind::Choice { field, key },
                    file_field: None,
                }),
                name: Text::n("Modulation type %1", row as u32 + 1),
                tip: Text::new(tips[row]),
                view: None,
                when: None,
                unless: None,
                only: None,
            });
        }
    }
    let waves: [(&str, &'static Asset, &'static Asset, &str, f32); 8] = [
        ("sine", &SIN_ON, &SIN_OFF, "Sine wave", 128.0),
        ("triangle", &TRI_ON, &TRI_OFF, "Triangle wave", 143.0),
        ("saw", &SAW_ON, &SAW_OFF, "Saw wave", 158.0),
        ("square", &SQR_ON, &SQR_OFF, "Square wave", 173.0),
        ("moog-saw", &MOOG_ON, &MOOG_OFF, "Moog-like saw wave", 188.0),
        ("exponential", &EXP_ON, &EXP_OFF, "Exponential wave", 203.0),
        ("white-noise", &NOISE_ON, &NOISE_OFF, "White noise", 218.0),
        ("user", &USR_ON, &USR_OFF, "User-defined wave", 233.0),
    ];
    for osc in 0..3u32 {
        let n = osc + 1;
        let knob_y = OSC_Y + osc as f32 * OSC_H;
        let knobs: [(&str, f32, &str, &str, &str, bool); 7] = [
            ("vol", 6.0, "Osc %1 volume:", "Osc %1 volume", "%", true),
            ("pan", 35.0, "Osc %1 panning:", "Osc %1 panning", "", false),
            ("coarse", 82.0, "Osc %1 coarse detuning:", "Osc %1 coarse detuning", " semitones", false),
            ("fineL", 111.0, "Osc %1 fine detuning left:", "Osc %1 fine detuning left", " cents", false),
            ("fineR", 140.0, "Osc %1 fine detuning right:", "Osc %1 fine detuning right", " cents", false),
            ("phase", 188.0, "Osc %1 phase-offset:", "Osc %1 phase-offset", " degrees", false),
            ("stereoPhase", 217.0, "Osc %1 stereo phase-detuning:", "Osc %1 stereo phase-detuning", " degrees", false),
        ];
        for (param, x, hint, name, unit, volume) in knobs {
            let param = leak(format!("{param}{n}"));
            controls.push(Control {
                id: param,
                x,
                y: knob_y,
                kind: Kind::Knob(KnobSpec {
                    style: knob(),
                    param,
                    step: 1.0,
                    center: None,
                    hint: Text::n(hint, n),
                    unit,
                    volume,
                    zero_dbfs: 100.0,
                    unit_of: None,
                    label: None,
                    choices: None,
                    int_field: false,
                    log: false,
                    sync: None,
                    wakes: None,
                }),
                name: Text::n(name, n),
                tip: Text::EMPTY,
                view: None,
                when: None,
                unless: None,
                only: None,
            });
        }
        let btn_y = 96.0 + osc as f32 * OSC_H;
        let table = leak(format!("table{n}"));
        controls.push(Control {
            id: table,
            x: 110.0,
            y: btn_y,
            kind: Kind::Button(ButtonSpec {
                active: &TABLE_ON,
                inactive: &TABLE_OFF,
                bind: Bind::Switch { param: table },
                file_field: None,
            }),
            name: Text::n("Osc %1 wavetable", n),
            tip: Text::new("Use alias-free wavetable oscillators."),
            view: None,
            when: None,
            unless: None,
            only: None,
        });
        let field = leak(format!("wave{n}"));
        let user = leak(format!("userWave{n}"));
        for (key, on, off, tip, x) in waves {
            controls.push(Control {
                id: leak(format!("{field}.{key}")),
                x,
                y: btn_y,
                kind: Kind::Button(ButtonSpec {
                    active: on,
                    inactive: off,
                    bind: Bind::Choice { field, key },
                    file_field: (key == "user").then_some(user),
                }),
                name: Text::n("Osc %1 wave shape", n),
                tip: Text::new(tip),
                view: None,
                when: None,
                unless: None,
                only: None,
            });
        }
    }
    PanelSpec {
        plugin: ID,
        size: (250.0, 250.0),
        background: Some(&ARTWORK),
        fill: 0x000000,
        controls,
        view_backgrounds: &[],
        labels: Vec::new(),
        file_drop: None,
    }
});

/// The panel's names are built once, as interned symbols owned by the symbol table (lm_base::intern)
fn leak(s: String) -> &'static str {
    lm_port::intern(&s)
}
