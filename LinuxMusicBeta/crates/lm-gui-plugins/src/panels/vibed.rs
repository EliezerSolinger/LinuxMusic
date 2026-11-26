//! Vibed (legado/plugins/Vibed/Vibed.cpp, `VibedView`; `NineButtonSelector.cpp`): the 250×250 artwork with the nine
//! string buttons (a 3×3 selector at 21, 39: which string the controls show, the panel's own view, not saved in the
//! plugin), the string's volume, stiffness, pick, pickup, panning, detune and fuzziness knobs, its length knob,
//! the 132×104 impulse graph over `wavegraph4.png` (orange, as the legacy `Graph`'s default), the wave buttons
//! beside it (sine, triangle, saw, square, noise, user wave) and under it (smooth, normalize), the impulse and
//! power LEDs, and the 3×3 harmonic selector (1/4, 1/2, fundamental, 2 to 7) at 21, 127.
//!
//! Each string has its own set of controls, shown with its button. The user-wave button asks for an audio file and
//! sends its bytes to the core's `plugin.load_into_field` action, which sets the shown string's impulse.

use std::sync::LazyLock;

use lm_port::Value;

use crate::asset;
use crate::assets::{Asset, lmms};
use crate::graphs;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, GraphSpec, GraphStyle, PanelSpec, PressCx, Target, button, graph, knob, led};
use crate::widgets::{KnobStyle, LedColor};

pub const ID: &str = "vibed";

static ARTWORK: Asset = asset!("vibed/artwork.png");
static WAVEGRAPH4: Asset = asset!("vibed/wavegraph4.png");
static NORMALIZE_ON: Asset = asset!("vibed/normalize_active.png");
static NORMALIZE_OFF: Asset = asset!("vibed/normalize_inactive.png");
static SMOOTH_ON: Asset = asset!("vibed/smooth_active.png");
static SMOOTH_OFF: Asset = asset!("vibed/smooth_inactive.png");
static USR_ON: Asset = asset!("bitinvader/usr_wave_active.png");
static USR_OFF: Asset = asset!("bitinvader/usr_wave_inactive.png");
static BM2_ON: Asset = asset!("vibed/button_-2_on.png");
static BM2_OFF: Asset = asset!("vibed/button_-2_off.png");
static BM1_ON: Asset = asset!("vibed/button_-1_on.png");
static BM1_OFF: Asset = asset!("vibed/button_-1_off.png");
static BF_ON: Asset = asset!("vibed/button_f_on.png");
static BF_OFF: Asset = asset!("vibed/button_f_off.png");
static B1_ON: Asset = asset!("vibed/button_1_on.png");
static B1_OFF: Asset = asset!("vibed/button_1_off.png");
static B2_ON: Asset = asset!("vibed/button_2_on.png");
static B2_OFF: Asset = asset!("vibed/button_2_off.png");
static B3_ON: Asset = asset!("vibed/button_3_on.png");
static B3_OFF: Asset = asset!("vibed/button_3_off.png");
static B4_ON: Asset = asset!("vibed/button_4_on.png");
static B4_OFF: Asset = asset!("vibed/button_4_off.png");
static B5_ON: Asset = asset!("vibed/button_5_on.png");
static B5_OFF: Asset = asset!("vibed/button_5_off.png");
static B6_ON: Asset = asset!("vibed/button_6_on.png");
static B6_OFF: Asset = asset!("vibed/button_6_off.png");
static B7_ON: Asset = asset!("vibed/button_7_on.png");
static B7_OFF: Asset = asset!("vibed/button_7_off.png");
static B8_ON: Asset = asset!("vibed/button_8_on.png");
static B8_OFF: Asset = asset!("vibed/button_8_off.png");
static B9_ON: Asset = asset!("vibed/button_9_on.png");
static B9_OFF: Asset = asset!("vibed/button_9_off.png");

pub static ASSETS: &[&Asset] = &[
    &ARTWORK,
    &WAVEGRAPH4,
    &NORMALIZE_ON,
    &NORMALIZE_OFF,
    &SMOOTH_ON,
    &SMOOTH_OFF,
    &USR_ON,
    &USR_OFF,
    &BM2_ON,
    &BM2_OFF,
    &BM1_ON,
    &BM1_OFF,
    &BF_ON,
    &BF_OFF,
    &B1_ON,
    &B1_OFF,
    &B2_ON,
    &B2_OFF,
    &B3_ON,
    &B3_OFF,
    &B4_ON,
    &B4_OFF,
    &B5_ON,
    &B5_OFF,
    &B6_ON,
    &B6_OFF,
    &B7_ON,
    &B7_OFF,
    &B8_ON,
    &B8_OFF,
    &B9_ON,
    &B9_OFF,
];

/// The core's action that loads a string's impulse from an audio file
pub const LOAD_ACTION: &str = "plugin.load_into_field";

const POINTS: usize = 128;
const SHAPES: [&str; 9] = ["shape1", "shape2", "shape3", "shape4", "shape5", "shape6", "shape7", "shape8", "shape9"];

/// The impulse of the string shown (the panel's view) and its samples
fn shown(cx: &PressCx<'_>) -> (&'static str, Vec<f64>) {
    let field = SHAPES[usize::from(cx.view) % 9];
    let mut s: Vec<f64> = (cx.field)(field).items().map(|x| x.as_real().unwrap_or(0.0)).collect();
    s.resize(POINTS, 0.0);
    (field, s)
}

fn out(field: &'static str, s: &[f64]) -> Vec<(Target, Value)> {
    vec![(Target::Field(field), Value::list(s.iter().map(|v| Value::real(*v).unwrap_or_default())))]
}

fn shape(cx: &PressCx<'_>, f: fn(f64) -> f64) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::fill(&mut s, POINTS, f);
    out(field, &s)
}

fn sine(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    shape(cx, graphs::sine)
}
fn triangle(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    shape(cx, graphs::triangle)
}
fn saw(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    shape(cx, graphs::saw)
}
fn square(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    shape(cx, graphs::square)
}
fn noise(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::noise(&mut s, POINTS, &mut *cx.rand);
    out(field, &s)
}
fn smooth(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::smooth(&mut s, POINTS);
    out(field, &s)
}
fn normalize(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::normalize(&mut s, POINTS, -1.0, 1.0);
    out(field, &s)
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls: Vec<Control> = Vec::new();
    let k = |name: &str, n: usize| -> &'static str { lm_port::intern(&format!("{name}{}", n + 1)) };
    // the nine strings' buttons (a `NineButtonSelector` of its own: not part of the model)
    let strings: [(&Asset, &Asset); 9] = [
        (&B1_ON, &B1_OFF),
        (&B2_ON, &B2_OFF),
        (&B3_ON, &B3_OFF),
        (&B4_ON, &B4_OFF),
        (&B5_ON, &B5_OFF),
        (&B6_ON, &B6_OFF),
        (&B7_ON, &B7_OFF),
        (&B8_ON, &B8_OFF),
        (&B9_ON, &B9_OFF),
    ];
    for (i, (on, off)) in strings.into_iter().enumerate() {
        let id = lm_port::intern(&format!("selectString{}", i + 1));
        let (x, y) = (21.0 + 1.0 + (i % 3) as f32 * 17.0, 39.0 + 1.0 + (i / 3) as f32 * 17.0);
        controls.push(button(Bind::View(i as u8), id, x, y, on, off, "String").tip("String"));
    }
    let harmonics: [(&'static str, &Asset, &Asset); 9] = [
        ("1/4", &BM2_ON, &BM2_OFF),
        ("1/2", &BM1_ON, &BM1_OFF),
        ("1", &BF_ON, &BF_OFF),
        ("2", &B2_ON, &B2_OFF),
        ("3", &B3_ON, &B3_OFF),
        ("4", &B4_ON, &B4_OFF),
        ("5", &B5_ON, &B5_OFF),
        ("6", &B6_ON, &B6_OFF),
        ("7", &B7_ON, &B7_OFF),
    ];
    for n in 0..9usize {
        let v = n as u8;
        let b26 = |name: &str, x, y, step, nm: &'static str, hint, unit| {
            knob(k(name, n), x, y, KnobStyle::Bright26, step, nm, hint, unit).in_view_n(v)
        };
        controls.push(b26("volume", 103.0, 142.0, 1.0, "String volume", "String volume:", "%"));
        controls.push(b26("stiffness", 129.0, 142.0, 0.0001, "String stiffness", "String stiffness:", ""));
        controls.push(b26("pick", 153.0, 142.0, 0.0001, "Pick position", "Pick position:", ""));
        controls.push(b26("pickup", 177.0, 142.0, 0.0001, "Pickup position", "Pickup position:", ""));
        controls.push(b26("panning", 105.0, 187.0, 0.01, "String panning", "String panning:", ""));
        controls.push(b26("detune", 150.0, 187.0, 0.001, "String detune", "String detune:", ""));
        controls.push(b26("fuzziness", 194.0, 187.0, 0.001, "String fuzziness", "String fuzziness:", ""));
        controls.push(
            knob(k("length", n), 23.0, 193.0, KnobStyle::Bright26, 1.0, "String length", "String length:", "")
                .int_field()
                .in_view_n(v),
        );
        controls.push(
            graph(
                GraphSpec {
                    field: SHAPES[n],
                    w: 132.0,
                    h: 104.0,
                    min: -1.0,
                    max: 1.0,
                    length: POINTS,
                    length_param: None,
                    style: GraphStyle::Linear,
                    color: Rgba::rgb(0xff, 0xaa, 0x00),
                    background: None,
                    foreground: Some(&WAVEGRAPH4),
                    step: 0.0,
                },
                76.0,
                21.0,
                "Impulse Editor",
            )
            .in_view_n(v),
        );
        controls.push(
            led(Bind::Switch { param: k("impulse", n) }, k("impulse", n), 23.0, 94.0, LedColor::Yellow, "", "Impulse")
                .tip("Impulse")
                .in_view_n(v),
        );
        controls.push(
            led(
                Bind::Switch { param: k("string", n) },
                k("string", n),
                212.0,
                130.0,
                LedColor::Yellow,
                "",
                "Enable waveform",
            )
            .tip("Enable/disable string")
            .in_view_n(v),
        );
        for (i, (key, on, off)) in harmonics.iter().enumerate() {
            let id = lm_port::intern(&format!("harmonic{}_{}", n + 1, i));
            let (x, y) = (21.0 + 1.0 + (i % 3) as f32 * 17.0, 127.0 + 1.0 + (i / 3) as f32 * 17.0);
            controls.push(
                button(Bind::Choice { field: k("harmonic", n), key }, id, x, y, on, off, "Octave")
                    .tip("Octave")
                    .in_view_n(v),
            );
        }
        let id = lm_port::intern(&format!("userWave{}", n + 1));
        controls.push(
            button(
                Bind::Open { field: SHAPES[n], action: Some(LOAD_ACTION), extensions: &[] },
                id,
                212.0,
                109.0,
                &USR_ON,
                &USR_OFF,
                "User-defined wave",
            )
            .tip("User-defined wave")
            .in_view_n(v),
        );
    }
    let tool = |f, id, x, y, on, off, tip: &'static str| button(Bind::Press(f), id, x, y, on, off, tip).tip(tip);
    controls.push(tool(smooth, "smooth", 79.0, 129.0, &SMOOTH_ON, &SMOOTH_OFF, "Smooth waveform"));
    controls.push(tool(normalize, "normalize", 96.0, 129.0, &NORMALIZE_ON, &NORMALIZE_OFF, "Normalize waveform"));
    controls.push(tool(sine, "sine", 212.0, 24.0, &lmms::SIN_WAVE_ACTIVE, &lmms::SIN_WAVE_INACTIVE, "Sine wave"));
    controls.push(tool(
        triangle,
        "triangle",
        212.0,
        41.0,
        &lmms::TRIANGLE_WAVE_ACTIVE,
        &lmms::TRIANGLE_WAVE_INACTIVE,
        "Triangle wave",
    ));
    controls.push(tool(saw, "saw", 212.0, 58.0, &lmms::SAW_WAVE_ACTIVE, &lmms::SAW_WAVE_INACTIVE, "Saw wave"));
    controls.push(tool(
        square,
        "square",
        212.0,
        75.0,
        &lmms::SQUARE_WAVE_ACTIVE,
        &lmms::SQUARE_WAVE_INACTIVE,
        "Square wave",
    ));
    controls.push(tool(
        noise,
        "noise",
        212.0,
        92.0,
        &lmms::WHITE_NOISE_WAVE_ACTIVE,
        &lmms::WHITE_NOISE_WAVE_INACTIVE,
        "White noise",
    ));
    PanelSpec {
        plugin: ID,
        size: (250.0, 250.0),
        background: Some(&ARTWORK),
        view_backgrounds: &[],
        fill: 0x000000,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});
