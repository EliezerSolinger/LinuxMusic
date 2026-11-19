//! The mixing page: the whole desk, a page of its own (user's decision, 19/11). Channels exist by themselves; each
//! strip shows what is used while mixing (inserts, sends, pan, mute, solo, the fader over the meter, the output),
//! and the selected channel's analog rack opens in full in the inspector on the right, instead of squeezing
//! fifteen knobs into every strip. Returns and the master stay pinned at the right edge.

use std::time::Instant;

use fake_core::mixer::{self, ChannelKind};
use fake_core::view;
use fake_core::{Field, GestureId, NodeId, Value};
use gpui::{
    Context, Entity, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, ScrollEvent,
    SharedString, Window, div, prelude::*, px,
};

use crate::core::Core;
use crate::ui::{self, THEME, button, c, caption, fader_over_meter, knob_face, tr, track_color};

pub const STRIP_W: f32 = 92.0;
const INSPECTOR_W: f32 = 300.0;
const FADER_H: f32 = 200.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Volume(usize),
    Pan(usize),
    Param(usize, &'static str),
    Send(usize, NodeId),
}

struct Drag {
    target: Target,
    start_y: f32,
    start_t: f64,
    /// pixels for the whole travel: a fader's height, or a knob's 200 px
    travel: f32,
    gesture: GestureId,
}

pub struct MixingPage {
    core: Entity<Core>,
    scroll: f32,
    selected: usize,
    drag: Option<Drag>,
}

impl MixingPage {
    pub fn new(core: Entity<Core>, _cx: &mut Context<Self>) -> Self {
        MixingPage { core, scroll: 0.0, selected: 0, drag: None }
    }

    /// A target's value as 0..1 of its control's travel
    fn travel_of(&self, t: Target, cx: &Context<Self>) -> f64 {
        let chs = self.core.read(cx).session.channels();
        match t {
            Target::Volume(i) => view::db_to_knob(chs[i].volume_db),
            Target::Pan(i) => (chs[i].pan + 1.0) / 2.0,
            Target::Param(i, k) => {
                let p = mixer::spec(k).unwrap();
                (chs[i].params[k] - p.min) / (p.max - p.min)
            }
            Target::Send(i, s) => view::db_to_knob(chs[i].sends.iter().find(|x| x.id == s).map_or(-60.0, |x| x.level_db)),
        }
    }

    fn edit_of(t: Target, travel: f64) -> (Field, Value) {
        match t {
            Target::Volume(_) => (Field::Volume, Value::Real(view::knob_to_db(travel).max(-60.0))),
            Target::Pan(_) => (Field::Pan, Value::Real(travel * 2.0 - 1.0)),
            Target::Param(_, k) => {
                let p = mixer::spec(k).unwrap();
                (Field::Param(k), Value::Real(p.min + travel * (p.max - p.min)))
            }
            Target::Send(_, s) => (Field::SendLevel(s), Value::Real(view::knob_to_db(travel).max(-60.0))),
        }
    }

    fn channel_of(t: Target) -> usize {
        match t {
            Target::Volume(i) | Target::Pan(i) | Target::Param(i, _) | Target::Send(i, _) => i,
        }
    }

    fn begin(&mut self, t: Target, travel: f32, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let i = Self::channel_of(t);
        self.selected = i;
        if ev.click_count == 2 {
            // double click: back to the default (0 dB, center, the parameter's default)
            let default = match t {
                Target::Volume(_) | Target::Send(..) => view::db_to_knob(0.0),
                Target::Pan(_) => 0.5,
                Target::Param(_, k) => {
                    let p = mixer::spec(k).unwrap();
                    (p.default - p.min) / (p.max - p.min)
                }
            };
            let (f, v) = Self::edit_of(t, default);
            self.core.update(cx, |core, _| {
                let id = core.session.channels()[i].id;
                let _ = core.session.transact("Reset", |e| e.set(id, f, v));
            });
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let start_t = self.travel_of(t, cx);
        let prev = self.drag.take();
        let gesture = self.core.update(cx, |core, _| {
            if let Some(d) = prev {
                let _ = core.session.gesture_commit(d.gesture);
            }
            core.session.begin_gesture("Mix")
        });
        self.drag = Some(Drag { target: t, start_y: ev.position.y.as_f32(), start_t, travel, gesture });
        cx.stop_propagation();
        cx.notify();
    }

    fn drag_to(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.drag else { return };
        let fine = if ev.modifiers.shift { 10.0 } else { 1.0 };
        let t = (d.start_t - ((ev.position.y.as_f32() - d.start_y) / (d.travel * fine)) as f64).clamp(0.0, 1.0);
        let (f, v) = Self::edit_of(d.target, t);
        let i = Self::channel_of(d.target);
        let ok = self.core.update(cx, |core, _| {
            let id = core.session.channels()[i].id;
            core.session.gesture_update(&self.drag.as_ref().unwrap().gesture, |e| e.set(id, f, v)).is_ok()
        });
        if !ok {
            self.drag = None;
        }
        cx.notify();
    }

    fn end(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            self.core.update(cx, |core, _| {
                let _ = core.session.gesture_commit(d.gesture);
            });
            cx.notify();
        }
    }

    fn toggle(&mut self, i: usize, f: Field, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            let ch = &core.session.channels()[i];
            let v = match f {
                Field::Mute => Value::Bool(!ch.mute),
                Field::Solo => Value::Bool(!ch.solo),
                Field::Param(k) => Value::Real(if ch.params.get(k).copied().unwrap_or(0.0) > 0.5 { 0.0 } else { 1.0 }),
                _ => return,
            };
            let id = ch.id;
            let _ = core.session.transact("Toggle", |e| e.set(id, f, v));
        });
        cx.notify();
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let delta = if d.x.as_f32().abs() > d.y.as_f32().abs() { d.x } else { d.y }.as_f32();
        let normal = self.core.read(cx).session.channels().iter().filter(|c| c.kind == ChannelKind::Normal).count();
        let visible = window.viewport_size().width.as_f32() - INSPECTOR_W - 5.0 * STRIP_W;
        let max = (normal as f32 * STRIP_W - visible).max(0.0);
        self.scroll = (self.scroll - delta).clamp(0.0, max);
        cx.notify();
    }

    /// A knob bound to a target, with the gesture wired
    fn knob(&self, id: SharedString, t: Target, d: f32, cx: &Context<Self>) -> impl IntoElement {
        let travel = self.travel_of(t, cx);
        div()
            .id(id)
            .cursor_ns_resize()
            .child(knob_face(travel, d))
            .on_mouse_down(MouseButton::Left, cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.begin(t, 200.0, ev, cx)))
    }

    fn strip(&self, i: usize, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let s = &core.session;
        let ch = &s.channels()[i];
        let level = core.shown[s.channel_meter(i)];
        let color = match ch.kind {
            ChannelKind::Normal => track_color(i),
            ChannelKind::Return => THEME.cyan,
            ChannelKind::Group => THEME.purple,
            ChannelKind::Master => THEME.green,
        };
        let out_name: SharedString = match ch.output {
            None => tr("→ Device").into(),
            Some(o) => format!("→ {}", s.channels().iter().find(|x| x.id == o).map_or("?", |x| x.name.as_str())).into(),
        };
        let selected = i == self.selected;
        let fader_t = view::db_to_knob(ch.volume_db);
        div()
            .id(("strip", i))
            .w(px(STRIP_W - 2.0))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(4.))
            .pb(px(6.))
            .bg(c(if selected { THEME.bg3 } else { THEME.bg2 }))
            .border_r_1()
            .border_color(c(THEME.line))
            .on_mouse_down(MouseButton::Left, cx.listener(move |p, _, _, cx| {
                p.selected = i;
                cx.notify();
            }))
            // name, in the channel's color
            .child(
                div()
                    .h(px(20.))
                    .px(px(4.))
                    .bg(c(color))
                    .text_color(c(THEME.on_accent))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::BOLD)
                    .overflow_hidden()
                    .child(ch.name.clone()),
            )
            // the rack's sections, as switches; the knobs are in the inspector
            .child(
                div().flex().flex_wrap().gap(px(2.)).px(px(3.)).children(mixer::RACK.iter().map(|(sec, label, _)| {
                    let key: &'static str = Box::leak(format!("rack.{sec}.on").into_boxed_str());
                    let on = ch.params.get(key).copied().unwrap_or(0.0) > 0.5;
                    let short: String = tr(label).chars().take(3).collect();
                    button(SharedString::from(format!("rk{i}{sec}")), short, on, THEME.orange)
                        .on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Param(key), cx)))
                })),
            )
            // inserts
            .child(div().px(px(3.)).child(caption(tr("Inserts"))))
            .children(ch.inserts.iter().map(|ins| {
                div()
                    .mx(px(3.))
                    .px(px(4.))
                    .h(px(16.))
                    .rounded(px(3.))
                    .bg(c(THEME.bg1))
                    .text_size(px(10.))
                    .text_color(c(if ins.bypass { THEME.text3 } else { THEME.text }))
                    .overflow_hidden()
                    .child(tr(ins.plugin))
            }))
            .child(div().mx(px(3.)).h(px(16.)).rounded(px(3.)).border_1().border_color(c(THEME.line2)).text_size(px(10.)).text_color(c(THEME.text3)).flex().justify_center().child("+"))
            // sends
            .when(!ch.sends.is_empty(), |d| d.child(div().px(px(3.)).child(caption(tr("Sends")))))
            .children(ch.sends.iter().map(|snd| {
                let to = s.channels().iter().find(|x| x.id == snd.to).map_or("?", |x| x.name.as_str()).to_string();
                let sid = snd.id;
                div()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .px(px(3.))
                    .child(self.knob(SharedString::from(format!("snd{i}-{}", sid.number)), Target::Send(i, sid), 16.0, cx))
                    .child(div().flex_1().text_size(px(9.5)).overflow_hidden().child(to))
                    .when(snd.pre_fader, |d| d.child(div().text_size(px(8.5)).text_color(c(THEME.yellow)).child(tr("PRE"))))
            }))
            .child(div().flex_1())
            // pan, mute, solo
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(4.))
                    .child(self.knob(SharedString::from(format!("pan{i}")), Target::Pan(i), 22.0, cx))
                    .child(button(SharedString::from(format!("mm{i}")), "M", ch.mute, THEME.orange).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Mute, cx))))
                    .child(button(SharedString::from(format!("ms{i}")), "S", ch.solo, THEME.yellow).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Solo, cx)))),
            )
            // the fader over the meter (user's decision): drag = gesture, double click = 0 dB
            .child(
                div()
                    .id(("fader", i))
                    .debug_selector(move || format!("fader-{i}"))
                    .mx_auto()
                    .cursor_ns_resize()
                    .child(fader_over_meter(fader_t, vec![level, (level * 0.92).min(1.0)], 40.0, FADER_H))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.begin(Target::Volume(i), FADER_H - 12.0, ev, cx)),
                    ),
            )
            .child(div().text_size(px(10.)).text_color(c(THEME.text2)).mx_auto().child(view::format_db(ch.volume_db)))
            .child(div().text_size(px(9.5)).text_color(c(THEME.text3)).mx_auto().child(out_name))
    }

    /// The selected channel's rack in full: every section with its switch and its knobs
    fn inspector(&self, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let i = self.selected.min(core.session.channels().len() - 1);
        let ch = &core.session.channels()[i];
        div()
            .id("inspector")
            .w(px(INSPECTOR_W))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .bg(c(THEME.bg1))
            .border_l_1()
            .border_color(c(THEME.line))
            .overflow_y_scroll()
            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(ch.name.clone()))
            .child(caption(tr("Analog desk rack")))
            .children(mixer::RACK.iter().map(|(sec, label, params)| {
                let key: &'static str = Box::leak(format!("rack.{sec}.on").into_boxed_str());
                let on = ch.params.get(key).copied().unwrap_or(0.0) > 0.5;
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(8.))
                    .rounded(px(6.))
                    .bg(c(THEME.bg2))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(div().flex_1().font_weight(FontWeight::BOLD).text_size(px(12.)).child(tr(label)))
                            .child(
                                button(SharedString::from(format!("ion{sec}")), if on { tr("On") } else { tr("Off") }, on, THEME.orange)
                                    .on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Param(key), cx))),
                            ),
                    )
                    .child(div().flex().flex_wrap().gap(px(10.)).children(params.iter().map(|ps| {
                        let v = ch.params[ps.key];
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(2.))
                            .child(self.knob(SharedString::from(format!("ik{}", ps.key)), Target::Param(i, ps.key), 28.0, cx))
                            .child(div().text_size(px(9.5)).text_color(c(THEME.text2)).child(tr(ps.label)))
                            .child(div().text_size(px(9.5)).child(format!("{v:.1} {}", ps.unit)))
                    })))
            }))
    }
}

impl Render for MixingPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = Instant::now();
        let chs = self.core.read(cx).session.channels();
        let normal: Vec<usize> = (0..chs.len()).filter(|&k| chs[k].kind == ChannelKind::Normal).collect();
        let pinned: Vec<usize> = (0..chs.len()).filter(|&k| chs[k].kind != ChannelKind::Normal).collect();
        let first = (self.scroll / STRIP_W).floor() as usize;
        // a strip is built only when visible (an upper bound of 24 strips on any screen width here)
        let visible: Vec<usize> = normal.iter().copied().skip(first).take(24).collect();
        let offset = -(self.scroll - first as f32 * STRIP_W);
        let _ = ui::trf;
        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(c(THEME.bg1))
            .on_mouse_move(cx.listener(|p, ev: &MouseMoveEvent, _, cx| p.drag_to(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|p, _: &MouseUpEvent, _, cx| p.end(cx)))
            .child(
                div()
                    .id("strips")
                    .flex_1()
                    .h_full()
                    .relative()
                    .overflow_hidden()
                    .on_scroll(cx.listener(Self::on_scroll))
                    .child(
                        div()
                            .absolute()
                            .left(px(offset))
                            .top(px(0.))
                            .h_full()
                            .flex()
                            .flex_row()
                            .children(visible.into_iter().map(|i| self.strip(i, cx))),
                    ),
            )
            .child(
                div()
                    .h_full()
                    .flex()
                    .flex_row()
                    .border_l_2()
                    .border_color(c(THEME.line))
                    .children(pinned.into_iter().map(|i| self.strip(i, cx))),
            )
            .child(self.inspector(cx))
    }
}
