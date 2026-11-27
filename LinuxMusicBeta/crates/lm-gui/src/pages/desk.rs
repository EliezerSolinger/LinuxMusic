//! The legacy desk's painted pieces, drawn as the legacy draws them (legado/src/gui: MixerChannelView.cpp,
//! MasterSectionView.cpp, widgets/DeskKnob.cpp, widgets/GainStageControls.cpp, widgets/Fader.cpp and the desk's rules
//! in data/themes/default/style.css): the same sizes in pixels, the same colors.
//!
//! The desk keeps the legacy's colors in both themes, as hardware keeps its colors in any room: they are the desk's
//! own, not the interface's (decisoes-autonomas.md, "A mesa de mixagem"). Everything else of the page takes the theme.

use gpui::{
    BorderStyle, Bounds, Div, FontWeight, Hsla, IntoElement, PathBuilder, Rgba, SharedString, Stateful, canvas, div,
    fill, linear_color_stop, linear_gradient, point, prelude::*, px, rgba, size,
};

use crate::view;

/// The desk's colors (style.css and the C++ sources), 0xRRGGBB
pub mod col {
    pub const STRIP: u32 = 0x14161a;
    pub const STRIP_ACTIVE: u32 = 0x3b424a;
    pub const STROKE_OUTER: u32 = 0x262b30;
    pub const STROKE_INNER: u32 = 0x0c0d0f;
    pub const TEXT: u32 = 0xd1d8e4;
    pub const DIM: u32 = 0x8c96a3;
    pub const KNOB_LABEL: u32 = 0xaab2be;
    pub const ANALOG_STRIP: u32 = 0x2f353c;
    pub const SECTION_LINE: u32 = 0x1a1d21;
    pub const PAN_BACKGROUND: u32 = 0x3b424a;
    pub const NAME_BACKGROUND: u32 = 0x2b3138;
    pub const MASTER_SECTION: u32 = 0x1f2328;
    pub const PEAK_BACKGROUND: u32 = 0x111811;
    pub const BUTTON_BORDER: u32 = 0x080b12;
    pub const GREEN_LED: u32 = 0x4afd85;
    pub const LED_OFF: u32 = 0x326e48;
    pub const LED_EMPTY: u32 = 0x1e2227;
    pub const LED_RED: u32 = 0xdc3c3c;
    pub const SLOT_LED_OFF: u32 = 0x464c54;
    // The knobs' caps, after their sections, as on Soundcraft and SSL desks
    pub const RED: u32 = 0xb02c2c;
    pub const GREY: u32 = 0x60666e;
    pub const TEAL: u32 = 0x268c80;
    pub const YELLOW: u32 = 0xd6aa28;
    pub const ORANGE: u32 = 0xd27428;
    pub const GREEN: u32 = 0x2e9654;
    pub const BLUE: u32 = 0x3466ba;
    pub const BROWN: u32 = 0x7a5230;
    pub const FX_SEND: u32 = 0x467096;
    pub const PAN: u32 = 0x969ca4;
    pub const BUS: u32 = 0x3c4046;
    // The meters' gradient (lmms--gui--Fader)
    pub const PEAK_OK: u32 = 0x0ad45c;
    pub const PEAK_WARN: u32 = 0xd6ec52;
    pub const PEAK_CLIP: u32 = 0xc12038;
}

/// Sizes in pixels (the legacy's constants)
pub const STRIP_W: f32 = 58.0;
pub const NAME_W: f32 = 50.0;
pub const PEAK_W: f32 = 26.0;
pub const MASTER_SECTION_W: f32 = 128.0;
pub const MASTER_W: f32 = MASTER_SECTION_W + 8.0;
pub const RACK_W: f32 = 245.0;
pub const KNOB_W: f32 = 25.0;
pub const KNOB_D: f32 = 17.0;
pub const KNOB_LABEL_H: f32 = 8.0;
pub const FADER_W: f32 = 23.0;
pub const PRE_FADER_W: f32 = 24.0;
pub const POST_FADER_W: f32 = 30.0;
/// The fader's handle (fader_knob.svg)
pub const HANDLE_H: f32 = 30.0;

pub fn rgb(hex: u32) -> Rgba {
    gpui::rgb(hex)
}

/// A color made lighter (`factor` > 100) or darker (< 100), as QColor::lighter and darker
pub fn shade(hex: u32, factor: f32) -> Rgba {
    let c = rgb(hex);
    let f = factor / 100.0;
    Rgba { r: (c.r * f).min(1.0), g: (c.g * f).min(1.0), b: (c.b * f).min(1.0), a: 1.0 }
}

fn glass() -> gpui::Background {
    // the buttons' gradient: light at the top, dark at the bottom
    linear_gradient(180.0, linear_color_stop(rgba(0x3a3f4633), 0.0), linear_color_stop(rgba(0x10131766), 1.0))
}

fn lit(top: u32, bottom: u32) -> gpui::Background {
    linear_gradient(180.0, linear_color_stop(rgb(top), 0.0), linear_color_stop(rgb(bottom), 1.0))
}

/// How a desk button lights when on (the style sheet's `:checked` rules)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    /// mute (red)
    Mute,
    /// solo (blue)
    Solo,
    /// phase, swap, low cut, link, the master's buttons (amber)
    Amber,
    /// MONO (orange, glowing)
    Glow,
    /// the gain stage's models and the compressors' families
    OpAmp,
    Tube,
    Fet,
    Optical,
    /// a choice that is never "on" (the gate's mode)
    Plain,
}

fn light(l: Light) -> (gpui::Background, u32, u32) {
    match l {
        Light::Mute => (lit(0xe2515b, 0x900122), 0x890120, 0xffffff),
        Light::Solo => (lit(0x5aa8d9, 0x16638c), 0x055f89, 0xffffff),
        Light::Amber => (lit(0xf2c25a, 0x9c6c08), 0x8a5a00, 0x14161a),
        Light::Glow | Light::Tube => (lit(0xffb347, 0xb33c00), 0x7a2a00, 0x2a0e00),
        Light::OpAmp => (lit(0x6fb0f0, 0x1b4f88), 0x0b3a6b, 0xffffff),
        Light::Fet => (lit(0x7ad89a, 0x1b6b3c), 0x0f4a22, 0xffffff),
        Light::Optical => (lit(0xfff0b0, 0xb8962e), 0x7a5a10, 0x2a1e00),
        Light::Plain => (glass(), col::BUTTON_BORDER, col::TEXT),
    }
}

/// A desk button: `w` × `h` pixels, a bold label of `text_px`, lit as `l` when `on`
pub fn desk_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    l: Light,
    w: f32,
    h: f32,
    text_px: f32,
) -> Stateful<Div> {
    let (bg, border, text) = if on { light(l) } else { (glass(), col::BUTTON_BORDER, col::TEXT) };
    div()
        .id(id.into())
        .w(px(w))
        .h(px(h))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .border_1()
        .border_color(rgb(border))
        .bg(bg)
        .text_color(rgb(text))
        .text_size(px(text_px))
        .font_weight(FontWeight::BOLD)
        .line_height(px(h))
        .overflow_hidden()
        .cursor_pointer()
        .child(label.into())
}

/// A section's title in the analog strip, with its divider above
pub fn section_title(text: impl Into<SharedString>, w: f32) -> Div {
    div()
        .w(px(w))
        .h(px(10.))
        .border_t_1()
        .border_color(rgb(col::SECTION_LINE))
        .flex()
        .justify_center()
        .text_size(px(7.))
        .line_height(px(9.))
        .text_color(rgb(col::DIM))
        .child(text.into())
}

/// The face of a desk knob (DeskKnob::paintEvent): a dark skirt, a colored cap lit from the top left, a white pointer
/// from -135 to 135 degrees. `t` is 0..1 of its travel; a knob that does not apply (a model without a release) is
/// drawn dim
pub fn desk_knob_face(cap: u32, t: f64, enabled: bool) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let center = point(b.origin.x + b.size.width / 2.0, b.origin.y + px(KNOB_D / 2.0));
            let disc = |r: f32| Bounds::new(point(center.x - px(r), center.y - px(r)), size(px(2.0 * r), px(2.0 * r)));
            window.paint_quad(fill(disc(KNOB_D / 2.0), rgb(0x121417)).corner_radii(px(KNOB_D / 2.0)));
            let r = KNOB_D / 2.0 - 2.5;
            let (hi, lo) =
                if enabled { (shade(cap, 150.0), shade(cap, 77.0)) } else { (shade(cap, 60.0), shade(cap, 40.0)) };
            window.paint_quad(
                gpui::quad(
                    disc(r),
                    px(r),
                    linear_gradient(135.0, linear_color_stop(hi, 0.0), linear_color_stop(lo, 1.0)),
                    px(0.0),
                    Hsla::transparent_black(),
                    BorderStyle::Solid,
                )
                .corner_radii(px(r)),
            );
            let a = ((t.clamp(0.0, 1.0) - 0.5) * 270.0).to_radians() as f32;
            let (dx, dy) = (a.sin(), -a.cos());
            let mut pb = PathBuilder::stroke(px(1.6));
            pb.move_to(point(center.x + px(dx * 1.5), center.y + px(dy * 1.5)));
            pb.line_to(point(center.x + px(dx * (r + 1.0)), center.y + px(dy * (r + 1.0))));
            if let Ok(p) = pb.build() {
                window.paint_path(p, rgb(if enabled { 0xf5f5f5 } else { 0x8a8f96 }));
            }
        },
    )
    .w(px(KNOB_W))
    .h(px(KNOB_D))
}

/// A desk knob with its name below (25 × 25 pixels)
pub fn desk_knob(cap: u32, label: impl Into<SharedString>, t: f64, enabled: bool) -> Div {
    div()
        .w(px(KNOB_W))
        .h(px(KNOB_D + KNOB_LABEL_H))
        .flex()
        .flex_col()
        .items_center()
        .child(desk_knob_face(cap, t, enabled))
        .child(
            div()
                .h(px(KNOB_LABEL_H))
                .text_size(px(7.))
                .line_height(px(KNOB_LABEL_H))
                .text_color(rgb(col::KNOB_LABEL))
                .child(label.into()),
        )
}

/// The plain knob of the digital desk (Knob Small17 and Bright26): a dark disc with a light ring and a pointer
pub fn plain_knob(t: f64, d: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let center = b.center();
            let r = d / 2.0 - 1.0;
            let disc = Bounds::new(point(center.x - px(r), center.y - px(r)), size(px(2.0 * r), px(2.0 * r)));
            window.paint_quad(gpui::quad(disc, px(r), rgb(0x23272c), px(1.0), rgb(0x5a626c), BorderStyle::Solid));
            let a = ((t.clamp(0.0, 1.0) - 0.5) * 270.0).to_radians() as f32;
            let mut pb = PathBuilder::stroke(px(1.5));
            pb.move_to(point(center.x + px(a.sin() * r * 0.25), center.y - px(a.cos() * r * 0.25)));
            pb.line_to(point(center.x + px(a.sin() * (r - 1.0)), center.y - px(a.cos() * (r - 1.0))));
            if let Ok(p) = pb.build() {
                window.paint_path(p, rgb(col::GREEN_LED));
            }
        },
    )
    .size(px(d))
}

/// What a fader is, by the color of its handle (MixerChannelView::updateRole)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Channel,
    Return,
    Post,
    Pre,
}

/// Where the handle's center is, for a travel `t`, in a fader `h` pixels tall
pub fn handle_y(t: f64, h: f32) -> f32 {
    HANDLE_H / 2.0 + (h - HANDLE_H) * (1.0 - t.clamp(0.0, 1.0) as f32)
}

/// The fader over its meters (Fader::paintEvent, the user's decision of 19/11): two meters, left and right, black
/// with rounded corners, filled from the bottom with the green-yellow-red gradient, and the handle of `role` at `t`.
/// `levels` are 0..1 of the meter's height
pub fn fader(t: f64, levels: [f32; 2], role: Role, w: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let (o, h) = (b.origin, b.size.height.as_f32());
            let mw = ((w - 2.0 - 2.0) / 2.0).floor();
            for (k, level) in levels.iter().enumerate() {
                let x = if k == 0 { 1.0 } else { w - 1.0 - mw };
                let outline = Bounds::new(point(o.x + px(x), o.y + px(1.0)), size(px(mw), px(h - 2.0)));
                window.paint_quad(fill(outline, rgb(0x000000)).corner_radii(px(2.0)));
                let inner_h = h - 4.0;
                let fill_h = inner_h * level.clamp(0.0, 1.0);
                if fill_h > 0.5 {
                    // the gradient runs from 0 dBFS (red) through -6 (yellow) to -12 (green) over the meter's height
                    let top = if *level > 0.98 {
                        col::PEAK_CLIP
                    } else if *level > 0.85 {
                        col::PEAK_WARN
                    } else {
                        col::PEAK_OK
                    };
                    let bar = Bounds::new(
                        point(o.x + px(x + 1.0), o.y + px(2.0 + inner_h - fill_h)),
                        size(px(mw - 2.0), px(fill_h)),
                    );
                    let bg = linear_gradient(
                        180.0,
                        linear_color_stop(rgb(top), 0.0),
                        linear_color_stop(rgb(col::PEAK_OK), 1.0),
                    );
                    window.paint_quad(gpui::quad(
                        bar,
                        px(2.0),
                        bg,
                        px(0.0),
                        Hsla::transparent_black(),
                        BorderStyle::Solid,
                    ));
                }
            }
            // the handle (fader_knob*.svg): a dark body with a line in the role's color
            let cy = handle_y(t, h);
            let body = Bounds::new(point(o.x + px((w - 12.0) / 2.0), o.y + px(cy - 12.0)), size(px(12.0), px(24.0)));
            window.paint_quad(gpui::quad(
                body,
                px(2.0),
                linear_gradient(180.0, linear_color_stop(rgb(0x2c3138), 0.0), linear_color_stop(rgb(0x1c1f24), 1.0)),
                px(1.0),
                rgb(0x000000),
                BorderStyle::Solid,
            ));
            let line = match role {
                Role::Channel => 0x0bd556,
                Role::Return => 0x4a9bf0,
                Role::Post => 0xe03040,
                Role::Pre => 0xf09020,
            };
            window.paint_quad(fill(
                Bounds::new(point(body.origin.x + px(0.5), o.y + px(cy - 1.0)), size(px(11.0), px(2.0))),
                rgb(line),
            ));
        },
    )
    .w(px(w))
    .h_full()
}

/// The peak level above the fader (PeakIndicator): the highest peak since the last click, in dB
pub fn peak_indicator(id: impl Into<gpui::ElementId>, db: f32, mode: crate::view::MeterMode) -> Stateful<Div> {
    // the held level in the mode's unit (VU and K read from their own zero)
    let text = if db <= -60.0 { "-inf".to_string() } else { format!("{:.1}", db + mode.offset()) };
    let red = mode.over(db + mode.offset());
    div()
        .id(id.into())
        .w(px(PEAK_W))
        .h(px(12.))
        .flex()
        .justify_center()
        .bg(rgb(col::PEAK_BACKGROUND))
        .text_size(px(8.))
        .line_height(px(12.))
        .text_color(rgb(if red { col::LED_RED } else { col::TEXT }))
        .cursor_pointer()
        .child(text)
}

/// The channel's number (LcdWidget, two digits)
pub fn lcd(n: usize) -> Div {
    div()
        .w(px(20.))
        .h(px(13.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .bg(rgb(0x0c0d0f))
        .border_1()
        .border_color(rgb(col::STROKE_OUTER))
        .font_family("Menlo")
        .text_size(px(9.))
        .text_color(rgb(col::GREEN_LED))
        .child(format!("{:02}", n % 100))
}

/// An effect slot's state, for the LEDs and the list
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    On,
    Off,
    Corrupted,
}

pub const SLOTS: usize = 5;
pub const LED_W: f32 = 11.0;
pub const LED_H: f32 = 5.0;
pub const LED_PITCH: f32 = LED_H + 2.0;

/// An effect LED's color (EffectLeds): empty, on, off, or red for corrupted audio
pub fn slot_color(state: Option<Slot>) -> u32 {
    match state {
        None => col::LED_EMPTY,
        Some(Slot::On) => col::GREEN_LED,
        Some(Slot::Off) => col::LED_OFF,
        Some(Slot::Corrupted) => col::LED_RED,
    }
}

/// The gain stage's SAT and CLIP LEDs (SaturationLeds), or with a tube its lamp: yellow in the light mode, red in
/// the realistic one, glowing as it saturates
pub fn saturation_leds(sat: f32, clip: bool, tube: bool, realistic: bool) -> Div {
    let led = |on: f32, color: u32| {
        let c = rgb(color);
        let dim = Rgba { r: c.r * 0.25, g: c.g * 0.25, b: c.b * 0.25, a: 1.0 };
        let mix = |a: f32, b: f32| a + (b - a) * on.clamp(0.0, 1.0);
        let shown = Rgba { r: mix(dim.r, c.r), g: mix(dim.g, c.g), b: mix(dim.b, c.b), a: 1.0 };
        div().size(px(7.)).rounded(px(3.5)).bg(shown).border_1().border_color(rgb(0x0c0d0f))
    };
    let label = |t: &'static str| div().text_size(px(6.)).line_height(px(12.)).text_color(rgb(0x969ea8)).child(t);
    let first = if tube {
        // the lamp: a glass bulb whose heater glows
        let heat = if realistic { 0xff341e } else { 0xffc846 };
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(led(0.25 + 0.75 * sat, heat).size(px(9.)).rounded(px(4.5)).border_color(rgb(0xaab4be)))
            .child(label(if realistic { "RL" } else { "LT" }))
    } else {
        div().flex().items_center().gap(px(2.)).child(led(sat, 0xffc428)).child(label("SAT"))
    };
    div().w(px(50.)).h(px(12.)).flex().items_center().justify_between().px(px(1.)).child(first).child(
        div().flex().items_center().gap(px(2.)).child(led(if clip { 1.0 } else { 0.0 }, 0xff281e)).child(label("CLIP")),
    )
}

/// The scale of dB between the master's faders (DbScale): its ticks at the fader's own positions
pub fn db_scale(h: f32) -> Div {
    let mut d = div().w(px(18.)).h(px(h)).relative();
    let mut last = -100.0;
    for db in [12.0, 6.0, 0.0, -6.0, -12.0, -18.0, -24.0, -30.0, -40.0, -60.0] {
        let y = handle_y(view::db_to_travel(db), h);
        if y - last < 9.0 || y > h {
            continue;
        }
        last = y;
        d = d.child(
            div()
                .absolute()
                .left(px(0.))
                .top(px(y - 4.5))
                .w(px(18.))
                .h(px(9.))
                .flex()
                .justify_center()
                .text_size(px(7.))
                .line_height(px(9.))
                .text_color(rgb(if db == 0.0 { 0xe6ebf0 } else { 0x8c96a3 }))
                .child(if db > 0.0 { format!("+{db}") } else { format!("{db}") }),
        );
    }
    d
}

/// The backlit face of the master's moving-coil meters (VuMeter and GainReductionMeter): `position` is the needle's
/// 0..1, `red_from` where the red part of the scale starts (1 for none)
fn needle_meter(position: f32, red_from: f32, ticks: Vec<f32>, w: f32, h: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(gpui::quad(
                b,
                px(3.0),
                linear_gradient(180.0, linear_color_stop(rgb(0xfaecbe), 0.0), linear_color_stop(rgb(0xe2c88c), 1.0)),
                px(1.0),
                rgb(0x141619),
                BorderStyle::Solid,
            ));
            let pivot = point(b.origin.x + b.size.width / 2.0, b.origin.y + b.size.height + px(10.0));
            let radius = h + 2.0;
            let at = |p: f32, r: f32| {
                let a = (-48.0 + 96.0 * p).to_radians();
                point(pivot.x + px(a.sin() * r), pivot.y - px(a.cos() * r))
            };
            let scale_r = radius - 12.0;
            for (from, to, color, width) in [(0.0, red_from, 0x1e1e1e, 1.0), (red_from, 1.0, 0xc81e1e, 2.5)] {
                if to <= from {
                    continue;
                }
                let mut pb = PathBuilder::stroke(px(width));
                pb.move_to(at(from, scale_r));
                let mut p = from;
                while p < to {
                    p = (p + 0.02).min(to);
                    pb.line_to(at(p, scale_r));
                }
                if let Ok(path) = pb.build() {
                    window.paint_path(path, rgb(color));
                }
            }
            for p in &ticks {
                let mut pb = PathBuilder::stroke(px(1.0));
                pb.move_to(at(*p, scale_r));
                pb.line_to(at(*p, scale_r + 3.0));
                if let Ok(path) = pb.build() {
                    window.paint_path(path, rgb(if *p > red_from { 0xc81e1e } else { 0x1e1e1e }));
                }
            }
            let mut pb = PathBuilder::stroke(px(1.2));
            pb.move_to(at(position.clamp(0.0, 1.08), 14.0));
            pb.line_to(at(position.clamp(0.0, 1.08), radius - 4.0));
            if let Ok(path) = pb.build() {
                window.paint_path(path, rgb(0x0f0f0f));
            }
        },
    )
    .w(px(w))
    .h(px(h))
}

/// The needle's travel for a level in dB (VU): proportional to the voltage, +3 VU at the end
pub fn vu_position(db: f32) -> f32 {
    10f32.powf(db / 20.0) / 10f32.powf(3.0 / 20.0)
}

/// A VU meter of the master (58 × 44), `label` L or R
pub fn vu_meter(label: &'static str, position: f32) -> Div {
    let ticks =
        [-20.0, -10.0, -7.0, -5.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0].iter().map(|d| vu_position(*d)).collect();
    div()
        .relative()
        .w(px(58.))
        .h(px(44.))
        .child(needle_meter(position, vu_position(0.0), ticks, 58.0, 44.0))
        .child(
            div()
                .absolute()
                .left(px(0.))
                .top(px(28.))
                .w(px(58.))
                .flex()
                .justify_center()
                .text_size(px(8.))
                .text_color(rgb(0x282828))
                .child("VU"),
        )
        .child(div().absolute().left(px(3.)).top(px(33.)).text_size(px(7.)).text_color(rgb(0x282828)).child(label))
}

/// The bus compressor's gain reduction, on its own meter as on the SSL: 0 at the right, 20 dB at the left
pub fn gain_reduction_meter(reduction_db: f32, w: f32) -> Div {
    let position = |r: f32| 1.0 - (r.clamp(0.0, 20.0) / 20.0).sqrt();
    let ticks = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 14.0, 20.0].iter().map(|d| position(*d)).collect();
    div().relative().w(px(w)).h(px(44.)).child(needle_meter(position(reduction_db), 1.0, ticks, w, 44.0)).child(
        div()
            .absolute()
            .left(px(0.))
            .top(px(31.))
            .w(px(w))
            .flex()
            .justify_center()
            .text_size(px(7.))
            .text_color(rgb(0x282828))
            .child("GR dB"),
    )
}

/// The goniometer: mid up, side sideways, the latest points brightest
pub fn goniometer(points: Vec<(f32, f32)>, w: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(fill(b, rgb(0x0c0d0f)));
            let c = b.center();
            let r = b.size.height.as_f32() / 2.0 - 2.0;
            for (from, to) in [((0.0, -r), (0.0, r)), ((-r, 0.0), (r, 0.0)), ((-r, -r), (r, r)), ((r, -r), (-r, r))] {
                let mut pb = PathBuilder::stroke(px(1.0));
                pb.move_to(point(c.x + px(from.0), c.y + px(from.1)));
                pb.line_to(point(c.x + px(to.0), c.y + px(to.1)));
                if let Ok(p) = pb.build() {
                    window.paint_path(p, rgb(0x282d32));
                }
            }
            let n = points.len().max(1) as f32;
            for (i, (side, mid)) in points.iter().enumerate() {
                let x = (-side * r).clamp(-r, r);
                let y = (mid * r).clamp(-r, r);
                let alpha = 0.15 + 0.6 * i as f32 / n;
                let mut color = rgb(col::GREEN_LED);
                color.a = alpha;
                window.paint_quad(fill(Bounds::new(point(c.x + px(x), c.y - px(y)), size(px(1.2), px(1.2))), color));
            }
        },
    )
    .w(px(w))
    .h(px(56.))
}

/// The correlation of left and right: +1 mono, 0 wide, below 0 they cancel
pub fn correlation(v: f32, w: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(fill(b, rgb(0x0c0d0f)));
            let center = b.origin.x + b.size.width / 2.0;
            let at = center + (b.size.width / 2.0 - px(1.0)) * v.clamp(-1.0, 1.0);
            let (x0, x1) = if at < center { (at, center) } else { (center, at) };
            window.paint_quad(fill(
                Bounds::new(point(x0, b.origin.y + px(1.0)), size(x1 - x0 + px(1.0), b.size.height - px(2.0))),
                rgb(if v < 0.0 { col::LED_RED } else { col::GREEN_LED }),
            ));
            window
                .paint_quad(fill(Bounds::new(point(center, b.origin.y), size(px(1.0), b.size.height)), rgb(0x8c96a3)));
        },
    )
    .w(px(w))
    .h(px(8.))
}

/// The send button (SendButtonIndicator): lit while the selected channel sends to this one
pub fn send_button(id: impl Into<gpui::ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id.into())
        .w(px(29.))
        .h(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .border_1()
        .border_color(rgb(col::BUTTON_BORDER))
        .bg(if on { lit(0x7affa8, 0x1f9a4c) } else { glass() })
        .text_size(px(8.))
        .text_color(rgb(if on { 0x0c1a10 } else { col::DIM }))
        .cursor_pointer()
        .child("▼")
}

/// The receive arrow: this channel sends to the selected one
pub fn receive_arrow() -> Div {
    div()
        .w(px(29.))
        .h(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(9.))
        .text_color(rgb(col::GREEN_LED))
        .child("▲")
}

/// The arrow under the send knob, from the send into the strip
pub fn send_arrow() -> Div {
    div().h(px(8.)).flex().justify_center().text_size(px(7.)).line_height(px(8.)).text_color(rgb(col::DIM)).child("▼")
}

/// The ghost channel at the end of the channels: a dashed outline and a plus; a click makes a channel
pub fn ghost(id: impl Into<gpui::ElementId>) -> Stateful<Div> {
    div().id(id.into()).w(px(STRIP_W)).h_full().p(px(3.)).cursor_pointer().child(
        div()
            .size_full()
            .rounded(px(3.))
            .border_1()
            .border_dashed()
            .border_color(rgb(0x464c54))
            .hover(|d| d.border_color(rgb(0x8c96a3)))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(16.))
            .text_color(rgb(0x464c54))
            .child("+"),
    )
}

/// The page's background, around the strips
pub fn page_bg() -> Rgba {
    rgb(0x101215)
}

/// A strip's background (MixerChannelView::paintEvent): its color darkened, unless muted; lighter when selected
pub fn strip_bg(color: Option<u32>, muted: bool, active: bool) -> Rgba {
    match color {
        Some(c) if !muted => shade(c, if active { 83.0 } else { 67.0 }),
        _ => rgb(if active { col::STRIP_ACTIVE } else { col::STRIP }),
    }
}

/// The two borders of a strip, inner and outer
pub fn strip_borders() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |b, _, window, _| {
            window.paint_quad(gpui::quad(
                b,
                px(0.0),
                Hsla::transparent_black(),
                px(1.0),
                rgb(col::STROKE_OUTER),
                BorderStyle::Solid,
            ));
            let inner = Bounds::new(b.origin + point(px(1.0), px(1.0)), b.size - size(px(2.0), px(2.0)));
            window.paint_quad(gpui::quad(
                inner,
                px(0.0),
                Hsla::transparent_black(),
                px(1.0),
                rgb(col::STROKE_INNER),
                BorderStyle::Solid,
            ));
        },
    )
    .absolute()
    .left(px(0.))
    .top(px(0.))
    .size_full()
}

/// The group channel's marker (a triangle, down while its channels show, right while hidden) and the line in the
/// group's color along a member's left edge
pub fn group_marker(collapsed: bool) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let (x, y, s) = (b.origin.x + px(2.0), b.origin.y + px(2.0), px(8.0));
            let mut pb = PathBuilder::fill();
            if collapsed {
                pb.move_to(point(x, y));
                pb.line_to(point(x, y + s));
                pb.line_to(point(x + s, y + s / 2.0));
            } else {
                pb.move_to(point(x, y));
                pb.line_to(point(x + s, y));
                pb.line_to(point(x + s / 2.0, y + s));
            }
            pb.close();
            if let Ok(p) = pb.build() {
                window.paint_path(p, rgb(col::TEXT));
            }
        },
    )
    .size(px(12.))
}

pub fn member_line(color: Rgba) -> Div {
    div().absolute().left(px(1.)).top(px(1.)).w(px(3.)).h_full().bg(Rgba { a: 0.55, ..color })
}

/// The legacy's colour palette for the mixer (ColorChooser::nicePalette(140)): 8 hues in 6 shades, by column
pub fn mixer_palette() -> [u32; 48] {
    let mut out = [0u32; 48];
    for x in 0..8 {
        for y in 0..6 {
            // QColor::setHsl: hue in degrees, saturation and lightness 0..255
            let h = (44 * x as i32 - 1).max(0) as f32;
            let s = (150 - 20 * y) as f32 / 255.0;
            let l = (140 - 10 * y) as f32 / 255.0;
            out[6 * x + y] = hsl(h, s, l);
        }
    }
    out
}

fn hsl(h: f32, s: f32, l: f32) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| (((v + m) * 255.0).round().clamp(0.0, 255.0)) as u32;
    (to(r) << 16) | (to(g) << 8) | to(b)
}
