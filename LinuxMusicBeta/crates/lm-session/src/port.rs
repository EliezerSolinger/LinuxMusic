//! The `LocalPort` (I-15): the `CorePort` of the core in the same process. It owns the [`Session`]; every call is a
//! direct Rust call with Rust values, nothing serialised. Edits as data become one transaction; requests that touch
//! the disk (the browser, the devices, a preview) are operations whose answers come back by `finished` (I-2), so the
//! interface never waits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lm_base::{Error, Label, OperationHandle, OperationId, Result};
use lm_doc::{Diff, Feed, GestureId, Topic};
use lm_port::{
    ActionId, Applied, BrowseEntry, BrowsePage, CorePort, DeviceInfo, DisplayView, EditOp, EntryId, EntryKind,
    Finished, Loading, NoteRef, Notice, OperationInfo, PeakBlock, Pending, Preview, Ref, RequestId, Target,
    TelemetryView,
};
use lm_render::{Command, PreviewCommand, PreviewId, SampleData};
use lm_state::{NodeId, PathStep, Resource, Value};

use crate::session::Session;

fn err(key: &'static str) -> Error {
    Error::new(key)
}

/// A prepared preview: the sound and the waveform
#[derive(Debug)]
struct Prepared {
    data: Arc<SampleData>,
    peaks: Arc<lm_io::Peaks>,
}

/// A file prepared for a preview: its resource, sound, peaks and length
type PreparedFile = (Resource, Arc<SampleData>, Arc<lm_io::Peaks>, i64);

/// The core in this process, as the interface sees it
pub struct LocalPort {
    session: Session,
    requests: Vec<(RequestId, OperationHandle<Finished>)>,
    next_request: RequestId,
    previews: BTreeMap<String, Prepared>,
    /// The prepared preview of each entry, by its resource's key
    preview_of: BTreeMap<EntryId, String>,
    search_request: Option<RequestId>,
    /// Imports asked through the port: the session's operation, and the request it answers
    imports: Vec<(OperationId, RequestId)>,
    preview_handles: Vec<(RequestId, EntryId, OperationHandle<PreparedFile>)>,
    playing_preview: Option<PreviewId>,
    next_preview: u64,
    done: Vec<(RequestId, Finished)>,
    tempo: lm_port::TempoCache,
}

impl std::fmt::Debug for LocalPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalPort").field("session", &self.session).finish_non_exhaustive()
    }
}

/// An id for a browser entry: stable for its page and path
fn entry_id(page: BrowsePage, key: &str) -> EntryId {
    crate::index::entry_id(page, key)
}

fn kind_of(path: &Path) -> EntryKind {
    if path.is_dir() {
        if path.join(crate::folder::PROJECT_FILE).exists() {
            return EntryKind::Project;
        }
        return EntryKind::Folder;
    }
    crate::browser::file_type(path).map_or(EntryKind::Other, |t| t.kind)
}

fn page_named(name: &str) -> Option<BrowsePage> {
    Some(match name {
        "Instruments" => BrowsePage::Instruments,
        "Effects" => BrowsePage::Effects,
        "Samples" => BrowsePage::Samples,
        "Presets" => BrowsePage::Presets,
        "Projects" => BrowsePage::Projects,
        "ProjectFiles" => BrowsePage::ProjectFiles,
        "Favorites" => BrowsePage::Favorites,
        _ => return None,
    })
}

/// The entries of a folder (on a worker)
fn list_folder(
    page: BrowsePage,
    dir: &Path,
    parent: Option<EntryId>,
    hidden: bool,
    favorites: &[String],
    factory: bool,
) -> Vec<(BrowseEntry, PathBuf)> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else { return out };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let is_hidden = name.starts_with('.');
        if is_hidden && !hidden {
            continue;
        }
        let kind = kind_of(&path);
        let file_type = crate::browser::file_type(&path);
        if page == BrowsePage::Samples && kind != EntryKind::Folder && !file_type.is_some_and(|t| t.sample) {
            continue;
        }
        if page == BrowsePage::Projects && !matches!(kind, EntryKind::Folder | EntryKind::Project) {
            continue;
        }
        let text = path.display().to_string();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        out.push((
            BrowseEntry {
                id: entry_id(page, &text),
                parent,
                name: Arc::from(name.as_str()),
                kind,
                factory,
                hidden: is_hidden,
                favorite: favorites.contains(&text),
                info: {
                    let info = Value::object([("path", Value::text(text)), ("bytes", Value::int(size as i64))]);
                    match file_type {
                        Some(t) => info.with("type", Value::text(t.name)),
                        None => info,
                    }
                },
            },
            path,
        ));
    }
    out
}

/// The user's presets: `<dir>/<plugin>/<name>.json`, one entry each with its plugin in `info.plugin`
fn preset_entries(dir: &Path, factory: bool) -> Vec<(BrowseEntry, Option<PathBuf>)> {
    let mut found = Vec::new();
    for plugin in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let plugin_dir = plugin.path();
        let plugin_id = plugin.file_name().to_string_lossy().into_owned();
        for file in std::fs::read_dir(&plugin_dir).into_iter().flatten().flatten() {
            let path = file.path();
            if path.extension().is_none_or(|e| e != "json") || !path.is_file() {
                continue;
            }
            found.push((plugin_id.clone(), path));
        }
    }
    found.sort();
    found
        .into_iter()
        .map(|(plugin, path)| {
            let text = path.display().to_string();
            let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (
                BrowseEntry {
                    id: entry_id(BrowsePage::Presets, &text),
                    parent: None,
                    name: Arc::from(name.as_str()),
                    kind: EntryKind::Preset,
                    factory,
                    hidden: false,
                    favorite: false,
                    info: Value::object([("path", Value::text(text)), ("plugin", Value::text(plugin))]),
                },
                Some(path),
            )
        })
        .collect()
}

impl LocalPort {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            requests: Vec::new(),
            next_request: 1,
            previews: BTreeMap::new(),
            preview_of: BTreeMap::new(),
            search_request: None,
            imports: Vec::new(),
            preview_handles: Vec::new(),
            playing_preview: None,
            next_preview: 1,
            done: Vec::new(),
            tempo: Default::default(),
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    pub fn into_session(self) -> Session {
        self.session
    }

    /// A request answered by an operation; `failed` turns its error (cancelled, panicked) into the request's kind
    fn request(
        &mut self,
        failed: fn(Error) -> Finished,
        work: impl FnOnce(&lm_base::Cancel, &lm_base::Progress) -> Result<Finished> + Send + 'static,
    ) -> RequestId {
        let id = self.next_request;
        self.next_request += 1;
        let handle = self.session.workers().start(move |c, p| Ok(work(c, p).unwrap_or_else(failed)));
        self.requests.push((id, handle));
        id
    }

    /// Applies a list of edits inside one transaction (the port's data form of `transact`)
    fn apply_ops(e: &mut lm_doc::Edit<'_>, ops: &[EditOp], applied: &mut Applied) -> Result<()> {
        let id = |r: Ref, created: &[(u32, NodeId)]| -> Result<NodeId> {
            match r {
                Ref::Id(id) => Ok(id),
                Ref::Temp(t) => {
                    created.iter().find(|(k, _)| *k == t).map(|(_, id)| *id).ok_or_else(|| err("port.ref.unknown_temp"))
                }
            }
        };
        let note = |n: NoteRef, notes: &[(u32, u64)]| -> Result<u64> {
            match n {
                NoteRef::N(n) => Ok(n),
                NoteRef::Temp(t) => {
                    notes.iter().find(|(k, _)| *k == t).map(|(_, n)| *n).ok_or_else(|| err("port.ref.unknown_temp"))
                }
            }
        };
        for op in ops {
            match op {
                EditOp::Set { id: r, path, value } => {
                    let n = id(*r, &applied.created)?;
                    // A plugin's parameter goes through set_param, so its slot hears it (S20)
                    if path.len() == 3
                        && path[0] == PathStep::key("state")
                        && path[1] == PathStep::key("params")
                        && let (PathStep::Key(key), Some(v)) = (&path[2], value.as_real())
                    {
                        e.set_param(n, key, v)?;
                        continue;
                    }
                    e.set(n, path, value.clone())?;
                }
                EditOp::Insert { temp, parent, list, index, node } => {
                    let p = id(*parent, &applied.created)?;
                    let new = e.insert(p, list, *index, node.clone())?;
                    applied.created.push((*temp, new));
                }
                EditOp::Remove { id: r } => e.remove(id(*r, &applied.created)?)?,
                EditOp::Move { id: r, parent, list, index } => {
                    let (n, p) = (id(*r, &applied.created)?, id(*parent, &applied.created)?);
                    e.move_to(n, p, list, *index)?;
                }
                EditOp::AddNote { temp, item, note: v } => {
                    let n = e.add_note(id(*item, &applied.created)?, v.clone())?;
                    applied.notes.push((*temp, n));
                }
                EditOp::SetNote { item, n, path, value } => {
                    let it = id(*item, &applied.created)?;
                    e.set_note(it, note(*n, &applied.notes)?, path, value.clone())?;
                }
                EditOp::RemoveNote { item, n } => {
                    let it = id(*item, &applied.created)?;
                    e.remove_note(it, note(*n, &applied.notes)?)?;
                }
            }
        }
        Ok(())
    }

    fn favorites(&self) -> Vec<String> {
        self.session
            .preferences()
            .get(&["favorites"])
            .map(|l| l.items().filter_map(Value::as_text).map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// A folder entry on the disk, with the factory folders listed with it
    fn folder_entry(
        page: BrowsePage,
        name: &str,
        path: PathBuf,
        factory: bool,
        extra: &[PathBuf],
    ) -> (BrowseEntry, Option<PathBuf>) {
        let text = path.display().to_string();
        let mut info = Value::object([("path", Value::text(text.as_str()))]);
        if !extra.is_empty() {
            info = info.with("extra", Value::list(extra.iter().map(|p| Value::text(p.display().to_string()))));
        }
        (
            BrowseEntry {
                id: entry_id(page, &text),
                parent: None,
                name: Arc::from(name),
                kind: EntryKind::Folder,
                factory,
                hidden: false,
                favorite: false,
                info,
            },
            Some(path),
        )
    }

    /// The user's folders of a page and the factory's, as one list: a factory folder joins the user's of the same
    /// name, else it comes after them (F-BRW-002)
    fn merged_roots(
        page: BrowsePage,
        user: Vec<(String, PathBuf)>,
        factory: Vec<PathBuf>,
        factory_name: &str,
    ) -> Vec<(BrowseEntry, Option<PathBuf>)> {
        let mut out: Vec<(String, PathBuf, Vec<PathBuf>)> = user.into_iter().map(|(n, p)| (n, p, Vec::new())).collect();
        let mut alone: Vec<PathBuf> = Vec::new();
        for f in factory {
            match out.iter_mut().find(|(n, _, _)| n == factory_name) {
                Some(same) => same.2.push(f),
                None => alone.push(f),
            }
        }
        let mut v: Vec<_> =
            out.into_iter().map(|(n, p, extra)| Self::folder_entry(page, &n, p, false, &extra)).collect();
        if let Some((first, rest)) = alone.split_first() {
            v.push(Self::folder_entry(page, factory_name, first.clone(), true, rest));
        }
        v
    }

    fn factory_part(&self, part: &str) -> Vec<PathBuf> {
        crate::browser::factory_part(&crate::browser::factory_dirs(), part)
    }

    /// The user's sample folders (the preferences'), or the default one
    fn sample_folders(&self) -> Vec<(String, PathBuf)> {
        let prefs = self.session.preferences();
        let projects = PathBuf::from(prefs.text(&["projectsFolder"]).unwrap_or("."));
        let mut v: Vec<(String, PathBuf)> = prefs
            .get(&["sampleFolders"])
            .map(|l| l.items().filter_map(Value::as_text).map(PathBuf::from).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), p))
            .collect();
        if v.is_empty() {
            v.push(("Samples".into(), projects.join("Samples")));
        }
        v
    }

    /// The roots of a page, and the folder each lists
    fn roots(&self, page: BrowsePage) -> Vec<(BrowseEntry, Option<PathBuf>)> {
        let prefs = self.session.preferences();
        let projects = PathBuf::from(prefs.text(&["projectsFolder"]).unwrap_or("."));
        match page {
            // The categories are made by `plugin_categories`
            BrowsePage::Instruments | BrowsePage::Effects => Vec::new(),
            BrowsePage::Samples => {
                Self::merged_roots(page, self.sample_folders(), self.factory_part("samples"), "Samples")
            }
            BrowsePage::Projects => {
                Self::merged_roots(page, vec![("Projects".into(), projects)], self.factory_part("projects"), "Projects")
            }
            BrowsePage::ProjectFiles => {
                vec![Self::folder_entry(page, "audio", self.session.project().folder.audio(), false, &[])]
            }
            BrowsePage::Presets => {
                let mut v = preset_entries(&self.session.presets_dir(), false);
                for dir in self.factory_part("presets") {
                    for f in preset_entries(&dir, true) {
                        let same = |x: &(BrowseEntry, Option<PathBuf>)| {
                            x.0.name == f.0.name && x.0.info.get("plugin") == f.0.info.get("plugin")
                        };
                        if !v.iter().any(same) {
                            v.push(f);
                        }
                    }
                }
                v
            }
            BrowsePage::Favorites => self
                .favorites()
                .into_iter()
                .map(|f| {
                    let p = PathBuf::from(&f);
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(f);
                    let mut e = Self::folder_entry(page, &name, p.clone(), false, &[]);
                    e.0.kind = kind_of(&p);
                    e.0.favorite = true;
                    e
                })
                .collect(),
        }
    }

    /// The plugins of the Instruments or Effects page by category (F-BRW-009/010): the category folders, each with
    /// its plugins
    fn plugin_categories(&self, page: BrowsePage) -> Vec<(BrowseEntry, Vec<BrowseEntry>)> {
        let instrument = page == BrowsePage::Instruments;
        let want = if instrument { lm_plugin::PluginKind::Instrument } else { lm_plugin::PluginKind::Effect };
        let mut cats: BTreeMap<&'static str, (&'static str, Vec<BrowseEntry>)> = BTreeMap::new();
        for d in self.session.registry().entries().filter(|d| d.kind == want) {
            let (key, label) = if d.third_party {
                crate::browser::THIRD_PARTY_CATEGORY
            } else {
                crate::browser::plugin_category(d.id, instrument)
            };
            cats.entry(key).or_insert((label, Vec::new())).1.push(BrowseEntry {
                id: entry_id(page, d.id),
                parent: Some(entry_id(page, &format!("category:{key}"))),
                name: Arc::from(d.name),
                kind: EntryKind::Plugin,
                factory: false,
                hidden: false,
                favorite: false,
                info: Value::object([
                    ("plugin", Value::text(d.id)),
                    ("category", Value::text(key)),
                    ("third_party", Value::bool(d.third_party)),
                ]),
            });
        }
        cats.into_iter()
            .map(|(key, (label, kids))| {
                let folder = BrowseEntry {
                    id: entry_id(page, &format!("category:{key}")),
                    parent: None,
                    name: Arc::from(label),
                    kind: EntryKind::Folder,
                    factory: false,
                    hidden: false,
                    favorite: false,
                    info: Value::object([("category", Value::text(key))]),
                };
                (folder, kids)
            })
            .collect()
    }

    /// The user's tags of files: path -> tags
    fn user_tags(&self) -> BTreeMap<String, Vec<String>> {
        self.session
            .preferences()
            .get(&["tags"])
            .map(|o| {
                o.entries()
                    .map(|(p, l)| (p.to_owned(), l.items().filter_map(Value::as_text).map(str::to_owned).collect()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The SoundFont a MIDI preview is played on: the preferences' (`defaultSoundFont`), else the first of the index
    fn default_soundfont(&self) -> Option<PathBuf> {
        if let Some(p) = self.session.preferences().text(&["defaultSoundFont"]).filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(p));
        }
        let index = self.session.browser.index.lock().ok()?.clone();
        index.items.iter().find(|i| i.ty == "soundfont").map(|i| PathBuf::from(&i.path))
    }

    /// What the index covers: the sample, project and preset folders of the user and of the factory
    fn index_roots(&self) -> Vec<crate::index::Root> {
        use crate::index::{Root, RootKind};
        let prefs = self.session.preferences();
        let projects = PathBuf::from(prefs.text(&["projectsFolder"]).unwrap_or("."));
        let mut roots: Vec<Root> = self
            .sample_folders()
            .into_iter()
            .map(|(_, dir)| Root { dir, factory: false, kind: RootKind::Files })
            .collect();
        roots.push(Root { dir: projects, factory: false, kind: RootKind::Files });
        roots.push(Root { dir: self.session.presets_dir(), factory: false, kind: RootKind::Presets });
        for dir in self.factory_part("samples").into_iter().chain(self.factory_part("projects")) {
            roots.push(Root { dir, factory: true, kind: RootKind::Files });
        }
        for dir in self.factory_part("presets") {
            roots.push(Root { dir, factory: true, kind: RootKind::Presets });
        }
        // The projects folder holds the samples folder and the presets: what is under another root is read once
        let mut seen = std::collections::BTreeSet::new();
        roots.retain(|r| seen.insert((r.dir.clone(), r.kind == RootKind::Presets)));
        roots
    }

    /// Starts the scan of the index when there is none yet, or the last one is old (an Operation of the session:
    /// its loading shows on `Scan(Library)`); the saved index is searchable at once
    fn ensure_index(&mut self) {
        use std::sync::atomic::Ordering;
        let b = &self.session.browser;
        if b.indexing.load(Ordering::Acquire) || b.indexed_at.is_some_and(|t| t.elapsed() < INDEX_AGE) {
            return;
        }
        let roots = self.index_roots();
        let (slot, running) = (b.index.clone(), b.indexing.clone());
        running.store(true, Ordering::Release);
        self.session.browser.indexed_at = Some(std::time::Instant::now());
        let file = crate::prefs::location().with_file_name("browser-index.json");
        let _ = self.session.start(
            Label::new("session.browser.indexing"),
            Target::Scan(lm_port::ScanKind::Library),
            true,
            "browser_index",
            move |cancel, _| {
                let result = (|| {
                    let mut prev = slot.lock().map(|g| g.clone()).unwrap_or_default();
                    if prev.built == 0 {
                        prev = Arc::new(crate::index::Index::load(&file));
                        if let Ok(mut g) = slot.lock() {
                            *g = prev.clone();
                        }
                    }
                    let fresh = Arc::new(crate::index::Index::scan(&roots, &prev, cancel)?);
                    if let Ok(mut g) = slot.lock() {
                        *g = fresh.clone();
                    }
                    let _ = fresh.save(&file);
                    Ok(Value::NULL)
                })();
                running.store(false, Ordering::Release);
                result
            },
        );
    }

    /// The actions that name a browser entry with the entry's file in its place (the session's resolution, which
    /// `Session::run_action` does too)
    fn entry_params(&self, action: &str, params: Value) -> Result<Value> {
        self.session.browser.resolve(action, params)
    }

    fn remember(&mut self, page: BrowsePage, entries: &[(BrowseEntry, Option<PathBuf>)]) {
        for (e, p) in entries {
            remember_entry(&mut self.session.browser, page, e, p.clone());
        }
    }
}

/// An entry given out, with what the browser needs to list it again: its file, the folders listed with it
fn remember_entry(b: &mut crate::browser::BrowserEntries, page: BrowsePage, e: &BrowseEntry, path: Option<PathBuf>) {
    let page = e.info.get("page").and_then(Value::as_text).and_then(page_named).unwrap_or(page);
    let extra: Vec<PathBuf> = e
        .info
        .get("extra")
        .map(|l| l.items().filter_map(Value::as_text).map(PathBuf::from).collect())
        .unwrap_or_default();
    if extra.is_empty() && !e.factory {
        b.remember(e.id, page, path);
    } else {
        b.remember_folder(e.id, page, path, extra, None, e.factory);
    }
}

/// How long an index is kept before the next browse or search reads the folders again
const INDEX_AGE: std::time::Duration = std::time::Duration::from_secs(30);

/// The waveform of rendered audio
fn peaks_of_data(data: &SampleData) -> lm_io::Peaks {
    let channels = data.channels().max(1);
    let mut b = lm_io::PeaksBuilder::new(channels);
    let mut frame = vec![0.0f32; channels * 4096];
    let mut at = 0;
    while at < data.frames() {
        let n = 4096.min(data.frames() - at);
        for c in 0..channels {
            let ch = data.channel(c);
            for i in 0..n {
                frame[i * channels + c] = ch[at + i];
            }
        }
        b.push_interleaved(&frame[..n * channels]);
        at += n;
    }
    b.finish()
}

fn peak_block(peaks: &lm_io::Peaks, level: u8, from: u64, to: u64) -> PeakBlock {
    let l = usize::from(level).min(peaks.levels().saturating_sub(1));
    let (a, b) = (from as usize, to as usize);
    let mut pairs: Vec<(i16, i16)> = Vec::new();
    for c in 0..peaks.channels() {
        let r = peaks.range(l, c, a, b);
        for (i, mm) in r.as_chunks::<2>().0.iter().enumerate() {
            if c == 0 {
                pairs.push((mm[0], mm[1]));
            } else if let Some(p) = pairs.get_mut(i) {
                p.0 = p.0.min(mm[0]);
                p.1 = p.1.max(mm[1]);
            }
        }
    }
    PeakBlock {
        level: u8::try_from(l).unwrap_or(0),
        from,
        samples_per_pair: lm_io::Peaks::frames_per_pair(l),
        pairs: Arc::from(pairs),
        length: peaks.frames(),
    }
}

impl CorePort for LocalPort {
    fn pump(&mut self) -> bool {
        let mut changed = self.session.pump();
        for (op, result) in self.session.take_imported() {
            if let Some(pos) = self.imports.iter().position(|(o, _)| *o == op) {
                let (_, id) = self.imports.swap_remove(pos);
                self.done.push((id, Finished::Imported(result)));
                changed = true;
            }
        }
        let mut i = 0;
        while i < self.requests.len() {
            if self.requests[i].1.is_finished() {
                let (id, mut h) = self.requests.swap_remove(i);
                let result =
                    h.poll().and_then(Result::ok).unwrap_or_else(|| Finished::Done(Err(err("port.request.lost"))));
                if let Finished::Entries(Ok(entries)) = &result {
                    for e in entries {
                        let path = e.info.get("path").and_then(Value::as_text).map(PathBuf::from);
                        let page = self
                            .session
                            .browser
                            .known
                            .get(&e.parent.unwrap_or(EntryId(0)))
                            .map_or(BrowsePage::Samples, |k| k.page);
                        remember_entry(&mut self.session.browser, page, e, path);
                    }
                }
                self.done.push((id, result));
                changed = true;
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.preview_handles.len() {
            if self.preview_handles[i].2.is_finished() {
                let (id, entry, mut h) = self.preview_handles.swap_remove(i);
                let result = h.poll().unwrap_or_else(|| Err(err("port.request.lost")));
                let preview = result.map(|(resource, data, peaks, length)| {
                    let rate = data.rate();
                    self.previews.insert(resource.key().to_owned(), Prepared { data, peaks });
                    self.preview_of.insert(entry, resource.key().to_owned());
                    Preview { entry, resource, length, sample_rate: rate }
                });
                self.done.push((id, Finished::Preview(preview)));
                changed = true;
            } else {
                i += 1;
            }
        }
        changed
    }

    fn finished(&mut self) -> Vec<(RequestId, Finished)> {
        std::mem::take(&mut self.done)
    }

    fn root(&self) -> &Value {
        self.session.document().root()
    }

    fn find(&self, id: NodeId) -> Option<Value> {
        self.session.document().find(id)
    }

    fn feed(&mut self, topic: Topic) -> Feed {
        self.session.document_mut().feed(topic)
    }

    fn drain(&self, feed: &mut Feed) -> Option<Diff> {
        feed.drain(self.session.document())
    }

    fn modified(&self) -> bool {
        self.session.modified()
    }

    fn can_undo(&self) -> bool {
        self.session.document().can_undo()
    }

    fn can_redo(&self) -> bool {
        self.session.document().can_redo()
    }

    fn apply(&mut self, label: Label, ops: Vec<EditOp>) -> Pending<Applied> {
        let mut applied = Applied::default();
        let r = self.session.document_mut().transact(label, |e| Self::apply_ops(e, &ops, &mut applied));
        Pending::Ready(r.map(|diff| Applied { diff: Some(diff), ..applied }))
    }

    fn run_action(&mut self, action: ActionId, params: Value) -> Pending<()> {
        // A browser entry is the port's: the session's actions take the file
        let params = match self.entry_params(action.as_str(), params) {
            Ok(p) => p,
            Err(e) => {
                self.session.notices().error(&e, None, None);
                return Pending::Ready(Err(e));
            }
        };
        let r = self.session.run_action(action.as_str(), &params);
        if let Err(e) = &r {
            let retry = Some((action.clone(), params));
            self.session.notices().error(e, None, retry);
        }
        Pending::Ready(r)
    }

    fn undo(&mut self) -> Pending<()> {
        Pending::Ready(self.session.document_mut().undo().map(|_| ()))
    }

    fn redo(&mut self) -> Pending<()> {
        Pending::Ready(self.session.document_mut().redo().map(|_| ()))
    }

    fn begin_gesture(&mut self, label: Label) -> GestureId {
        self.session.document_mut().begin_gesture(label)
    }

    fn gesture_apply(&mut self, g: &GestureId, ops: Vec<EditOp>) {
        let mut applied = Applied::default();
        let r = self.session.document_mut().gesture_update(g, |e| Self::apply_ops(e, &ops, &mut applied));
        if let Err(e) = r {
            self.session.notices().error(&e, None, None);
        }
    }

    fn gesture_commit(&mut self, g: GestureId) {
        if let Err(e) = self.session.document_mut().gesture_commit(g) {
            self.session.notices().error(&e, None, None);
        }
    }

    fn gesture_cancel(&mut self, g: GestureId) {
        if let Err(e) = self.session.document_mut().gesture_cancel(g) {
            self.session.notices().error(&e, None, None);
        }
    }

    fn param_write(&mut self, node: NodeId, param: &str, value: f32) {
        self.session.param_write(node, param, value);
    }

    fn note_on(&mut self, track: NodeId, key: u8, velocity: u8) {
        self.session.live_note(track, key, Some(velocity));
    }

    fn note_off(&mut self, track: NodeId, key: u8) {
        self.session.live_note(track, key, None);
    }

    fn all_notes_off(&mut self, track: NodeId) {
        self.session.live_note(track, 255, None);
    }

    fn loading(&self, target: &Target) -> Loading {
        self.session.loading(target)
    }

    fn activity(&self) -> Vec<OperationInfo> {
        self.session.activity()
    }

    fn cancel(&mut self, op: OperationId) {
        self.session.cancel(op);
    }

    fn notices(&mut self) -> Vec<Notice> {
        self.session.notices().all()
    }

    fn dismiss_notice(&mut self, id: u64) {
        self.session.notices().dismiss(id);
    }

    fn telemetry(&self) -> &TelemetryView {
        self.session.telemetry()
    }

    fn peaks(&mut self, resource: &Resource, level: u8, from: u64, to: u64) -> Pending<PeakBlock> {
        if let Some(p) = self.previews.get(resource.key()) {
            return Pending::Ready(Ok(peak_block(&p.peaks, level, from, to)));
        }
        // The take being recorded grows: its peaks are the ones the disk thread has sent so far (U3)
        if let Some(p) = self.session.live_peaks(resource.key()) {
            return Pending::Ready(Ok(peak_block(&p, level, from, to)));
        }
        match self.session.resources().peaks(resource.key()) {
            Some(p) => Pending::Ready(Ok(peak_block(&p, level, from, to))),
            None => {
                // Not loaded yet: asked for; its loading shows on the resource's target
                self.session.request(resource);
                Pending::Ready(Err(err("port.peaks.loading")))
            }
        }
    }

    fn param_range(&self, node: NodeId, key: &str) -> Option<lm_port::ParamRange> {
        // the one value law: the compiled range when the parameter has a slot, else its schema's
        Some(self.session.param_range(node, key))
    }

    fn tempo_map(&self) -> std::sync::Arc<lm_port::TempoMap> {
        self.tempo.get(self.root())
    }

    fn display(&self, node: NodeId) -> Option<DisplayView> {
        // A plugin node (instrument or insert) gives what its processor wrote (`kind` is the plugin's id); a desk
        // channel or the master gives its rack's readings
        if let Some((plugin, frame)) = self.session.plugin_display(node) {
            return Some(DisplayView { kind: Arc::from(plugin), frame: Arc::from(frame) });
        }
        self.session.rack_display(node).map(|frame| DisplayView { kind: Arc::from("rack"), frame: Arc::from(frame) })
    }

    fn preferences(&self) -> &Value {
        self.session.preferences().tree()
    }

    fn set_preference(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()> {
        Pending::Ready(self.session.set_preference(&path, value))
    }

    fn config_dir(&self) -> Option<std::path::PathBuf> {
        self.session.preferences().file().and_then(|f| f.parent()).map(std::path::Path::to_path_buf)
    }

    fn status(&self) -> &Value {
        self.session.status()
    }

    fn devices(&mut self) -> Pending<Vec<DeviceInfo>> {
        Pending::Waiting(self.request(
            |e| Finished::Devices(Err(e)),
            |cancel, progress| Ok(Finished::Devices(list_devices(cancel, progress))),
        ))
    }

    fn browse(&mut self, page: BrowsePage, parent: Option<EntryId>, show_hidden: bool) -> Pending<Vec<BrowseEntry>> {
        self.ensure_index();
        let Some(parent) = parent else {
            if matches!(page, BrowsePage::Instruments | BrowsePage::Effects) {
                let cats = self.plugin_categories(page);
                let mut folders = Vec::new();
                for (folder, kids) in cats {
                    self.session.browser.remember_folder(folder.id, page, None, Vec::new(), Some(kids), false);
                    folders.push(folder);
                }
                return Pending::Ready(Ok(folders));
            }
            let roots = self.roots(page);
            self.remember(page, &roots);
            return Pending::Ready(Ok(roots.into_iter().map(|(e, _)| e).collect()));
        };
        let Some(known) = self.session.browser.known.get(&parent).cloned() else {
            return Pending::Ready(Err(err("port.browse.unknown_entry")));
        };
        if let Some(kids) = known.kids {
            let list: Vec<(BrowseEntry, Option<PathBuf>)> = kids.iter().cloned().map(|e| (e, None)).collect();
            self.remember(known.page, &list);
            return Pending::Ready(Ok(kids));
        }
        let Some(dir) = known.path else {
            return Pending::Ready(Err(err("port.browse.unknown_entry")));
        };
        let (page, extra, primary_factory) = (known.page, known.extra, known.factory);
        let favorites = self.favorites();
        Pending::Waiting(self.request(
            |e| Finished::Entries(Err(e)),
            move |_, _| {
                let user = list_folder(page, &dir, Some(parent), show_hidden, &favorites, primary_factory);
                let mut factory = Vec::new();
                for d in &extra {
                    factory.extend(list_folder(page, d, Some(parent), show_hidden, &favorites, true));
                }
                let merged = crate::browser::merge_listing(user, factory);
                let entries = merged
                    .into_iter()
                    .map(|(mut e, _, extra)| {
                        if !extra.is_empty() {
                            e.info = e
                                .info
                                .with("extra", Value::list(extra.iter().map(|p| Value::text(p.display().to_string()))));
                        }
                        e
                    })
                    .collect();
                Ok(Finished::Entries(Ok(entries)))
            },
        ))
    }

    /// The search over the index (F-BRW-004): words, quoted phrases, `type:`, `tag:` and `is:fav`
    fn search(&mut self, query: &str) -> Pending<Vec<BrowseEntry>> {
        self.ensure_index();
        let q = crate::index::Query::parse(query);
        let index = self.session.browser.index.lock().map(|g| g.clone()).unwrap_or_default();
        let cx = crate::index::Context {
            plugins: self
                .session
                .registry()
                .entries()
                .map(|d| crate::index::PluginItem {
                    id: d.id.to_owned(),
                    name: d.name.to_owned(),
                    instrument: d.kind == lm_plugin::PluginKind::Instrument,
                    third_party: d.third_party,
                })
                .collect(),
            favorites: self.favorites(),
            user_tags: self.user_tags(),
        };
        // A new search replaces the last one
        if let Some(last) = self.search_request.take()
            && let Some((_, h)) = self.requests.iter().find(|(id, _)| *id == last)
        {
            h.cancel();
        }
        let id = self.request(
            |e| Finished::Entries(Err(e)),
            move |cancel, _| {
                cancel.check()?;
                Ok(Finished::Entries(Ok(index.search(&q, &cx))))
            },
        );
        self.search_request = Some(id);
        Pending::Waiting(id)
    }

    fn prepare_preview(&mut self, entry: EntryId) -> Pending<Preview> {
        let Some(known) = self.session.browser.known.get(&entry).cloned() else {
            return Pending::Ready(Err(err("port.preview.not_audio")));
        };
        let Some(path) = known.path.clone() else {
            return Pending::Ready(Err(err("port.preview.not_audio")));
        };
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let source = if known.page == BrowsePage::Presets && ext == "json" {
            Some(crate::preview::Source::Preset(path.clone()))
        } else if matches!(ext.as_str(), "sf2" | "sf3") {
            Some(crate::preview::Source::SoundFont(path.clone()))
        } else if matches!(ext.as_str(), "mid" | "midi" | "rmi") {
            Some(crate::preview::Source::Midi(path.clone(), self.default_soundfont()))
        } else {
            None
        };
        if source.is_none() && kind_of(&path) != EntryKind::Audio {
            return Pending::Ready(Err(err("port.preview.not_audio")));
        }
        let registry = self.session.registry().clone();
        let rate = self.session.spec().sample_rate;
        let id = self.next_request;
        self.next_request += 1;
        let handle = self.session.workers().start(move |cancel, progress| {
            if let Some(source) = source {
                // Played by its plugin in the worker: the sample and its waveform are the preview
                let mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                let key = blake3::hash(format!("preview:{}:{mtime}", path.display()).as_bytes()).to_hex().to_string();
                let resource = Resource::new("audio", &key)?;
                let data = Arc::new(crate::preview::render(&registry, &source, rate, None, cancel)?);
                let peaks = Arc::new(peaks_of_data(&data));
                let length = data.frames() as i64;
                return Ok((resource, data, peaks, length));
            }
            let key = lm_io::hash_file(&path, cancel, progress)?;
            let resource = Resource::new("audio", &key)?;
            let data = lm_io::load_file(&path, cancel, progress)?;
            let peaks = Arc::new(lm_io::peaks_of_file(&path, cancel, progress)?);
            let length = data.frames() as i64;
            Ok((resource, data, peaks, length))
        });
        self.preview_handles.push((id, entry, handle));
        Pending::Waiting(id)
    }

    fn preview_play(&mut self, entry: EntryId, _from: i64) {
        self.preview_stop();
        // The prepared preview of this entry (prepare_preview first)
        let Some(p) = self.preview_of.get(&entry).and_then(|k| self.previews.get(k)) else { return };
        let id = PreviewId(self.next_preview);
        self.next_preview += 1;
        let cmd = Command::Preview(PreviewCommand::Sample { id, data: p.data.clone(), gain: 1.0 });
        if self.session.engine().commands().send(cmd).is_ok() {
            self.playing_preview = Some(id);
        }
    }

    fn preview_stop(&mut self) {
        if let Some(id) = self.playing_preview.take() {
            let _ = self.session.engine().commands().send(Command::Preview(PreviewCommand::Stop { id }));
        }
    }

    fn import(&mut self, path: PathBuf) -> Pending<Resource> {
        let op = self.session.import(path);
        let id = self.next_request;
        self.next_request += 1;
        self.imports.push((op, id));
        Pending::Waiting(id)
    }

    fn plugins(&self) -> Vec<lm_port::PluginInfo> {
        self.session
            .registry()
            .descriptors()
            .map(|d| lm_port::PluginInfo {
                id: Arc::from(d.id),
                name: Arc::from(d.name),
                instrument: d.kind == lm_plugin::PluginKind::Instrument,
                category: Arc::from(match d.kind {
                    lm_plugin::PluginKind::Instrument => "synth",
                    lm_plugin::PluginKind::Effect => "effect",
                    lm_plugin::PluginKind::NoteEffect => "note",
                }),
                third_party: false,
                latency: 0,
                sidechain: self
                    .session
                    .registry()
                    .defaults(d.id)
                    .ok()
                    .and_then(|s| self.session.registry().ports(d.id, &s))
                    .is_some_and(|p| !p.sidechains.is_empty()),
            })
            .collect()
    }

    fn plugin_schema(&self, plugin: &str) -> Option<lm_state::NodeSchema> {
        self.session.registry().get(plugin).map(|d| d.schema())
    }

    fn ui(&self) -> &Value {
        self.session.ui_document().root()
    }

    fn set_ui(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()> {
        Pending::Ready(self.session.set_ui(&path, value))
    }

    fn midi_learn(&mut self, target: lm_port::MapTarget, scope: lm_port::MapScope) -> Pending<lm_port::LearnId> {
        Pending::Ready(Ok(self.session.midi_learn(target, scope)))
    }

    fn cancel_learn(&mut self, id: lm_port::LearnId) {
        self.session.cancel_learn(id);
    }

    fn learning(&self) -> Option<lm_port::LearnId> {
        self.session.learning()
    }

    fn unmap(&mut self, target: &lm_port::MapTarget, scope: lm_port::MapScope) -> Pending<()> {
        Pending::Ready(self.session.midi_unmap(target, scope))
    }

    fn drain_midi_ui(&mut self, out: &mut Vec<lm_port::MidiUiAction>) {
        self.session.drain_midi_ui(out);
    }

    fn midi_activity(&self, since: u64) -> (u64, Vec<lm_port::MidiActivity>) {
        self.session.midi_activity(since)
    }

    fn set_favorite(&mut self, entry: EntryId, on: bool) -> Pending<()> {
        let Some(path) = self.session.browser.known.get(&entry).and_then(|k| k.path.clone()) else {
            return Pending::Ready(Err(err("port.browse.unknown_entry")));
        };
        let text = path.display().to_string();
        let mut list = self.favorites();
        list.retain(|f| *f != text);
        if on {
            list.push(text);
        }
        let value = Value::list(list.into_iter().map(Value::text));
        Pending::Ready(self.session.set_preference(&[PathStep::key("favorites")], value))
    }
}

/// The audio devices (U17): ALSA's on Linux, CoreAudio's on macOS, and always the Dummy
fn list_devices(cancel: &lm_base::Cancel, progress: &lm_base::Progress) -> Result<Vec<DeviceInfo>> {
    #[allow(unused_mut)]
    let mut out = vec![DeviceInfo {
        id: Arc::from("dummy"),
        name: Arc::from("Dummy"),
        inputs: 2,
        outputs: 2,
        rates: vec![44_100, 48_000, 96_000, 192_000],
        details: Value::NULL,
    }];
    #[cfg(target_os = "linux")]
    for d in lm_io::list_devices(cancel, progress)? {
        out.push(DeviceInfo {
            id: Arc::from(d.name.as_str()),
            name: Arc::from(d.description.as_str()),
            inputs: d.capture.as_ref().map_or(0, |c| c.channels_max),
            outputs: d.playback.as_ref().map_or(0, |c| c.channels_max),
            rates: d.playback.as_ref().or(d.capture.as_ref()).map(|c| c.rates.clone()).unwrap_or_default(),
            details: Value::object([("busy", Value::bool(d.busy)), ("denied", Value::bool(d.denied))]),
        });
    }
    #[cfg(target_os = "macos")]
    for d in lm_io::coreaudio_list_devices(cancel, progress)? {
        out.push(DeviceInfo {
            id: Arc::from(d.name.as_str()),
            name: Arc::from(d.description.as_str()),
            inputs: d.capture.as_ref().map_or(0, |c| c.channels_max),
            outputs: d.playback.as_ref().map_or(0, |c| c.channels_max),
            rates: d.playback.as_ref().or(d.capture.as_ref()).map(|c| c.rates.clone()).unwrap_or_default(),
            details: Value::object([("busy", Value::bool(d.busy)), ("denied", Value::bool(d.denied))]),
        });
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (cancel, progress);
    }
    // JACK: one entry when a server runs (audio.system = "jack"; the id is the device name the session opens)
    #[cfg(feature = "jack")]
    for d in lm_io::jack_list_devices(cancel, progress)? {
        out.push(DeviceInfo {
            id: Arc::from(d.name.as_str()),
            name: Arc::from(d.description.as_str()),
            inputs: d.capture.as_ref().map_or(0, |c| c.channels_max),
            outputs: d.playback.as_ref().map_or(0, |c| c.channels_max),
            rates: d.playback.as_ref().or(d.capture.as_ref()).map(|c| c.rates.clone()).unwrap_or_default(),
            details: Value::object([("busy", Value::bool(d.busy)), ("denied", Value::bool(d.denied))]),
        });
    }
    Ok(out)
}
