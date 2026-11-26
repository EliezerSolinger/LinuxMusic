//! BitInvader (legado/plugins/BitInvader/BitInvader.cpp, `BitInvaderView`): the 250×250 artwork, the sample length
//! knob, the 204×134 graph over `wavegraph.png` (as many samples as the length), the preset wave buttons that fill
//! the graph, smooth, and the three LEDs.
//!
//! Not ported yet: the user-defined wave button (it loads a sample file into the graph; the new graph's field is a
//! list of reals, not a resource, and no panel action reads a file into a list yet).

use std::sync::LazyLock;

use lm_port::Value;

use crate::asset;
use crate::assets::{Asset, lmms};
use crate::graphs;
use crate::raster::Rgba;
use crate::spec::{Bind, GraphSpec, GraphStyle, PanelSpec, PressCx, Target, button, graph, knob, led};
use crate::widgets::{KnobStyle, LedColor};

pub const ID: &str = "bitinvader";

static ARTWORK: Asset = asset!("bitinvader/artwork.png");
static WAVEGRAPH: Asset = asset!("bitinvader/wavegraph.png");
static SMOOTH_ON: Asset = asset!("bitinvader/smooth_active.png");
static SMOOTH_OFF: Asset = asset!("bitinvader/smooth_inactive.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK, &WAVEGRAPH, &SMOOTH_ON, &SMOOTH_OFF];

/// The wave and its length
fn wave(cx: &PressCx<'_>) -> (Vec<f64>, usize) {
    let mut s: Vec<f64> = (cx.field)("wave").items().map(|x| x.as_real().unwrap_or(0.0)).collect();
    if s.len() < 200 {
        s.resize(200, 0.0);
    }
    let n = ((cx.field)("sampleLength").as_int().unwrap_or(200).max(1) as usize).min(s.len());
    (s, n)
}

fn out(samples: &[f64]) -> Vec<(Target, Value)> {
    vec![(Target::Field("wave"), Value::list(samples.iter().map(|v| Value::real(*v).unwrap_or_default())))]
}

fn shape(cx: &PressCx<'_>, f: fn(f64) -> f64) -> Vec<(Target, Value)> {
    let (mut s, n) = wave(cx);
    graphs::fill(&mut s, n, f);
    out(&s)
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
    let (mut s, n) = wave(cx);
    graphs::noise(&mut s, n, &mut *cx.rand);
    out(&s)
}
fn smooth(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let (mut s, n) = wave(cx);
    graphs::smooth(&mut s, n);
    out(&s)
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let w = |f, id, x, on, off, tip: &'static str| button(Bind::Press(f), id, x, 205.0, on, off, tip).tip(tip);
    PanelSpec {
        plugin: ID,
        size: (250.0, 250.0),
        background: Some(&ARTWORK),
        fill: 0x000000,
        controls: vec![
            knob("sampleLength", 6.0, 201.0, KnobStyle::Dark28, 1.0, "Sample length", "Sample length", "").int_field(),
            graph(
                GraphSpec {
                    field: "wave",
                    w: 204.0,
                    h: 134.0,
                    min: -1.0,
                    max: 1.0,
                    length: 200,
                    length_param: Some("sampleLength"),
                    style: GraphStyle::Nearest,
                    color: Rgba::rgb(255, 255, 255),
                    background: Some(&WAVEGRAPH),
                    foreground: None,
                    step: 0.0,
                },
                23.0,
                59.0,
                "Wave",
            )
            .tip("Draw your own waveform here by dragging your mouse on this graph."),
            w(sine, "sine", 131.0, &lmms::SIN_WAVE_ACTIVE, &lmms::SIN_WAVE_INACTIVE, "Sine wave"),
            w(triangle, "triangle", 145.0, &lmms::TRIANGLE_WAVE_ACTIVE, &lmms::TRIANGLE_WAVE_INACTIVE, "Triangle wave"),
            w(saw, "saw", 159.0, &lmms::SAW_WAVE_ACTIVE, &lmms::SAW_WAVE_INACTIVE, "Saw wave"),
            w(square, "square", 173.0, &lmms::SQUARE_WAVE_ACTIVE, &lmms::SQUARE_WAVE_INACTIVE, "Square wave"),
            w(noise, "noise", 187.0, &lmms::WHITE_NOISE_WAVE_ACTIVE, &lmms::WHITE_NOISE_WAVE_INACTIVE, "White noise"),
            w(smooth, "smooth", 215.0, &SMOOTH_ON, &SMOOTH_OFF, "Smooth waveform"),
            led(
                Bind::Flag { field: "interpolation" },
                "interpolation",
                131.0,
                221.0,
                LedColor::Yellow,
                "Interpolation",
                "Interpolation",
            ),
            led(
                Bind::Switch { param: "normalize" },
                "normalize",
                131.0,
                236.0,
                LedColor::Green,
                "Normalize",
                "Normalize",
            ),
            led(
                Bind::Flag { field: "antiAliasing" },
                "antiAliasing",
                38.0,
                236.0,
                LedColor::Green,
                "Anti-aliasing",
                "Anti-aliasing",
            )
            .tip("High notes without the tones their harmonics make when they fold back below half the sample rate"),
        ],
        labels: Vec::new(),
        file_drop: None,
    }
});
