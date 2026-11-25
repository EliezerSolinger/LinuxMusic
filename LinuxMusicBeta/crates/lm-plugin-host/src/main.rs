//! `lm-plugin-host`: the helper process that hosts one third-party plugin out of the core (interfaces.md, I-9.6).
//!
//! - `lm-plugin-host run`: hosts one instance. The core sends `load` on the standard input (the format, the plugin,
//!   the shared block's name, the saved state and parameters); the helper answers `ready` and then processes a
//!   block every time the core asks through the shared block (a thread waiting on its `REQUEST` word), while the main
//!   thread answers `save`, `gui` and `quit` on the control channel. The end of the standard input (the core is gone)
//!   ends it.
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
//! Formats: `lv2`, `clap`, `vst3`, `test` (the helper's own test plugins). LADSPA and VST2 (through wine) take the same
//! place behind [`backend::Plugin`].

#![allow(clippy::print_stderr)]

mod backend;
mod clap;
mod expressions;
mod lv2;
mod test_plugins;
mod vst3;

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use lm_base::{Error, Result};
use lm_plugin_proxy::layout::{EVENT_WORDS, Layout, OUT_WORDS, STATUS_READY, decode, get, put, w};
use lm_plugin_proxy::protocol::{PROTOCOL, error_to, hex, read_message, unhex, write_message};
use lm_render::EventKind;
use lm_state::Value;
use lm_sys_ipc::SharedMemory;

use crate::backend::{Listed, Plugin, Transport, unknown_format};

/// The most channels a plugin gets per direction
const MAX_CHANNELS: usize = 64;
/// How often the plugin's main thread gets its turn
const IDLE: Duration = Duration::from_millis(10);

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
        "lv2" | "test" => Ok(None),
        other => Err(unknown_format(other)),
    }
}

fn list(format: &str, paths: &[PathBuf], file: Option<&Path>, system: bool) -> Result<Vec<Listed>> {
    match format {
        "lv2" => lv2::list(paths),
        "clap" => clap::list(paths, file, system),
        "vst3" => vst3::list(paths, file, system),
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

/// The audio side: waits for a block, processes it, answers
fn audio_loop(shm: Arc<SharedMemory>, layout: Layout, plugin: Arc<Mutex<Box<dyn Plugin>>>, quit: Arc<AtomicBool>) {
    lm_sys_clap::set_thread_role(lm_sys_clap::ThreadRole::Audio);
    let words = shm.words();
    let rate = words[w::RATE].load(Ordering::Relaxed);
    let mut last = words[w::REQUEST].load(Ordering::Acquire);
    let mut ins = vec![vec![0.0f32; layout.max_block]; layout.inputs];
    let mut outs = vec![vec![0.0f32; layout.max_block]; layout.outputs];
    let mut events: Vec<(u32, EventKind)> = Vec::with_capacity(layout.max_events);
    let mut changes = Vec::with_capacity(layout.max_out);
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
        {
            let mut p = plugin.lock().unwrap_or_else(PoisonError::into_inner);
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
            if p.state_changed() {
                words[w::STATE].store(1, Ordering::Release);
            }
        }
        for (ch, b) in outs.iter().enumerate() {
            put(words, layout.output(ch), &b[..frames]);
        }
        let at = layout.out_events();
        let m = changes.len().min(layout.max_out);
        for (k, (index, value)) in changes.iter().take(m).enumerate() {
            words[at + k * OUT_WORDS].store(*index, Ordering::Relaxed);
            words[at + k * OUT_WORDS + 1].store(value.to_bits(), Ordering::Relaxed);
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

fn run(paths: &[PathBuf]) -> Result<()> {
    say(&Value::object([("t", Value::text("hello")), ("protocol", Value::int(PROTOCOL))]));
    let mut input = BufReader::new(std::io::stdin());
    let Some(load) = read_message(&mut input)? else { return Ok(()) };
    let text = |k: &str| lm_plugin_proxy::protocol::text(&load, k);
    let format = text("format")?.to_owned();
    let uri = text("uri")?.to_owned();
    let file = load.get("file").and_then(Value::as_text).map(PathBuf::from);
    let words = lm_plugin_proxy::protocol::int(&load, "words")?;
    let shm = SharedMemory::open(text("shm")?, usize::try_from(words).map_err(|_| Error::new("plugin_host.message"))?)?;
    let layout = Layout::read_header(shm.words()).ok_or_else(|| Error::new("plugin_host.block"))?;
    let rate = shm.words()[w::RATE].load(Ordering::Relaxed);
    let mut plugin = open(&format, &uri, file.as_deref(), paths, rate, layout.max_block)?;
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
    let plugin = Arc::new(Mutex::new(plugin));
    let quit = Arc::new(AtomicBool::new(false));
    let audio = {
        let (shm, plugin, quit) = (Arc::clone(&shm), Arc::clone(&plugin), Arc::clone(&quit));
        std::thread::Builder::new()
            .name("lm-plugin-audio".into())
            .spawn(move || audio_loop(shm, layout, plugin, quit))
            .map_err(|e| Error::new("plugin_host.thread").arg("error", e.to_string()))?
    };
    say(&ready);
    // The control channel, read by a thread of its own; this thread is the plugin's main thread and gives it its turn
    // between messages
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("lm-plugin-control".into())
        .spawn(move || {
            while let Ok(Some(m)) = read_message(&mut input) {
                if tx.send(m).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| Error::new("plugin_host.thread").arg("error", e.to_string()))?;
    loop {
        let m = match rx.recv_timeout(IDLE) {
            Ok(m) => m,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                plugin.lock().unwrap_or_else(PoisonError::into_inner).idle();
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let reply = match m.get("t").and_then(Value::as_text) {
            Some("save") => {
                let mut p = plugin.lock().unwrap_or_else(PoisonError::into_inner);
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
                    .and_then(|c| plugin.lock().unwrap_or_else(PoisonError::into_inner).restore(&c));
                match r {
                    Ok(()) => Value::object([("t", Value::text("restored"))]),
                    Err(e) => error(&e),
                }
            }
            Some("gui") => {
                let on = m.get("on").and_then(Value::as_bool).unwrap_or(false);
                match plugin.lock().unwrap_or_else(PoisonError::into_inner).window(on) {
                    Ok(shown) => Value::object([("t", Value::text("gui")), ("on", Value::bool(shown))]),
                    Err(e) => error(&e),
                }
            }
            Some("quit") => break,
            _ => error(&Error::new("plugin_host.message")),
        };
        say(&reply);
    }
    quit.store(true, Ordering::Release);
    lm_sys_ipc::wake(&shm.words()[w::REQUEST], 1);
    let _ = audio.join();
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&paths(&args)),
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
