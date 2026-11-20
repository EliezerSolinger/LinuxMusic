//! A helper process with one plugin instance in it (I-9.6), from the core's side: starting it (an `Operation` of
//! the session: it may take seconds), its control channel, the state, the plugin's window, and the supervisor thread
//! that notices a crash and kills a hung helper.
//!
//! One helper per instance (decisoes-autonomas.md): a plugin that crashes or hangs takes nothing else with it, and a
//! hung one is killed without stopping another plugin's audio. The helper's audio thread waits on the shared block,
//! so an idle helper costs no CPU.

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lm_base::{Error, Result};
use lm_state::Value;
use lm_sys_ipc::SharedMemory;

use crate::layout::{Layout, w};
use crate::protocol::{PROTOCOL, error_from, hex, read_message, unhex, write_message};
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
    /// The plugin's id in its format: an LV2 URI, a CLAP id, a VST3 class id, a LADSPA `file:label`
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

/// A running helper
pub struct Hosted {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    replies: Mutex<Receiver<Value>>,
    state: Arc<ProxyState>,
    stop: Arc<AtomicBool>,
    supervisor: Mutex<Option<JoinHandle<()>>>,
    shm: Arc<SharedMemory>,
    pub ready: Ready,
}

impl std::fmt::Debug for Hosted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hosted").field("ready", &self.ready).finish_non_exhaustive()
    }
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

/// Starts a helper, loads the plugin, and gives the proxy the renderer will run and the helper's handle (for the
/// state and the window). Blocking for up to `load_timeout`: run it in an `Operation`
pub fn start(config: &HostConfig, req: &LoadRequest) -> Result<(Proxy, Arc<Hosted>)> {
    let layout = Layout::new(req.max_block, req.inputs, req.outputs, req.sidechain);
    let mut shm = SharedMemory::create(&lm_sys_ipc::unique_name(), layout.words())?;
    layout.write_header(shm.words(), req.rate);
    let mut command = Command::new(&config.binary);
    command.arg("run");
    for p in &config.paths {
        command.arg("--path").arg(p);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| io("plugin_host.spawn", &e).arg("program", config.binary.display().to_string()))?;
    let stdin = child.stdin.take().ok_or_else(|| Error::new("plugin_host.spawn"))?;
    let stdout = child.stdout.take().ok_or_else(|| Error::new("plugin_host.spawn"))?;
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("lm-plugin-host-out".into())
        .spawn(move || {
            let mut r = BufReader::new(stdout);
            while let Ok(Some(v)) = read_message(&mut r) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| io("plugin_host.spawn", &e))?;
    let until = Instant::now() + config.load_timeout;
    let fail = |child: &mut Child, e: Error| {
        let _ = child.kill();
        let _ = child.wait();
        e
    };
    let hello = match wait_reply(&rx, until, &["hello"]) {
        Ok(h) => h,
        Err(e) => return Err(fail(&mut child, e)),
    };
    if hello.get("protocol").and_then(Value::as_int) != Some(PROTOCOL) {
        return Err(fail(&mut child, Error::new("plugin_host.protocol")));
    }
    let params =
        Value::object(req.params.iter().map(|(k, v)| (k.as_str(), Value::real(f64::from(*v)).unwrap_or(Value::NULL))));
    let load = Value::object([
        ("t", Value::text("load")),
        ("format", Value::text(req.format.as_str())),
        ("uri", Value::text(req.uri.as_str())),
        ("file", req.file.as_deref().map_or(Value::NULL, Value::text)),
        ("shm", Value::text(shm.name())),
        ("words", Value::int(layout.words() as i64)),
        ("chunk", req.chunk.as_ref().map_or(Value::NULL, |c| Value::text(hex(c)))),
        ("params", params),
    ]);
    let mut stdin = stdin;
    if let Err(e) = write_message(&mut stdin, &load) {
        return Err(fail(&mut child, e));
    }
    let ready = match wait_reply(&rx, until, &["ready"]) {
        Ok(r) => r,
        Err(e) => return Err(fail(&mut child, e)),
    };
    // The helper has it mapped: the name can go
    shm.unlink();
    let latency = ready.get("latency").and_then(Value::as_int).unwrap_or(0).clamp(0, i64::from(u32::MAX)) as u32;
    shm.words()[w::LATENCY].store(latency, Ordering::Release);
    let shm = Arc::new(shm);
    let state = Arc::new(ProxyState::default());
    let stop = Arc::new(AtomicBool::new(false));
    let hosted = Arc::new(Hosted {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        replies: Mutex::new(rx),
        state: Arc::clone(&state),
        stop: Arc::clone(&stop),
        supervisor: Mutex::new(None),
        shm: Arc::clone(&shm),
        ready: Ready { latency, info: ready },
    });
    // The supervisor: a crash makes the node silent; a hang (declared by the proxy) kills the helper
    let watched = Arc::downgrade(&hosted);
    let sup = std::thread::Builder::new()
        .name("lm-plugin-host-watch".into())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let Some(h) = watched.upgrade() else { break };
                let mut child = h.child.lock().unwrap_or_else(PoisonError::into_inner);
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
        write_message(&mut *self.stdin.lock().unwrap_or_else(PoisonError::into_inner), msg)?;
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
            .map(|p| p.entries().filter_map(|(k, v)| Some((k.to_owned(), v.as_real()? as f32))).collect())
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
        let r =
            self.request(&Value::object([("t", Value::text("gui")), ("on", Value::bool(on))]), &["gui"], timeout)?;
        Ok(r.get("on").and_then(Value::as_bool).unwrap_or(false))
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
        let _ = self.child.lock().unwrap_or_else(PoisonError::into_inner).kill();
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
        {
            let mut stdin = self.stdin.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = write_message(&mut *stdin, &Value::object([("t", Value::text("quit"))]));
            let _ = stdin.flush();
        }
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
        drop(child);
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
