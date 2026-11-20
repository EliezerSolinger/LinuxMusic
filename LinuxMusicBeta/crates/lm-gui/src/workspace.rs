//! The shell: the transport bar over everything, the page tabs (Composition, Mixing: separate pages, the user's
//! decision of 19/11), the docks for the editors and side windows, the modals, the keys of the registry, and asking
//! before losing changes. It owns the `Core` entity and the page views, and redraws when the core says something
//! changed (it observes the entity) or, while playing, every display frame (the cursor).

use std::time::Instant;

use gpui::{
    AnyView, App, Context, Entity, FocusHandle, FontWeight, Render, SharedString, Subscription, Window, canvas, div,
    prelude::*, px,
};
use lm_port::{CorePort, NodeId, Value};

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
    open: Vec<(Screen, AnyView)>,
    pub display_format: transport::DisplayFormat,
    pub tempo_drag: Option<transport::TempoDrag>,
    /// Asking before losing changes
    pub confirm: Option<Pending>,
    _observe: Subscription,
}

impl Workspace {
    pub fn new(port: Box<dyn CorePort>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let core = cx.new(|cx| Core::new(port, cx));
        let weak = cx.weak_entity();
        let composition = cx.new(|cx| CompositionPage::new(core.clone(), weak.clone(), cx));
        let mixing = cx.new(|cx| MixingPage::new(core.clone(), weak.clone(), cx));
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
            confirm: None,
            _observe: observe,
        };
        // LM_OPEN=Settings,PianoRoll... opens screens at start (screenshots, and each author's own testing)
        if let Ok(list) = std::env::var("LM_OPEN") {
            for name in list.split(',') {
                if let Some(s) = Screen::ALL.iter().find(|s| format!("{s:?}") == name.trim()) {
                    ws.open(*s, None, window, cx);
                }
            }
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
        self.open.retain(|(o, _)| o.placement() != place);
        let scx = ScreenCx { core: self.core.clone(), workspace: cx.weak_entity(), screen: s, target };
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
        if self.is_open(s) { self.close(s, cx) } else { self.open(s, None, window, cx) }
    }

    fn run(&mut self, id: &str, params: Value, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            let _ = core.port_mut().run_action(id.into(), params);
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
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
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
        if animate {
            window.request_animation_frame();
        }
        let page_view: AnyView = match self.page {
            Page::Composition => self.composition.clone().into(),
            Page::Mixing => self.mixing.clone().into(),
        };
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
                    transport::drag_tempo(ws, ev.position.y.as_f32(), ev.modifiers.shift, cx);
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|ws, _: &gpui::MouseUpEvent, _, cx| transport::end_tempo(ws, cx)),
            )
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
            .map(|d| screens::palette::screen_actions(d, cx))
            .on_action(cx.listener(|ws, _: &Undo, _, cx| {
                ws.core.update(cx, |core, _| {
                    let _ = core.port_mut().undo();
                });
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &Redo, _, cx| {
                ws.core.update(cx, |core, _| {
                    let _ = core.port_mut().redo();
                });
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &Save, _, cx| ws.run("project.save", Value::NULL, cx)))
            .on_action(cx.listener(|ws, _: &Quit, window, cx| ws.request(Pending::Quit, window, cx)))
            .on_action(cx.listener(|ws, _: &CloseModal, window, cx| {
                if ws.confirm.take().is_none() {
                    ws.open.retain(|(s, _)| s.placement() != Placement::Modal);
                }
                // the closed modal may have had the focus (a text field): the keys go back to the pages
                window.focus(&ws.focus, cx);
                cx.notify();
            }))
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
                    .children(
                        self.docked(Placement::Left).map(|v| {
                            div().w(px(SIDE_DOCK_W)).h_full().border_r_1().border_color(c(THEME.line)).child(v)
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(div().flex_1().overflow_hidden().child(page_view))
                            .children(
                                self.docked(Placement::Bottom).map(|v| {
                                    div().h(px(BOTTOM_DOCK_H)).border_t_2().border_color(c(THEME.line)).child(v)
                                }),
                            ),
                    )
                    .children(
                        self.docked(Placement::Right).map(|v| {
                            div().w(px(SIDE_DOCK_W)).h_full().border_l_1().border_color(c(THEME.line)).child(v)
                        }),
                    ),
            )
            .children(modal.map(|v| scrim("modal-scrim").child(v)))
            .children(confirm.map(|what| self.render_confirm(what, cx)))
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
            .children([Page::Composition, Page::Mixing].into_iter().map(|p| {
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
                button(s.title(), tr(s.title()), self.is_open(s), THEME.green)
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
                    div()
                        .text_size(px(12.))
                        .text_color(c(THEME.text2))
                        .child(tr("The project has changes that were not saved.")),
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
    crate::ui::theme::set_theme(preferences.get("theme").and_then(Value::as_text));
    gpui_component::init(cx);
    let mode = if crate::ui::theme::theme_name() == "light" {
        gpui_component::ThemeMode::Light
    } else {
        gpui_component::ThemeMode::Dark
    };
    gpui_component::Theme::change(mode, None, cx);
    let _ = bind_keys(cx, preferences);
}
