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

/// One entry of the browser the port has given out, to find its children and its file again
#[derive(Clone, Debug)]
struct Known {
    page: BrowsePage,
    path: Option<PathBuf>,
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
    known: BTreeMap<EntryId, Known>,
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
}

impl std::fmt::Debug for LocalPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalPort").field("session", &self.session).finish_non_exhaustive()
    }
}

/// An id for a browser entry: stable for its page and path
fn entry_id(page: BrowsePage, key: &str) -> EntryId {
    let mut h = blake3::Hasher::new();
    h.update(format!("{page:?}").as_bytes());
    h.update(key.as_bytes());
    let mut b = [0u8; 8];
    b.copy_from_slice(&h.finalize().as_bytes()[..8]);
    EntryId(u64::from_le_bytes(b) >> 1)
}

const AUDIO_EXTENSIONS: [&str; 7] = ["wav", "flac", "ogg", "mp3", "aif", "aiff", "rf64"];

fn kind_of(path: &Path) -> EntryKind {
    if path.is_dir() {
        if path.join(crate::folder::PROJECT_FILE).exists() {
            return EntryKind::Project;
        }
        return EntryKind::Folder;
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        EntryKind::Audio
    } else if ext == "mid" || ext == "midi" {
        EntryKind::Midi
    } else {
        EntryKind::Other
    }
}

/// The entries of a folder (on a worker)
fn list_folder(
    page: BrowsePage,
    dir: &Path,
    parent: Option<EntryId>,
    hidden: bool,
    favorites: &[String],
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
        if page == BrowsePage::Samples && !matches!(kind, EntryKind::Folder | EntryKind::Audio) {
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
                factory: false,
                hidden: is_hidden,
                favorite: favorites.contains(&text),
                info: Value::object([("path", Value::text(text)), ("bytes", Value::int(size as i64))]),
            },
            path,
        ));
    }
    out
}

impl LocalPort {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            requests: Vec::new(),
            next_request: 1,
            known: BTreeMap::new(),
            previews: BTreeMap::new(),
            preview_of: BTreeMap::new(),
            search_request: None,
            imports: Vec::new(),
            preview_handles: Vec::new(),
            playing_preview: None,
            next_preview: 1,
            done: Vec::new(),
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

    /// The roots of a page, and the folder each lists
    fn roots(&self, page: BrowsePage) -> Vec<(BrowseEntry, Option<PathBuf>)> {
        let folder = |name: &str, path: PathBuf, factory: bool| {
            let text = path.display().to_string();
            (
                BrowseEntry {
                    id: entry_id(page, &text),
                    parent: None,
                    name: Arc::from(name),
                    kind: EntryKind::Folder,
                    factory,
                    hidden: false,
                    favorite: false,
                    info: Value::object([("path", Value::text(text))]),
                },
                Some(path),
            )
        };
        let prefs = self.session.preferences();
        let projects = PathBuf::from(prefs.text(&["projectsFolder"]).unwrap_or("."));
        match page {
            BrowsePage::Instruments | BrowsePage::Effects => {
                let want = if page == BrowsePage::Instruments {
                    lm_plugin::PluginKind::Instrument
                } else {
                    lm_plugin::PluginKind::Effect
                };
                self.session
                    .registry()
                    .descriptors()
                    .filter(|d| d.kind == want)
                    .map(|d| {
                        (
                            BrowseEntry {
                                id: entry_id(page, d.id),
                                parent: None,
                                name: Arc::from(d.name),
                                kind: EntryKind::Plugin,
                                factory: true,
                                hidden: false,
                                favorite: false,
                                info: Value::object([
                                    ("plugin", Value::text(d.id)),
                                    ("category", Value::text("internal")),
                                    ("third_party", Value::bool(false)),
                                ]),
                            },
                            None,
                        )
                    })
                    .collect()
            }
            BrowsePage::Samples => {
                let mut v: Vec<_> = prefs
                    .get(&["sampleFolders"])
                    .map(|l| l.items().filter_map(Value::as_text).map(PathBuf::from).collect::<Vec<_>>())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| {
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        folder(&name, p, false)
                    })
                    .collect();
                if v.is_empty() {
                    v.push(folder("Samples", projects.join("Samples"), false));
                }
                v
            }
            BrowsePage::Projects => vec![folder("Projects", projects, false)],
            BrowsePage::ProjectFiles => vec![folder("audio", self.session.project().folder.audio(), false)],
            BrowsePage::Presets => vec![folder("Presets", projects.join("Presets"), false)],
            BrowsePage::Favorites => self
                .favorites()
                .into_iter()
                .map(|f| {
                    let p = PathBuf::from(&f);
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(f);
                    let mut e = folder(&name, p.clone(), false);
                    e.0.kind = kind_of(&p);
                    e.0.favorite = true;
                    e
                })
                .collect(),
        }
    }

    fn remember(&mut self, page: BrowsePage, entries: &[(BrowseEntry, Option<PathBuf>)]) {
        for (e, p) in entries {
            self.known.insert(e.id, Known { page, path: p.clone() });
        }
    }
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
                        let page =
                            self.known.get(&e.parent.unwrap_or(EntryId(0))).map_or(BrowsePage::Samples, |k| k.page);
                        self.known.insert(e.id, Known { page, path });
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
        match self.session.resources().peaks(resource.key()) {
            Some(p) => Pending::Ready(Ok(peak_block(&p, level, from, to))),
            None => {
                // Not loaded yet: asked for; its loading shows on the resource's target
                self.session.request(resource);
                Pending::Ready(Err(err("port.peaks.loading")))
            }
        }
    }

    fn display(&self, _node: NodeId) -> Option<DisplayView> {
        None
    }

    fn preferences(&self) -> &Value {
        self.session.preferences().tree()
    }

    fn set_preference(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()> {
        Pending::Ready(self.session.set_preference(&path, value))
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
        let Some(parent) = parent else {
            let roots = self.roots(page);
            self.remember(page, &roots);
            return Pending::Ready(Ok(roots.into_iter().map(|(e, _)| e).collect()));
        };
        let Some(dir) = self.known.get(&parent).and_then(|k| k.path.clone()) else {
            return Pending::Ready(Err(err("port.browse.unknown_entry")));
        };
        let page = self.known.get(&parent).map_or(page, |k| k.page);
        let favorites = self.favorites();
        Pending::Waiting(self.request(
            |e| Finished::Entries(Err(e)),
            move |_, _| {
                let list = list_folder(page, &dir, Some(parent), show_hidden, &favorites);
                Ok(Finished::Entries(Ok(list.into_iter().map(|(e, _)| e).collect())))
            },
        ))
    }

    fn search(&mut self, query: &str) -> Pending<Vec<BrowseEntry>> {
        let q = query.to_lowercase();
        let roots: Vec<(BrowsePage, PathBuf)> = [BrowsePage::Samples, BrowsePage::Projects, BrowsePage::ProjectFiles]
            .into_iter()
            .flat_map(|page| self.roots(page).into_iter().filter_map(move |(_, p)| p.map(|p| (page, p))))
            .collect();
        let favorites = self.favorites();
        // A new search replaces the last one
        if let Some(last) = self.search_request.take()
            && let Some((_, h)) = self.requests.iter().find(|(id, _)| *id == last)
        {
            h.cancel();
        }
        let id = self.request(
            |e| Finished::Entries(Err(e)),
            move |cancel, _| {
                let mut found = Vec::new();
                let mut stack: Vec<(BrowsePage, PathBuf, usize)> = roots.into_iter().map(|(p, d)| (p, d, 0)).collect();
                while let Some((page, dir, depth)) = stack.pop() {
                    cancel.check()?;
                    for (e, path) in list_folder(page, &dir, None, false, &favorites) {
                        if e.kind == EntryKind::Folder && depth < 8 {
                            stack.push((page, path.clone(), depth + 1));
                        }
                        if e.name.to_lowercase().contains(&q) {
                            found.push(e);
                        }
                        if found.len() >= 500 {
                            return Ok(Finished::Entries(Ok(found)));
                        }
                    }
                }
                Ok(Finished::Entries(Ok(found)))
            },
        );
        self.search_request = Some(id);
        Pending::Waiting(id)
    }

    fn prepare_preview(&mut self, entry: EntryId) -> Pending<Preview> {
        let Some(path) = self.known.get(&entry).and_then(|k| k.path.clone()).filter(|p| kind_of(p) == EntryKind::Audio)
        else {
            return Pending::Ready(Err(err("port.preview.not_audio")));
        };
        let id = self.next_request;
        self.next_request += 1;
        let handle = self.session.workers().start(move |cancel, progress| {
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
                category: Arc::from(if d.kind == lm_plugin::PluginKind::Instrument { "synth" } else { "effect" }),
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

    fn set_favorite(&mut self, entry: EntryId, on: bool) -> Pending<()> {
        let Some(path) = self.known.get(&entry).and_then(|k| k.path.clone()) else {
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

/// The audio devices (U17): ALSA's on Linux, and always the Dummy
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
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (cancel, progress);
    }
    Ok(out)
}
