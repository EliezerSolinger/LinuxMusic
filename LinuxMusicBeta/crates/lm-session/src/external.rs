//! Third-party plugins in the session (I-9.6, I-9.4): the scan as an operation, its results and the block list in
//! the preferences, the catalog growing with what it found, the helper of each running node, the plugin's own state
//! and parameter changes coming back into the document, and the notices of a helper that crashed or hung with their
//! "recreate".
//!
//! A child module of `session`, so it works on the session's own fields.
//!
//! - **The catalog only grows** while the session runs: a plugin the project uses never leaves it (the document,
//!   the compiler and the open history keep naming it). A new scan adds plugins; one that a scan no longer finds
//!   stays until the next start. The document goes on with the wider registry (`Document::widen`), the compiler
//!   with the wider catalog (`Compiler::set_catalog`): nothing is reopened, the history stays.
//! - **The helper of a node** is reached through the proxy the build made (`Proxy::hosted`); the session keeps a
//!   weak handle, so the helper lives exactly as long as its processor.
//! - **The plugin's own state** (`"chunk"`): asked of the helper in an operation when the plugin says it changed
//!   (`StateChanged`) and before each save, and written into the document as an `Origin::Plugin` transaction. A
//!   change of the chunk alone never builds the processor again (`lm_plugin::CHUNK_KEY`): when the document's chunk
//!   differs from the one the plugin has (an undo, a preset), the session gives it to the running plugin.
//! - **The plugin's own parameter changes** (its window) are one gesture (I-9.4): a plugin that says when a knob is
//!   grabbed and let go (VST3's begin and end edit) gets exactly that gesture, committed at the end (an undo step,
//!   automation recorded as one); one that does not is committed 300 ms after the last change. A begin whose end
//!   never came (a lost message) is closed after 2 s of silence.
//! - **The plugin's window** (I-22: a third-party plugin opens its own): `plugin.show_window {node}` asks the node's
//!   helper to open it (in an operation: a plugin's window may take seconds); a plugin that has none (or a machine
//!   without a window system) says why in a notice, and the generic editor stays.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use lm_base::{Error, HostTime, Label, Result};
use lm_compile::NodeKey;
use lm_doc::GestureId;
use lm_plugin::{CHUNK_KEY, External, Registry};
use lm_plugin_proxy::{
    Blocked, Group, HostConfig, Hosted, Proxy, Saved, Scanned, WindowCommand, external_in, plugin_id, scanned_from,
};
use lm_port::{ActionId, Severity, Target};
use lm_render::{PluginRequest, Processor};
use lm_state::{NodeId, PathStep, Value};

use super::Session;
use crate::actions::ActionSpec;

/// How long a probe of one plugin may take during a scan
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeouts of one plugin, across scans, after which it is blocked until the user lifts it. A crash blocks at once
/// (it repeats); a timeout may be a cold disk or a busy machine, so the first one only counts and the plugin is
/// probed again by the next scan
const TIMEOUT_STRIKES: i64 = 2;
/// How long the helper may take to give its state
const STATE_TIMEOUT: Duration = Duration::from_secs(5);
/// The plugin's own parameter changes closer than this are one undo step (I-9.4), when it gives no gesture edges
const PARAM_GROUP: Duration = Duration::from_millis(300);
/// A gesture whose end never came is closed after this long without a change
const GESTURE_STALE: Duration = Duration::from_secs(2);
/// How long a plugin's window may take to open
const WINDOW_TIMEOUT: Duration = Duration::from_secs(20);

/// One running helper, as the session knows it
struct Running {
    hosted: Weak<Hosted>,
    /// The chunk (hex) the plugin has now, as far as the session knows
    chunk: Option<String>,
}

/// The session's side of the third-party plugins
#[derive(Default)]
pub(crate) struct ExternalCore {
    /// `None`: no helper program (third-party plugins are not offered)
    config: Option<Arc<HostConfig>>,
    /// Every external plugin made in this run, by id; the catalog is the internal plugins and these
    made: BTreeMap<String, External>,
    running: BTreeMap<NodeKey, Running>,
    /// The plugin's own parameter changes waiting for the next pump, and the gesture they go into
    params: Vec<(NodeId, &'static str, f64)>,
    gesture: Option<(GestureId, HostTime)>,
    /// Gestures the plugins' windows began and have not ended (a begin and its end are counted)
    open_edits: u32,
    /// A window's gesture ended: the pump commits the gesture once it has the changes that came with it
    edit_ended: bool,
    /// A save waiting for the plugins' states
    pub(crate) save_waiting: bool,
    /// The plugins' states were just taken: the save goes on
    states_fresh: bool,
}

/// The session's actions about third-party plugins (chained into `actions::all`)
pub(crate) static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "plugin.recreate",
        label: "action.plugin.recreate",
        run: |s, p| {
            let id = p.get("node").and_then(Value::as_text).and_then(|t| t.parse::<NodeId>().ok());
            s.recreate_plugin(id.ok_or_else(|| Error::new("action.params"))?)
        },
        enabled: |_| true,
    },
    ActionSpec {
        id: "plugin.show_window",
        label: "action.plugin.show_window",
        run: |s, p| {
            let id = p.get("node").and_then(Value::as_text).and_then(|t| t.parse::<NodeId>().ok());
            s.plugin_window(id.ok_or_else(|| Error::new("action.params"))?, true)
        },
        enabled: |s| s.external.config.is_some(),
    },
    ActionSpec {
        id: "plugin.hide_window",
        label: "action.plugin.hide_window",
        run: |s, p| {
            let id = p.get("node").and_then(Value::as_text).and_then(|t| t.parse::<NodeId>().ok());
            s.plugin_window(id.ok_or_else(|| Error::new("action.params"))?, false)
        },
        enabled: |s| s.external.config.is_some(),
    },
    ActionSpec {
        id: "plugin.group",
        label: "action.plugin.group",
        run: |s, p| {
            let id = p.get("plugin").and_then(Value::as_text).ok_or_else(|| Error::new("action.params"))?;
            s.set_plugin_grouped(id, p.get("on").and_then(Value::as_bool).unwrap_or(true))
        },
        enabled: |s| s.external.config.is_some(),
    },
    ActionSpec {
        id: "plugin.unblock",
        label: "action.plugin.unblock",
        run: |s, p| s.unblock_plugins(p.get("format").and_then(Value::as_text), p.get("uri").and_then(Value::as_text)),
        enabled: |s| s.external.config.is_some(),
    },
    ActionSpec {
        id: "plugin.scan",
        label: "action.plugin.scan",
        run: |s, p| {
            let format = p.get("format").and_then(Value::as_text).unwrap_or("lv2").to_owned();
            s.scan_plugins(&format).map(|_| ())
        },
        enabled: |s| s.external.config.is_some(),
    },
];

fn plugins_pref(prefs: &crate::prefs::Preferences) -> Value {
    prefs.get(&["plugins"]).cloned().unwrap_or_else(Value::empty_object)
}

/// The formats a plugin folder can be set for
pub const FOLDER_FORMATS: [&str; 4] = ["lv2", "clap", "vst3", "ladspa"];
/// The most folders kept per format, and the longest folder name (a preference is the user's file: bounded)
const MAX_FOLDERS: usize = 64;
const MAX_FOLDER_LEN: usize = 4096;

/// The folders the user set for each format (`plugins.folders.<format>`), what is not a folder name dropped
fn folders_of(prefs: &Value) -> std::collections::BTreeMap<String, Vec<std::path::PathBuf>> {
    let mut out = std::collections::BTreeMap::new();
    let Some(folders) = prefs.get("folders") else { return out };
    for format in FOLDER_FORMATS {
        let list: Vec<std::path::PathBuf> = list_of(folders, format)
            .iter()
            .filter_map(Value::as_text)
            .map(str::trim)
            .filter(|t| !t.is_empty() && t.len() <= MAX_FOLDER_LEN && !t.contains('\0'))
            .take(MAX_FOLDERS)
            .map(|t| match t.strip_prefix("~/") {
                Some(rest) => std::env::var_os("HOME").map_or_else(|| t.into(), |h| std::path::Path::new(&h).join(rest)),
                None => t.into(),
            })
            .collect();
        if !list.is_empty() {
            out.insert(format.to_owned(), list);
        }
    }
    out
}

fn list_of(v: &Value, key: &str) -> Vec<Value> {
    v.get(key).map(|l| l.items().cloned().collect()).unwrap_or_default()
}

fn blocked_value(b: &Blocked) -> Value {
    Value::object([
        ("format", Value::text(b.format.as_str())),
        ("uri", Value::text(b.uri.as_str())),
        ("reason", Value::text(b.reason.key())),
    ])
}

fn strikes_of(b: &Value) -> i64 {
    b.get("strikes").and_then(Value::as_int).unwrap_or(1)
}

fn is_timeout(b: &Value) -> bool {
    b.get("reason").and_then(Value::as_text) == Some("plugin_host.timeout")
}

/// A block entry that is only a count so far: one timeout (no crash), below the strike limit
fn soft(b: &Value) -> bool {
    is_timeout(b) && strikes_of(b) < TIMEOUT_STRIKES
}

fn same_plugin(a: &Value, b: &Value) -> bool {
    a.get("uri") == b.get("uri") && a.get("format") == b.get("format")
}

/// The ids of `format` a scan does not try: the hard blocks (a crash, or enough timeouts); a soft one is tried again
fn skipped_uris(blocked: &[Value], format: &str) -> Vec<String> {
    blocked
        .iter()
        .filter(|b| b.get("format").and_then(Value::as_text) == Some(format) && !soft(b))
        .filter_map(|b| b.get("uri").and_then(Value::as_text).map(str::to_owned))
        .collect()
}

/// The block list after a scan of `format`: a plugin the scan found working leaves it (it was only slow once); one
/// that failed again is there with its strikes counted and its latest reason
fn merge_blocked(existing: Vec<Value>, new: &[Value], found: &[Scanned], format: &str) -> Vec<Value> {
    let mut out: Vec<Value> = existing
        .into_iter()
        .filter(|b| {
            !(b.get("format").and_then(Value::as_text) == Some(format)
                && found.iter().any(|s| b.get("uri").and_then(Value::as_text) == Some(s.uri.as_str())))
        })
        .collect();
    for n in new {
        match out.iter().position(|x| same_plugin(x, n)) {
            Some(i) => {
                let strikes = strikes_of(&out[i]) + 1;
                out[i] = n.with("strikes", Value::int(strikes));
            }
            None => out.push(n.with("strikes", Value::int(1))),
        }
    }
    out
}

/// The block list without what `format` and `uri` name (`None`: every entry of the format; no format: all)
fn unblock(blocked: Vec<Value>, format: Option<&str>, uri: Option<&str>) -> Vec<Value> {
    blocked
        .into_iter()
        .filter(|b| {
            let f = format.is_none_or(|f| b.get("format").and_then(Value::as_text) == Some(f));
            let u = uri.is_none_or(|u| b.get("uri").and_then(Value::as_text) == Some(u));
            !(f && u)
        })
        .collect()
}

/// The plugin node a node key belongs to (its instrument or insert)
fn plugin_node(key: NodeKey) -> Option<NodeId> {
    match key {
        NodeKey::Instrument(id) | NodeKey::Insert(id) | NodeKey::NoteEffect(id) => Some(id),
        _ => None,
    }
}

impl Session {
    /// Finds the helper program and puts the plugins of the last scans (the preferences) in the catalog
    pub(crate) fn external_start(&mut self) {
        let config = HostConfig::find();
        if lm_plugin_proxy::hosted::helper_exists(&config) {
            self.set_plugin_host(config);
        }
    }

    /// Uses this helper program (tests, a packaged build that knows where it is) and the plugins of the last scans
    pub fn set_plugin_host(&mut self, mut config: HostConfig) {
        let prefs = plugins_pref(&self.preferences);
        config.paths.extend(list_of(&prefs, "paths").iter().filter_map(Value::as_text).map(std::path::PathBuf::from));
        config.format_paths = folders_of(&prefs);
        let config = Arc::new(config);
        self.external.config = Some(Arc::clone(&config));
        let scanned: Vec<Scanned> = list_of(&prefs, "scanned").iter().filter_map(scanned_from).collect();
        if let Err(e) = self.add_externals(&scanned) {
            self.notices.error(&e, None, None);
        }
    }

    /// The folders of the preferences go to the helper's configuration: the next scan and the next plugin started
    /// search them (a plugin already running is not moved)
    pub(crate) fn refresh_plugin_folders(&mut self) {
        let Some(config) = self.external.config.clone() else { return };
        let folders = folders_of(&plugins_pref(&self.preferences));
        if config.format_paths != folders {
            let mut next = (*config).clone();
            next.format_paths = folders;
            self.external.config = Some(Arc::new(next));
        }
    }

    /// Scans a format (`"lv2"`, `"clap"`, `"vst3"`, `"ladspa"`; `"test"`: the helper's test plugins) in an
    /// operation: each plugin probed in a helper of its own, the ones that crash or hang it blocked. The plugins
    /// found join the catalog; the scan and the block list go to the preferences
    pub fn scan_plugins(&mut self, format: &str) -> Result<lm_base::OperationId> {
        let config = self.external.config.clone().ok_or_else(|| Error::new("plugin_host.missing"))?;
        let prefs = plugins_pref(&self.preferences);
        let blocked = skipped_uris(&list_of(&prefs, "blocked"), format);
        let format = format.to_owned();
        Ok(self.start(
            Label::new("plugins.scanning"),
            Target::Project,
            true,
            "plugins.scan",
            move |cancel, progress| {
                let r = lm_plugin_proxy::scan(&config, &format, &[], &blocked, PROBE_TIMEOUT, cancel, progress)?;
                Ok(Value::object([
                    ("format", Value::text(format.as_str())),
                    ("plugins", Value::list(r.plugins.iter().map(Scanned::to_value))),
                    ("blocked", Value::list(r.blocked.iter().map(blocked_value))),
                    ("skipped", Value::int(r.skipped.len() as i64)),
                ]))
            },
        ))
    }

    /// Lifts blocks: the plugin `format`/`uri` names, every plugin of `format` when there is no `uri`, all of them when
    /// there is no `format` either. The next scan tries them again
    pub fn unblock_plugins(&mut self, format: Option<&str>, uri: Option<&str>) -> Result<()> {
        let prefs = plugins_pref(&self.preferences);
        let blocked = list_of(&prefs, "blocked");
        let kept = unblock(blocked.clone(), format, uri);
        if kept.len() == blocked.len() {
            return Ok(());
        }
        self.set_preference(&[PathStep::key("plugins")], prefs.with("blocked", Value::list(kept)))
    }

    pub(crate) fn scan_finished(&mut self, report: &Value) -> Result<()> {
        let format = report.get("format").and_then(Value::as_text).unwrap_or("");
        let found: Vec<Scanned> = list_of(report, "plugins").iter().filter_map(scanned_from).collect();
        let new_blocked = list_of(report, "blocked");
        // The preferences: this format's plugins replaced by the scan's, the block list grown
        let prefs = plugins_pref(&self.preferences);
        let mut scanned: Vec<Value> = list_of(&prefs, "scanned")
            .into_iter()
            .filter(|p| p.get("format").and_then(Value::as_text) != Some(format))
            .collect();
        scanned.extend(found.iter().map(Scanned::to_value));
        let blocked = merge_blocked(list_of(&prefs, "blocked"), &new_blocked, &found, format);
        let tree = prefs.with("scanned", Value::list(scanned)).with("blocked", Value::list(blocked));
        self.set_preference(&[PathStep::key("plugins")], tree)?;
        self.add_externals(&found)?;
        self.notices.push(
            "plugins.scan.done",
            vec![
                ("found", Value::int(found.len() as i64)),
                ("blocked", Value::int(new_blocked.len() as i64)),
                // Of those, the ones that only timed out once: the next scan tries them again
                ("retry", Value::int(new_blocked.iter().filter(|b| soft(b)).count() as i64)),
                ("format", Value::text(format)),
            ],
            Severity::Info,
            None,
            Vec::new(),
        );
        Ok(())
    }

    /// The catalog with these plugins too; the document and the compiler go on with it
    fn add_externals(&mut self, scanned: &[Scanned]) -> Result<()> {
        let Some(config) = self.external.config.clone() else { return Ok(()) };
        let mut added = false;
        let grouped: Vec<String> = list_of(&plugins_pref(&self.preferences), "grouped")
            .iter()
            .filter_map(Value::as_text)
            .map(str::to_owned)
            .collect();
        for s in scanned {
            let id = plugin_id(&s.format, &s.uri);
            if !self.external.made.contains_key(&id) && self.plugins.kind(&id).is_none() {
                // A plugin the user asked to group runs all its instances in one helper process
                let group = grouped.contains(&id).then(Group::new);
                self.external.made.insert(id, external_in(s, Arc::clone(&config), None, group));
                added = true;
            }
        }
        if !added {
            return Ok(());
        }
        let statics: Vec<&'static lm_plugin::Descriptor> = self.plugins.descriptors().collect();
        let registry = Registry::with_external(&statics, self.external.made.values().cloned().collect())?;
        let schemas = Arc::new(registry.project_schemas()?);
        self.document.widen(Arc::clone(&schemas), Some(registry.ports_fn()))?;
        self.compiler.set_catalog(registry.clone(), Arc::clone(&schemas));
        self.plugins = registry;
        self.schemas = schemas;
        Ok(())
    }

    /// A processor was built for `node`: if it is a third-party plugin's proxy, its helper is kept
    pub(crate) fn external_built(&mut self, node: NodeKey, p: &mut Box<dyn Processor>) {
        let Some(proxy) = p.as_any_mut().downcast_mut::<Proxy>() else { return };
        let Some(hosted) = proxy.hosted() else { return };
        // The plugin was built from the document's chunk (or from its defaults when it could not take it)
        let chunk = plugin_node(node).and_then(|id| self.doc_chunk(id));
        self.external.running.insert(node, Running { hosted: Arc::downgrade(&hosted), chunk });
    }

    fn doc_chunk(&self, id: NodeId) -> Option<String> {
        let node = self.document.find(id)?;
        node.get("state")?.get(CHUNK_KEY)?.as_text().map(str::to_owned)
    }

    fn hosted(&self, key: NodeKey) -> Option<Arc<Hosted>> {
        self.external.running.get(&key)?.hosted.upgrade()
    }

    /// A third-party plugin's request (from the telemetry)
    pub(crate) fn external_request(&mut self, key: NodeKey, request: PluginRequest) {
        let Some(id) = plugin_node(key) else { return };
        match request {
            PluginRequest::Error(e) if e.0.starts_with("plugin_host.") => {
                let node = Value::text(id.to_string());
                self.notices.push(
                    e.0,
                    vec![("node", node.clone())],
                    Severity::Error,
                    Some(Target::Node(id)),
                    vec![(ActionId::from("plugin.recreate"), Value::object([("node", node)]))],
                );
            }
            PluginRequest::StateChanged => self.fetch_states(vec![key], false),
            PluginRequest::GestureBegin { .. } => {
                self.external.open_edits = self.external.open_edits.saturating_add(1);
                self.external.edit_ended = false;
            }
            PluginRequest::GestureEnd { .. } => {
                self.external.open_edits = self.external.open_edits.saturating_sub(1);
                self.external.edit_ended = self.external.open_edits == 0;
            }
            PluginRequest::ParamChanged { index, value } => {
                let Some(plugin) = self.document.find(id).and_then(|n| n.get("plugin")?.as_text().map(str::to_owned))
                else {
                    return;
                };
                if !self.external.made.contains_key(&plugin) {
                    return;
                }
                let Some(schema) = self.plugins.schema(&plugin) else { return };
                if let Some(p) = schema.params.get(index as usize) {
                    self.external.params.push((id, p.key, f64::from(value)));
                }
            }
            _ => {}
        }
    }

    /// Asks the helpers of `keys` for their state, in an operation; `then_save`: the save waiting goes on after
    fn fetch_states(&mut self, keys: Vec<NodeKey>, then_save: bool) {
        let helpers: Vec<(NodeKey, Arc<Hosted>)> =
            keys.into_iter().filter_map(|k| Some((k, self.hosted(k)?))).filter(|(_, h)| !h.is_dead()).collect();
        if helpers.is_empty() && !then_save {
            return;
        }
        let kind = if then_save { "plugins.state.save" } else { "plugins.state" };
        self.start(Label::new("plugins.state"), Target::Project, false, kind, move |_, _| {
            let mut out = Vec::new();
            for (key, h) in helpers {
                let Some(id) = plugin_node(key) else { continue };
                // A helper that does not answer keeps the state the document has (its last one)
                if let Ok(Saved { chunk: Some(chunk), params }) = h.saved(STATE_TIMEOUT) {
                    let params =
                        params.iter().map(|(k, v)| (k.as_str(), Value::real(f64::from(*v)).unwrap_or(Value::NULL)));
                    out.push(Value::object([
                        ("node", Value::text(id.to_string())),
                        ("instrument", Value::bool(matches!(key, NodeKey::Instrument(_)))),
                        ("chunk", Value::text(lm_plugin_proxy::protocol::hex(&chunk))),
                        ("params", Value::object(params)),
                    ]));
                }
            }
            Ok(Value::list(out))
        });
    }

    /// The states the helpers gave: into the document (one `Origin::Plugin` step), then the save if one waits
    pub(crate) fn states_finished(&mut self, kind: &str, result: Result<Value>) {
        let states = result.unwrap_or_else(|e| {
            self.notices.error(&e, None, None);
            Value::empty_list()
        });
        let mut writes = Vec::new();
        for s in states.items() {
            let (Some(id), Some(chunk)) = (
                s.get("node").and_then(Value::as_text).and_then(|t| t.parse::<NodeId>().ok()),
                s.get("chunk").and_then(Value::as_text),
            ) else {
                continue;
            };
            let key = if s.get("instrument").and_then(Value::as_bool) == Some(true) {
                NodeKey::Instrument(id)
            } else {
                NodeKey::Insert(id)
            };
            if let Some(r) = self.external.running.get_mut(&key) {
                r.chunk = Some(chunk.to_owned());
            }
            if !self.document.exists(id) {
                continue;
            }
            // The parameters the state carries, where the document has them otherwise (by the schema's keys)
            let plugin = self.document.find(id).and_then(|n| n.get("plugin")?.as_text().map(str::to_owned));
            let schema = plugin.and_then(|p| self.plugins.schema(&p).cloned());
            let mut params = Vec::new();
            if let (Some(schema), Some(values)) = (schema, s.get("params")) {
                for p in &schema.params {
                    if let Some(v) = values.get(p.key).and_then(Value::as_real)
                        && self.document.param(id, p.key).is_none_or(|now| (now - v).abs() > 1e-6)
                    {
                        params.push((p.key, v));
                    }
                }
            }
            if self.doc_chunk(id).as_deref() != Some(chunk) || !params.is_empty() {
                writes.push((id, chunk.to_owned(), params));
            }
        }
        if !writes.is_empty() && !self.document.read_only() {
            let r = self.document.transact_as(Label::new("plugin.state"), lm_doc::Origin::Plugin, |e| {
                for (id, chunk, params) in &writes {
                    e.set(*id, &[PathStep::key("state"), PathStep::key(CHUNK_KEY)], Value::text(chunk.as_str()))?;
                    for (key, v) in params {
                        e.set_param(*id, key, *v)?;
                    }
                }
                Ok(())
            });
            if let Err(e) = r {
                self.notices.error(&e, None, None);
            }
        }
        if kind == "plugins.state.save" && self.external.save_waiting {
            self.external.save_waiting = false;
            self.external.states_fresh = true;
            if let Err(e) = self.save_here() {
                self.notices.error(&e, Some(Target::Project), None);
            }
        }
    }

    /// Before a save: `true` when the plugins' states are being taken first (the save goes on by itself after)
    pub(crate) fn external_before_save(&mut self) -> bool {
        if std::mem::take(&mut self.external.states_fresh) {
            return false;
        }
        let keys: Vec<NodeKey> = self
            .external
            .running
            .iter()
            .filter(|(_, r)| r.hosted.upgrade().is_some_and(|h| !h.is_dead()))
            .map(|(k, _)| *k)
            .collect();
        if keys.is_empty() {
            return false;
        }
        self.external.save_waiting = true;
        self.fetch_states(keys, true);
        true
    }

    /// Each pump: helpers gone are forgotten, the plugin's own parameter changes go into their gesture, and a
    /// chunk the document changed (an undo, a preset) goes to the running plugin
    pub(crate) fn external_pump(&mut self) -> bool {
        if self.external.running.is_empty() && self.external.gesture.is_none() {
            return false;
        }
        self.external.running.retain(|_, r| r.hosted.strong_count() > 0);
        let now = self.clock.now();
        let params = std::mem::take(&mut self.external.params);
        let mut changed = false;
        if !params.is_empty() && !self.document.read_only() {
            let g = match self.external.gesture.take() {
                Some((g, _)) => g,
                None => self.document.begin_gesture(Label::new("plugin.params")),
            };
            let r = self.document.gesture_update(&g, |e| {
                for (id, key, v) in &params {
                    if e.exists(*id) {
                        e.set_param(*id, key, *v)?;
                    }
                }
                Ok(())
            });
            if let Err(e) = r {
                self.notices.error(&e, None, None);
            }
            // The end of a window's gesture came with these changes: it closes now
            if self.external.edit_ended && self.external.open_edits == 0 {
                self.external.edit_ended = false;
                if let Err(e) = self.document.gesture_commit(g) {
                    self.notices.error(&e, None, None);
                }
            } else {
                self.external.gesture = Some((g, now));
            }
            changed = true;
        } else if let Some((_, at)) = &self.external.gesture {
            let quiet = now.since(*at);
            // With gesture edges the end closes it; without any, a pause of 300 ms; a begin never ended, 2 s
            let done = if self.external.open_edits > 0 {
                quiet >= GESTURE_STALE.as_nanos() as u64
            } else {
                self.external.edit_ended || quiet >= PARAM_GROUP.as_nanos() as u64
            };
            if done && let Some((g, _)) = self.external.gesture.take() {
                self.external.open_edits = 0;
                self.external.edit_ended = false;
                if let Err(e) = self.document.gesture_commit(g) {
                    self.notices.error(&e, None, None);
                }
                changed = true;
            }
        }
        // The document's chunk against the plugin's
        let mut give = Vec::new();
        for (key, r) in &self.external.running {
            let Some(id) = plugin_node(*key) else { continue };
            let doc = self.doc_chunk(id);
            if doc.is_some() && doc != r.chunk {
                give.push((*key, doc.unwrap_or_default()));
            }
        }
        for (key, chunk) in give {
            let Some(h) = self.hosted(key) else { continue };
            if let Some(r) = self.external.running.get_mut(&key) {
                r.chunk = Some(chunk.clone());
            }
            self.start(Label::new("plugins.state"), Target::Project, false, "plugins.load_state", move |_, _| {
                let bytes = lm_plugin_proxy::protocol::unhex(&chunk)?;
                h.load_state(&bytes, STATE_TIMEOUT)?;
                Ok(Value::NULL)
            });
        }
        changed
    }

    /// The plugin's preference to run all its instances in one helper process (default: one process per instance, so
    /// a crash or a hang costs one plugin; in a group it costs them all). Saved in the preferences
    /// (`plugins.grouped`); it applies to the instances built after the next start
    pub fn set_plugin_grouped(&mut self, plugin: &str, on: bool) -> Result<()> {
        if !self.external.made.contains_key(plugin) {
            return Err(Error::new("plugin.group.unknown").arg("plugin", plugin));
        }
        let prefs = plugins_pref(&self.preferences);
        let mut grouped: Vec<Value> =
            list_of(&prefs, "grouped").into_iter().filter(|g| g.as_text() != Some(plugin)).collect();
        if on {
            grouped.push(Value::text(plugin));
        }
        let tree = prefs.with("grouped", Value::list(grouped));
        self.set_preference(&[PathStep::key("plugins")], tree)
    }

    /// Shows or hides the plugin's own window (I-22), in an operation: the helper makes the window; the answer says
    /// whether it is open and, when there is none to show, why (a notice; the generic editor stays)
    pub fn plugin_window(&mut self, node: NodeId, on: bool) -> Result<()> {
        let hosted = self
            .plugin_host_of(node)
            .filter(|h| !h.is_dead())
            .ok_or_else(|| Error::new("plugin.window.unknown").arg("node", node.to_string()))?;
        self.start(Label::new("plugins.window"), Target::Node(node), false, "plugins.window", move |_, _| {
            let command = if on { WindowCommand::Show } else { WindowCommand::Hide };
            let info = hosted.window(command, WINDOW_TIMEOUT)?;
            Ok(Value::object([
                ("node", Value::text(node.to_string())),
                ("asked", Value::bool(on)),
                ("open", Value::bool(info.open)),
                ("reason", info.reason.as_deref().map_or(Value::NULL, Value::text)),
            ]))
        });
        Ok(())
    }

    /// The helper answered about a window: a window that was asked for and does not exist says why
    pub(crate) fn window_finished(&mut self, result: Result<Value>) {
        match result {
            Err(e) => self.notices.error(&e, None, None),
            Ok(v) => {
                let asked = v.get("asked").and_then(Value::as_bool) == Some(true);
                let open = v.get("open").and_then(Value::as_bool) == Some(true);
                if asked && !open {
                    let node = v.get("node").and_then(Value::as_text).unwrap_or("");
                    let reason = v.get("reason").and_then(Value::as_text).unwrap_or("plugin.window.none");
                    self.notices.push(
                        "plugin.window.unavailable",
                        vec![("node", Value::text(node)), ("reason", Value::text(reason))],
                        Severity::Info,
                        node.parse::<NodeId>().ok().map(Target::Node),
                        Vec::new(),
                    );
                }
            }
        }
    }

    /// "Recreate": the node's plugin is built again from the document's state (its last saved state), in a new
    /// helper; the old one (silent) plays until then
    pub fn recreate_plugin(&mut self, node: NodeId) -> Result<()> {
        let keys = [NodeKey::Instrument(node), NodeKey::Insert(node)];
        let mut any = false;
        for k in keys {
            if self.external.running.contains_key(&k) {
                self.external.running.remove(&k);
                self.compiler.renew(k);
                any = true;
            }
        }
        if !any {
            return Err(Error::new("plugin.recreate.unknown").arg("node", node.to_string()));
        }
        self.needs_compile = true;
        Ok(())
    }

    /// The helper running a node's plugin (tests; the plugin's window)
    pub fn plugin_host_of(&self, node: NodeId) -> Option<Arc<Hosted>> {
        self.hosted(NodeKey::Instrument(node)).or_else(|| self.hosted(NodeKey::Insert(node)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folders_of_each_format_are_read_and_bounded() {
        let many: Vec<Value> = (0..MAX_FOLDERS + 5).map(|i| Value::text(format!("/p/{i}"))).collect();
        let prefs = Value::object([(
            "folders",
            Value::object([
                ("lv2", Value::list([Value::text(" /a "), Value::text(""), Value::int(3)])),
                ("clap", Value::list(many)),
                ("vst2", Value::list([Value::text("/ignored")])),
            ]),
        )]);
        let f = folders_of(&prefs);
        assert_eq!(f["lv2"], [std::path::PathBuf::from("/a")]);
        assert_eq!(f["clap"].len(), MAX_FOLDERS);
        assert!(!f.contains_key("vst2") && !f.contains_key("vst3"), "only the hosted formats");
    }

    fn entry(format: &str, uri: &str, reason: &str) -> Value {
        Value::object([
            ("format", Value::text(format)),
            ("uri", Value::text(uri)),
            ("reason", Value::text(reason)),
        ])
    }

    fn uris(v: &[Value]) -> Vec<&str> {
        v.iter().filter_map(|b| b.get("uri")?.as_text()).collect()
    }

    #[test]
    fn a_crash_is_skipped_at_once_and_a_first_timeout_is_tried_again() {
        let after = merge_blocked(
            Vec::new(),
            &[entry("clap", "boom", "plugin_host.crashed"), entry("clap", "slow", "plugin_host.timeout")],
            &[],
            "clap",
        );
        assert_eq!(uris(&after), ["boom", "slow"], "both are on the list, so the user sees them");
        assert_eq!(skipped_uris(&after, "clap"), ["boom"], "only the crash is skipped");
        // The second timeout makes it a block, until the user lifts it
        let again = merge_blocked(after, &[entry("clap", "slow", "plugin_host.timeout")], &[], "clap");
        assert_eq!(strikes_of(&again[1]), 2);
        assert_eq!(skipped_uris(&again, "clap"), ["boom", "slow"]);
        assert!(skipped_uris(&again, "vst3").is_empty(), "other formats are not touched");
    }

    #[test]
    fn a_plugin_that_works_again_leaves_the_list() {
        let blocked = merge_blocked(Vec::new(), &[entry("clap", "slow", "plugin_host.timeout")], &[], "clap");
        let found = [Scanned {
            format: "clap".into(),
            uri: "slow".into(),
            name: "Slow".into(),
            instrument: false,
            inputs: 2,
            outputs: 2,
            midi_in: false,
            params: Vec::new(),
            latency: false,
            has_window: false,
            file: None,
        }];
        assert!(merge_blocked(blocked, &[], &found, "clap").is_empty());
    }

    #[test]
    fn the_user_can_lift_one_block_or_all() {
        let list = vec![
            entry("clap", "a", "plugin_host.crashed"),
            entry("clap", "b", "plugin_host.timeout"),
            entry("vst3", "c", "plugin_host.crashed"),
        ];
        assert_eq!(uris(&unblock(list.clone(), Some("clap"), Some("a"))), ["b", "c"]);
        assert_eq!(uris(&unblock(list.clone(), Some("clap"), None)), ["c"]);
        assert!(unblock(list, None, None).is_empty());
    }
}
