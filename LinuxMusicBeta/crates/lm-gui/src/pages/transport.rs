//! The transport bar (design 03): one big display with the position (its format chosen in one click), a smaller
//! clock, the state in a word and by the display's border color (stopped, playing, recording, count-in), tempo
//! editable in place (drag = one gesture, wheel = 1 BPM, double click back to 120), signature, metronome, count-in,
//! pre-roll, loop and punch, the global transposition (P-37) and the xrun count, always shown (P-14). It is the
//! workspace's, over both pages. Everything through the port: the song's fields as `EditOp`s on the root, the
//! transport as actions.

use gpui::{Context, FontWeight, IntoElement, MouseButton, MouseDownEvent, ScrollEvent, div, prelude::*, px};
use lm_gui_plugins::Gesture;
use lm_port::{EditOp, HostTime, Label, Position, ROOT, Value};

use crate::keymap;
use crate::model;
use crate::ui::{THEME, c, icon, tbutton, tr, trf};
use crate::view;
use crate::workspace::Workspace;

/// The big display's formats (design 03, item 5)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DisplayFormat {
    #[default]
    Bars,
    Time,
    Samples,
    Timecode,
}

impl DisplayFormat {
    pub fn next(self) -> Self {
        match self {
            DisplayFormat::Bars => DisplayFormat::Time,
            DisplayFormat::Time => DisplayFormat::Samples,
            DisplayFormat::Samples => DisplayFormat::Timecode,
            DisplayFormat::Timecode => DisplayFormat::Bars,
        }
    }
}

/// The tempo's drag gesture
pub struct TempoDrag {
    pub start_y: f32,
    pub start_bpm: f64,
    pub gesture: Gesture,
}

fn bpm_text(bpm: f64) -> Value {
    Value::text(format!("{:.2}", bpm.clamp(20.0, 400.0)).trim_end_matches('0').trim_end_matches('.').to_string())
}

fn first_bpm(root: &Value) -> f64 {
    model::list(root, "tempoMap")
        .next()
        .and_then(|p| p.get("bpm"))
        .and_then(Value::as_text)
        .and_then(|t| t.parse().ok())
        .unwrap_or(120.0)
}

/// The first tempo point's bpm, as an edit of the root (the map's first point)
fn tempo_op(bpm: f64) -> EditOp {
    EditOp::Set {
        id: lm_port::Ref::Id(ROOT),
        path: vec![lm_port::PathStep::key("tempoMap"), lm_port::PathStep::Index(0), lm_port::PathStep::key("bpm")],
        value: bpm_text(bpm),
    }
}

pub fn set_song(ws: &mut Workspace, path: &[&str], v: Value, cx: &mut Context<Workspace>) {
    let op = EditOp::set(ROOT, path, v);
    ws.core.update(cx, |core, _| {
        core.report(|pt| pt.apply(Label::new("Transport"), vec![op]));
    });
    cx.notify();
}

pub fn toggle_loop(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let on = model::song(ws.core.read(cx).port().root()).loop_on;
    set_song(ws, &["transport", "loop", "on"], Value::bool(!on), cx);
}

/// Stopped: count-in and record; playing: punch in, and again to punch out (F-REC-006); with nothing armed, the
/// selected item's track is armed (F-REC-021)
pub fn record(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let track = {
        let root = ws.core.read(cx).port().root();
        ws.composition.read(cx).selected_track(root)
    };
    let param = track.map_or(Value::NULL, |t| Value::text(t.to_string()));
    ws.core.update(cx, |core, _| {
        core.report(|pt| pt.run_action("transport.record".into(), param));
    });
    cx.notify();
}

/// Tap Tempo (F-JAN-023): the tempo from the taps, written to the tempo map's first point
pub fn tap_tempo(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let now = ws.tap_clock.elapsed().as_secs_f64();
    if let Some(bpm) = ws.tap.tap(now) {
        let bpm = (bpm * 100.0).round() / 100.0;
        ws.core.update(cx, |core, _| {
            core.report(|pt| pt.apply(Label::new("Tempo"), vec![tempo_op(bpm)]));
        });
    }
    cx.notify();
}

/// The signature's numerator and denominator, as the first point of the tempo map has them
pub fn set_signature(ws: &mut Workspace, numerator: i64, denominator: i64, cx: &mut Context<Workspace>) {
    let (n, d) = clamp_signature(numerator, denominator);
    let op = EditOp::Set {
        id: lm_port::Ref::Id(ROOT),
        path: vec![lm_port::PathStep::key("tempoMap"), lm_port::PathStep::Index(0), lm_port::PathStep::key("timeSig")],
        value: Value::list([Value::int(n), Value::int(d)]),
    };
    ws.core.update(cx, |core, _| {
        core.report(|pt| pt.apply(Label::new("Time signature"), vec![op]));
    });
    cx.notify();
}

/// A valid signature: 1 to 32 beats of a power of two from 1 to 32
pub fn clamp_signature(numerator: i64, denominator: i64) -> (i64, i64) {
    let d = [1, 2, 4, 8, 16, 32].into_iter().min_by_key(|p| (p - denominator).abs()).unwrap_or(4);
    (numerator.clamp(1, 32), d)
}

/// The next denominator up or down the powers of two
pub fn step_denominator(d: i64, up: bool) -> i64 {
    if up { (d * 2).min(32) } else { (d / 2).max(1) }
}

/// Mutes or unmutes the master channel (F-JAN-049, F-MIX-015)
pub fn toggle_master_mute(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let master =
        model::channels(ws.core.read(cx).port().root()).into_iter().find(|c| c.kind == model::ChannelKind::Master);
    let Some(m) = master else { return };
    let op = EditOp::set(m.id, &["mute"], Value::bool(!m.mute));
    ws.core.update(cx, |core, _| {
        core.report(|pt| pt.apply(Label::new("Mute"), vec![op]));
    });
    cx.notify();
}

/// A small level bar of the transport (CPU, OUT): red above 95 %
fn level_bar(id: &'static str, label: &'static str, fraction: f32) -> impl IntoElement {
    let f = fraction.clamp(0.0, 1.0);
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().text_size(px(9.)).text_color(c(THEME.text3)).child(tr(label)))
        .child(div().w(px(44.)).h(px(5.)).rounded(px(2.)).bg(c(THEME.lcd_bg)).child(
            div().h_full().w(gpui::relative(f)).rounded(px(2.)).bg(c(if f > 0.95 { THEME.red } else { THEME.green })),
        ))
}

pub fn begin_tempo(ws: &mut Workspace, ev: &MouseDownEvent, cx: &mut Context<Workspace>) {
    if ev.click_count == 2 {
        ws.core.update(cx, |core, _| {
            core.report(|pt| pt.apply(Label::new("Tempo"), vec![tempo_op(120.0)]));
        });
        cx.notify();
        return;
    }
    let start_bpm = first_bpm(ws.core.read(cx).port().root());
    let gesture = ws.core.update(cx, |core, _| core.begin_gesture(Label::new("Tempo")));
    ws.tempo_drag = Some(TempoDrag { start_y: ev.position.y.as_f32(), start_bpm, gesture });
}

pub fn drag_tempo(ws: &mut Workspace, y: f32, fine: bool, cx: &mut Context<Workspace>) {
    let Some(d) = &ws.tempo_drag else { return };
    let bpm = ((d.start_bpm - (y - d.start_y) as f64 * if fine { 0.05 } else { 0.5 }) * 100.0).round() / 100.0;
    ws.core.update(cx, |core, _| {
        core.gesture_apply(ws.tempo_drag.as_ref().unwrap().gesture, vec![tempo_op(bpm)])
    });
    cx.notify();
}

pub fn end_tempo(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    if let Some(d) = ws.tempo_drag.take() {
        ws.core.update(cx, |core, _| core.gesture_commit(d.gesture));
        cx.notify();
    }
}

pub fn render(ws: &Workspace, position: Position, cx: &Context<Workspace>) -> impl IntoElement {
    let port = ws.core.read(cx).port();
    let root = port.root();
    let tele = port.telemetry();
    let map = model::tempo_map(root);
    let song = model::song(root);
    let rate = model::sample_rate(root);
    let samples = position.at(HostTime::now());
    let tick = map.tick_at(samples as i64);
    let format = |f: DisplayFormat| match f {
        DisplayFormat::Bars => view::format_bbt(&map, tick),
        DisplayFormat::Time => view::format_clock(samples, rate),
        DisplayFormat::Samples => format!("{:>10}", samples.max(0.0) as i64),
        DisplayFormat::Timecode => view::format_timecode(samples, rate),
    };
    let (word, word_color, border) = if tele.counting_in {
        (tr("Count-in").to_string(), THEME.yellow, THEME.yellow)
    } else if position.recording {
        (tr("Recording").to_string(), THEME.red, THEME.red)
    } else if position.playing {
        (tr("Playing").to_string(), THEME.green, THEME.green)
    } else {
        (tr("Stopped").to_string(), THEME.text3, THEME.line)
    };
    let bpm = first_bpm(root);
    let sig = map.time_signature_at(tick);
    let other = if ws.display_format == DisplayFormat::Bars { DisplayFormat::Time } else { DisplayFormat::Bars };
    let master = model::channels(root).into_iter().find(|c| c.kind == model::ChannelKind::Master);
    let master_muted = master.as_ref().is_some_and(|m| m.mute);
    let out_level = master.as_ref().map_or(0.0, |m| ws.core.read(cx).meter(m.id));
    let load = tele.load;
    let (sig_n, sig_d) = (sig.numerator() as i64, sig.denominator() as i64);
    let sig_open = ws.sig_editor;
    let (loop_on, punch_on, metro, count_in, pre_roll, transpose) =
        (song.loop_on, song.punch_on, song.metronome, song.count_in, song.pre_roll, song.transpose);
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
        .child(tbutton("rew", "", false, 0).child(icon("back_to_start")).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| {
                core.report(|pt| pt.run_action("transport.locate".into(), Value::int(0)));
            });
            cx.notify();
        })))
        .child(tbutton("stop", "", false, 0).child(icon("stop")).on_click(cx.listener(|ws, _, _, cx| {
            ws.core.update(cx, |core, _| {
                core.report(|pt| pt.run_action("transport.stop".into(), Value::NULL));
            });
            cx.notify();
        })))
        .child(tbutton("play", "", position.playing && !position.recording, THEME.green).child(icon("play")).on_click(
            cx.listener(|ws, _, _, cx| {
                ws.core.update(cx, |core, _| {
                    core.report(|pt| pt.run_action("transport.play".into(), Value::NULL));
                });
                cx.notify();
            }),
        ))
        .child(
            tbutton("rec", "", position.recording, THEME.red)
                .child(icon("record"))
                .on_click(cx.listener(|ws, _, _, cx| record(ws, cx))),
        )
        .child(
            tbutton("loop", "", loop_on, THEME.green)
                .child(icon(if loop_on { "loop_points_on" } else { "loop_points_off" }))
                .on_click(cx.listener(|ws, _, _, cx| toggle_loop(ws, cx))),
        )
        .child(tbutton("punch", tr("Punch"), punch_on, THEME.red).on_click(
            cx.listener(move |ws, _, _, cx| set_song(ws, &["transport", "punch", "on"], Value::bool(!punch_on), cx)),
        ))
        .child(
            div()
                .id("lcd")
                .debug_selector(|| "lcd".into())
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
                .child(format(ws.display_format))
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
                .child(format(other)),
        )
        .child(div().w(px(110.)).text_size(px(11.)).font_weight(FontWeight::BOLD).text_color(c(word_color)).child(word))
        .child(
            tbutton("tempo", format!("♩ {bpm:.2}"), ws.tempo_drag.is_some(), THEME.blue)
                .debug_selector(|| "tempo".into())
                .cursor_ns_resize()
                .on_mouse_down(MouseButton::Left, cx.listener(|ws, ev: &MouseDownEvent, _, cx| begin_tempo(ws, ev, cx)))
                .on_scroll(cx.listener(|ws, ev: &ScrollEvent, _, cx| {
                    let d = ev.delta.pixel_delta(px(16.0)).y.as_f32();
                    let bpm = first_bpm(ws.core.read(cx).port().root()) + d.signum() as f64;
                    ws.core.update(cx, |core, _| {
                        core.report(|pt| pt.apply(Label::new("Tempo"), vec![tempo_op(bpm)]));
                    });
                    cx.notify();
                })),
        )
        .child(
            tbutton("sig", format!("{sig_n}/{sig_d}"), sig_open, THEME.blue).debug_selector(|| "sig".into()).on_click(
                cx.listener(|ws, _, _, cx| {
                    ws.sig_editor = !ws.sig_editor;
                    cx.notify();
                }),
            ),
        )
        .when(sig_open, |d| {
            d.child(
                tbutton("sig-n-", "-", false, 0)
                    .on_click(cx.listener(move |ws, _, _, cx| set_signature(ws, sig_n - 1, sig_d, cx))),
            )
            .child(
                tbutton("sig-n+", "+", false, 0)
                    .on_click(cx.listener(move |ws, _, _, cx| set_signature(ws, sig_n + 1, sig_d, cx))),
            )
            .child(div().text_size(px(12.)).text_color(c(THEME.text3)).child("/"))
            .child(tbutton("sig-d-", "-", false, 0).on_click(
                cx.listener(move |ws, _, _, cx| set_signature(ws, sig_n, step_denominator(sig_d, false), cx)),
            ))
            .child(
                tbutton("sig-d+", "+", false, 0).on_click(
                    cx.listener(move |ws, _, _, cx| set_signature(ws, sig_n, step_denominator(sig_d, true), cx)),
                ),
            )
        })
        .child(
            tbutton("tap", tr("Tap"), ws.tap.count() > 0, THEME.blue)
                .debug_selector(|| "tap".into())
                .on_click(cx.listener(|ws, _, _, cx| tap_tempo(ws, cx))),
        )
        // the global automation button (I-21.5): "Lanes" lets each lane use its own mode
        .child({
            let now = model::at(root, &["transport", "automationMode"])
                .and_then(Value::as_text)
                .unwrap_or("lanes")
                .to_string();
            let (label, color) = match now.as_str() {
                "off" => ("Auto: Off", THEME.text3),
                "read" => ("Auto: Read", THEME.green),
                "touch" => ("Auto: Touch", THEME.yellow),
                "latch" => ("Auto: Latch", THEME.orange),
                "write" => ("Auto: Write", THEME.red),
                _ => ("Auto: Lanes", THEME.blue),
            };
            tbutton("automation", tr(label), now != "lanes", color)
                .debug_selector(|| "automation-mode".into())
                .on_click(cx.listener(move |ws, _, _, cx| {
                    let next = match now.as_str() {
                        "lanes" => "off",
                        "off" => "read",
                        "read" => "touch",
                        "touch" => "latch",
                        "latch" => "write",
                        _ => "lanes",
                    };
                    set_song(ws, &["transport", "automationMode"], Value::text(next), cx)
                }))
        })
        // the hand won somewhere: re-arm every lane back to its curve
        .when(!tele.hand_won.is_empty(), |d| {
            d.child(tbutton("rearm", tr("Re-arm"), true, THEME.yellow).debug_selector(|| "rearm-all".into()).on_click(
                cx.listener(|ws, _, _, cx| {
                    ws.core.update(cx, |core, _| {
                        core.report(|pt| pt.run_action("automation.rearm".into(), Value::empty_object()));
                    });
                    cx.notify();
                }),
            ))
        })
        .child(tbutton("metro", "", metro, THEME.green).child(icon("metronome")).on_click(
            cx.listener(move |ws, _, _, cx| set_song(ws, &["transport", "metronome", "on"], Value::bool(!metro), cx)),
        ))
        .child(tbutton("countin", trf("Count-in {0}", &[&count_in]), count_in > 0, THEME.yellow).on_click(cx.listener(
            move |ws, _, _, cx| {
                set_song(ws, &["transport", "metronome", "countIn"], Value::int((count_in + 1) % 3), cx)
            },
        )))
        .child(tbutton("preroll", trf("Pre-roll {0}", &[&pre_roll]), pre_roll > 0, THEME.yellow).on_click(cx.listener(
            move |ws, _, _, cx| set_song(ws, &["transport", "preRoll", "bars"], Value::int((pre_roll + 1) % 3), cx),
        )))
        // the global transposition (P-37): click up, Alt+click down, double click back to 0
        .child(
            tbutton("transpose", trf("Transpose {0}", &[&format!("{transpose:+}")]), transpose != 0, THEME.purple)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |ws, ev: &MouseDownEvent, _, cx| {
                        let v = if keymap::fires(
                            "Transport",
                            "transport.transpose_reset",
                            ev.button,
                            ev.click_count,
                            &ev.modifiers,
                        ) {
                            0
                        } else if keymap::holds("Transport", "transport.transpose_down", &ev.modifiers) {
                            transpose - 1
                        } else {
                            transpose + 1
                        };
                        set_song(ws, &["transport", "transpose"], Value::int(v.clamp(-12, 12)), cx);
                    }),
                ),
        )
        .child(
            div()
                .px(px(6.))
                .text_size(px(11.))
                .text_color(c(if tele.xruns > 0 { THEME.red } else { THEME.text3 }))
                .child(trf("xruns {0}", &[&tele.xruns])),
        )
        .child(div().flex_1())
        .child(level_bar("cpu", "CPU", load))
        .child(level_bar("out", "OUT", out_level))
        .child(
            tbutton("master-mute", tr("Master"), master_muted, THEME.red)
                .debug_selector(|| "master-mute".into())
                .on_click(cx.listener(|ws, _, _, cx| toggle_master_mute(ws, cx))),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signature_is_kept_valid() {
        assert_eq!(clamp_signature(0, 4), (1, 4));
        assert_eq!(clamp_signature(40, 4), (32, 4));
        assert_eq!(clamp_signature(7, 7), (7, 8));
        assert_eq!(clamp_signature(3, 100), (3, 32));
    }

    #[test]
    fn the_denominator_steps_along_the_powers_of_two() {
        assert_eq!(step_denominator(4, true), 8);
        assert_eq!(step_denominator(4, false), 2);
        assert_eq!(step_denominator(32, true), 32);
        assert_eq!(step_denominator(1, false), 1);
    }
}
