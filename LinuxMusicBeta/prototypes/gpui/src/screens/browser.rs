//! Owner: agent C. The browser, docked on the left (design 06 is not approved: the layout is rethought from the
//! legacy's features, F-BRW-001..012, F-FX-003, F-FX-036). One search for everything on top (words and "quoted
//! phrases", in the background, a new query cancels the old); the pages as text tabs (Instruments, Effects,
//! Samples, Presets, Projects, Project files, Favourites); the user's and the factory's folders in one tree each
//! (the factory's after a line); a detail pane at the bottom with the preview: the waveform (the shared loading
//! component in its place while the preview is prepared), play and stop, "play to the end", favourite, and the
//! actions (send to the instrument track, add to the effect window's channel, show in the file manager, open).
//!
//! Keys (F-BRW-004/005/006): Cmd/Ctrl+F the search; ↑/↓ walk the list and preview each sample; Space held previews;
//! Enter sends; F5 refreshes. Every row drags (`Dragged`) to the instrument window, the effect chain and the
//! timeline (the timeline's drop is the composition page's: a long audio file makes an audio track, a short one a
//! sampler, F-ARR-011).

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Duration;

use fake_core::Peaks;
use fake_core::browser::{self, Entry, Kind, Page};
use fake_core::plugins::{self, Descriptor, Host, Role};
use fake_core::{Loading, NodeId};
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, Global, KeyDownEvent, KeyUpEvent, MouseButton,
    PathBuilder, Render, SharedString, Subscription, Task, Window, canvas, div, point, prelude::*, px,
};
use gpui_component::Sizable;
use gpui_component::input::{Input, InputEvent, InputState};

use super::effect::{DragChip, EffectFocus};
use super::instrument::{InstrumentFocus, apply_ops, drop_edits, ops, plugin_chip};
use super::{Screen, ScreenCx};
use crate::ui::{THEME, button, c, caption, loading_component, tab, tr, trf};

/// What a row of the browser carries when dragged
#[derive(Clone, Debug, PartialEq)]
pub enum Dragged {
    /// a file or folder of the library
    Entry(u32),
    /// a plugin by its id
    Plugin(&'static str),
}

/// What the browser remembers between openings (F-BRW-001: the page; the open folders, F-BRW-002)
#[derive(Default)]
struct BrowserState {
    page: Option<Page>,
    open: HashSet<u32>,
    favourites: BTreeSet<Fav>,
    hidden: bool,
    to_end: bool,
}
impl Global for BrowserState {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Fav {
    Entry(u32),
    Plugin(&'static str),
}

/// Opens the browser on a page (the instrument window's "Change…" opens the samples)
pub fn show_page(scx: &ScreenCx, page: Page, window: &mut Window, cx: &mut App) {
    cx.default_global::<BrowserState>().page = Some(page);
    scx.open(Screen::Browser, window, cx);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sel {
    Entry(u32),
    Plugin(&'static str),
}

/// A job in the background with the loading rule (I-2): nothing for 200 ms, then the shared loading component
struct Job<T> {
    started: std::time::Instant,
    total: Duration,
    stage: &'static str,
    _task: Task<()>,
    _result: std::marker::PhantomData<T>,
}

/// Waits a job's time, redrawing once at 200 ms so the loading appears exactly then (I-2), and then each frame
async fn wait_with_loading(exec: &gpui::BackgroundExecutor, this: &gpui::WeakEntity<Browser>, cx: &mut gpui::AsyncApp, total: Duration) {
    let first = fake_core::LOADING_THRESHOLD.min(total);
    exec.timer(first).await;
    let _ = this.update(cx, |_, cx| cx.notify());
    exec.timer(total - first).await;
}

impl<T> Job<T> {
    fn loading(&self, now: std::time::Instant) -> Loading {
        let since = now.saturating_duration_since(self.started);
        if since < fake_core::LOADING_THRESHOLD {
            return Loading::Pending;
        }
        let p = (since.as_secs_f32() / self.total.as_secs_f32()).min(0.99);
        Loading::Visible { progress: Some(p), stage: self.stage, cancellable: since >= fake_core::CANCEL_AFTER, since }
    }
}

struct Preview {
    entry: u32,
    peaks: Option<Arc<Peaks>>,
    job: Option<Job<Arc<Peaks>>>,
    /// playing since (the executor's clock), or stopped
    playing: Option<std::time::Instant>,
}

pub struct Browser {
    scx: ScreenCx,
    page: Page,
    search: Entity<InputState>,
    query: String,
    results: Option<Vec<u32>>,
    search_job: Option<Job<Vec<u32>>>,
    rescan: Option<Job<()>>,
    blocked: Vec<&'static str>,
    sel: Option<Sel>,
    preview: Option<Preview>,
    /// a line the actions write ("Shown in the file manager")
    status: Option<SharedString>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl Browser {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Search everything: words, or \"a phrase\"")));
        let sub = cx.subscribe(&search, |b: &mut Browser, input, ev: &InputEvent, cx| match ev {
            InputEvent::Change => {
                let q = input.read(cx).value().to_string();
                b.start_search(q, cx);
            }
            InputEvent::PressEnter { .. } => b.send(cx),
            _ => {}
        });
        let observe = cx.observe(&scx.core, |_, _, cx| cx.notify());
        let page = cx.try_global::<BrowserState>().and_then(|s| s.page).unwrap_or(Page::Samples);
        let watch = cx.observe_global::<BrowserState>(|b: &mut Browser, cx| {
            if let Some(p) = cx.global::<BrowserState>().page {
                if p != b.page {
                    b.page = p;
                    cx.notify();
                }
            }
        });
        Browser {
            scx,
            page,
            search,
            query: String::new(),
            results: None,
            search_job: None,
            rescan: None,
            blocked: Vec::new(),
            sel: None,
            preview: None,
            status: None,
            focus: cx.focus_handle(),
            _subs: vec![sub, observe, watch],
        }
    }

    fn state<'a>(&self, cx: &'a App) -> Option<&'a BrowserState> {
        cx.try_global::<BrowserState>()
    }

    fn set_page(&mut self, p: Page, cx: &mut Context<Self>) {
        self.page = p;
        cx.default_global::<BrowserState>().page = Some(p);
        cx.notify();
    }

    /// The search in the background: a new query drops (cancels) the task of the old one (F-BRW-004)
    fn start_search(&mut self, q: String, cx: &mut Context<Self>) {
        self.query = q.clone();
        let terms = browser::terms(&q);
        if terms.is_empty() {
            self.results = None;
            self.search_job = None;
            cx.notify();
            return;
        }
        let exec = cx.background_executor().clone();
        let started = exec.now();
        let task = cx.spawn(async move |this, cx| {
            wait_with_loading(&exec, &this, cx, Duration::from_millis(browser::SEARCH_MS)).await;
            let found = exec.spawn(async move { browser::search(&terms) }).await;
            let _ = this.update(cx, |b, cx| {
                b.results = Some(found);
                b.search_job = None;
                cx.notify();
            });
        });
        self.search_job = Some(Job { started, total: Duration::from_millis(browser::SEARCH_MS), stage: "Searching", _task: task, _result: Default::default() });
        cx.notify();
    }

    fn rescan(&mut self, cx: &mut Context<Self>) {
        let exec = cx.background_executor().clone();
        let started = exec.now();
        let task = cx.spawn(async move |this, cx| {
            wait_with_loading(&exec, &this, cx, Duration::from_millis(browser::RESCAN_MS)).await;
            let _ = this.update(cx, |b, cx| {
                b.blocked = browser::BLOCKED_BY_SCAN.to_vec();
                b.rescan = None;
                cx.notify();
            });
        });
        self.rescan = Some(Job { started, total: Duration::from_millis(browser::RESCAN_MS), stage: "Scanning the plugins", _task: task, _result: Default::default() });
        cx.notify();
    }

    fn select(&mut self, s: Sel, preview: bool, cx: &mut Context<Self>) {
        self.sel = Some(s);
        self.status = None;
        if let Sel::Entry(id) = s {
            let e = browser::entry(id);
            if e.is_some_and(|e| e.kind.previews()) {
                if preview { self.start_preview(id, cx) } else { self.prepare(id, cx) }
            } else {
                self.preview = None;
            }
        } else {
            self.preview = None;
        }
        cx.notify();
    }

    /// Prepares the waveform (I-7.3: decoding a sample, or building a preset's processor, is an operation)
    fn prepare(&mut self, id: u32, cx: &mut Context<Self>) {
        if self.preview.as_ref().is_some_and(|p| p.entry == id) {
            return;
        }
        let exec = cx.background_executor().clone();
        let started = exec.now();
        let ms = browser::preview_ms(id);
        let task = cx.spawn(async move |this, cx| {
            wait_with_loading(&exec, &this, cx, Duration::from_millis(ms)).await;
            let peaks = exec.spawn(async move { browser::preview_peaks(id) }).await;
            let _ = this.update(cx, |b, cx| {
                if let Some(p) = b.preview.as_mut().filter(|p| p.entry == id) {
                    p.peaks = Some(peaks);
                    p.job = None;
                }
                cx.notify();
            });
        });
        let stage = if browser::entry(id).is_some_and(|e| e.kind == Kind::Audio) { "Loading the sample" } else { "Preparing the preview" };
        self.preview = Some(Preview {
            entry: id,
            peaks: None,
            job: Some(Job { started, total: Duration::from_millis(ms), stage, _task: task, _result: Default::default() }),
            playing: None,
        });
    }

    fn start_preview(&mut self, id: u32, cx: &mut Context<Self>) {
        self.prepare(id, cx);
        let now = cx.background_executor().now();
        if let Some(p) = self.preview.as_mut() {
            p.playing = Some(now);
        }
        cx.notify();
    }

    fn stop_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.preview.as_mut() {
            p.playing = None;
        }
        cx.notify();
    }

    fn toggle_fav(&mut self, f: Fav, cx: &mut Context<Self>) {
        let s = cx.default_global::<BrowserState>();
        if !s.favourites.remove(&f) {
            s.favourites.insert(f);
        }
        cx.notify();
    }

    fn is_fav(&self, f: Fav, cx: &App) -> bool {
        self.state(cx).is_some_and(|s| s.favourites.contains(&f))
    }

    /// The instrument track the actions go to: the instrument window's, or the first instrument track
    fn target_track(&self, cx: &App) -> Option<NodeId> {
        let root = self.scx.core.read(cx).port().root();
        cx.try_global::<InstrumentFocus>()
            .map(|f| f.0)
            .filter(|t| root.tracks.iter().any(|x| x.id == *t))
            .or_else(|| root.tracks.iter().find(|t| root.plugins.instrument_of(t.id).is_some()).map(|t| t.id))
    }

    /// The desk channel the effects go to: the effect window's, or the first channel
    fn target_channel(&self, cx: &App) -> Option<NodeId> {
        let root = self.scx.core.read(cx).port().root();
        cx.try_global::<EffectFocus>().map(|f| f.0).filter(|c| root.channels.iter().any(|x| x.id == *c)).or_else(|| root.channels.first().map(|c| c.id))
    }

    /// Enter / double click / the primary button: to the instrument track (a sample into its sampler, a preset, an
    /// instrument), an effect to the channel, a project opens (F-BRW-006)
    fn send(&mut self, cx: &mut Context<Self>) {
        let Some(sel) = self.sel else { return };
        let core = self.scx.core.clone();
        let d = match sel {
            Sel::Entry(id) => Dragged::Entry(id),
            Sel::Plugin(p) => Dragged::Plugin(p),
        };
        if let Sel::Plugin(p) = sel {
            if plugins::descriptor(p).is_some_and(|d| d.role == Role::Effect) {
                if let Some(ch) = self.target_channel(cx) {
                    let at = core.read(cx).port().root().channels.iter().find(|c| c.id == ch).map_or(0, |c| c.inserts.len());
                    apply_ops(&core, "Add an effect", vec![ops::insert(ch, at, p)], cx);
                    self.status = Some(trf("Added to {0}", &[&self.channel_name(ch, cx)]).into());
                }
                return;
            }
        }
        if let Sel::Entry(id) = sel {
            if browser::entry(id).is_some_and(|e| e.kind == Kind::Project) {
                self.status = Some(tr("Opening the project…").into());
                return;
            }
        }
        let Some(track) = self.target_track(cx) else { return };
        let edits = drop_edits(core.read(cx).port().root(), track, &d);
        match edits {
            Some((label, edits)) => {
                apply_ops(&core, label, edits, cx);
                let name = core.read(cx).port().root().tracks.iter().find(|t| t.id == track).map_or(String::new(), |t| t.name.clone());
                self.status = Some(trf("Sent to {0}", &[&name]).into());
            }
            None => self.status = Some(tr("Drag it to the timeline or a track.").into()),
        }
        cx.notify();
    }

    fn channel_name(&self, ch: NodeId, cx: &App) -> String {
        self.scx.core.read(cx).port().root().channels.iter().find(|c| c.id == ch).map_or(String::new(), |c| c.name.clone())
    }

    /// The rows shown, in order: what the keys walk
    fn visible(&self, cx: &App) -> Vec<Sel> {
        if let Some(r) = &self.results {
            let mut v: Vec<Sel> = r.iter().map(|id| Sel::Entry(*id)).collect();
            let terms = browser::terms(&self.query);
            v.extend(plugins::all().filter(|d| browser::matches(tr(d.id), &terms) || browser::matches(d.id, &terms)).map(|d| Sel::Plugin(d.id)));
            return v;
        }
        match self.page {
            Page::Instruments | Page::Effects => self.plugin_list(cx).into_iter().map(|d| Sel::Plugin(d.id)).collect(),
            Page::Favourites => self.state(cx).map_or(Vec::new(), |s| {
                s.favourites.iter().map(|f| match f {
                    Fav::Entry(e) => Sel::Entry(*e),
                    Fav::Plugin(p) => Sel::Plugin(p),
                }).collect()
            }),
            p => {
                let mut out = Vec::new();
                self.walk(p, None, cx, &mut |e, _| out.push(Sel::Entry(e.id)));
                out
            }
        }
    }

    fn walk(&self, page: Page, parent: Option<u32>, cx: &App, f: &mut dyn FnMut(&'static Entry, usize)) {
        let hidden = self.state(cx).is_some_and(|s| s.hidden);
        let depth = {
            let mut d = 0;
            let mut at = parent;
            while let Some(p) = at {
                d += 1;
                at = browser::entry(p).and_then(|e| e.parent);
            }
            d
        };
        for e in browser::children(page, parent, hidden) {
            f(e, depth);
            if e.is_folder() && self.state(cx).is_some_and(|s| s.open.contains(&e.id)) {
                self.walk(page, Some(e.id), cx, f);
            }
        }
    }

    fn plugin_list(&self, cx: &App) -> Vec<&'static Descriptor> {
        let _ = cx;
        let role = if self.page == Page::Effects { Role::Effect } else { Role::Instrument };
        let mut v: Vec<&'static Descriptor> = plugins::all().filter(|d| d.role == role).collect();
        // the categories, the hosts (third-party) after the internal ones (F-FX-003, F-FX-036)
        v.sort_by_key(|d| (d.host != Host::Internal, d.category, d.id));
        v
    }

    fn step(&mut self, d: i32, cx: &mut Context<Self>) {
        let rows = self.visible(cx);
        if rows.is_empty() {
            return;
        }
        let at = self.sel.and_then(|s| rows.iter().position(|r| *r == s)).map_or(if d > 0 { -1 } else { 0 }, |k| k as i32);
        let next = rows[(at + d).clamp(0, rows.len() as i32 - 1) as usize];
        // ↑/↓ preview the samples as they pass (F-BRW-005)
        let audio = matches!(next, Sel::Entry(id) if browser::entry(id).is_some_and(|e| e.kind == Kind::Audio));
        self.select(next, audio, cx);
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let primary = if cfg!(target_os = "macos") { k.modifiers.platform } else { k.modifiers.control };
        match k.key.as_str() {
            "f" if primary => {
                self.search.update(cx, |i, cx| i.focus(window, cx));
            }
            "down" => self.step(1, cx),
            "up" => self.step(-1, cx),
            "enter" => self.send(cx),
            "space" if !ev.is_held => {
                if let Some(Sel::Entry(id)) = self.sel {
                    self.start_preview(id, cx);
                }
            }
            "f5" => {
                self.results = None;
                let q = self.query.clone();
                self.start_search(q, cx);
            }
            "right" | "left" => {
                if let Some(Sel::Entry(id)) = self.sel {
                    if browser::entry(id).is_some_and(|e| e.is_folder()) {
                        let open = k.key == "right";
                        let s = cx.default_global::<BrowserState>();
                        if open { s.open.insert(id) } else { s.open.remove(&id) };
                        cx.notify();
                    }
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn on_key_up(&mut self, ev: &KeyUpEvent, cx: &mut Context<Self>) {
        // Space previews while held (unless "play to the end")
        if ev.keystroke.key == "space" && !self.state(cx).is_some_and(|s| s.to_end) {
            self.stop_preview(cx);
        }
    }

    // --- drawing ----------------------------------------------------------------------------------------------

    fn kind_color(k: Kind) -> u32 {
        match k {
            Kind::Folder => THEME.yellow,
            Kind::Audio => THEME.green,
            Kind::Midi => THEME.purple,
            Kind::Project => THEME.blue,
            Kind::Preset => THEME.cyan,
            Kind::SoundFont | Kind::Patch => THEME.orange,
            Kind::Nam => THEME.red,
            Kind::Other => THEME.text3,
        }
    }

    fn entry_row(&self, e: &'static Entry, depth: usize, cx: &Context<Self>) -> AnyElement {
        let id = e.id;
        let selected = self.sel == Some(Sel::Entry(id));
        let open = self.state(cx).is_some_and(|s| s.open.contains(&id));
        let fav = self.is_fav(Fav::Entry(id), cx);
        let label: SharedString = e.name.clone().into();
        let meta = match e.kind {
            Kind::Audio => format!("{:.1} s", e.seconds),
            Kind::Folder => String::new(),
            k => tr(k.label()).to_string(),
        };
        div()
            .id(("br-entry", id as usize))
            .debug_selector(move || format!("br-entry-{id}"))
            .h(px(22.))
            .pl(px(8. + 14. * depth as f32))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .bg(c(if selected { THEME.bg4 } else { THEME.bg1 }))
            .hover(|s| s.bg(c(THEME.bg3)))
            .cursor_pointer()
            .on_click(cx.listener(move |b, ev: &gpui::ClickEvent, _, cx| {
                if ev.click_count() == 2 {
                    b.send(cx);
                    return;
                }
                if browser::entry(id).is_some_and(|e| e.is_folder()) {
                    let s = cx.default_global::<BrowserState>();
                    if !s.open.remove(&id) {
                        s.open.insert(id);
                    }
                }
                // a click on a file previews it (F-BRW-005)
                b.select(Sel::Entry(id), true, cx);
            }))
            .on_drag(Dragged::Entry(id), move |_, _, _, cx| cx.new(|_| DragChip(label.clone())))
            .child(div().w(px(10.)).text_size(px(9.)).text_color(c(THEME.text3)).child(if e.is_folder() { if open { "▾" } else { "▸" } } else { "" }))
            .child(div().size(px(8.)).rounded(px(2.)).bg(c(Self::kind_color(e.kind))))
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_size(px(12.)).text_color(c(if e.hidden { THEME.text3 } else { THEME.text })).child(e.name.clone()))
            .when(fav, |d| d.child(div().text_size(px(10.)).text_color(c(THEME.yellow)).child("★")))
            .when_some(e.bpm, |d, b| d.child(div().text_size(px(9.5)).text_color(c(THEME.text3)).child(format!("{b} BPM"))))
            .child(div().text_size(px(9.5)).text_color(c(THEME.text3)).child(meta))
            .into_any_element()
    }

    fn plugin_row(&self, d: &'static Descriptor, cx: &Context<Self>) -> AnyElement {
        let id = d.id;
        let selected = self.sel == Some(Sel::Plugin(id));
        let fav = self.is_fav(Fav::Plugin(id), cx);
        let label: SharedString = tr(id).into();
        div()
            .id(SharedString::from(format!("br-plugin-{id}")))
            .debug_selector(move || format!("br-plugin-{id}"))
            .h(px(22.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .bg(c(if selected { THEME.bg4 } else { THEME.bg1 }))
            .hover(|s| s.bg(c(THEME.bg3)))
            .cursor_pointer()
            .on_click(cx.listener(move |b, ev: &gpui::ClickEvent, _, cx| {
                b.select(Sel::Plugin(id), false, cx);
                if ev.click_count() == 2 {
                    b.send(cx);
                }
            }))
            .on_drag(Dragged::Plugin(id), move |_, _, _, cx| cx.new(|_| DragChip(label.clone())))
            .child(plugin_chip(Some(d)))
            .child(div().flex_1().text_size(px(12.)).child(tr(id)))
            .when(fav, |e| e.child(div().text_size(px(10.)).text_color(c(THEME.yellow)).child("★")))
            .when(d.host != Host::Internal, |e| e.child(div().text_size(px(9.5)).text_color(c(THEME.purple)).child(tr("outside"))))
            .into_any_element()
    }

    fn heading(text: impl Into<SharedString>) -> AnyElement {
        div()
            .px(px(8.))
            .pt(px(6.))
            .pb(px(2.))
            .text_size(px(10.))
            .font_weight(FontWeight::BOLD)
            .text_color(c(THEME.text3))
            .child(text.into())
            .into_any_element()
    }

    fn list(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        if let Some(r) = &self.results {
            let terms = browser::terms(&self.query);
            let plugs: Vec<&'static Descriptor> =
                plugins::all().filter(|d| browser::matches(tr(d.id), &terms) || browser::matches(d.id, &terms)).collect();
            if r.is_empty() && plugs.is_empty() {
                rows.push(div().p(px(10.)).child(caption(tr("Nothing found."))).into_any_element());
            }
            if !plugs.is_empty() {
                rows.push(Self::heading(tr("Plugins")));
                rows.extend(plugs.into_iter().map(|d| self.plugin_row(d, cx)));
            }
            for page in [Page::Samples, Page::Presets, Page::Projects, Page::ProjectFiles] {
                let hits: Vec<&'static Entry> = r.iter().filter_map(|id| browser::entry(*id)).filter(|e| e.page == page).collect();
                if !hits.is_empty() {
                    rows.push(Self::heading(tr(page.title())));
                    rows.extend(hits.into_iter().map(|e| self.entry_row(e, 0, cx)));
                }
            }
            return rows;
        }
        match self.page {
            Page::Instruments | Page::Effects => {
                let mut last = "";
                for d in self.plugin_list(cx) {
                    if d.category != last {
                        rows.push(Self::heading(tr(d.category)));
                        last = d.category;
                    }
                    rows.push(self.plugin_row(d, cx));
                }
                if !self.blocked.is_empty() {
                    rows.push(Self::heading(tr("Blocked: they crashed the scan")));
                    for b in &self.blocked {
                        let b: &'static str = b;
                        rows.push(
                            div()
                                .px(px(8.))
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(div().flex_1().text_size(px(12.)).text_color(c(THEME.text3)).child(b))
                                .child(button(SharedString::from(format!("br-unblock-{b}")), tr("Try again"), false, 0).on_click(cx.listener(move |br, _, _, cx| {
                                    br.blocked.retain(|x| *x != b);
                                    br.rescan(cx);
                                })))
                                .into_any_element(),
                        );
                    }
                }
            }
            Page::Favourites => {
                let favs: Vec<Fav> = self.state(cx).map_or(Vec::new(), |s| s.favourites.iter().copied().collect());
                if favs.is_empty() {
                    rows.push(div().p(px(10.)).child(caption(tr("No favourites yet: ★ in the detail pane adds one."))).into_any_element());
                }
                for f in favs {
                    match f {
                        Fav::Entry(e) => {
                            if let Some(e) = browser::entry(e) {
                                rows.push(self.entry_row(e, 0, cx));
                            }
                        }
                        Fav::Plugin(p) => {
                            if let Some(d) = plugins::descriptor(p) {
                                rows.push(self.plugin_row(d, cx));
                            }
                        }
                    }
                }
            }
            page => {
                let mut factory_line = false;
                let mut items: Vec<(&'static Entry, usize)> = Vec::new();
                self.walk(page, None, cx, &mut |e, d| items.push((e, d)));
                for (e, d) in items {
                    if d == 0 && e.factory && !factory_line {
                        rows.push(Self::heading(tr("Factory files")));
                        factory_line = true;
                    }
                    rows.push(self.entry_row(e, d, cx));
                }
            }
        }
        rows
    }

    fn detail(&self, cx: &Context<Self>) -> AnyElement {
        let now = cx.background_executor().now();
        let to_end = self.state(cx).is_some_and(|s| s.to_end);
        let body: AnyElement = match self.sel {
            None => caption(tr("Select a file or a plugin to see it here; a click previews it.")).into_any_element(),
            Some(Sel::Plugin(p)) => {
                let Some(d) = plugins::descriptor(p) else { return div().into_any_element() };
                let fav = self.is_fav(Fav::Plugin(p), cx);
                let target: String = if d.role == Role::Effect {
                    self.target_channel(cx).map_or(String::new(), |c| self.channel_name(c, cx))
                } else {
                    let root = self.scx.core.read(cx).port().root();
                    self.target_track(cx).and_then(|t| root.tracks.iter().find(|x| x.id == t)).map_or(String::new(), |t| t.name.clone())
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().flex().items_center().gap(px(6.)).child(plugin_chip(Some(d))).child(div().text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr(p))))
                    .child(caption(format!("{} · {}", tr(d.category), if d.host == Host::Internal { tr("internal") } else { tr("third-party, runs outside the program") })))
                    .child(div().text_size(px(11.)).text_color(c(THEME.text2)).child(tr(d.description)))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.))
                            .child(
                                button("br-send", if d.role == Role::Effect { trf("Add to {0}", &[&target]) } else { trf("Use on {0}", &[&target]) }, false, 0)
                                    .h(px(22.))
                                    .px(px(8.))
                                    .debug_selector(|| "br-send".into())
                                    .on_click(cx.listener(|b, _, _, cx| b.send(cx))),
                            )
                            .child(button("br-fav", if fav { "★" } else { "☆" }, fav, THEME.yellow).h(px(22.)).on_click(cx.listener(move |b, _, _, cx| b.toggle_fav(Fav::Plugin(p), cx)))),
                    )
                    .into_any_element()
            }
            Some(Sel::Entry(id)) => {
                let Some(e) = browser::entry(id) else { return div().into_any_element() };
                let fav = self.is_fav(Fav::Entry(id), cx);
                let mut col = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::BOLD).overflow_hidden().whitespace_nowrap().child(e.name.clone()))
                    .child(caption(format!("{} · {}{}", tr(e.kind.label()), e.format(), e.bpm.map_or(String::new(), |b| format!(" · {b} BPM")))))
                    .child(div().text_size(px(9.5)).text_color(c(THEME.text3)).overflow_hidden().whitespace_nowrap().child(e.path.clone()));
                if e.kind.previews() {
                    col = col.child(self.wave(id, now, cx));
                }
                let long = e.kind == Kind::Audio && e.seconds > browser::LONG_AUDIO_SECONDS;
                let playing = self.preview.as_ref().is_some_and(|p| p.entry == id && p.playing.is_some());
                let mut actions = div().flex().flex_wrap().gap(px(6.));
                if e.kind.previews() {
                    actions = actions
                        .child(
                            button("br-play", if playing { "■" } else { "▶" }, playing, THEME.green)
                                .h(px(22.))
                                .w(px(30.))
                                .debug_selector(|| "br-play".into())
                                .on_click(cx.listener(move |b, _, _, cx| if playing { b.stop_preview(cx) } else { b.start_preview(id, cx) })),
                        )
                        .child(button("br-to-end", tr("To the end"), to_end, THEME.green).h(px(22.)).on_click(cx.listener(|_, _, _, cx| {
                            let s = cx.default_global::<BrowserState>();
                            s.to_end = !s.to_end;
                            cx.notify();
                        })));
                }
                let send_label = match e.kind {
                    Kind::Project => Some(tr("Open")),
                    Kind::Audio if long => None,
                    Kind::Audio => Some(tr("To the sampler")),
                    Kind::Preset | Kind::SoundFont => Some(tr("To the instrument")),
                    _ => None,
                };
                if let Some(l) = send_label {
                    actions = actions.child(button("br-send", l, false, 0).h(px(22.)).px(px(8.)).debug_selector(|| "br-send".into()).on_click(cx.listener(|b, _, _, cx| b.send(cx))));
                }
                let path = e.path.clone();
                actions = actions
                    .child(button("br-reveal", tr("Show in the file manager"), false, 0).h(px(22.)).px(px(8.)).on_click(cx.listener(move |b, _, _, cx| {
                        b.status = Some(trf("Shown in the file manager: {0}", &[&path]).into());
                        cx.notify();
                    })))
                    .child(button("br-fav", if fav { "★" } else { "☆" }, fav, THEME.yellow).h(px(22.)).debug_selector(|| "br-fav".into()).on_click(cx.listener(move |b, _, _, cx| b.toggle_fav(Fav::Entry(id), cx))));
                col = col.child(actions);
                if long {
                    col = col.child(caption(tr("A long recording: drag it to the timeline for an audio track.")));
                }
                col.into_any_element()
            }
        };
        div()
            .id("br-detail")
            .debug_selector(|| "br-detail".into())
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .bg(c(THEME.bg2))
            .border_t_1()
            .border_color(c(THEME.line))
            .child(body)
            .when_some(self.status.clone(), |d, s| d.child(div().text_size(px(10.5)).text_color(c(THEME.green)).child(s)))
            .into_any_element()
    }

    /// The preview's waveform: one filled path (GPUI's cost is per primitive), the play position over it, and the
    /// shared loading component in its place while it is prepared (I-2)
    fn wave(&self, id: u32, now: std::time::Instant, cx: &Context<Self>) -> AnyElement {
        let p = self.preview.as_ref().filter(|p| p.entry == id);
        let frame = div().id("br-wave").debug_selector(|| "br-wave".into()).h(px(48.)).rounded(px(4.)).bg(c(THEME.lcd_bg)).overflow_hidden();
        if let Some(job) = p.and_then(|p| p.job.as_ref()) {
            let loading = job.loading(now);
            let entity = cx.entity().downgrade();
            return match loading_component("br-loading", &loading, move |_, cx| {
                let _ = entity.update(cx, |b, cx| {
                    b.preview = None;
                    cx.notify();
                });
            }) {
                Some(l) => frame.debug_selector(|| "br-loading".into()).child(l).into_any_element(),
                None => frame.into_any_element(),
            };
        }
        let Some(peaks) = p.and_then(|p| p.peaks.clone()) else { return frame.into_any_element() };
        let length = peaks.length as f64;
        let pos = p.and_then(|p| p.playing).map(|t| now.saturating_duration_since(t).as_secs_f64() * fake_core::SAMPLE_RATE as f64 / length.max(1.0));
        frame
            .child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        let n = b.size.width.as_f32().max(1.0) as usize;
                        let mut cols = Vec::with_capacity(n);
                        peaks.columns(0.0, length / n as f64, n, &mut cols);
                        let mid = b.origin.y.as_f32() + b.size.height.as_f32() / 2.0;
                        let h = b.size.height.as_f32() / 2.0 - 2.0;
                        let mut pb = PathBuilder::fill();
                        let x = |k: usize| px(b.origin.x.as_f32() + k as f32);
                        pb.move_to(point(x(0), px(mid)));
                        for (k, col) in cols.iter().enumerate() {
                            let hi = col.map_or(0.0, |(_, hi)| hi as f32 / i16::MAX as f32);
                            pb.line_to(point(x(k), px(mid - hi * h)));
                        }
                        for (k, col) in cols.iter().enumerate().rev() {
                            let lo = col.map_or(0.0, |(lo, _)| lo as f32 / i16::MAX as f32);
                            pb.line_to(point(x(k), px(mid - lo * h)));
                        }
                        pb.close();
                        if let Ok(path) = pb.build() {
                            window.paint_path(path, c(THEME.green));
                        }
                        if let Some(f) = pos.filter(|f| *f <= 1.0) {
                            let px_ = b.origin.x + b.size.width * f as f32;
                            window.paint_quad(gpui::fill(gpui::Bounds::new(point(px_, b.origin.y), gpui::size(px(1.5), b.size.height)), c(THEME.cursor)));
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = cx.background_executor().now();
        // the preview ends with the file (a preset's note after 2 s); while it plays, a frame per display refresh
        if let Some(p) = self.preview.as_mut() {
            if let (Some(t), Some(peaks)) = (p.playing, p.peaks.as_ref()) {
                let secs = peaks.length as f64 / fake_core::SAMPLE_RATE as f64;
                if now.saturating_duration_since(t).as_secs_f64() > secs {
                    p.playing = None;
                } else {
                    window.request_animation_frame();
                }
            }
            if p.job.is_some() {
                window.request_animation_frame();
            }
        }
        if self.search_job.is_some() || self.rescan.is_some() {
            window.request_animation_frame();
        }
        let hidden = self.state(cx).is_some_and(|s| s.hidden);
        let searching = self.search_job.as_ref().map(|j| j.loading(now));
        let rescanning = self.rescan.as_ref().map(|j| j.loading(now));
        let plugin_page = matches!(self.page, Page::Instruments | Page::Effects);
        let rows = self.list(cx);
        div()
            .id("browser")
            .debug_selector(|| "browser".into())
            .key_context("Browser")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .on_key_down(cx.listener(|b, ev: &KeyDownEvent, window, cx| b.on_key(ev, window, cx)))
            .on_key_up(cx.listener(|b, ev: &KeyUpEvent, _, cx| b.on_key_up(ev, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|b, _, window, cx| {
                if !b.search.read(cx).focus_handle(cx).is_focused(window) {
                    window.focus(&b.focus, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .p(px(8.))
                    .bg(c(THEME.bg2))
                    .child(div().id("br-search").debug_selector(|| "br-search".into()).flex_1().child(Input::new(&self.search).small().cleanable(true)))
                    .child(button("br-close", "×", false, 0).on_click(cx.listener(|b, _, _, cx| b.scx.close(cx)))),
            )
            .child(
                div().flex().flex_wrap().px(px(4.)).bg(c(THEME.bg2)).border_b_1().border_color(c(THEME.line)).children(Page::ALL.into_iter().map(|p| {
                    tab(SharedString::from(format!("br-tab-{p:?}")), tr(p.title()), self.page == p && self.results.is_none())
                        .debug_selector(move || format!("br-tab-{p:?}"))
                        .px(px(7.))
                        .text_size(px(11.5))
                        .on_click(cx.listener(move |b, _, window, cx| {
                            b.search.update(cx, |i, cx| i.set_value("", window, cx));
                            b.results = None;
                            b.search_job = None;
                            b.set_page(p, cx);
                        }))
                })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(4.))
                    .when(!plugin_page, |d| {
                        d.child(button("br-hidden", tr("Hidden files"), hidden, THEME.blue).on_click(cx.listener(|_, _, _, cx| {
                            let s = cx.default_global::<BrowserState>();
                            s.hidden = !s.hidden;
                            cx.notify();
                        })))
                    })
                    .when(plugin_page, |d| d.child(button("br-rescan", tr("Rescan the plugins"), false, 0).debug_selector(|| "br-rescan".into()).on_click(cx.listener(|b, _, _, cx| b.rescan(cx)))))
                    .child(div().flex_1())
                    .when(self.results.is_some(), |d| d.child(caption(trf("{0} found", &[&self.visible(cx).len()])))),
            )
            .children(searching.and_then(|l| loading_component("br-searching", &l, |_, _| {})).map(|l| div().h(px(28.)).debug_selector(|| "br-searching".into()).child(l)))
            .children(rescanning.and_then(|l| loading_component("br-rescanning", &l, |_, _| {})).map(|l| div().h(px(28.)).child(l)))
            .child(div().id("br-list").flex_1().overflow_y_scroll().py(px(2.)).children(rows))
            .child(self.detail(cx))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui::TestAppContext;

    use super::*;
    use crate::screens::instrument::tests::{click, open};

    /// A folder opens, a sample previews (the loading past 200 ms in place of the waveform, then the waveform),
    /// ★ makes a favourite, and "To the sampler" loads it into the instrument track's sampler (one undo step)
    #[gpui::test]
    fn browse_preview_favourite_and_send(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Screen::Browser);
        click(cx, "br-tab-Samples");
        let drums = browser::children(Page::Samples, None, false).into_iter().find(|e| e.name == "Baterias").unwrap().id;
        click(cx, &format!("br-entry-{drums}"));
        let kick = browser::children(Page::Samples, Some(drums), false).into_iter().find(|e| e.name.starts_with("kick")).unwrap().id;
        click(cx, &format!("br-entry-{kick}"));
        cx.executor().advance_clock(Duration::from_millis(250));
        cx.run_until_parked();
        assert!(cx.debug_bounds("br-wave").is_some(), "the waveform's place is drawn");
        cx.executor().advance_clock(Duration::from_millis(2000));
        cx.run_until_parked();
        assert!(cx.debug_bounds("br-loading").is_none(), "prepared");
        click(cx, "br-fav");
        click(cx, "br-tab-Favourites");
        assert!(cx.debug_bounds(Box::leak(format!("br-entry-{kick}").into_boxed_str())).is_some(), "in the favourites");
        click(cx, &format!("br-entry-{kick}"));
        click(cx, "br-send");
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        let ok = core.read_with(cx, |core, _| {
            let root = core.port().root();
            root.plugins.nodes.values().any(|n| n.plugin == "AudioFileProcessor" && n.resource.as_deref() == browser::entry(kick).map(|e| e.path.as_str()))
        });
        assert!(ok, "the sample went to a sampler");
    }

    /// The search runs in the background: nothing before 200 ms, then the loading, then the results grouped
    #[gpui::test]
    fn search_with_phrases(cx: &mut TestAppContext) {
        let (_ws, cx) = open(cx, Screen::Browser);
        click(cx, "br-search");
        cx.simulate_input(r#""tr909 snare""#);
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert!(cx.debug_bounds("br-searching").is_none(), "nothing before 200 ms");
        cx.executor().advance_clock(Duration::from_millis(150));
        cx.run_until_parked();
        assert!(cx.debug_bounds("br-searching").is_some(), "the loading past 200 ms");
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
        let found = browser::search(&browser::terms(r#""tr909 snare""#));
        assert_eq!(found.len(), 1);
        assert!(cx.debug_bounds(Box::leak(format!("br-entry-{}", found[0]).into_boxed_str())).is_some(), "the result is listed");
        let other = browser::search(&browser::terms("tr909 kick"))[0];
        assert!(cx.debug_bounds(Box::leak(format!("br-entry-{other}").into_boxed_str())).is_none(), "the phrase is kept whole");
    }
}
