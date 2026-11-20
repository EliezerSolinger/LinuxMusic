//! The composition page: the timeline with the track headers (design 02, no pan), folders, several outputs per
//! track, audio and MIDI items, take layers with comping (U2), pre-roll and punch handles (U4), the take that grows
//! while recording (U3), the tempo lane with ramps, automation lanes, and the ruler with bars, loop, punch and
//! markers. The transport bar is the workspace's, over both pages.
//!
//! The rows (tracks, their take rows and lanes) are laid out once per frame from the document; the headers and the
//! timeline draw the same rows, so they never disagree.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use fake_core::arrange::{self, BAR, ItemKind, TrackKind};
use fake_core::view::{self, TimelineView};
use fake_core::port::EditOp;
use fake_core::{Doc, Feed, Field, GestureId, NodeId, ResourceId, SAMPLE_RATE, Target, Topic, Value};
use gpui::{
    AnyElement, Bounds, Context, Entity, FocusHandle, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PathBuilder, PinchEvent, Pixels, Render, ScrollDelta, ScrollEvent, SharedString, WeakEntity, Window,
    actions, canvas, div, fill, point, prelude::*, px, size,
};

use crate::core::Core;
use crate::tiles::{TileCache, level_for, paint_wave, tile_range};
use crate::ui::{THEME, button, c, knob_face, loading_component, meter, tr, trf, track_color};
use crate::workspace::Workspace;

// The arrangement's keys (F-ARR-005..007), REAPER-like; bound in workspace.rs to the "Composition" context
actions!(composition, [Split, AddMarker, RegionFromLoop, Duplicate, DeleteItem, EmptyItem, Marker1, Marker2, Marker3, Marker4, Marker5, Marker6, Marker7, Marker8, Marker9]);

pub const TEMPO_H: f32 = 34.0;
const FOLDER_H: f32 = 30.0;
const TAKE_H: f32 = 34.0;
const LANE_H: f32 = 48.0;
const INDENT: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum RowKind {
    Track(usize),
    /// a take layer of a track's comped item: (track, item, take)
    Take(usize, usize, usize),
    Lane(usize, usize),
}

#[derive(Clone, Copy, Debug)]
struct Row {
    kind: RowKind,
    /// in content coordinates (0 = the first row, under the tempo lane)
    top: f32,
    h: f32,
    depth: u8,
}

/// The rows of the arrangement: tracks not hidden by a collapsed folder, then each track's take rows and lanes
fn layout(d: &Doc, track_h: f32) -> Vec<Row> {
    let tracks = &d.tracks;
    let mut rows = Vec::with_capacity(tracks.len() + 8);
    let mut top = 0.0;
    for (i, t) in tracks.iter().enumerate() {
        let parent = t.parent.and_then(|p| tracks.iter().find(|x| x.id == p));
        if parent.is_some_and(|p| p.collapsed) {
            continue;
        }
        let depth = parent.is_some() as u8;
        let h = if t.kind == TrackKind::Folder { FOLDER_H.min(track_h) } else { track_h };
        rows.push(Row { kind: RowKind::Track(i), top, h, depth });
        top += h;
        if t.show_takes {
            for (k, it) in t.items.iter().enumerate() {
                if let ItemKind::Audio { takes, .. } = &it.kind {
                    for tk in 0..takes.len() {
                        rows.push(Row { kind: RowKind::Take(i, k, tk), top, h: TAKE_H, depth });
                        top += TAKE_H;
                    }
                }
            }
        }
        if t.show_lanes {
            for l in 0..t.lanes.len() {
                rows.push(Row { kind: RowKind::Lane(i, l), top, h: LANE_H, depth });
                top += LANE_H;
            }
        }
    }
    rows
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
    /// the timeline's bounds from the last paint, for zooming around the pointer and clicks on the ruler
    timeline_bounds: Rc<RefCell<Bounds<Pixels>>>,
    tiles: Rc<RefCell<TileCache>>,
    pub follow_cursor: bool,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    /// the track height (design 02: compact 24 px, normal 70, large; Ctrl+wheel changes all, F-ARR-007)
    track_h: f32,
    selected_item: Option<NodeId>,
    item_drag: Option<ItemDrag>,
    /// what was drawn where in the last frame: (x, y, w, h, item, fade in px, fade out px), relative to the timeline
    hits: Rc<RefCell<Vec<(f32, f32, f32, f32, NodeId, f32, f32)>>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ItemZone {
    Move,
    FadeIn,
    FadeOut,
    Gain,
}

struct ItemDrag {
    item: NodeId,
    zone: ItemZone,
    start: (f32, f32),
    start_value: f64,
    gesture: GestureId,
}

impl CompositionPage {
    pub fn new(core: Entity<Core>, workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        let feed = core.update(cx, |core, _| core.port_mut().feed(Topic::All));
        let track_db = core.read(cx).port().root().tracks.iter().map(|t| view::format_db(t.volume_db).into()).collect();
        CompositionPage {
            core,
            view: TimelineView::default(),
            feed,
            drag: None,
            track_db,
            timeline_bounds: Rc::new(RefCell::new(Bounds::default())),
            tiles: Rc::new(RefCell::new(TileCache::default())),
            follow_cursor: false,
            workspace,
            focus: cx.focus_handle(),
            track_h: view::TRACK_H,
            selected_item: None,
            item_drag: None,
            hits: Default::default(),
        }
    }

    // --- the volume knob's gesture (I-3.7) -----------------------------------------------------------------

    fn begin_knob(&mut self, track: usize, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        if ev.click_count == 2 {
            self.core.update(cx, |core, _| {
                let id = core.port().root().tracks[track].id;
                let _ = core.port_mut().apply("Volume", vec![EditOp::set(id, Field::Volume, Value::Real(0.0))]);
            });
            cx.stop_propagation();
            return;
        }
        let start_t = view::db_to_knob(self.core.read(cx).port().root().tracks[track].volume_db);
        let prev = self.drag.take();
        let gesture = self.core.update(cx, |core, _| {
            if let Some(d) = prev {
                core.port_mut().gesture_commit(d.gesture);
            }
            core.port_mut().begin_gesture("Volume")
        });
        self.drag = Some(KnobDrag { track, start_y: ev.position.y.as_f32(), start_t, gesture });
        cx.stop_propagation();
    }

    fn drag_knob(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.drag else { return };
        let t = view::knob_drag(d.start_t, (ev.position.y.as_f32() - d.start_y) as f64, ev.modifiers.shift);
        let db = view::knob_to_db(t).max(-60.0);
        let track = d.track;
        self.core.update(cx, |core, _| {
            let id = core.port().root().tracks[track].id;
            let g = &self.drag.as_ref().unwrap().gesture;
            core.port_mut().gesture_apply(g, vec![EditOp::set(id, Field::Volume, Value::Real(db))]);
        });
        cx.notify();
    }

    fn end_knob(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            self.core.update(cx, |core, _| core.port_mut().gesture_commit(d.gesture));
            cx.notify();
        }
    }

    fn toggle(&mut self, i: usize, f: Field, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            let t = &core.port().root().tracks[i];
            let v = match f {
                Field::Mute => t.mute,
                Field::Solo => t.solo,
                Field::Armed => t.armed,
                Field::Monitor => t.monitor,
                Field::Collapsed => t.collapsed,
                Field::ShowLanes => t.show_lanes,
                Field::ShowTakes => t.show_takes,
                _ => return,
            };
            let id = t.id;
            let _ = core.port_mut().apply("Toggle", vec![EditOp::set(id, f, Value::Bool(!v))]);
        });
        cx.notify();
    }

    // --- scroll, zoom, the ruler ---------------------------------------------------------------------------

    fn view_h(window: &Window) -> f64 {
        (window.viewport_size().height.as_f32() - crate::workspace::CHROME_H - view::RULER_H - TEMPO_H) as f64
    }

    fn content_h(&self, cx: &Context<Self>) -> f64 {
        layout(self.core.read(cx).port().root(), self.track_h).last().map_or(0.0, |r| (r.top + r.h) as f64)
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let b = *self.timeline_bounds.borrow();
        let x = (ev.position.x - b.origin.x).as_f32() as f64;
        let (dx, dy) = (d.x.as_f32() as f64, d.y.as_f32() as f64);
        let wheel = matches!(ev.delta, ScrollDelta::Lines(_));
        // REAPER's wheel (F-ARR-007): a mouse wheel zooms; Ctrl+wheel changes every track's height; Alt+wheel (or
        // Shift) scrolls sideways. A trackpad pans in both directions; with Cmd/Ctrl it zooms.
        if ev.modifiers.control && wheel {
            self.track_h = (self.track_h * (1.0 + dy as f32 * 0.004)).clamp(24.0, 160.0);
            cx.notify();
            return;
        }
        let zoom = if wheel { !(ev.modifiers.alt || ev.modifiers.shift) } else { ev.modifiers.platform || ev.modifiers.control };
        let (dx, dy) = if wheel && (ev.modifiers.alt || ev.modifiers.shift) { (dy, 0.0) } else { (dx, dy) };
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

    fn action(&mut self, id: &'static str, v: Value, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            let _ = core.port_mut().run_action(id, v);
        });
        cx.notify();
    }

    fn item_action(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if let Some(it) = self.selected_item {
            self.action(id, Value::Id(it), cx);
        }
    }

    fn delete_item(&mut self, cx: &mut Context<Self>) {
        if let Some(it) = self.selected_item.take() {
            self.core.update(cx, |core, _| {
                let _ = core.port_mut().apply("Delete", vec![EditOp::Remove { id: fake_core::port::Ref::Id(it) }]);
            });
            cx.notify();
        }
    }

    /// An empty item on the selected item's track, or the first armed one, at the cursor (P-29)
    fn empty_item(&mut self, cx: &mut Context<Self>) {
        let root = self.core.read(cx).port().root();
        let track = self
            .selected_item
            .and_then(|it| root.tracks.iter().position(|t| t.items.iter().any(|i| i.id == it)))
            .or_else(|| root.tracks.iter().position(|t| t.armed))
            .unwrap_or(2);
        self.action("item.empty", Value::Real(track as f64), cx);
    }

    /// FX on a track header: the inserts of the channel its main output goes to (P-28), on the mixing page
    fn open_fx(&mut self, i: usize, _window: &mut Window, cx: &mut Context<Self>) {
        let to = self.core.read(cx).port().root().tracks[i].outputs.first().map(|o| o.to);
        if let Some(ch) = to {
            let _ = self.workspace.update(cx, |ws, cx| {
                ws.page = crate::pages::Page::Mixing;
                ws.mixing.update(cx, |m, cx| m.select_channel(ch, cx));
                cx.notify();
            });
        }
    }

    // --- items: select, move, fades, gain (F-CLP-061) ---------------------------------------------------------

    fn on_timeline_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx); // the arrangement's keys (S, M, Ctrl+D...) go to the page
        let b = *self.timeline_bounds.borrow();
        let (mx, my) = ((ev.position.x - b.origin.x).as_f32(), (ev.position.y - b.origin.y).as_f32());
        if my < view::RULER_H + TEMPO_H {
            return;
        }
        let hit = self.hits.borrow().iter().rev().find(|(x, y, w, h, ..)| mx >= *x && mx <= x + w && my >= *y && my <= y + h).copied();
        let Some((x, y, w, _, item, fi, fo)) = hit else {
            self.selected_item = None;
            cx.notify();
            return;
        };
        self.selected_item = Some(item);
        // the top 7 px: the fade handles at the corners, the gain handle in between; the rest moves the item
        let zone = if my - y <= 7.0 {
            if mx <= x + fi.max(10.0) {
                ItemZone::FadeIn
            } else if mx >= x + w - fo.max(10.0) {
                ItemZone::FadeOut
            } else {
                ItemZone::Gain
            }
        } else {
            ItemZone::Move
        };
        let it = self.core.read(cx).port().root().tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.id == item).cloned();
        let Some(it) = it else { return };
        let start_value = match zone {
            ItemZone::Move => it.start as f64,
            ItemZone::FadeIn => it.fades.0 as f64,
            ItemZone::FadeOut => it.fades.1 as f64,
            ItemZone::Gain => it.gain_db,
        };
        let gesture = self.core.update(cx, |core, _| core.port_mut().begin_gesture("Item"));
        self.item_drag = Some(ItemDrag { item, zone, start: (ev.position.x.as_f32(), ev.position.y.as_f32()), start_value, gesture });
        cx.stop_propagation();
        cx.notify();
    }

    fn drag_item(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.item_drag else { return };
        let dx = (ev.position.x.as_f32() - d.start.0) as f64 * self.view.samples_per_px;
        let dy = (ev.position.y.as_f32() - d.start.1) as f64;
        let (field, value) = match d.zone {
            ItemZone::Move => {
                // snapped to the beat, unless Shift
                let beat = (BAR / 4) as f64;
                let s = (d.start_value + dx).max(0.0);
                (Field::ItemStart, if ev.modifiers.shift { s } else { (s / beat).round() * beat })
            }
            ItemZone::FadeIn => (Field::FadeIn, (d.start_value + dx).max(0.0)),
            ItemZone::FadeOut => (Field::FadeOut, (d.start_value - dx).max(0.0)),
            ItemZone::Gain => (Field::ItemGain, (d.start_value - dy * 0.3).clamp(-60.0, 12.0)),
        };
        let op = EditOp::set(d.item, field, Value::Real(value));
        self.core.update(cx, |core, _| core.port_mut().gesture_apply(&self.item_drag.as_ref().unwrap().gesture, vec![op]));
        cx.notify();
    }

    fn end_item(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.item_drag.take() {
            self.core.update(cx, |core, _| core.port_mut().gesture_commit(d.gesture));
            cx.notify();
        }
    }

    /// The track of the selected item: what Record arms when nothing is armed (F-REC-021)
    pub fn selected_track(&self, root: &Doc) -> Option<NodeId> {
        let it = self.selected_item?;
        root.tracks.iter().find(|t| t.items.iter().any(|i| i.id == it)).map(|t| t.id)
    }

    /// A click on the ruler moves the cursor there
    fn on_ruler(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let b = *self.timeline_bounds.borrow();
        let s = self.view.scroll_samples + (ev.position.x - b.origin.x).as_f32() as f64 * self.view.samples_per_px;
        self.core.update(cx, |core, _| {
            let _ = core.port_mut().run_action("transport.locate", Value::Real(s));
        });
        cx.notify();
    }
}

impl Render for CompositionPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let view_h = Self::view_h(window);
        let content_h = self.content_h(cx);
        // I-3.6: drain the feed; only the labels that changed are rebuilt
        {
            let core = self.core.read(cx);
            let port = core.port();
            if let Some(diff) = port.drain(&mut self.feed) {
                let tracks = &port.root().tracks;
                for (id, _) in &diff.changed {
                    if let Some(i) = tracks.iter().position(|t| t.id == *id) {
                        self.track_db[i] = view::format_db(tracks[i].volume_db).into();
                    }
                }
            }
            if let Some(v) = core.bench.view(now) {
                self.view.scroll_samples = v.scroll_samples;
                self.view.samples_per_px = v.samples_per_px;
                self.view.scroll_y = v.scroll_y * (content_h - view_h).max(0.0);
            }
        }
        self.view.scroll_y = self.view.scroll_y.clamp(0.0, (content_h - view_h).max(0.0));
        let pos = self.core.read(cx).port().telemetry().position;
        let cursor = pos.at(now);
        if self.follow_cursor && pos.playing {
            let w = window.viewport_size().width.as_f32() as f64 - view::HEADER_W as f64;
            let x = self.view.sample_to_x(cursor);
            if x < 0.0 || x > w * 0.9 {
                self.view.scroll_samples = (cursor - w * 0.1 * self.view.samples_per_px).max(0.0);
            }
        }
        let rows = layout(self.core.read(cx).port().root(), self.track_h);
        let y0 = self.view.scroll_y as f32;
        let visible: Vec<Row> =
            rows.into_iter().filter(|r| r.top + r.h >= y0 && r.top <= y0 + view_h as f32).collect();
        let headers = self.render_headers(&visible, cursor, cx).into_any_element();
        let timeline = self.render_timeline(&visible, cursor, cx);
        // the waveform blocks this frame wants, asked of the port before paint (a Waiting one: asked next frame)
        let tiles = self.tiles.clone();
        let waiting = self.core.update(cx, |core, _| {
            let mut t = tiles.borrow_mut();
            t.fetch(core.port_mut());
            t.waiting()
        });
        if waiting {
            window.request_animation_frame();
        }
        div()
            .size_full()
            .flex()
            .flex_row()
            .overflow_hidden()
            .key_context("Composition")
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, cx.listener(|p, _, window, cx| window.focus(&p.focus, cx)))
            .on_action(cx.listener(|p, _: &Split, _, cx| p.item_action("item.split", cx)))
            .on_action(cx.listener(|p, _: &Duplicate, _, cx| p.item_action("item.duplicate", cx)))
            .on_action(cx.listener(|p, _: &DeleteItem, _, cx| p.delete_item(cx)))
            .on_action(cx.listener(|p, _: &AddMarker, _, cx| p.action("marker.add", Value::Bool(true), cx)))
            .on_action(cx.listener(|p, _: &RegionFromLoop, _, cx| p.action("region.from_loop", Value::Bool(true), cx)))
            .on_action(cx.listener(|p, _: &EmptyItem, _, cx| p.empty_item(cx)))
            .on_action(cx.listener(|p, _: &Marker1, _, cx| p.action("marker.goto", Value::Real(0.0), cx)))
            .on_action(cx.listener(|p, _: &Marker2, _, cx| p.action("marker.goto", Value::Real(1.0), cx)))
            .on_action(cx.listener(|p, _: &Marker3, _, cx| p.action("marker.goto", Value::Real(2.0), cx)))
            .on_action(cx.listener(|p, _: &Marker4, _, cx| p.action("marker.goto", Value::Real(3.0), cx)))
            .on_action(cx.listener(|p, _: &Marker5, _, cx| p.action("marker.goto", Value::Real(4.0), cx)))
            .on_action(cx.listener(|p, _: &Marker6, _, cx| p.action("marker.goto", Value::Real(5.0), cx)))
            .on_action(cx.listener(|p, _: &Marker7, _, cx| p.action("marker.goto", Value::Real(6.0), cx)))
            .on_action(cx.listener(|p, _: &Marker8, _, cx| p.action("marker.goto", Value::Real(7.0), cx)))
            .on_action(cx.listener(|p, _: &Marker9, _, cx| p.action("marker.goto", Value::Real(8.0), cx)))
            .on_mouse_move(cx.listener(|p, ev: &MouseMoveEvent, _, cx| {
                p.drag_knob(ev, cx);
                p.drag_item(ev, cx);
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|p, _: &MouseUpEvent, _, cx| {
                p.end_knob(cx);
                p.end_item(cx);
            }))
            .child(headers)
            .child(timeline)
    }
}

// -----------------------------------------------------------------------------------------------------------------
// Headers

impl CompositionPage {
    fn render_headers(&self, rows: &[Row], cursor: f64, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let d = core.port().root();
        let bpm = arrange::tempo_at(&d.song.tempo, cursor / BAR as f64);
        let top0 = view::RULER_H + TEMPO_H;
        div()
            .w(px(view::HEADER_W))
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(c(THEME.bg1))
            .border_r_1()
            .border_color(c(THEME.line))
            .child(div().h(px(view::RULER_H)).bg(c(THEME.bg3)))
            // the tempo lane's header
            .child(
                div()
                    .h(px(TEMPO_H))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .bg(c(THEME.bg2))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(div().text_size(px(11.)).font_weight(FontWeight::BOLD).child(tr("Tempo")))
                    .child(div().text_size(px(11.)).text_color(c(THEME.text2)).child(format!("♩ {bpm:.2}")))
                    .child(div().flex_1())
                    .child(div().text_size(px(10.)).text_color(c(THEME.text3)).child(format!("{}/{}", d.song.signature.0, d.song.signature.1))),
            )
            .children(rows.iter().map(|r| {
                let top = top0 + r.top - self.view.scroll_y as f32;
                let el: AnyElement = match r.kind {
                    RowKind::Track(i) => self.track_header(i, r, d, core, cx),
                    RowKind::Take(i, k, tk) => self.take_header(i, k, tk, r, d),
                    RowKind::Lane(i, l) => self.lane_header(i, l, r, d, cursor),
                };
                div().absolute().top(px(top)).left(px(0.)).w(px(view::HEADER_W)).h(px(r.h)).child(el)
            }))
    }

    fn track_header(&self, i: usize, r: &Row, d: &Doc, core: &Core, cx: &Context<Self>) -> AnyElement {
        let t = &d.tracks[i];
        let tele_inputs = core.port().telemetry().inputs;
        let indent = r.depth as f32 * INDENT;
        let number = div()
            .w(px(22.))
            .h_full()
            .bg(c(track_color(i)))
            .text_size(px(10.))
            .font_weight(FontWeight::BOLD)
            .text_color(c(THEME.on_accent))
            .pt(px(4.))
            .flex()
            .justify_center()
            .child(format!("{}", i + 1));
        let base = div()
            .size_full()
            .flex()
            .flex_row()
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .pl(px(indent))
            .child(number);
        if t.kind == TrackKind::Folder {
            // a folder: collapse, name, M and S (it organizes and mutes or solos the group; it does not sum audio)
            return base
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(6.))
                        .child(
                            button(SharedString::from(format!("fold{i}")), if t.collapsed { "▸" } else { "▾" }, false, 0)
                                .on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Collapsed, cx))),
                        )
                        .child(div().text_size(px(12.)).font_weight(FontWeight::BOLD).flex_1().child(format!("📁 {}", t.name)))
                        .child(button(SharedString::from(format!("m{i}")), "M", t.mute, THEME.orange).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Mute, cx))))
                        .child(button(SharedString::from(format!("s{i}")), "S", t.solo, THEME.yellow).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Solo, cx)))),
                )
                .into_any_element();
        }
        let icon = if t.kind == TrackKind::Audio { "🎤" } else { "🎹" };
        let kv = view::db_to_knob(t.volume_db);
        let has_takes = t.items.iter().any(|it| matches!(&it.kind, ItemKind::Audio { takes, .. } if !takes.is_empty()));
        let outs = t.outputs.len();
        base.child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(6.))
                // 2, 3: record, the name with the type, M and S
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(4.))
                        .items_center()
                        .child(button(SharedString::from(format!("r{i}")), "●", t.armed, THEME.red).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Armed, cx))))
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
                                .child(format!("{icon} {}", t.name)),
                        )
                        .child(button(SharedString::from(format!("m{i}")), "M", t.mute, THEME.orange).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Mute, cx))))
                        .child(button(SharedString::from(format!("s{i}")), "S", t.solo, THEME.yellow).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Solo, cx)))),
                )
                // 4: the volume knob (no pan: I-3.9), its value, the outputs, takes and lanes, FX, I/O
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(4.))
                        .items_center()
                        .child(
                            div()
                                .id(("tknob", i))
                                .debug_selector(move || format!("tknob-{i}"))
                                .cursor_ns_resize()
                                .child(knob_face(kv, 22.0))
                                .on_mouse_down(MouseButton::Left, cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.begin_knob(i, ev, cx))),
                        )
                        .child(div().w(px(42.)).text_size(px(9.5)).text_color(c(THEME.text2)).child(self.track_db[i].clone()))
                        .when(t.monitor, |d| d.child(button(SharedString::from(format!("mon{i}")), "◉", true, THEME.blue).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Monitor, cx)))))
                        .child(div().flex_1())
                        .when(has_takes, |d| d.child(button(SharedString::from(format!("tk{i}")), tr("Takes"), t.show_takes, THEME.blue).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::ShowTakes, cx)))))
                        .when(!t.lanes.is_empty(), |d| d.child(button(SharedString::from(format!("ln{i}")), "A", t.show_lanes, THEME.blue).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::ShowLanes, cx)))))
                        .child(button(SharedString::from(format!("fx{i}")), "FX", false, 0).on_click(cx.listener(move |p, _, window, cx| p.open_fx(i, window, cx))))
                        .child(button(SharedString::from(format!("io{i}")), if outs > 1 { trf("I/O ×{0}", &[&outs]) } else { "I/O".into() }, outs > 1, THEME.green)),
                ),
        )
        // the meter on the right border: the main output; a dot says there are more (spec item 2)
        .child(
            div()
                .p(px(3.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .gap(px(2.))
                        // armed: the input's level beside the output's, even without monitoring (F-REC-009)
                        .when(t.armed, |d| d.child(meter(fake_core::view::meter_fill(tele_inputs[i % 8]), 4.0, r.h - (if outs > 1 { 14.0 } else { 6.0 }))))
                        .child(meter(core.shown[i], 6.0, r.h - (if outs > 1 { 14.0 } else { 6.0 }))),
                )
                .when(outs > 1, |d| d.child(div().size(px(5.)).rounded(px(3.)).bg(c(THEME.green)))),
        )
        .into_any_element()
    }

    fn take_header(&self, i: usize, k: usize, tk: usize, r: &Row, d: &Doc) -> AnyElement {
        let it = &d.tracks[i].items[k];
        let ItemKind::Audio { takes, comp, .. } = &it.kind else { return div().into_any_element() };
        let uses = comp.iter().filter(|seg| seg.take == takes[tk].id).count();
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(30. + r.depth as f32 * INDENT))
            .bg(c(THEME.bg1))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(div().text_size(px(11.)).child(trf("Take {0}", &[&(tk + 1)])))
            .child(div().text_size(px(10.)).text_color(c(if uses > 0 { THEME.green } else { THEME.text3 })).child(trf("used in {0} parts", &[&uses])))
            .into_any_element()
    }

    fn lane_header(&self, i: usize, l: usize, r: &Row, d: &Doc, cursor: f64) -> AnyElement {
        let lane = &d.tracks[i].lanes[l];
        let v = lane_value(&lane.points, cursor as i64);
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(30. + r.depth as f32 * INDENT))
            .bg(c(THEME.bg1))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(div().text_size(px(11.)).child(tr(lane.param)))
            .child(div().text_size(px(10.)).text_color(c(THEME.text2)).child(format!("{:.0}%", v * 100.0)))
            .into_any_element()
    }
}

/// An automation lane's value at a sample (linear between points; the curve is drawn bent, the value is linear here)
fn lane_value(points: &[(i64, f64, f32)], at: i64) -> f64 {
    match points.iter().position(|p| p.0 > at) {
        None => points.last().map_or(0.0, |p| p.1),
        Some(0) => points[0].1,
        Some(k) => {
            let (a, b) = (points[k - 1], points[k]);
            a.1 + (b.1 - a.1) * (at - a.0) as f64 / (b.0 - a.0) as f64
        }
    }
}

// -----------------------------------------------------------------------------------------------------------------
// Timeline

/// What the paint closure needs, gathered in render (the closure is 'static and cannot borrow the session)
enum Paint {
    Lane { y: f32, h: f32 },
    Item { y: f32, h: f32, x: f32, w: f32, color: u32, body: Body },
    Handle { y: f32, h: f32, x: f32, w: f32 },
    Boundary { y: f32, h: f32, x: f32, fade_px: f32 },
    Curve { y: f32, h: f32, pts: Vec<(f32, f32)> },
    /// an item's fades, gain line, selection and the automatic crossfade of overlapping items
    Edges { y: f32, h: f32, x: f32, w: f32, fade_in: f32, fade_out: f32, gain_db: f64, selected: bool, reversed: bool },
    Overlap { y: f32, h: f32, x: f32, w: f32 },
}

enum Body {
    /// audio: (resource, x of the audio's sample 0, x0..x1 to show)
    Wave(Vec<(ResourceId, f32, f32, f32)>),
    /// MIDI: notes as (x, w, key 0..1, velocity), and ghost copies for the repeats
    Notes(Vec<(f32, f32, f32, f32)>, Vec<(f32, f32)>),
    Recording(ResourceId, f32),
    Dim(ResourceId, f32, Vec<(f32, f32)>),
}

impl CompositionPage {
    fn render_timeline(&self, rows: &[Row], cursor: f64, cx: &Context<Self>) -> AnyElement {
        let core = self.core.read(cx);
        let port = core.port();
        let d = port.root();
        let v = self.view;
        let top0 = view::RULER_H + TEMPO_H;
        let sy = v.scroll_y as f32;
        let xs = |sample: f64| v.sample_to_x(sample) as f32;
        let mut paint = Vec::new();
        self.hits.borrow_mut().clear();
        let mut overlays: Vec<AnyElement> = Vec::new();
        let tele = port.telemetry();
        let rec = &tele.recording;
        // the tiles each waveform needs: asked of the port after render, drawn in paint (tiles.rs)
        let mut wants: Vec<(ResourceId, f32, f32, f32)> = Vec::new();
        for r in rows {
            let y = top0 + r.top - sy;
            paint.push(Paint::Lane { y, h: r.h });
            match r.kind {
                RowKind::Track(i) => {
                    let t = &d.tracks[i];
                    let color = track_color(i);
                    for it in &t.items {
                        let x = xs(it.start as f64);
                        let w = (it.length as f64 / v.samples_per_px) as f32;
                        if x > 4000.0 || x + w < 0.0 {
                            continue;
                        }
                        let body = match &it.kind {
                            ItemKind::Audio { resource, takes, comp, handle_before, handle_after } => {
                                if *handle_before > 0 {
                                    let hw = (*handle_before as f64 / v.samples_per_px) as f32;
                                    paint.push(Paint::Handle { y, h: r.h, x: x - hw, w: hw });
                                }
                                if *handle_after > 0 {
                                    let hw = (*handle_after as f64 / v.samples_per_px) as f32;
                                    paint.push(Paint::Handle { y, h: r.h, x: x + w, w: hw });
                                }
                                if comp.is_empty() {
                                    wants.push((*resource, x, x, x + w));
                                    Body::Wave(vec![(*resource, x, x, x + w)])
                                } else {
                                    // the comp: each segment plays its take, with a crossfade at each boundary (U2)
                                    let mut parts = Vec::new();
                                    for (k, seg) in comp.iter().enumerate() {
                                        let a = x + (seg.start as f64 / v.samples_per_px) as f32;
                                        let b = comp.get(k + 1).map_or(x + w, |n| x + (n.start as f64 / v.samples_per_px) as f32);
                                        if let Some(tk) = takes.iter().find(|t| t.id == seg.take) {
                                            wants.push((tk.resource, x, a, b));
                                            parts.push((tk.resource, x, a, b));
                                        }
                                        if k > 0 {
                                            let fade = (seg.crossfade_ms / 1000.0 * SAMPLE_RATE as f64 / v.samples_per_px) as f32;
                                            paint.push(Paint::Boundary { y, h: r.h, x: a, fade_px: fade.max(3.0) });
                                        }
                                    }
                                    Body::Wave(parts)
                                }
                            }
                            ItemKind::Midi { notes, repeats } => {
                                let ns = notes
                                    .iter()
                                    .map(|n| {
                                        let nx = x + (n.start as f64 / v.samples_per_px) as f32;
                                        let nw = ((n.length as f64 / v.samples_per_px) as f32).max(1.0);
                                        (nx, nw, (n.key as f32 - 30.0) / 50.0, n.velocity)
                                    })
                                    .collect();
                                let ghosts = (1..=*repeats).map(|k| (x + w * k as f32, w)).collect();
                                Body::Notes(ns, ghosts)
                            }
                        };
                        paint.push(Paint::Item { y, h: r.h, x, w, color, body });
                        let fi = (it.fades.0 as f64 / v.samples_per_px) as f32;
                        let fo = (it.fades.1 as f64 / v.samples_per_px) as f32;
                        paint.push(Paint::Edges { y, h: r.h, x, w, fade_in: fi, fade_out: fo, gain_db: it.gain_db, selected: self.selected_item == Some(it.id), reversed: it.reversed });
                        self.hits.borrow_mut().push((x, y + 3.0, w, r.h - 6.0, it.id, fi, fo));
                        // the item's name, and the loading component in place of the waveform (I-2)
                        let target = Target::Node(it.id);
                        let loading = port.loading(&target);
                        let core_handle = self.core.clone();
                        if let Some(el) = loading_component(SharedString::from(format!("load{}", it.id.number)), &loading, move |_, cx| {
                            core_handle.update(cx, |core, cx| {
                                for a in core.port().activity().into_iter().filter(|a| a.target == target || matches!(a.target, Target::Resource(_))) {
                                    core.port_mut().cancel(a.op);
                                }
                                cx.notify();
                            });
                        }) {
                            let x0 = x.max(0.0);
                            overlays.push(div().absolute().top(px(y + 3.0)).left(px(x0)).w(px((x + w - x0).clamp(0.0, 4000.0))).h(px(r.h - 6.0)).child(el).into_any_element());
                        } else if w > 40.0 {
                            overlays.push(
                                div()
                                    .absolute()
                                    .top(px(y + 4.0))
                                    .left(px(x.max(0.0) + 4.0))
                                    .max_w(px(w.min(400.0)))
                                    .text_size(px(10.))
                                    .text_color(c(THEME.text))
                                    .overflow_hidden()
                                    .child(it.name.clone())
                                    .into_any_element(),
                            );
                        }
                    }
                    // overlapping items cross-fade by themselves (F-CLP-062): the overlap is marked
                    let mut spans: Vec<(i64, i64)> = t.items.iter().map(|it| (it.start, it.start + it.length)).collect();
                    spans.sort();
                    for w2 in spans.windows(2) {
                        if w2[1].0 < w2[0].1 {
                            let (a, z) = (xs(w2[1].0 as f64), xs(w2[0].1.min(w2[1].1) as f64));
                            paint.push(Paint::Overlap { y, h: r.h, x: a, w: z - a });
                        }
                    }
                    // the take growing while recording (U3), with its pre-roll handle (U4)
                    for take in rec.iter().filter(|rt| rt.track == t.id) {
                        let x = xs(take.start as f64);
                        let w = (take.length as f64 / v.samples_per_px) as f32;
                        let pre = (d.song.pre_roll_bars as i64 * BAR) as f64 / v.samples_per_px;
                        paint.push(Paint::Handle { y, h: r.h, x: x - pre as f32, w: pre as f32 });
                        wants.push((take.resource, x, x, x + w));
                        paint.push(Paint::Item { y, h: r.h, x, w, color: THEME.red, body: Body::Recording(take.resource, x) });
                    }
                }
                RowKind::Take(i, k, tk) => {
                    let it = &d.tracks[i].items[k];
                    if let ItemKind::Audio { takes, comp, .. } = &it.kind {
                        let x = xs(it.start as f64);
                        let w = (it.length as f64 / v.samples_per_px) as f32;
                        let used: Vec<(f32, f32)> = comp
                            .iter()
                            .enumerate()
                            .filter(|(_, seg)| seg.take == takes[tk].id)
                            .map(|(n, seg)| {
                                let a = x + (seg.start as f64 / v.samples_per_px) as f32;
                                let b = comp.get(n + 1).map_or(x + w, |nx| x + (nx.start as f64 / v.samples_per_px) as f32);
                                (a, b)
                            })
                            .collect();
                        wants.push((takes[tk].resource, x, x, x + w));
                        paint.push(Paint::Item { y, h: r.h, x, w, color: track_color(i), body: Body::Dim(takes[tk].resource, x, used) });
                    }
                }
                RowKind::Lane(i, l) => {
                    let lane = &d.tracks[i].lanes[l];
                    let pts = lane.points.iter().map(|p| (xs(p.0 as f64), (1.0 - p.1 as f32))).collect();
                    paint.push(Paint::Curve { y, h: r.h, pts });
                }
            }
        }
        // the regions, in the tempo lane's top: a click makes one the loop (F-REG-011)
        for reg in &d.song.regions {
            let (a, z) = (xs(reg.start as f64), xs(reg.end as f64));
            if z < 0.0 || a > 2400.0 {
                continue;
            }
            let rid = reg.id;
            overlays.push(
                div()
                    .id(SharedString::from(format!("reg{}", rid.number)))
                    .absolute()
                    .top(px(view::RULER_H + 1.0))
                    .left(px(a.max(0.0)))
                    .w(px((z - a.max(0.0)).max(4.0)))
                    .h(px(11.))
                    .px(px(4.))
                    .bg(gpui::rgba(THEME.selection))
                    .text_size(px(8.5))
                    .text_color(c(THEME.text))
                    .overflow_hidden()
                    .cursor_pointer()
                    .child(reg.name.clone())
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.core.update(cx, |core, _| {
                            let _ = core.port_mut().run_action("loop.from_region", Value::Id(rid));
                        });
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        // the ruler: bars, loop, punch, markers (texts as elements)
        let song = d.song.clone();
        {
            let mut tiles = self.tiles.borrow_mut();
            let level = level_for(v.samples_per_px);
            for (res, ox, a, z) in wants {
                tiles.want(res, level, tile_range(level, v.samples_per_px, ox, a.max(0.0), z.min(2400.0)));
            }
        }
        let bar = BAR as f64;
        let step = bar * ((80.0 * v.samples_per_px / bar).max(1.0)).ceil();
        let mut sample = (v.scroll_samples / step).floor() * step;
        while v.sample_to_x(sample) < 2400.0 {
            let x = v.sample_to_x(sample) as f32;
            if x >= -40.0 {
                overlays.push(div().absolute().top(px(2.)).left(px(x + 3.0)).text_size(px(10.)).text_color(c(THEME.text2)).child(format!("{}", (sample / bar).round() as i64 + 1)).into_any_element());
            }
            sample += step;
        }
        for m in &song.markers {
            let x = xs(m.at as f64);
            if (-100.0..2400.0).contains(&x) {
                overlays.push(
                    div()
                        .absolute()
                        .top(px(view::RULER_H + 2.0))
                        .left(px(x + 2.0))
                        .px(px(4.))
                        .rounded(px(3.))
                        .bg(c(THEME.purple))
                        .text_color(c(THEME.on_accent))
                        .text_size(px(9.5))
                        .child(m.name.clone())
                        .into_any_element(),
                );
            }
        }
        // tempo labels at each tempo point
        for p in &song.tempo {
            let x = xs(p.bar as f64 * bar);
            if (-60.0..2400.0).contains(&x) {
                overlays.push(div().absolute().top(px(view::RULER_H + TEMPO_H - 13.0)).left(px(x + 3.0)).text_size(px(9.)).text_color(c(THEME.text3)).child(format!("{:.0}{}", p.bpm, if p.ramp { " ↗" } else { "" })).into_any_element());
            }
        }
        let tempo_pts: Vec<(f32, f32)> = {
            let mut pts = Vec::new();
            for (k, p) in song.tempo.iter().enumerate() {
                let x = xs(p.bar as f64 * bar);
                let yv = 1.0 - ((p.bpm - 60.0) / 120.0) as f32;
                if k > 0 && !song.tempo[k - 1].ramp {
                    // a step: flat to the point, then jump
                    let prev = song.tempo[k - 1];
                    pts.push((x, 1.0 - ((prev.bpm - 60.0) / 120.0) as f32));
                }
                pts.push((x, yv));
            }
            if let Some(last) = song.tempo.last() {
                pts.push((4000.0, 1.0 - ((last.bpm - 60.0) / 120.0) as f32));
            }
            pts
        };
        let bounds_cell = self.timeline_bounds.clone();
        let tile_cache = self.tiles.clone();
        let playing_x = xs(cursor);
        let xrun_xs: Vec<f32> = tele.xrun_marks.iter().map(|m| xs(*m as f64)).collect();
        let loop_x = (xs(song.loop_range.start as f64), xs(song.loop_range.end as f64), song.loop_range.on);
        let punch_x = (xs(song.punch.start as f64), xs(song.punch.end as f64), song.punch.on);
        let spp = v.samples_per_px;
        div()
            .id("timeline")
            .flex_1()
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(c(THEME.bg1))
            .on_scroll(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_timeline_down))
            .child(
                canvas(
                    move |b, _, _| {
                        *bounds_cell.borrow_mut() = b;
                    },
                    move |b, _, window, _| {
                        let o = b.origin;
                        let w = b.size.width.as_f32();
                        let full_h = b.size.height.as_f32();
                        let at = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
                        let mut cache = tile_cache.borrow_mut();
                        // loop and punch ranges behind everything
                        for (a, z, on, col) in [(loop_x.0, loop_x.1, loop_x.2, THEME.loop_range), (punch_x.0, punch_x.1, punch_x.2, THEME.punch_range)] {
                            if on && z > 0.0 && a < w {
                                window.paint_quad(fill(Bounds::new(at(a.max(0.0), 0.0), size(px(z.min(w) - a.max(0.0)), px(full_h))), gpui::rgba(col)));
                            }
                        }
                        for p in &paint {
                            match p {
                                Paint::Lane { y, h } => {
                                    window.paint_quad(fill(Bounds::new(at(0.0, y + h - 1.0), size(b.size.width, px(1.0))), c(THEME.line)));
                                }
                                Paint::Handle { y, h, x, w: hw } => {
                                    let bb = Bounds::new(at(*x, y + 3.0), size(px(*hw), px(h - 6.0)));
                                    window.paint_quad(gpui::quad(bb, px(3.0), c(THEME.bg2), px(1.0), c(THEME.line2), Default::default()));
                                }
                                Paint::Boundary { y, h, x, fade_px } => {
                                    // the crossfade: a small X over the boundary
                                    let mut pb = PathBuilder::stroke(px(1.5));
                                    pb.move_to(at(x - fade_px, y + 4.0));
                                    pb.line_to(at(x + fade_px, y + h - 4.0));
                                    pb.move_to(at(x - fade_px, y + h - 4.0));
                                    pb.line_to(at(x + fade_px, y + 4.0));
                                    if let Ok(path) = pb.build() {
                                        window.paint_path(path, c(THEME.yellow));
                                    }
                                }
                                Paint::Edges { y, h, x, w: iw, fade_in, fade_out, gain_db, selected, reversed } => {
                                    let iy = y + 3.0;
                                    let ih = h - 6.0;
                                    // fades: a shaded corner triangle each, the handle in the top 7 px
                                    for (fx, fw, left) in [(*x, *fade_in, true), (x + iw - fade_out, *fade_out, false)] {
                                        if fw > 0.5 {
                                            let mut pb = PathBuilder::fill();
                                            if left {
                                                pb.move_to(at(fx, iy));
                                                pb.line_to(at(fx + fw, iy));
                                                pb.line_to(at(fx, iy + ih));
                                            } else {
                                                pb.move_to(at(fx, iy));
                                                pb.line_to(at(fx + fw, iy));
                                                pb.line_to(at(fx + fw, iy + ih));
                                            }
                                            pb.close();
                                            if let Ok(path) = pb.build() {
                                                window.paint_path(path, gpui::rgba(THEME.scrim));
                                            }
                                        }
                                    }
                                    // the gain: a line, lower when quieter (0 dB at the top)
                                    if *gain_db < -0.05 {
                                        let gy = iy + ((-gain_db / 60.0).min(1.0) as f32) * ih;
                                        window.paint_quad(fill(Bounds::new(at(x.max(0.0), gy), size(px((x + iw).min(w) - x.max(0.0)), px(1.0))), c(THEME.yellow)));
                                    }
                                    if *selected {
                                        window.paint_quad(gpui::outline(Bounds::new(at(x.max(0.0), iy), size(px((x + iw).min(w) - x.max(0.0)), px(ih))), c(THEME.cursor), gpui::BorderStyle::Solid));
                                    }
                                    if *reversed {
                                        window.paint_quad(fill(Bounds::new(at(x + iw - 10.0, iy + 2.0), size(px(6.0), px(6.0))), c(THEME.orange)));
                                    }
                                }
                                Paint::Overlap { y, h, x, w: ow } => {
                                    let mut pb = PathBuilder::stroke(px(1.5));
                                    pb.move_to(at(*x, y + 3.0));
                                    pb.line_to(at(x + ow, y + h - 3.0));
                                    pb.move_to(at(*x, y + h - 3.0));
                                    pb.line_to(at(x + ow, y + 3.0));
                                    if let Ok(path) = pb.build() {
                                        window.paint_path(path, c(THEME.cyan));
                                    }
                                }
                                Paint::Curve { y, h, pts } => {
                                    let mut pb = PathBuilder::stroke(px(1.5));
                                    for (k, (x, yv)) in pts.iter().enumerate() {
                                        let pt = at(*x, y + 4.0 + yv * (h - 8.0));
                                        if k == 0 { pb.move_to(pt) } else { pb.line_to(pt) }
                                    }
                                    if let Ok(path) = pb.build() {
                                        window.paint_path(path, c(THEME.blue));
                                    }
                                    for (x, yv) in pts {
                                        if (0.0..w).contains(x) {
                                            window.paint_quad(fill(Bounds::new(at(x - 2.5, y + 4.0 + yv * (h - 8.0) - 2.5), size(px(5.0), px(5.0))), c(THEME.blue)));
                                        }
                                    }
                                }
                                Paint::Item { y, h, x, w: iw, color, body } => {
                                    let x0 = x.max(0.0);
                                    let x1 = (x + iw).min(w);
                                    if x1 <= x0 {
                                        continue;
                                    }
                                    let iy = y + 3.0;
                                    let ih = h - 6.0;
                                    let mut bg = c(*color);
                                    bg.a = 0.18;
                                    window.paint_quad(gpui::quad(Bounds::new(at(x0, iy), size(px(x1 - x0), px(ih))), px(3.0), bg, px(1.0), c(*color), Default::default()));
                                    let clip_of = |a: f32, z: f32| Bounds::new(at(a.max(x0), iy), size(px((z.min(x1) - a.max(x0)).max(0.0)), px(ih)));
                                    match body {
                                        Body::Wave(parts) => {
                                            for (res, ox, a, z) in parts {
                                                let clip = clip_of(*a, *z);
                                                if clip.size.width <= px(0.0) {
                                                    continue;
                                                }
                                                paint_wave(window, &mut cache, *res, *color, spp, o.x + px(*ox), clip, o.y + px(iy + 12.0), px(ih - 12.0));
                                            }
                                        }
                                        Body::Notes(notes, ghosts) => {
                                            for (nx, nw, key, vel) in notes {
                                                if *nx + *nw < x0 || *nx > x1 {
                                                    continue;
                                                }
                                                let ny = iy + 12.0 + (1.0 - key.clamp(0.0, 1.0)) * (ih - 16.0);
                                                let mut col = c(*color);
                                                col.a = 0.5 + 0.5 * vel;
                                                window.paint_quad(fill(Bounds::new(at(nx.max(x0), ny), size(px((nx + nw).min(x1) - nx.max(x0)), px(3.0))), col));
                                            }
                                            for (gx, gw) in ghosts {
                                                // a repeat of the pattern: a ghost of the item
                                                let a = gx.max(0.0);
                                                let z = (gx + gw).min(w);
                                                if z > a {
                                                    let mut g = c(*color);
                                                    g.a = 0.07;
                                                    window.paint_quad(gpui::quad(Bounds::new(at(a, iy), size(px(z - a), px(ih))), px(3.0), g, px(1.0), c(THEME.line2), Default::default()));
                                                }
                                            }
                                        }
                                        Body::Recording(res, ox) => {
                                            paint_wave(window, &mut cache, *res, THEME.red, spp, o.x + px(*ox), clip_of(x0, x1), o.y + px(iy + 12.0), px(ih - 12.0));
                                        }
                                        Body::Dim(res, ox, used) => {
                                            paint_wave(window, &mut cache, *res, THEME.text3, spp, o.x + px(*ox), clip_of(x0, x1), o.y + px(iy), px(ih));
                                            for (a, z) in used {
                                                let cl = clip_of(*a, *z);
                                                if cl.size.width > px(0.0) {
                                                    paint_wave(window, &mut cache, *res, *color, spp, o.x + px(*ox), cl, o.y + px(iy), px(ih));
                                                    window.paint_quad(gpui::outline(cl, c(THEME.green), gpui::BorderStyle::Solid));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // ruler and tempo lane on top (they do not scroll vertically)
                        window.paint_quad(fill(Bounds::new(o, size(b.size.width, px(view::RULER_H))), c(THEME.bg3)));
                        window.paint_quad(fill(Bounds::new(at(0.0, view::RULER_H), size(b.size.width, px(TEMPO_H))), c(THEME.bg2)));
                        for (a, z, on, col) in [(loop_x.0, loop_x.1, loop_x.2, THEME.green), (punch_x.0, punch_x.1, punch_x.2, THEME.red)] {
                            if z > 0.0 && a < w {
                                let mut cc = c(col);
                                cc.a = if on { 0.9 } else { 0.25 };
                                window.paint_quad(fill(Bounds::new(at(a.max(0.0), view::RULER_H - 5.0), size(px(z.min(w) - a.max(0.0)), px(4.0))), cc));
                            }
                        }
                        let bar = BAR as f64;
                        let step = bar * ((80.0 * spp / bar).max(1.0)).ceil();
                        let mut s = (v.scroll_samples / step).floor() * step;
                        while v.sample_to_x(s) < w as f64 {
                            let x = v.sample_to_x(s) as f32;
                            window.paint_quad(fill(Bounds::new(at(x, 8.0), size(px(1.0), px(view::RULER_H - 8.0))), c(THEME.text3)));
                            s += step;
                        }
                        let mut pb = PathBuilder::stroke(px(1.5));
                        for (k, (x, yv)) in tempo_pts.iter().enumerate() {
                            let pt = at(*x, view::RULER_H + 4.0 + yv * (TEMPO_H - 8.0));
                            if k == 0 { pb.move_to(pt) } else { pb.line_to(pt) }
                        }
                        if let Ok(path) = pb.build() {
                            window.paint_path(path, c(THEME.orange));
                        }
                        // the xruns, as red marks (F-CLP-063)
                        for x in &xrun_xs {
                            if (0.0..w).contains(x) {
                                window.paint_quad(fill(Bounds::new(at(*x, 0.0), size(px(2.0), b.size.height)), gpui::rgba(THEME.punch_range)));
                                window.paint_quad(fill(Bounds::new(at(*x - 2.0, 0.0), size(px(6.0), px(6.0))), c(THEME.red)));
                            }
                        }
                        // the play cursor, interpolated between telemetry frames
                        if playing_x >= 0.0 && playing_x < w {
                            window.paint_quad(fill(Bounds::new(at(playing_x, 0.0), size(px(1.0), b.size.height)), c(THEME.cursor)));
                        }
                    },
                )
                .size_full(),
            )
            .child(
                // the ruler's hit area: a click moves the cursor
                div()
                    .id("ruler")
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .w_full()
                    .h(px(view::RULER_H))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(|p, ev: &MouseDownEvent, _, cx| p.on_ruler(ev, cx))),
            )
            .children(overlays)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui::{Modifiers, TestAppContext, point};

    use super::*;
    use crate::workspace::{self, Workspace};

    /// Drag an item by its body: one gesture, snapped to the beat, one undo step; then S splits it at the cursor
    #[gpui::test]
    fn move_and_split_an_item(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            workspace::bind_keys(cx);
        });
        let (ws, cx) = cx.add_window_view(|window, cx| Workspace::new(window, cx));
        cx.run_until_parked();
        let page = ws.read_with(cx, |ws, _| ws.composition.clone());
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        let bounds = page.read_with(cx, |p, _| *p.timeline_bounds.borrow());
        let (x, y, w, h, item, ..) = page.read_with(cx, |p, _| p.hits.borrow().iter().copied().find(|hit| hit.2 > 60.0 && hit.0 > 0.0).expect("an item is drawn"));
        let start0 = core.read_with(cx, |c, _| c.port().root().tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.id == item).unwrap().start);
        let p0 = point(bounds.origin.x + px(x + w / 2.0), bounds.origin.y + px(y + h / 2.0));
        cx.simulate_mouse_down(p0, MouseButton::Left, Modifiers::none());
        for k in 1..=5 {
            cx.simulate_mouse_move(point(p0.x + px(k as f32 * 20.0), p0.y), MouseButton::Left, Modifiers::none());
        }
        cx.simulate_mouse_up(point(p0.x + px(100.0), p0.y), MouseButton::Left, Modifiers::none());
        let start1 = core.read_with(cx, |c, _| c.port().root().tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.id == item).unwrap().start);
        assert!(start1 > start0, "moved right");
        assert_eq!(start1 % (BAR / 4), 0, "snapped to the beat");
        // S at the cursor, inside the item
        core.update(cx, |c, _| {
            let _ = c.port_mut().run_action("transport.locate", Value::Real((start1 + BAR) as f64));
        });
        std::thread::sleep(Duration::from_millis(60)); // the fake engine thread publishes the position
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        let before = core.read_with(cx, |c, _| c.port().root().tracks.iter().map(|t| t.items.len()).sum::<usize>());
        cx.dispatch_action(Split);
        let after = core.read_with(cx, |c, _| c.port().root().tracks.iter().map(|t| t.items.len()).sum::<usize>());
        assert_eq!(after, before + 1, "split in two");
        core.update(cx, |c, _| {
            let _ = c.port_mut().undo();
            let _ = c.port_mut().undo();
        });
        let back = core.read_with(cx, |c, _| c.port().root().tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.id == item).unwrap().start);
        assert_eq!(back, start0, "two undo steps: the split, then the whole drag");
    }
}
