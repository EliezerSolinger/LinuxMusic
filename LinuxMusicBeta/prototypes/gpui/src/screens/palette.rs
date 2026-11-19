//! Owner: agent D. The command palette (Cmd/Ctrl+K, spec item 6): a fuzzy search over the action registry
//! (`fake_core::actions`, I-19), with each action's group and shortcut. Before typing it lists the actions used last.
//! Up and down move, Enter runs, Esc closes. A disabled action (saving a read-only project) is shown dimmed with
//! why, and does not run.
//!
//! It also has what the other screens of agent D share: running an action by id (`run_action`, also used by the
//! notices and the settings) and writing its keys for the platform (`keys_label`).

use fake_core::actions::{self, ActionSpec, Run};
use gpui::{
    App, Context, Entity, FontWeight, HighlightStyle, KeyDownEvent, Render, ScrollHandle, StyledText, Subscription,
    Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};

use super::{Screen, ScreenCx};
use crate::pages::Page;
use crate::ui::{THEME, c, caption, tr, trf};

/// How many results the list shows at most
const MAX_RESULTS: usize = 40;

pub struct Palette {
    scx: ScreenCx,
    query: Entity<InputState>,
    selected: usize,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

/// One row: the action and the matched characters of its label (for the highlight)
struct Hit {
    spec: &'static ActionSpec,
    label: &'static str,
    matched: Vec<usize>,
    enabled: bool,
}

impl Palette {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Type a command…")));
        query.update(cx, |q, cx| q.focus(window, cx));
        let sub = cx.subscribe_in(&query, window, |p: &mut Palette, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => {
                p.selected = 0;
                p.scroll.scroll_to_item(0);
                cx.notify();
            }
            InputEvent::PressEnter { .. } => p.run_selected(window, cx),
            _ => {}
        });
        Palette { scx, query, selected: 0, scroll: ScrollHandle::new(), _subs: vec![sub] }
    }

    fn hits(&self, cx: &App) -> Vec<Hit> {
        let q = self.query.read(cx).value().to_string();
        let session = &self.scx.core.read(cx).session;
        let usable = |a: &&'static ActionSpec| a.palette;
        let mut scored: Vec<(i32, Hit)> = Vec::new();
        if q.trim().is_empty() {
            // the recent ones first, then everything in the registry's order
            let recent = &session.services.prefs.recent_actions;
            let mut order: Vec<&'static ActionSpec> = recent.iter().filter_map(|id| actions::find(id)).collect();
            order.extend(actions::REGISTRY.iter().filter(|a| !recent.contains(&a.id)));
            for (i, a) in order.into_iter().filter(usable).enumerate() {
                scored.push((-(i as i32), Hit { spec: a, label: tr(a.label), matched: vec![], enabled: (a.enabled)(session) }));
            }
        } else {
            for a in actions::REGISTRY.iter().filter(usable) {
                let label = tr(a.label);
                // the label first; the group and the id (English) also find it, with a lower score
                let hit = actions::fuzzy(&q, label)
                    .or_else(|| actions::fuzzy(&q, tr(a.group)).map(|(s, _)| (s - 30, vec![])))
                    .or_else(|| actions::fuzzy(&q, a.id).map(|(s, _)| (s - 40, vec![])));
                if let Some((score, matched)) = hit {
                    scored.push((score, Hit { spec: a, label, matched, enabled: (a.enabled)(session) }));
                }
            }
            scored.sort_by(|a, b| b.0.cmp(&a.0));
        }
        scored.into_iter().take(MAX_RESULTS).map(|(_, h)| h).collect()
    }

    fn run_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hits = self.hits(cx);
        let Some(h) = hits.get(self.selected) else { return };
        if !h.enabled {
            return;
        }
        let id = h.spec.id;
        let scx = self.scx.clone();
        scx.close(cx);
        run_action(&scx, id, "", window, cx);
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let n = self.hits(cx).len();
        if n == 0 {
            return;
        }
        let step: isize = match ev.keystroke.key.as_str() {
            "down" => 1,
            "up" => -1,
            "pagedown" => 8,
            "pageup" => -8,
            _ => return,
        };
        self.selected = (self.selected as isize + step).clamp(0, n as isize - 1) as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hits = self.hits(cx);
        let empty = hits.is_empty();
        let typed = !self.query.read(cx).value().trim().is_empty();
        let prefs = &self.scx.core.read(cx).session.services.prefs;
        let rows: Vec<_> = hits
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                let sel = i == self.selected;
                let keys = actions::shortcut(prefs, h.spec.id);
                let hl = HighlightStyle { color: Some(c(THEME.green).into()), font_weight: Some(FontWeight::BOLD), ..Default::default() };
                let ranges: Vec<_> = char_ranges(h.label, &h.matched).into_iter().map(|r| (r, hl)).collect();
                div()
                    .id(("palette-row", i))
                    .debug_selector(move || format!("palette-row-{i}"))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .when(sel, |d| d.bg(c(THEME.bg4)))
                    .when(!h.enabled, |d| d.opacity(0.45))
                    .cursor_pointer()
                    .on_mouse_move(cx.listener(move |p, _, _, cx| {
                        if p.selected != i {
                            p.selected = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |p, _, window, cx| {
                        p.selected = i;
                        p.run_selected(window, cx);
                    }))
                    .child(div().flex_1().text_size(px(13.)).child(StyledText::new(h.label).with_highlights(ranges)))
                    .when(!h.enabled, |d| d.child(caption(tr("not available now"))))
                    .child(div().w(px(110.)).flex().justify_end().child(caption(tr(h.spec.group))))
                    .child(div().w(px(90.)).flex().justify_end().children(keys.map(|k| kbd(keys_label(&k)))))
            })
            .collect();
        div()
            .id("palette")
            .w(px(640.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .capture_key_down(cx.listener(Self::on_key))
            .child(div().p(px(10.)).border_b_1().border_color(c(THEME.line)).child(Input::new(&self.query)))
            .child(
                div()
                    .px(px(14.))
                    .pt(px(8.))
                    .child(caption(if typed { tr("Commands") } else { tr("Used recently, then every command") })),
            )
            .child(
                div()
                    .id("palette-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(6.))
                    .children(rows)
                    .when(empty, |d| {
                        d.child(div().p(px(16.)).text_color(c(THEME.text3)).text_size(px(13.)).child(tr("No command has these letters")))
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(14.))
                    .px(px(14.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(c(THEME.line))
                    .child(caption(trf("{0} {1} to choose", &[&"↑", &"↓"])))
                    .child(caption(trf("{0} to run", &[&tr("Enter")])))
                    .child(caption(trf("{0} to close", &[&"Esc"]))),
            )
    }
}

/// The byte ranges of the chars at `pos` (merged when consecutive)
fn char_ranges(text: &str, pos: &[usize]) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for (ci, (b, ch)) in text.char_indices().enumerate() {
        if pos.contains(&ci) {
            let r = b..b + ch.len_utf8();
            match out.last_mut() {
                Some(last) if last.end == r.start => last.end = r.end,
                _ => out.push(r),
            }
        }
    }
    out
}

/// A key cap
pub fn kbd(text: String) -> impl IntoElement {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(c(THEME.line2))
        .bg(c(THEME.bg3))
        .text_size(px(11.))
        .text_color(c(THEME.text2))
        .child(text)
}

/// Keys as the platform writes them: `primary-shift-s` is ⌘⇧S on macOS and Ctrl+Shift+S elsewhere
pub fn keys_label(keys: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let parts: Vec<&str> = keys.split('-').filter(|p| !p.is_empty()).collect();
    let (mods, key) = match parts.split_last() {
        Some((k, m)) => (m.to_vec(), *k),
        None => (vec![], ","),
    };
    // "primary-," splits into ["primary", ","]... and "primary--" would be the minus key
    let key = if keys.ends_with("--") { "-" } else { key };
    let key_name = match key {
        "space" => tr("Space").to_string(),
        "home" => tr("Home").to_string(),
        "end" => tr("End").to_string(),
        "tab" => "Tab".to_string(),
        "enter" => tr("Enter").to_string(),
        "escape" => "Esc".to_string(),
        "delete" => tr("Delete").to_string(),
        "backspace" => "⌫".to_string(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        k => k.to_uppercase(),
    };
    let mut out = String::new();
    for m in mods {
        let s = match (m, mac) {
            ("primary", true) | ("cmd", true) => "⌘",
            ("primary", false) | ("ctrl", false) => "Ctrl+",
            ("ctrl", true) => "⌃",
            ("shift", true) => "⇧",
            ("shift", false) => "Shift+",
            ("alt", true) => "⌥",
            ("alt", false) => "Alt+",
            (_, _) => "",
        };
        out.push_str(s);
    }
    out.push_str(&key_name);
    out
}

/// The keystroke as the registry writes it (`primary-shift-s`), from a key event
pub fn keys_of(ev: &KeyDownEvent) -> Option<String> {
    let k = &ev.keystroke;
    let key = k.key.as_str();
    if matches!(key, "shift" | "control" | "alt" | "cmd" | "platform" | "function" | "fn" | "") {
        return None;
    }
    let m = &k.modifiers;
    let mut s = String::new();
    let primary = if cfg!(target_os = "macos") { m.platform } else { m.control };
    if primary {
        s.push_str("primary-");
    }
    if cfg!(target_os = "macos") && m.control {
        s.push_str("ctrl-");
    }
    if m.alt {
        s.push_str("alt-");
    }
    if m.shift {
        s.push_str("shift-");
    }
    s.push_str(key);
    Some(s)
}

/// The interface's screen by its name in the registry (`Run::Screen("Settings", ..)`)
pub fn screen_by_name(name: &str) -> Option<Screen> {
    Screen::ALL.iter().copied().find(|s| format!("{s:?}") == name)
}

/// Runs an action of the registry by id, with the argument a notice gives. A disabled action does nothing.
pub fn run_action(scx: &ScreenCx, id: &str, arg: &str, window: &mut Window, cx: &mut App) {
    let Some(a) = actions::find(id) else { return };
    if !(a.enabled)(&scx.core.read(cx).session) {
        return;
    }
    scx.core.update(cx, |core, cx| {
        if a.palette {
            core.session.services.prefs.used_action(a.id);
        }
        match a.run {
            Run::Core(f) => f(&mut core.session),
            Run::CoreWith(f) => f(&mut core.session, arg),
            Run::Screen(_, part) => core.session.services.screen_arg = part,
            _ => {}
        }
        cx.notify();
    });
    match a.run {
        Run::Screen(name, _) => {
            if let Some(s) = screen_by_name(name) {
                scx.open(s, window, cx);
            }
        }
        Run::Page(p) => {
            let page = if p == "mixing" { Page::Mixing } else { Page::Composition };
            let _ = scx.workspace.update(cx, |ws, cx| {
                ws.page = page;
                cx.notify();
            });
        }
        Run::Reveal => {
            let path = expand_home(arg);
            if std::path::Path::new(&path).exists() {
                cx.reveal_path(std::path::Path::new(&path));
            }
        }
        Run::Core(_) | Run::CoreWith(..) => {}
    }
}

/// `~/x` to the home folder
pub fn expand_home(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => p.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! The shell under GPUI's headless test platform, for the screens' tests
    use gpui::{Entity, TestAppContext, VisualTestContext};

    use crate::screens::Screen;
    use crate::workspace::{self, Workspace};

    pub fn open(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            workspace::bind_keys(cx);
        });
        cx.add_window_view(|window, cx| Workspace::new(window, cx))
    }

    pub fn open_screen(ws: &Entity<Workspace>, cx: &mut VisualTestContext, s: Screen) {
        ws.update_in(cx, |ws, window, cx| ws.open(s, window, cx));
        cx.run_until_parked();
    }

    pub fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let p = cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is drawn")).center();
        cx.simulate_click(p, gpui::Modifiers::none());
        cx.run_until_parked();
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[gpui::test]
    fn typing_finds_and_enter_runs(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        open_screen(&ws, cx, Screen::Palette);
        cx.simulate_input("exporaud");
        cx.run_until_parked();
        assert!(cx.debug_bounds("palette-row-0").is_some());
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(ws.is_open(Screen::Export), "the palette ran \"Export audio\"");
            assert!(!ws.is_open(Screen::Palette), "and closed");
        });
    }

    #[gpui::test]
    fn arrows_move_and_page_actions_switch_pages(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        open_screen(&ws, cx, Screen::Palette);
        cx.simulate_input("mixing");
        cx.run_until_parked();
        cx.simulate_keystrokes("down up");
        click(cx, "palette-row-0");
        ws.read_with(cx, |ws, _| assert_eq!(ws.page, Page::Mixing));
    }

    #[test]
    fn keys_are_written_for_the_platform() {
        let s = keys_label("primary-shift-s");
        if cfg!(target_os = "macos") { assert_eq!(s, "⌘⇧S") } else { assert_eq!(s, "Ctrl+Shift+S") }
        assert_eq!(char_ranges("Exportar áudio", &[0, 9, 10]), vec![0..1, 9..12]);
    }
}
