//! The shell: the transport bar over everything, the page tabs (Composition, Mixing: separate pages, the user's
//! decision of 19/11), the docks for the editors and side windows, the modals, the keys of the registry, and asking
//! before losing changes. It owns the `Core` entity and the page views, and redraws when the core says something
//! changed (it observes the entity) or, while playing, every display frame (the cursor).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, AnyView, App, Context, Entity, FocusHandle, FontWeight, Render, SharedString, Subscription, Window,
    canvas, div, prelude::*, px,
};
use lm_port::trees::ui;
use lm_port::{CorePort, NodeId, PathStep, Value};

use crate::actions::*;
use crate::core::Core;
use crate::pages::composition::CompositionPage;
use crate::pages::mixing::MixingPage;
use crate::pages::{Page, transport};
use crate::screens::{self, Placement, Screen, ScreenCx};
use crate::ui::{THEME, button, c, tab, tr};

pub const TABS_H: f32 = 34.0;
/// Height of what is above the page: the transport bar and the tabs
pub const CHROME_H: f32 = crate::view::TRANSPORT_H + TABS_H;
const BOTTOM_DOCK_H: f32 = 300.0;
const SIDE_DOCK_W: f32 = 380.0;

/// What waits for the user's answer before losing changes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Quit,
    Close,
}

pub struct Workspace {
    pub core: Entity<Core>,
    focus: FocusHandle,
    pub page: Page,
    pub composition: Entity<CompositionPage>,
    pub mixing: Entity<MixingPage>,
    pub midi_maps: Entity<crate::pages::midi_maps::MidiMapsPage>,
    open: Vec<(Screen, AnyView)>,
    /// The plugin whose window is open (the instrument's or an effect's): the selection's `plugin` (I-21.2)
    focused_plugin: Option<NodeId>,
    pub display_format: transport::DisplayFormat,
    pub tempo_drag: Option<transport::TempoDrag>,
    /// Asking before losing changes
    pub confirm: Option<Pending>,
    /// The control the MIDI binding window shows (I-21.2)
    pub binding: Option<lm_port::MapTarget>,
    /// A screen closed: the focus comes back to the shell at the next render
    refocus: bool,
    /// The project shown (its folder and name): when it changes, the arrangement comes to the front (F-ARR-013)
    project_key: String,
    _observe: Subscription,
    /// The pages' changes: the selection goes to the interface document when it changes
    _pages: [Subscription; 2],
    /// The docks' sizes (the editors' height, the side windows' widths), dragged by their splitters and kept in the
    /// interface document (`docks`)
    pub docks: Docks,
    dock_drag: Option<(DockEdge, f32, f32)>,
    /// Tap Tempo's taps and the clock they are counted on
    pub tap: crate::tap::TapTempo,
    pub tap_clock: Instant,
    /// The time signature's editor is open in the transport
    pub sig_editor: bool,
    /// The floating notices (F-JAN-013), and the notices already seen (id and how many times it came)
    pub toasts: Vec<Toast>,
    seen_notices: HashMap<u64, u32>,
    toast_seq: u64,
    /// Closes the open gestures when this window loses the focus
    _activation: gpui::Subscription,
    /// The in-window menu bar's open menu (Linux)
    pub menu_open: Option<usize>,
    /// The pages and screens in a window of their own (F-JAN-006), with the window
    detached: Vec<(Detachable, gpui::AnyWindowHandle)>,
}

/// What a detached window shows: the page or screen, with the guard that closes the gestures a release outside
/// the window or a lost focus would leave open (the main window has its own in the workspace)
struct Hosted {
    view: AnyView,
    core: Entity<Core>,
    activation: Option<gpui::Subscription>,
}

impl Render for Hosted {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation.is_none() {
            self.activation = Some(lm_gui_plugins::guard_activation(self.core.clone(), window, cx));
        }
        div()
            .size_full()
            .child(self.view.clone())
            .child(lm_gui_plugins::release_guard(self.core.downgrade()))
    }
}

/// A floating notice: its text, its color, and its number (it goes away by itself)
#[derive(Clone, Debug)]
pub struct Toast {
    pub seq: u64,
    pub notice: u64,
    pub text: String,
    pub color: u32,
}

/// How long a floating notice stays
const TOAST_LIFE: Duration = Duration::from_secs(4);
/// How many show at once
const TOASTS_MAX: usize = 4;

/// What can be in a window of its own: a page (the mixer) or a docked screen (an editor)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detachable {
    Page(Page),
    Screen(Screen),
}

impl Detachable {
    /// How the interface document remembers it
    pub fn key(self) -> String {
        match self {
            Detachable::Page(p) => format!("page:{p:?}"),
            Detachable::Screen(s) => format!("screen:{s:?}"),
        }
    }

    pub fn parse(key: &str) -> Option<Detachable> {
        let (kind, name) = key.split_once(':')?;
        match kind {
            "page" => [Page::Composition, Page::Mixing, Page::MidiMaps]
                .into_iter()
                .find(|p| format!("{p:?}") == name)
                .map(Detachable::Page),
            "screen" => Screen::ALL
                .iter()
                .find(|s| format!("{s:?}") == name && s.placement() != Placement::Modal)
                .map(|s| Detachable::Screen(*s)),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Detachable::Page(p) => p.title(),
            Detachable::Screen(s) => s.title(),
        }
    }
}

/// The docks' sizes, in logical pixels
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Docks {
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockEdge {
    Bottom,
    Left,
    Right,
}

impl DockEdge {
    fn key(self) -> &'static str {
        match self {
            DockEdge::Bottom => "bottom",
            DockEdge::Left => "left",
            DockEdge::Right => "right",
        }
    }
    fn range(self) -> (f32, f32) {
        match self {
            DockEdge::Bottom => (140.0, 900.0),
            DockEdge::Left | DockEdge::Right => (220.0, 1000.0),
        }
    }
}

impl Workspace {
    pub fn new(port: Box<dyn CorePort>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let core = cx.new(|cx| Core::new(port, cx));
        let weak = cx.weak_entity();
        let composition = cx.new(|cx| CompositionPage::new(core.clone(), weak.clone(), cx));
        let mixing = cx.new(|cx| MixingPage::new(core.clone(), weak.clone(), cx));
        let midi_maps = cx.new(|cx| crate::pages::midi_maps::MidiMapsPage::new(core.clone(), weak.clone(), window, cx));
        let observe = cx.observe(&core, |_, _, cx| cx.notify());
        let activation = lm_gui_plugins::guard_activation(core.clone(), window, cx);
        let pages = [
            cx.observe(&composition, |ws, _, cx| ws.write_selection(cx)),
            cx.observe(&mixing, |ws, _, cx| ws.write_selection(cx)),
        ];
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let docks = {
            let saved = core.read(cx).port().ui().get("docks").cloned().unwrap_or_default();
            let size = |e: DockEdge, d: f32| {
                let (lo, hi) = e.range();
                saved.get(e.key()).and_then(lm_port::Value::as_int).map_or(d, |v| (v as f32).clamp(lo, hi))
            };
            Docks {
                bottom: size(DockEdge::Bottom, BOTTOM_DOCK_H),
                left: size(DockEdge::Left, SIDE_DOCK_W),
                right: size(DockEdge::Right, SIDE_DOCK_W),
            }
        };
        let mut ws = Workspace {
            core,
            focus,
            page: if std::env::var("LM_PAGE").as_deref() == Ok("mixing") { Page::Mixing } else { Page::Composition },
            composition,
            mixing,
            midi_maps,
            open: Vec::new(),
            focused_plugin: None,
            display_format: Default::default(),
            tempo_drag: None,
            confirm: None,
            binding: None,
            refocus: false,
            project_key: String::new(),
            _observe: observe,
            _pages: pages,
            docks,
            dock_drag: None,
            tap: Default::default(),
            tap_clock: Instant::now(),
            sig_editor: false,
            toasts: Vec::new(),
            seen_notices: HashMap::new(),
            toast_seq: 0,
            menu_open: None,
            detached: Vec::new(),
            _activation: activation,
        };
        // what is there at the start is not news: only the notices that come later float
        let first = ws.core.update(cx, |core, _| core.port_mut().notices());
        ws.seen_notices = first.iter().map(|n| (n.id, n.repeats)).collect();
        // LM_OPEN=Settings,PianoRoll... opens screens at start (screenshots, and each author's own testing)
        if let Ok(list) = std::env::var("LM_OPEN") {
            for name in list.split(',') {
                if let Some(s) = Screen::ALL.iter().find(|s| format!("{s:?}") == name.trim()) {
                    ws.open(*s, None, window, cx);
                }
            }
        }
        // the windows the last session left detached (the interface document), after the first frame
        let saved: Vec<Detachable> = ws
            .core
            .read(cx)
            .port()
            .ui()
            .get("detached")
            .map(|l| l.items().filter_map(|v| v.as_text().and_then(Detachable::parse)).collect())
            .unwrap_or_default();
        if !saved.is_empty() {
            cx.defer_in(window, move |ws, window, cx| {
                for d in saved {
                    if let Detachable::Screen(s) = d {
                        ws.open(s, None, window, cx);
                    }
                    ws.detach(d, cx);
                }
            });
        }
        ws
    }

    /// Opens a screen, on a node when it edits one (an item for the Piano Roll, a plugin for its window)
    pub fn open(&mut self, s: Screen, target: Option<NodeId>, window: &mut Window, cx: &mut Context<Self>) {
        let place = s.placement();
        if let Some(i) = self.open.iter().position(|(o, _)| *o == s) {
            if target.is_none() {
                return;
            }
            self.open.remove(i);
        }
        // one modal at a time, one screen per dock
        if let Some(d) = self.detached.iter().position(|(d, _)| *d == Detachable::Screen(s)) {
            // asking for a detached screen brings it back
            self.dock(self.detached[d].0, cx);
        }
        let detached: Vec<Screen> = self
            .detached
            .iter()
            .filter_map(|(d, _)| if let Detachable::Screen(s) = d { Some(*s) } else { None })
            .collect();
        self.open.retain(|(o, _)| o.placement() != place || detached.contains(o));
        if matches!(s, Screen::Effect | Screen::Instrument) {
            self.focused_plugin = target;
            // after the current update: the page that opened the window may be the one being updated
            let me = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = me.update(cx, |ws, cx| ws.write_selection(cx));
            });
        }
        let binding = if matches!(s, Screen::MidiBinding | Screen::Modulator) { self.binding.clone() } else { None };
        let scx = ScreenCx { core: self.core.clone(), workspace: cx.weak_entity(), screen: s, target, binding };
        let view = screens::open(scx, window, cx);
        self.open.push((s, view));
        cx.notify();
    }

    pub fn close(&mut self, s: Screen, cx: &mut Context<Self>) {
        if matches!(s, Screen::Effect | Screen::Instrument)
            && !self.open.iter().any(|(o, _)| *o != s && matches!(o, Screen::Effect | Screen::Instrument))
        {
            self.focused_plugin = None;
        }
        self.open.retain(|(o, _)| *o != s);
        if self.detached.iter().any(|(d, _)| *d == Detachable::Screen(s)) {
            self.dock(Detachable::Screen(s), cx);
        }
        // the screen may have held the focus: the shell takes it back at the next frame, or the keys reach nothing
        self.refocus = true;
        cx.notify();
    }

    pub fn is_open(&self, s: Screen) -> bool {
        self.open.iter().any(|(o, _)| *o == s)
    }

    /// An open screen's view (the tests reach the screen's own state through it)
    #[cfg(all(test, debug_assertions))]
    pub fn screen_view(&self, s: Screen) -> Option<AnyView> {
        self.open.iter().find(|(o, _)| *o == s).map(|(_, v)| v.clone())
    }

    fn docked(&self, p: Placement) -> Option<AnyView> {
        self.open
            .iter()
            .find(|(s, _)| s.placement() == p && !self.is_detached(Detachable::Screen(*s)))
            .map(|(_, v)| v.clone())
    }

    pub fn is_detached(&self, d: Detachable) -> bool {
        self.detached.iter().any(|(o, _)| *o == d)
    }

    /// The view a detachable shows
    fn view_of(&self, d: Detachable) -> Option<AnyView> {
        match d {
            Detachable::Page(Page::Composition) => Some(self.composition.clone().into()),
            Detachable::Page(Page::Mixing) => Some(self.mixing.clone().into()),
            Detachable::Page(Page::MidiMaps) => Some(self.midi_maps.clone().into()),
            Detachable::Screen(s) => self.open.iter().find(|(o, _)| *o == s).map(|(_, v)| v.clone()),
        }
    }

    /// Puts a page or an open screen in a window of its own (F-JAN-006); closing that window docks it back
    pub fn detach(&mut self, d: Detachable, cx: &mut Context<Self>) {
        if self.is_detached(d) {
            return;
        }
        let Some(view) = self.view_of(d) else { return };
        let me = cx.weak_entity();
        let options = gpui::WindowOptions {
            titlebar: Some(gpui::TitlebarOptions { title: Some(tr(d.title()).into()), ..Default::default() }),
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                gpui::point(px(80.), px(80.)),
                gpui::size(px(crate::view::WINDOW_W * 0.7), px(crate::view::WINDOW_H * 0.7)),
            ))),
            ..Default::default()
        };
        let core = self.core.clone();
        let opened = cx.open_window(options, move |window, cx| {
            let back = me.clone();
            window.on_window_should_close(cx, move |_, cx| {
                // the user closed it: it goes back to the main window
                let _ = back.update(cx, |ws, cx| ws.forget_detached(d, cx));
                true
            });
            let hosted = cx.new(|_| Hosted { view, core, activation: None });
            cx.new(|cx| gpui_component::Root::new(hosted, window, cx))
        });
        if let Ok(handle) = opened {
            self.detached.push((d, handle.into()));
            self.write_detached(cx);
            cx.notify();
        }
    }

    /// The window is closing by itself: only the record goes
    fn forget_detached(&mut self, d: Detachable, cx: &mut Context<Self>) {
        self.detached.retain(|(o, _)| *o != d);
        self.write_detached(cx);
        cx.notify();
    }

    /// Brings a detached page or screen back into the main window
    pub fn dock(&mut self, d: Detachable, cx: &mut Context<Self>) {
        let Some(i) = self.detached.iter().position(|(o, _)| *o == d) else { return };
        let (_, handle) = self.detached.remove(i);
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        self.write_detached(cx);
        cx.notify();
    }

    pub fn dock_all(&mut self, cx: &mut Context<Self>) {
        let all: Vec<Detachable> = self.detached.iter().map(|(d, _)| *d).collect();
        for d in all {
            self.dock(d, cx);
        }
    }

    /// Every open editor and side window in a window of its own
    pub fn detach_all(&mut self, cx: &mut Context<Self>) {
        let screens: Vec<Screen> =
            self.open.iter().map(|(s, _)| *s).filter(|s| s.placement() != Placement::Modal).collect();
        for s in screens {
            self.detach(Detachable::Screen(s), cx);
        }
    }

    /// What is detached, remembered in the interface document (I-3.8): no undo, saved with the project
    fn write_detached(&mut self, cx: &mut Context<Self>) {
        let list = Value::list(self.detached.iter().map(|(d, _)| Value::text(d.key())));
        self.core.update(cx, |core, _| {
            core.report(|pt| pt.set_ui(vec![PathStep::key("detached")], list));
        });
    }

    /// The floating notices: the new ones (or the ones that came again) join the stack, and go by themselves
    fn collect_toasts(&mut self, cx: &mut Context<Self>) {
        let list = self.core.update(cx, |core, _| core.port_mut().notices());
        let mut fresh = Vec::new();
        for n in &list {
            if self.seen_notices.get(&n.id) != Some(&n.repeats) {
                self.seen_notices.insert(n.id, n.repeats);
                if !n.read {
                    fresh.push(n.clone());
                }
            }
        }
        self.seen_notices.retain(|id, _| list.iter().any(|n| n.id == *id));
        // the errors of the calls the interface made (`Core::report`): one short line each, the same error again
        // replaces the one still showing
        let errors = self.core.update(cx, |core, _| core.take_errors());
        for e in errors {
            self.toast_seq += 1;
            let seq = self.toast_seq;
            let text = crate::screens::notices::error_text(&e);
            self.toasts.retain(|t| t.text != text);
            self.toasts.push(Toast {
                seq,
                notice: u64::MAX - seq,
                text,
                color: crate::screens::notices::severity_color(lm_port::Severity::Error),
            });
            if self.toasts.len() > TOASTS_MAX {
                self.toasts.remove(0);
            }
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(TOAST_LIFE).await;
                let _ = this.update(cx, |ws, cx| {
                    ws.toasts.retain(|t| t.seq != seq);
                    cx.notify();
                });
            })
            .detach();
        }
        for n in fresh {
            self.toast_seq += 1;
            let seq = self.toast_seq;
            self.toasts.retain(|t| t.notice != n.id);
            self.toasts.push(Toast {
                seq,
                notice: n.id,
                text: crate::screens::notices::notice_text(n.key, &n.args),
                color: crate::screens::notices::severity_color(n.severity),
            });
            if self.toasts.len() > TOASTS_MAX {
                self.toasts.remove(0);
            }
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(TOAST_LIFE).await;
                let _ = this.update(cx, |ws, cx| {
                    ws.toasts.retain(|t| t.seq != seq);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn render_toasts(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        div().absolute().right(px(16.)).bottom(px(16.)).flex().flex_col().items_end().gap(px(6.)).children(
            self.toasts.iter().map(|t| {
                let seq = t.seq;
                div()
                    .id(("toast", seq as usize))
                    .debug_selector(|| "toast".into())
                    .occlude()
                    .max_w(px(420.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .bg(c(THEME.bg3))
                    .border_1()
                    .border_color(c(t.color))
                    .shadow_lg()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .child(t.text.clone())
                    .on_click(cx.listener(move |ws, _, _, cx| {
                        ws.toasts.retain(|t| t.seq != seq);
                        cx.notify();
                    }))
            }),
        )
    }

    fn toggle_screen(&mut self, s: Screen, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_open(s) { self.close(s, cx) } else { self.open(s, None, window, cx) }
    }

    fn run(&mut self, id: &str, params: Value, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            core.report(|pt| pt.run_action(id.into(), params));
        });
        cx.notify();
    }

    /// Quit, or close the window: first ask, when the project changed and the preference says so
    pub fn request(&mut self, what: Pending, window: &mut Window, cx: &mut Context<Self>) {
        let (modified, ask) = {
            let port = self.core.read(cx).port();
            (port.modified(), port.preferences().get("askBeforeClosing").and_then(Value::as_bool).unwrap_or(true))
        };
        if modified && ask {
            self.confirm = Some(what);
            cx.notify();
        } else {
            self.finish(what, window, cx);
        }
    }

    /// The selection, written to the interface document when it changed (I-3.8): the focused track, the selected
    /// desk channel and the focused plugin, which the selection-following MIDI targets follow (I-21.2)
    fn write_selection(&mut self, cx: &mut Context<Self>) {
        let root = self.core.read(cx).port().root().clone();
        let track = self.composition.read(cx).selected_track(&root);
        let channel = self.mixing.read(cx).selected;
        let plugin = self.focused_plugin;
        let text = |id: Option<NodeId>| id.map_or(Value::NULL, |id| Value::text(id.to_string()));
        let mut selection =
            self.core.read(cx).port().ui().get(ui::SELECTION).cloned().unwrap_or_else(Value::empty_object);
        let before = selection.clone();
        for (key, id) in [(ui::TRACK, track), (ui::CHANNEL, channel), (ui::PLUGIN, plugin)] {
            selection = selection.with(key, text(id));
        }
        if selection != before {
            self.core.update(cx, |core, _| {
                core.report(|pt| pt.set_ui(vec![PathStep::key(ui::SELECTION)], selection));
            });
        }
    }

    /// The project's own operation (opening, creating), over the pages from the first window on: nothing in its
    /// first 200 ms, then the shared loading component with its stage and Cancel (I-2)
    fn project_loading(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let loading = self.core.read(cx).port().loading(&lm_port::Target::Project);
        let core = self.core.clone();
        crate::ui::loading_component("project-loading", &loading, move |_, cx| {
            core.update(cx, |core, _| {
                let ops: Vec<_> = core
                    .port()
                    .activity()
                    .into_iter()
                    .filter(|a| a.target == lm_port::Target::Project)
                    .map(|a| a.id)
                    .collect();
                for op in ops {
                    core.port_mut().cancel(op);
                }
            })
        })
        .map(|l| {
            div().absolute().left(px(0.)).top(px(0.)).size_full().bg(c(THEME.bg1)).occlude().child(l).into_any_element()
        })
    }

    /// While a MIDI learn waits for a control: say so, with Cancel (I-21.2)
    fn learning_banner(&self, cx: &Context<Self>) -> Option<impl IntoElement + use<>> {
        let id = self.core.read(cx).port().learning()?;
        let core = self.core.clone();
        Some(
            div().absolute().bottom(px(16.)).left(px(0.)).w_full().flex().justify_center().child(
                div()
                    .id("learning")
                    .debug_selector(|| "learning".into())
                    .occlude()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .bg(c(THEME.bg3))
                    .border_1()
                    .border_color(c(THEME.yellow))
                    .shadow_lg()
                    .child(div().text_size(px(13.)).child(tr("Move a MIDI control…")))
                    .child(
                        button("learning-cancel", tr("Cancel"), false, 0)
                            .debug_selector(|| "learning-cancel".into())
                            .on_click(move |_, _, cx| {
                                core.update(cx, |core, cx| {
                                    core.port_mut().cancel_learn(id);
                                    cx.notify();
                                })
                            }),
                    ),
            ),
        )
    }

    /// The interface's actions that mapped MIDI controls reached (the `MidiUiAction` feed, I-21.2): run as if their
    /// keys were pressed, through the registry
    fn run_midi_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let actions = self.core.update(cx, |core, _| std::mem::take(&mut core.midi_ui));
        for a in actions {
            let Some(def) = crate::actions::find(a.action.as_str()) else { continue };
            if let Ok(action) = cx.build_action(def.gpui, None) {
                window.dispatch_action(action, cx);
            }
        }
    }

    fn finish(&mut self, what: Pending, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        match what {
            Pending::Quit => cx.quit(),
            Pending::Close => window.remove_window(),
        }
    }

    fn answer(&mut self, save: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(what) = self.confirm else { return };
        match save {
            None => {
                self.confirm = None;
                cx.notify();
            }
            Some(true) => {
                self.run("project.save", Value::NULL, cx);
                self.finish(what, window, cx);
            }
            Some(false) => self.finish(what, window, cx),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // a screen that had the focus (a text field) closed: the keys go back to the pages
        let in_a_page = self.composition.read(cx).has_focus(window, cx) || self.mixing.read(cx).has_focus(window, cx);
        if (std::mem::take(&mut self.refocus) && !in_a_page) || window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        // after opening a project: the arrangement in front, from its start (F-ARR-013)
        let key = {
            let st = self.core.read(cx).port().status();
            let p = st.get("project");
            let t = |k: &str| p.and_then(|p| p.get(k)).and_then(Value::as_text).unwrap_or("").to_string();
            format!("{}/{}", t("folder"), t("name"))
        };
        if key != self.project_key {
            let first = std::mem::replace(&mut self.project_key, key).is_empty();
            if !first {
                self.page = Page::Composition;
                self.composition.update(cx, |c, cx| c.to_start(cx));
            }
        }
        let started = Instant::now();
        let (quit, probe, animate, position) = self.core.update(cx, |core, _| {
            let quit = core.bench.frame();
            let probe = core.bench.active().then(|| core.bench.work_probe());
            core.advance_meters(started);
            let pos = core.port().telemetry().position;
            (quit, probe, pos.playing || core.bench.view(started).is_some(), pos)
        });
        if quit {
            cx.quit();
        }
        self.run_midi_actions(window, cx);
        self.collect_toasts(cx);
        if animate || !self.toasts.is_empty() {
            window.request_animation_frame();
        }
        let page_view: AnyView = if self.is_detached(Detachable::Page(self.page)) {
            // the page is in a window of its own: here, the way back
            let page = self.page;
            let workspace = cx.weak_entity();
            cx.new(|_| DetachedNote { page, workspace }).into()
        } else {
            match self.page {
                Page::Composition => self.composition.clone().into(),
                Page::Mixing => self.mixing.clone().into(),
                Page::MidiMaps => self.midi_maps.clone().into(),
            }
        };
        let prefs = self.core.read(cx).port().preferences().clone();
        let modal = self.docked(Placement::Modal);
        let confirm = self.confirm;
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(c(THEME.window))
            .text_color(c(THEME.text))
            .font_family(".SystemUIFont")
            .key_context("Workspace")
            .on_mouse_move(cx.listener(|ws, ev: &gpui::MouseMoveEvent, _, cx| {
                if ws.tempo_drag.is_some() {
                    transport::drag_tempo(
                        ws,
                        ev.position.y.as_f32(),
                        crate::keymap::holds("Transport", "transport.tempo_fine", &ev.modifiers),
                        cx,
                    );
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|ws, _: &gpui::MouseUpEvent, _, cx| transport::end_tempo(ws, cx)),
            )
            .child(lm_gui_plugins::release_guard(self.core.downgrade()))
            .on_action(cx.listener(|ws, _: &ShowComposition, _, cx| {
                ws.page = Page::Composition;
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &ShowMidiMaps, _, cx| {
                ws.page = Page::MidiMaps;
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &ShowMixing, _, cx| {
                ws.page = Page::Mixing;
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &NextPage, _, cx| {
                ws.page = match ws.page {
                    Page::Composition => Page::Mixing,
                    Page::Mixing => Page::MidiMaps,
                    Page::MidiMaps => Page::Composition,
                };
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &OpenPalette, window, cx| ws.toggle_screen(Screen::Palette, window, cx)))
            .on_action(cx.listener(|ws, _: &OpenSettings, window, cx| ws.toggle_screen(Screen::Settings, window, cx)))
            .map(|d| screens::palette::screen_actions(d, cx))
            .on_action(cx.listener(|ws, _: &Undo, _, cx| {
                ws.core.update(cx, |core, _| {
                    core.report(|pt| pt.undo());
                });
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &Redo, _, cx| {
                ws.core.update(cx, |core, _| {
                    core.report(|pt| pt.redo());
                });
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &Save, _, cx| ws.run("project.save", Value::NULL, cx)))
            .on_action(cx.listener(|ws, _: &Quit, window, cx| ws.request(Pending::Quit, window, cx)))
            .on_action(cx.listener(|ws, _: &SaveVersion, _, cx| ws.run("project.save_version", Value::NULL, cx)))
            .on_action(cx.listener(|ws, _: &SaveTemplate, _, cx| ws.run("project.save_template", Value::NULL, cx)))
            .on_action(cx.listener(|ws, _: &ShowPianoRoll, window, cx| ws.toggle_screen(Screen::PianoRoll, window, cx)))
            .on_action(
                cx.listener(|ws, _: &ShowAutomation, window, cx| ws.toggle_screen(Screen::Automation, window, cx)),
            )
            .on_action(cx.listener(|ws, _: &ShowSteps, window, cx| ws.toggle_screen(Screen::Steps, window, cx)))
            .on_action(cx.listener(|ws, _: &ToggleBrowser, window, cx| ws.toggle_screen(Screen::Browser, window, cx)))
            .on_action(cx.listener(|_, _: &ToggleFullscreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|ws, _: &DetachPage, _, cx| {
                let d = Detachable::Page(ws.page);
                if ws.is_detached(d) { ws.dock(d, cx) } else { ws.detach(d, cx) }
            }))
            .on_action(cx.listener(|ws, _: &DetachAll, _, cx| ws.detach_all(cx)))
            .on_action(cx.listener(|ws, _: &DockAll, _, cx| ws.dock_all(cx)))
            .on_action(cx.listener(|ws, _: &TapTempo, _, cx| transport::tap_tempo(ws, cx)))
            .on_action(cx.listener(|ws, _: &ToggleMasterMute, _, cx| transport::toggle_master_mute(ws, cx)))
            .on_action(cx.listener(|ws, _: &OpenManual, window, cx| {
                cx.set_global(screens::palette::ScreenArg(Some("manual")));
                ws.open(Screen::About, None, window, cx);
            }))
            .on_action(cx.listener(|ws, _: &CloseModal, window, cx| {
                if ws.confirm.take().is_none() {
                    ws.open.retain(|(s, _)| s.placement() != Placement::Modal);
                }
                // the closed modal may have had the focus (a text field): the keys go back to the pages
                window.focus(&ws.focus, cx);
                cx.notify();
            }))
            .children(crate::menus::in_window().then(|| self.render_bar(&prefs, cx)))
            .child(transport::render(self, position, cx))
            .child(self.render_tabs(cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .key_context("Pages")
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|ws, _: &TogglePlay, _, cx| ws.run("transport.toggle", Value::NULL, cx)))
                    .on_action(cx.listener(|ws, _: &Stop, _, cx| ws.run("transport.stop", Value::NULL, cx)))
                    .on_action(cx.listener(|ws, _: &Rewind, _, cx| ws.run("transport.locate", Value::int(0), cx)))
                    .on_action(cx.listener(|ws, _: &Record, _, cx| transport::record(ws, cx)))
                    .on_action(cx.listener(|ws, _: &ToggleLoop, _, cx| transport::toggle_loop(ws, cx)))
                    .children(self.docked(Placement::Left).map(|v| div().w(px(self.docks.left)).h_full().child(v)))
                    .children(self.docked(Placement::Left).map(|_| self.splitter(DockEdge::Left, cx)))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex_1()
                                    .relative()
                                    .overflow_hidden()
                                    .child(page_view)
                                    // the project opening (I-2: shown only past 200 ms, with its stage and Cancel)
                                    .children(self.project_loading(cx)),
                            )
                            .children(self.docked(Placement::Bottom).map(|_| self.splitter(DockEdge::Bottom, cx)))
                            .children(
                                self.docked(Placement::Bottom)
                                    .map(|v| div().h(px(self.docks.bottom)).flex_none().child(v)),
                            ),
                    )
                    .children(self.docked(Placement::Right).map(|_| self.splitter(DockEdge::Right, cx)))
                    .children(self.docked(Placement::Right).map(|v| div().w(px(self.docks.right)).h_full().child(v))),
            )
            .children(self.dock_drag.is_some().then(|| {
                crate::screens::piano_roll::kit::capture(cx.weak_entity(), Self::splitter_move, Self::splitter_up)
            }))
            .children(modal.map(|v| scrim("modal-scrim").child(v)))
            .children(confirm.map(|what| self.render_confirm(what, cx)))
            .children(self.learning_banner(cx))
            .children((!self.toasts.is_empty()).then(|| self.render_toasts(cx)))
            // an open menu closes with a click anywhere else
            .children(self.menu_open.map(|_| {
                div()
                    .id("menu-backdrop")
                    .absolute()
                    .left(px(0.))
                    .top(px(crate::menus::BAR_H))
                    .size_full()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|ws, _, _, cx| {
                            ws.menu_open = None;
                            cx.notify();
                        }),
                    )
            }))
            // painted last: the CPU time of the frame, from the start of render to the end of paint
            .children(probe.map(|probe| canvas(|_, _, _| {}, move |_, _, _, _| probe(started)).absolute().size(px(0.))))
    }
}

fn scrim(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::rgba(THEME.scrim))
}

impl Workspace {
    fn render_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let modified = self.core.read(cx).port().modified();
        let name = self
            .core
            .read(cx)
            .port()
            .status()
            .get("project")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_text)
            .unwrap_or("")
            .to_string();
        div()
            .h(px(TABS_H))
            .flex()
            .flex_row()
            .items_end()
            .px(px(8.))
            .gap(px(2.))
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .children([Page::Composition, Page::Mixing, Page::MidiMaps].into_iter().map(|p| {
                tab(p.title(), tr(p.title()), self.page == p).on_click(cx.listener(move |ws, _, _, cx| {
                    ws.page = p;
                    cx.notify();
                }))
            }))
            .child(div().pb(px(8.)).pl(px(12.)).text_size(px(11.)).text_color(c(THEME.text3)).child(if modified {
                format!("{name} •")
            } else {
                name
            }))
            .child(div().flex_1())
            // the launcher for every other screen (the real program reaches them from menus, the palette and
            // context actions too)
            .child(div().flex().flex_row().gap(px(3.)).pb(px(6.)).children(Screen::ALL.iter().map(|&s| {
                crate::screens::piano_roll::kit::icon_button(s.title(), s.icon(), self.is_open(s), s.title())
                    .on_click(cx.listener(move |ws, _, window, cx| ws.toggle_screen(s, window, cx)))
            })))
    }

    fn render_confirm(&self, what: Pending, cx: &Context<Self>) -> impl IntoElement {
        let title = if what == Pending::Quit { tr("Quit") } else { tr("Close") };
        scrim("confirm-scrim").child(
            div()
                .id("confirm")
                .w(px(440.))
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .rounded(px(10.))
                .bg(c(THEME.bg2))
                .border_1()
                .border_color(c(THEME.line2))
                .child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child(SharedString::from(title)))
                .child(
                    div().text_size(px(12.)).text_color(c(THEME.text2)).child(tr("The project has unsaved changes.")),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .justify_end()
                        .child(
                            button("confirm-cancel", tr("Cancel"), false, 0)
                                .on_click(cx.listener(|ws, _, window, cx| ws.answer(None, window, cx))),
                        )
                        .child(
                            button("confirm-discard", tr("Don't save"), false, 0)
                                .on_click(cx.listener(|ws, _, window, cx| ws.answer(Some(false), window, cx))),
                        )
                        .child(
                            button("confirm-save", tr("Save"), true, THEME.green)
                                .on_click(cx.listener(|ws, _, window, cx| ws.answer(Some(true), window, cx))),
                        ),
                ),
        )
    }
}

/// The app's start: language, theme and keys from the preferences, then the window
pub fn start(cx: &mut App, preferences: &Value) {
    crate::ui::i18n::set_language(preferences.get("language").and_then(Value::as_text));
    lm_gui_plugins::set_language(crate::ui::i18n::language());
    crate::ui::theme::set_theme(preferences.get("theme").and_then(Value::as_text));
    gpui_component::init(cx);
    let mode = if crate::ui::theme::theme_name() == "light" {
        gpui_component::ThemeMode::Light
    } else {
        gpui_component::ThemeMode::Dark
    };
    gpui_component::Theme::change(mode, None, cx);
    let _ = bind_keys(cx, preferences);
    crate::menus::install_native(cx);
}

// --- the docks' splitters ----------------------------------------------------------------------------------------

impl Workspace {
    /// The splitter between a dock and the page: dragging it resizes the dock
    fn splitter(&self, edge: DockEdge, cx: &Context<Self>) -> impl IntoElement {
        let id: &'static str = match edge {
            DockEdge::Bottom => "splitter-bottom",
            DockEdge::Left => "splitter-left",
            DockEdge::Right => "splitter-right",
        };
        let vertical = edge != DockEdge::Bottom;
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .flex_none()
            .when(vertical, |d| d.w(px(5.)).h_full().cursor(gpui::CursorStyle::ResizeLeftRight))
            .when(!vertical, |d| d.h(px(5.)).w_full().cursor(gpui::CursorStyle::ResizeUpDown))
            .bg(c(THEME.line))
            .hover(|s| s.bg(c(THEME.line2)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |ws, ev: &gpui::MouseDownEvent, _, cx| {
                    let at = if vertical { ev.position.x.as_f32() } else { ev.position.y.as_f32() };
                    let size = match edge {
                        DockEdge::Bottom => ws.docks.bottom,
                        DockEdge::Left => ws.docks.left,
                        DockEdge::Right => ws.docks.right,
                    };
                    ws.dock_drag = Some((edge, at, size));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
    }

    fn splitter_move(&mut self, ev: &gpui::MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((edge, from, size)) = self.dock_drag else { return };
        let (lo, hi) = edge.range();
        // the bottom dock grows upward, the left one rightward, the right one leftward
        let d = match edge {
            DockEdge::Bottom => from - ev.position.y.as_f32(),
            DockEdge::Left => ev.position.x.as_f32() - from,
            DockEdge::Right => from - ev.position.x.as_f32(),
        };
        let v = (size + d).clamp(lo, hi);
        match edge {
            DockEdge::Bottom => self.docks.bottom = v,
            DockEdge::Left => self.docks.left = v,
            DockEdge::Right => self.docks.right = v,
        }
        cx.notify();
    }

    fn splitter_up(&mut self, _ev: &gpui::MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.dock_drag.take().is_none() {
            return;
        }
        // remembered in the interface document (I-3.8): no undo, saved with the project; the object whole, so it
        // exists the first time
        let d = self.docks;
        let docks = lm_port::Value::object([
            ("bottom", lm_port::Value::int(d.bottom.round() as i64)),
            ("left", lm_port::Value::int(d.left.round() as i64)),
            ("right", lm_port::Value::int(d.right.round() as i64)),
        ]);
        self.core.update(cx, |core, _| {
            core.report(|pt| pt.set_ui(vec![lm_port::PathStep::key("docks")], docks));
        });
        cx.notify();
    }
}

/// What a detached page leaves in the main window: its name and the way back
struct DetachedNote {
    page: Page,
    workspace: gpui::WeakEntity<Workspace>,
}

impl Render for DetachedNote {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (page, ws) = (self.page, self.workspace.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .child(div().text_size(px(13.)).text_color(c(THEME.text3)).child(tr(page.title())))
            .child(button("dock-back", tr("Dock"), false, 0).debug_selector(|| "dock-back".into()).on_click(
                move |_, _, cx| {
                    let _ = ws.update(cx, |ws, cx| ws.dock(Detachable::Page(page), cx));
                },
            ))
    }
}

#[cfg(test)]
mod detach_tests {
    use super::*;

    #[test]
    fn detached_keys_round_trip() {
        for d in [
            Detachable::Page(Page::Mixing),
            Detachable::Page(Page::MidiMaps),
            Detachable::Screen(Screen::PianoRoll),
            Detachable::Screen(Screen::Browser),
        ] {
            assert_eq!(Detachable::parse(&d.key()), Some(d));
        }
    }

    #[test]
    fn a_modal_screen_cannot_be_detached() {
        assert_eq!(Detachable::parse("screen:Settings"), None);
        assert_eq!(Detachable::parse("nonsense"), None);
    }
}
