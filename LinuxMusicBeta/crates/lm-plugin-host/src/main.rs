//! `lm-plugin-host`: the helper process that hosts one third-party plugin out of the core (interfaces.md, I-9.6).
//!
//! - `lm-plugin-host run`: hosts one instance. The core sends the shared block's descriptor on the socket it started
//!   the helper with (descriptor 3: the helper never opens a block by name, and the sealed object cannot be resized
//!   under it) and then `load` on the standard input (the format, the plugin, the saved state and parameters); the
//!   helper answers `ready` and then processes a block every time the core asks through the shared block (a thread
//!   waiting on its `REQUEST` word), while the main thread answers `save`, `gui` and `quit` on the control channel.
//!   The helper ends by itself when the core is gone: the end of the standard input, or a parent that changed
//!   (checked from a thread of its own, for a main thread stuck inside a plugin).
//! - `lm-plugin-host files <format> [--path P]...`: for the formats whose listing runs plugin code (CLAP, VST3,
//!   LADSPA), the files to list, found without running any; the scan lists each file in a helper of its own.
//! - `lm-plugin-host list <format> [--path P]... [--file F]`: the plugins of a format (or of one file).
//! - `lm-plugin-host probe <format> <uri> [--path P]... [--file F]`: instantiates, runs, saves, restores and frees
//!   one plugin, then says `probed`; the scan runs it with a deadline and blocks the plugins that crash or hang it
//!   (H15).
//!
//! The main thread is the plugin's main thread: it answers the control channel (read by a thread of its own) and
//! gives the plugin its turn every 10 ms ([`backend::Plugin::idle`]).
//!
//! Formats: `lv2`, `clap`, `vst3`, `ladspa`, `test` (the helper's own test plugins). VST2 (through wine) takes the same
//! place behind [`backend::Plugin`]. A mono plugin on a stereo channel runs as two instances ([`dual`]).

#![allow(clippy::print_stderr)]

mod backend;
mod clap;
mod dual;
mod expressions;
mod ladspa;
mod lv2;
mod shared;
mod test_plugins;
mod vst3;

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use lm_base::{Error, Result};
use lm_plugin_proxy::layout::{
    EVENT_WORDS, Layout, OUT_BEGIN, OUT_END, OUT_INDEX, OUT_WORDS, STATUS_READY, decode, get, put, w,
};
use lm_plugin_proxy::protocol::{PROTOCOL, error_to, hex, read_message, unhex, write_message};
use lm_render::EventKind;
use lm_state::Value;
use lm_sys_ipc::{FdChannel, SharedMemory};

use crate::shared::Shared;
use crate::backend::{Listed, Plugin, Transport, WindowRequest, WindowState, unknown_format};

/// The most channels a plugin gets per direction
const MAX_CHANNELS: usize = 64;
/// The priority of the helper's audio thread: the same as the core's render threads (`lm_io::rt::DEFAULT_PRIORITY`)
#[cfg(target_os = "linux")]
const AUDIO_PRIORITY: u32 = 40;
/// How long rtkit may take to answer
#[cfg(target_os = "linux")]
const RTKIT_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the plugin's main thread gets its turn
const IDLE: Duration = Duration::from_millis(10);
/// How long the main thread waits for the audio thread's block to end before a turn of its own is skipped (`idle`)
const IDLE_WAIT: Duration = Duration::from_millis(2);
/// How long a request (state, window) waits for the plugin before it is answered `plugin_host.busy`
const CONTROL_WAIT: Duration = Duration::from_secs(3);
/// How long the descriptor that goes before a `load` may take to arrive (it was sent already)
const FD_WAIT: Duration = Duration::from_secs(5);
/// After the control channel closes, the longest the helper takes to end by itself before it is ended
const EXIT_GRACE: Duration = Duration::from_secs(3);

fn say(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = write_message(&mut out, v);
}

fn error(e: &Error) -> Value {
    Value::object([("t", Value::text("error")), ("error", error_to(e))])
}

/// The files a format's listing goes through one by one (`None`: the format lists without running plugin code)
fn files(format: &str, paths: &[PathBuf], system: bool) -> Result<Option<Vec<PathBuf>>> {
    match format {
        "clap" => Ok(Some(clap::files(paths, system))),
        "vst3" => Ok(Some(vst3::files(paths, system))),
        "ladspa" => Ok(Some(ladspa::files(paths, system))),
        "lv2" | "test" => Ok(None),
        other => Err(unknown_format(other)),
    }
}

fn list(format: &str, paths: &[PathBuf], file: Option<&Path>, system: bool) -> Result<Vec<Listed>> {
    match format {
        "lv2" => lv2::list(paths),
        "clap" => clap::list(paths, file, system),
        "vst3" => vst3::list(paths, file, system),
        "ladspa" => ladspa::list(paths, file, system),
        "test" => Ok(test_plugins::list()),
        other => Err(unknown_format(other)),
    }
}

fn open(
    format: &str,
    uri: &str,
    file: Option<&Path>,
    paths: &[PathBuf],
    rate: u32,
    max_block: usize,
) -> Result<Box<dyn Plugin>> {
    match format {
        "lv2" => Ok(Box::new(lv2::Lv2::open(paths, uri, rate, max_block)?)),
        "clap" => Ok(Box::new(clap::Clap::open(paths, file, uri, rate, max_block)?)),
        "vst3" => Ok(Box::new(vst3::Vst3::open(paths, file, uri, rate, max_block)?)),
        "ladspa" => Ok(Box::new(ladspa::Ladspa::open(paths, file, uri, rate, max_block)?)),
        "test" => Ok(Box::new(test_plugins::Test::open(uri, max_block)?)),
        other => Err(unknown_format(other)),
    }
}

/// `--path P` options after the positional arguments
fn paths(args: &[String]) -> Vec<PathBuf> {
    args.windows(2).filter(|w| w[0] == "--path").map(|w| PathBuf::from(&w[1])).collect()
}

/// Whether the system's plugin folders are searched too (`--no-system`: only the `--path` ones; the tests)
fn system(args: &[String]) -> bool {
    !args.iter().any(|a| a == "--no-system")
}

/// The `--file F` option
fn file(args: &[String]) -> Option<PathBuf> {
    args.windows(2).find(|w| w[0] == "--file").map(|w| PathBuf::from(&w[1]))
}

fn probe(format: &str, uri: &str, paths: &[PathBuf], file: Option<&Path>) -> Result<()> {
    let listed = list(format, paths, file, true)?
        .into_iter()
        .find(|l| l.uri == uri)
        .ok_or_else(|| Error::new("plugin_host.not_found"))?;
    let mut p = open(format, uri, file, paths, 48_000, 256)?;
    let ins = vec![vec![0.0f32; 256]; listed.inputs];
    let mut outs = vec![vec![0.0f32; 256]; listed.outputs];
    let transport = Transport { rate: 48_000, ppq: 960, bpm: 120.0, signature: (4, 4), ..Transport::default() };
    for _ in 0..4 {
        let i: Vec<&[f32]> = ins.iter().map(Vec::as_slice).collect();
        let mut o: Vec<&mut [f32]> = outs.iter_mut().map(Vec::as_mut_slice).collect();
        p.process(256, &i, &mut o, &[], &transport);
        p.idle();
    }
    if let Some(chunk) = p.save()? {
        p.restore(&chunk)?;
    }
    drop(p);
    say(&Value::object([("t", Value::text("probed"))]));
    Ok(())
}

/// The audio thread joins the real-time group: `SCHED_FIFO` directly when the limits allow, otherwise through rtkit
/// (the same as the core's render threads), with its stack touched first. Once, before the loop. Without permission
/// the plugin plays anyway (the core already tells the user what to configure) and the helper says so on stderr
#[cfg(target_os = "linux")]
fn realtime() -> Option<lm_sys_linux::RealtimeGuard> {
    lm_sys_linux::prefault_stack(128 * 1024);
    let bus = lm_sys_linux::SystemBus::start().ok();
    match lm_sys_linux::make_current_thread_realtime(AUDIO_PRIORITY, bus.as_ref(), RTKIT_TIMEOUT) {
        Ok(guard) => Some(guard),
        Err(e) => {
            eprintln!("lm-plugin-host: the audio thread is not real time: {e}");
            None
        }
    }
}

/// The audio side: waits for a block, processes it, answers
fn audio_loop(shm: Arc<SharedMemory>, layout: Layout, plugin: Arc<Shared>, quit: Arc<AtomicBool>) {
    lm_sys_clap::set_thread_role(lm_sys_clap::ThreadRole::Audio);
    // `LM_PLUGIN_HOST_NO_RT` turns it off
    #[cfg(target_os = "linux")]
    let _realtime = if std::env::var_os("LM_PLUGIN_HOST_NO_RT").is_some() { None } else { realtime() };
    let words = shm.words();
    let rate = words[w::RATE].load(Ordering::Relaxed);
    let mut last = words[w::REQUEST].load(Ordering::Acquire);
    let mut ins = vec![vec![0.0f32; layout.max_block]; layout.inputs];
    let mut outs = vec![vec![0.0f32; layout.max_block]; layout.outputs];
    let mut events: Vec<(u32, EventKind)> = Vec::with_capacity(layout.max_events);
    let mut changes = Vec::with_capacity(layout.max_out);
    let mut gestures: Vec<(u32, bool)> = Vec::with_capacity(layout.max_out);
    let (ni, no) = (layout.inputs.min(MAX_CHANNELS), layout.outputs.min(MAX_CHANNELS));
    loop {
        if quit.load(Ordering::Acquire) || words[w::QUIT].load(Ordering::Acquire) != 0 {
            return;
        }
        let req = words[w::REQUEST].load(Ordering::Acquire);
        if req == last {
            lm_sys_ipc::wait(&words[w::REQUEST], last, Some(Duration::from_millis(100)));
            continue;
        }
        let frames = (words[w::FRAMES].load(Ordering::Relaxed) as usize).min(layout.max_block);
        for (ch, b) in ins.iter_mut().enumerate() {
            get(words, layout.input(ch), &mut b[..frames]);
        }
        events.clear();
        let n = (words[w::EVENTS].load(Ordering::Relaxed) as usize).min(layout.max_events);
        let base = layout.events();
        for k in 0..n {
            let mut x = [0u32; EVENT_WORDS];
            for (j, v) in x.iter_mut().enumerate() {
                *v = words[base + k * EVENT_WORDS + j].load(Ordering::Relaxed);
            }
            if let Some(e) = decode(x) {
                events.push(e);
            }
        }
        let sig = words[w::SIGNATURE].load(Ordering::Relaxed);
        let flags = words[w::FLAGS].load(Ordering::Relaxed);
        let pair = |lo: usize, hi: usize| {
            (u64::from(words[lo].load(Ordering::Relaxed)) | (u64::from(words[hi].load(Ordering::Relaxed)) << 32)) as i64
        };
        let transport = Transport {
            rate,
            playing: flags & 1 != 0,
            recording: flags & 2 != 0,
            position: pair(w::POSITION_LO, w::POSITION_HI),
            tick: pair(w::TICK_LO, w::TICK_HI),
            ppq: words[w::PPQ].load(Ordering::Relaxed),
            bpm: f64::from(f32::from_bits(words[w::BPM].load(Ordering::Relaxed))),
            signature: ((sig >> 8) as u8, sig as u8),
        };
        let latency;
        changes.clear();
        gestures.clear();
        // The audio thread never waits for the plugin: when the main thread holds it (state, window, a restart) this
        // block is silence and the loop goes on (see `shared`)
        if let Some(mut p) = plugin.audio() {
            // The channel lists the plugin gets, on the stack (no allocation per block)
            let mut iref: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
            let mut oref: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| <&mut [f32]>::default());
            for (r, b) in iref.iter_mut().zip(&ins) {
                *r = &b[..frames];
            }
            for (r, b) in oref.iter_mut().zip(outs.iter_mut()) {
                *r = &mut b[..frames];
            }
            p.process(frames, &iref[..ni], &mut oref[..no], &events, &transport);
            latency = p.latency();
            p.changes(&mut changes);
            p.gestures(&mut gestures);
            if p.state_changed() {
                words[w::STATE].store(1, Ordering::Release);
            }
        } else {
            for b in outs.iter_mut() {
                b[..frames].fill(0.0);
            }
            latency = words[w::LATENCY].load(Ordering::Relaxed);
        }
        for (ch, b) in outs.iter().enumerate() {
            put(words, layout.output(ch), &b[..frames]);
        }
        // The gesture edges first (the core's block of requests is short), then the values
        let at = layout.out_events();
        let mut m = 0;
        for (index, begin) in gestures.iter().take(layout.max_out) {
            let flag = if *begin { OUT_BEGIN } else { OUT_END };
            words[at + m * OUT_WORDS].store((index & OUT_INDEX) | flag, Ordering::Relaxed);
            words[at + m * OUT_WORDS + 1].store(0, Ordering::Relaxed);
            m += 1;
        }
        for (index, value) in changes.iter().take(layout.max_out - m) {
            words[at + m * OUT_WORDS].store(index & OUT_INDEX, Ordering::Relaxed);
            words[at + m * OUT_WORDS + 1].store(value.to_bits(), Ordering::Relaxed);
            m += 1;
        }
        words[w::OUT_EVENTS].store(m as u32, Ordering::Relaxed);
        words[w::LATENCY].store(latency, Ordering::Release);
        words[w::DONE].store(req, Ordering::Release);
        lm_sys_ipc::wake(&words[w::DONE], 1);
        last = req;
    }
}

/// The parameters' values now, by key (with the state: a state carries its parameters too)
fn param_values(p: &dyn Plugin) -> Value {
    Value::object(
        p.params()
            .iter()
            .enumerate()
            .map(|(i, x)| (x.key.as_str(), Value::real(f64::from(p.param(i as u32))).unwrap_or(Value::NULL))),
    )
}

/// The helper's answer about a window
fn gui_reply(state: &WindowState) -> Value {
    let size =
        state.size.map_or(Value::NULL, |(w, h)| Value::list([Value::int(i64::from(w)), Value::int(i64::from(h))]));
    Value::object([
        ("t", Value::text("gui")),
        ("on", Value::bool(state.open)),
        ("size", size),
        ("reason", state.reason.map_or(Value::NULL, Value::text)),
    ])
}

/// One plugin instance the helper runs: its plugin, its shared block and its audio thread
struct Slot {
    plugin: Arc<Shared>,
    shm: Arc<SharedMemory>,
    quit: Arc<AtomicBool>,
    audio: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Slot {
    /// A slot that goes (a group's `load` with a number in use replaces the old one) ends its audio thread too
    fn drop(&mut self) {
        self.stop();
    }
}

impl Slot {
    /// Ends the audio thread (the instance is dropped with the slot)
    fn stop(&mut self) {
        self.quit.store(true, Ordering::Release);
        lm_sys_ipc::wake(&self.shm.words()[w::REQUEST], 1);
        if let Some(a) = self.audio.take() {
            let _ = a.join();
        }
    }
}

/// Loads the plugin a `load` message names, starts its audio thread; returns the slot and the `ready` answer
fn load_slot(load: &Value, id: i64, paths: &[PathBuf], fds: &FdChannel) -> Result<(Slot, Value)> {
    let text = |k: &str| lm_plugin_proxy::protocol::text(load, k);
    let format = text("format")?.to_owned();
    let uri = text("uri")?.to_owned();
    let file = load.get("file").and_then(Value::as_text).map(PathBuf::from);
    let words = lm_plugin_proxy::protocol::int(load, "words")?;
    // The block came over the socket before this message, tagged with the instance's number; the core made it, sized
    // it and (on Linux) sealed it, and `from_fd` checks all that
    let (fd, tag) = fds.recv(FD_WAIT)?;
    if tag != id as u64 {
        return Err(Error::new("plugin_host.block"));
    }
    let shm = SharedMemory::from_fd(fd, usize::try_from(words).map_err(|_| Error::new("plugin_host.message"))?)?;
    let layout = Layout::read_header(shm.words()).ok_or_else(|| Error::new("plugin_host.block"))?;
    let rate = shm.words()[w::RATE].load(Ordering::Relaxed);
    let mut plugin = open(&format, &uri, file.as_deref(), paths, rate, layout.max_block)?;
    // A mono plugin on a stereo channel: two instances, one per channel
    if dual::applies(plugin.io(), layout.inputs, layout.outputs) {
        let second = open(&format, &uri, file.as_deref(), paths, rate, layout.max_block)?;
        plugin = Box::new(dual::DualMono::new(plugin, second));
    } else if dual::spreads(plugin.io(), layout.inputs, layout.outputs) {
        // A mono instrument on two outputs plays on both
        plugin = Box::new(dual::Spread::new(plugin));
    }
    // A state the plugin cannot take back leaves it at its defaults (the project still opens), and says so
    let restored = match load.get("chunk").and_then(Value::as_text) {
        Some(h) => unhex(h).and_then(|c| plugin.restore(&c)).is_ok(),
        None => true,
    };
    if let Some(params) = load.get("params") {
        for (k, v) in params.entries() {
            if let (Some(i), Some(x)) = (plugin.params().iter().position(|p| p.key == k), v.as_real()) {
                plugin.set_param(i as u32, x as f32);
            }
        }
    }
    // A restored state may ask for a restart (a new latency): the plugin is not processing yet, so it happens now
    plugin.idle();
    let ready = Value::object([
        ("t", Value::text("ready")),
        ("latency", Value::int(i64::from(plugin.latency()))),
        ("restored", Value::bool(restored)),
        ("params", Value::list(plugin.params().iter().map(|p| p.to_value()))),
    ]);
    shm.words()[w::LATENCY].store(plugin.latency(), Ordering::Release);
    shm.words()[w::STATUS].store(STATUS_READY, Ordering::Release);
    let shm = Arc::new(shm);
    let plugin = Arc::new(Shared::new(plugin));
    let quit = Arc::new(AtomicBool::new(false));
    let audio = {
        let (shm, plugin, quit) = (Arc::clone(&shm), Arc::clone(&plugin), Arc::clone(&quit));
        std::thread::Builder::new()
            .name("lm-plugin-audio".into())
            .spawn(move || audio_loop(shm, layout, plugin, quit))
            .map_err(|e| Error::new("plugin_host.thread").arg("error", e.to_string()))?
    };
    Ok((Slot { plugin, shm, quit, audio: Some(audio) }, ready))
}

/// The answer to one control message for one instance (`None`: the message was `quit`)
fn answer(slot: &Slot, m: &Value, gui_used: &mut bool) -> Option<Value> {
    let plugin = &slot.plugin;
    Some(match m.get("t").and_then(Value::as_text) {
        Some("save") => {
            let mut p = match plugin.main(CONTROL_WAIT) {
                Ok(p) => p,
                Err(e) => return Some(error(&e)),
            };
            match p.save() {
                Ok(c) => Value::object([
                    ("t", Value::text("state")),
                    ("chunk", c.map_or(Value::NULL, |c| Value::text(hex(&c)))),
                    ("params", param_values(p.as_ref())),
                ]),
                Err(e) => error(&e),
            }
        }
        Some("restore") => {
            let r = m
                .get("chunk")
                .and_then(Value::as_text)
                .ok_or_else(|| Error::new("plugin_host.message"))
                .and_then(unhex)
                .and_then(|c| plugin.main(CONTROL_WAIT)?.restore(&c));
            match r {
                Ok(()) => Value::object([("t", Value::text("restored"))]),
                Err(e) => error(&e),
            }
        }
        Some("gui") => {
            *gui_used = true;
            let request = match (m.get("on").and_then(Value::as_bool), m.get("resize")) {
                (_, Some(size)) => {
                    let at = |i: usize| size.at(i).and_then(Value::as_int).and_then(|v| u32::try_from(v).ok());
                    match (at(0), at(1)) {
                        (Some(w), Some(h)) => WindowRequest::Resize(w, h),
                        _ => WindowRequest::Hide,
                    }
                }
                (Some(true), None) => WindowRequest::Show,
                _ => WindowRequest::Hide,
            };
            match plugin.main(CONTROL_WAIT).and_then(|mut p| p.window(request)) {
                Ok(state) => gui_reply(&state),
                Err(e) => error(&e),
            }
        }
        Some("quit") => return None,
        _ => error(&Error::new("plugin_host.message")),
    })
}

/// A message with the instance it is about (a group's messages and answers carry `"i"`)
fn tagged(v: Value, id: Option<i64>) -> Value {
    match id {
        Some(i) => v.with("i", Value::int(i)),
        None => v,
    }
}

/// Hosts the plugins the core loads. Alone (the default, one process per instance): the first message is the one
/// `load`, untagged, and `quit` ends the helper. In a group (`--group`): any number of `load`s, each with an `"i"`
/// the core chose, every later message and answer about an instance carries its `"i"`, `quit` with an `"i"` ends that
/// instance only, and the end of the standard input ends all. The main thread is every plugin's main thread
fn run(paths: &[PathBuf], group: bool) -> Result<()> {
    // The socket the shared blocks' descriptors arrive on (the core started the helper with it)
    let fds = FdChannel::inherited()?;
    say(&Value::object([
        ("t", Value::text("hello")),
        ("protocol", Value::int(PROTOCOL)),
        ("group", Value::bool(group)),
    ]));
    let mut input = BufReader::new(std::io::stdin());
    // The control channel, read by a thread of its own; this thread is the plugins' main thread and gives them their
    // turn between messages
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("lm-plugin-control".into())
        .spawn(move || {
            while let Ok(Some(m)) = read_message(&mut input) {
                if tx.send(m).is_err() {
                    break;
                }
            }
            // The control channel closed (the core is gone, or sent what cannot be read): the main loop ends the
            // helper in order, and if the main thread is stuck inside a plugin the helper ends anyway
            std::thread::sleep(EXIT_GRACE);
            lm_sys_ipc::exit_now(0);
        })
        .map_err(|e| Error::new("plugin_host.thread").arg("error", e.to_string()))?;
    let mut slots: std::collections::BTreeMap<i64, Slot> = std::collections::BTreeMap::new();
    // Alone, the one instance is loaded before anything else, and a failure ends the helper
    if !group {
        let Ok(load) = rx.recv() else { return Ok(()) };
        let (slot, ready) = load_slot(&load, 0, paths, &fds)?;
        slots.insert(0, slot);
        say(&ready);
    }
    // Once a window was asked for, this thread also runs the window system's loop (macOS needs it on the main thread)
    let mut gui_used = false;
    loop {
        let waited = if gui_used {
            match rx.try_recv() {
                Ok(m) => Ok(m),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    lm_sys_gui::pump(IDLE);
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                }
            }
        } else {
            rx.recv_timeout(IDLE)
        };
        let m = match waited {
            Ok(m) => m,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                for (id, slot) in &slots {
                    // A plugin the audio thread is still processing keeps its turn: this one is skipped
                    let Some(mut p) = slot.plugin.main_for(IDLE_WAIT) else { continue };
                    p.idle();
                    // The user closed a window: the core is told, so its button shows the truth
                    if p.window_closed() {
                        drop(p);
                        say(&tagged(Value::object([("t", Value::text("gui_closed"))]), group.then_some(*id)));
                    }
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let id = if group { m.get("i").and_then(Value::as_int) } else { Some(0) };
        let Some(id) = id else {
            say(&error(&Error::new("plugin_host.message")));
            continue;
        };
        let reply_id = group.then_some(id);
        if group && m.get("t").and_then(Value::as_text) == Some("load") {
            let reply = match load_slot(&m, id, paths, &fds) {
                Ok((slot, ready)) => {
                    slots.insert(id, slot);
                    ready
                }
                Err(e) => error(&e),
            };
            say(&tagged(reply, reply_id));
            continue;
        }
        let Some(slot) = slots.get(&id) else {
            say(&tagged(error(&Error::new("plugin_host.message")), reply_id));
            continue;
        };
        match answer(slot, &m, &mut gui_used) {
            Some(reply) => say(&tagged(reply, reply_id)),
            None if group => {
                // This instance only: its audio thread ends, its plugin is dropped (the window with it)
                if let Some(mut gone) = slots.remove(&id) {
                    gone.stop();
                }
            }
            None => break,
        }
    }
    for slot in slots.values_mut() {
        slot.stop();
    }
    Ok(())
}

fn main() {
    // Whatever the helper was asked to do, it does not outlive the core that started it
    lm_sys_ipc::exit_when_parent_dies();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&paths(&args), args.iter().any(|a| a == "--group")),
        Some("files") => match args.get(1) {
            Some(format) => files(format, &paths(&args), system(&args)).map(|f| match f {
                Some(files) => files.iter().for_each(|p| {
                    say(&Value::object([("t", Value::text("file")), ("path", Value::text(p.display().to_string()))]))
                }),
                None => say(&Value::object([("t", Value::text("no_files"))])),
            }),
            None => Err(Error::new("plugin_host.usage")),
        },
        Some("list") => match args.get(1) {
            Some(format) => list(format, &paths(&args), file(&args).as_deref(), system(&args))
                .map(|l| l.iter().for_each(|p| say(&p.to_value()))),
            None => Err(Error::new("plugin_host.usage")),
        },
        Some("probe") => match (args.get(1), args.get(2)) {
            (Some(f), Some(u)) => probe(f, u, &paths(&args), file(&args).as_deref()),
            _ => Err(Error::new("plugin_host.usage")),
        },
        _ => Err(Error::new("plugin_host.usage")),
    };
    if let Err(e) = result {
        say(&error(&e));
        eprintln!("lm-plugin-host: {e}");
        let _ = std::io::stdout().flush();
        std::process::exit(1);
    }
}
