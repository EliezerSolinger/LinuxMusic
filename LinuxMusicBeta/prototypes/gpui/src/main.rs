//! The GUI prototype in GPUI (interfaces.md, I-15), against `fake-core`.
//!
//! How the core fits GPUI:
//! - the `!Send` `Session` lives in the one view entity (`Daw`), so it never leaves the main thread and every event
//!   handler reaches it through `cx.listener` with `&mut Daw`: no `Rc<RefCell<>>`;
//! - the pump is a task on GPUI's foreground executor, on a 10 ms timer that does not depend on frames (H2): it pumps,
//!   and asks for a frame (`cx.notify`) only when something the UI shows changed;
//! - feeds are drained in `render` (I-3.6); the loading state is read per item each frame (I-2);
//! - while the transport plays, `request_animation_frame` keeps one frame per display refresh for the cursor, which
//!   is interpolated from the last telemetry frame and its host time (I-7.2).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fake_core::bench::Bench;
use fake_core::view::{self, THEME, TimelineView};
use fake_core::{
    Config, Feed, Field, GestureId, Loading, MAX_METERS, Peaks, SAMPLE_RATE, Session, Target, Topic, Value,
};
use gpui::{
    App, Bounds, Context, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder,
    PinchEvent, Pixels, Rgba, ScrollDelta, ScrollEvent, SharedString, Task, Window, WindowBounds, WindowOptions,
    canvas, div, fill, point, prelude::*, px, rgb, size,
};
use gpui_component::button::Button;
use gpui_component::input::{Input, InputState};
use gpui_component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_component::switch::Switch;
use gpui_component::{Root, Theme, ThemeMode};
use gpui_platform::application;

fn c(hex: u32) -> Rgba {
    rgb(hex)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum KnobTarget {
    Track(usize),
    Channel(usize),
}

struct KnobDrag {
    target: KnobTarget,
    start_y: f32,
    start_t: f64,
    gesture: GestureId,
}

/// Waveforms as one quad per pixel column (default) or as one filled path per item (`LM_WAVE=path`)
#[derive(Clone, Copy, PartialEq)]
enum WaveMode {
    Quads,
    Path,
}

struct Daw {
    session: Session,
    bench: Bench,
    view: TimelineView,
    mixer_scroll: f32,
    feed: Feed,
    drag: Option<KnobDrag>,
    shown: [f32; MAX_METERS],
    last_render: Instant,
    /// volume labels, rebuilt from the feed's diffs, not every frame
    track_db: Vec<SharedString>,
    channel_db: Vec<SharedString>,
    /// the timeline's bounds from the last paint, for zooming around the pointer
    timeline_bounds: Rc<RefCell<Bounds<Pixels>>>,
    wave_mode: WaveMode,
    /// the settings panel (design 08), made of gpui-component pieces
    settings_open: bool,
    project_dir: gpui::Entity<InputState>,
    block: gpui::Entity<SliderState>,
    block_size: f32,
    follow_cursor: bool,
    _subscriptions: Vec<gpui::Subscription>,
    _pump: Task<()>,
}

impl Daw {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Daw {
        let mut session = Session::create(Config::default());
        let bench = Bench::from_env("gpui");
        if bench.wants_play() {
            session.play();
        }
        let feed = session.feed(Topic::All);
        // H2: the pump on a 10 ms timer of the main loop, with a fixed cadence (no drift), independent of drawing
        let pump = cx.spawn(async move |this, cx| {
            let period = Duration::from_millis(10);
            let mut next = Instant::now() + period;
            loop {
                let wait = next.saturating_duration_since(Instant::now());
                cx.background_executor().timer(wait).await;
                next += period;
                if next < Instant::now() {
                    next = Instant::now() + period;
                }
                if this.update(cx, |d, cx| d.on_pump(cx)).is_err() {
                    break;
                }
            }
        });
        let project_dir = cx.new(|cx| InputState::new(window, cx).default_value("~/Música/LinuxMusic"));
        let block = cx.new(|_| SliderState::new().min(64.0).max(2048.0).step(64.0).default_value(256.0));
        // settings apply at once (design 08): the slider's events reach the view
        let block_sub = cx.subscribe(&block, |d: &mut Daw, _, ev: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(v)) = ev {
                d.block_size = *v;
                cx.notify();
            }
        });
        let track_db = session.tracks().iter().map(|t| view::format_db(t.volume_db).into()).collect();
        let channel_db = session.channels().iter().map(|c| view::format_db(c.volume_db).into()).collect();
        Daw {
            session,
            bench,
            view: TimelineView::default(),
            mixer_scroll: 0.0,
            feed,
            drag: None,
            shown: [0.0; MAX_METERS],
            last_render: Instant::now(),
            track_db,
            channel_db,
            timeline_bounds: Rc::new(RefCell::new(Bounds::default())),
            wave_mode: if std::env::var("LM_WAVE").as_deref() == Ok("path") { WaveMode::Path } else { WaveMode::Quads },
            settings_open: std::env::var("LM_SETTINGS").is_ok(), // open at start, for the screenshot
            project_dir,
            block,
            block_size: 256.0,
            follow_cursor: false,
            _subscriptions: vec![block_sub],
            _pump: pump,
        }
    }

    fn on_pump(&mut self, cx: &mut Context<Self>) {
        let changed = self.session.pump();
        if self.bench.tick() {
            cx.quit();
            return;
        }
        let falling = self.shown.iter().any(|&m| m > 0.0);
        if changed || falling {
            cx.notify();
        }
    }

    // --- knobs and their gestures (I-3.7) ------------------------------------------------------------------

    fn knob_value(&self, t: KnobTarget) -> f64 {
        view::db_to_knob(match t {
            KnobTarget::Track(i) => self.session.tracks()[i].volume_db,
            KnobTarget::Channel(i) => self.session.channels()[i].volume_db,
        })
    }

    fn begin_knob(&mut self, t: KnobTarget, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            let _ = self.session.gesture_commit(d.gesture);
        }
        let gesture = self.session.begin_gesture("Volume");
        self.drag = Some(KnobDrag { target: t, start_y: ev.position.y.as_f32(), start_t: self.knob_value(t), gesture });
        cx.stop_propagation();
    }

    fn drag_knob(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.drag else { return };
        let v = view::knob_drag(d.start_t, (ev.position.y.as_f32() - d.start_y) as f64, ev.modifiers.shift);
        let db = view::knob_to_db(v);
        let db = if db.is_infinite() { -60.0 } else { db };
        let (id, target) = match d.target {
            KnobTarget::Track(i) => (self.session.tracks()[i].id, d.target),
            KnobTarget::Channel(i) => (self.session.channels()[i].id, d.target),
        };
        let _ = target;
        let r = self.session.gesture_update(&d.gesture, |e| e.set(id, Field::Volume, Value::Real(db)));
        if r.is_err() {
            self.drag = None; // G2/G3: the gesture closed under us; let go of the knob
        }
        cx.notify();
    }

    fn end_knob(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            let _ = self.session.gesture_commit(d.gesture);
            cx.notify();
        }
    }

    fn toggle(&mut self, i: usize, f: Field, cx: &mut Context<Self>) {
        let t = &self.session.tracks()[i];
        let (id, v) = match f {
            Field::Mute => (t.id, !t.mute),
            Field::Solo => (t.id, !t.solo),
            Field::Armed => (t.id, !t.armed),
            Field::Volume => return,
        };
        let _ = self.session.transact("Toggle", |e| e.set(id, f, Value::Bool(v)));
        cx.notify();
    }

    // --- scroll and zoom -----------------------------------------------------------------------------------

    fn timeline_view_h(window: &Window) -> f64 {
        (window.viewport_size().height.as_f32() - view::TRANSPORT_H - view::MIXER_H - view::RULER_H) as f64
    }

    fn content_h(&self) -> f64 {
        self.session.tracks().len() as f64 * view::TRACK_H as f64
    }

    fn on_timeline_scroll(&mut self, ev: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let b = *self.timeline_bounds.borrow();
        let x = (ev.position.x - b.origin.x).as_f32() as f64;
        let zoom = ev.modifiers.platform || ev.modifiers.control;
        let (dx, dy) = (d.x.as_f32() as f64, d.y.as_f32() as f64);
        // a mouse wheel with Shift scrolls horizontally
        let (dx, dy) = if ev.modifiers.shift && matches!(ev.delta, ScrollDelta::Lines(_)) { (dy, 0.0) } else { (dx, dy) };
        let content = self.content_h();
        self.view.wheel(dx, if zoom { -dy } else { dy }, zoom, x, content, Self::timeline_view_h(window));
        cx.notify();
    }

    fn on_timeline_pinch(&mut self, ev: &PinchEvent, window: &mut Window, cx: &mut Context<Self>) {
        let b = *self.timeline_bounds.borrow();
        let x = (ev.position.x - b.origin.x).as_f32() as f64;
        let content = self.content_h();
        // a pinch of +0.1 is 10 % closer
        let dy = -(1.0 + ev.delta as f64).ln() * 100.0;
        self.view.wheel(0.0, dy, true, x, content, Self::timeline_view_h(window));
        cx.notify();
    }

    fn on_mixer_scroll(&mut self, ev: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let delta = if d.x.as_f32().abs() > d.y.as_f32().abs() { d.x } else { d.y }.as_f32();
        let max = (self.session.channels().len() as f32 * view::STRIP_W - window.viewport_size().width.as_f32()).max(0.0);
        self.mixer_scroll = (self.mixer_scroll - delta).clamp(0.0, max);
        cx.notify();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Small pieces

fn button(id: impl Into<SharedString>, label: &'static str, on: bool, on_color: u32) -> gpui::Stateful<gpui::Div> {
    let id: SharedString = id.into();
    div()
        .id(id)
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
        .text_color(c(if on { 0x111111 } else { THEME.text }))
        .text_size(px(10.5))
        .font_weight(FontWeight::BOLD)
        .child(label)
}

fn tbutton(id: &'static str, label: &'static str, on: bool, on_color: u32) -> gpui::Stateful<gpui::Div> {
    button(id, label, on, on_color).h(px(26.)).min_w(px(28.)).text_size(px(12.))
}

/// A vertical meter: fill 0..1 on a -60..0 dB scale, green / yellow / red as design 02
fn meter(fill_v: f32, w: f32, h: f32) -> impl IntoElement {
    canvas(
        move |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(fill(b, c(THEME.lcd_bg)));
            let hh = b.size.height.as_f32();
            let level = fill_v * hh;
            let zones = [(0.0, 0.7, THEME.green), (0.7, 0.9, THEME.yellow), (0.9, 1.0, THEME.red)];
            for (a, z, col) in zones {
                let lo = a * hh;
                let hi = (z * hh).min(level);
                if hi > lo {
                    let bb = Bounds::new(
                        point(b.origin.x, b.origin.y + px(hh - hi)),
                        size(b.size.width, px(hi - lo)),
                    );
                    window.paint_quad(fill(bb, c(col)));
                }
            }
        },
    )
    .w(px(w))
    .h(px(h))
}

/// The knob of design 02 (REAPER-like): a disc with an indicator, -135..+135 degrees
fn knob_face(t: f64, d: f32) -> impl IntoElement {
    canvas(
        move |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(
                gpui::quad(b, px(d / 2.0), c(THEME.bg4), px(1.0), c(0x111111), Default::default()),
            );
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

/// THE loading component (I-2.3): one for every screen. It reads the target's `Loading`; before 200 ms it draws
/// nothing, then the progress in place of the object, the cancel button after 2 s, and a failure in red.
fn loading_component(
    id: impl Into<gpui::ElementId>,
    loading: &Loading,
    on_cancel: impl Fn(&mut Window, &mut App) + 'static,
) -> Option<gpui::AnyElement> {
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
                    .bg(gpui::rgba(0x0b0d0fcc))
                    .child(
                        div().w(px(160.)).h(px(6.)).rounded(px(3.)).bg(c(THEME.bg4)).child(
                            div().h_full().w(px(160. * p)).rounded(px(3.)).bg(c(THEME.green)),
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(THEME.text2))
                            .child(format!("{}… {:.0}%", stage, p * 100.0)),
                    )
                    .when(*cancellable, |d| {
                        d.child(button("cancel", "Cancelar", false, 0).on_click(move |_, w, cx| on_cancel(w, cx)))
                    })
                    .into_any_element(),
            )
        }
        Loading::Failed(e) => Some(div().size_full().text_color(c(THEME.red)).child(e.clone()).into_any_element()),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Render

impl Render for Daw {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = Instant::now();
        if self.bench.frame() {
            cx.quit();
        }
        let probe = self.bench.active().then(|| self.bench.work_probe());
        // I-3.6: drain the feeds; only what changed is rebuilt
        if let Some(diff) = self.feed.drain(&self.session) {
            for (id, _) in &diff.changed {
                if let Some(i) = self.session.tracks().iter().position(|t| t.id == *id) {
                    self.track_db[i] = view::format_db(self.session.tracks()[i].volume_db).into();
                }
                if let Some(i) = self.session.channels().iter().position(|c| c.id == *id) {
                    self.channel_db[i] = view::format_db(self.session.channels()[i].volume_db).into();
                }
            }
        }
        let now = Instant::now();
        let view_h = Self::timeline_view_h(window);
        if let Some(v) = self.bench.view(now) {
            self.view.scroll_samples = v.scroll_samples;
            self.view.samples_per_px = v.samples_per_px;
            self.view.scroll_y = v.scroll_y * (self.content_h() - view_h).max(0.0);
        }
        // meters: the telemetry's latest frame, with the fall smoothed per frame (H16)
        let dt = now.duration_since(self.last_render).as_secs_f32().min(0.1);
        self.last_render = now;
        let meters = *self.session.meters();
        for (s, m) in self.shown.iter_mut().zip(meters.iter()) {
            *s = view::meter_ballistics(*s, view::meter_fill(*m), dt);
        }
        let pos = self.session.position();
        let cursor = pos.at(now);
        if self.follow_cursor && pos.playing && !self.bench.active() {
            let w = window.viewport_size().width.as_f32() as f64 - view::HEADER_W as f64;
            let x = self.view.sample_to_x(cursor);
            if x < 0.0 || x > w * 0.9 {
                self.view.scroll_samples = (cursor - w * 0.1 * self.view.samples_per_px).max(0.0);
            }
        }
        if pos.playing || self.bench.view(now).is_some() {
            window.request_animation_frame();
        }

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(c(THEME.window))
            .text_color(c(THEME.text))
            .font_family(".SystemUIFont")
            .on_mouse_move(cx.listener(|d, ev: &MouseMoveEvent, _, cx| d.drag_knob(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|d, _: &MouseUpEvent, _, cx| d.end_knob(cx)))
            .child(self.render_transport(cursor, pos.playing, pos.recording, cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(self.render_headers(view_h, cx))
                    .child(self.render_timeline(cursor, view_h, cx)),
            )
            .child(self.render_mixer(window, cx))
            .when(self.settings_open, |d| d.child(self.render_settings(cx)))
            // painted last: the CPU time of the frame, from the start of render to the end of paint
            .children(probe.map(|probe| {
                canvas(|_, _, _| {}, move |_, _, _, _| probe(started)).absolute().size(px(0.))
            }))
    }
}

impl Daw {
    fn render_transport(&self, cursor: f64, playing: bool, recording: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let (word, word_color, lcd_border) = if recording {
            (view::tr("Recording"), THEME.red, THEME.red)
        } else if playing {
            (view::tr("Playing"), THEME.green, THEME.green)
        } else {
            (view::tr("Stopped"), THEME.text3, THEME.line)
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
            .child(tbutton("rew", "⏮", false, 0).on_click(cx.listener(|d, _, _, cx| {
                d.session.locate(0);
                cx.notify();
            })))
            .child(tbutton("stop", "■", false, 0).on_click(cx.listener(|d, _, _, cx| {
                d.session.stop();
                cx.notify();
            })))
            .child(tbutton("play", "▶", playing, THEME.green).on_click(cx.listener(|d, _, _, cx| {
                d.session.play();
                cx.notify();
            })))
            .child(tbutton("rec", "●", recording, THEME.red).on_click(cx.listener(|d, _, _, cx| {
                d.session.toggle_record();
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
            .child(tbutton("preroll", "Pré-roll 1", false, 0))
            .child(div().flex_1())
            .child(tbutton("reload", "Recarregar take", false, 0).on_click(cx.listener(|d, _, _, cx| {
                d.session.reload_take(2);
                cx.notify();
            })))
            .child(tbutton("settings", "Configurações", self.settings_open, THEME.green).on_click(cx.listener(
                |d, _, _, cx| {
                    d.settings_open = !d.settings_open;
                    cx.notify();
                },
            )))
    }

    /// Design 08 in miniature: applies at once, says what needs a restart. Built from gpui-component
    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let row = |label: &'static str| {
            div().flex().flex_col().gap(px(6.)).child(div().text_size(px(12.)).text_color(c(THEME.text2)).child(label))
        };
        div()
            .absolute()
            .top(px(view::TRANSPORT_H + 8.0))
            .right(px(8.))
            .w(px(380.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .rounded(px(10.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child("Configurações"))
            .child(row("Pasta dos projetos").child(Input::new(&self.project_dir)))
            .child(
                row("Tamanho do bloco do áudio")
                    .child(Slider::new(&self.block))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(THEME.orange))
                            .child(format!("{} amostras · vale ao reabrir o dispositivo", self.block_size as u32)),
                    ),
            )
            .child(
                Switch::new("follow")
                    .checked(self.follow_cursor)
                    .label("Seguir o cursor ao tocar")
                    .on_click(cx.listener(|d, on: &bool, _, cx| {
                        d.follow_cursor = *on;
                        cx.notify();
                    })),
            )
            .child(div().flex().justify_end().child(Button::new("close-settings").label("Fechar").on_click(
                cx.listener(|d, _, _, cx| {
                    d.settings_open = false;
                    cx.notify();
                }),
            )))
    }

    fn render_headers(&self, view_h: f64, cx: &mut Context<Self>) -> impl IntoElement {
        let (first, n) = self.view.visible_tracks(view_h, self.session.tracks().len());
        let mut col = div()
            .w(px(view::HEADER_W))
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(c(THEME.bg1))
            .border_r_1()
            .border_color(c(THEME.line));
        for i in first..first + n {
            let t = &self.session.tracks()[i];
            let top = view::RULER_H as f64 + i as f64 * view::TRACK_H as f64 - self.view.scroll_y;
            let kt = KnobTarget::Track(i);
            let kv = self.knob_value(kt);
            col = col.child(
                div()
                    .absolute()
                    .top(px(top as f32))
                    .left(px(0.))
                    .w(px(view::HEADER_W))
                    .h(px(view::TRACK_H))
                    .flex()
                    .flex_row()
                    .bg(c(THEME.bg2))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    // 1: the track number in its color
                    .child(
                        div()
                            .w(px(22.))
                            .h_full()
                            .bg(c(t.color))
                            .text_size(px(10.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(c(0x111111))
                            .pt(px(4.))
                            .flex()
                            .justify_center()
                            .child(format!("{}", i + 1)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .p(px(6.))
                            // 2, 3: record, the name, M and S
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(4.))
                                    .items_center()
                                    .child(button(format!("r{i}"), "●", t.armed, THEME.red).on_click(
                                        cx.listener(move |d, _, _, cx| d.toggle(i, Field::Armed, cx)),
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .h(px(18.))
                                            .px(px(6.))
                                            .rounded(px(3.))
                                            .bg(c(THEME.bg1))
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::BOLD)
                                            .overflow_hidden()
                                            .child(t.name.clone()),
                                    )
                                    .child(button(format!("m{i}"), "M", t.mute, THEME.orange).on_click(
                                        cx.listener(move |d, _, _, cx| d.toggle(i, Field::Mute, cx)),
                                    ))
                                    .child(button(format!("s{i}"), "S", t.solo, THEME.yellow).on_click(
                                        cx.listener(move |d, _, _, cx| d.toggle(i, Field::Solo, cx)),
                                    )),
                            )
                            // 4: the volume knob (no pan: I-3.9), its value, FX and I/O
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(6.))
                                    .items_center()
                                    .child(
                                        div()
                                            .id(("tknob", i))
                                            .cursor_ns_resize()
                                            .child(knob_face(kv, 22.0))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |d, ev: &MouseDownEvent, _, cx| d.begin_knob(kt, ev, cx)),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(9.5))
                                            .text_color(c(THEME.text2))
                                            .child(self.track_db[i].clone()),
                                    )
                                    .child(div().flex_1())
                                    .child(button(format!("fx{i}"), "FX", false, 0))
                                    .child(button(format!("io{i}"), "I/O", false, 0)),
                            ),
                    )
                    // the meter on the right border
                    .child(div().p(px(3.)).child(meter(self.shown[i], 6.0, view::TRACK_H - 6.0))),
            );
        }
        col
    }

    fn render_timeline(&self, cursor: f64, view_h: f64, cx: &mut Context<Self>) -> impl IntoElement {
        let (first, n) = self.view.visible_tracks(view_h, self.session.tracks().len());
        let v = self.view;
        struct Row {
            top: f32,
            item_x: f32,
            item_w: f32,
            item_start: i64,
            color: u32,
            peaks: Option<Arc<Peaks>>,
        }
        let mut rows = Vec::with_capacity(n);
        let mut overlays = Vec::new();
        for i in first..first + n {
            let t = &self.session.tracks()[i];
            let top = view::RULER_H + (i as f64 * view::TRACK_H as f64 - v.scroll_y) as f32;
            let item_x = v.sample_to_x(t.item.start as f64) as f32;
            let item_w = (t.item.length as f64 / v.samples_per_px) as f32;
            rows.push(Row {
                top,
                item_x,
                item_w,
                item_start: t.item.start,
                color: t.color,
                peaks: self.session.peaks(t.item.resource).cloned(),
            });
            let target = Target::Node(t.item.id);
            let loading = self.session.loading(&target);
            let weak = cx.weak_entity();
            if let Some(el) = loading_component(("load", i), &loading, move |_, cx| {
                let _ = weak.update(cx, |d, cx| {
                    d.session.cancel_target(&target);
                    cx.notify();
                });
            }) {
                // in place of the waveform: the visible part of the item
                let x0 = item_x.max(0.0);
                overlays.push(
                    div()
                        .absolute()
                        .top(px(top + 3.0))
                        .left(px(x0))
                        .w(px((item_x + item_w - x0).clamp(0.0, 4000.0)))
                        .h(px(view::TRACK_H - 6.0))
                        .child(el),
                );
            }
        }
        // bar numbers on the ruler: text as elements (a canvas would shape its own text with the text system)
        let bar = 2.0 * SAMPLE_RATE as f64;
        let bars_per_label = ((80.0 * v.samples_per_px / bar).max(1.0)).ceil();
        let step = bar * bars_per_label;
        let mut labels = Vec::new();
        let mut s = (v.scroll_samples / step).floor() * step;
        let width = 1600.0; // labels past the edge are clipped
        while v.sample_to_x(s) < width {
            let x = v.sample_to_x(s) as f32;
            if x >= -40.0 {
                labels.push(
                    div()
                        .absolute()
                        .top(px(2.))
                        .left(px(x + 3.0))
                        .text_size(px(10.))
                        .text_color(c(THEME.text2))
                        .child(format!("{}", (s / bar).round() as i64 + 1)),
                );
            }
            s += step;
        }
        let bounds_cell = self.timeline_bounds.clone();
        let mode = self.wave_mode;
        let playing_x = v.sample_to_x(cursor) as f32;
        div()
            .id("timeline")
            .flex_1()
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(c(THEME.bg1))
            .on_scroll(cx.listener(Self::on_timeline_scroll))
            .on_pinch(cx.listener(Self::on_timeline_pinch))
            .child(
                canvas(
                    move |b, _, _| {
                        *bounds_cell.borrow_mut() = b;
                    },
                    move |b, _, window, _| {
                        let w = b.size.width.as_f32();
                        let o = b.origin;
                        let mut cols = Vec::with_capacity(w as usize + 1);
                        for r in &rows {
                            // lane line
                            window.paint_quad(fill(
                                Bounds::new(point(o.x, o.y + px(r.top + view::TRACK_H - 1.0)), size(b.size.width, px(1.0))),
                                c(THEME.line),
                            ));
                            let x0 = r.item_x.max(0.0);
                            let x1 = (r.item_x + r.item_w).min(w);
                            if x1 <= x0 {
                                continue;
                            }
                            let ih = view::TRACK_H - 6.0;
                            let iy = r.top + 3.0;
                            let mut body = c(r.color);
                            body.a = 0.18;
                            window.paint_quad(gpui::quad(
                                Bounds::new(point(o.x + px(x0), o.y + px(iy)), size(px(x1 - x0), px(ih))),
                                px(3.0),
                                body,
                                px(1.0),
                                c(r.color),
                                Default::default(),
                            ));
                            let Some(peaks) = &r.peaks else { continue };
                            // the right pyramid level for the zoom, one (min, max) per pixel column
                            let first_col = x0.floor() as usize;
                            let ncol = (x1.ceil() - x0.floor()) as usize;
                            let start = v.scroll_samples + first_col as f64 * v.samples_per_px - r.item_start as f64;
                            peaks.columns(start, v.samples_per_px, ncol, &mut cols);
                            let mid = o.y + px(iy + ih / 2.0);
                            let half = ih / 2.0 - 2.0;
                            let scale = half / i16::MAX as f32;
                            let wave = c(r.color);
                            match mode {
                                WaveMode::Quads => {
                                    for (k, col) in cols.iter().enumerate() {
                                        let Some((lo, hi)) = col else { continue };
                                        let top = -(*hi as f32) * scale;
                                        let bot = -(*lo as f32) * scale;
                                        let x = o.x + px((first_col + k) as f32);
                                        window.paint_quad(fill(
                                            Bounds::new(point(x, mid + px(top)), size(px(1.0), px((bot - top).max(1.0)))),
                                            wave,
                                        ));
                                    }
                                }
                                WaveMode::Path => {
                                    let mut pts = Vec::with_capacity(cols.len() * 2);
                                    for (k, col) in cols.iter().enumerate() {
                                        if let Some((_, hi)) = col {
                                            pts.push(point(o.x + px((first_col + k) as f32 + 0.5), mid - px(*hi as f32 * scale)));
                                        }
                                    }
                                    for (k, col) in cols.iter().enumerate().rev() {
                                        if let Some((lo, _)) = col {
                                            pts.push(point(o.x + px((first_col + k) as f32 + 0.5), mid - px(*lo as f32 * scale)));
                                        }
                                    }
                                    if pts.len() >= 3 {
                                        let mut pb = PathBuilder::fill();
                                        pb.add_polygon(&pts, true);
                                        if let Ok(p) = pb.build() {
                                            window.paint_path(p, wave);
                                        }
                                    }
                                }
                            }
                        }
                        // ruler: a tick per bar (2 s at 120 BPM in 4/4)
                        window.paint_quad(fill(Bounds::new(o, size(b.size.width, px(view::RULER_H))), c(THEME.bg3)));
                        let bar = 2.0 * SAMPLE_RATE as f64;
                        let step = (bar * ((80.0 * v.samples_per_px / bar).max(1.0)).ceil()).max(bar);
                        let mut s = (v.scroll_samples / step).floor() * step;
                        while v.sample_to_x(s) < w as f64 {
                            let x = v.sample_to_x(s) as f32;
                            window.paint_quad(fill(
                                Bounds::new(point(o.x + px(x), o.y + px(8.0)), size(px(1.0), px(view::RULER_H - 8.0))),
                                c(THEME.text3),
                            ));
                            s += step;
                        }
                        // the play cursor, interpolated between telemetry frames
                        if playing_x >= 0.0 && playing_x < w {
                            window.paint_quad(fill(
                                Bounds::new(point(o.x + px(playing_x), o.y), size(px(1.0), b.size.height)),
                                c(THEME.cursor),
                            ));
                        }
                    },
                )
                .size_full(),
            )
            .children(labels)
            .children(overlays)
    }

    fn render_mixer(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width.as_f32();
        let count = self.session.channels().len();
        let first = (self.mixer_scroll / view::STRIP_W).floor() as usize;
        let n = ((width / view::STRIP_W).ceil() as usize + 1).min(count - first.min(count));
        let mut row = div()
            .id("mixer")
            .h(px(view::MIXER_H))
            .w_full()
            .relative()
            .overflow_hidden()
            .bg(c(THEME.bg1))
            .border_t_2()
            .border_color(c(THEME.line))
            .on_scroll(cx.listener(Self::on_mixer_scroll));
        for i in first..first + n {
            let ch = &self.session.channels()[i];
            let kt = KnobTarget::Channel(i);
            let kv = self.knob_value(kt);
            row = row.child(
                div()
                    .absolute()
                    .top(px(0.))
                    .left(px(i as f32 * view::STRIP_W - self.mixer_scroll))
                    .w(px(view::STRIP_W - 2.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .py(px(6.))
                    .bg(c(THEME.bg2))
                    .border_r_1()
                    .border_color(c(THEME.line))
                    .child(div().text_size(px(11.)).font_weight(FontWeight::BOLD).child(ch.name.clone()))
                    .child(div().flex_1().child(meter(self.shown[64 + i], 10.0, view::MIXER_H - 100.0)))
                    .child(
                        div()
                            .id(("mknob", i))
                            .cursor_ns_resize()
                            .child(knob_face(kv, 30.0))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |d, ev: &MouseDownEvent, _, cx| d.begin_knob(kt, ev, cx)),
                            ),
                    )
                    .child(div().text_size(px(9.5)).text_color(c(THEME.text2)).child(self.channel_db[i].clone())),
            );
        }
        row
    }
}

fn main() {
    application().run(|cx: &mut App| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(view::WINDOW_W), px(view::WINDOW_H)));
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |window, cx| {
                let daw = cx.new(|cx| Daw::new(window, cx));
                cx.new(|cx| Root::new(daw, window, cx))
            },
        )
        .expect("open window");
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}
