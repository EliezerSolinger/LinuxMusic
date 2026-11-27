//! Amp & IR (legado/plugins/AmpAndIr, the fork's own): the legacy view had no art (two loose buttons and knobs), so
//! by the owner's rule (I-22) the panel is in its own style, the rack unit of the Guitarix panels: the brushed skin,
//! two dark boxes (the amp model and the impulse response, each with its open button, its file's name and a button
//! that takes it away) and the input and output gains as vector knobs with the LED ring.
//!
//! The model is a `.nam` file (sent as bytes to `plugin.load_into_field`, which stores it as a resource of the
//! project) and the impulse response any audio file (imported through the port).

use std::sync::LazyLock;

use crate::gxwidgets::{self as gx, GxKnob};
use crate::panels::sampler::LOAD_ACTION;
use crate::raster::{Canvas, Rgba};
use crate::spec::{
    Bind, Control, PanelSpec, PressCx, StaticText, SurfaceSpec, Target, Text, knob, readout, surface, toggle,
};
use crate::widgets::KnobStyle;
use lm_port::Value;

pub const ID: &str = "ampandir";
pub static ASSETS: &[&crate::assets::Asset] = &[];

const W: f32 = 420.0;
const H: f32 = 224.0;
const ROW_X: f32 = 44.0;
const ROW_W: f32 = W - 88.0;
const ROW_H: f32 = 34.0;
const COLOR: u32 = 0xd8d8d8;

fn decoration(w: f32, h: f32, draw: crate::spec::SurfDrawFn) -> SurfaceSpec {
    SurfaceSpec {
        w,
        h,
        primary: "",
        reads: &[],
        field_reads: &[],
        draw,
        texts: None,
        handles: None,
        drag: None,
        grab: 0.0,
        set_on_press: false,
        live: false,
        wheel: false,
        wheel_edit: None,
        peaks: None,
        tag: 0,
        markers: None,
    }
}

/// The "x" of a clear button on a tab
fn draw_clear(c: &mut Canvas, on: bool) {
    gx::draw_tab(c, on);
    let (w, h) = (c.width as f32 / c.scale, c.height as f32 / c.scale);
    let paint = crate::raster::Paint::Solid(Rgba::hex(gx::ORANGE_HEX));
    let (cx, cy) = (w / 2.0, (h - 2.0) / 2.0);
    c.line((cx - 4.0, cy - 4.0), (cx + 4.0, cy + 4.0), 1.5, crate::raster::Cap::Round, &paint);
    c.line((cx - 4.0, cy + 4.0), (cx + 4.0, cy - 4.0), 1.5, crate::raster::Cap::Round, &paint);
}

fn clear_nam(_: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    vec![(Target::Field("nam"), Value::NULL)]
}

fn clear_ir(_: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    vec![(Target::Field("ir"), Value::NULL)]
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls: Vec<Control> = vec![surface(decoration(W, H, gx::skin_surface), "skin", 0.0, 0.0, "Amp & IR")];
    let mut labels = vec![StaticText { x: 44.0, y: 8.0, px: 13.0, text: Text::new("Amp & IR"), color: 0x222222, view: None }];
    // (a row: the box, the open button, the name of the file, the button that clears it)
    type Clear = fn(&mut PressCx<'_>) -> Vec<(Target, Value)>;
    type Row = (&'static str, f32, &'static str, &'static str, Option<&'static str>, &'static [&'static str], Clear);
    let rows: [Row; 2] = [
        ("nam", 40.0, "Open amp model", "Amp model", Some(LOAD_ACTION), &["nam"], clear_nam),
        ("ir", 84.0, "Open impulse response", "Impulse response", None, &[], clear_ir),
    ];
    for (field, y, open_name, title, action, extensions, clear) in rows {
        controls.push(surface(decoration(ROW_W, ROW_H, gx::dark_surface), leak(format!("box-{field}")), ROW_X, y, title));
        controls.push(toggle(
            Bind::Open { field, action, extensions },
            leak(format!("open-{field}")),
            ROW_X + 4.0,
            y + 6.0,
            50.0,
            22.0,
            gx::draw_tab,
            open_name,
        ));
        labels.push(StaticText {
            x: ROW_X + 15.0,
            y: y + 10.0,
            px: 11.0,
            text: Text::new("Open"),
            color: COLOR,
            view: None,
        });
        controls.push(readout(field, ROW_X + 62.0, y + 9.0, ROW_W - 100.0, gx::ORANGE_HEX, "-").small());
        controls.push(toggle(
            Bind::Press(clear),
            leak(format!("clear-{field}")),
            ROW_X + ROW_W - 30.0,
            y + 6.0,
            22.0,
            22.0,
            draw_clear,
            leak(format!("Clear {title}")),
        ));
        labels.push(StaticText { x: ROW_X, y: y - 14.0, px: 11.0, text: Text::new(title), color: 0x222222, view: None });
    }
    for (param, x, label) in [("input", 100.0, "Input"), ("output", 252.0, "Output")] {
        controls.push(knob(param, x, 130.0, KnobStyle::Gx(GxKnob::SmallR), 0.1, label, label, "dB"));
        labels.push(StaticText { x: x + 18.0, y: 196.0, px: 11.0, text: Text::new(label), color: 0x222222, view: None });
    }
    PanelSpec { plugin: ID, size: (W, H), background: None, view_backgrounds: &[], fill: 0x7a9a58, controls, labels, file_drop: None }
});

fn leak(s: String) -> &'static str {
    lm_port::intern(&s)
}
