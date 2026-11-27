//! Owner: agent C. The browser, as the legacy's (F-BRW-001..012; design 06 is not approved, so the layout, the
//! density and the look follow the legacy `FileBrowser`/`PluginBrowser`, changed only where the new model forces):
//! the pages, the page's title, one search field, and a dense tree with the user's folders and then the factory's
//! after a "Factory files" line; a click previews (the loading line at the bottom while it is prepared), a double
//! click or Enter sends the entry to the active instrument track or desk channel, the right button opens the entry's
//! menu, every row drags to the windows and the timeline. Keys of the registry (context "Browser"): ↑/↓ walk and
//! preview the samples, Enter sends, →/← open and close a folder, F5 reads again, Cmd/Ctrl+F the search; Space held
//! previews. Everything goes through the browser service of the port (the files are the core's, I-15).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, Global, KeyDownEvent, KeyUpEvent,
    MouseButton, MouseDownEvent, Pixels, Point, Render, SharedString, Subscription, Window, anchored, deferred, div,
    prelude::*, px,
};
use gpui_component::Sizable;
use gpui_component::input::{Input, InputEvent, InputState};
use lm_port::{BrowseEntry, BrowsePage, EditOp, EntryId, EntryKind, Pending, Ref, RequestId, ScanKind, Target, Value};

use super::ScreenCx;
use super::instrument::{apply_ops, cancel_target, run_action};
use crate::actions::{
    BrowserCloseFolder, BrowserDown, BrowserFind, BrowserOpenFolder, BrowserRefresh, BrowserSend, BrowserUp,
};
use crate::model;
use crate::ui::{THEME, c, loading_component, tr, trf};

/// What a row carries when dragged
#[derive(Clone, Debug, PartialEq)]
pub enum Dragged {
    Entry(BrowseEntry),
    /// a plugin, by its id (the Instruments and Effects pages)
    Plugin(Arc<str>),
}

/// The instrument track the browser sends to: the instrument window's, set by it (the legacy's "active instrument
/// track")
pub struct ActiveTrack(pub lm_port::NodeId);
impl Global for ActiveTrack {}

/// The desk channel the browser adds effects to: the effect window's, set by it
pub struct ActiveChannel(pub lm_port::NodeId);
impl Global for ActiveChannel {}

/// What the browser remembers between openings: the page, the open folders, the hidden files (F-BRW-001/002)
#[derive(Default)]
struct BrowserState {
    page: Option<BrowsePage>,
    open: HashSet<EntryId>,
    hidden: bool,
    /// The index view (search over everything) instead of the folders
    index: bool,
    /// The type filters of the index view (`type:` words) and the favourites
    filters: Vec<&'static str>,
    fav: bool,
}
impl Global for BrowserState {}

const PAGES: [BrowsePage; 7] = [
    BrowsePage::Instruments,
    BrowsePage::Effects,
    BrowsePage::Samples,
    BrowsePage::Presets,
    BrowsePage::Projects,
    BrowsePage::ProjectFiles,
    BrowsePage::Favorites,
];

fn page_title(p: BrowsePage) -> &'static str {
    match p {
        BrowsePage::Instruments => "Instruments",
        BrowsePage::Effects => "Effects",
        BrowsePage::Samples => "Samples",
        BrowsePage::Presets => "Presets",
        BrowsePage::Projects => "Projects",
        BrowsePage::ProjectFiles => "Project files",
        BrowsePage::Favorites => "Favourites",
    }
}

/// The legacy's file icons by kind (FileBrowser's): an open or closed folder, a sample, a MIDI file, a preset, a
/// project, a plugin
fn kind_icon_of(k: EntryKind, open: bool, ty: Option<&str>) -> &'static str {
    match (k, ty) {
        (EntryKind::Other, Some("soundfont" | "sfz" | "gig" | "patch")) => return "soundfont_file",
        (EntryKind::Other, Some("drumsynth" | "voc" | "nam")) => return "sample_file",
        _ => {}
    }
    match k {
        EntryKind::Folder if open => "folder_opened",
        EntryKind::Folder => "folder",
        EntryKind::Audio => "sample_file",
        EntryKind::Midi => "midi_file",
        EntryKind::Preset => "preset_file",
        EntryKind::Project => "project_file",
        EntryKind::Plugin => "plugins",
        EntryKind::Other => "dont_know",
    }
}

const ROW_H: f32 = 18.0;

/// The type filters of the index view: the word the search takes, and the chip's label
const FILTERS: [(&str, &str); 6] = [
    ("instruments", "Instruments"),
    ("effects", "Effects"),
    ("samples", "Samples"),
    ("presets", "Presets"),
    ("midi", "MIDI"),
    ("projects", "Projects"),
];

/// The text the core's search gets: the filters, the favourites and what was typed
pub fn compose_query(filters: &[&str], fav: bool, typed: &str) -> String {
    let mut q = String::new();
    for f in filters {
        q.push_str("type:");
        q.push_str(f);
        q.push(' ');
    }
    if fav {
        q.push_str("is:fav ");
    }
    q.push_str(typed.trim());
    q.trim().to_owned()
}

/// The waveform thumbnail's bars: `width` columns of (min, max) from the peak pairs, each in -1..1
pub fn thumb_columns(pairs: &[(i16, i16)], width: usize) -> Vec<(f32, f32)> {
    if pairs.is_empty() || width == 0 {
        return Vec::new();
    }
    (0..width)
        .map(|x| {
            let a = x * pairs.len() / width;
            let b = (((x + 1) * pairs.len()).div_ceil(width)).clamp(a + 1, pairs.len());
            let (lo, hi) = pairs[a..b].iter().fold((i16::MAX, i16::MIN), |(l, h), p| (l.min(p.0), h.max(p.1)));
            (f32::from(lo) / 32768.0, f32::from(hi) / 32768.0)
        })
        .collect()
}

pub struct Browser {
    scx: ScreenCx,
    page: BrowsePage,
    search: Entity<InputState>,
    query: String,
    results: Option<Vec<BrowseEntry>>,
    /// the children read so far, by folder (None: the page's top); a `Waiting` answer is asked again next frame
    children: HashMap<Option<EntryId>, Vec<BrowseEntry>>,
    sel: Option<EntryId>,
    /// the preview being prepared or playing
    preview: Option<(BrowseEntry, Option<lm_port::Preview>)>,
    /// the preview being prepared, by request
    preview_req: Option<RequestId>,
    /// listings and the search that wait for the core
    waiting: HashMap<Option<EntryId>, RequestId>,
    search_req: Option<RequestId>,
    /// the waveform of the preview: its pairs, for the thumbnail
    thumb: Option<(EntryId, Arc<[(i16, i16)]>)>,
    menu: Option<(BrowseEntry, Point<Pixels>)>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl Browser {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Search")));
        let sub = cx.subscribe(&search, |b: &mut Browser, input, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                let q = input.read(cx).value().to_string();
                b.start_search(q, cx);
            }
        });
        let observe = cx.observe(&scx.core, |_, _, cx| cx.notify());
        let page = cx.try_global::<BrowserState>().and_then(|s| s.page).unwrap_or(BrowsePage::Samples);
        Browser {
            scx,
            page,
            search,
            query: String::new(),
            results: None,
            children: HashMap::new(),
            sel: None,
            preview: None,
            preview_req: None,
            waiting: HashMap::new(),
            search_req: None,
            thumb: None,
            menu: None,
            focus: cx.focus_handle(),
            _subs: vec![sub, observe],
        }
    }

    fn set_page(&mut self, p: BrowsePage, cx: &mut Context<Self>) {
        self.page = p;
        self.children.clear();
        self.results = None;
        cx.default_global::<BrowserState>().page = Some(p);
        cx.notify();
    }

    /// A folder's children (or the page's top): asked to the port; a `Waiting` answer is taken in `poll`
    fn children_of(&mut self, parent: Option<EntryId>, cx: &mut Context<Self>) -> Vec<BrowseEntry> {
        if let Some(v) = self.children.get(&parent) {
            return v.clone();
        }
        if self.waiting.contains_key(&parent) {
            return Vec::new();
        }
        let (page, hidden) = (self.page, cx.try_global::<BrowserState>().is_some_and(|s| s.hidden));
        match self.scx.core.update(cx, |core, _| core.browse(page, parent, hidden)) {
            Pending::Ready(Ok(v)) => {
                self.children.insert(parent, v.clone());
                v
            }
            Pending::Waiting(id) => {
                self.waiting.insert(parent, id);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// The answers that came from the core: listings, the search, the preview
    fn poll(&mut self, cx: &mut Context<Self>) {
        let core = self.scx.core.clone();
        let ids: Vec<(Option<EntryId>, RequestId)> = self.waiting.iter().map(|(k, v)| (*k, *v)).collect();
        for (parent, id) in ids {
            if let Some(answer) = core.update(cx, |c, _| c.take_entries(id)) {
                self.waiting.remove(&parent);
                self.children.insert(parent, answer.unwrap_or_default());
            }
        }
        if let Some(id) = self.search_req
            && let Some(answer) = core.update(cx, |c, _| c.take_entries(id))
        {
            self.search_req = None;
            self.results = Some(answer.unwrap_or_default());
        }
        if let Some(id) = self.preview_req
            && let Some(answer) = core.update(cx, |c, _| c.take_preview(id))
        {
            self.preview_req = None;
            if let Ok(p) = answer {
                let entry = p.entry;
                self.thumb = self.thumbnail(&p, cx).map(|t| (entry, t));
                core.update(cx, |c, _| c.port_mut().preview_play(entry, 0));
                if let Some((_, slot)) = self.preview.as_mut() {
                    *slot = Some(p);
                }
            } else {
                self.preview = None;
            }
        }
    }

    /// The waveform of a prepared preview, from the peaks service: the first level that fits a thumbnail
    fn thumbnail(&self, p: &lm_port::Preview, cx: &mut Context<Self>) -> Option<Arc<[(i16, i16)]>> {
        const WANT: u64 = 600;
        self.scx.core.update(cx, |c, _| {
            for level in 0..=12u8 {
                let head = c.port_mut().peaks(&p.resource, level, 0, 1).ok()?;
                let pairs = head.length / head.samples_per_pair.max(1) + 1;
                if pairs <= WANT || level == 12 {
                    return c.port_mut().peaks(&p.resource, level, 0, pairs).ok().map(|b| b.pairs);
                }
            }
            None
        })
    }

    /// The search, in the background on the core: a new one replaces the last; its loading is on Scan(Library)
    fn start_search(&mut self, q: String, cx: &mut Context<Self>) {
        self.query = q;
        let (index, filters, fav) =
            cx.try_global::<BrowserState>().map(|s| (s.index, s.filters.clone(), s.fav)).unwrap_or_default();
        let composed = compose_query(&filters, fav, &self.query);
        // The folders view searches only when something is typed; the index view also with the filters alone
        // a new search replaces the last: the old one's answer is no longer waited for
        if let Some(old) = self.search_req.take() {
            self.scx.core.update(cx, |core, _| core.forget_request(old));
        }
        if composed.is_empty() || (!index && self.query.trim().is_empty()) {
            self.results = None;
        } else {
            match self.scx.core.update(cx, |core, _| core.search(&composed)) {
                Pending::Ready(Ok(v)) => self.results = Some(v),
                Pending::Waiting(id) => self.search_req = Some(id),
                _ => {}
            }
        }
        cx.notify();
    }

    /// The rows shown, in order (what the keys walk), with their depth
    fn rows(&mut self, cx: &mut Context<Self>) -> Vec<(BrowseEntry, usize)> {
        if let Some(r) = &self.results {
            return r.iter().map(|e| (e.clone(), 0)).collect();
        }
        if cx.try_global::<BrowserState>().is_some_and(|s| s.index) {
            return Vec::new();
        }
        let mut out = Vec::new();
        self.walk(None, 0, &mut out, cx);
        out
    }

    fn walk(
        &mut self,
        parent: Option<EntryId>,
        depth: usize,
        out: &mut Vec<(BrowseEntry, usize)>,
        cx: &mut Context<Self>,
    ) {
        for e in self.children_of(parent, cx) {
            let open =
                e.kind == EntryKind::Folder && cx.try_global::<BrowserState>().is_some_and(|s| s.open.contains(&e.id));
            let id = e.id;
            out.push((e, depth));
            if open {
                self.walk(Some(id), depth + 1, out, cx);
            }
        }
    }

    fn toggle_folder(&mut self, id: EntryId, open: Option<bool>, cx: &mut Context<Self>) {
        let s = cx.default_global::<BrowserState>();
        let now = s.open.contains(&id);
        if open.unwrap_or(!now) {
            s.open.insert(id);
        } else {
            s.open.remove(&id);
        }
        cx.notify();
    }

    /// A click on a file previews it (F-BRW-005): samples, presets and plugins of instruments
    fn select(&mut self, e: &BrowseEntry, preview: bool, cx: &mut Context<Self>) {
        self.sel = Some(e.id);
        self.menu = None;
        if e.kind == EntryKind::Folder {
            self.toggle_folder(e.id, None, cx);
            return;
        }
        if preview && matches!(e.kind, EntryKind::Audio | EntryKind::Preset) {
            self.start_preview(e.clone(), cx);
        }
        cx.notify();
    }

    fn start_preview(&mut self, e: BrowseEntry, cx: &mut Context<Self>) {
        let id = e.id;
        self.thumb = None;
        if let Some(old) = self.preview_req.take() {
            self.scx.core.update(cx, |core, _| core.forget_request(old));
        }
        let r = self.scx.core.update(cx, |core, _| core.prepare_preview(id));
        match r {
            Pending::Ready(Ok(p)) => {
                self.thumb = self.thumbnail(&p, cx).map(|t| (id, t));
                self.scx.core.update(cx, |core, _| core.port_mut().preview_play(id, 0));
                self.preview = Some((e, Some(p)));
            }
            // asked: `poll` plays it when it is prepared (the loading line shows meanwhile)
            Pending::Waiting(req) => {
                self.preview_req = Some(req);
                self.preview = Some((e, None));
            }
            _ => self.preview = None,
        }
        cx.notify();
    }

    fn stop_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(old) = self.preview_req.take() {
            self.scx.core.update(cx, |core, _| core.forget_request(old));
        }
        if self.preview.take().is_some() {
            self.scx.core.update(cx, |core, _| core.port_mut().preview_stop());
            cx.notify();
        }
    }

    fn step(&mut self, d: i32, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        if rows.is_empty() {
            return;
        }
        let at = self
            .sel
            .and_then(|s| rows.iter().position(|(e, _)| e.id == s))
            .map_or(if d > 0 { -1 } else { 0 }, |k| k as i32);
        let (e, _) = rows[(at + d).clamp(0, rows.len() as i32 - 1) as usize].clone();
        self.sel = Some(e.id);
        // ↑/↓ preview the samples as they pass (F-BRW-005)
        if e.kind == EntryKind::Audio {
            self.start_preview(e, cx);
        }
        cx.notify();
    }

    fn selected(&mut self, cx: &mut Context<Self>) -> Option<BrowseEntry> {
        let s = self.sel?;
        self.rows(cx).into_iter().map(|(e, _)| e).find(|e| e.id == s)
    }

    /// The active instrument track: the instrument window's, else the first with an instrument
    fn active_track(&self, cx: &App) -> Option<(lm_port::NodeId, Option<Instrument>)> {
        let tracks = model::tracks(self.scx.core.read(cx).port().root());
        let want = cx.try_global::<ActiveTrack>().map(|a| a.0);
        tracks
            .iter()
            .find(|t| Some(t.id) == want)
            .or_else(|| tracks.iter().find(|t| t.instrument.is_some()))
            .map(|t| (t.id, t.instrument.clone()))
    }

    fn active_channel(&self, cx: &App) -> Option<model::ChannelView> {
        let chs = model::channels(self.scx.core.read(cx).port().root());
        let want = cx.try_global::<ActiveChannel>().map(|a| a.0);
        chs.iter().find(|c| Some(c.id) == want).or(chs.first()).cloned()
    }

    /// Double click / Enter (F-BRW-006): an instrument swaps the active track's instrument; an effect goes to the end
    /// of the active channel's inserts; a preset loads on its plugin; a project opens
    fn send(&mut self, e: &BrowseEntry, cx: &mut Context<Self>) {
        let core = self.scx.core.clone();
        let plugin = e.info.get("plugin").and_then(Value::as_shared_text).cloned();
        let plugins = core.read(cx).port().plugins();
        match e.kind {
            EntryKind::Plugin => {
                let Some(p) = plugin.and_then(|id| plugins.into_iter().find(|x| x.id == id)) else { return };
                if p.instrument {
                    if let Some((track, _)) = self.active_track(cx) {
                        let node =
                            Value::object([("plugin", Value::text(p.id.clone())), ("state", Value::empty_object())]);
                        apply_ops(&core, "Swap the instrument", vec![EditOp::set(track, &["instrument"], node)], cx);
                    }
                } else if let Some(ch) = self.active_channel(cx) {
                    let node = Value::object([("plugin", Value::text(p.id.clone())), ("state", Value::empty_object())]);
                    let op = EditOp::Insert {
                        temp: 1,
                        parent: Ref::Id(ch.id),
                        list: "inserts".into(),
                        index: ch.inserts.len(),
                        node,
                    };
                    apply_ops(&core, "Add an effect", vec![op], cx);
                }
            }
            EntryKind::Preset => {
                let Some(plugin) = plugin else { return };
                let root = core.read(cx).port().root().clone();
                // the instrument of the active track, or the first node of that plugin
                let node = self
                    .active_track(cx)
                    .and_then(|(_, i)| i)
                    .filter(|(_, p)| *p == plugin)
                    .map(|(n, _)| n)
                    .or_else(|| {
                        model::tracks(&root)
                            .into_iter()
                            .filter_map(|t| t.instrument)
                            .find(|(_, p)| *p == plugin)
                            .map(|(n, _)| n)
                    })
                    .or_else(|| {
                        model::channels(&root)
                            .into_iter()
                            .flat_map(|c| c.inserts)
                            .find(|x| x.plugin == plugin)
                            .map(|x| x.id)
                    });
                if let Some(node) = node {
                    let params =
                        Value::object([("node", Value::text(node.to_string())), ("entry", Value::int(e.id.0 as i64))]);
                    run_action(&core, "plugin.load_preset", params, cx);
                }
            }
            EntryKind::Project => run_action(&core, "project.open_entry", Value::int(e.id.0 as i64), cx),
            // a sample or a MIDI file: a new track at the playhead (F-BRW-006)
            EntryKind::Audio | EntryKind::Midi => {
                if let Some(path) = e.info.get("path").and_then(Value::as_text) {
                    let tick = super::browser_drop::playhead_tick(&core, cx);
                    super::files::import_paths(&core, &[std::path::PathBuf::from(path)], tick, cx);
                }
            }
            // a bank: the instrument that opens it (the one table of the core, `info.opened_by`) on the active track
            EntryKind::Other => {
                let Some(id) = e.info.get("opened_by").and_then(Value::as_shared_text).cloned() else { return };
                if plugins.iter().any(|x| x.id == id && x.instrument)
                    && let Some((track, _)) = self.active_track(cx)
                {
                    let node = Value::object([("plugin", Value::text(id)), ("state", Value::empty_object())]);
                    apply_ops(&core, "Swap the instrument", vec![EditOp::set(track, &["instrument"], node)], cx);
                }
            }
            EntryKind::Folder => {}
        }
    }

    /// The legacy's pages, as text tabs
    fn page_tabs(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .px(px(2.))
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .children(PAGES.into_iter().map(|p| {
                let active = self.page == p;
                div()
                    .id(SharedString::from(format!("br-tab-{p:?}")))
                    .debug_selector(move || format!("br-tab-{p:?}"))
                    .px(px(6.))
                    .py(px(3.))
                    .text_size(px(11.))
                    .text_color(c(if active { THEME.text } else { THEME.text2 }))
                    .when(active, |d| d.bg(c(THEME.bg4)))
                    .cursor_pointer()
                    .child(tr(page_title(p)))
                    .on_click(cx.listener(move |b, _, window, cx| {
                        b.search.update(cx, |i, cx| i.set_value("", window, cx));
                        b.set_page(p, cx);
                    }))
            }))
            .into_any_element()
    }

    /// The type filters and the favourites of the index view
    fn chips(&self, cx: &Context<Self>) -> AnyElement {
        let (filters, fav) = cx.try_global::<BrowserState>().map(|s| (s.filters.clone(), s.fav)).unwrap_or_default();
        div()
            .flex()
            .flex_wrap()
            .px(px(2.))
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .children(FILTERS.into_iter().map(|(word, label)| {
                let on = filters.contains(&word);
                div()
                    .id(SharedString::from(format!("br-chip-{word}")))
                    .debug_selector(move || format!("br-chip-{word}"))
                    .px(px(6.))
                    .py(px(3.))
                    .text_size(px(11.))
                    .text_color(c(if on { THEME.text } else { THEME.text2 }))
                    .when(on, |d| d.bg(c(THEME.bg4)))
                    .cursor_pointer()
                    .child(tr(label))
                    .on_click(cx.listener(move |b, _, _, cx| b.toggle_filter(word, cx)))
            }))
            .child(
                div()
                    .id("br-chip-fav")
                    .debug_selector(|| "br-chip-fav".into())
                    .px(px(6.))
                    .py(px(3.))
                    .text_size(px(11.))
                    .text_color(c(if fav { THEME.text } else { THEME.text2 }))
                    .when(fav, |d| d.bg(c(THEME.bg4)))
                    .cursor_pointer()
                    .child("★")
                    .on_click(cx.listener(|b, _, _, cx| b.toggle_fav(cx))),
            )
            .into_any_element()
    }

    fn set_view(&mut self, index: bool, cx: &mut Context<Self>) {
        cx.default_global::<BrowserState>().index = index;
        self.results = None;
        let q = self.query.clone();
        self.start_search(q, cx);
    }

    fn toggle_filter(&mut self, word: &'static str, cx: &mut Context<Self>) {
        let s = cx.default_global::<BrowserState>();
        match s.filters.iter().position(|f| *f == word) {
            Some(i) => {
                s.filters.remove(i);
            }
            None => s.filters.push(word),
        }
        let q = self.query.clone();
        self.start_search(q, cx);
    }

    fn toggle_fav(&mut self, cx: &mut Context<Self>) {
        let s = cx.default_global::<BrowserState>();
        s.fav = !s.fav;
        let q = self.query.clone();
        self.start_search(q, cx);
    }

    fn set_favorite(&mut self, e: &BrowseEntry, on: bool, cx: &mut Context<Self>) {
        let id = e.id;
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.set_favorite(id, on));
        });
        self.children.clear();
        if let Some(r) = &mut self.results {
            for x in r.iter_mut().filter(|x| x.id == id) {
                x.favorite = on;
            }
        }
        cx.notify();
    }

    // --- drawing ----------------------------------------------------------------------------------------------

    fn row(&self, e: &BrowseEntry, depth: usize, cx: &Context<Self>) -> AnyElement {
        let id = e.id;
        let selected = self.sel == Some(id);
        let open = e.kind == EntryKind::Folder && cx.try_global::<BrowserState>().is_some_and(|s| s.open.contains(&id));
        let (ec, ed, er) = (e.clone(), e.clone(), e.clone());
        let label: SharedString = e.name.to_string().into();
        let blocked = e.info.get("blocked").and_then(Value::as_bool).unwrap_or(false);
        let drag = match (e.kind, e.info.get("plugin").and_then(Value::as_shared_text)) {
            (EntryKind::Plugin, Some(p)) => Dragged::Plugin(p.clone()),
            _ => Dragged::Entry(e.clone()),
        };
        div()
            .id(("br-row", id.0 as usize))
            .debug_selector(move || format!("br-entry-{}", id.0))
            .h(px(ROW_H))
            .pl(px(4. + 14. * depth as f32))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(5.))
            .bg(c(if selected { THEME.bg4 } else { THEME.bg1 }))
            .text_size(px(12.))
            .text_color(c(if e.hidden || blocked { THEME.text3 } else { THEME.text }))
            .cursor_pointer()
            .on_click(cx.listener(move |b, ev: &gpui::ClickEvent, _, cx| {
                if ev.click_count() == 2 {
                    b.send(&ed, cx);
                } else {
                    b.select(&ec, true, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |b, ev: &MouseDownEvent, _, cx| {
                    b.sel = Some(er.id);
                    b.menu = Some((er.clone(), ev.position));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|b, _, _, cx| {
                    // the preview stops when the button is let go, unless it is a sample (F-BRW-005)
                    if b.preview.as_ref().is_some_and(|(e, _)| e.kind != EntryKind::Audio) {
                        b.stop_preview(cx);
                    }
                }),
            )
            .on_drag(drag, move |_, _, _, cx| cx.new(|_| DragLabel(label.clone())))
            .child(div().w(px(9.)).text_size(px(8.)).text_color(c(THEME.text3)).child(match e.kind {
                EntryKind::Folder if open => "▾",
                EntryKind::Folder => "▸",
                _ => "",
            }))
            .child(
                crate::ui::icon(kind_icon_of(e.kind, open, e.info.get("type").and_then(Value::as_text))).size(px(14.)),
            )
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().child(e.name.to_string()))
            .when(e.favorite, |d| d.child(crate::ui::icon("star").size(px(12.))))
            .when(blocked, |d| d.child(div().text_size(px(9.5)).text_color(c(THEME.red)).child(tr("blocked"))))
            .into_any_element()
    }

    fn menu_layer(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (e, at) = self.menu.clone()?;
        let mut items: Vec<(SharedString, bool, RowAction)> = Vec::new();
        let is_plugin = e.kind == EntryKind::Plugin;
        let send_label = match e.kind {
            EntryKind::Plugin if e.info.get("blocked").and_then(Value::as_bool) == Some(true) => None,
            EntryKind::Plugin => Some(tr("Send to the active track or channel")),
            EntryKind::Preset => Some(tr("Load on its plugin")),
            EntryKind::Project => Some(tr("Open")),
            _ => None,
        };
        if let Some(l) = send_label {
            let x = e.clone();
            items.push((l.into(), true, std::rc::Rc::new(move |b, cx| b.send(&x, cx))));
        }
        if !is_plugin {
            let x = e.clone();
            items.push((
                tr("Show in the file manager").into(),
                true,
                std::rc::Rc::new(move |b, cx| {
                    let core = b.scx.core.clone();
                    run_action(&core, "browser.reveal", Value::int(x.id.0 as i64), cx);
                }),
            ));
        }
        let fav = e.favorite;
        let x = e.clone();
        items.push((
            if fav { tr("Remove from the favourites") } else { tr("Add to the favourites") }.into(),
            true,
            std::rc::Rc::new(move |b, cx| b.set_favorite(&x, !fav, cx)),
        ));
        if let Some(path) = e.info.get("path").and_then(Value::as_text) {
            items.push((path.to_string().into(), false, std::rc::Rc::new(|_, _| {})));
        }
        Some(
            deferred(
                anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .id("br-menu")
                        .debug_selector(|| "br-menu".into())
                        .occlude()
                        .min_w(px(180.))
                        .py(px(3.))
                        .bg(c(THEME.bg3))
                        .border_1()
                        .border_color(c(THEME.line2))
                        .shadow_lg()
                        .child(
                            div()
                                .px(px(10.))
                                .py(px(3.))
                                .text_size(px(11.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(c(THEME.text2))
                                .child(e.name.to_string()),
                        )
                        .on_mouse_down_out(cx.listener(|b, _, _, cx| {
                            b.menu = None;
                            cx.notify();
                        }))
                        .children(items.into_iter().enumerate().map(|(k, (label, enabled, action))| {
                            div()
                                .id(("br-menu-item", k))
                                .debug_selector(move || format!("br-menu-item-{k}"))
                                .px(px(10.))
                                .py(px(3.))
                                .text_size(px(12.))
                                .text_color(c(if enabled { THEME.text } else { THEME.text3 }))
                                .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(c(THEME.bg4))))
                                .child(label)
                                .on_click(cx.listener(move |b, _, _, cx| {
                                    if enabled {
                                        b.menu = None;
                                        action(b, cx);
                                        cx.notify();
                                    }
                                }))
                        })),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The waveform of the preview, as bars (F-BRW-011): from the peaks service
    fn thumb_el(&self, entry: EntryId) -> Option<AnyElement> {
        let (id, pairs) = self.thumb.as_ref()?;
        if *id != entry {
            return None;
        }
        const W: usize = 240;
        let cols = thumb_columns(pairs, W);
        Some(
            div()
                .id("br-thumb")
                .debug_selector(|| "br-thumb".into())
                .h(px(28.))
                .mx(px(6.))
                .flex()
                .items_center()
                .children(cols.into_iter().map(|(lo, hi)| {
                    let h = ((hi - lo) * 13.0).clamp(1.0, 26.0);
                    div().flex_1().h(px(h)).bg(c(THEME.green))
                }))
                .into_any_element(),
        )
    }

    /// The line at the bottom: the search's or the preview's loading, or what is playing
    fn status(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let port = self.scx.core.read(cx).port();
        let core = self.scx.core.clone();
        let scan = Target::Scan(ScanKind::Library);
        if let Some(l) =
            loading_component("br-searching", &port.loading(&scan), move |_, cx| cancel_target(&core, scan.clone(), cx))
        {
            return Some(div().h(px(24.)).debug_selector(|| "br-searching".into()).child(l).into_any_element());
        }
        let (e, prepared) = self.preview.as_ref()?;
        if let Some(p) = prepared {
            let target = Target::Resource(p.resource.clone());
            let core = self.scx.core.clone();
            if let Some(l) = loading_component("br-loading", &port.loading(&target), move |_, cx| {
                cancel_target(&core, target.clone(), cx)
            }) {
                return Some(div().h(px(24.)).debug_selector(|| "br-loading".into()).child(l).into_any_element());
            }
        }
        let line = div()
            .id("br-playing")
            .debug_selector(|| "br-playing".into())
            .h(px(22.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(11.))
            .text_color(c(THEME.text2))
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().child(trf("Preview: {0}", &[&e.name])))
            .child(
                div()
                    .id("br-stop")
                    .debug_selector(|| "br-stop".into())
                    .px(px(5.))
                    .cursor_pointer()
                    .text_color(c(THEME.text))
                    .child("■")
                    .on_click(cx.listener(|b, _, _, cx| b.stop_preview(cx))),
            )
            .into_any_element();
        Some(div().flex().flex_col().children(self.thumb_el(e.id)).child(line).into_any_element())
    }
}

/// A track's instrument: its node and its plugin
type Instrument = (lm_port::NodeId, Arc<str>);

/// What an item of a row's menu does
type RowAction = std::rc::Rc<dyn Fn(&mut Browser, &mut Context<Browser>)>;

/// The chip under the mouse while dragging
pub struct DragLabel(pub SharedString);
impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(6.))
            .py(px(2.))
            .bg(c(THEME.bg4))
            .border_1()
            .border_color(c(THEME.green))
            .text_size(px(11.))
            .text_color(c(THEME.text))
            .child(self.0.clone())
    }
}

impl Focusable for Browser {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Browser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.poll(cx);
        let index_view = cx.try_global::<BrowserState>().is_some_and(|s| s.index);
        let rows = self.rows(cx);
        let mut list: Vec<AnyElement> = Vec::new();
        let mut factory_line = false;
        let searching = self.results.is_some();
        for (e, depth) in &rows {
            if depth == &0 && e.factory && !factory_line && !searching {
                list.push(
                    div()
                        .h(px(ROW_H))
                        .px(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(10.))
                        .text_color(c(THEME.text3))
                        .child(tr("Factory files"))
                        .child(div().flex_1().h(px(1.)).bg(c(THEME.line2)))
                        .into_any_element(),
                );
                factory_line = true;
            }
            list.push(self.row(e, *depth, cx));
        }
        if searching && rows.is_empty() {
            list.push(
                div()
                    .p(px(6.))
                    .text_size(px(11.))
                    .text_color(c(THEME.text3))
                    .child(tr("Nothing found."))
                    .into_any_element(),
            );
        }
        let hidden = cx.try_global::<BrowserState>().is_some_and(|s| s.hidden);
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
            .on_action(cx.listener(|b, _: &BrowserDown, _, cx| b.step(1, cx)))
            .on_action(cx.listener(|b, _: &BrowserUp, _, cx| b.step(-1, cx)))
            .on_action(cx.listener(|b, _: &BrowserSend, _, cx| {
                if let Some(e) = b.selected(cx) {
                    b.send(&e, cx);
                }
            }))
            .on_action(cx.listener(|b, _: &BrowserOpenFolder, _, cx| {
                if let Some(e) = b.selected(cx).filter(|e| e.kind == EntryKind::Folder) {
                    b.toggle_folder(e.id, Some(true), cx);
                }
            }))
            .on_action(cx.listener(|b, _: &BrowserCloseFolder, _, cx| {
                if let Some(e) = b.selected(cx).filter(|e| e.kind == EntryKind::Folder) {
                    b.toggle_folder(e.id, Some(false), cx);
                }
            }))
            .on_action(cx.listener(|b, _: &BrowserRefresh, _, cx| {
                b.children.clear();
                let q = b.query.clone();
                b.start_search(q, cx);
            }))
            .on_action(cx.listener(|b, _: &BrowserFind, window, cx| b.search.update(cx, |i, cx| i.focus(window, cx))))
            // Space held previews the selected entry (F-BRW-005); keymap-ok: a key held, down and up, is not a command
            .on_key_down(cx.listener(|b, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "space"
                    && !ev.is_held
                    && let Some(e) = b.selected(cx).filter(|e| e.kind != EntryKind::Folder)
                {
                    b.start_preview(e, cx);
                    cx.stop_propagation();
                }
            }))
            // keymap-ok: a key held, down and up, is not a command
            .on_key_up(cx.listener(|b, ev: &KeyUpEvent, _, cx| {
                // keymap-ok: a key held, down and up, is not a command
                if ev.keystroke.key == "space" {
                    b.stop_preview(cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|b, _, window, cx| {
                    if !b.search.read(cx).focus_handle(cx).is_focused(window) {
                        window.focus(&b.focus, cx);
                    }
                }),
            )
            // the view: folders or the index
            .child(div().flex().px(px(2.)).bg(c(THEME.bg2)).border_b_1().border_color(c(THEME.line)).children(
                [(false, "Folders"), (true, "Index")].into_iter().map(|(v, label)| {
                    div()
                        .id(SharedString::from(format!("br-view-{label}")))
                        .debug_selector(move || format!("br-view-{label}"))
                        .px(px(6.))
                        .py(px(3.))
                        .text_size(px(11.))
                        .text_color(c(if index_view == v { THEME.text } else { THEME.text2 }))
                        .when(index_view == v, |d| d.bg(c(THEME.bg4)))
                        .cursor_pointer()
                        .child(tr(label))
                        .on_click(cx.listener(move |b, _, _, cx| b.set_view(v, cx)))
                }),
            ))
            .child(if index_view { self.chips(cx) } else { self.page_tabs(cx) })
            // the page's title, as the legacy's
            .child(
                div()
                    .px(px(6.))
                    .pt(px(4.))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .child(tr(page_title(self.page))),
                    )
                    .child(
                        div()
                            .id("br-hidden")
                            .debug_selector(|| "br-hidden".into())
                            .text_size(px(10.))
                            .text_color(c(if hidden { THEME.text } else { THEME.text3 }))
                            .cursor_pointer()
                            .child(tr("Hidden files"))
                            .on_click(cx.listener(|b, _, _, cx| {
                                let s = cx.default_global::<BrowserState>();
                                s.hidden = !s.hidden;
                                b.children.clear();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("br-search")
                    .debug_selector(|| "br-search".into())
                    .px(px(4.))
                    .py(px(4.))
                    .child(crate::ui::text_entry(Input::new(&self.search).small().cleanable(true))),
            )
            .child(div().id("br-list").flex_1().overflow_y_scroll().children(list))
            .children(self.status(cx))
            .children(self.menu_layer(cx))
    }
}
