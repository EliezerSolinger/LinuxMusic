//! The shared pieces every screen uses. Colors come only from the theme; texts are passed already translated.

use gpui::{
    AnyElement, App, Bounds, ElementId, FontWeight, IntoElement, PathBuilder, SharedString, Window, canvas, div, fill,
    point, prelude::*, px, size,
};
use lm_port::Loading;

use super::i18n::{tr, trf};
use super::theme::{THEME, c};

/// A small toggle-style button (design 02: M, S, record...): `on` paints it in `on_color`
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    on_color: u32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .flex()
        .items_center()
        .justify_center()
        .h(px(18.))
        .min_w(px(18.))
        .px(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(c(if on { on_color } else { THEME.line2 }))
        .bg(c(if on { on_color } else { THEME.bg3 }))
        .text_color(c(if on { THEME.on_accent } else { THEME.text }))
        .text_size(px(10.5))
        .font_weight(FontWeight::BOLD)
        .cursor_pointer()
        .child(label.into())
}

/// The bigger button of the transport and toolbars
pub fn tbutton(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    on_color: u32,
) -> gpui::Stateful<gpui::Div> {
    button(id, label, on, on_color).h(px(26.)).min_w(px(28.)).px(px(8.)).text_size(px(12.))
}

/// A tab of a tab bar (pages, editor tabs, design 05's text tabs)
pub fn tab(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .px(px(14.))
        .py(px(6.))
        .text_size(px(13.))
        .font_weight(if active { FontWeight::BOLD } else { FontWeight::NORMAL })
        .text_color(c(if active { THEME.text } else { THEME.text2 }))
        .border_b_2()
        .border_color(c(if active { THEME.green } else { THEME.bg2 }))
        .cursor_pointer()
        .child(label.into())
}

/// A small caption above a group of controls
pub fn caption(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_size(px(10.)).text_color(c(THEME.text3)).child(text.into())
}

/// A vertical meter: fill 0..1 on a -60..0 dB scale, green / yellow / red as design 02
pub fn meter(fill_v: f32, w: f32, h: f32) -> impl IntoElement {
    canvas(move |_, _, _| {}, move |b, _, window, _| paint_meter(b, fill_v, window)).w(px(w)).h(px(h))
}

fn paint_meter(b: Bounds<gpui::Pixels>, fill_v: f32, window: &mut Window) {
    window.paint_quad(fill(b, c(THEME.lcd_bg)));
    let hh = b.size.height.as_f32();
    let level = fill_v * hh;
    let zones = [(0.0, 0.7, THEME.green), (0.7, 0.9, THEME.yellow), (0.9, 1.0, THEME.red)];
    for (a, z, col) in zones {
        let lo = a * hh;
        let hi = (z * hh).min(level);
        if hi > lo {
            let bb = Bounds::new(point(b.origin.x, b.origin.y + px(hh - hi)), size(b.size.width, px(hi - lo)));
            window.paint_quad(fill(bb, c(col)));
        }
    }
}

/// The knob of design 02 (REAPER-like): a disc with an indicator, -135..+135 degrees, `t` in 0..1
pub fn knob_face(t: f64, d: f32) -> impl IntoElement {
    canvas(
        move |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(gpui::quad(b, px(d / 2.0), c(THEME.bg4), px(1.0), c(THEME.line), Default::default()));
            let center = b.center();
            let a = (-135.0 + 270.0 * t as f32).to_radians();
            let r = d / 2.0 - 3.0;
            let mut pb = PathBuilder::stroke(px(2.0));
            pb.move_to(point(center.x + px(a.sin() * r * 0.3), center.y - px(a.cos() * r * 0.3)));
            pb.line_to(point(center.x + px(a.sin() * r), center.y - px(a.cos() * r)));
            if let Ok(p) = pb.build() {
                window.paint_path(p, c(THEME.green));
            }
        },
    )
    .size(px(d))
}

/// The mixing desk's fader over its meter (user's decision, 19/11): the meter bar runs the whole travel and the
/// fader cap slides over it. `t` is the fader's travel 0..1 (`view::db_to_knob`), `levels` the meter fills (one
/// per channel of the strip). The caller makes it interactive (gesture on drag, 0 dB on double click).
pub fn fader_over_meter(t: f64, levels: Vec<f32>, w: f32, h: f32) -> impl IntoElement {
    canvas(
        move |_, _, _| {},
        move |b, _, window, _| {
            let o = b.origin;
            let hh = b.size.height.as_f32();
            // the meters, side by side, in the middle of the strip
            let n = levels.len().max(1) as f32;
            let mw = 5.0;
            let total = n * mw + (n - 1.0) * 2.0;
            let x0 = (w - total) / 2.0;
            for (k, l) in levels.iter().enumerate() {
                let mb = Bounds::new(
                    point(o.x + px(x0 + k as f32 * (mw + 2.0)), o.y + px(6.0)),
                    size(px(mw), px(hh - 12.0)),
                );
                paint_meter(mb, *l, window);
            }
            // the travel's scale: 0 dB line
            let y0 = o.y + px(6.0 + (1.0 - 0.75) * (hh - 12.0));
            window.paint_quad(fill(Bounds::new(point(o.x + px(4.0), y0), size(px(w - 8.0), px(1.0))), c(THEME.text3)));
            // the cap
            let cy = 6.0 + (1.0 - t as f32) * (hh - 12.0);
            let cap = Bounds::new(point(o.x + px(2.0), o.y + px(cy - 7.0)), size(px(w - 4.0), px(14.0)));
            window.paint_quad(gpui::quad(cap, px(3.0), c(THEME.fader_cap), px(1.0), c(THEME.line), Default::default()));
            window.paint_quad(fill(
                Bounds::new(point(cap.origin.x + px(3.0), o.y + px(cy)), size(px(w - 10.0), px(1.0))),
                c(THEME.text),
            ));
        },
    )
    .w(px(w))
    .h(px(h))
}

/// A modal's frame: title, body, and a close button. The workspace centers it over a dimmed window
#[expect(dead_code, reason = "read by the screens B, C and D port onto lm-gui next")]
pub fn modal_frame(
    id: impl Into<ElementId>,
    title: &'static str,
    body: AnyElement,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id.into())
        .min_w(px(420.))
        .max_w(px(900.))
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .rounded(px(10.))
        .bg(c(THEME.bg2))
        .border_1()
        .border_color(c(THEME.line2))
        .shadow_lg()
        .child(
            div()
                .flex()
                .items_center()
                .child(div().flex_1().text_size(px(16.)).font_weight(FontWeight::BOLD).child(tr(title)))
                .child(button("close", tr("Close"), false, 0).on_click(move |_, w, cx| on_close(w, cx))),
        )
        .child(body)
}

/// THE loading component (I-2.3): one for every screen. It reads the target's `Loading`; before 200 ms it draws
/// nothing, then the progress in place of the object, the cancel button after 2 s, and a failure in red.
pub fn loading_component(
    id: impl Into<ElementId>,
    loading: &Loading,
    on_cancel: impl Fn(&mut Window, &mut App) + 'static,
) -> Option<AnyElement> {
    match loading {
        Loading::Ready | Loading::Pending => None,
        Loading::Visible { progress, stage, cancellable, .. } => {
            let p = progress.unwrap_or(0.0);
            Some(
                div()
                    .id(id.into())
                    .size_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .bg(gpui::rgba(THEME.scrim))
                    .child(
                        div()
                            .w(px(160.))
                            .h(px(6.))
                            .rounded(px(3.))
                            .bg(c(THEME.bg4))
                            .child(div().h_full().w(px(160. * p)).rounded(px(3.)).bg(c(THEME.green))),
                    )
                    .child(div().text_size(px(11.)).text_color(c(THEME.text2)).child(trf(
                        "{0}… {1}%",
                        &[&stage.as_ref().map_or("", |l| tr(l.key)), &format!("{:.0}", p * 100.0)],
                    )))
                    .when(*cancellable, |d| {
                        d.child(button("cancel", tr("Cancel"), false, 0).on_click(move |_, w, cx| on_cancel(w, cx)))
                    })
                    .into_any_element(),
            )
        }
        Loading::Failed(e) => {
            Some(div().size_full().text_color(c(THEME.red)).child(tr(e.label.key)).into_any_element())
        }
        Loading::Missing { .. } => {
            Some(div().size_full().text_color(c(THEME.orange)).child(tr("Missing file: locate it")).into_any_element())
        }
    }
}
