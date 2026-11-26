//! The browser's index (decision of 26/11, item 8): the files and presets of the user's and the factory's folders,
//! read in the background (an Operation of the session), incremental by the folders' mtime, kept in the user's data
//! folder, and searched at once. The folder tree is one more view of the same files.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lm_base::{Cancel, Result};
use lm_port::{BrowseEntry, BrowsePage, EntryId, EntryKind};
use lm_state::{Value, from_json, to_json};

use crate::browser::{file_type, plugin_category};

/// How deep the scan goes into a folder
const MAX_DEPTH: usize = 12;
/// The answers of a search
pub const MAX_RESULTS: usize = 500;

/// One file, folder-project or preset of the index
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub path: String,
    pub name: String,
    /// `info.type`: `"audio"`, `"midi"`, `"soundfont"`, `"preset"`, `"project"`...
    pub ty: String,
    /// The type filter it belongs to: `"sample"`, `"preset"`, `"midi"`, `"project"`
    pub group: String,
    pub factory: bool,
    pub mtime: i64,
    pub size: u64,
    /// A preset's plugin
    pub plugin: String,
    /// The folders it is in under its root, lower case
    pub tags: Vec<String>,
}

/// What the scan knows about a folder, to skip it when it did not change
#[derive(Clone, Debug, PartialEq, Eq)]
struct Dir {
    mtime: i64,
    subdirs: Vec<String>,
}

/// The whole index
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub items: Vec<Item>,
    dirs: BTreeMap<String, Dir>,
    /// Seconds since the epoch when the scan ended (0: never scanned)
    pub built: i64,
}

/// What to scan
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub dir: PathBuf,
    pub factory: bool,
    pub kind: RootKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootKind {
    /// Samples, banks, MIDI files and projects, in any depth
    Files,
    /// `<dir>/<plugin>/<name>.json`
    Presets,
}

fn mtime_of(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn group_of(ty: &str) -> Option<&'static str> {
    match ty {
        "audio" | "soundfont" | "sfz" | "patch" | "gig" | "nam" | "drumsynth" | "voc" => Some("sample"),
        "midi" => Some("midi"),
        "preset" => Some("preset"),
        "project" => Some("project"),
        _ => None,
    }
}

fn tags_of(root: &Path, dir: &Path) -> Vec<String> {
    dir.strip_prefix(root)
        .map(|rel| rel.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect())
        .unwrap_or_default()
}

impl Index {
    /// Scans the roots, reusing what `prev` has for the folders whose mtime did not change. Cancellable
    pub fn scan(roots: &[Root], prev: &Index, cancel: &Cancel) -> Result<Index> {
        let mut by_dir: HashMap<&str, Vec<&Item>> = HashMap::new();
        for it in &prev.items {
            if let Some(parent) = Path::new(&it.path).parent().and_then(Path::to_str) {
                by_dir.entry(parent).or_default().push(it);
            }
        }
        let mut out = Index::default();
        for root in roots {
            cancel.check()?;
            match root.kind {
                RootKind::Files => {
                    walk(&root.dir, &root.dir, root.factory, 0, prev, &by_dir, &mut out, cancel)?;
                }
                RootKind::Presets => scan_presets(root, &mut out, cancel)?,
            }
        }
        out.items.sort_by(|a, b| a.path.cmp(&b.path));
        out.items.dedup_by(|a, b| a.path == b.path);
        out.built =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        Ok(out)
    }

    // ---- the file ----

    pub fn to_value(&self) -> Value {
        let items = self.items.iter().map(|i| {
            Value::object([
                ("path", Value::text(i.path.as_str())),
                ("name", Value::text(i.name.as_str())),
                ("type", Value::text(i.ty.as_str())),
                ("factory", Value::bool(i.factory)),
                ("mtime", Value::int(i.mtime)),
                ("size", Value::int(i.size as i64)),
                ("plugin", Value::text(i.plugin.as_str())),
                ("tags", Value::list(i.tags.iter().map(|t| Value::text(t.as_str())))),
            ])
        });
        let dirs = self.dirs.iter().map(|(p, d)| {
            Value::object([
                ("path", Value::text(p.as_str())),
                ("mtime", Value::int(d.mtime)),
                ("subdirs", Value::list(d.subdirs.iter().map(|s| Value::text(s.as_str())))),
            ])
        });
        Value::object([
            ("version", Value::int(1)),
            ("built", Value::int(self.built)),
            ("items", Value::list(items)),
            ("dirs", Value::list(dirs)),
        ])
    }

    /// An index read back; anything that does not read is an empty index (it is made again)
    pub fn from_value(v: &Value) -> Index {
        let text = |x: &Value, k: &str| x.get(k).and_then(Value::as_text).unwrap_or("").to_owned();
        if v.get("version").and_then(Value::as_int) != Some(1) {
            return Index::default();
        }
        let items = v
            .get("items")
            .map(|l| {
                l.items()
                    .filter_map(|x| {
                        let ty = text(x, "type");
                        let group = group_of(&ty)?.to_owned();
                        Some(Item {
                            path: text(x, "path"),
                            name: text(x, "name"),
                            ty,
                            group,
                            factory: x.get("factory").and_then(Value::as_bool).unwrap_or(false),
                            mtime: x.get("mtime").and_then(Value::as_int).unwrap_or(0),
                            size: x.get("size").and_then(Value::as_int).unwrap_or(0).max(0) as u64,
                            plugin: text(x, "plugin"),
                            tags: x
                                .get("tags")
                                .map(|t| t.items().filter_map(Value::as_text).map(str::to_owned).collect())
                                .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let dirs = v
            .get("dirs")
            .map(|l| {
                l.items()
                    .map(|x| {
                        (
                            text(x, "path"),
                            Dir {
                                mtime: x.get("mtime").and_then(Value::as_int).unwrap_or(0),
                                subdirs: x
                                    .get("subdirs")
                                    .map(|t| t.items().filter_map(Value::as_text).map(str::to_owned).collect())
                                    .unwrap_or_default(),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Index { items, dirs, built: v.get("built").and_then(Value::as_int).unwrap_or(0) }
    }

    pub fn load(file: &Path) -> Index {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|t| from_json(&t).ok())
            .map(|v| Index::from_value(&v))
            .unwrap_or_default()
    }

    /// Writes the index (to a temporary file, then renamed)
    pub fn save(&self, file: &Path) -> Result<()> {
        let io = |e: std::io::Error| lm_base::Error::new("session.browser.index_write").arg("problem", e.to_string());
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, to_json(&self.to_value())?.as_bytes()).map_err(io)?;
        std::fs::rename(&tmp, file).map_err(io)
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    root: &Path,
    dir: &Path,
    factory: bool,
    depth: usize,
    prev: &Index,
    by_dir: &HashMap<&str, Vec<&Item>>,
    out: &mut Index,
    cancel: &Cancel,
) -> Result<()> {
    cancel.check()?;
    let key = dir.display().to_string();
    let mtime = mtime_of(dir);
    let mut subdirs = Vec::new();
    match prev.dirs.get(&key) {
        // Not changed since the last scan: what it had, without reading it
        Some(d) if d.mtime == mtime && mtime != 0 => {
            if let Some(items) = by_dir.get(key.as_str()) {
                out.items.extend(items.iter().filter(|i| i.factory == factory).map(|i| (*i).clone()));
            }
            subdirs = d.subdirs.clone();
        }
        _ => {
            let Ok(read) = std::fs::read_dir(dir) else { return Ok(()) };
            let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
            paths.sort();
            for path in paths {
                let name = name_of(&path);
                if name.starts_with('.') {
                    continue;
                }
                let tags = tags_of(root, dir);
                if path.is_dir() {
                    if path.join(crate::folder::PROJECT_FILE).exists() {
                        out.items.push(Item {
                            path: path.display().to_string(),
                            name,
                            ty: "project".into(),
                            group: "project".into(),
                            factory,
                            mtime: mtime_of(&path),
                            size: 0,
                            plugin: String::new(),
                            tags,
                        });
                    } else {
                        subdirs.push(path.display().to_string());
                    }
                } else if let Some(t) = file_type(&path)
                    && let Some(group) = group_of(t.name)
                {
                    let meta = std::fs::metadata(&path).ok();
                    let mut tags = tags;
                    tags.push(t.name.to_owned());
                    out.items.push(Item {
                        path: path.display().to_string(),
                        name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name),
                        ty: t.name.to_owned(),
                        group: group.to_owned(),
                        factory,
                        mtime: mtime_of(&path),
                        size: meta.map_or(0, |m| m.len()),
                        plugin: String::new(),
                        tags,
                    });
                }
            }
        }
    }
    out.dirs.insert(key, Dir { mtime, subdirs: subdirs.clone() });
    if depth < MAX_DEPTH {
        for sub in subdirs {
            walk(root, Path::new(&sub), factory, depth + 1, prev, by_dir, out, cancel)?;
        }
    }
    Ok(())
}

fn scan_presets(root: &Root, out: &mut Index, cancel: &Cancel) -> Result<()> {
    for plugin in std::fs::read_dir(&root.dir).into_iter().flatten().flatten() {
        cancel.check()?;
        let plugin_dir = plugin.path();
        let plugin_id = name_of(&plugin_dir);
        for file in std::fs::read_dir(&plugin_dir).into_iter().flatten().flatten() {
            let path = file.path();
            if path.extension().is_none_or(|e| e != "json") || !path.is_file() {
                continue;
            }
            out.items.push(Item {
                path: path.display().to_string(),
                name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                ty: "preset".into(),
                group: "preset".into(),
                factory: root.factory,
                mtime: mtime_of(&path),
                size: file.metadata().map_or(0, |m| m.len()),
                plugin: plugin_id.clone(),
                tags: vec![plugin_id.to_lowercase()],
            });
        }
    }
    Ok(())
}

// ---- search ----

/// A search text: words that must all be in the name or tags, phrases between quotes that must be there as they are,
/// `type:samples` (also instruments, effects, presets, midi, projects), `tag:drums` or `#drums`, `is:fav`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    terms: Vec<String>,
    groups: Vec<String>,
    tags: Vec<String>,
    fav: bool,
}

fn group_name(word: &str) -> Option<&'static str> {
    Some(match word {
        "instrument" | "instruments" => "instrument",
        "effect" | "effects" => "effect",
        "sample" | "samples" => "sample",
        "preset" | "presets" => "preset",
        "midi" => "midi",
        "project" | "projects" => "project",
        _ => return None,
    })
}

impl Query {
    pub fn parse(text: &str) -> Query {
        // (text, was between quotes)
        let mut tokens: Vec<(String, bool)> = Vec::new();
        let mut word = String::new();
        let mut quoted = false;
        for ch in text.chars() {
            match ch {
                '"' => {
                    if !word.is_empty() {
                        tokens.push((std::mem::take(&mut word), quoted));
                    }
                    quoted = !quoted;
                }
                c if c.is_whitespace() && !quoted => {
                    if !word.is_empty() {
                        tokens.push((std::mem::take(&mut word), false));
                    }
                }
                c => word.extend(c.to_lowercase()),
            }
        }
        if !word.trim().is_empty() {
            tokens.push((word.trim().to_owned(), quoted));
        }
        let mut q = Query::default();
        for (t, quoted) in tokens {
            let t = t.trim().to_owned();
            if t.is_empty() {
                continue;
            }
            if quoted {
                q.terms.push(t);
            } else if let Some(g) = t.strip_prefix("type:").and_then(group_name) {
                q.groups.push(g.to_owned());
            } else if let Some(g) = t.strip_prefix("tag:").or_else(|| t.strip_prefix('#')) {
                if !g.is_empty() {
                    q.tags.push(g.to_owned());
                }
            } else if t == "is:fav" || t == "fav:" {
                q.fav = true;
            } else {
                q.terms.push(t);
            }
        }
        q
    }

    /// No text and no filter: nothing to look for
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.groups.is_empty() && self.tags.is_empty() && !self.fav
    }

    fn text_matches(&self, hay: &str) -> bool {
        self.terms.iter().all(|t| hay.contains(t.as_str()))
    }

    /// The name alone (a plain text search over names)
    pub fn matches(&self, name: &str) -> bool {
        self.text_matches(&name.to_lowercase())
    }

    fn wants_group(&self, group: &str) -> bool {
        self.groups.is_empty() || self.groups.iter().any(|g| g == group)
    }

    fn wants_tags(&self, tags: &[String]) -> bool {
        self.tags.iter().all(|w| tags.iter().any(|t| t.to_lowercase().contains(w.as_str())))
    }
}

/// A plugin of the program, as the search sees it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginItem {
    pub id: String,
    pub name: String,
    pub instrument: bool,
    pub third_party: bool,
}

/// What a search needs besides the index
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub plugins: Vec<PluginItem>,
    pub favorites: Vec<String>,
    /// The user's tags by path (`tags` of the preferences)
    pub user_tags: BTreeMap<String, Vec<String>>,
}

pub fn page_of(group: &str) -> BrowsePage {
    match group {
        "preset" => BrowsePage::Presets,
        "project" => BrowsePage::Projects,
        "instrument" => BrowsePage::Instruments,
        "effect" => BrowsePage::Effects,
        _ => BrowsePage::Samples,
    }
}

/// An id for a browser entry: stable for its page and path
pub fn entry_id(page: BrowsePage, key: &str) -> EntryId {
    let mut h = blake3::Hasher::new();
    h.update(format!("{page:?}").as_bytes());
    h.update(key.as_bytes());
    let mut b = [0u8; 8];
    b.copy_from_slice(&h.finalize().as_bytes()[..8]);
    EntryId(u64::from_le_bytes(b) >> 1)
}

fn entry_of(item: &Item, favorite: bool, tags: &[String]) -> BrowseEntry {
    let page = page_of(&item.group);
    let kind = match item.group.as_str() {
        "preset" => EntryKind::Preset,
        "project" => EntryKind::Project,
        "midi" => EntryKind::Midi,
        _ if item.ty == "audio" => EntryKind::Audio,
        _ => EntryKind::Other,
    };
    let mut info = Value::object([
        ("path", Value::text(item.path.as_str())),
        ("type", Value::text(item.ty.as_str())),
        ("group", Value::text(item.group.as_str())),
        ("page", Value::text(format!("{page:?}"))),
        ("bytes", Value::int(item.size as i64)),
        ("tags", Value::list(tags.iter().map(|t| Value::text(t.as_str())))),
    ]);
    if !item.plugin.is_empty() {
        info = info.with("plugin", Value::text(item.plugin.as_str()));
    }
    if let Some(p) = crate::browser::opened_by(&item.ty) {
        info = info.with("opened_by", Value::text(p));
    }
    BrowseEntry {
        id: entry_id(page, &item.path),
        parent: None,
        name: Arc::from(item.name.as_str()),
        kind,
        factory: item.factory,
        hidden: false,
        favorite,
        info,
    }
}

/// How well a name answers the first word: 0 starts with it, 1 has it at a word, 2 anywhere
fn rank(q: &Query, name: &str) -> u8 {
    let name = name.to_lowercase();
    match q.terms.first() {
        None => 0,
        Some(t) if name.starts_with(t.as_str()) => 0,
        Some(t)
            if name.contains(&format!(" {t}"))
                || name.contains(&format!("_{t}"))
                || name.contains(&format!("-{t}")) =>
        {
            1
        }
        _ => 2,
    }
}

impl Index {
    /// The entries that answer `q` (at most [`MAX_RESULTS`]): the better names first, the user's before the
    /// factory's, then by name. A query with nothing in it finds nothing
    pub fn search(&self, q: &Query, cx: &Context) -> Vec<BrowseEntry> {
        if q.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(u8, bool, String, BrowseEntry)> = Vec::new();
        for it in &self.items {
            if !q.wants_group(&it.group) {
                continue;
            }
            let favorite = cx.favorites.iter().any(|f| *f == it.path);
            if q.fav && !favorite {
                continue;
            }
            let mut tags = it.tags.clone();
            if let Some(user) = cx.user_tags.get(&it.path) {
                tags.extend(user.iter().cloned());
            }
            if !q.wants_tags(&tags) {
                continue;
            }
            let hay = format!("{} {} {}", it.name, it.plugin, tags.join(" ")).to_lowercase();
            if !q.text_matches(&hay) {
                continue;
            }
            found.push((rank(q, &it.name), it.factory, it.name.to_lowercase(), entry_of(it, favorite, &tags)));
        }
        if !q.fav {
            for p in &cx.plugins {
                let group = if p.instrument { "instrument" } else { "effect" };
                let (cat, _) = if p.third_party {
                    crate::browser::THIRD_PARTY_CATEGORY
                } else {
                    plugin_category(&p.id, p.instrument)
                };
                let tags = vec![cat.to_owned()];
                if !q.wants_group(group) || !q.wants_tags(&tags) {
                    continue;
                }
                if !q.text_matches(&format!("{} {} {}", p.name, p.id, cat).to_lowercase()) {
                    continue;
                }
                let page = page_of(group);
                found.push((
                    rank(q, &p.name),
                    false,
                    p.name.to_lowercase(),
                    BrowseEntry {
                        id: entry_id(page, &p.id),
                        parent: None,
                        name: Arc::from(p.name.as_str()),
                        kind: EntryKind::Plugin,
                        factory: !p.third_party,
                        hidden: false,
                        favorite: false,
                        info: Value::object([
                            ("plugin", Value::text(p.id.as_str())),
                            ("category", Value::text(cat)),
                            ("group", Value::text(group)),
                            ("page", Value::text(format!("{page:?}"))),
                            ("third_party", Value::bool(p.third_party)),
                        ]),
                    },
                ));
            }
        }
        found.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
        found.into_iter().take(MAX_RESULTS).map(|(_, _, _, e)| e).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, name: &str, ty: &str, factory: bool, tags: &[&str]) -> Item {
        Item {
            path: path.into(),
            name: name.into(),
            ty: ty.into(),
            group: group_of(ty).unwrap().into(),
            factory,
            mtime: 1,
            size: 10,
            plugin: if ty == "preset" { "monstro".into() } else { String::new() },
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    fn index() -> Index {
        Index {
            items: vec![
                item("/u/drums/Kick Hard.wav", "Kick Hard", "audio", false, &["drums", "audio"]),
                item("/f/drums/Kick Soft.wav", "Kick Soft", "audio", true, &["drums", "audio"]),
                item("/f/bass/riff.mid", "riff", "midi", true, &["bass", "midi"]),
                item("/u/p/Wobble.json", "Wobble", "preset", false, &["monstro"]),
            ],
            ..Index::default()
        }
    }

    #[test]
    fn words_phrases_and_filters() {
        let ix = index();
        let cx = Context::default();
        let names =
            |s: &str| ix.search(&Query::parse(s), &cx).into_iter().map(|e| e.name.to_string()).collect::<Vec<_>>();
        assert_eq!(names("kick"), vec!["Kick Hard", "Kick Soft"]);
        assert_eq!(names("\"kick soft\""), vec!["Kick Soft"]);
        assert_eq!(names("kick hard"), vec!["Kick Hard"]);
        assert_eq!(names("type:midi"), vec!["riff"]);
        assert_eq!(names("type:presets wob"), vec!["Wobble"]);
        assert_eq!(names("#bass"), vec!["riff"]);
        assert_eq!(names("tag:drums soft"), vec!["Kick Soft"]);
        assert!(names("").is_empty());
        assert!(names("zzz").is_empty());
    }

    #[test]
    fn favourites_and_user_tags() {
        let ix = index();
        let mut cx = Context { favorites: vec!["/f/bass/riff.mid".into()], ..Context::default() };
        cx.user_tags.insert("/u/drums/Kick Hard.wav".into(), vec!["punchy".into()]);
        let q = |s: &str| ix.search(&Query::parse(s), &cx);
        assert_eq!(q("is:fav").len(), 1);
        assert!(q("is:fav")[0].favorite);
        assert_eq!(q("#punchy").len(), 1);
    }

    #[test]
    fn plugins_are_found_by_type() {
        let ix = Index::default();
        let cx = Context {
            plugins: vec![
                PluginItem {
                    id: "compressor".into(),
                    name: "Compressor".into(),
                    instrument: false,
                    third_party: false,
                },
                PluginItem { id: "monstro".into(), name: "Monstro".into(), instrument: true, third_party: false },
            ],
            ..Context::default()
        };
        let r = ix.search(&Query::parse("type:effects"), &cx);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].info.get("category").and_then(Value::as_text), Some("dynamics"));
        assert_eq!(ix.search(&Query::parse("type:instruments mon"), &cx).len(), 1);
    }

    #[test]
    fn the_index_round_trips() {
        let mut ix = index();
        ix.dirs.insert("/u/drums".into(), Dir { mtime: 5, subdirs: vec!["/u/drums/x".into()] });
        ix.built = 9;
        assert_eq!(Index::from_value(&ix.to_value()), ix);
        assert_eq!(Index::from_value(&Value::NULL), Index::default());
    }

    #[test]
    fn a_scan_is_incremental() {
        let dir = std::env::temp_dir().join(format!("lm-index-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("drums")).unwrap();
        std::fs::write(dir.join("drums/Kick.wav"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("presets/monstro")).unwrap();
        std::fs::write(dir.join("presets/monstro/A.json"), b"{}").unwrap();
        let roots = vec![
            Root { dir: dir.clone(), factory: false, kind: RootKind::Files },
            Root { dir: dir.join("presets"), factory: true, kind: RootKind::Presets },
        ];
        let cancel = Cancel::new();
        let first = Index::scan(&roots, &Index::default(), &cancel).unwrap();
        assert!(first.items.iter().any(|i| i.name == "Kick" && i.tags.contains(&"drums".to_string())));
        assert!(first.items.iter().any(|i| i.group == "preset" && i.plugin == "monstro" && i.factory));
        assert!(!first.items.iter().any(|i| i.name == "notes"));
        // nothing changed: the same
        let again = Index::scan(&roots, &first, &cancel).unwrap();
        assert_eq!(again.items, first.items);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
