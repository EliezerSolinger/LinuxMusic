//! The transport bar (design 03): one big display with the position, a smaller clock, the state in a word and by
//! the display's border color (stopped, playing, recording, count-in), tempo editable in place (drag, wheel,
//! double click back to 120), signature, metronome, count-in, pre-roll, loop and punch. It is the workspace's,
//! over both pages.

use fake_core::view;
use fake_core::port::EditOp;
use fake_core::{Field, GestureId, ROOT, SAMPLE_RATE, Value};
use gpui::{Context, FontWeight, IntoElement, MouseButton, MouseDownEvent, ScrollEvent, div, prelude::*, px};

use crate::ui::{THEME, c, tbutton, tr, trf};
use crate::workspace::Workspace;

/// The big display's formats (design 03, item 5: chosen in one click)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DisplayFormat {
    #[default]
    Bars,
    Time,
    Samples,
    Timecode,
}

impl DisplayFormat {
    fn next(self) -> Self {
        match self {
            DisplayFormat::Bars => DisplayFormat::Time,
            DisplayFormat::Time => DisplayFormat::Samples,
            DisplayFormat::Samples => DisplayFormat::Timecode,
            DisplayFormat::Timecode => DisplayFormat::Bars,
        }
    }
    fn format(self, sample: f64) -> String {
        let sr = SAMPLE_RATE as f64;
        match self {
            DisplayFormat::Bars => view::format_bbt(sample, sr),
            DisplayFormat::Time => view::format_clock(sample, sr),
            DisplayFormat::Samples => format!("{:>10}", sample.max(0.0) as i64),
            DisplayFormat::Timecode => {
                let s = sample.max(0.0) / sr;
                let f = ((s.fract()) * 25.0).floor();
                format!("{:02}:{:02}:{:02}:{:02}", (s / 3600.0) as i64, (s / 60.0) as i64 % 60, s as i64 % 60, f as i64)
            }
        }
    }
}

/// The tempo's drag gesture
pub struct TempoDrag {
    pub start_y: f32,
    pub start_bpm: f64,
    pub gesture: GestureId,
}

fn set_song(ws: &mut Workspace, f: Field, v: Value, cx: &mut Context<Workspace>) {
    ws.core.update(cx, |core, _| {
        let _ = core.port_mut().apply("Transport", vec![EditOp::set(ROOT, f, v)]);
    });
    cx.notify();
}

pub fn begin_tempo(ws: &mut Workspace, ev: &MouseDownEvent, cx: &mut Context<Workspace>) {
    if ev.click_count == 2 {
        set_song(ws, Field::Tempo, Value::Real(120.0), cx);
        return;
    }
    let start_bpm = ws.core.read(cx).port().root().song.tempo[0].bpm;
    let gesture = ws.core.update(cx, |core, _| core.port_mut().begin_gesture("Tempo"));
    ws.tempo_drag = Some(TempoDrag { start_y: ev.position.y.as_f32(), start_bpm, gesture });
}

pub fn drag_tempo(ws: &mut Workspace, y: f32, fine: bool, cx: &mut Context<Workspace>) {
    let Some(d) = &ws.tempo_drag else { return };
    let bpm = ((d.start_bpm - (y - d.start_y) as f64 * if fine { 0.05 } else { 0.5 }) * 100.0).round() / 100.0;
    ws.core.update(cx, |core, _| {
        core.port_mut().gesture_apply(&ws.tempo_drag.as_ref().unwrap().gesture, vec![EditOp::set(ROOT, Field::Tempo, Value::Real(bpm))])
    });
    cx.notify();
}

pub fn end_tempo(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    if let Some(d) = ws.tempo_drag.take() {
        ws.core.update(cx, |core, _| core.port_mut().gesture_commit(d.gesture));
    }
}

pub fn render(ws: &Workspace, cursor: f64, cx: &Context<Workspace>) -> impl IntoElement {
    let port = ws.core.read(cx).port();
    let tele = port.telemetry();
    let pos = tele.position;
    let song = &port.root().song;
    let counting = tele.counting_in;
    let (word, word_color, border) = if counting {
        (trf("Count-in {0}…", &[&(((cursor.max(0.0) / fake_core::arrange::BAR as f64) as i64) % 4 + 1)]), THEME.yellow, THEME.yellow)
    } else if pos.recording {
        (tr("Recording").to_string(), THEME.red, THEME.red)
    } else if pos.playing {
        (tr("Playing").to_string(), THEME.green, THEME.green)
    } else {
        (tr("Stopped").to_string(), THEME.text3, THEME.line)
    };
    let bpm = song.tempo[0].bpm;
    let (loop_on, punch_on, metro, count_in, pre_roll) =
        (song.loop_range.on, song.punch.on, song.metronome, song.count_in_bars, song.pre_roll_bars);
    let other = if ws.display_format == DisplayFormat::Bars { DisplayFormat::Time } else { DisplayFormat::Bars };
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
            ws.core.update(cx, |core, _| { let _ = core.port_mut().run_action("transport.locate", Value::Real(0.0)); });
            cx.notify();
        })))
        .child(tbutton("stop", "■", false, 0).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| { let _ = core.port_mut().run_action("transport.stop", Value::Bool(true)); });
            cx.notify();
        })))
        .child(tbutton("play", "▶", pos.playing && !pos.recording, THEME.green).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| { let _ = core.port_mut().run_action("transport.play", Value::Bool(true)); });
            cx.notify();
        })))
        .child(tbutton("rec", "●", pos.recording, THEME.red).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| { let _ = core.port_mut().run_action("transport.record", Value::Bool(true)); });
            cx.notify();
        })))
        .child(tbutton("loop", "⟲", loop_on, THEME.green).on_click(cx.listener(move |ws, _, _, cx| set_song(ws, Field::LoopOn, Value::Bool(!loop_on), cx))))
        .child(tbutton("punch", tr("Punch"), punch_on, THEME.red).on_click(cx.listener(move |ws, _, _, cx| set_song(ws, Field::PunchOn, Value::Bool(!punch_on), cx))))
        // the big display: a click changes the format
        .child(
            div()
                .id("lcd")
                .ml(px(6.))
                .px(px(10.))
                .py(px(2.))
                .min_w(px(170.))
                .rounded(px(6.))
                .bg(c(THEME.lcd_bg))
                .border_1()
                .border_color(c(border))
                .font_family("Menlo")
                .text_size(px(26.))
                .text_color(c(THEME.green))
                .cursor_pointer()
                .child(ws.display_format.format(cursor))
                .on_click(cx.listener(|ws, _, _, cx| {
                    ws.display_format = ws.display_format.next();
                    cx.notify();
                })),
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
                .child(other.format(cursor)),
        )
        .child(div().w(px(110.)).text_size(px(11.)).font_weight(FontWeight::BOLD).text_color(c(word_color)).child(word))
        // tempo, editable in place
        .child(
            tbutton("tempo", format!("♩ {bpm:.2}"), ws.tempo_drag.is_some(), THEME.blue)
                .cursor_ns_resize()
                .on_mouse_down(MouseButton::Left, cx.listener(|ws, ev: &MouseDownEvent, _, cx| begin_tempo(ws, ev, cx)))
                .on_scroll(cx.listener(|ws, ev: &ScrollEvent, _, cx| {
                    let d = ev.delta.pixel_delta(px(16.0)).y.as_f32();
                    let bpm = ws.core.read(cx).port().root().song.tempo[0].bpm + if d > 0.0 { 1.0 } else if d < 0.0 { -1.0 } else { 0.0 };
                    set_song(ws, Field::Tempo, Value::Real(bpm), cx);
                })),
        )
        .child(tbutton("sig", format!("{}/{}", song.signature.0, song.signature.1), false, 0))
        .child(tbutton("metro", "◷", metro, THEME.green).on_click(cx.listener(move |ws, _, _, cx| set_song(ws, Field::Metronome, Value::Bool(!metro), cx))))
        .child(
            tbutton("countin", trf("Count-in {0}", &[&count_in]), count_in > 0, THEME.yellow)
                .on_click(cx.listener(move |ws, _, _, cx| set_song(ws, Field::CountIn, Value::Real(((count_in + 1) % 3) as f64), cx))),
        )
        .child(
            tbutton("preroll", trf("Pre-roll {0}", &[&pre_roll]), pre_roll > 0, THEME.yellow)
                .on_click(cx.listener(move |ws, _, _, cx| set_song(ws, Field::PreRoll, Value::Real(((pre_roll + 1) % 3) as f64), cx))),
        )
        // the global transposition (P-37): click up, Alt+click down, double click back to 0
        .child(
            tbutton("transpose", trf("Transpose {0}", &[&format!("{:+}", song.transpose)]), song.transpose != 0, THEME.purple).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |ws, ev: &MouseDownEvent, _, cx| {
                    let t = ws.core.read(cx).port().root().song.transpose;
                    let v = if ev.click_count == 2 { 0 } else if ev.modifiers.alt { t - 1 } else { t + 1 };
                    set_song(ws, Field::Transpose, Value::Real(v as f64), cx);
                }),
            ),
        )
        // the xruns, always visible (P-14)
        .child(
            div()
                .px(px(6.))
                .text_size(px(11.))
                .text_color(c(if tele.xruns > 0 { THEME.red } else { THEME.text3 }))
                .child(trf("xruns {0}", &[&tele.xruns])),
        )
        .child(div().flex_1())
        .child(tbutton("reload", tr("Reload take"), false, 0).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| { let _ = core.port_mut().run_action("take.reload", Value::Real(2.0)); });
            cx.notify();
        })))
}
