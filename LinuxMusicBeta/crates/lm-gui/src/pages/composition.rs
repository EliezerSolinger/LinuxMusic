//! The composition page: the timeline with the track headers (design 02, no pan), folders, several outputs per
//! track, audio items with their take lanes and comp (U2), MIDI items with their notes, the take that grows while
//! recording (U3), the tempo lane, automation lanes, the ruler with bars, loop and punch, xrun marks; and editing:
//! items selected, moved, faded and gained by gesture, split, duplicated, deleted, created empty, opened in their
//! editor. Everything is read from the project tree (`model`) and changed through the port (`EditOp`s, gestures,
//! actions). Positions are in ticks; the tempo map (lm-time) converts the cursor's samples.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, Bounds, Context, Entity, FocusHandle, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PathBuilder, PinchEvent, Pixels, Render, ScrollDelta, ScrollEvent, SharedString, Subscription,
    WeakEntity, Window, canvas, div, fill, point, prelude::*, px, size,
};
use gpui_component::input::{Input, InputEvent, InputState};
use lm_gui_plugins::Gesture;
use lm_port::{EditOp, Feed, HostTime, Label, NodeId, PathStep, Ref, Resource, Target, Topic, Value};
use lm_time::TempoMap;

use crate::actions::{
    ArrCopy, ArrCut, ArrPaste, BarLeft, BarRight, DeleteItem, Duplicate, EmptyItem, Glue, InsertBar, OpenItem,
    RemoveBar, SelectAllItems, SelectNoItems, Split, TempoHere, ToolDraw, ToolKnife, ToolSelect, ZoomIn, ZoomOut, ZoomReset,
};
use crate::core::Core;
use crate::model::{self, ItemView, TrackView};
use crate::screens::Screen;
use crate::tiles::{TileCache, level_for, paint_wave, tile_range};
use crate::ui::{THEME, button, c, tbutton, knob_face, loading_component, meter, midi, tr, tr_str, track_color, trf};
use crate::view::{self, TimelineView};

mod edit;
mod gestures;
mod ruler;
mod tempo;

use self::edit::EditState;
use self::gestures::{fires, holds, pick_wheel};
use super::marks::{self, MarksState};
use crate::workspace::Workspace;

pub const TEMPO_H: f32 = 34.0;
const FOLDER_H: f32 = 30.0;
const TAKE_H: f32 = 34.0;
const LANE_H: f32 = 48.0;
const INDENT: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum RowKind {
    Track(usize),
    /// a take lane of a track's item: (track, item, lane)
    Take(usize, usize, usize),
    Automation(usize, usize),
}

#[derive(Clone, Copy, Debug)]
struct Row {
    kind: RowKind,
    /// in content coordinates (0 = the first row, under the tempo lane)
    top: f32,
    h: f32,
    depth: u8,
}

struct KnobDrag {
    track: NodeId,
    start_y: f32,
    start_t: f64,
    gesture: Gesture,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ItemZone {
    Move,
    FadeIn,
    FadeOut,
    /// The handle in the middle of a fade: its tension (F-CLP-066)
    FadeInCurve,
    FadeOutCurve,
    Gain,
    /// the right edge: the length (F-CLP-003)
    ResizeEnd,
    /// the left edge: trims the start, the content stays (F-CLP-003)
    TrimStart,
}

struct ItemDrag {
    item: NodeId,
    zone: ItemZone,
    start: (f32, f32),
    start_value: f64,
    /// the item's length when the drag began
    length: i64,
    /// the other selected items and their starts, moved with it
    others: Vec<(NodeId, i64)>,
    /// None for a copy: nothing changes until the button goes up (F-CLP-005)
    gesture: Option<Gesture>,
    /// Ctrl+drag on a selected item: the copy (F-CLP-005)
    copy: bool,
    /// The place the items would land: the distance in ticks and, when the tracks changed, each item's track
    /// (F-CLP-004)
    ghost: Option<(i64, Vec<(NodeId, NodeId)>)>,
}

/// What was drawn where in the last frame, relative to the timeline: (x, y, w, h, item, fade in px, fade out px)
/// A lane drawn in the last frame, relative to the timeline: (y, h, lane, track)
type LaneHit = (f32, f32, NodeId, NodeId);

type Hit = (f32, f32, f32, f32, NodeId, f32, f32);

pub struct CompositionPage {
    core: Entity<Core>,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    pub view: TimelineView,
    feed: Feed,
    drag: Option<KnobDrag>,
    item_drag: Option<ItemDrag>,
    pub selected_item: Option<NodeId>,
    /// The zoom kept in the interface document (`arrangement.ticksPerPx`): the last value written, and whether the
    /// saved one was read
    saved_zoom: f64,
    zoom_read: bool,
    /// The cursor is being dragged on the ruler (ruler.rs)
    cursor_drag: bool,
    /// The tempo lane's state: the point dragged, the menu, the BPM typed (tempo.rs)
    tempo: tempo::TempoLane,
    /// The interface's state (I-3.8): which tracks show their take lanes and their automation, the track height
    show_takes: HashSet<NodeId>,
    show_automation: HashSet<NodeId>,
    pub track_h: f32,
    /// The height a track was dragged to (F-TRK-030); the others have `track_h`
    pub track_heights: HashMap<NodeId, f32>,
    /// A track being dragged by its number (F-TRK-029) and one being resized by its bottom border (F-TRK-030)
    track_drag: Option<edit::TrackDrag>,
    height_drag: Option<(NodeId, f32, f32)>,
    /// The track whose name is being edited in place (F-TRK-027)
    renaming: Option<(NodeId, Entity<InputState>, Subscription)>,
    /// The volume knob's menu (F-CTL-006, I-21.2): the track and where it opened, in window coordinates
    knob_menu: Option<(NodeId, gpui::Point<Pixels>)>,
    /// An automation lane's menu: the lane, its track, the tick under the pointer, where it opened
    lane_menu: Option<(NodeId, NodeId, i64, gpui::Point<Pixels>)>,
    /// Items move with the automation under them (I-21.5, on by default): the interface's option
    pub move_automation: bool,
    /// The lanes drawn in the last frame, relative to the timeline: (y, h, lane, track)
    lane_hits: Rc<RefCell<Vec<LaneHit>>>,
    /// The track rows drawn in the last frame, relative to the timeline: (y, h, track)
    track_hits: Rc<RefCell<Vec<(f32, f32, NodeId)>>>,
    /// The page's bounds, for placing the menu
    page_bounds: Rc<RefCell<Bounds<Pixels>>>,
    timeline_bounds: Rc<RefCell<Bounds<Pixels>>>,
    tiles: Rc<RefCell<TileCache>>,
    hits: Rc<RefCell<Vec<Hit>>>,
    /// The ruler's markers and regions (agent D, marks.rs)
    pub(crate) marks: MarksState,
    /// The editing's state: tool, grid, selection, menus (edit.rs)
    ed: EditState,
}

/// How deep a track is in the folders, and whether a collapsed folder above it hides it
fn depth_of(tracks: &[TrackView], t: &TrackView) -> (u8, bool) {
    let (mut depth, mut hidden, mut at) = (0u8, false, t.parent);
    while let Some(pid) = at {
        let Some(parent) = tracks.iter().find(|x| x.id == pid) else { break };
        depth += 1;
        hidden |= parent.collapsed;
        at = parent.parent;
        if depth > 16 {
            break;
        }
    }
    (depth, hidden)
}

/// The rows: tracks not hidden by a collapsed folder (each with its own height, else `track_h`), then each track's
/// take lanes and automation lanes
fn layout(
    tracks: &[TrackView],
    track_h: f32,
    heights: &HashMap<NodeId, f32>,
    takes: &HashSet<NodeId>,
    automation: &HashSet<NodeId>,
) -> Vec<Row> {
    let mut rows = Vec::with_capacity(tracks.len() + 8);
    let mut top = 0.0;
    for (i, t) in tracks.iter().enumerate() {
        let (depth, hidden) = depth_of(tracks, t);
        if hidden {
            continue;
        }
        let h = if t.folder { FOLDER_H.min(track_h) } else { heights.get(&t.id).copied().unwrap_or(track_h) };
        rows.push(Row { kind: RowKind::Track(i), top, h, depth });
        top += h;
        if takes.contains(&t.id) {
            for (k, it) in t.items.iter().enumerate() {
                if it.lanes.len() > 1 {
                    for l in 0..it.lanes.len() {
                        rows.push(Row { kind: RowKind::Take(i, k, l), top, h: TAKE_H, depth });
                        top += TAKE_H;
                    }
                }
            }
        }
        if automation.contains(&t.id) {
            for a in 0..t.automation.len() {
                rows.push(Row { kind: RowKind::Automation(i, a), top, h: LANE_H, depth });
                top += LANE_H;
            }
        }
    }
    rows
}

impl CompositionPage {
    pub fn new(core: Entity<Core>, workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        let feed = core.update(cx, |core, _| core.port_mut().feed(Topic::All));
        let core_for_marks = core.clone();
        CompositionPage {
            core,
            workspace,
            focus: cx.focus_handle(),
            view: TimelineView::default(),
            feed,
            drag: None,
            item_drag: None,
            selected_item: None,
            saved_zoom: 0.0,
            zoom_read: false,
            cursor_drag: false,
            tempo: tempo::TempoLane::default(),
            show_takes: HashSet::new(),
            show_automation: HashSet::new(),
            track_h: view::TRACK_H,
            track_heights: HashMap::new(),
            track_drag: None,
            height_drag: None,
            renaming: None,
            knob_menu: None,
            lane_menu: None,
            move_automation: true,
            lane_hits: Default::default(),
            track_hits: Default::default(),
            page_bounds: Default::default(),
            timeline_bounds: Rc::new(RefCell::new(Bounds::default())),
            tiles: Rc::new(RefCell::new(TileCache::default())),
            hits: Default::default(),
            marks: MarksState::new(core_for_marks),
            ed: EditState::default(),
        }
    }

    #[cfg(all(test, debug_assertions))]
    pub(crate) fn view_for_tests(&self) -> TimelineView {
        self.view
    }

    /// Files dropped on the arrangement (F-ARR-011): at the bar under the pointer; audio as new audio tracks, MIDI
    /// as imported tracks, a project folder opens
    pub(crate) fn drop_files(&mut self, paths: &[std::path::PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        let b = *self.timeline_bounds.borrow();
        let tick = self.view.tick_at((window.mouse_position().x - b.origin.x).as_f32()).max(0.0) as i64;
        let map = self.core.read(cx).port().tempo_map();
        let bar = map.bar_start(map.bar_beat_tick(tick).bar).unwrap_or(0);
        crate::screens::files::import_paths(&self.core, paths, bar, cx);
    }

    /// A browser row dropped on the arrangement: at the bar under the pointer, on the track under it (F-BRW-007)
    pub(crate) fn drop_dragged(
        &mut self,
        d: &crate::screens::browser::Dragged,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let b = *self.timeline_bounds.borrow();
        let at = window.mouse_position();
        let tick = self.view.tick_at((at.x - b.origin.x).as_f32()).max(0.0) as i64;
        let map = self.core.read(cx).port().tempo_map();
        let bar = map.bar_start(map.bar_beat_tick(tick).bar).unwrap_or(0);
        let y = (at.y - b.origin.y).as_f32();
        let track = self.track_hits.borrow().iter().find(|(top, h, _)| y >= *top && y < top + h).map(|(_, _, id)| *id);
        crate::screens::browser_drop::drop_entry(&self.core, d, track, bar, cx);
    }

    /// A window position in the marks' lanes (x from the timeline's left, y from the lanes' top)
    pub(crate) fn marks_local(&self, at: gpui::Point<Pixels>) -> (f32, f32) {
        let b = *self.timeline_bounds.borrow();
        ((at.x - b.origin.x).as_f32(), (at.y - b.origin.y).as_f32() - view::RULER_H)
    }

    /// "Export the region as audio" (F-REG-012): the export window on that range
    pub(crate) fn export_region(&mut self, from: i64, to: i64, window: &mut Window, cx: &mut Context<Self>) {
        cx.set_global(crate::screens::export::ExportRange(Some((from, to))));
        let _ = self.workspace.update(cx, |ws, cx| ws.open(Screen::Export, None, window, cx));
    }

    fn tracks(&self, cx: &Context<Self>) -> Vec<TrackView> {
        model::tracks(self.core.read(cx).port().root())
    }

    fn apply(&mut self, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            // Pending: in-process at once; in the client the feed brings the change in a later pump
            core.report(|pt| pt.apply(Label::new(label), ops));
        });
        cx.notify();
    }

    fn action(&mut self, id: &str, params: Value, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            core.report(|pt| pt.run_action(id.into(), params));
        });
        cx.notify();
    }

    // --- the volume knob's gesture (I-3.7) -----------------------------------------------------------------

    fn begin_knob(&mut self, track: NodeId, volume: f64, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if ev.click_count == 2 || ev.button == MouseButton::Middle {
            return self.apply(
                "Volume",
                vec![EditOp::set(track, &["volume"], Value::real(1.0).unwrap_or_default())],
                cx,
            );
        }
        let prev = self.drag.take();
        let gesture = self.core.update(cx, |core, _| {
            if let Some(d) = prev {
                core.gesture_commit(d.gesture);
            }
            core.begin_gesture(Label::new("Volume"))
        });
        self.drag =
            Some(KnobDrag { track, start_y: ev.position.y.as_f32(), start_t: view::gain_to_travel(volume), gesture });
    }

    fn drag_knob(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.drag else { return };
        let t = view::knob_drag(
            d.start_t,
            (ev.position.y.as_f32() - d.start_y) as f64,
            holds("comp.knob.fine", &ev.modifiers),
        );
        let op = EditOp::set(d.track, &["volume"], Value::real(view::travel_to_gain(t)).unwrap_or_default());
        self.core.update(cx, |core, _| core.gesture_apply(self.drag.as_ref().unwrap().gesture, vec![op]));
        cx.notify();
    }

    fn toggle(&mut self, track: NodeId, key: &str, now: bool, cx: &mut Context<Self>) {
        self.apply("Toggle", vec![EditOp::set(track, &[key], Value::bool(!now))], cx);
    }

    /// Monitor: auto → on → off → auto
    fn cycle_monitor(&mut self, track: NodeId, now: &str, cx: &mut Context<Self>) {
        let next = match now {
            "auto" => "on",
            "on" => "off",
            _ => "auto",
        };
        self.apply("Monitor", vec![EditOp::set(track, &["monitor"], Value::text(next))], cx);
    }

    /// FX on a header: the inserts of the channel its main output goes to (P-28), on the mixing page
    fn open_fx(&mut self, to: Option<NodeId>, cx: &mut Context<Self>) {
        if let Some(ch) = to {
            let _ = self.workspace.update(cx, |ws, cx| {
                ws.page = crate::pages::Page::Mixing;
                ws.mixing.update(cx, |m, cx| m.select_channel(ch, cx));
                cx.notify();
            });
        }
    }

    // --- scroll, zoom, the ruler ---------------------------------------------------------------------------

    fn view_h(window: &Window) -> f64 {
        (window.viewport_size().height.as_f32() - crate::workspace::CHROME_H - view::RULER_H - marks::MARKS_H - TEMPO_H)
            as f64
    }

    fn content_h(&self, cx: &Context<Self>) -> f64 {
        layout(&self.tracks(cx), self.track_h, &self.track_heights, &self.show_takes, &self.show_automation)
            .last()
            .map_or(0.0, |r| (r.top + r.h) as f64)
    }

    /// The wheel (F-ARR-007) does what the mouse actions say (REAPER's by default: zoom, Ctrl track height, Alt or
    /// Shift sideways). A trackpad pans both ways, and zooms when the zoom gesture has a modifier held
    fn on_scroll(&mut self, ev: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let b = *self.timeline_bounds.borrow();
        let x = (ev.position.x - b.origin.x).as_f32() as f64;
        let (dx, dy) = (d.x.as_f32() as f64, d.y.as_f32() as f64);
        let wheel = matches!(ev.delta, ScrollDelta::Lines(_));
        let ids = [
            "comp.view.track_height_fine",
            "comp.view.track_height_coarse",
            "comp.view.track_height",
            "comp.view.zoom",
            "comp.view.scroll_h",
            "comp.view.scroll_v",
        ];
        // a mouse wheel does what the table says; a trackpad pans both ways and, with Cmd/Ctrl, zooms
        let which = if wheel { pick_wheel(&ids, &ev.modifiers) } else { None };
        match which {
            Some("comp.view.track_height_fine" | "comp.view.track_height_coarse") => {
                let step = if which == Some("comp.view.track_height_coarse") { 5.0 } else { 1.0 };
                self.track_h = (self.track_h + step * dy.signum() as f32).clamp(24.0, 160.0);
                self.track_heights.clear();
                cx.notify();
                return;
            }
            Some("comp.view.track_height") => {
                self.track_h = (self.track_h * (1.0 + dy as f32 * 0.004)).clamp(24.0, 160.0);
                self.track_heights.clear();
                cx.notify();
                return;
            }
            None if wheel => return,
            _ => {}
        }
        let sideways = which == Some("comp.view.scroll_h");
        let zoom = if wheel {
            which == Some("comp.view.zoom")
        } else {
            holds("comp.view.zoom", &ev.modifiers) && ev.modifiers.modified()
        };
        let (dx, dy) = if wheel && sideways { (dy, 0.0) } else { (dx, dy) };
        let content = self.content_h(cx);
        self.view.wheel(dx, if zoom { -dy } else { dy }, zoom, x, content, Self::view_h(window));
        self.remember_zoom(cx);
        cx.notify();
    }

    fn on_pinch(&mut self, ev: &PinchEvent, window: &mut Window, cx: &mut Context<Self>) {
        let b = *self.timeline_bounds.borrow();
        let x = (ev.position.x - b.origin.x).as_f32() as f64;
        let content = self.content_h(cx);
        let dy = -(1.0 + ev.delta as f64).ln() * 100.0;
        self.view.wheel(0.0, dy, true, x, content, Self::view_h(window));
        self.remember_zoom(cx);
        cx.notify();
    }

    fn zoom(&mut self, factor: f64, cx: &mut Context<Self>) {
        let w = self.timeline_bounds.borrow().size.width.as_f32() as f64 / 2.0;
        self.view.wheel(0.0, factor, true, w, 0.0, 0.0);
        self.remember_zoom(cx);
        cx.notify();
    }

    /// Ctrl+0 (F-ARR-007): the default zoom, around the middle of the view
    fn zoom_reset(&mut self, cx: &mut Context<Self>) {
        let w = self.timeline_bounds.borrow().size.width.as_f32() as f64 / 2.0;
        let tick = self.view.tick_at(w as f32);
        self.view.ticks_per_px = view::TimelineView::default().ticks_per_px;
        self.view.scroll_ticks = (tick - w * self.view.ticks_per_px).max(0.0);
        self.remember_zoom(cx);
        cx.notify();
    }

    /// A click on the ruler moves the cursor there
    fn on_ruler(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        // Ctrl+drag selects the items of every track in that stretch (F-REG-008)
        if holds("comp.ruler.range_select", &ev.modifiers) {
            let b = *self.timeline_bounds.borrow();
            let tick = self.view.tick_at((ev.position.x - b.origin.x).as_f32()).max(0.0) as i64;
            self.ed.ruler_select = Some((tick, tick));
            cx.notify();
            return;
        }
        if self.loop_down(ev, cx) {
            return;
        }
        self.ruler_down(ev, cx);
    }

    // --- items: select, move, fades, gain, the keys (F-CLP-061, F-ARR-005..007) --------------------------------

    /// The track of the selected item: what Record arms when nothing is armed (F-REC-021)
    pub fn selected_track(&self, root: &Value) -> Option<NodeId> {
        let it = self.selected_item?;
        model::tracks(root).into_iter().find(|t| t.items.iter().any(|i| i.id == it)).map(|t| t.id)
    }

    fn selected(&self, cx: &Context<Self>) -> Option<(TrackView, ItemView)> {
        let it = self.selected_item?;
        self.tracks(cx).into_iter().find_map(|t| t.items.iter().find(|i| i.id == it).cloned().map(|i| (t, i)))
    }

    fn cursor_tick(&self, cx: &Context<Self>) -> i64 {
        let port = self.core.read(cx).port();
        let map = port.tempo_map();
        map.tick_at(port.telemetry().position.at(HostTime::now()) as i64)
    }

    /// An empty audio item a bar long at the cursor, on the selected item's track or the first armed one (P-29)
    fn empty_item(&mut self, cx: &mut Context<Self>) {
        let root = self.core.read(cx).port().root().clone();
        let tracks = model::tracks(&root);
        let track = self.selected_track(&root).or_else(|| tracks.iter().find(|t| t.armed).map(|t| t.id));
        let Some(track) = track else { return };
        let n = tracks.iter().find(|t| t.id == track).map_or(0, |t| t.items.len());
        let bar = 4 * model::ppq(&root) as i64;
        let node = Value::object([
            ("kind", Value::text("audio")),
            ("name", Value::text(tr("Empty"))),
            ("start", Value::int(self.cursor_tick(cx))),
            ("length", Value::int(bar)),
        ]);
        self.apply(
            "Empty item",
            vec![EditOp::Insert { temp: 0, parent: Ref::Id(track), list: "items".into(), index: n, node }],
            cx,
        );
    }

    /// Enter or a double click: the item's editor (the Piano Roll for MIDI)
    fn open_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, item)) = self.selected(cx) else { return };
        if !item.audio {
            let id = item.id;
            let _ = self.workspace.update(cx, |ws, cx| ws.open(Screen::PianoRoll, Some(id), window, cx));
        }
    }

    /// Where an item is drawn, in window coordinates: a point in its body (the tests click there)
    #[cfg(all(test, debug_assertions))]
    pub fn item_point(&self, item: NodeId) -> Option<gpui::Point<Pixels>> {
        let b = *self.timeline_bounds.borrow();
        let (x, y, w, h, ..) = self.hits.borrow().iter().find(|h| h.4 == item).copied()?;
        Some(gpui::point(b.origin.x + px(x + w.min(40.0) / 2.0), b.origin.y + px(y + h * 0.6)))
    }
}

impl Render for CompositionPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let view_h = Self::view_h(window);
        if !self.zoom_read {
            self.read_zoom(cx);
        }
        // I-3.6: the feed is drained each frame; a removed item stops being selected
        let diff = {
            let core = self.core.read(cx);
            let port = core.port();
            port.drain(&mut self.feed)
        };
        if let (Some(diff), Some(sel)) = (&diff, self.selected_item)
            && diff.removed.contains(&sel)
        {
            self.selected_item = None;
        }
        let tracks = self.tracks(cx);
        let rows = layout(&tracks, self.track_h, &self.track_heights, &self.show_takes, &self.show_automation);
        let content_h = rows.last().map_or(0.0, |r| (r.top + r.h) as f64);
        if let Some(v) = self.core.read(cx).bench.view(now) {
            // the measurement's script: its samples read as ticks at 120 BPM (25 samples a tick)
            self.view.scroll_ticks = v.scroll_samples / 25.0;
            self.view.ticks_per_px = v.samples_per_px / 25.0;
            self.view.scroll_y = v.scroll_y * (content_h - view_h).max(0.0);
        }
        self.view.scroll_y = self.view.scroll_y.clamp(0.0, (content_h - view_h).max(0.0));
        let (cursor, playing) = {
            let port = self.core.read(cx).port();
            let map = port.tempo_map();
            let pos = port.telemetry().position;
            (map.tick_at(pos.at(HostTime::now()) as i64) as f64, pos.playing)
        };
        // the automatic scroll while playing (F-REG-003): by page, continuous, or off
        let (follow, continuous) = self.scroll_mode(cx);
        if follow && playing {
            let w = window.viewport_size().width.as_f32() - view::HEADER_W;
            let x = self.view.x(cursor);
            if continuous {
                self.view.scroll_ticks = (cursor - (w * 0.3) as f64 * self.view.ticks_per_px).max(0.0);
            } else if x < 0.0 || x > w * 0.9 {
                self.view.scroll_ticks = (cursor - (w * 0.1) as f64 * self.view.ticks_per_px).max(0.0);
            }
        }
        let y0 = self.view.scroll_y as f32;
        let visible: Vec<Row> = rows.into_iter().filter(|r| r.top + r.h >= y0 && r.top <= y0 + view_h as f32).collect();
        let headers = self.render_headers(&tracks, &visible, cx).into_any_element();
        let timeline = self.render_timeline(&tracks, &visible, cursor, cx);
        // the waveform blocks this frame wants, asked of the port before paint (a Waiting one: asked next frame)
        let tiles = self.tiles.clone();
        let waiting = self.core.update(cx, |core, _| {
            let mut t = tiles.borrow_mut();
            let answers = core.take_peak_answers();
            for e in t.fetch(core.port_mut(), answers) {
                core.fail(e);
            }
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
            .on_action(cx.listener(|p, _: &Split, _, cx| p.split(cx)))
            .on_action(cx.listener(|p, _: &Duplicate, _, cx| p.duplicate(cx)))
            .on_action(cx.listener(|p, _: &DeleteItem, _, cx| p.delete_item(cx)))
            .on_action(cx.listener(|p, _: &EmptyItem, _, cx| p.empty_item(cx)))
            .on_action(cx.listener(|p, _: &OpenItem, window, cx| p.open_item(window, cx)))
            .on_action(cx.listener(|p, _: &ArrCopy, _, cx| p.copy_sel(cx)))
            .on_action(cx.listener(|p, _: &ArrCut, _, cx| p.cut_sel(cx)))
            .on_action(cx.listener(|p, _: &ArrPaste, _, cx| p.paste_at_cursor(cx)))
            .on_action(cx.listener(|p, _: &Glue, _, cx| p.glue(cx)))
            .on_action(cx.listener(|p, _: &SelectAllItems, _, cx| p.select_all(cx)))
            .on_action(cx.listener(|p, _: &SelectNoItems, _, cx| p.select_none(cx)))
            .on_action(cx.listener(|p, _: &BarLeft, _, cx| p.cursor_bar(-1, cx)))
            .on_action(cx.listener(|p, _: &BarRight, _, cx| p.cursor_bar(1, cx)))
            .on_action(cx.listener(|p, _: &InsertBar, _, cx| p.shift_bar(true, cx)))
            .on_action(cx.listener(|p, _: &RemoveBar, _, cx| p.shift_bar(false, cx)))
            .on_action(cx.listener(|p, _: &ToolDraw, _, cx| p.set_tool(edit::Tool::Draw, cx)))
            .on_action(cx.listener(|p, _: &ToolKnife, _, cx| p.set_tool(edit::Tool::Knife, cx)))
            .on_action(cx.listener(|p, _: &ToolSelect, _, cx| p.set_tool(edit::Tool::Select, cx)))
            .on_action(cx.listener(|p, _: &TempoHere, window, cx| p.tempo_here(window, cx)))
            .on_action(cx.listener(|p, _: &ZoomIn, _, cx| p.zoom(-40.0, cx)))
            .on_action(cx.listener(|p, _: &ZoomOut, _, cx| p.zoom(40.0, cx)))
            .on_action(cx.listener(|p, _: &ZoomReset, _, cx| p.zoom_reset(cx)))
            .map(|d| marks::actions(d, cx))
            .on_mouse_move(cx.listener(|p, ev: &MouseMoveEvent, _, cx| {
                p.drag_knob(ev, cx);
                p.drag_item(ev, cx);
                p.edit_move(ev, cx);
                p.ruler_move(ev, cx);
                p.tempo_move(ev, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|p, _: &MouseUpEvent, _, cx| {
                    p.release(cx);
                    p.ruler_up();
                    p.tempo_up(cx);
                }),
            )
            .relative()
            .child({
                let pb = self.page_bounds.clone();
                canvas(move |b, _, _| *pb.borrow_mut() = b, |_, _, _, _| {}).absolute().size_full()
            })
            .child(headers)
            .child(timeline)
            .children(self.render_knob_menu(cx))
            .children(self.render_lane_menu(cx))
            .children(self.render_clip_menu(cx))
            .children(self.render_track_menu(cx))
            .children(self.render_io_menu(cx))
            .children(self.render_arm_menu(cx))
            .children(self.render_palette(cx))
            .children(self.render_loop_menu(cx))
            .children(self.render_confirm_remove(cx))
    }
}

impl CompositionPage {
    /// The right button on a lane: its menu
    fn on_timeline_right(&mut self, ev: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.right_down(ev, cx) {
            return;
        }
        let b = *self.timeline_bounds.borrow();
        let (mx, my) = ((ev.position.x - b.origin.x).as_f32(), (ev.position.y - b.origin.y).as_f32());
        let lane = self.lane_hits.borrow().iter().find(|(y, h, ..)| my >= *y && my <= y + h).copied();
        if let Some((_, _, lane, track)) = lane {
            cx.stop_propagation();
            let tick = self.view.tick_at(mx).max(0.0) as i64;
            self.lane_menu = Some((lane, track, tick, ev.position));
            cx.notify();
        }
    }

    /// An automation lane's menu: open it in the editor, thin it, convert it into the MIDI item's CC lane under the
    /// pointer, and the option of moving the automation with the items (I-21.5)
    fn render_lane_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (lane, track, tick, pos) = self.lane_menu?;
        let origin = self.page_bounds.borrow().origin;
        let item = |id: &'static str, label: String| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px(px(10.))
                .py(px(3.))
                .text_size(px(12.))
                .cursor_pointer()
                .hover(|d| d.bg(c(THEME.bg4)))
                .child(label)
        };
        let midi_item = self
            .tracks(cx)
            .into_iter()
            .find(|t| t.id == track)
            .and_then(|t| t.items.into_iter().find(|i| !i.audio && tick >= i.start && tick < i.start + i.length))
            .map(|i| i.id);
        let l1 = Value::text(lane.to_string());
        let l2 = l1.clone();
        let moving = self.move_automation;
        Some(
            div()
                .id("lane-menu")
                .absolute()
                .left(pos.x - origin.x)
                .top(pos.y - origin.y)
                .w(px(260.))
                .occlude()
                .py(px(4.))
                .rounded(px(6.))
                .bg(c(THEME.bg3))
                .border_1()
                .border_color(c(THEME.line2))
                .shadow_lg()
                .text_color(c(THEME.text))
                .on_mouse_down_out(cx.listener(|p, _, _, cx| {
                    p.lane_menu = None;
                    cx.notify();
                }))
                .child(item("lane-menu-open", tr("Open in the automation editor").into()).on_click(cx.listener(
                    move |p, _, window, cx| {
                        p.lane_menu = None;
                        let _ = p.workspace.update(cx, |ws, cx| ws.open(Screen::Automation, Some(lane), window, cx));
                    },
                )))
                .child(item("lane-menu-thin", tr("Thin (remove redundant points)").into()).on_click(cx.listener(
                    move |p, _, _, cx| {
                        p.lane_menu = None;
                        p.action("automation.thin", Value::object([("lane", l1.clone())]), cx);
                    },
                )))
                .children(midi_item.map(|it| {
                    item("lane-menu-to-item", tr("Convert to the MIDI item's CC lane").into()).on_click(cx.listener(
                        move |p, _, _, cx| {
                            p.lane_menu = None;
                            let params = Value::object([("lane", l2.clone()), ("item", Value::text(it.to_string()))]);
                            p.action("automation.to_item", params, cx);
                        },
                    ))
                }))
                .child(div().my(px(3.)).h(px(1.)).bg(c(THEME.line)))
                .child(
                    item(
                        "lane-menu-move-with-items",
                        format!("{}{}", if moving { "✓ " } else { "" }, tr("Move automation with items")),
                    )
                    .on_click(cx.listener(|p, _, _, cx| {
                        p.lane_menu = None;
                        p.move_automation = !p.move_automation;
                        cx.notify();
                    })),
                )
                .into_any_element(),
        )
    }

    /// The volume knob's menu: back to 0 dB, and the MIDI items (learn, remove the mapping, the binding window)
    fn render_knob_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (track, pos) = self.knob_menu?;
        let origin = self.page_bounds.borrow().origin;
        let me = cx.weak_entity();
        let close = move |cx: &mut gpui::App| {
            let _ = me.update(cx, |p, cx| {
                p.knob_menu = None;
                cx.notify();
            });
        };
        let mut items = vec![
            div()
                .id("knob-menu-reset")
                .debug_selector(|| "knob-menu-reset".into())
                .px(px(10.))
                .py(px(3.))
                .text_size(px(12.))
                .cursor_pointer()
                .hover(|d| d.bg(c(THEME.bg4)))
                .child(tr("Reset"))
                .on_click(cx.listener(move |p, _, _, cx| {
                    p.knob_menu = None;
                    p.apply("Volume", vec![EditOp::set(track, &["volume"], Value::real(1.0).unwrap_or_default())], cx);
                }))
                .into_any_element(),
        ];
        items.extend(midi::menu_items(&self.core, &self.workspace, midi::param(track, "volume"), close, cx));
        Some(
            div()
                .id("knob-menu")
                .absolute()
                .left(pos.x - origin.x)
                .top(pos.y - origin.y)
                .w(px(220.))
                .occlude()
                .py(px(4.))
                .rounded(px(6.))
                .bg(c(THEME.bg3))
                .border_1()
                .border_color(c(THEME.line2))
                .shadow_lg()
                .text_color(c(THEME.text))
                .on_mouse_down_out(cx.listener(|p, _, _, cx| {
                    p.knob_menu = None;
                    cx.notify();
                }))
                .children(items)
                .into_any_element(),
        )
    }
}

// -----------------------------------------------------------------------------------------------------------------
// Headers

impl CompositionPage {
    fn render_headers(&self, tracks: &[TrackView], rows: &[Row], cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let root = core.port().root();
        let map = model::tempo_map(root);
        let tick = map.tick_at(core.port().telemetry().position.samples);
        let bpm = map.bpm_at_sample(map.sample_at(tick));
        let sig = map.time_signature_at(tick);
        let inputs = core.port().telemetry().inputs.clone();
        let top0 = view::RULER_H + marks::MARKS_H + TEMPO_H;
        div()
            .w(px(view::HEADER_W))
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(c(THEME.bg1))
            .border_r_1()
            .border_color(c(THEME.line))
            .child(self.render_toolbar(cx))
            .child(marks::MarksState::header())
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
                    .child(div().text_size(px(10.)).text_color(c(THEME.text3)).child(format!(
                        "{}/{}",
                        sig.numerator(),
                        sig.denominator()
                    ))),
            )
            .children(rows.iter().map(|r| {
                let top = top0 + r.top - self.view.scroll_y as f32;
                let el: AnyElement = match r.kind {
                    RowKind::Track(i) => self.track_header(&tracks[i], r, core, &inputs, cx),
                    RowKind::Take(i, k, l) => self.take_header(&tracks[i].items[k], l, r, cx),
                    RowKind::Automation(i, a) => self.lane_header(&tracks[i].automation[a], r, core, cx),
                };
                div().absolute().top(px(top)).left(px(0.)).w(px(view::HEADER_W)).h(px(r.h)).child(el)
            }))
            // the line where a dragged track would land (F-TRK-029)
            .children(self.track_drag.as_ref().and_then(|d| d.indicator).map(|(y, depth)| {
                div()
                    .absolute()
                    .top(px(y - 1.0))
                    .left(px(depth as f32 * INDENT))
                    .w(px(view::HEADER_W - depth as f32 * INDENT))
                    .h(px(2.))
                    .bg(c(THEME.cursor))
            }))
            // under the last track: add one (the legacy song editor's "Add instrument track" and "Add sample track")
            .child(
                div()
                    .absolute()
                    .top(px(top0 + self.content_h(cx) as f32 - self.view.scroll_y as f32 + 4.0))
                    .left(px(0.))
                    .w(px(view::HEADER_W))
                    .px(px(8.))
                    .flex()
                    .gap(px(6.))
                    .child(
                        button("add-instrument-track", tr("+ Instrument track"), false, 0)
                            .debug_selector(|| "add-instrument-track".into())
                            .on_click(cx.listener(|p, _, _, cx| p.add_track(true, cx))),
                    )
                    .child(
                        button("add-audio-track", tr("+ Audio track"), false, 0)
                            .debug_selector(|| "add-audio-track".into())
                            .on_click(cx.listener(|p, _, _, cx| p.add_track(false, cx))),
                    )
                    .child(
                        button("add-folder-track", tr("+ Folder"), false, 0)
                            .debug_selector(|| "add-folder-track".into())
                            .on_click(cx.listener(|p, _, _, cx| p.add_folder(cx))),
                    ),
            )
    }

    /// The view back at the start of the song: the left edge, the first track (F-ARR-013), at the project's own
    /// zoom (the interface document)
    pub(crate) fn to_start(&mut self, cx: &mut Context<Self>) {
        self.view.scroll_ticks = 0.0;
        self.view.scroll_y = 0.0;
        self.read_zoom(cx);
        cx.notify();
    }

    /// The zoom the project had (the interface document's `arrangement.ticksPerPx`); none saved: the default
    fn read_zoom(&mut self, cx: &Context<Self>) {
        let saved = model::at(self.core.read(cx).port().ui(), &["arrangement", "ticksPerPx"]).and_then(Value::as_real);
        let tpp = saved.map_or(view::TimelineView::default().ticks_per_px, |t| t.clamp(view::MIN_TPP, view::MAX_TPP));
        self.view.ticks_per_px = tpp;
        self.saved_zoom = tpp;
        self.zoom_read = true;
    }

    /// Writes the zoom to the interface document when it changed (no undo, not in the journal)
    fn remember_zoom(&mut self, cx: &mut Context<Self>) {
        let tpp = self.view.ticks_per_px;
        if (tpp - self.saved_zoom).abs() < 1e-9 {
            return;
        }
        self.saved_zoom = tpp;
        if let Ok(v) = Value::real(tpp) {
            let path = vec![PathStep::key("arrangement"), PathStep::key("ticksPerPx")];
            self.core.update(cx, |core, _| {
                core.report(|pt| pt.set_ui(path, v));
            });
        }
    }

    /// Whether the focus is in this page (its keys work)
    pub fn has_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Where a track's row was drawn in the last frame, relative to the timeline: (y, h)
    #[cfg(all(test, debug_assertions))]
    pub fn track_row(&self, track: NodeId) -> Option<(f32, f32)> {
        self.track_hits.borrow().iter().find(|(.., t)| *t == track).map(|(y, h, _)| (*y, *h))
    }

    /// A new track at the end: an instrument track with the TripleOscillator, as the legacy's default, or an
    /// audio track; its main output goes to the master (I-3.9)
    /// A new empty folder track at the end (F-ARR-010); tracks go into it from their menu or by dragging
    fn add_folder(&mut self, cx: &mut Context<Self>) {
        let n = self.tracks(cx).len();
        let node = Value::object([
            ("name", Value::text(trf("Folder {0}", &[&(n + 1)]))),
            ("folder", Value::bool(true)),
        ]);
        self.apply(
            "Add folder",
            vec![EditOp::Insert { temp: 0, parent: Ref::Id(lm_port::ROOT), list: "tracks".into(), index: n, node }],
            cx,
        );
    }

    fn add_track(&mut self, instrument: bool, cx: &mut Context<Self>) {
        let n = self.tracks(cx).len();
        let node = if instrument {
            Value::object([
                ("name", Value::text("TripleOscillator")),
                ("instrument", Value::object([("plugin", Value::text("tripleoscillator"))])),
            ])
        } else {
            Value::object([("name", Value::text(trf("Audio {0}", &[&(n + 1)])))])
        };
        self.apply(
            if instrument { "Add instrument track" } else { "Add audio track" },
            vec![EditOp::Insert { temp: 0, parent: Ref::Id(lm_port::ROOT), list: "tracks".into(), index: n, node }],
            cx,
        );
    }

    fn track_header(&self, t: &TrackView, r: &Row, core: &Core, inputs: &[f32], cx: &Context<Self>) -> AnyElement {
        let (i, id) = (t.index, t.id);
        // the number on the track's colour is the handle that moves the track (F-TRK-020, F-TRK-029)
        let number = div()
            .id(("grip", i))
            .debug_selector(move || format!("grip-{i}"))
            .w(px(22.))
            .h_full()
            .bg(c(t.color.unwrap_or_else(|| track_color(i))))
            .text_size(px(10.))
            .font_weight(FontWeight::BOLD)
            .text_color(c(THEME.on_accent))
            .pt(px(4.))
            .flex()
            .justify_center()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    p.ed.track = Some(id);
                    p.begin_track_drag(id, ev, cx);
                }),
            )
            .child(format!("{}", i + 1));
        let base = div()
            .size_full()
            .relative()
            .flex()
            .flex_row()
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .pl(px(r.depth as f32 * INDENT))
            .when(self.ed.track == Some(id), |d| d.bg(c(THEME.bg3)))
            // a click anywhere on the header selects the track (F-ARR-012); the right button opens its menu
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                    p.ed.track = Some(id);
                    // Shift+click: the default height again (F-TRK-030)
                    if holds("comp.track.reset_height", &ev.modifiers) {
                        p.track_h = view::TRACK_H;
                        p.track_heights.remove(&id);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                    p.ed.track = Some(id);
                    p.ed.open_track_menu(id, ev.position);
                    cx.notify();
                }),
            )
            .child(number);
        let (mute, solo, collapsed, armed) = (t.mute, t.solo, t.collapsed, t.armed);
        let mute_b = button(SharedString::from(format!("m{i}")), "M", mute, THEME.orange)
            .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, "mute", mute, cx)));
        let solo_b = button(SharedString::from(format!("s{i}")), "S", solo, THEME.yellow)
            .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, "solo", solo, cx)));
        if t.folder {
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
                            button(SharedString::from(format!("fold{i}")), if collapsed { "▸" } else { "▾" }, false, 0)
                                .debug_selector(move || format!("fold-{i}"))
                                .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, "collapsed", collapsed, cx))),
                        )
                        .child(self.name_label(t, "📁", cx))
                        .child(mute_b)
                        .child(solo_b),
                )
                .into_any_element();
        }
        let icon = if t.instrument.is_some() { "🎹" } else { "🎤" };
        let volume = t.volume;
        let kv = view::gain_to_travel(volume);
        let has_takes = t.items.iter().any(|it| it.lanes.len() > 1);
        let outs = t.outputs.len();
        let main_to = t.outputs.first().map(|o| o.to);
        let main_out = t.outputs.first().map(|o| o.id);
        let takes_on = self.show_takes.contains(&id);
        let auto_on = self.show_automation.contains(&id);
        let monitor = t.monitor.clone();
        let compact = r.h < 50.0;
        let name = self.name_label(t, icon, cx);
        let first_row = div()
            .flex()
            .flex_row()
            .gap(px(4.))
            .items_center()
            .child(
                button(SharedString::from(format!("r{i}")), "●", armed, THEME.red)
                    .debug_selector(move || format!("arm-{i}"))
                    .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, "armed", armed, cx)))
                    // an instrument track's button has a menu on the right button (F-TRK-034)
                    .when(t.instrument.is_some(), |b| {
                        b.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                p.open_arm_menu(id, ev.position, cx)
                            }),
                        )
                    }),
            )
            // armed with no input to record from (F-REC-004)
            .when(armed && t.instrument.is_none() && t.input.is_empty(), |d| {
                d.child(div().text_size(px(11.)).text_color(c(THEME.yellow)).child("⚠"))
            })
            .child(self.activity_lamp(t, main_out, main_to, core, cx))
            .child(name)
            .child(mute_b)
            .child(solo_b);
        let meter_h = r.h - if outs > 1 { 14.0 } else { 6.0 };
        let input_level =
            t.input.iter().filter_map(|ch| inputs.get((ch - 1).max(0) as usize).copied()).fold(0.0f32, f32::max);
        base.child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(if compact { 3. } else { 6. }))
                .child(first_row)
                // 4: the volume knob (no pan: I-3.9), its value, monitoring, takes, lanes, FX, I/O
                .when(!compact, |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(4.))
                            .items_center()
                            .child(
                                div()
                                    .id(("tknob", i))
                                    .debug_selector(move || format!("tknob-{i}"))
                                    .role(gpui::Role::Slider)
                                    .aria_label(SharedString::from(format!("{} {}", tr("Volume"), t.name)))
                                    .aria_numeric_value(view::gain_to_db(volume).max(-60.0))
                                    .aria_min_numeric_value(-60.0)
                                    .aria_max_numeric_value(12.0)
                                    .cursor_ns_resize()
                                    .child(knob_face(kv, 22.0))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                                            p.begin_knob(id, volume, ev, cx)
                                        }),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                                            cx.stop_propagation();
                                            p.knob_menu = Some((id, ev.position));
                                            cx.notify();
                                        }),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Middle,
                                        cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                                            p.begin_knob(id, volume, ev, cx)
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(46.))
                                    .text_size(px(9.5))
                                    .text_color(c(THEME.text2))
                                    .child(view::format_gain(volume)),
                            )
                            .child(
                                button(
                                    SharedString::from(format!("mon{i}")),
                                    match &*monitor {
                                        "on" => "◉",
                                        "off" => "○",
                                        _ => "◐",
                                    },
                                    &*monitor == "on",
                                    THEME.blue,
                                )
                                .on_click(cx.listener(move |p, _, _, cx| p.cycle_monitor(id, &monitor, cx))),
                            )
                            .child(div().flex_1())
                            .when(has_takes, |d| {
                                d.child(
                                    button(SharedString::from(format!("tk{i}")), tr("Takes"), takes_on, THEME.blue)
                                        .on_click(cx.listener(move |p, _, _, cx| {
                                            if !p.show_takes.remove(&id) {
                                                p.show_takes.insert(id);
                                            }
                                            cx.notify();
                                        })),
                                )
                            })
                            .when(!t.automation.is_empty(), |d| {
                                d.child(
                                    button(SharedString::from(format!("ln{i}")), "A", auto_on, THEME.blue)
                                        .debug_selector(move || format!("lanes-{i}"))
                                        .on_click(cx.listener(move |p, _, _, cx| {
                                            if !p.show_automation.remove(&id) {
                                                p.show_automation.insert(id);
                                            }
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                button(SharedString::from(format!("fx{i}")), "FX", false, 0)
                                    .debug_selector(move || format!("fx-{i}"))
                                    .on_click(cx.listener(move |p, _, _, cx| p.open_fx(main_to, cx))),
                            )
                            .child(
                                button(
                                    SharedString::from(format!("io{i}")),
                                    if outs > 1 { trf("I/O ×{0}", &[&outs]) } else { "I/O".into() },
                                    outs > 1,
                                    THEME.green,
                                )
                                .debug_selector(move || format!("io-{i}"))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        p.open_io_menu(id, ev.position, cx)
                                    }),
                                ),
                            ),
                    )
                })
                // 3 (a tall header): the input and the mixer channel (F-TRK-025)
                .when(r.h >= 92.0, |d| d.child(edit::third_row(t, core.port().root()))),
        )
        // the meters on the right border: the input when armed (F-REC-009), the main output; a dot says there are more
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
                        .when(armed, |d| d.child(meter(view::meter_fill(input_level), 4.0, meter_h)))
                        .child(meter(main_out.map_or(0.0, |o| core.meter(o)), 6.0, meter_h)),
                )
                .when(outs > 1, |d| d.child(div().size(px(5.)).rounded(px(3.)).bg(c(THEME.green)))),
        )
        // the bottom border: drag changes this track's height, Shift+click gives it back the default (F-TRK-030)
        .child(
            div()
                .id(("resize", i))
                .debug_selector(move || format!("resize-{i}"))
                .absolute()
                .left(px(0.))
                .bottom(px(0.))
                .w_full()
                .h(px(4.))
                .cursor_ns_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        if holds("comp.track.reset_height", &ev.modifiers) {
                            p.track_heights.remove(&id);
                        } else {
                            let h = p.track_heights.get(&id).copied().unwrap_or(p.track_h);
                            p.height_drag = Some((id, ev.position.y.as_f32(), h));
                        }
                        cx.notify();
                    }),
                ),
        )
        .into_any_element()
    }

    /// The track's name in the header: a double click edits it in place (Enter or leaving the field keeps it, Esc
    /// cancels, F-TRK-027)
    fn name_label(&self, t: &TrackView, icon: &str, cx: &Context<Self>) -> AnyElement {
        let id = t.id;
        let plugin = t.instrument.as_ref().map(|i| i.0);
        if let Some((_, state, _)) = self.renaming.as_ref().filter(|(r, ..)| *r == id) {
            return div()
                .flex_1()
                .h(px(22.))
                .text_size(px(11.))
                .child(crate::ui::text_entry(Input::new(state)))
                .into_any_element();
        }
        div()
            .id(("tname", t.index))
            .debug_selector(move || format!("tname-{}", t.index))
            .flex_1()
            .h(px(18.))
            .px(px(6.))
            .rounded(px(3.))
            .bg(c(THEME.bg1))
            .text_size(px(12.))
            .font_weight(FontWeight::BOLD)
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |p, ev: &MouseDownEvent, window, cx| {
                    p.ed.track = Some(id);
                    if ev.click_count == 2 {
                        cx.stop_propagation();
                        p.start_rename(id, window, cx);
                    } else if let Some(plugin) = plugin {
                        // a click opens or closes the instrument's window (F-TRK-027)
                        let _ = p.workspace.update(cx, |ws, cx| {
                            if ws.is_open(Screen::Instrument) {
                                ws.close(Screen::Instrument, cx)
                            } else {
                                ws.open(Screen::Instrument, Some(plugin), window, cx)
                            }
                        });
                    }
                }),
            )
            .child(format!("{icon} {}", t.name))
            .into_any_element()
    }

    pub(crate) fn start_rename(&mut self, id: NodeId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tracks(cx).into_iter().find(|t| t.id == id) else { return };
        self.finish_rename(true, cx);
        let state = cx.new(|cx| InputState::new(window, cx).default_value(t.name.to_string()));
        state.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe_in(&state, window, move |p: &mut CompositionPage, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                p.finish_rename(true, cx);
            }
        });
        self.renaming = Some((id, state, sub));
        self.ed.close_menus();
        cx.notify();
    }

    /// The edit ends: the new name is kept (`keep`), or dropped (Esc)
    pub(crate) fn finish_rename(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some((id, state, _)) = self.renaming.take() else { return };
        let name = state.read(cx).value().trim().to_string();
        if keep && !name.is_empty() && self.tracks(cx).iter().any(|t| t.id == id && *t.name != name) {
            self.apply("Rename track", vec![EditOp::set(id, &["name"], Value::text(name))], cx);
        }
        cx.notify();
    }

    /// The track's activity lamp (F-TRK-028): green while its output carries sound, red when an effect of its channel
    /// was silenced (corrupt audio). Pressing it plays middle C on an instrument track, as long as it is held
    fn activity_lamp(
        &self,
        t: &TrackView,
        main_out: Option<NodeId>,
        main_to: Option<NodeId>,
        core: &Core,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (id, i) = (t.id, t.index);
        let meters = &core.port().telemetry().meters;
        let dead = main_to.is_some_and(|ch| {
            model::at(core.port().root(), &["mixer", "channels"])
                .into_iter()
                .flat_map(|l| l.items())
                .filter(|c| model::id_of(c) == Some(ch))
                .flat_map(|c| model::list(c, "inserts"))
                .any(|ins| model::id_of(ins).is_some_and(|n| meters.get(&n).is_some_and(|m| m.dead)))
        });
        let lit = main_out.is_some_and(|o| core.meter(o) > 0.02);
        let colour = if dead {
            THEME.red
        } else if lit {
            THEME.green
        } else {
            THEME.bg3
        };
        let play = t.instrument.is_some();
        div()
            .id(("act", i))
            .debug_selector(move || format!("activity-{i}"))
            .size(px(8.))
            .flex_none()
            .rounded(px(4.))
            .bg(c(colour))
            .when(play, |d| {
                d.cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |p, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            p.ed.track = Some(id);
                            p.core.update(cx, |core, _| core.port_mut().note_on(id, 60, 100));
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |p, _: &MouseUpEvent, _, cx| {
                            p.core.update(cx, |core, _| core.port_mut().note_off(id, 60));
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |p, _: &MouseUpEvent, _, cx| {
                            p.core.update(cx, |core, _| core.port_mut().note_off(id, 60));
                        }),
                    )
            })
            .into_any_element()
    }

    fn take_header(&self, it: &ItemView, l: usize, r: &Row, cx: &Context<Self>) -> AnyElement {
        let lane = &it.lanes[l];
        let uses = it.comp.iter().filter(|s| s.lane == Some(lane.id)).count();
        // Listening to a layer alone: the comp becomes this one lane over the whole item (F-REC-043)
        let solo = it.comp.len() == 1 && it.comp[0].from == 0 && it.comp[0].to >= it.length && uses == 1;
        let (item, lane_id, length) = (it.id, lane.id, it.length);
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(30. + r.depth as f32 * INDENT))
            .bg(c(THEME.bg1))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(div().text_size(px(11.)).child(if lane.name.is_empty() {
                trf("Take {0}", &[&(l + 1)])
            } else {
                lane.name.to_string()
            }))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(c(if uses > 0 { THEME.green } else { THEME.text3 }))
                    .child(trf("used in {0} parts", &[&uses])),
            )
            .child(
                tbutton(SharedString::from(format!("take-listen-{item}-{l}")), tr("Listen"), solo, THEME.green).on_click(
                    cx.listener(move |p, _, _, cx| {
                        let seg = Value::object([
                            ("from", Value::int(0)),
                            ("to", Value::int(length)),
                            ("lane", Value::text(lane_id.to_string())),
                            ("fade", Value::int(0)),
                        ]);
                        p.apply("Listen to the layer", vec![EditOp::set(item, &["comp"], Value::list([seg]))], cx);
                    }),
                ),
            )
            .into_any_element()
    }

    /// An automation lane's header: its parameter, its mode (Off, Read, Touch, Latch, Write: a click goes to the next,
    /// I-21.5), and "the hand won" with its re-arm while the hand overrides the curve
    fn lane_header(&self, a: &model::AutomationView, r: &Row, core: &Core, cx: &Context<Self>) -> AnyElement {
        let lane = a.id;
        let mode = a.mode.to_string();
        let won =
            a.node.is_some_and(|n| core.port().telemetry().hand_won.iter().any(|(o, k)| *o == n && **k == *a.param));
        let (label, color) = match mode.as_str() {
            "off" => ("Off", THEME.text3),
            "touch" => ("Touch", THEME.yellow),
            "latch" => ("Latch", THEME.orange),
            "write" => ("Write", THEME.red),
            _ => ("Read", THEME.green),
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(30. + r.depth as f32 * INDENT))
            .pr(px(6.))
            .bg(c(THEME.bg1))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(div().flex_1().overflow_hidden().text_size(px(11.)).child(tr_str(&capitalize(&a.param))))
            .child(div().text_size(px(10.)).text_color(c(THEME.text3)).child(if a.on { "" } else { "(off)" }))
            .when(won, |d| {
                d.child(
                    button(SharedString::from(format!("rearm-{}", lane.number)), tr("Hand won ↺"), true, THEME.yellow)
                        .debug_selector(move || format!("rearm-{}", lane.number))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.action("automation.rearm", Value::object([("lane", Value::text(lane.to_string()))]), cx)
                        })),
                )
            })
            .child(
                button(SharedString::from(format!("lmode-{}", lane.number)), tr(label), mode != "read", color)
                    .debug_selector(move || format!("lane-mode-{}", lane.number))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        let next = match mode.as_str() {
                            "off" => "read",
                            "read" => "touch",
                            "touch" => "latch",
                            "latch" => "write",
                            _ => "off",
                        };
                        p.apply("Automation mode", vec![EditOp::set(lane, &["mode"], Value::text(next))], cx);
                    })),
            )
            .into_any_element()
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

// -----------------------------------------------------------------------------------------------------------------
// Timeline

/// What the paint closure needs, gathered in render (the closure is 'static and cannot borrow the port)
enum Paint {
    Lane { y: f32, h: f32 },
    Item { y: f32, h: f32, x: f32, w: f32, color: u32, body: Body },
    Edges {
        y: f32,
        h: f32,
        x: f32,
        w: f32,
        fade_in: f32,
        fade_out: f32,
        curves: (f32, f32),
        gain: f64,
        selected: bool,
    },
    Boundary { y: f32, h: f32, x: f32, fade_px: f32 },
    Overlap { y: f32, h: f32, x: f32, w: f32 },
    /// A capture xrun's silence in a take, between x0 and x1 (F-REC-084)
    Gap { y: f32, h: f32, x: f32, w: f32 },
    Curve { y: f32, h: f32, pts: Vec<(f32, f32)> },
}

/// A lane's xrun gaps as (x, width) between `a` and `z`; the waveform is drawn from the item's start, so a gap's x
/// counts from the lane's source offset
fn gap_marks(lane: &model::LaneView, ox: f32, spp: f64, a: f32, z: f32) -> Vec<(f32, f32)> {
    lane.gaps
        .iter()
        .filter_map(|(frame, frames)| {
            let x0 = ox + ((frame - lane.offset) as f64 / spp.max(1e-9)) as f32;
            let x1 = x0 + (*frames as f64 / spp.max(1e-9)) as f32;
            (x1 > a && x0 < z).then(|| (x0.max(a), (x1.min(z) - x0.max(a)).max(1.0)))
        })
        .collect()
}

enum Body {
    /// audio: (resource, x of the source's sample 0, samples per pixel, x0..x1 shown)
    Wave(Vec<(Resource, f32, f64, f32, f32)>),
    /// MIDI: notes as (x, w, key 0..1, velocity)
    Notes(Vec<(f32, f32, f32, f32)>),
    /// a take lane: the lane's waveform dim, its used parts bright
    Dim(Resource, f32, f64, Vec<(f32, f32)>),
    Empty,
}

impl CompositionPage {
    fn render_timeline(&self, tracks: &[TrackView], rows: &[Row], cursor: f64, cx: &Context<Self>) -> AnyElement {
        let core = self.core.read(cx);
        let port = core.port();
        let root = port.root();
        let map = model::tempo_map(root);
        let song = model::song(root);
        let v = self.view;
        let top0 = view::RULER_H + marks::MARKS_H + TEMPO_H;
        let sy = v.scroll_y as f32;
        let mut paint = Vec::new();
        let mut overlays: Vec<AnyElement> = Vec::new();
        let mut wants: Vec<(Resource, f32, f64, f32, f32)> = Vec::new();
        self.hits.borrow_mut().clear();
        self.lane_hits.borrow_mut().clear();
        self.track_hits.borrow_mut().clear();
        let tele = port.telemetry();
        // the samples per pixel of audio at a tick (the tempo there)
        let spp_at = |map: &TempoMap, tick: i64| {
            let a = map.sample_at(tick);
            let b = map.sample_at(tick + 960);
            (b - a) as f64 / 960.0 * v.ticks_per_px
        };
        for r in rows {
            let y = top0 + r.top - sy;
            paint.push(Paint::Lane { y, h: r.h });
            match r.kind {
                RowKind::Track(i) => {
                    let t = &tracks[i];
                    self.track_hits.borrow_mut().push((y, r.h, t.id));
                    let color = t.color.unwrap_or_else(|| track_color(i));
                    for it in &t.items {
                        let x = v.x(it.start as f64);
                        let w = (it.length as f64 / v.ticks_per_px) as f32;
                        if x > 4000.0 || x + w < 0.0 {
                            continue;
                        }
                        let spp = spp_at(&map, it.start);
                        let mut gaps: Vec<(f32, f32)> = Vec::new();
                        let body = if it.audio {
                            // the comp: each segment plays its lane, with a crossfade at each boundary (U2)
                            let mut parts = Vec::new();
                            for (k, seg) in it.comp.iter().enumerate() {
                                let a = x + (seg.from as f64 / v.ticks_per_px) as f32;
                                let b = x + (seg.to as f64 / v.ticks_per_px) as f32;
                                if let Some(src) = seg.lane.and_then(|l| it.lane(l)).and_then(|l| l.source.clone()) {
                                    wants.push((src.clone(), x, spp, a, b));
                                    parts.push((src, x, spp, a, b));
                                    for (gx, gw) in seg.lane.and_then(|l| it.lane(l)).map_or(Vec::new(), |l| gap_marks(l, x, spp, a, b)) {
                                        gaps.push((gx, gw));
                                    }
                                }
                                if k > 0 {
                                    let fade = (seg.fade as f64 / v.ticks_per_px) as f32;
                                    paint.push(Paint::Boundary { y, h: r.h, x: a, fade_px: fade.max(3.0) });
                                }
                            }
                            if parts.is_empty() { Body::Empty } else { Body::Wave(parts) }
                        } else {
                            Body::Notes(
                                it.notes
                                    .iter()
                                    .map(|(key, s, l, vel)| {
                                        (
                                            x + (*s as f64 / v.ticks_per_px) as f32,
                                            ((*l as f64 / v.ticks_per_px) as f32).max(1.0),
                                            (*key as f32 - 30.0) / 60.0,
                                            *vel as f32,
                                        )
                                    })
                                    .collect(),
                            )
                        };
                        paint.push(Paint::Item {
                            y,
                            h: r.h,
                            x,
                            w,
                            color: if it.mute { THEME.text3 } else { it.color.unwrap_or(color) },
                            body,
                        });
                        for (gx, gw) in gaps {
                            paint.push(Paint::Gap { y, h: r.h, x: gx, w: gw });
                        }
                        let fi = (it.fade_in as f64 / v.ticks_per_px) as f32;
                        let fo = (it.fade_out as f64 / v.ticks_per_px) as f32;
                        paint.push(Paint::Edges {
                            y,
                            h: r.h,
                            x,
                            w,
                            fade_in: fi,
                            fade_out: fo,
                            curves: (it.fade_in_curve as f32, it.fade_out_curve as f32),
                            gain: it.gain,
                            selected: self.selected_item == Some(it.id) || self.ed.selection.contains(&it.id),
                        });
                        self.hits.borrow_mut().push((x, y + 3.0, w, r.h - 6.0, it.id, fi, fo));
                        // the item's name, or the loading component in place of the waveform (I-2)
                        let target = Target::Node(it.id);
                        let loading = port.loading(&target);
                        let core_handle = self.core.clone();
                        if let Some(el) = loading_component(
                            SharedString::from(format!("load{}", it.id.number)),
                            &loading,
                            move |_, cx| {
                                core_handle.update(cx, |core, cx| {
                                    let ops: Vec<_> = core
                                        .port()
                                        .activity()
                                        .into_iter()
                                        .filter(|a| a.target == target || matches!(a.target, Target::Resource(_)))
                                        .map(|a| a.id)
                                        .collect();
                                    for op in ops {
                                        core.port_mut().cancel(op);
                                    }
                                    cx.notify();
                                });
                            },
                        ) {
                            let x0 = x.max(0.0);
                            overlays.push(
                                div()
                                    .absolute()
                                    .top(px(y + 3.0))
                                    .left(px(x0))
                                    .w(px((x + w - x0).clamp(0.0, 4000.0)))
                                    .h(px(r.h - 6.0))
                                    .child(el)
                                    .into_any_element(),
                            );
                        } else if w > 40.0 && r.h >= 40.0 {
                            overlays.push(
                                div()
                                    .absolute()
                                    .top(px(y + 4.0))
                                    .left(px(x.max(0.0) + 4.0))
                                    .max_w(px(w.min(400.0)))
                                    .text_size(px(10.))
                                    .text_color(c(THEME.text))
                                    .overflow_hidden()
                                    .child(it.name.to_string())
                                    .into_any_element(),
                            );
                        }
                    }
                    // overlapping items cross-fade by themselves (F-CLP-062): the overlap is marked
                    let mut spans: Vec<(i64, i64)> =
                        t.items.iter().map(|it| (it.start, it.start + it.length)).collect();
                    spans.sort_unstable();
                    for w2 in spans.windows(2) {
                        if w2[1].0 < w2[0].1 {
                            let (a, z) = (v.x(w2[1].0 as f64), v.x(w2[0].1.min(w2[1].1) as f64));
                            paint.push(Paint::Overlap { y, h: r.h, x: a, w: z - a });
                        }
                    }
                    // the take growing while recording (U3), with its pre-roll handle (U4)
                    for take in tele.recording.iter().filter(|rt| rt.track == t.id) {
                        let start = map.tick_at(take.start);
                        let x = v.x(start as f64);
                        let w = ((map.tick_at(take.start + take.length) - start) as f64 / v.ticks_per_px) as f32;
                        let spp = spp_at(&map, start);
                        // its peaks grow: what the cache kept of it is stale
                        self.tiles.borrow_mut().forget(&take.resource);
                        wants.push((take.resource.clone(), x, spp, x, x + w));
                        paint.push(Paint::Item {
                            y,
                            h: r.h,
                            x,
                            w,
                            color: THEME.red,
                            body: Body::Wave(vec![(take.resource.clone(), x, spp, x, x + w)]),
                        });
                    }
                }
                RowKind::Take(i, k, l) => {
                    let it = &tracks[i].items[k];
                    let lane = &it.lanes[l];
                    let x = v.x(it.start as f64);
                    let w = (it.length as f64 / v.ticks_per_px) as f32;
                    let used: Vec<(f32, f32)> = it
                        .comp
                        .iter()
                        .filter(|s| s.lane == Some(lane.id))
                        .map(|s| {
                            (x + (s.from as f64 / v.ticks_per_px) as f32, x + (s.to as f64 / v.ticks_per_px) as f32)
                        })
                        .collect();
                    if let Some(src) = lane.source.clone() {
                        let spp = spp_at(&map, it.start);
                        wants.push((src.clone(), x, spp, x, x + w));
                        paint.push(Paint::Item {
                            y,
                            h: r.h,
                            x,
                            w,
                            color: tracks[i].color.unwrap_or_else(|| track_color(i)),
                            body: Body::Dim(src, x, spp, used),
                        });
                        for (gx, gw) in gap_marks(lane, x, spp, x, x + w) {
                            paint.push(Paint::Gap { y, h: r.h, x: gx, w: gw });
                        }
                    }
                }
                RowKind::Automation(i, a) => {
                    let lane = &tracks[i].automation[a];
                    let (lo, hi) =
                        lane.points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
                    let span = (hi - lo).max(1e-9);
                    let pts =
                        lane.points.iter().map(|p| (v.x(p.0 as f64), (1.0 - ((p.1 - lo) / span) as f32))).collect();
                    paint.push(Paint::Curve { y, h: r.h, pts });
                    self.lane_hits.borrow_mut().push((y, r.h, lane.id, tracks[i].id));
                }
            }
        }
        {
            let mut tiles = self.tiles.borrow_mut();
            for (res, ox, spp, a, z) in &wants {
                let level = level_for(*spp);
                tiles.want(res, level, tile_range(level, *spp, *ox, a.max(0.0), z.min(2400.0)));
            }
        }
        // the ruler: a bar number at each bar line that fits (texts as elements)
        let bar_ticks = |bar: i64| map.bar_start(bar).unwrap_or(0) as f64;
        let first_bar = map.bar_beat_tick(v.scroll_ticks.max(0.0) as i64).bar;
        let px_per_bar = ((bar_ticks(first_bar + 1) - bar_ticks(first_bar)) / v.ticks_per_px).max(1.0);
        let every = (80.0 / px_per_bar).ceil().max(1.0) as i64;
        let mut ticks_lines = Vec::new();
        let mut bar = first_bar;
        while bar < first_bar + 400 {
            let x = v.x(bar_ticks(bar));
            if x > 2400.0 {
                break;
            }
            if (bar - 1) % every == 0 {
                ticks_lines.push(x);
                overlays.push(
                    div()
                        .absolute()
                        .top(px(2.))
                        .left(px(x + 3.0))
                        .text_size(px(10.))
                        .text_color(c(THEME.text2))
                        .child(format!("{bar}"))
                        .into_any_element(),
                );
            }
            bar += 1;
        }
        // the lane's line (steps and ramps); the points and their labels are the lane's overlay (tempo.rs)
        let points: Vec<(f32, f64, bool)> = map
            .points()
            .iter()
            .map(|p| (v.x(p.tick as f64), p.bpm.to_f64(), matches!(p.shape, lm_time::TempoShape::Linear)))
            .collect();
        let tempo_pts: Vec<(f32, f32)> = {
            let (lo, hi) = self.tempo.range(map.points().iter().map(|p| p.bpm.to_f64()));
            let yv = |bpm: f64| tempo::bpm_y(bpm, lo, hi);
            let mut pts = Vec::new();
            for (k, (x, bpm, _)) in points.iter().enumerate() {
                if k > 0 && !points[k - 1].2 {
                    pts.push((*x, yv(points[k - 1].1)));
                }
                pts.push((*x, yv(*bpm)));
            }
            if let Some(last) = points.last() {
                pts.push((4000.0, yv(last.1)));
            }
            pts
        };
        // the background grid (F-ARR-014): every other group of four bars shaded, bar lines, and the grid's lines when
        // they are at least 8 px apart
        let frac: f64 = self.frac();
        let mut shaded: Vec<(f32, f32)> = Vec::new();
        let mut grid_lines: Vec<(f32, bool)> = Vec::new();
        for gb in first_bar..first_bar + 600 {
            let (a, z) = (v.x(bar_ticks(gb)), v.x(bar_ticks(gb + 1)));
            if a > 2400.0 {
                break;
            }
            if ((gb - 1) / 4) % 2 == 1 {
                shaded.push((a, z));
            }
            grid_lines.push((a, true));
            if frac < 1.0 && (z - a) * frac as f32 >= 8.0 {
                let n = (1.0 / frac).round() as i64;
                for k in 1..n {
                    grid_lines.push((a + (z - a) * k as f32 / n as f32, false));
                }
            }
        }
        let bounds_cell = self.timeline_bounds.clone();
        let tile_cache = self.tiles.clone();
        let playing_x = v.x(cursor);
        let cursor_color = if tele.position.recording { THEME.red } else { THEME.cursor };
        let loop_x = (v.x(song.loop_range.0 as f64), v.x(song.loop_range.1 as f64), song.loop_on);
        let punch_x = (v.x(song.punch_range.0 as f64), v.x(song.punch_range.1 as f64), song.punch_on);
        let xrun_xs: Vec<f32> = tele.xrun_marks.iter().map(|m| v.x(map.tick_at(*m) as f64)).collect();
        div()
            .id("timeline")
            .debug_selector(|| "timeline".into())
            .flex_1()
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(c(THEME.bg1))
            .on_scroll(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_timeline_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_timeline_right))
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
                        let top = view::RULER_H + marks::MARKS_H + TEMPO_H;
                        let mut alt = c(THEME.text);
                        alt.a = 0.035;
                        for (a, z) in &shaded {
                            let (a, z) = (a.max(0.0), z.min(w));
                            if z > a {
                                window.paint_quad(fill(
                                    Bounds::new(at(a, top), size(px(z - a), px((full_h - top).max(0.0)))),
                                    alt,
                                ));
                            }
                        }
                        for (x, bar) in &grid_lines {
                            if (0.0..w).contains(x) {
                                let mut lc = c(THEME.line);
                                lc.a = if *bar { 0.9 } else { 0.35 };
                                window.paint_quad(fill(
                                    Bounds::new(at(*x, top), size(px(1.0), px((full_h - top).max(0.0)))),
                                    lc,
                                ));
                            }
                        }
                        for (a, z, on, col) in [
                            (loop_x.0, loop_x.1, loop_x.2, THEME.loop_range),
                            (punch_x.0, punch_x.1, punch_x.2, THEME.punch_range),
                        ] {
                            if on && z > 0.0 && a < w {
                                window.paint_quad(fill(
                                    Bounds::new(at(a.max(0.0), 0.0), size(px(z.min(w) - a.max(0.0)), px(full_h))),
                                    gpui::rgba(col),
                                ));
                            }
                        }
                        for p in &paint {
                            paint_one(window, &mut cache, p, o, w);
                        }
                        // the ruler and the tempo lane on top (they do not scroll vertically)
                        window.paint_quad(fill(Bounds::new(o, size(b.size.width, px(view::RULER_H))), c(THEME.bg3)));
                        window.paint_quad(fill(
                            Bounds::new(at(0.0, view::RULER_H + marks::MARKS_H), size(b.size.width, px(TEMPO_H))),
                            c(THEME.bg2),
                        ));
                        // loop and punch in the ruler's upper half (F-REG-006)
                        for (a, z, on, col) in
                            [(loop_x.0, loop_x.1, loop_x.2, THEME.green), (punch_x.0, punch_x.1, punch_x.2, THEME.red)]
                        {
                            if z > 0.0 && a < w {
                                let mut cc = c(col);
                                cc.a = if on { 0.9 } else { 0.25 };
                                window.paint_quad(fill(
                                    Bounds::new(at(a.max(0.0), 1.0), size(px(z.min(w) - a.max(0.0)), px(5.0))),
                                    cc,
                                ));
                            }
                        }
                        for x in &ticks_lines {
                            window.paint_quad(fill(
                                Bounds::new(at(*x, 8.0), size(px(1.0), px(view::RULER_H - 8.0))),
                                c(THEME.text3),
                            ));
                        }
                        let mut pb = PathBuilder::stroke(px(1.5));
                        for (k, (x, yv)) in tempo_pts.iter().enumerate() {
                            let pt = at(*x, view::RULER_H + marks::MARKS_H + 4.0 + yv * (TEMPO_H - 8.0));
                            if k == 0 { pb.move_to(pt) } else { pb.line_to(pt) }
                        }
                        if let Ok(path) = pb.build() {
                            window.paint_path(path, c(THEME.orange));
                        }
                        // the xruns, as red marks (F-CLP-063)
                        for x in &xrun_xs {
                            if (0.0..w).contains(x) {
                                window.paint_quad(fill(
                                    Bounds::new(at(*x, 0.0), size(px(2.0), b.size.height)),
                                    gpui::rgba(THEME.punch_range),
                                ));
                                window.paint_quad(fill(
                                    Bounds::new(at(*x - 2.0, 0.0), size(px(6.0), px(6.0))),
                                    c(THEME.red),
                                ));
                            }
                        }
                        // the play cursor, interpolated between telemetry frames
                        if playing_x >= 0.0 && playing_x < w {
                            window.paint_quad(fill(
                                Bounds::new(at(playing_x, 0.0), size(px(1.0), b.size.height)),
                                c(cursor_color),
                            ));
                        }
                    },
                )
                .size_full(),
            )
            .child(
                // the ruler's hit area: a click moves the cursor
                div()
                    .id("ruler")
                    .debug_selector(|| "ruler".into())
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .w_full()
                    .h(px(view::RULER_H))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|p, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            p.on_ruler(ev, cx)
                        }),
                    )
                    // Shift+right sets the loop's end (F-REG-007)
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|p, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if !p.loop_down(ev, cx) {
                                p.open_loop_menu(ev, cx);
                            }
                        }),
                    ),
            )
            // files dropped from the file manager (F-ARR-011, agent D's files.rs)
            .on_drop(cx.listener(|p, paths: &gpui::ExternalPaths, window, cx| p.drop_files(paths.paths(), window, cx)))
            // rows dragged from the browser
            .on_drop(cx.listener(|p, d: &crate::screens::browser::Dragged, window, cx| p.drop_dragged(d, window, cx)))
            // the ruler's markers and regions (agent D, marks.rs)
            .child(self.marks.render(v, root, self.timeline_bounds.borrow().size.width.as_f32(), cx))
            .children(self.marks.render_menu(root, cx))
            .children(overlays)
            .children(self.edit_overlays(cx))
            .children(self.tempo_overlays(cx))
            .into_any_element()
    }
}

/// The exponent of a fade's gain curve for its tension (the renderer's `fade_power`): 4^tension
fn fade_power(curve: f32) -> f32 {
    4f32.powf(curve.clamp(-1.0, 1.0))
}

/// The tension handle of a fade of `fw` px at `fx` in an item box (`iy`, `ih`): on the curve, in the middle of the fade
pub(super) fn curve_handle(fx: f32, fw: f32, iy: f32, ih: f32, curve: f32) -> (f32, f32) {
    let g = 0.5f32.powf(fade_power(curve));
    (fx + fw / 2.0, iy + (1.0 - g) * ih)
}

/// The tension that puts the handle at height `y` of the item box: the gain there is `1 - (y - iy) / ih` at the middle
/// of the fade, `0.5^power`
pub(super) fn curve_at(y: f32, iy: f32, ih: f32) -> f64 {
    let g = (1.0 - (y - iy) / ih.max(1.0)).clamp(0.02, 0.98) as f64;
    // power = ln g / ln 0.5; tension = log4 power
    ((g.ln() / 0.5f64.ln()).ln() / 4f64.ln()).clamp(-1.0, 1.0)
}

fn paint_one(window: &mut Window, cache: &mut TileCache, p: &Paint, o: gpui::Point<Pixels>, w: f32) {
    let at = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
    match p {
        Paint::Lane { y, h } => {
            window.paint_quad(fill(Bounds::new(at(0.0, y + h - 1.0), size(px(w), px(1.0))), c(THEME.line)));
        }
        Paint::Boundary { y, h, x, fade_px } => {
            let mut pb = PathBuilder::stroke(px(1.5));
            pb.move_to(at(x - fade_px, y + 4.0));
            pb.line_to(at(x + fade_px, y + h - 4.0));
            pb.move_to(at(x - fade_px, y + h - 4.0));
            pb.line_to(at(x + fade_px, y + 4.0));
            if let Ok(path) = pb.build() {
                window.paint_path(path, c(THEME.yellow));
            }
        }
        Paint::Gap { y, h, x, w: gw } => {
            let mut red = c(THEME.red);
            red.a = 0.55;
            window.paint_quad(fill(Bounds::new(at(*x, y + 3.0), size(px(gw.max(2.0)), px(h - 6.0))), red));
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
                    window.paint_quad(fill(
                        Bounds::new(at(x - 2.5, y + 4.0 + yv * (h - 8.0) - 2.5), size(px(5.0), px(5.0))),
                        c(THEME.blue),
                    ));
                }
            }
        }
        Paint::Edges { y, h, x, w: iw, fade_in, fade_out, curves, gain, selected } => {
            let (iy, ih) = (y + 3.0, h - 6.0);
            // fades: the part above the fade's curve is shaded; the handles are in the top 7 px, and a small one
            // sits on the curve in the middle of a selected item's fade (its tension)
            for (fx, fw, left, curve) in [(*x, *fade_in, true, curves.0), (x + iw - fade_out, *fade_out, false, curves.1)] {
                if fw > 0.5 {
                    let power = fade_power(curve);
                    let mut pb = PathBuilder::fill();
                    let steps = (fw / 6.0).clamp(1.0, 24.0) as usize;
                    let point = |k: usize| {
                        let t = k as f32 / steps as f32;
                        // the gain at the fraction t of the fade, from its start to its end
                        let g = if left { t.powf(power) } else { (1.0 - t).powf(power) };
                        at(fx + t * fw, iy + (1.0 - g) * ih)
                    };
                    pb.move_to(point(0));
                    for k in 1..=steps {
                        pb.line_to(point(k));
                    }
                    pb.line_to(at(fx + fw, iy));
                    pb.line_to(at(fx, iy));
                    pb.close();
                    if let Ok(path) = pb.build() {
                        window.paint_path(path, gpui::rgba(THEME.scrim));
                    }
                    if *selected && fw > 12.0 {
                        let (hx, hy) = curve_handle(fx, fw, iy, ih, curve);
                        window.paint_quad(fill(Bounds::new(at(hx - 2.5, hy - 2.5), size(px(5.0), px(5.0))), c(THEME.blue)));
                    }
                }
            }
            // the gain: a line, lower when quieter (unity at the top)
            if *gain < 0.999 {
                let db = view::gain_to_db(*gain).max(-60.0);
                let gy = iy + ((-db / 60.0) as f32).min(1.0) * ih;
                window.paint_quad(fill(
                    Bounds::new(at(x.max(0.0), gy), size(px(((x + iw).min(w) - x.max(0.0)).max(0.0)), px(1.0))),
                    c(THEME.yellow),
                ));
            }
            if *selected {
                window.paint_quad(gpui::outline(
                    Bounds::new(at(x.max(0.0), iy), size(px(((x + iw).min(w) - x.max(0.0)).max(0.0)), px(ih))),
                    c(THEME.cursor),
                    gpui::BorderStyle::Solid,
                ));
            }
        }
        Paint::Item { y, h, x, w: iw, color, body } => {
            let x0 = x.max(0.0);
            let x1 = (x + iw).min(w);
            if x1 <= x0 {
                return;
            }
            let (iy, ih) = (y + 3.0, h - 6.0);
            let mut bg = c(*color);
            bg.a = 0.18;
            window.paint_quad(gpui::quad(
                Bounds::new(at(x0, iy), size(px(x1 - x0), px(ih))),
                px(3.0),
                bg,
                px(1.0),
                c(*color),
                Default::default(),
            ));
            let clip_of =
                |a: f32, z: f32| Bounds::new(at(a.max(x0), iy), size(px((z.min(x1) - a.max(x0)).max(0.0)), px(ih)));
            let top = if ih >= 40.0 { 12.0 } else { 0.0 };
            match body {
                Body::Wave(parts) => {
                    for (res, ox, spp, a, z) in parts {
                        let clip = clip_of(*a, *z);
                        if clip.size.width > px(0.0) {
                            paint_wave(
                                window,
                                cache,
                                res,
                                *color,
                                *spp,
                                o.x + px(*ox),
                                clip,
                                o.y + px(iy + top),
                                px(ih - top),
                            );
                        }
                    }
                }
                Body::Notes(notes) => {
                    for (nx, nw, key, vel) in notes {
                        if *nx + *nw < x0 || *nx > x1 {
                            continue;
                        }
                        let ny = iy + top + (1.0 - key.clamp(0.0, 1.0)) * (ih - top - 4.0);
                        let mut col = c(*color);
                        col.a = 0.5 + 0.5 * vel;
                        window.paint_quad(fill(
                            Bounds::new(at(nx.max(x0), ny), size(px((nx + nw).min(x1) - nx.max(x0)), px(3.0))),
                            col,
                        ));
                    }
                }
                Body::Dim(res, ox, spp, used) => {
                    paint_wave(
                        window,
                        cache,
                        res,
                        THEME.text3,
                        *spp,
                        o.x + px(*ox),
                        clip_of(x0, x1),
                        o.y + px(iy),
                        px(ih),
                    );
                    for (a, z) in used {
                        let cl = clip_of(*a, *z);
                        if cl.size.width > px(0.0) {
                            paint_wave(window, cache, res, *color, *spp, o.x + px(*ox), cl, o.y + px(iy), px(ih));
                            window.paint_quad(gpui::outline(cl, c(THEME.green), gpui::BorderStyle::Solid));
                        }
                    }
                }
                Body::Empty => {}
            }
        }
    }
}
