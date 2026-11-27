//! A helper process with one plugin instance in it (I-9.6), from the core's side: starting it (an `Operation` of
//! the session: it may take seconds), its control channel, the state, the plugin's window, and the supervisor thread
//! that notices a crash and kills a hung helper.
//!
//! One helper per instance (decisoes-autonomas.md): a plugin that crashes or hangs takes nothing else with it, and a
//! hung one is killed without stopping another plugin's audio. The helper's audio thread waits on the shared block,
//! so an idle helper costs no CPU.

use std::collections::BTreeMap;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicI64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lm_base::{Error, Result};
use lm_state::Value;
use lm_sys_ipc::{FdChannel, HELPER_FD, SharedMemory};

use crate::layout::{Layout, clamp_latency, w};
use crate::protocol::{PROTOCOL, encode_message, error_from, hex, read_message, unhex};
use crate::proxy::{Deadline, Proxy, ProxyState};

/// Where the helper program is and how the instances run
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// The `lm-plugin-host` program
    pub binary: PathBuf,
    pub deadline: Deadline,
    /// How long starting a plugin may take before it is given up
    pub load_timeout: Duration,
    /// Folders the helper searches for plugins besides the system's (an LV2 bundle or a folder of them)
    pub paths: Vec<PathBuf>,
    /// Whether the system's plugin folders are searched too (the tests search only `paths`)
    pub system_paths: bool,
}

impl HostConfig {
    /// `LM_PLUGIN_HOST`, else `lm-plugin-host` next to this program, else on the `PATH`
    pub fn find() -> HostConfig {
        let binary = std::env::var_os("LM_PLUGIN_HOST").map(PathBuf::from).unwrap_or_else(|| {
            let here = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("lm-plugin-host")));
            match here {
                Some(p) if p.exists() => p,
                _ => PathBuf::from("lm-plugin-host"),
            }
        });
        HostConfig {
            binary,
            deadline: Deadline::default(),
            load_timeout: Duration::from_secs(20),
            paths: Vec::new(),
            system_paths: true,
        }
    }
}

/// What to load
#[derive(Clone, Debug, PartialEq)]
pub struct LoadRequest {
    /// `"lv2"`, `"clap"`, `"vst3"`, `"ladspa"` (and `"test"`, the helper's own test plugins)
    pub format: String,
    /// The plugin's id in its format: an LV2 URI, a CLAP id, a VST3 class id, a LADSPA `<file stem>:<label>`
    pub uri: String,
    pub rate: u32,
    pub max_block: usize,
    pub inputs: usize,
    pub outputs: usize,
    pub sidechain: usize,
    /// The file it was found in, tried first
    pub file: Option<String>,
    /// The plugin's own state, saved before
    pub chunk: Option<Vec<u8>>,
    /// Parameter values by their key (port symbol, param id)
    pub params: Vec<(String, f32)>,
}

/// What the helper said when the plugin was ready
#[derive(Clone, Debug, PartialEq)]
pub struct Ready {
    pub latency: u32,
    /// The rest of the helper's answer (the plugin's parameters as it sees them, its name...)
    pub info: Value,
}

/// A plugin's state as the helper gave it
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub chunk: Option<Vec<u8>>,
    /// The parameters' values, by key
    pub params: Vec<(String, f32)>,
}

/// What the core asks of the plugin's window
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowCommand {
    Show,
    Hide,
    /// The client area's new size
    Resize(u32, u32),
}

/// What the helper answers about the plugin's window
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub open: bool,
    pub size: Option<(u32, u32)>,
    /// Why there is no window when one was asked for (an error key such as `lv2.ui.generic`, `gui.unsupported`)
    pub reason: Option<String>,
}

/// A running helper
pub struct Hosted {
    process: Arc<Process>,
    /// The instance's number in its helper (a group's messages carry it; alone it is 0)
    id: i64,
    replies: Mutex<Receiver<Value>>,
    state: Arc<ProxyState>,
    stop: Arc<AtomicBool>,
    supervisor: Mutex<Option<JoinHandle<()>>>,
    shm: Arc<SharedMemory>,
    /// The plugin's own window is shown (the helper says when the user closes it)
    gui_open: Arc<AtomicBool>,
    pub ready: Ready,
}

impl std::fmt::Debug for Hosted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hosted").field("ready", &self.ready).finish_non_exhaustive()
    }
}

/// A value from the helper as an `f32`: `None` when it does not fit (a double above the `f32` range becomes infinity)
fn finite_f32(x: f64) -> Option<f32> {
    let v = x as f32;
    v.is_finite().then_some(v)
}

fn io(key: &'static str, e: &std::io::Error) -> Error {
    Error::new(key).arg("error", e.to_string())
}

fn wait_reply(rx: &Receiver<Value>, until: Instant, want: &[&str]) -> Result<Value> {
    loop {
        let left = until.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(v) => {
                let t = v.get("t").and_then(Value::as_text).unwrap_or("");
                if t == "error" {
                    return Err(error_from(v.get("error").unwrap_or(&Value::NULL)));
                }
                if want.contains(&t) {
                    return Ok(v);
                }
            }
            Err(RecvTimeoutError::Timeout) => return Err(Error::new("plugin_host.timeout")),
            Err(RecvTimeoutError::Disconnected) => return Err(Error::new("plugin_host.crashed")),
        }
    }
}

/// Messages of one instance waiting to be read: an instance that never reads them (the helper chatters) loses the
/// newest ones instead of growing the core's memory
const REPLY_QUEUE: usize = 256;
/// Messages waiting to be written to the helper's standard input
const OUTBOX: usize = 64;
/// The largest window size the core takes from a helper (a made up size must not reach the window system)
const MAX_WINDOW: u32 = 16_384;
/// The longest reason text taken from a helper
const MAX_REASON: usize = 160;

/// Where a helper's answers go: by the instance they are about (`-1`: the helper's own `hello`)
type Routes = Arc<Mutex<BTreeMap<i64, (SyncSender<Value>, Arc<AtomicBool>)>>>;

impl std::fmt::Debug for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Process").field("group", &self.group).finish_non_exhaustive()
    }
}

/// A helper process: one instance alone (the default) or a group of instances (`lm-plugin-host run --group`), the
/// cost of a group being that a crash or a hang of one takes the others down with it
pub struct Process {
    child: Arc<Mutex<Child>>,
    /// Lines for the helper's standard input, written by a thread of their own: a helper that stopped reading blocks
    /// that thread, never the caller (and never the thread that drops the last reference)
    outbox: Mutex<Option<SyncSender<Vec<u8>>>>,
    routes: Routes,
    group: bool,
    next: AtomicI64,
    /// The way the shared blocks' descriptors reach the helper (the helper never opens anything by name)
    fds: FdChannel,
    /// Held while a block's descriptor and its `load` are sent, so a group's helper receives them in the same order
    load_order: Mutex<()>,
}

impl Process {
    fn spawn(config: &HostConfig, group: bool, until: Instant) -> Result<Arc<Process>> {
        let mut command = Command::new(&config.binary);
        command.arg("run");
        if group {
            command.arg("--group");
        }
        for p in &config.paths {
            command.arg("--path").arg(p);
        }
        let (fds, helper_end) = FdChannel::pair()?;
        lm_sys_ipc::pass_to_child(&mut command, &helper_end, HELPER_FD);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| io("plugin_host.spawn", &e).arg("program", config.binary.display().to_string()))?;
        // The helper has its end now (and only it: the core's copy would keep the socket open after the helper died)
        drop(helper_end);
        let stdin = child.stdin.take().ok_or_else(|| Error::new("plugin_host.spawn"))?;
        let stdout = child.stdout.take().ok_or_else(|| Error::new("plugin_host.spawn"))?;
        let routes: Routes = Arc::new(Mutex::new(BTreeMap::new()));
        let child = Arc::new(Mutex::new(child));
        let (tx, hello_rx) = sync_channel(REPLY_QUEUE);
        routes.lock().unwrap_or_else(PoisonError::into_inner).insert(-1, (tx, Arc::new(AtomicBool::new(false))));
        let reader_routes = Arc::clone(&routes);
        let reader_child = Arc::clone(&child);
        let reader = std::thread::Builder::new().name("lm-plugin-host-out".into()).spawn(move || {
            let mut r = BufReader::new(stdout);
            while let Ok(Some(v)) = read_message(&mut r) {
                let t = v.get("t").and_then(Value::as_text).unwrap_or("");
                let id = match v.get("i").and_then(Value::as_int) {
                    Some(i) => i,
                    None if t == "hello" => -1,
                    None => 0,
                };
                let routes = reader_routes.lock().unwrap_or_else(PoisonError::into_inner);
                let Some((tx, gui)) = routes.get(&id) else { continue };
                // The user closed the plugin's window: said at any time, not an answer to a request
                if t == "gui_closed" {
                    gui.store(false, Ordering::Release);
                    continue;
                }
                // Never blocks: a full queue drops the message
                let _ = tx.try_send(v);
            }
            // The helper is gone, or sent a line that cannot be read (the control channel is lost, so the instance
            // would be half dead: the audio goes on, the state and the window never answer): it is killed, and every
            // instance's wait ends as a crash
            let _ = reader_child.lock().unwrap_or_else(PoisonError::into_inner).kill();
            reader_routes.lock().unwrap_or_else(PoisonError::into_inner).clear();
        });
        let (out_tx, out_rx) = sync_channel::<Vec<u8>>(OUTBOX);
        let writer = std::thread::Builder::new().name("lm-plugin-host-in".into()).spawn(move || {
            let mut stdin = stdin;
            for line in out_rx {
                if stdin.write_all(&line).and_then(|()| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        if let Err(e) = reader.and(writer) {
            let mut c = child.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = c.kill();
            let _ = c.wait();
            return Err(io("plugin_host.spawn", &e));
        }
        let process = Arc::new(Process {
            child,
            outbox: Mutex::new(Some(out_tx)),
            routes,
            group,
            next: AtomicI64::new(if group { 1 } else { 0 }),
            fds,
            load_order: Mutex::new(()),
        });
        let hello = wait_reply(&hello_rx, until, &["hello"]).and_then(|h| {
            if h.get("protocol").and_then(Value::as_int) == Some(PROTOCOL) {
                Ok(h)
            } else {
                Err(Error::new("plugin_host.protocol"))
            }
        });
        process.routes.lock().unwrap_or_else(PoisonError::into_inner).remove(&-1);
        if let Err(e) = hello {
            process.kill();
            return Err(e);
        }
        Ok(process)
    }

    fn kill(&self) {
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = child.kill();
        let _ = child.wait();
    }

    fn is_dead(&self) -> bool {
        matches!(self.child.lock().unwrap_or_else(PoisonError::into_inner).try_wait(), Ok(Some(_)))
    }

    fn send(&self, msg: &Value, id: i64) -> Result<()> {
        let msg = if self.group { msg.with("i", Value::int(id)) } else { msg.clone() };
        let line = encode_message(&msg)?;
        let outbox = self.outbox.lock().unwrap_or_else(PoisonError::into_inner);
        match outbox.as_ref().map(|o| o.try_send(line)) {
            Some(Ok(())) => Ok(()),
            Some(Err(TrySendError::Full(_))) => Err(Error::new("plugin_host.pipe").arg("error", "the helper is not reading")),
            Some(Err(TrySendError::Disconnected(_))) | None => Err(Error::new("plugin_host.crashed")),
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // The end of the standard input ends the helper; a helper that does not leave is killed
        drop(self.outbox.lock().unwrap_or_else(PoisonError::into_inner).take());
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        let until = Instant::now() + Duration::from_millis(500);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(5)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }
}

impl std::fmt::Debug for Group {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Group").finish_non_exhaustive()
    }
}

/// Plugin instances that share one helper process (a per-plugin preference; the default is one process per
/// instance, so a crash costs one plugin). Instances started in the same group run in the same helper while it
/// lives; when it dies the next instance starts a new one
#[derive(Default)]
pub struct Group {
    process: Mutex<Weak<Process>>,
}

impl Group {
    pub fn new() -> Arc<Group> {
        Arc::new(Group::default())
    }

    /// The helper of the group: the running one, or a new one
    fn process(&self, config: &HostConfig, until: Instant) -> Result<Arc<Process>> {
        let mut slot = self.process.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(p) = slot.upgrade()
            && !p.is_dead()
        {
            return Ok(p);
        }
        let p = Process::spawn(config, true, until)?;
        *slot = Arc::downgrade(&p);
        Ok(p)
    }
}

/// Starts a helper, loads the plugin, and gives the proxy the renderer will run and the helper's handle (for the
/// state and the window). Blocking for up to `load_timeout`: run it in an `Operation`
pub fn start(config: &HostConfig, req: &LoadRequest) -> Result<(Proxy, Arc<Hosted>)> {
    start_in(config, req, None)
}

/// [`start`], in the helper of a group when there is one (several instances in one process)
pub fn start_in(config: &HostConfig, req: &LoadRequest, group: Option<&Group>) -> Result<(Proxy, Arc<Hosted>)> {
    let layout = Layout::new(req.max_block, req.inputs, req.outputs, req.sidechain);
    let mut shm = SharedMemory::create(layout.words())?;
    layout.write_header(shm.words(), req.rate);
    let until = Instant::now() + config.load_timeout;
    let process = match group {
        Some(g) => g.process(config, until)?,
        None => Process::spawn(config, false, until)?,
    };
    let id = process.next.fetch_add(1, Ordering::AcqRel);
    let (tx, rx) = sync_channel(REPLY_QUEUE);
    let gui_open = Arc::new(AtomicBool::new(false));
    process.routes.lock().unwrap_or_else(PoisonError::into_inner).insert(id, (tx, Arc::clone(&gui_open)));
    let forget = |process: &Process| {
        process.routes.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
    };
    // A failure kills a helper that is alone, and one that stopped answering; a group's others are left alone
    let fail = |process: &Process, e: Error| {
        forget(process);
        if !process.group || e.key() == "plugin_host.timeout" || e.key() == "plugin_host.crashed" {
            process.kill();
        }
        e
    };
    let params =
        Value::object(req.params.iter().map(|(k, v)| (k.as_str(), Value::real(f64::from(*v)).unwrap_or(Value::NULL))));
    let load = Value::object([
        ("t", Value::text("load")),
        ("format", Value::text(req.format.as_str())),
        ("uri", Value::text(req.uri.as_str())),
        ("file", req.file.as_deref().map_or(Value::NULL, Value::text)),
        ("words", Value::int(layout.words() as i64)),
        ("chunk", req.chunk.as_ref().map_or(Value::NULL, |c| Value::text(hex(c)))),
        ("params", params),
    ]);
    // The block's descriptor goes first, then the `load` that tells the helper to take it (the same order for a
    // group's helper, whatever the instances' threads do). The core's copy of the descriptor is closed at once
    {
        let _order = process.load_order.lock().unwrap_or_else(PoisonError::into_inner);
        let sent = match shm.fd() {
            Some(fd) => process.fds.send(fd, id as u64),
            None => Err(Error::new("ipc.shm.create")),
        };
        if let Err(e) = sent.and_then(|()| process.send(&load, id)) {
            return Err(fail(&process, e));
        }
    }
    shm.close_fd();
    let ready = match wait_reply(&rx, until, &["ready"]) {
        Ok(r) => r,
        Err(e) => return Err(fail(&process, e)),
    };
    // What the helper says is the plugin's own number: limited here, once, before anything allocates by it
    let latency = clamp_latency(ready.get("latency").and_then(Value::as_int).unwrap_or(0).clamp(0, i64::from(u32::MAX)) as u32);
    shm.words()[w::LATENCY].store(latency, Ordering::Release);
    let shm = Arc::new(shm);
    let state = Arc::new(ProxyState::default());
    let stop = Arc::new(AtomicBool::new(false));
    let hosted = Arc::new(Hosted {
        process,
        id,
        replies: Mutex::new(rx),
        state: Arc::clone(&state),
        stop: Arc::clone(&stop),
        supervisor: Mutex::new(None),
        shm: Arc::clone(&shm),
        gui_open,
        ready: Ready { latency, info: ready },
    });
    // The supervisor: a crash makes the node silent; a hang (declared by the proxy) kills the helper
    let watched = Arc::downgrade(&hosted);
    let sup = std::thread::Builder::new()
        .name("lm-plugin-host-watch".into())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let Some(h) = watched.upgrade() else { break };
                let mut child = h.process.child.lock().unwrap_or_else(PoisonError::into_inner);
                if h.state.hung.load(Ordering::Acquire) {
                    let _ = child.kill();
                }
                if let Ok(Some(_)) = child.try_wait() {
                    h.state.dead.store(true, Ordering::Release);
                    lm_sys_ipc::wake(&h.shm.words()[w::DONE], 1);
                    break;
                }
                drop(child);
                drop(h);
                std::thread::sleep(Duration::from_millis(20));
            }
        })
        .map_err(|e| io("plugin_host.spawn", &e))?;
    *hosted.supervisor.lock().unwrap_or_else(PoisonError::into_inner) = Some(sup);
    // The proxy keeps the helper alive: it ends when the last of the proxy and the session's handle goes
    let owner: Arc<dyn std::any::Any + Send + Sync> = Arc::clone(&hosted) as Arc<dyn std::any::Any + Send + Sync>;
    let proxy = Proxy::new(shm, layout, state, config.deadline, req.rate, latency, owner);
    Ok((proxy, hosted))
}

impl Hosted {
    fn request(&self, msg: &Value, want: &[&str], timeout: Duration) -> Result<Value> {
        if self.is_dead() {
            return Err(Error::new("plugin_host.crashed"));
        }
        let rx = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        // Answers to older requests that timed out are dropped
        while rx.try_recv().is_ok() {}
        self.process.send(msg, self.id)?;
        wait_reply(&rx, Instant::now() + timeout, want)
    }

    /// The plugin's own state (`None`: it has none). Blocking up to `timeout`: an `Operation`
    pub fn save_state(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        self.saved(timeout).map(|s| s.chunk)
    }

    /// The plugin's own state and its parameters' values with it (a state the plugin took by itself changes them
    /// too). Blocking up to `timeout`: an `Operation`
    pub fn saved(&self, timeout: Duration) -> Result<Saved> {
        let r = self.request(&Value::object([("t", Value::text("save"))]), &["state"], timeout)?;
        let chunk = match r.get("chunk").and_then(Value::as_text) {
            Some(h) => Some(unhex(h)?),
            None => None,
        };
        let params = r
            .get("params")
            .map(|p| p.entries().filter_map(|(k, v)| Some((k.to_owned(), finite_f32(v.as_real()?)?))).collect())
            .unwrap_or_default();
        Ok(Saved { chunk, params })
    }

    /// Gives the plugin a state (an undo, a preset), without building it again. Blocking up to `timeout`: an
    /// `Operation`. A state it cannot take is an error, and it keeps the one it had
    pub fn load_state(&self, chunk: &[u8], timeout: Duration) -> Result<()> {
        let msg = Value::object([("t", Value::text("restore")), ("chunk", Value::text(hex(chunk)))]);
        self.request(&msg, &["restored"], timeout).map(|_| ())
    }

    /// Opens or closes the plugin's own window, made by the helper (I-22: third-party plugins open their own)
    pub fn show_window(&self, on: bool, timeout: Duration) -> Result<bool> {
        self.window(if on { WindowCommand::Show } else { WindowCommand::Hide }, timeout).map(|w| w.open)
    }

    /// Shows, hides or resizes the plugin's own window. Blocking up to `timeout` (a plugin's window may take
    /// seconds): an `Operation`. A plugin with no window answers `open: false` with the `reason` (an error key)
    pub fn window(&self, command: WindowCommand, timeout: Duration) -> Result<WindowInfo> {
        let msg = match command {
            WindowCommand::Show => Value::object([("t", Value::text("gui")), ("on", Value::bool(true))]),
            WindowCommand::Hide => Value::object([("t", Value::text("gui")), ("on", Value::bool(false))]),
            WindowCommand::Resize(w, h) => Value::object([
                ("t", Value::text("gui")),
                ("resize", Value::list([Value::int(i64::from(w)), Value::int(i64::from(h))])),
            ]),
        };
        let r = self.request(&msg, &["gui"], timeout)?;
        let open = r.get("on").and_then(Value::as_bool).unwrap_or(false);
        self.gui_open.store(open, Ordering::Release);
        let at = |i: usize| {
            r.get("size")
                .and_then(|s| s.at(i))
                .and_then(Value::as_int)
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| (1..=MAX_WINDOW).contains(v))
        };
        Ok(WindowInfo {
            open,
            size: at(0).zip(at(1)),
            reason: r.get("reason").and_then(Value::as_text).map(|t| t.chars().take(MAX_REASON).collect()),
        })
    }

    /// The plugin's window is shown now (the user may have closed it since: the helper says so)
    pub fn window_is_open(&self) -> bool {
        self.gui_open.load(Ordering::Acquire)
    }

    /// The helper crashed, was killed or ended
    pub fn is_dead(&self) -> bool {
        self.state.dead.load(Ordering::Acquire)
    }

    pub fn is_hung(&self) -> bool {
        self.state.hung.load(Ordering::Acquire)
    }

    /// Blocks the proxy missed since the start
    pub fn missed(&self) -> u32 {
        self.state.missed_total.load(Ordering::Relaxed)
    }

    /// Kills the helper (the node goes silent)
    pub fn kill(&self) {
        let _ = self.process.child.lock().unwrap_or_else(PoisonError::into_inner).kill();
    }

    pub fn state(&self) -> &Arc<ProxyState> {
        &self.state
    }
}

impl Drop for Hosted {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let words = self.shm.words();
        words[w::QUIT].store(1, Ordering::Release);
        lm_sys_ipc::wake(&words[w::REQUEST], 1);
        // Alone, `quit` ends the helper (its process is then waited for when the last reference goes); in a group it
        // ends this instance only
        let _ = self.process.send(&Value::object([("t", Value::text("quit"))]), self.id);
        self.process.routes.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.id);
        if let Some(t) = self.supervisor.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = t.join();
        }
    }
}

/// The helper program's path, checked
pub fn helper_exists(config: &HostConfig) -> bool {
    Path::new(&config.binary).exists() || which(&config.binary)
}

fn which(name: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).exists()))
}
