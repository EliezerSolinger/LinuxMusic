//! The transport bar (design 03): one big display with the position in bars, a smaller clock, the state in a word
//! and by color, tempo, signature, metronome and pre-roll. It is the workspace's, over both pages.

use fake_core::SAMPLE_RATE;
use fake_core::view;
use gpui::{Context, FontWeight, IntoElement, div, prelude::*, px};

use crate::ui::{THEME, c, tbutton, tr, trf};
use crate::workspace::Workspace;

pub fn render(ws: &Workspace, cursor: f64, cx: &Context<Workspace>) -> impl IntoElement {
    let pos = ws.core.read(cx).session.position();
    let (word, word_color, lcd_border) = if pos.recording {
        (tr("Recording"), THEME.red, THEME.red)
    } else if pos.playing {
        (tr("Playing"), THEME.green, THEME.green)
    } else {
        (tr("Stopped"), THEME.text3, THEME.line)
    };
    div()
        .h(px(view::TRANSPORT_H))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .px(px(12.))
        .bg(c(THEME.bg2))
        .border_b_1()
        .border_color(c(THEME.line))
        .child(tbutton("rew", "⏮", false, 0).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| core.session.locate(0));
            cx.notify();
        })))
        .child(tbutton("stop", "■", false, 0).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| core.session.stop());
            cx.notify();
        })))
        .child(tbutton("play", "▶", pos.playing, THEME.green).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| core.session.play());
            cx.notify();
        })))
        .child(tbutton("rec", "●", pos.recording, THEME.red).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| core.session.toggle_record());
            cx.notify();
        })))
        .child(tbutton("loop", "⟲", true, THEME.green))
        .child(
            div()
                .ml(px(6.))
                .px(px(10.))
                .py(px(2.))
                .rounded(px(6.))
                .bg(c(THEME.lcd_bg))
                .border_1()
                .border_color(c(lcd_border))
                .font_family("Menlo")
                .text_size(px(26.))
                .text_color(c(THEME.green))
                .child(view::format_bbt(cursor, SAMPLE_RATE as f64)),
        )
        .child(
            div()
                .px(px(8.))
                .py(px(4.))
                .rounded(px(5.))
                .bg(c(THEME.lcd_bg))
                .font_family("Menlo")
                .text_size(px(14.))
                .text_color(c(THEME.text2))
                .child(view::format_clock(cursor, SAMPLE_RATE as f64)),
        )
        .child(div().w(px(90.)).text_size(px(11.)).font_weight(FontWeight::BOLD).text_color(c(word_color)).child(word))
        .child(tbutton("tempo", "♩ 120,00", false, 0))
        .child(tbutton("sig", "4/4", false, 0))
        .child(tbutton("metro", "◷", true, THEME.green))
        .child(tbutton("preroll", trf("Pre-roll {0}", &[&1]), false, 0))
        .child(div().flex_1())
        .child(tbutton("reload", tr("Reload take"), false, 0).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| core.session.reload_take(2));
            cx.notify();
        })))
}
