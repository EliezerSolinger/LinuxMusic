//! Oscilloscope (legado/plugins/Oscilloscope/OscilloscopeControlDialog.cpp, OscilloscopeGraph.cpp): the graph over
//! the whole width (the legacy's minimum, 400×200; the theme's colors: white for the mono trace, `#289fff` and
//! `#ee043b` for the two sides, `#ffc976` for the clipping lines, a gray center line on black), and under it the row
//! of controls with the legacy's 5 px margin and 10 px spacing: the pause button (the play icon while paused), the
//! Stereo LED and three `Bright26` knobs, Window Size (ms), Offset and Scale (a volume knob).
//!
//! The graph draws the plugin's live frame (`lm-plugins-effects`' `oscilloscope`: columns of minimum and maximum of
//! the span the knobs ask for) as the legacy's `paintEvent` does: a line from each column's minimum to its maximum
//! (to the next frame's, where a column is one frame). The wheel over it zooms the window (`Window Size`).
//!
//! Not ported: dragging the graph to scroll the offset (the Offset knob does it) and the wheel's zoom that keeps
//! the point under the cursor (the wheel here steps the window's length).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::{Canvas, Cap, Paint, Rgba};
use crate::spec::{Bind, PanelSpec, SurfaceCx, SurfaceSpec, button, knob, led, surface};
use crate::widgets::{KnobStyle, LedColor};

pub const ID: &str = "oscilloscope";

static PLAY: Asset = asset!("oscilloscope/play.png");
static PAUSE: Asset = asset!("oscilloscope/pause.png");
pub static ASSETS: &[&Asset] = &[&PLAY, &PAUSE];

/// The columns of the plugin's frame (`lm_plugins_effects::oscilloscope::COLS`)
const COLS: usize = 512;
const W: f32 = 400.0;
const GRAPH_H: f32 = 200.0;

const MONO: Rgba = Rgba::rgb(255, 255, 255);
const LEFT: Rgba = Rgba::rgb(0x28, 0x9f, 0xff);
const RIGHT: Rgba = Rgba::rgb(0xee, 0x04, 0x3b);
const MINOR: Rgba = Rgba::rgb(100, 100, 100);
const CLIPPING: Rgba = Rgba::rgb(0xff, 0xc9, 0x76);

/// `OscilloscopeGraph::paintEvent`
fn draw(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let (w, h) = (cx.w, cx.h);
    c.fill_rect(0.0, 0.0, w, h, Rgba::rgb(0, 0, 0));
    let amp = ((cx.param)("amp") * 0.01) as f32;
    c.line((0.0, (h / 2.0).floor()), (w, (h / 2.0).floor()), 1.0, Cap::Flat, &Paint::Solid(MINOR));
    // one pixel outside, so that the lines do not show at the default scale
    for y in [(h / 2.0 * (1.0 - amp) - 1.0).floor(), (h / 2.0 * (1.0 + amp) + 1.0).floor()] {
        c.line((0.0, y), (w, y), 1.0, Cap::Flat, &Paint::Solid(CLIPPING));
    }
    let cols = cx.live(0, 0.0) as usize;
    let hq = cx.live(1, 0.0) >= 0.5;
    let win = cx.live(2, 0.0);
    let per_col = cx.live(3, 1.0);
    if cols == 0 || win <= per_col {
        return;
    }
    let xscale = w / (win - per_col);
    let end = if hq { 0.0 } else { per_col };
    let stereo = (cx.param)("stereo") >= 0.5;
    let sides: &[(usize, Rgba)] = if stereo { &[(0, LEFT), (2, RIGHT)] } else { &[(0, MONO)] };
    for &(slot, color) in sides {
        let paint = Paint::Solid(Rgba { a: 0.8, ..color });
        for col in 0..cols.min(COLS) {
            let at = 4 + col * 4 + slot;
            let (lo, hi) = (cx.live(at, 0.0), cx.live(at + 1, 0.0));
            let x = col as f32 * per_col * xscale;
            let y = |v: f32| h * (1.0 - v * amp) / 2.0;
            c.line((x, y(lo)), (x + end * xscale, y(hi)), 1.0, Cap::Flat, &paint);
        }
    }
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls = vec![surface(
        SurfaceSpec {
            w: W,
            h: GRAPH_H,
            primary: "length",
            reads: &["amp", "stereo"],
            field_reads: &[],
            draw,
            texts: None,
            handles: None,
            drag: None,
            grab: 0.0,
            set_on_press: false,
            live: true,
            wheel: true,
            wheel_edit: None,
            peaks: None,
            markers: None,
            tag: 0,
        },
        "graph",
        0.0,
        0.0,
        "Oscilloscope",
    )];
    controls.push(button(Bind::Switch { param: "pause" }, "pause", 5.0, 209.0, &PLAY, &PAUSE, "Pause").tip("Pause"));
    controls.push(led(Bind::Switch { param: "stereo" }, "stereo", 45.0, 217.0, LedColor::Green, "Stereo", "Stereo"));
    controls.push(knob("length", 247.0, 207.0, KnobStyle::Bright26, 0.001, "Length", "Window Size", " ms").log());
    controls.push(knob("phase", 294.0, 207.0, KnobStyle::Bright26, 0.00001, "Phase", "Offset", ""));
    controls.push(knob("amp", 341.0, 207.0, KnobStyle::Bright26, 0.00001, "Amplitude", "Scale", "%").volume_at(100.0));
    PanelSpec {
        plugin: ID,
        size: (W, 245.0),
        background: None,
        view_backgrounds: &[],
        fill: 0x101116,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});
