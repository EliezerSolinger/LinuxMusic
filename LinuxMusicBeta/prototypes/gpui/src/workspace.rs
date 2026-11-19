//! The shell: the transport bar over everything, the page tabs (Composition, Mixing), the docks for the editors
//! and side windows, and the modals. It owns the `Core` entity and the page views, and redraws when the core says
//! something changed (it observes the entity) or, while playing, every display frame (the cursor).
//!
//! Keys: Cmd/Ctrl+1 composition, Cmd/Ctrl+2 mixing, Ctrl+Tab switches; Space plays or stops (on the pages only, so a
//! text field keeps its spaces); Cmd/Ctrl+K the command palette; Cmd/Ctrl+, the settings; Esc closes a modal.

use std::time::Instant;

use gpui::{
    AnyView, App, Context, Entity, FocusHandle, KeyBinding, Render, Subscription, Window, actions, canvas, div,
    prelude::*, px,
};

use crate::core::Core;
use crate::pages::composition::CompositionPage;
use crate::pages::mixing::{self, MixingPage};
use crate::pages::{Page, transport};
use crate::screens::{self, Placement, Screen, ScreenCx};
use crate::ui::{THEME, button, c, tab, tr};

actions!(linuxmusic, [ShowComposition, ShowMixing, NextPage, TogglePlay, OpenPalette, OpenSettings, CloseModal]);

pub const TABS_H: f32 = 34.0;
/// Height of what is above the page: the transport bar and the tabs
pub const CHROME_H: f32 = fake_core::view::TRANSPORT_H + TABS_H;
const BOTTOM_DOCK_H: f32 = 300.0;
const SIDE_DOCK_W: f32 = 380.0;

/// Cmd on macOS, Ctrl elsewhere (this GPUI's keystroke parser has no "secondary" modifier)
fn primary(key: &str) -> &'static str {
    Box::leak(if cfg!(target_os = "macos") { format!("cmd-{key}") } else { format!("ctrl-{key}") }.into_boxed_str())
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(primary("1"), ShowComposition, None),
        KeyBinding::new(primary("2"), ShowMixing, None),
        KeyBinding::new("ctrl-tab", NextPage, None),
        KeyBinding::new("space", TogglePlay, Some("Pages")),
        KeyBinding::new(primary("k"), OpenPalette, None),
        KeyBinding::new(primary(","), OpenSettings, None),
        KeyBinding::new("escape", CloseModal, None),
        // the desk (F-MIX-013)
        KeyBinding::new("left", mixing::SelectLeft, Some("Mixing")),
        KeyBinding::new("right", mixing::SelectRight, Some("Mixing")),
        KeyBinding::new("up", mixing::FaderUp, Some("Mixing")),
        KeyBinding::new("down", mixing::FaderDown, Some("Mixing")),
        KeyBinding::new("shift-up", mixing::FaderUpCoarse, Some("Mixing")),
        KeyBinding::new("shift-down", mixing::FaderDownCoarse, Some("Mixing")),
        KeyBinding::new("alt-up", mixing::FaderUpFine, Some("Mixing")),
        KeyBinding::new("alt-down", mixing::FaderDownFine, Some("Mixing")),
        KeyBinding::new("shift-insert", mixing::AddChannel, Some("Mixing")),
    ]);
}

pub struct Workspace {
    pub core: Entity<Core>,
    focus: FocusHandle,
    pub page: Page,
    pub composition: Entity<CompositionPage>,
    pub mixing: Entity<MixingPage>,
    open: Vec<(Screen, AnyView)>,
    pub display_format: transport::DisplayFormat,
    pub tempo_drag: Option<transport::TempoDrag>,
    _observe: Subscription,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let core = cx.new(Core::new);
        let composition = cx.new(|cx| CompositionPage::new(core.clone(), cx));
        let weak = cx.weak_entity();
        let mixing = cx.new(|cx| MixingPage::new(core.clone(), weak, cx));
        let observe = cx.observe(&core, |_, _, cx| cx.notify());
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut ws = Workspace {
            core,
            focus,
            page: if std::env::var("LM_PAGE").as_deref() == Ok("mixing") { Page::Mixing } else { Page::Composition },
            composition,
            mixing,
            open: Vec::new(),
            display_format: Default::default(),
            tempo_drag: None,
            _observe: observe,
        };
        // LM_OPEN=Settings,PianoRoll... opens screens at start (screenshots, and each author's own testing)
        if let Ok(list) = std::env::var("LM_OPEN") {
            for name in list.split(',') {
                if let Some(s) = Screen::ALL.iter().find(|s| format!("{s:?}") == name.trim()) {
                    ws.open(*s, window, cx);
                }
            }
        }
        ws
    }

    pub fn open(&mut self, s: Screen, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.iter().any(|(o, _)| *o == s) {
            return;
        }
        // one modal at a time, one screen per dock
        let place = s.placement();
        self.open.retain(|(o, _)| o.placement() != place);
        let scx = ScreenCx { core: self.core.clone(), workspace: cx.weak_entity(), screen: s };
        let view = screens::open(scx, window, cx);
        self.open.push((s, view));
        cx.notify();
    }

    pub fn close(&mut self, s: Screen, cx: &mut Context<Self>) {
        self.open.retain(|(o, _)| *o != s);
        cx.notify();
    }

    pub fn is_open(&self, s: Screen) -> bool {
        self.open.iter().any(|(o, _)| *o == s)
    }

    fn docked(&self, p: Placement) -> Option<AnyView> {
        self.open.iter().find(|(s, _)| s.placement() == p).map(|(_, v)| v.clone())
    }

    fn toggle_screen(&mut self, s: Screen, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_open(s) { self.close(s, cx) } else { self.open(s, window, cx) }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = Instant::now();
        let (quit, probe, animate, cursor) = self.core.update(cx, |core, _| {
            let quit = core.bench.frame();
            let probe = core.bench.active().then(|| core.bench.work_probe());
            core.advance_meters(started);
            let pos = core.port().telemetry().position;
            (quit, probe, pos.playing || core.bench.view(started).is_some(), pos.at(started))
        });
        if quit {
            cx.quit();
        }
        if animate {
            window.request_animation_frame();
        }
        let page_view: AnyView = match self.page {
            Page::Composition => self.composition.clone().into(),
            Page::Mixing => self.mixing.clone().into(),
        };
        let modal = self.docked(Placement::Modal);
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
                    transport::drag_tempo(ws, ev.position.y.as_f32(), ev.modifiers.shift, cx);
                }
            }))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(|ws, _: &gpui::MouseUpEvent, _, cx| transport::end_tempo(ws, cx)))
            .on_action(cx.listener(|ws, _: &ShowComposition, _, cx| {
                ws.page = Page::Composition;
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &ShowMixing, _, cx| {
                ws.page = Page::Mixing;
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &NextPage, _, cx| {
                ws.page = if ws.page == Page::Composition { Page::Mixing } else { Page::Composition };
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &OpenPalette, window, cx| ws.toggle_screen(Screen::Palette, window, cx)))
            .on_action(cx.listener(|ws, _: &OpenSettings, window, cx| ws.toggle_screen(Screen::Settings, window, cx)))
            .on_action(cx.listener(|ws, _: &CloseModal, _, cx| {
                ws.open.retain(|(s, _)| s.placement() != Placement::Modal);
                cx.notify();
            }))
            .child(transport::render(self, cursor, cx))
            .child(self.render_tabs(cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .key_context("Pages")
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|ws, _: &TogglePlay, _, cx| {
                        ws.core.update(cx, |core, _| {
                            let _ = core.port_mut().run_action("transport.toggle", fake_core::Value::Bool(true));
                        });
                        cx.notify();
                    }))
                    .children(self.docked(Placement::Left).map(|v| div().w(px(SIDE_DOCK_W)).h_full().border_r_1().border_color(c(THEME.line)).child(v)))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(div().flex_1().overflow_hidden().child(page_view))
                            .children(self.docked(Placement::Bottom).map(|v| {
                                div().h(px(BOTTOM_DOCK_H)).border_t_2().border_color(c(THEME.line)).child(v)
                            })),
                    )
                    .children(self.docked(Placement::Right).map(|v| div().w(px(SIDE_DOCK_W)).h_full().border_l_1().border_color(c(THEME.line)).child(v))),
            )
            .children(modal.map(|v| {
                div()
                    .id("modal-scrim")
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::rgba(THEME.scrim))
                    .child(v)
            }))
            // painted last: the CPU time of the frame, from the start of render to the end of paint
            .children(probe.map(|probe| canvas(|_, _, _| {}, move |_, _, _, _| probe(started)).absolute().size(px(0.))))
    }
}

impl Workspace {
    fn render_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
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
            .children([Page::Composition, Page::Mixing].into_iter().map(|p| {
                tab(p.title(), tr(p.title()), self.page == p).on_click(cx.listener(move |ws, _, _, cx| {
                    ws.page = p;
                    cx.notify();
                }))
            }))
            .child(div().flex_1())
            // the prototype's launcher for every other screen (the real program reaches them from menus,
            // the palette and context actions)
            .child(
                div().flex().flex_row().gap(px(3.)).pb(px(6.)).children(Screen::ALL.iter().map(|&s| {
                    button(s.title(), tr(s.title()), self.is_open(s), THEME.green)
                        .on_click(cx.listener(move |ws, _, window, cx| ws.toggle_screen(s, window, cx)))
                })),
            )
    }
}
