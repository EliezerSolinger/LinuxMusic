//! Xpressive (legado/plugins/Xpressive/Xpressive.cpp, `XpressiveView`): the 250×250 artwork, the wave graph at the top
//! left (drawn by the mouse for W1 to W3), the five selectors of what is shown and edited (W1, W2, W3, O1, O2: the
//! panel's own view, not saved in the plugin) and the help, the expression editor with the validity lamp, the wave
//! shortcuts that append to the expression, the smoothness knob and the interpolate lamp of each W, and the six
//! knobs on the right (A1 to A3, the two pannings, the release transition).
//!
//! The editor is the panel's text editor over the field of the selected view (`w1Expr`... `o1`, `o2`); the help is a
//! view that covers the rest with the names the expressions know.
//!
//! The live frame (`lm_plugins_instruments::xpressive::FRAME_FLOATS`): one flag for each expression in the order W1,
//! W2, W3, O1, O2 (1 = does not compile) for the validity lamp, then the curve each output makes alone (a note of A4,
//! half a second, 256 columns) for the O1 and O2 graphs. A W graph is drawn whatever the smoothing
//! and the text (the legacy allows it only with the smoothing at 0 and an empty text).

use std::sync::LazyLock;

use lm_port::Value;

use crate::asset;
use crate::assets::{self, Asset};
use crate::raster::{Canvas, Cap, Paint, Rgba};
use crate::spec::{
    Bind, Control, GraphSpec, GraphStyle, PanelSpec, PressCx, StaticText, SurfText, SurfaceCx, SurfaceSpec, Target,
    Text, button, graph, knob, led, surface, text_edit, toggle,
};
use crate::widgets::{self, KnobStyle, LedColor, Styled};

pub const ID: &str = "xpressive";

static ARTWORK: Asset = asset!("xpressive/artwork.png");
static WAVEGRAPH: Asset = asset!("xpressive/wavegraph.png");

pub static ASSETS: &[&Asset] = &[&ARTWORK, &WAVEGRAPH];

const EXPR: [&str; 5] = ["w1Expr", "w2Expr", "w3Expr", "o1", "o2"];
const GRAPHS: [&str; 3] = ["w1Graph", "w2Graph", "w3Graph"];
const SMOOTH: [&str; 3] = ["smoothW1", "smoothW2", "smoothW3"];
const INTERP: [&str; 3] = ["interpolateW1", "interpolateW2", "interpolateW3"];
const HELP: u8 = 5;
const GRAPH_LEN: usize = 4096;
/// The live frame's layout (`lm_plugins_instruments::xpressive`: the tests check these numbers)
pub(crate) const FLAGS: usize = 5;
pub(crate) const PREVIEW_POINTS: usize = 256;

/// A wave shortcut: the text it appends to a W's expression and to an output's
fn append(cx: &mut PressCx<'_>, wave: &str, output: &str) -> Vec<(Target, Value)> {
    let view = (cx.view as usize).min(4);
    let field = EXPR[view];
    let mut text = (cx.field)(field).as_text().unwrap_or("").to_owned();
    text.push_str(if view < 3 { wave } else { output });
    vec![(Target::Field(field), Value::text(text.as_str()))]
}

macro_rules! shortcut {
    ($name:ident, $w:literal, $o:literal) => {
        fn $name(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
            append(cx, $w, $o)
        }
    };
}

shortcut!(moog_saw, "moogsaww(t)", "moogsaww(integrate(f))");
shortcut!(exp_wave, "expw(t)", "expw(integrate(f))");
shortcut!(saw, "saww(t)", "saww(integrate(f))");
shortcut!(sine, "sinew(t)", "sinew(integrate(f))");
shortcut!(triangle, "trianglew(t)", "trianglew(integrate(f))");
shortcut!(square, "squarew(t)", "squarew(integrate(f))");
shortcut!(noise, "randsv(t*srate,0)", "randsv(t*srate,0)");

/// The user wave (`setWaveToUser`): the text is cleared and the drawn points play (a sine to start from when
/// nothing was drawn)
fn user_wave(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let n = (cx.view as usize).min(2);
    let graph = (cx.field)(GRAPHS[n]);
    let mut edits = vec![(Target::Field(EXPR[n]), Value::text(""))];
    if graph.len() == 0 {
        let sine = (0..GRAPH_LEN)
            .map(|i| Value::real((i as f64 / GRAPH_LEN as f64 * std::f64::consts::TAU).sin()).unwrap_or_default());
        edits.push((Target::Field(GRAPHS[n]), Value::list(sine)));
    }
    edits
}

/// The knobs on the right: `XpressiveKnob`, 29×29, radii 4 and 9, line 3, a plain black pen
fn big_knob() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 29.0,
        h: 29.0,
        color: Rgba::rgb(0, 0, 0),
        outer: None,
        inner_radius: 4.0,
        outer_radius: 9.0,
        center: (14.5, 14.5),
        line_width: 3.0,
    })
}

/// The smoothness knob: 25×25, centered at 12.5
fn small_knob() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 25.0,
        h: 25.0,
        color: Rgba::rgb(0, 0, 0),
        outer: None,
        inner_radius: 4.0,
        outer_radius: 9.0,
        center: (12.5, 12.5),
        line_width: 3.0,
    })
}

/// A selector button of the view: a small plate, lit when selected
fn draw_plate(c: &mut Canvas, on: bool) {
    let (w, h) = (c.width as f32 / c.scale, c.height as f32 / c.scale);
    let (edge, fill) =
        if on { (Rgba::hex(0x7fd57f), Rgba::hex(0x1f4a24)) } else { (Rgba::hex(0x3d6a3d), Rgba::hex(0x12210f)) };
    crate::gxwidgets::rrect(c, 0.0, 0.0, w, h, 3.0, edge);
    crate::gxwidgets::rrect(c, 1.0, 1.0, w - 2.0, h - 2.0, 2.0, fill);
}

/// The validity lamp of the expression of the view in the tag (the live frame holds a flag for each)
fn draw_valid(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    widgets::draw_led(c, LedColor::Red, cx.live(cx.tag as usize, 0.0) >= 0.5);
}

/// The curve output `tag` (0 for O1, 1 for O2) makes, from the plugin's live frame: a note of A4 as the core played it
/// alone, one point per column
fn draw_output(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    c.fill_rect(0.0, 0.0, cx.w, cx.h, Rgba::rgb(0, 0, 0));
    let mid = (cx.h / 2.0).floor();
    c.line((0.0, mid), (cx.w, mid), 1.0, Cap::Flat, &Paint::Solid(Rgba::hex(0x2a3a2a)));
    let base = FLAGS + cx.tag as usize * PREVIEW_POINTS;
    let y = |v: f32| mid - v.clamp(-1.0, 1.0) * (cx.h / 2.0 - 1.0);
    let step = cx.w / (PREVIEW_POINTS - 1) as f32;
    let paint = Paint::Solid(Rgba::rgb(255, 255, 255));
    for i in 1..PREVIEW_POINTS {
        let (a, b) = (cx.live(base + i - 1, 0.0), cx.live(base + i, 0.0));
        c.line(((i - 1) as f32 * step, y(a)), (i as f32 * step, y(b)), 1.0, Cap::Flat, &paint);
    }
}

fn draw_cover(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    c.fill_rect(0.0, 0.0, cx.w, cx.h, Rgba::hex(0x101410));
}

fn help_texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
    let lines: &[&str] = if cx.tag == 0 {
        &["t  f  key  rel  trel", "A1  A2  A3  v  seed", "srate  bnote  tempo", "W1  W2  W3"]
    } else {
        &[
            "sinew  saww  squarew",
            "trianglew  expw  moogsaww",
            "randsv  integrate",
            "sin  cos  tan  atan  exp",
            "log  sqrt  abs  min  max",
        ]
    };
    lines
        .iter()
        .enumerate()
        .map(|(i, t)| SurfText {
            x: 8.0,
            y: 14.0 + i as f32 * 14.0,
            px: 10.0,
            text: (*t).to_owned(),
            color: 0x9be59b,
            centered: false,
        })
        .collect()
}

fn cover(tag: u32, w: f32, h: f32, texts: bool) -> SurfaceSpec {
    SurfaceSpec {
        w,
        h,
        // a parameter of the plugin only so that the cover takes the clicks (it has no handles)
        primary: "a1",
        reads: &[],
        field_reads: &[],
        draw: draw_cover,
        texts: texts.then_some(help_texts),
        handles: None,
        drag: None,
        grab: 0.0,
        set_on_press: false,
        live: false,
        wheel: false,
        wheel_edit: None,
        peaks: None,
        tag,
        markers: None,
    }
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut c: Vec<Control> = Vec::new();
    let mut labels: Vec<StaticText> = Vec::new();
    let leak = |s: String| -> &'static str { lm_port::intern(&s) };
    for v in 0..5usize {
        let view = v as u8;
        if v < 3 {
            c.push(
                graph(
                    GraphSpec {
                        field: GRAPHS[v],
                        w: 180.0,
                        h: 81.0,
                        min: -1.0,
                        max: 1.0,
                        length: GRAPH_LEN,
                        length_param: None,
                        style: GraphStyle::Linear,
                        color: Rgba::rgb(255, 255, 255),
                        background: Some(&WAVEGRAPH),
                        foreground: None,
                        step: 0.0,
                    },
                    3.0,
                    3.0,
                    "Wave",
                )
                .in_view_n(view),
            );
            c.push(
                knob(SMOOTH[v], 66.0, 198.0, small_knob(), 1.0, "Smoothness", "Smoothness", "")
                    .int_field()
                    .in_view_n(view),
            );
            c.push(
                led(
                    Bind::Switch { param: INTERP[v] },
                    leak(format!("interpolate{v}")),
                    2.0,
                    230.0,
                    LedColor::Green,
                    "Interpolate",
                    "Interpolate",
                )
                .in_view_n(view),
            );
            c.push(
                button(
                    Bind::Press(user_wave),
                    leak(format!("usr{v}")),
                    46.0,
                    195.0,
                    &assets::lmms::USR_WAVE_ACTIVE,
                    &assets::lmms::USR_WAVE_INACTIVE,
                    "User wave",
                )
                .in_view_n(view),
            );
        }
        c.push(
            text_edit(std::slice::from_ref(&EXPR[v]), leak(format!("expr{v}")), 3.0, 104.0, 180.0, 90.0, "Expression")
                .in_view_n(view),
        );
        c.push(
            surface(
                SurfaceSpec {
                    live: true,
                    ..{
                        let mut s = cover(v as u32, 16.0, 16.0, false);
                        s.primary = "";
                        s.draw = draw_valid;
                        s
                    }
                },
                leak(format!("valid{v}")),
                166.0,
                190.0,
                "Valid",
            )
            .in_view_n(view),
        );
    }
    for k in 0..2u32 {
        let mut graph = cover(k, 180.0, 81.0, false);
        graph.primary = "";
        graph.draw = draw_output;
        graph.live = true;
        c.push(surface(graph, leak(format!("output{k}")), 3.0, 3.0, "Output").in_view_n(3 + k as u8));
    }
    let waves: [(&str, fn(&mut PressCx<'_>) -> Vec<(Target, Value)>, f32, f32, &'static Asset, &'static Asset); 7] = [
        ("moog", moog_saw, 4.0, 195.0, &assets::lmms::MOOG_SAW_WAVE_ACTIVE, &assets::lmms::MOOG_SAW_WAVE_INACTIVE),
        ("exp", exp_wave, 18.0, 195.0, &assets::lmms::EXP_WAVE_ACTIVE, &assets::lmms::EXP_WAVE_INACTIVE),
        ("saw", saw, 32.0, 195.0, &assets::lmms::SAW_WAVE_ACTIVE, &assets::lmms::SAW_WAVE_INACTIVE),
        ("sine", sine, 4.0, 209.0, &assets::lmms::SIN_WAVE_ACTIVE, &assets::lmms::SIN_WAVE_INACTIVE),
        ("triangle", triangle, 18.0, 209.0, &assets::lmms::TRIANGLE_WAVE_ACTIVE, &assets::lmms::TRIANGLE_WAVE_INACTIVE),
        ("square", square, 32.0, 209.0, &assets::lmms::SQUARE_WAVE_ACTIVE, &assets::lmms::SQUARE_WAVE_INACTIVE),
        ("noise", noise, 46.0, 209.0, &assets::lmms::WHITE_NOISE_WAVE_ACTIVE, &assets::lmms::WHITE_NOISE_WAVE_INACTIVE),
    ];
    for (name, f, x, y, on, off) in waves {
        c.push(button(Bind::Press(f), leak(format!("wave-{name}")), x, y, on, off, leak(format!("Wave {name}"))));
    }
    let knobs: [(&str, f32, f32, &str, &str, f64); 6] = [
        ("a1", 191.0, 2.0, "A1", "General purpose 1:", 0.01),
        ("a2", 191.0, 34.0, "A2", "General purpose 2:", 0.01),
        ("a3", 191.0, 66.0, "A3", "General purpose 3:", 0.01),
        ("panning1", 191.0, 102.0, "O1 panning", "O1 panning:", 0.01),
        ("panning2", 191.0, 134.0, "O2 panning", "O2 panning:", 0.01),
        ("relTransition", 191.0, 166.0, "Release transition", "Release transition:", 0.1),
    ];
    for (p, x, y, name, hint, step) in knobs {
        let unit = if p == "relTransition" { "ms" } else { "" };
        c.push(knob(p, x, y, big_knob(), step, name, hint, unit));
    }
    // the help covers the controls under it (the cover takes the clicks), the selectors stay on top
    c.push(surface(cover(0, 250.0, 86.0, true), "help-top", 0.0, 0.0, "Help").in_view_n(HELP));
    c.push(surface(cover(1, 250.0, 148.0, true), "help-bottom", 0.0, 102.0, "Help").in_view_n(HELP));
    let plates: [(&str, u8, f32, f32); 5] = [
        ("W1", 0, 3.0, 22.0),
        ("W2", 1, 26.0, 22.0),
        ("W3", 2, 49.0, 22.0),
        ("O1", 3, 79.0, 19.0),
        ("O2", 4, 101.0, 19.0),
    ];
    for (name, view, x, w) in plates {
        c.push(toggle(
            Bind::View(view),
            leak(format!("select-{name}")),
            x,
            87.0,
            w,
            14.0,
            draw_plate,
            leak(format!("Select {name}")),
        ));
        labels.push(StaticText {
            x: x + 4.0,
            y: 88.0,
            px: 10.0,
            text: Text::new(leak(name.to_owned())),
            color: 0x9be59b,
            view: None,
        });
    }
    c.push(toggle(Bind::View(HELP), "select-help", 133.0, 87.0, 50.0, 14.0, draw_plate, "Help"));
    labels.push(StaticText { x: 148.0, y: 88.0, px: 10.0, text: Text::new("Help"), color: 0x9be59b, view: None });
    PanelSpec {
        plugin: ID,
        size: (250.0, 250.0),
        background: Some(&ARTWORK),
        view_backgrounds: &[],
        fill: 0x000000,
        controls: c,
        labels,
        file_drop: None,
    }
});
