//! Watsyn (legado/plugins/Watsyn/Watsyn.cpp, `WatsynView`): the 250×250 artwork, four rows of five knobs (volume,
//! panning, frequency multiplier, left and right detune: A1, A2 in blue, B1, B2 in orange, `Knob#aKnob` and `#bKnob`
//! 19×19 on the style sheet's `WatsynView Knob` values), the A-B mix knob (31×31) with the mix envelope's amount,
//! attack, hold and decay and the crosstalk knob, the two modulation groups (mix, AM, RM, PM for A and for B), the
//! four buttons that select the oscillator whose 224×105 wave is shown and drawn (A1, A2, B1, B2: the panel's own
//! view, not saved in the plugin), the wave tools (load, phase -15 and +15, normalize, invert, smooth, and the sine,
//! triangle, saw and square shapes), and the anti-aliasing LED.
//!
//! The envelope's time knobs are `TempoSyncKnob`s: their menu has the tempo sync notes (`envAttackSync`...) and
//! moving the knob turns the sync off. The
//! load button asks for an audio file and sends its bytes to the core's `watsyn.load_wave` action, which sets the
//! graph of the oscillator shown.

use std::sync::LazyLock;

use lm_port::Value;

use crate::asset;
use crate::assets::Asset;
use crate::graphs;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, GraphSpec, GraphStyle, PanelSpec, PressCx, Target, button, graph, knob, led};
use crate::widgets::{KnobStyle, LedColor, Styled};

pub const ID: &str = "watsyn";

static ARTWORK: Asset = asset!("watsyn/artwork.png");
static WAVEGRAPH: Asset = asset!("watsyn/wavegraph.png");
static A1_ON: Asset = asset!("watsyn/a1_active.png");
static A1_OFF: Asset = asset!("watsyn/a1_inactive.png");
static A2_ON: Asset = asset!("watsyn/a2_active.png");
static A2_OFF: Asset = asset!("watsyn/a2_inactive.png");
static B1_ON: Asset = asset!("watsyn/b1_active.png");
static B1_OFF: Asset = asset!("watsyn/b1_inactive.png");
static B2_ON: Asset = asset!("watsyn/b2_active.png");
static B2_OFF: Asset = asset!("watsyn/b2_inactive.png");
static AMIX_ON: Asset = asset!("watsyn/amix_active.png");
static AMIX_OFF: Asset = asset!("watsyn/amix_inactive.png");
static AAM_ON: Asset = asset!("watsyn/aam_active.png");
static AAM_OFF: Asset = asset!("watsyn/aam_inactive.png");
static ARM_ON: Asset = asset!("watsyn/arm_active.png");
static ARM_OFF: Asset = asset!("watsyn/arm_inactive.png");
static APM_ON: Asset = asset!("watsyn/apm_active.png");
static APM_OFF: Asset = asset!("watsyn/apm_inactive.png");
static BMIX_ON: Asset = asset!("watsyn/bmix_active.png");
static BMIX_OFF: Asset = asset!("watsyn/bmix_inactive.png");
static BAM_ON: Asset = asset!("watsyn/bam_active.png");
static BAM_OFF: Asset = asset!("watsyn/bam_inactive.png");
static BRM_ON: Asset = asset!("watsyn/brm_active.png");
static BRM_OFF: Asset = asset!("watsyn/brm_inactive.png");
static BPM_ON: Asset = asset!("watsyn/bpm_active.png");
static BPM_OFF: Asset = asset!("watsyn/bpm_inactive.png");
static LOAD_ON: Asset = asset!("watsyn/load_active.png");
static LOAD_OFF: Asset = asset!("watsyn/load_inactive.png");
static PHL_ON: Asset = asset!("watsyn/phl_active.png");
static PHL_OFF: Asset = asset!("watsyn/phl_inactive.png");
static PHR_ON: Asset = asset!("watsyn/phr_active.png");
static PHR_OFF: Asset = asset!("watsyn/phr_inactive.png");
static NORM_ON: Asset = asset!("watsyn/norm_active.png");
static NORM_OFF: Asset = asset!("watsyn/norm_inactive.png");
static INV_ON: Asset = asset!("watsyn/inv_active.png");
static INV_OFF: Asset = asset!("watsyn/inv_inactive.png");
static SMOOTH_ON: Asset = asset!("watsyn/smooth_active.png");
static SMOOTH_OFF: Asset = asset!("watsyn/smooth_inactive.png");
static SIN_ON: Asset = asset!("watsyn/sin_active.png");
static SIN_OFF: Asset = asset!("watsyn/sin_inactive.png");
static TRI_ON: Asset = asset!("watsyn/tri_active.png");
static TRI_OFF: Asset = asset!("watsyn/tri_inactive.png");
static SAW_ON: Asset = asset!("watsyn/saw_active.png");
static SAW_OFF: Asset = asset!("watsyn/saw_inactive.png");
static SQR_ON: Asset = asset!("watsyn/sqr_active.png");
static SQR_OFF: Asset = asset!("watsyn/sqr_inactive.png");

pub static ASSETS: &[&Asset] = &[
    &ARTWORK,
    &WAVEGRAPH,
    &A1_ON,
    &A1_OFF,
    &A2_ON,
    &A2_OFF,
    &B1_ON,
    &B1_OFF,
    &B2_ON,
    &B2_OFF,
    &AMIX_ON,
    &AMIX_OFF,
    &AAM_ON,
    &AAM_OFF,
    &ARM_ON,
    &ARM_OFF,
    &APM_ON,
    &APM_OFF,
    &BMIX_ON,
    &BMIX_OFF,
    &BAM_ON,
    &BAM_OFF,
    &BRM_ON,
    &BRM_OFF,
    &BPM_ON,
    &BPM_OFF,
    &LOAD_ON,
    &LOAD_OFF,
    &PHL_ON,
    &PHL_OFF,
    &PHR_ON,
    &PHR_OFF,
    &NORM_ON,
    &NORM_OFF,
    &INV_ON,
    &INV_OFF,
    &SMOOTH_ON,
    &SMOOTH_OFF,
    &SIN_ON,
    &SIN_OFF,
    &TRI_ON,
    &TRI_OFF,
    &SAW_ON,
    &SAW_OFF,
    &SQR_ON,
    &SQR_OFF,
];

/// The core's action that loads a waveform from an audio file into a graph field
pub const LOAD_ACTION: &str = "watsyn.load_wave";

const GRAPH_LEN: usize = 220;
const GRAPHS: [&str; 4] = ["a1Graph", "a2Graph", "b1Graph", "b2Graph"];
const OSC: [&str; 4] = ["a1", "a2", "b1", "b2"];
const ROWS: [f32; 4] = [26.0, 49.0, 72.0, 95.0];

/// The graph shown (the panel's view) and its samples
fn shown(cx: &PressCx<'_>) -> (&'static str, Vec<f64>) {
    let field = GRAPHS[usize::from(cx.view) % 4];
    let mut s: Vec<f64> = (cx.field)(field).items().map(|x| x.as_real().unwrap_or(0.0)).collect();
    s.resize(GRAPH_LEN, 0.0);
    (field, s)
}

fn out(field: &'static str, s: &[f64]) -> Vec<(Target, Value)> {
    vec![(Target::Field(field), Value::list(s.iter().map(|v| Value::real(*v).unwrap_or_default())))]
}

fn shape(cx: &PressCx<'_>, f: fn(f64) -> f64) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::fill(&mut s, GRAPH_LEN, f);
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
fn normalize(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::normalize(&mut s, GRAPH_LEN, -1.0, 1.0);
    out(field, &s)
}
fn invert(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::invert(&mut s, GRAPH_LEN, -1.0, 1.0);
    out(field, &s)
}
fn smooth(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::smooth(&mut s, GRAPH_LEN);
    out(field, &s)
}
fn phase_left(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::shift_phase(&mut s, GRAPH_LEN, -15);
    out(field, &s)
}
fn phase_right(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let (field, mut s) = shown(cx);
    graphs::shift_phase(&mut s, GRAPH_LEN, 15);
    out(field, &s)
}

/// `Knob#aKnob`, `#bKnob`, `#mixenvKnob`, `#xtalkKnob` on `WatsynView Knob`: 19×19, radii 1 and 7, center 9.5, line 2
fn styled(color: u32) -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 19.0,
        h: 19.0,
        color: Rgba::hex(color),
        outer: Some(Rgba::hex(color)),
        inner_radius: 1.0,
        outer_radius: 7.0,
        center: (9.5, 9.5),
        line_width: 2.0,
    })
}

/// `Knob#mixKnob`: 31×31 (resized after the 19×19), outer radius 13, center 15.5
fn mix_style() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 31.0,
        h: 31.0,
        color: Rgba::hex(0x43ff82),
        outer: Some(Rgba::hex(0x43ff82)),
        inner_radius: 1.0,
        outer_radius: 13.0,
        center: (15.5, 15.5),
        line_width: 2.0,
    })
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls: Vec<Control> = Vec::new();
    let key = |o: &str, s: &str| -> &'static str { lm_port::intern(&format!("{o}{s}")) };
    let name = |s: &str, o: &str| -> &'static str { lm_port::intern(&format!("{s} {}", o.to_uppercase())) };
    for (i, o) in OSC.iter().enumerate() {
        let color = if i < 2 { 0x43b2ff } else { 0xfc5431 };
        let y = ROWS[i];
        controls.push(knob(key(o, "Volume"), 130.0, y, styled(color), 1.0, name("Volume", o), "Volume", "%").volume());
        controls.push(knob(key(o, "Panning"), 154.0, y, styled(color), 1.0, name("Panning", o), "Panning", ""));
        controls.push(knob(
            key(o, "Multiplier"),
            178.0,
            y,
            styled(color),
            1.0,
            name("Freq. multiplier", o),
            "Freq. multiplier",
            "/8",
        ));
        controls.push(knob(
            key(o, "LeftDetune"),
            202.0,
            y,
            styled(color),
            1.0,
            name("Left detune", o),
            "Left detune",
            " cents",
        ));
        controls.push(knob(
            key(o, "RightDetune"),
            226.0,
            y,
            styled(color),
            1.0,
            name("Right detune", o),
            "Right detune",
            " cents",
        ));
    }
    controls.push(knob("abMix", 4.0, 3.0, mix_style(), 1.0, "A-B Mix", "A-B Mix", ""));
    controls.push(knob(
        "envAmount",
        88.0,
        3.0,
        styled(0x43ff82),
        1.0,
        "A-B Mix envelope amount",
        "Mix envelope amount",
        "",
    ));
    controls.push(
        knob("envAttack", 88.0, 26.0, styled(0x43ff82), 1.0, "A-B Mix envelope attack", "Mix envelope attack", " ms")
            .synced("envAttackSync"),
    );
    controls.push(
        knob("envHold", 88.0, 49.0, styled(0x43ff82), 1.0, "A-B Mix envelope hold", "Mix envelope hold", " ms")
            .synced("envHoldSync"),
    );
    controls.push(
        knob("envDecay", 88.0, 72.0, styled(0x43ff82), 1.0, "A-B Mix envelope decay", "Mix envelope decay", " ms")
            .synced("envDecaySync"),
    );
    controls.push(knob("crosstalk", 88.0, 95.0, styled(0xfb50fb), 1.0, "A1-B2 Crosstalk", "Crosstalk", ""));

    // which oscillator's wave is shown: the panel's own view
    for (n, (id, x, on, off, tip)) in [
        ("select1", 4.0, &A1_ON, &A1_OFF, "Select oscillator A1"),
        ("select2", 44.0, &A2_ON, &A2_OFF, "Select oscillator A2"),
        ("select3", 84.0, &B1_ON, &B1_OFF, "Select oscillator B1"),
        ("select4", 124.0, &B2_ON, &B2_OFF, "Select oscillator B2"),
    ]
    .into_iter()
    .enumerate()
    {
        controls.push(button(Bind::View(n as u8), id, x, 121.0, on, off, "Selected graph").tip(tip));
    }
    // the modulation groups
    let group =
        |field: &'static str,
         x: f32,
         items: [(&'static str, &'static str, &'static Asset, &'static Asset, &'static str); 4]| {
            items
                .into_iter()
                .enumerate()
                .map(|(i, (key, id, on, off, tip))| {
                    button(Bind::Choice { field, key }, id, x, 50.0 + 16.0 * i as f32, on, off, "Modulation").tip(tip)
                })
                .collect::<Vec<_>>()
        };
    controls.extend(group(
        "aMod",
        4.0,
        [
            ("mix", "amix", &AMIX_ON, &AMIX_OFF, "Mix output of A2 to A1"),
            ("am", "aam", &AAM_ON, &AAM_OFF, "Modulate amplitude of A1 by output of A2"),
            ("rm", "arm", &ARM_ON, &ARM_OFF, "Ring modulate A1 and A2"),
            ("pm", "apm", &APM_ON, &APM_OFF, "Modulate phase of A1 by output of A2"),
        ],
    ));
    controls.extend(group(
        "bMod",
        44.0,
        [
            ("mix", "bmix", &BMIX_ON, &BMIX_OFF, "Mix output of B2 to B1"),
            ("am", "bam", &BAM_ON, &BAM_OFF, "Modulate amplitude of B1 by output of B2"),
            ("rm", "brm", &BRM_ON, &BRM_OFF, "Ring modulate B1 and B2"),
            ("pm", "bpm", &BPM_ON, &BPM_OFF, "Modulate phase of B1 by output of B2"),
        ],
    ));
    // the four waves, one shown at a time
    for (n, field) in GRAPHS.iter().enumerate() {
        let color = if n < 2 { Rgba::rgb(0x43, 0xb2, 0xff) } else { Rgba::rgb(0xfc, 0x54, 0x31) };
        controls.push(
            graph(
                GraphSpec {
                    field,
                    w: 224.0,
                    h: 105.0,
                    min: -1.0,
                    max: 1.0,
                    length: GRAPH_LEN,
                    length_param: None,
                    style: GraphStyle::Linear,
                    color,
                    background: Some(&WAVEGRAPH),
                    foreground: None,
                    step: 0.0,
                },
                4.0,
                141.0,
                "Wave",
            )
            .tip("Draw your own waveform here by dragging your mouse on this graph.")
            .in_view_n(n as u8),
        );
    }
    // the tools: the one that loads a file is a button per wave, shown with it
    let tool = |f, id, x, y, on, off, tip: &'static str| button(Bind::Press(f), id, x, y, on, off, tip).tip(tip);
    for (n, field) in GRAPHS.iter().enumerate() {
        let id = lm_port::intern(&format!("load{}", n + 1));
        controls.push(
            button(
                Bind::Open { field, action: Some(LOAD_ACTION), extensions: &[] },
                id,
                173.0,
                121.0,
                &LOAD_ON,
                &LOAD_OFF,
                "Load waveform",
            )
            .tip("Load a waveform from a sample file")
            .in_view_n(n as u8),
        );
    }
    controls.push(tool(phase_left, "phaseLeft", 193.0, 121.0, &PHL_ON, &PHL_OFF, "Shift phase by -15 degrees"));
    controls.push(tool(phase_right, "phaseRight", 210.0, 121.0, &PHR_ON, &PHR_OFF, "Shift phase by +15 degrees"));
    controls.push(tool(normalize, "normalize", 230.0, 121.0, &NORM_ON, &NORM_OFF, "Normalize"));
    controls.push(tool(invert, "invert", 230.0, 138.0, &INV_ON, &INV_OFF, "Invert"));
    controls.push(tool(smooth, "smooth", 230.0, 155.0, &SMOOTH_ON, &SMOOTH_OFF, "Smooth"));
    controls.push(tool(sine, "sine", 230.0, 176.0, &SIN_ON, &SIN_OFF, "Sine wave"));
    controls.push(tool(triangle, "triangle", 230.0, 194.0, &TRI_ON, &TRI_OFF, "Triangle wave"));
    controls.push(tool(saw, "saw", 230.0, 212.0, &SAW_ON, &SAW_OFF, "Saw wave"));
    controls.push(tool(square, "square", 230.0, 230.0, &SQR_ON, &SQR_OFF, "Square wave"));
    controls.push(
        led(
            Bind::Flag { field: "antiAliasing" },
            "antiAliasing",
            150.0,
            2.0,
            LedColor::Green,
            "Anti-aliasing",
            "Anti-aliasing",
        )
        .tip("High notes without the tones their harmonics make when they fold back below half the sample rate"),
    );
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
