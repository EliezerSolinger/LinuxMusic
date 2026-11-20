//! LinuxMusic without an interface (interfaces.md, I-13, I-20.7; P-35).
//!
//! ```text
//! lm-headless render <project folder> <out.wav> [--from SAMPLE] [--to SAMPLE] [--tail SECONDS]
//! lm-headless info <project folder>
//! lm-headless serve [<project folder>] [--device NAME] [--socket PATH] [--tcp ADDR] [--token TOKEN:ROLE]...
//! ```
//!
//! `render` is the export's path exactly (T19): the reference of the deterministic render. It renders the saved
//! file (not the journal), waits for every resource and plugin (T8), and prints the operation and its progress
//! (I-2.4) on the standard error. `serve` runs a session behind its LocalPort, pumped every 10 ms (H2) with the
//! network server (lm-remote, I-20.7: a Unix socket by default, TCP only when asked and only with tokens), printing
//! the notices and the operations. It ends when its standard input closes.

#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use lm_base::{Error, Main, Result, Role, Workers};
use lm_doc::{Document, Options};
use lm_session::export::{ExportJob, ExportTarget, export};
use lm_session::folder::ProjectFolder;
use lm_session::{DeviceSetup, Session, SessionConfig, internal_plugins};
use lm_state::{Value, from_json, to_json};

fn usage() -> ! {
    eprintln!(
        "usage:\n  lm-headless render <project> <out.wav> [--from S] [--to S] [--tail SECONDS]\n  lm-headless info \
         <project>\n  lm-headless serve [<project>] [--device NAME] [--socket PATH] [--tcp ADDR] [--token TOKEN:ROLE]"
    );
    std::process::exit(2)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn positional(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") {
            i += 2;
        } else {
            out.push(args[i].clone());
            i += 1;
        }
    }
    out
}

type Opened = (Value, Arc<lm_plugin::Registry>, Arc<lm_state::SchemaRegistry>);

/// The saved project, opened (upgraded, validated) as the session would
fn open_saved(folder: &ProjectFolder) -> Result<Opened> {
    let plugins = internal_plugins()?;
    let schemas = Arc::new(plugins.project_schemas()?);
    let path = folder.project();
    let text = std::fs::read_to_string(&path).map_err(|e| lm_session::folder::io_error("headless.read", &path, &e))?;
    let options = Options { journal: false, ports: Some(plugins.ports_fn()), ..Options::default() };
    let opened = Document::open(schemas.clone(), from_json(&text)?, options)?;
    for n in &opened.notices {
        eprintln!("notice: {n}");
    }
    Ok((opened.document.root().clone(), plugins, schemas))
}

fn number(args: &[String], name: &'static str) -> Option<Result<f64>> {
    flag(args, name).map(|v| v.parse::<f64>().map_err(|_| Error::new("headless.number").arg("flag", name)))
}

fn render(args: &[String]) -> Result<()> {
    let pos = positional(args);
    let (Some(project), Some(out)) = (pos.first(), pos.get(1)) else { usage() };
    let folder = ProjectFolder::new(PathBuf::from(project));
    let (root, plugins, schemas) = open_saved(&folder)?;
    let mut target = ExportTarget::wav(PathBuf::from(out));
    if let Some(t) = number(args, "--tail") {
        target.tail = t?;
    }
    match (number(args, "--from"), number(args, "--to")) {
        (Some(f), Some(t)) => target.range = Some((f? as i64, t? as i64)),
        (None, None) => {}
        _ => return Err(Error::new("headless.range")),
    }
    let limits = lm_render::Limits { max_nodes: 4096, max_params: 32768, ..lm_render::Limits::default() };
    let job = ExportJob { root, project: folder.root().to_path_buf(), plugins, schemas, limits };
    let workers = Workers::new("lm-headless", 1, Role::Worker);
    let mut handle = workers.start(move |cancel, progress| export(job, &target, cancel, progress));
    let mut last = String::new();
    loop {
        if let Some(result) = handle.poll() {
            let report = result?;
            eprintln!();
            println!("{}", to_json(&report)?);
            return Ok(());
        }
        let line = match handle.phase() {
            lm_base::Phase::Running { progress, stage } => {
                format!("{} {:.0}%", stage.map(|s| s.to_string()).unwrap_or_default(), progress.unwrap_or(0.0) * 100.0)
            }
            _ => String::new(),
        };
        if line != last {
            eprint!("\r{line}   ");
            last = line;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn info(args: &[String]) -> Result<()> {
    let pos = positional(args);
    let Some(project) = pos.first() else { usage() };
    let folder = ProjectFolder::new(PathBuf::from(project));
    let (root, _, _) = open_saved(&folder)?;
    let tracks = root.get("tracks").map_or(0, Value::len);
    let end = lm_session::export::song_end(&root)?;
    let rate = lm_session::project_rate(&root);
    let v = Value::object([
        ("tracks", Value::int(tracks as i64)),
        ("sampleRate", Value::int(i64::from(rate))),
        ("seconds", Value::real(end as f64 / f64::from(rate))?),
    ]);
    println!("{}", to_json(&v)?);
    Ok(())
}

fn serve(args: &[String]) -> Result<()> {
    let pos = positional(args);
    let untitled = std::env::temp_dir().join("linuxmusic-headless");
    let mut config = SessionConfig::headless(untitled)?;
    if let Some(device) = flag(args, "--device") {
        config.device = DeviceSetup::Alsa { device };
    }
    let (prefs, error) = lm_session::prefs::Preferences::load(lm_session::prefs::location());
    if let Some(e) = error {
        eprintln!("preferences: {e}");
    }
    config.preferences = prefs;
    let mut session = Session::new(config)?;
    if let Some(project) = pos.first() {
        let _ = session.open(PathBuf::from(project));
    }
    let mut server_config = lm_remote::ServerConfig::new("lm-headless");
    server_config.socket = Some(match flag(args, "--socket") {
        Some(p) => PathBuf::from(p),
        None => std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("linuxmusic.sock"),
    });
    if let Some(addr) = flag(args, "--tcp") {
        server_config.tcp = Some(addr.parse().map_err(|_| Error::new("headless.tcp").arg("address", addr.as_str()))?);
    }
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--token"
            && let Some(t) = args.get(i + 1)
        {
            let (token, role) = t.split_once(':').ok_or_else(|| Error::new("headless.token"))?;
            let role = lm_remote::Role::parse(role).ok_or_else(|| Error::new("headless.role").arg("role", role))?;
            server_config.tokens.push((token.to_owned(), role));
        }
        i += 1;
    }
    let mut server = lm_remote::Server::start(server_config)?;
    let mut port = lm_session::LocalPort::new(session);
    // Ends when the standard input closes (a supervisor, or ^D)
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut sink);
        let _ = tx.send(());
    });
    eprintln!("serving; close the standard input to stop");
    loop {
        use lm_port::CorePort;
        port.pump();
        server.pump(&mut port);
        let session = port.session_mut();
        for n in session.notices().take_new() {
            eprintln!("notice: {} {:?}", n.key, n.args);
        }
        for f in session.take_finished() {
            let failed = if f.ok { "" } else { " (failed)" };
            eprintln!("operation: {} {:?} in {:?}{failed}", f.label, f.target, f.took);
        }
        if rx.try_recv().is_ok() {
            break;
        }
        std::thread::sleep(lm_session::PUMP_PERIOD);
    }
    drop(server);
    let mut session = port.into_session();
    if session.modified() && !session.project().untitled && !session.project().read_only {
        session.save()?;
        session.settle(Duration::from_secs(10));
    }
    session.close();
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else { usage() };
    let rest = &args[1..];
    let result = match command.as_str() {
        "render" | "info" => {
            // The main thread's proof, held while the document lives here
            let Some(_main) = Main::acquire() else { std::process::exit(1) };
            if command == "render" { render(rest) } else { info(rest) }
        }
        // The session takes the proof itself
        "serve" => serve(rest),
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
