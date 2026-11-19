//! The lilv world and plugin discovery

use crate::ffi;
use crate::urid::{UridMap, Urids, uri};
use lm_base::{Error, Result};
use std::ffi::{CStr, CString, OsStr};
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// What kind of data a port carries
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortKind {
    /// One `f32` per frame, connected to the host's buffers at every [`run`](crate::Instance::run)
    Audio,
    /// One `f32` per block (a parameter, or a value the plugin reports)
    Control,
    /// One `f32` per frame of control signal; connected to buffers the instance owns
    Cv,
    /// An atom buffer (`atom:Sequence` of events: MIDI and others)
    Atom,
    /// A port type this host does not know: the plugin can only be instantiated if the port is optional
    Unknown,
}

/// Which way a port goes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Input,
    Output,
    /// Neither declared: the plugin can only be instantiated if the port is optional
    Unknown,
}

/// One port of a plugin, as its Turtle data describes it
#[derive(Clone, Debug, PartialEq)]
pub struct PortInfo {
    pub index: u32,
    /// The permanent name of the port (the key a host saves a control value under)
    pub symbol: String,
    pub name: String,
    pub kind: PortKind,
    pub direction: Direction,
    pub default: Option<f32>,
    pub minimum: Option<f32>,
    pub maximum: Option<f32>,
    pub connection_optional: bool,
    pub toggled: bool,
    pub integer: bool,
    pub enumeration: bool,
    pub logarithmic: bool,
    /// The range and default are fractions of the sample rate
    pub sample_rate: bool,
    pub reports_latency: bool,
    /// An atom port that takes or gives MIDI events
    pub midi: bool,
    /// The smallest buffer an atom port accepts, in bytes (`rsz:minimumSize`)
    pub minimum_size: Option<u32>,
}

impl PortInfo {
    /// The value a control port starts at: the default, else the minimum, else 0, scaled by the rate if the port
    /// says so
    pub fn initial_value(&self, sample_rate: f64) -> f32 {
        let value = self.default.or(self.minimum).unwrap_or(0.0);
        if self.sample_rate { (f64::from(value) * sample_rate) as f32 } else { value }
    }
}

/// One plugin, as its Turtle data describes it. Owned data: nothing in it points into lilv
#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    pub uri: String,
    pub name: String,
    pub author: Option<String>,
    pub class_uri: String,
    pub class_label: String,
    pub bundle_path: PathBuf,
    pub library_path: PathBuf,
    pub ports: Vec<PortInfo>,
    pub required_features: Vec<String>,
    pub optional_features: Vec<String>,
    /// The `lv2:extensionData` the plugin declares (`worker:interface`, `state:interface`, ...)
    pub extension_data: Vec<String>,
    /// The options the plugin needs (`opts:requiredOption`)
    pub required_options: Vec<String>,
    /// The index of the control output that reports the latency in frames
    pub latency_port: Option<u32>,
    /// lilv's check of the plugin's data (every port has an index, a symbol and a name, and so on)
    pub verified: bool,
}

impl PluginInfo {
    /// The required features and options this host does not provide with `config` (empty = it can be
    /// instantiated, as far as its data says). `log:log` is never provided (see the crate's safety sheet)
    pub fn missing(&self, config: &crate::InstanceConfig) -> Vec<String> {
        let features = self.required_features.iter().filter(|f| !crate::instance::host_provides(f, config));
        let options = self.required_options.iter().filter(|o| !crate::instance::host_option(o));
        features.chain(options).cloned().collect()
    }

    pub fn has_extension_data(&self, uri: &str) -> bool {
        self.extension_data.iter().any(|e| e == uri)
    }

    pub fn port(&self, symbol: &str) -> Option<&PortInfo> {
        self.ports.iter().find(|p| p.symbol == symbol)
    }
}

/// The nodes the discovery compares against, made once per world
pub(crate) struct Nodes {
    pub audio_port: *mut ffi::LilvNode,
    pub control_port: *mut ffi::LilvNode,
    pub cv_port: *mut ffi::LilvNode,
    pub atom_port: *mut ffi::LilvNode,
    pub input_port: *mut ffi::LilvNode,
    pub output_port: *mut ffi::LilvNode,
    pub connection_optional: *mut ffi::LilvNode,
    pub toggled: *mut ffi::LilvNode,
    pub integer: *mut ffi::LilvNode,
    pub enumeration: *mut ffi::LilvNode,
    pub logarithmic: *mut ffi::LilvNode,
    pub sample_rate: *mut ffi::LilvNode,
    pub reports_latency: *mut ffi::LilvNode,
    pub midi_event: *mut ffi::LilvNode,
    pub minimum_size: *mut ffi::LilvNode,
    pub required_option: *mut ffi::LilvNode,
}

impl Nodes {
    fn new(world: *mut ffi::LilvWorld) -> Self {
        let n = |u: &CStr| {
            // SAFETY: `world` is valid; lilv copies the string
            unsafe { ffi::lilv_new_uri(world, u.as_ptr()) }
        };
        Self {
            audio_port: n(uri::AUDIO_PORT),
            control_port: n(uri::CONTROL_PORT),
            cv_port: n(uri::CV_PORT),
            atom_port: n(uri::ATOM_PORT),
            input_port: n(uri::INPUT_PORT),
            output_port: n(uri::OUTPUT_PORT),
            connection_optional: n(uri::CONNECTION_OPTIONAL),
            toggled: n(uri::TOGGLED),
            integer: n(uri::INTEGER),
            enumeration: n(uri::ENUMERATION),
            logarithmic: n(uri::LOGARITHMIC),
            sample_rate: n(uri::SAMPLE_RATE),
            reports_latency: n(uri::REPORTS_LATENCY),
            midi_event: n(uri::MIDI_EVENT),
            minimum_size: n(uri::MINIMUM_SIZE),
            required_option: n(uri::REQUIRED_OPTION),
        }
    }

    fn all(&self) -> [*mut ffi::LilvNode; 16] {
        [
            self.audio_port,
            self.control_port,
            self.cv_port,
            self.atom_port,
            self.input_port,
            self.output_port,
            self.connection_optional,
            self.toggled,
            self.integer,
            self.enumeration,
            self.logarithmic,
            self.sample_rate,
            self.reports_latency,
            self.midi_event,
            self.minimum_size,
            self.required_option,
        ]
    }
}

/// The `LilvWorld` and its nodes: only touched with the world's mutex held
pub(crate) struct Core {
    pub world: NonNull<ffi::LilvWorld>,
    pub nodes: Nodes,
}

// SAFETY: a LilvWorld (and the nodes, plugins and libraries it owns) is plain memory with no thread affinity; lilv is
// not thread-safe, so every use goes through the `Mutex<Core>` of `Shared`: one thread at a time
unsafe impl Send for Core {}

impl Drop for Core {
    fn drop(&mut self) {
        for node in self.nodes.all() {
            // SAFETY: each node came from lilv_new_uri (NULL is accepted by lilv_node_free) and is freed once, here
            unsafe { ffi::lilv_node_free(node) };
        }
        // SAFETY: the world came from lilv_world_new and is freed once, after every instance made from it (each holds
        // an `Arc<Shared>`, so the last instance is gone when this runs)
        unsafe { ffi::lilv_world_free(self.world.as_ptr()) };
    }
}

pub(crate) struct Shared {
    core: Mutex<Core>,
    pub urid_map: UridMap,
    pub urids: Urids,
}

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The lilv world: the plugins installed on the system (or in the folders given), read from their Turtle data.
///
/// Discovery reads Turtle only and never runs plugin code (lilv's dynamic manifests are switched off): a plugin that
/// crashes does so at [`instantiate`](World::instantiate), which is what the scan's helper process probes (H15).
/// `Send` and `Sync`: every lilv call takes the world's lock (lilv itself is not thread-safe). Instances keep the
/// world alive.
#[derive(Clone)]
pub struct World {
    pub(crate) shared: Arc<Shared>,
}

fn c_path(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::new("lilv.bad_path").arg("path", path.to_string_lossy().into_owned()))
}

/// Text of a node (NULL = empty)
fn node_string(node: *const ffi::LilvNode) -> String {
    if node.is_null() {
        return String::new();
    }
    // SAFETY: a non-NULL node from lilv; lilv_node_as_string returns a string owned by the node (or NULL)
    let text = unsafe { ffi::lilv_node_as_string(node) };
    if text.is_null() {
        return String::new();
    }
    // SAFETY: NUL-terminated, valid while the node lives (we copy it now)
    unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()
}

/// Text of a node lilv gave us to own, freed here
fn take_string(node: *mut ffi::LilvNode) -> Option<String> {
    if node.is_null() {
        return None;
    }
    let text = node_string(node);
    // SAFETY: the node was returned as owned by the caller (lilv_plugin_get_name and the like) and is freed once
    unsafe { ffi::lilv_node_free(node) };
    Some(text)
}

/// A file URI turned into a path (lilv's parser handles the escapes)
fn node_path(node: *const ffi::LilvNode) -> PathBuf {
    if node.is_null() {
        return PathBuf::new();
    }
    // SAFETY: a non-NULL node; lilv_node_as_uri returns its URI string or NULL
    let uri = unsafe { ffi::lilv_node_as_uri(node) };
    if uri.is_null() {
        return PathBuf::new();
    }
    // SAFETY: `uri` is NUL-terminated; the result is a malloc'ed string (or NULL) we free with lilv_free
    let path = unsafe { ffi::lilv_file_uri_parse(uri, std::ptr::null_mut()) };
    if path.is_null() {
        return PathBuf::new();
    }
    // SAFETY: NUL-terminated string just returned
    let out = PathBuf::from(OsStr::from_bytes(unsafe { CStr::from_ptr(path) }.to_bytes()));
    // SAFETY: allocated by lilv for us, freed once
    unsafe { ffi::lilv_free(path.cast()) };
    out
}

/// The URIs in a node collection lilv gave us to own, freed here
fn take_strings(nodes: *mut ffi::LilvNodes) -> Vec<String> {
    let mut out = Vec::new();
    if nodes.is_null() {
        return out;
    }
    // SAFETY: iteration over a valid collection, each element read before the collection is freed below
    unsafe {
        let mut i = ffi::lilv_nodes_begin(nodes);
        while !ffi::lilv_nodes_is_end(nodes, i) {
            out.push(node_string(ffi::lilv_nodes_get(nodes, i)));
            i = ffi::lilv_nodes_next(nodes, i);
        }
        ffi::lilv_nodes_free(nodes);
    }
    out
}

/// A numeric node as `f32`, taken and freed
fn take_number(node: *mut ffi::LilvNode) -> Option<f32> {
    if node.is_null() {
        return None;
    }
    // SAFETY: a valid node owned by us, freed once after reading
    unsafe {
        let value = if ffi::lilv_node_is_float(node) || ffi::lilv_node_is_int(node) {
            Some(ffi::lilv_node_as_float(node))
        } else if ffi::lilv_node_is_bool(node) {
            Some(if ffi::lilv_node_as_bool(node) { 1.0 } else { 0.0 })
        } else {
            None
        };
        ffi::lilv_node_free(node);
        value.filter(|v| v.is_finite())
    }
}

impl Core {
    pub fn plugins(&self) -> *const ffi::LilvPlugins {
        // SAFETY: the world is valid; the collection belongs to the world
        unsafe { ffi::lilv_world_get_all_plugins(self.world.as_ptr()) }
    }

    pub fn find(&self, plugin_uri: &str) -> Result<*const ffi::LilvPlugin> {
        let not_found = || Error::new("lilv.no_plugin").arg("uri", plugin_uri);
        let c_uri = CString::new(plugin_uri).map_err(|_| not_found())?;
        // SAFETY: valid world; lilv copies the string into the node
        let node = unsafe { ffi::lilv_new_uri(self.world.as_ptr(), c_uri.as_ptr()) };
        if node.is_null() {
            return Err(not_found());
        }
        // SAFETY: valid collection and node; the node is freed right after the lookup
        let plugin = unsafe { ffi::lilv_plugins_get_by_uri(self.plugins(), node) };
        // SAFETY: made above, freed once
        unsafe { ffi::lilv_node_free(node) };
        if plugin.is_null() { Err(not_found()) } else { Ok(plugin) }
    }

    pub fn info(&self, plugin: *const ffi::LilvPlugin) -> PluginInfo {
        let n = &self.nodes;
        // SAFETY: `plugin` belongs to this world (from its collection) and the lock is held by the caller (`&self`
        // comes from the guard); every node lilv returns as owned is freed by the take_* helpers
        unsafe {
            let class = ffi::lilv_plugin_get_class(plugin);
            let (class_uri, class_label) = if class.is_null() {
                (uri::LV2_PLUGIN.to_string_lossy().into_owned(), String::from("Plugin"))
            } else {
                (
                    node_string(ffi::lilv_plugin_class_get_uri(class)),
                    node_string(ffi::lilv_plugin_class_get_label(class)),
                )
            };
            let latency_port =
                ffi::lilv_plugin_has_latency(plugin).then(|| ffi::lilv_plugin_get_latency_port_index(plugin));
            let ports = (0..ffi::lilv_plugin_get_num_ports(plugin))
                .map(|index| {
                    let port = ffi::lilv_plugin_get_port_by_index(plugin, index);
                    let is = |class: *mut ffi::LilvNode| ffi::lilv_port_is_a(plugin, port, class);
                    let has = |property: *mut ffi::LilvNode| ffi::lilv_port_has_property(plugin, port, property);
                    let kind = if is(n.audio_port) {
                        PortKind::Audio
                    } else if is(n.control_port) {
                        PortKind::Control
                    } else if is(n.cv_port) {
                        PortKind::Cv
                    } else if is(n.atom_port) {
                        PortKind::Atom
                    } else {
                        PortKind::Unknown
                    };
                    let direction = if is(n.input_port) {
                        Direction::Input
                    } else if is(n.output_port) {
                        Direction::Output
                    } else {
                        Direction::Unknown
                    };
                    let (mut def, mut min, mut max) =
                        (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
                    ffi::lilv_port_get_range(plugin, port, &mut def, &mut min, &mut max);
                    let minimum_size = take_number(ffi::lilv_port_get(plugin, port, n.minimum_size))
                        .filter(|v| *v >= 0.0)
                        .map(|v| v as u32);
                    PortInfo {
                        index,
                        symbol: node_string(ffi::lilv_port_get_symbol(plugin, port)),
                        name: take_string(ffi::lilv_port_get_name(plugin, port)).unwrap_or_default(),
                        kind,
                        direction,
                        default: take_number(def),
                        minimum: take_number(min),
                        maximum: take_number(max),
                        connection_optional: has(n.connection_optional),
                        toggled: has(n.toggled),
                        integer: has(n.integer),
                        enumeration: has(n.enumeration),
                        logarithmic: has(n.logarithmic),
                        sample_rate: has(n.sample_rate),
                        reports_latency: has(n.reports_latency) || latency_port == Some(index),
                        midi: kind == PortKind::Atom && ffi::lilv_port_supports_event(plugin, port, n.midi_event),
                        minimum_size,
                    }
                })
                .collect();
            PluginInfo {
                uri: node_string(ffi::lilv_plugin_get_uri(plugin)),
                name: take_string(ffi::lilv_plugin_get_name(plugin)).unwrap_or_default(),
                author: take_string(ffi::lilv_plugin_get_author_name(plugin)),
                class_uri,
                class_label,
                bundle_path: node_path(ffi::lilv_plugin_get_bundle_uri(plugin)),
                library_path: node_path(ffi::lilv_plugin_get_library_uri(plugin)),
                ports,
                required_features: take_strings(ffi::lilv_plugin_get_required_features(plugin)),
                optional_features: take_strings(ffi::lilv_plugin_get_optional_features(plugin)),
                extension_data: take_strings(ffi::lilv_plugin_get_extension_data(plugin)),
                required_options: take_strings(ffi::lilv_plugin_get_value(plugin, n.required_option)),
                latency_port,
                verified: ffi::lilv_plugin_verify(plugin),
            }
        }
    }
}

impl World {
    /// An empty world: nothing is loaded until [`load_all`](Self::load_all), [`load_path`](Self::load_path) or
    /// [`load_bundle`](Self::load_bundle). Dynamic manifests are switched off (they would run plugin code during
    /// discovery). Error: `lilv.world_failed`
    pub fn new() -> Result<Self> {
        let raw = ffi::lilv_world_new();
        let world = NonNull::new(raw).ok_or_else(|| Error::new("lilv.world_failed"))?;
        // SAFETY: valid world; the option node is copied by lilv and freed right after
        unsafe {
            let no = ffi::lilv_new_bool(world.as_ptr(), false);
            ffi::lilv_world_set_option(world.as_ptr(), uri::LILV_OPTION_DYN_MANIFEST.as_ptr(), no);
            ffi::lilv_node_free(no);
        }
        let core = Core { world, nodes: Nodes::new(world.as_ptr()) };
        let urid_map = UridMap::new();
        let urids = urid_map.urids();
        Ok(Self { shared: Arc::new(Shared { core: Mutex::new(core), urid_map, urids }) })
    }

    /// Loads every bundle on the LV2 path: `LV2_PATH` if set, else the system's default folders (on Linux
    /// `~/.lv2:/usr/local/lib/lv2:/usr/lib/lv2`; on macOS `~/Library/Audio/Plug-Ins/LV2:/Library/Audio/Plug-Ins/LV2`
    /// and the same Unix folders). Call once
    pub fn load_all(&self) {
        let core = self.shared.lock();
        // SAFETY: valid world, lock held
        unsafe { ffi::lilv_world_load_all(core.world.as_ptr()) };
    }

    /// Loads every bundle in the folders of `path` (a `:`-separated list, like `LV2_PATH`), instead of the
    /// environment. Call once, instead of [`load_all`](Self::load_all). Error: `lilv.bad_path`
    pub fn load_path(&self, path: &OsStr) -> Result<()> {
        let c = CString::new(path.as_bytes())
            .map_err(|_| Error::new("lilv.bad_path").arg("path", path.to_string_lossy().into_owned()))?;
        let core = self.shared.lock();
        // SAFETY: valid world, lock held; the string node is copied by lilv and freed after
        unsafe {
            let node = ffi::lilv_new_string(core.world.as_ptr(), c.as_ptr());
            ffi::lilv_world_set_option(core.world.as_ptr(), uri::LILV_OPTION_LV2_PATH.as_ptr(), node);
            ffi::lilv_node_free(node);
            ffi::lilv_world_load_all(core.world.as_ptr());
        }
        Ok(())
    }

    /// Loads one bundle (a folder ending in `.lv2`). Error: `lilv.bad_path` (not an existing folder)
    pub fn load_bundle(&self, bundle: &Path) -> Result<()> {
        let bad = || Error::new("lilv.bad_path").arg("path", bundle.to_string_lossy().into_owned());
        let absolute = bundle.canonicalize().map_err(|_| bad())?;
        if !absolute.is_dir() {
            return Err(bad());
        }
        // lilv wants the bundle's URI with a trailing slash
        let mut text = absolute.into_os_string();
        text.push("/");
        let c = c_path(Path::new(&text))?;
        let core = self.shared.lock();
        // SAFETY: valid world, lock held; the URI node is freed after loading
        unsafe {
            let node = ffi::lilv_new_file_uri(core.world.as_ptr(), std::ptr::null(), c.as_ptr());
            if node.is_null() {
                return Err(bad());
            }
            ffi::lilv_world_load_bundle(core.world.as_ptr(), node);
            ffi::lilv_node_free(node);
        }
        Ok(())
    }

    /// Every plugin loaded, as owned data. Reads Turtle only (no plugin code runs)
    pub fn plugins(&self) -> Vec<PluginInfo> {
        let core = self.shared.lock();
        let plugins = core.plugins();
        let mut out = Vec::new();
        // SAFETY: iteration over the world's collection with the lock held
        unsafe {
            let mut i = ffi::lilv_plugins_begin(plugins);
            while !ffi::lilv_plugins_is_end(plugins, i) {
                out.push(core.info(ffi::lilv_plugins_get(plugins, i)));
                i = ffi::lilv_plugins_next(plugins, i);
            }
        }
        out
    }

    /// The plugin with this URI, if loaded
    pub fn plugin(&self, plugin_uri: &str) -> Option<PluginInfo> {
        let core = self.shared.lock();
        core.find(plugin_uri).ok().map(|p| core.info(p))
    }

    /// The URID map the instances of this world share
    pub fn urid_map(&self) -> &UridMap {
        &self.shared.urid_map
    }

    /// The ids of the URIs the host uses (MIDI events, atom types)
    pub fn urids(&self) -> Urids {
        self.shared.urids
    }
}

impl fmt::Debug for World {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("World").field("urid_map", &self.shared.urid_map).finish_non_exhaustive()
    }
}
