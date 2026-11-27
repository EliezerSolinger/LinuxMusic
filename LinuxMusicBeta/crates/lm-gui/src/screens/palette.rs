//! Owner: agent D. The command palette (Cmd/Ctrl+K, spec item 6): a fuzzy search over the action registry
//! (`actions::REGISTRY`, I-19), without accents, with each action's group and keys as the platform writes them (⌘⇧S,
//! Ctrl+Shift+S); the recently used first before typing; up and down move, Enter runs, Esc closes. Running an action
//! dispatches its GPUI action, the same path as its keys.
//!
//! Also here, for agent D's screens: the actions that open them (`screen_actions`, hooked by the shell), the part of a
//! screen an action asks for (`ScreenArg`), the accent folding and the keys' text.

use gpui::{
    App, Context, Div, Entity, FontWeight, Global, HighlightStyle, KeyDownEvent, Render, ScrollHandle, StyledText,
    Subscription, Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use lm_port::Value;

use super::{Screen, ScreenCx};
use crate::actions::{
    ActionDef, ExportMidi, ImportFiles, OpenAbout, OpenExport, OpenNewProject, OpenNotes, OpenNotices, OpenProject,
    REGISTRY,
    SaveAs, keys_of,
};
use crate::ui::{THEME, c, caption, tr};
use crate::workspace::Workspace;

/// How many results the list shows at most
const MAX_RESULTS: usize = 40;

/// The part of a screen an action asked for ("save_as", "keys"): interface state, taken by the screen as it opens
pub struct ScreenArg(pub Option<&'static str>);
impl Global for ScreenArg {}

/// Takes the part asked for the screen opening now
pub fn take_screen_arg(cx: &mut App) -> Option<&'static str> {
    cx.try_global::<ScreenArg>()?;
    cx.update_global::<ScreenArg, _>(|a, _| a.0.take())
}

/// The recently run palette actions (interface state of this run)
struct Recent(Vec<&'static str>);
impl Global for Recent {}

/// The shell's handlers of the actions that open agent D's screens: one hook in the workspace's root
pub fn screen_actions(d: Div, cx: &mut Context<Workspace>) -> Div {
    fn open(
        ws: &mut Workspace,
        s: Screen,
        part: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        cx.set_global(ScreenArg(part));
        ws.open(s, None, window, cx);
    }
    d.on_action(cx.listener(|ws, _: &OpenExport, window, cx| open(ws, Screen::Export, None, window, cx)))
        .on_action(cx.listener(|ws, _: &OpenProject, window, cx| open(ws, Screen::Project, Some("open"), window, cx)))
        .on_action(cx.listener(|ws, _: &SaveAs, window, cx| open(ws, Screen::Project, Some("save_as"), window, cx)))
        .on_action(cx.listener(|ws, _: &OpenNewProject, window, cx| open(ws, Screen::NewProject, None, window, cx)))
        .on_action(cx.listener(|ws, _: &OpenNotes, window, cx| open(ws, Screen::Project, Some("notes"), window, cx)))
        .on_action(cx.listener(|ws, _: &OpenAbout, window, cx| open(ws, Screen::About, None, window, cx)))
        .on_action(cx.listener(|ws, _: &OpenNotices, window, cx| open(ws, Screen::Notices, None, window, cx)))
        // Import (F-PRJ-019, F-PRJ-020): the system's dialog when the files are local, at the cursor
        .on_action(cx.listener(|ws, _: &ImportFiles, window, cx| {
            let core = ws.core.clone();
            if super::files::local(&core, cx) {
                super::files::choose(cx, "Import (MIDI, audio)", false, true, move |paths, cx| {
                    let tick = {
                        let p = core.read(cx).port();
                        p.tempo_map().tick_at(p.telemetry().position.samples)
                    };
                    super::files::import_paths(&core, &paths, tick, cx);
                });
            } else {
                open(ws, Screen::Project, Some("import"), window, cx);
            }
        }))
        // Export MIDI (F-PRJ-021)
        .on_action(cx.listener(|ws, _: &ExportMidi, window, cx| {
            let core = ws.core.clone();
            if super::files::local(&core, cx) {
                let (folder, name) = {
                    let st = core.read(cx).port().status();
                    let p = st.get("project");
                    let t = |k: &str| p.and_then(|p| p.get(k)).and_then(Value::as_text).unwrap_or("").to_string();
                    (t("folder"), t("name"))
                };
                let folder = std::path::PathBuf::from(super::palette::expand_home(&folder));
                super::files::choose_new(cx, &folder, &format!("{name}.mid"), move |path, cx| {
                    core.update(cx, |c, cx| {
                        let params = Value::object([("path", Value::text(path.display().to_string()))]);
                        c.report(|pt| pt.run_action("midi.export".into(), params));
                        cx.notify();
                    });
                });
            } else {
                open(ws, Screen::Project, Some("export_midi"), window, cx);
            }
        }))
}

/// `~/x` to the home folder (a path the dialogs open in)
pub fn expand_home(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => p.to_string(),
    }
}

/// Lower case without accents ("Configurações" is found by "configuracoes", F-CFG-001)
pub fn fold_char(c: char) -> char {
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        c => c,
    }
}

pub fn fold(s: &str) -> String {
    s.chars().map(fold_char).collect()
}

/// Fuzzy match of `query` in `text` (subsequence, without accents): the score rewards word starts and runs; the
/// positions (in chars) are for the highlight
pub fn fuzzy(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).map(fold_char).collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let t: Vec<char> = text.chars().collect();
    let lower: Vec<char> = t.iter().map(|c| fold_char(*c)).collect();
    let mut best: Option<(i32, Vec<usize>)> = None;
    for start in 0..lower.len() {
        if lower[start] != q[0] {
            continue;
        }
        let mut pos = vec![start];
        let mut i = start + 1;
        for &qc in &q[1..] {
            while i < lower.len() && lower[i] != qc {
                i += 1;
            }
            if i == lower.len() {
                break;
            }
            pos.push(i);
            i += 1;
        }
        if pos.len() < q.len() {
            break;
        }
        let mut score = 0;
        for (k, &p) in pos.iter().enumerate() {
            if p == 0 || !t[p - 1].is_alphanumeric() {
                score += 10;
            }
            if k > 0 && pos[k - 1] + 1 == p {
                score += 6;
            } else if k > 0 {
                score -= (p - pos[k - 1]) as i32;
            }
        }
        score -= start as i32 / 2;
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, pos));
        }
    }
    best
}

/// Keys as the platform writes them: `secondary-shift-s` is ⌘⇧S on macOS and Ctrl+Shift+S elsewhere
pub fn keys_label(keys: &str) -> String {
    let mac = cfg!(target_os = "macos");
    keys.split(' ')
        .map(|stroke| {
            let key_minus = stroke.ends_with("--") || stroke == "-";
            let parts: Vec<&str> = stroke.split('-').filter(|p| !p.is_empty()).collect();
            let (mods, key) = match parts.split_last() {
                Some((k, m)) if !key_minus => (m.to_vec(), *k),
                Some((_, _)) => (parts.clone(), "-"),
                None => (Vec::new(), "-"),
            };
            let key = match key {
                "space" => tr("Space").to_string(),
                "enter" => tr("Enter").to_string(),
                "escape" => "Esc".into(),
                "delete" => tr("Delete").to_string(),
                "backspace" => "⌫".into(),
                "home" => tr("Home").to_string(),
                "tab" => "Tab".into(),
                "up" => "↑".into(),
                "down" => "↓".into(),
                "left" => "←".into(),
                "right" => "→".into(),
                "insert" => "Ins".into(),
                k => k.to_uppercase(),
            };
            let mut out = String::new();
            for m in mods {
                out.push_str(match (m, mac) {
                    ("secondary" | "cmd", true) => "⌘",
                    ("secondary" | "ctrl", false) => "Ctrl+",
                    ("ctrl", true) => "⌃",
                    ("shift", true) => "⇧",
                    ("shift", false) => "Shift+",
                    ("alt", true) => "⌥",
                    ("alt", false) => "Alt+",
                    _ => "",
                });
            }
            out + &key
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A key event as the registry writes keys (`secondary-shift-s`); None for a modifier alone
pub fn keys_of_event(ev: &KeyDownEvent) -> Option<String> {
    // keymap-ok: turns a pressed key into the registry's text, for the key capture
    let k = &ev.keystroke;
    let key = k.key.as_str();
    if matches!(key, "shift" | "control" | "alt" | "cmd" | "platform" | "function" | "fn" | "") {
        return None;
    }
    let m = &k.modifiers;
    let mac = cfg!(target_os = "macos");
    let mut s = String::new();
    if (mac && m.platform) || (!mac && m.control) {
        s.push_str("secondary-");
    }
    if mac && m.control {
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

pub struct Palette {
    scx: ScreenCx,
    query: Entity<InputState>,
    selected: usize,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

struct Hit {
    def: &'static ActionDef,
    label: &'static str,
    matched: Vec<usize>,
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
        if q.trim().is_empty() {
            let recent = cx.try_global::<Recent>().map(|r| r.0.clone()).unwrap_or_default();
            let mut order: Vec<&'static ActionDef> =
                recent.iter().filter_map(|id| REGISTRY.iter().find(|d| d.id == *id)).collect();
            order.extend(REGISTRY.iter().filter(|d| !recent.contains(&d.id)));
            return order
                .into_iter()
                .take(MAX_RESULTS)
                .map(|def| Hit { def, label: tr(def.label), matched: Vec::new() })
                .collect();
        }
        let mut scored: Vec<(i32, Hit)> = REGISTRY
            .iter()
            .filter_map(|def| {
                let label = tr(def.label);
                // the label first; the group and the id also find it, with a lower score
                let (score, matched) = fuzzy(&q, label)
                    .or_else(|| fuzzy(&q, tr(def.group)).map(|(s, _)| (s - 30, Vec::new())))
                    .or_else(|| fuzzy(&q, def.id).map(|(s, _)| (s - 40, Vec::new())))?;
                Some((score, Hit { def, label, matched }))
            })
            .collect();
        scored.sort_by_key(|a| std::cmp::Reverse(a.0));
        scored.into_iter().take(MAX_RESULTS).map(|(_, h)| h).collect()
    }

    fn run_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hits = self.hits(cx);
        let Some(h) = hits.get(self.selected) else { return };
        let def = h.def;
        let mut recent = cx.try_global::<Recent>().map(|r| r.0.clone()).unwrap_or_default();
        recent.retain(|id| *id != def.id);
        recent.insert(0, def.id);
        recent.truncate(8);
        cx.set_global(Recent(recent));
        self.scx.close(cx);
        // the same path as the action's keys: its GPUI action, from the focused element
        if let Ok(action) = cx.build_action(def.gpui, None) {
            window.dispatch_action(action, cx);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let n = self.hits(cx).len();
        if n == 0 {
            return;
        }
        // keymap-ok: the palette is a text field with a list, its arrows and Enter are the field's
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

/// The byte ranges of the chars at `pos`, merged when consecutive
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

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hits = self.hits(cx);
        let empty = hits.is_empty();
        let prefs: Value = self.scx.core.read(cx).port().preferences().clone();
        let rows: Vec<_> = hits
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                let keys = keys_of(h.def, &prefs).iter().map(|k| keys_label(k)).collect::<Vec<_>>().join("  ");
                let hl = HighlightStyle {
                    color: Some(c(THEME.green).into()),
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                };
                let ranges: Vec<_> = char_ranges(h.label, &h.matched).into_iter().map(|r| (r, hl)).collect();
                div()
                    .id(("palette-row", i))
                    .debug_selector(move || format!("palette-row-{i}"))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .when(i == self.selected, |d| d.bg(c(THEME.bg4)))
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
                    .child(div().w(px(100.)).flex().justify_end().child(caption(tr(h.def.group))))
                    .child(
                        div()
                            .w(px(110.))
                            .flex()
                            .justify_end()
                            .text_size(px(11.))
                            .text_color(c(THEME.text2))
                            .child(keys),
                    )
            })
            .collect();
        div()
            .id("palette")
            .w(px(600.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            // keymap-ok: text entry, see `on_key`
            .capture_key_down(cx.listener(Self::on_key))
            .child(
                div()
                    .p(px(8.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(crate::ui::text_entry(Input::new(&self.query))),
            )
            .child(
                div()
                    .id("palette-list")
                    .max_h(px(400.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(4.))
                    .children(rows)
                    .when(empty, |d| {
                        d.child(div().p(px(12.)).text_color(c(THEME.text3)).text_size(px(13.)).child(tr("No command")))
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_ignores_accents_and_prefers_word_starts() {
        assert!(fuzzy("configuracoes", "Configurações").is_some());
        assert!(fuzzy("xyz", "Export audio").is_none());
        let (_, p) = fuzzy("exa", "Export audio").unwrap();
        assert_eq!(p, vec![0, 1, 7]);
    }

    #[test]
    fn keys_are_written_for_the_platform() {
        let s = keys_label("secondary-shift-s");
        if cfg!(target_os = "macos") {
            assert_eq!(s, "⌘⇧S")
        } else {
            assert_eq!(s, "Ctrl+Shift+S")
        }
        assert_eq!(keys_label("-"), "-");
        assert_eq!(char_ranges("Exportar áudio", &[0, 9, 10]), vec![0..1, 9..12]);
    }
}
