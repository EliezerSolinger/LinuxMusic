//! The composition page: the timeline with the track headers (design 02, no pan), the items, recording, the lanes
//! and the ruler. The transport bar is the workspace's, over both pages.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use fake_core::view::{self, TimelineView};
use fake_core::{Feed, Field, GestureId, Peaks, SAMPLE_RATE, Target, Topic, Value};
use gpui::{
    Bounds, Context, Entity, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder,
    PinchEvent, Pixels, Render, ScrollDelta, ScrollEvent, SharedString, Window, canvas, div, fill, point, prelude::*,
    px, size,
};

use crate::core::Core;
use crate::tiles::{self, TILE_H, TILE_PAIRS, TileCache};
use crate::ui::{self, THEME, button, c, knob_face, loading_component, meter, track_color};

/// How waveforms are drawn: cached image tiles of the pyramid (default), one quad per pixel column
/// (`LM_WAVE=quads`), or one filled path per item (`LM_WAVE=path`). The three are measured in the report.
#[derive(Clone, Copy, PartialEq)]
pub enum WaveMode {
    Tiles,
    Quads,
    Path,
}

struct KnobDrag {
    track: usize,
    start_y: f32,
    start_t: f64,
    gesture: GestureId,
}

pub struct CompositionPage {
    core: Entity<Core>,
    pub view: TimelineView,
    feed: Feed,
    drag: Option<KnobDrag>,
    /// volume labels, rebuilt from the feed's diffs, not every frame (I-3.6)
    track_db: Vec<SharedString>,
    /// the timeline's bounds from the last paint, for zooming around the pointer
    timeline_bounds: Rc<RefCell<Bounds<Pixels>>>,
    wave_mode: WaveMode,
    tiles: Rc<RefCell<TileCache>>,
    pub follow_cursor: bool,
}

impl CompositionPage {
    pub fn new(core: Entity<Core>, cx: &mut Context<Self>) -> Self {
        let feed = core.update(cx, |core, _| core.session.feed(Topic::All));
        let track_db = core.read(cx).session.tracks().iter().map(|t| view::format_db(t.volume_db).into()).collect();
        CompositionPage {
            core,
            view: TimelineView::default(),
            feed,
            drag: None,
            track_db,
            timeline_bounds: Rc::new(RefCell::new(Bounds::default())),
            wave_mode: match std::env::var("LM_WAVE").as_deref() {
                Ok("path") => WaveMode::Path,
                Ok("quads") => WaveMode::Quads,
                _ => WaveMode::Tiles,
            },
            tiles: Rc::new(RefCell::new(TileCache::default())),
            follow_cursor: false,
        }
    }

    // --- the volume knob's gesture (I-3.7) -----------------------------------------------------------------

    fn begin_knob(&mut self, track: usize, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let start_t = view::db_to_knob(self.core.read(cx).session.tracks()[track].volume_db);
        let prev = self.drag.take();
        let gesture = self.core.update(cx, |core, _| {
            if let Some(d) = prev {
                let _ = core.session.gesture_commit(d.gesture);
            }
            core.session.begin_gesture("Volume")
        });
        self.drag = Some(KnobDrag { track, start_y: ev.position.y.as_f32(), start_t, gesture });
        cx.stop_propagation();
    }

    fn drag_knob(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.drag else { return };
        let t = view::knob_drag(d.start_t, (ev.position.y.as_f32() - d.start_y) as f64, ev.modifiers.shift);
        let db = view::knob_to_db(t).max(-60.0);
        let track = d.track;
        let ok = self.core.update(cx, |core, _| {
            let id = core.session.tracks()[track].id;
            let g = &self.drag.as_ref().unwrap().gesture;
            core.session.gesture_update(g, |e| e.set(id, Field::Volume, Value::Real(db))).is_ok()
        });
        if !ok {
            self.drag = None; // G2/G3: the gesture closed under us; let go of the knob
        }
        cx.notify();
    }

    fn end_knob(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            self.core.update(cx, |core, _| {
                let _ = core.session.gesture_commit(d.gesture);
            });
            cx.notify();
        }
    }

    fn toggle(&mut self, i: usize, f: Field, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            let t = &core.session.tracks()[i];
            let (id, v) = match f {
                Field::Mute => (t.id, !t.mute),
                Field::Solo => (t.id, !t.solo),
                Field::Armed => (t.id, !t.armed),
                _ => return,
            };
            let _ = core.session.transact("Toggle", |e| e.set(id, f, Value::Bool(v)));
        });
        cx.notify();
    }

    // --- scroll and zoom -----------------------------------------------------------------------------------

    fn view_h(window: &Window) -> f64 {
        (window.viewport_size().height.as_f32() - crate::workspace::CHROME_H - view::RULER_H) as f64
    }

    fn content_h(&self, cx: &Context<Self>) -> f64 {
        self.core.read(cx).session.tracks().len() as f64 * view::TRACK_H as f64
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let b = *self.timeline_bounds.borrow();
        let x = (ev.position.x - b.origin.x).as_f32() as f64;
        let zoom = ev.modifiers.platform || ev.modifiers.control;
        let (dx, dy) = (d.x.as_f32() as f64, d.y.as_f32() as f64);
        // a mouse wheel with Shift scrolls horizontally
        let (dx, dy) = if ev.modifiers.shift && matches!(ev.delta, ScrollDelta::Lines(_)) { (dy, 0.0) } else { (dx, dy) };
        let content = self.content_h(cx);
        self.view.wheel(dx, if zoom { -dy } else { dy }, zoom, x, content, Self::view_h(window));
        cx.notify();
    }

    fn on_pinch(&mut self, ev: &PinchEvent, window: &mut Window, cx: &mut Context<Self>) {
        let b = *self.timeline_bounds.borrow();
        let x = (ev.position.x - b.origin.x).as_f32() as f64;
        let content = self.content_h(cx);
        let dy = -(1.0 + ev.delta as f64).ln() * 100.0; // a pinch of +0.1 is 10 % closer
        self.view.wheel(0.0, dy, true, x, content, Self::view_h(window));
        cx.notify();
    }
}

impl Render for CompositionPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let view_h = Self::view_h(window);
        // I-3.6: drain the feed; only the labels that changed are rebuilt
        {
            let core = self.core.read(cx);
            if let Some(diff) = self.feed.drain(&core.session) {
                for (id, _) in &diff.changed {
                    if let Some(i) = core.session.tracks().iter().position(|t| t.id == *id) {
                        self.track_db[i] = view::format_db(core.session.tracks()[i].volume_db).into();
                    }
                }
            }
            if let Some(v) = core.bench.view(now) {
                let content = core.session.tracks().len() as f64 * view::TRACK_H as f64;
                self.view.scroll_samples = v.scroll_samples;
                self.view.samples_per_px = v.samples_per_px;
                self.view.scroll_y = v.scroll_y * (content - view_h).max(0.0);
            }
        }
        let pos = self.core.read(cx).session.position();
        let cursor = pos.at(now);
        if self.follow_cursor && pos.playing {
            let w = window.viewport_size().width.as_f32() as f64 - view::HEADER_W as f64;
            let x = self.view.sample_to_x(cursor);
            if x < 0.0 || x > w * 0.9 {
                self.view.scroll_samples = (cursor - w * 0.1 * self.view.samples_per_px).max(0.0);
            }
        }
        div()
            .size_full()
            .flex()
            .flex_row()
            .overflow_hidden()
            .on_mouse_move(cx.listener(|p, ev: &MouseMoveEvent, _, cx| p.drag_knob(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|p, _: &MouseUpEvent, _, cx| p.end_knob(cx)))
            .child(self.render_headers(view_h, cx))
            .child(self.render_timeline(cursor, view_h, cx))
    }
}

impl CompositionPage {
    fn render_headers(&self, view_h: f64, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let tracks = core.session.tracks();
        let (first, n) = self.view.visible_tracks(view_h, tracks.len());
        let mut col = div()
            .w(px(view::HEADER_W))
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(c(THEME.bg1))
            .border_r_1()
            .border_color(c(THEME.line))
            .child(div().h(px(view::RULER_H)).bg(c(THEME.bg3)));
        for i in first..first + n {
            let t = &tracks[i];
            let top = view::RULER_H as f64 + i as f64 * view::TRACK_H as f64 - self.view.scroll_y;
            let kv = view::db_to_knob(t.volume_db);
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
                            .bg(c(track_color(i)))
                            .text_size(px(10.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(c(THEME.on_accent))
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
                                    .child(
                                        button(SharedString::from(format!("r{i}")), "●", t.armed, THEME.red)
                                            .on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Armed, cx))),
                                    )
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
                                    .child(
                                        button(SharedString::from(format!("m{i}")), "M", t.mute, THEME.orange)
                                            .on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Mute, cx))),
                                    )
                                    .child(
                                        button(SharedString::from(format!("s{i}")), "S", t.solo, THEME.yellow)
                                            .on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Solo, cx))),
                                    ),
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
                                            .debug_selector(move || format!("tknob-{i}"))
                                            .cursor_ns_resize()
                                            .child(knob_face(kv, 22.0))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.begin_knob(i, ev, cx)),
                                            ),
                                    )
                                    .child(div().text_size(px(9.5)).text_color(c(THEME.text2)).child(self.track_db[i].clone()))
                                    .child(div().flex_1())
                                    .child(button(SharedString::from(format!("fx{i}")), "FX", false, 0))
                                    .child(button(SharedString::from(format!("io{i}")), "I/O", false, 0)),
                            ),
                    )
                    // the meter on the right border
                    .child(div().p(px(3.)).child(meter(core.shown[i], 6.0, view::TRACK_H - 6.0))),
            );
        }
        col
    }

    fn render_timeline(&self, cursor: f64, view_h: f64, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let tracks = core.session.tracks();
        let (first, n) = self.view.visible_tracks(view_h, tracks.len());
        let v = self.view;
        struct Row {
            top: f32,
            item_x: f32,
            item_w: f32,
            item_start: i64,
            color: u32,
            resource: u32,
            peaks: Option<Arc<Peaks>>,
        }
        let mut rows = Vec::with_capacity(n);
        let mut overlays = Vec::new();
        for i in first..first + n {
            let t = &tracks[i];
            let top = view::RULER_H + (i as f64 * view::TRACK_H as f64 - v.scroll_y) as f32;
            let item_x = v.sample_to_x(t.item.start as f64) as f32;
            let item_w = (t.item.length as f64 / v.samples_per_px) as f32;
            rows.push(Row {
                top,
                item_x,
                item_w,
                item_start: t.item.start,
                color: track_color(i),
                resource: t.item.resource.0,
                peaks: core.session.peaks(t.item.resource).cloned(),
            });
            let target = Target::Node(t.item.id);
            let loading = core.session.loading(&target);
            let core_handle = self.core.clone();
            if let Some(el) = loading_component(("load", i), &loading, move |_, cx| {
                core_handle.update(cx, |core, cx| {
                    core.session.cancel_target(&target);
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
        let step = bar * ((80.0 * v.samples_per_px / bar).max(1.0)).ceil();
        let mut labels = Vec::new();
        let mut s = (v.scroll_samples / step).floor() * step;
        while v.sample_to_x(s) < 2400.0 {
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
        let tile_cache = self.tiles.clone();
        let playing_x = v.sample_to_x(cursor) as f32;
        let _ = ui::tr; // texts of this page go through ui::tr (keys in locales/pt-BR/composition.txt)
        div()
            .id("timeline")
            .flex_1()
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(c(THEME.bg1))
            .on_scroll(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
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
                            if mode == WaveMode::Tiles {
                                let level = TileCache::level_for(peaks, v.samples_per_px);
                                let tile_samples = Peaks::samples_per_pair(level) as f64 * TILE_PAIRS as f64;
                                let tile_w = (tile_samples / v.samples_per_px) as f32;
                                let s0 = (v.scroll_samples + x0 as f64 * v.samples_per_px - r.item_start as f64).max(0.0);
                                let s1 = v.scroll_samples + x1 as f64 * v.samples_per_px - r.item_start as f64;
                                let clip = Bounds::new(point(o.x + px(x0), o.y + px(iy)), size(px(x1 - x0), px(ih)));
                                let ty = iy + (ih - TILE_H as f32) / 2.0;
                                let mut cache = tile_cache.borrow_mut();
                                for t in (s0 / tile_samples) as usize..(s1 / tile_samples).ceil() as usize {
                                    let img = cache.get(window, peaks, r.resource, level, t, r.color);
                                    let tx = r.item_x + t as f32 * tile_w;
                                    let tb = Bounds::new(point(o.x + px(tx), o.y + px(ty)), size(px(tile_w), px(TILE_H as f32)));
                                    let _ = window.paint_image(clip, tb, Default::default(), img, 0, false);
                                }
                                continue;
                            }
                            // the right pyramid level for the zoom, one (min, max) per pixel column
                            let first_col = x0.floor() as usize;
                            let ncol = (x1.ceil() - x0.floor()) as usize;
                            let start = v.scroll_samples + first_col as f64 * v.samples_per_px - r.item_start as f64;
                            peaks.columns(start, v.samples_per_px, ncol, &mut cols);
                            let mid = o.y + px(iy + ih / 2.0);
                            let scale = (ih / 2.0 - 2.0) / i16::MAX as f32;
                            let wave = c(r.color);
                            if mode == WaveMode::Quads {
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
                            } else {
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
                        // ruler: a tick per bar (2 s at 120 BPM in 4/4)
                        window.paint_quad(fill(Bounds::new(o, size(b.size.width, px(view::RULER_H))), c(THEME.bg3)));
                        let bar = 2.0 * SAMPLE_RATE as f64;
                        let step = bar * ((80.0 * v.samples_per_px / bar).max(1.0)).ceil();
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
}

#[allow(unused)]
fn _keep(_: tiles::TileCache) {}
