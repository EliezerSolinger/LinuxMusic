//! LinuxMusic without an interface (interfaces.md, I-13, I-20.7; P-35).
//!
//! ```text
//! lm-headless render <project folder> <out.wav> [--from SAMPLE] [--to SAMPLE] [--tail SECONDS]
//! lm-headless render <project folder> <out.(wav|flac|ogg|mp3) | folder with --stems> [--formats wav,flac,ogg,mp3]
//!     [--depth 16|24|32] [--rate HZ] [--dither] [--normalize peak:DB | lufs:LUFS] [--fade-in S] [--fade-out S]
//!     [--stems tracks|channels] [--loop] [--repeats N] [--range-ticks FROM:TO [--fast] [--warmup S]]
//!     [--ogg-quality 0..10] [--mp3-kbps N] [--tail SECONDS]
//! lm-headless info <project folder>
//! lm-headless serve [<project folder>] [--device NAME] [--socket PATH] [--tcp ADDR] [--token TOKEN:ROLE]...
//! lm-headless slice <new project folder> [--device NAME] [--seconds S] [--input N] [--block FRAMES]
//! ```
//!
//! `slice` runs the first slice (step 10) on a device, the Dummy with a noise like a fan's when none is named:
//! record with monitoring while a synth plays through an effect, the analog rack and the master, save, close,
//! reopen, export before and after. It prints the report (latencies, xruns, the take's length and RMS, whether the
//! export after reopening is the same bit for bit) as JSON on the standard output, and fails when it is not.
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
        "usage:\n  lm-headless render <project> <out.wav> [--from S] [--to S] [--tail SECONDS]\n  lm-headless render <project> \
         <out.(wav|flac|ogg|mp3) or folder> [--formats wav,flac] [--depth N] [--rate HZ] [--dither] [--normalize \
         peak:DB|lufs:L] [--fade-in S] [--fade-out S] [--stems tracks|channels] [--loop] [--repeats N] [--range-ticks \
         A:B [--fast] [--warmup S]]\n  lm-headless info \
         <project>\n  lm-headless serve [<project>] [--device NAME] [--socket PATH] [--tcp ADDR] [--token TOKEN:ROLE]\n  \
         lm-headless slice <new project> [--device NAME] [--seconds S] [--input N] [--block FRAMES]"
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
        if SWITCHES.contains(&args[i].as_str()) {
            i += 1;
        } else if args[i].starts_with("--") {
            i += 2;
        } else {
            out.push(args[i].clone());
            i += 1;
        }
    }
    out
}

type Opened = (Value, Arc<lm_plugin::Registry>, Arc<lm_state::SchemaRegistry>);

/// Flags that stand alone (every other flag takes a value)
const SWITCHES: [&str; 3] = ["--loop", "--dither", "--fast"];

fn switch(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// The export request the flags ask for (`project.export`'s parameters), or `None` when none of the flags that
/// only the full export has was given (then `render` is the plain WAV of the whole song or of a sample range)
fn request_from_args(args: &[String], out: &str) -> Result<Option<Value>> {
    const EXTRA: [&str; 12] = [
        "--formats",
        "--depth",
        "--rate",
        "--normalize",
        "--fade-in",
        "--fade-out",
        "--stems",
        "--repeats",
        "--range-ticks",
        "--ogg-quality",
        "--mp3-kbps",
        "--warmup",
    ];
    if !EXTRA.iter().any(|f| flag(args, f).is_some()) && !SWITCHES.iter().any(|f| switch(args, f)) {
        // an extension of another format is the full export too
        let ext = std::path::Path::new(out).extension().and_then(|e| e.to_str()).unwrap_or("wav").to_lowercase();
        if ext == "wav" {
            return Ok(None);
        }
    }
    let bad = |name: &'static str| Error::new("headless.number").arg("flag", name);
    let int = |name: &'static str| flag(args, name).map(|v| v.parse::<i64>().map_err(|_| bad(name))).transpose();
    let real = |name: &'static str| flag(args, name).map(|v| v.parse::<f64>().map_err(|_| bad(name))).transpose();
    let real_value = |x: f64| Value::real(x).map_err(|_| Error::new("headless.number"));
    let stems = flag(args, "--stems");
    let path = std::path::Path::new(out);
    let (folder, name, ext) = if stems.is_some() {
        (path.to_path_buf(), "export".to_owned(), "wav".to_owned())
    } else {
        (
            path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(std::path::Path::new(".")).to_path_buf(),
            path.file_stem().and_then(|s| s.to_str()).unwrap_or("export").to_owned(),
            path.extension().and_then(|e| e.to_str()).unwrap_or("wav").to_lowercase(),
        )
    };
    let formats: Vec<Value> =
        flag(args, "--formats").unwrap_or(ext).split(',').map(|f| Value::text(f.trim())).collect();
    let mut v = Value::object([
        ("folder", Value::text(folder.display().to_string())),
        ("name", Value::text(name)),
        ("formats", Value::list(formats)),
        ("dither", Value::bool(switch(args, "--dither"))),
        ("asLoop", Value::bool(switch(args, "--loop"))),
    ]);
    for (key, n) in [
        ("depth", int("--depth")?),
        ("rate", int("--rate")?),
        ("repeats", int("--repeats")?),
        ("oggQuality", int("--ogg-quality")?),
        ("mp3Kbps", int("--mp3-kbps")?),
    ] {
        if let Some(n) = n {
            v = v.with(key, Value::int(n));
        }
    }
    for (key, x) in [("fadeIn", real("--fade-in")?), ("fadeOut", real("--fade-out")?), ("tail", real("--tail")?)] {
        if let Some(x) = x {
            v = v.with(key, real_value(x)?);
        }
    }
    if let Some(n) = flag(args, "--normalize") {
        let (kind, target) = n.split_once(':').ok_or_else(|| Error::new("headless.normalize"))?;
        let target: f64 = target.parse().map_err(|_| Error::new("headless.normalize"))?;
        v = v.with("normalize", Value::object([("kind", Value::text(kind)), ("target", real_value(target)?)]));
    }
    if let Some(by) = stems {
        if !matches!(by.as_str(), "tracks" | "channels") {
            return Err(Error::new("headless.stems").arg("by", by.as_str()));
        }
        // no `nodes`: every track (or channel) is a stem
        v = v.with("stems", Value::object([("by", Value::text(by))]));
    }
    if let Some(r) = flag(args, "--range-ticks") {
        let (a, b) = r.split_once(':').ok_or_else(|| Error::new("headless.range"))?;
        let tick = |t: &str| t.parse::<i64>().map_err(|_| Error::new("headless.range"));
        let warmup = real("--warmup")?.unwrap_or(10.0);
        v = v.with(
            "range",
            Value::object([
                ("from", Value::int(tick(a)?)),
                ("to", Value::int(tick(b)?)),
                ("fast", Value::bool(switch(args, "--fast"))),
                ("warmup", real_value(warmup)?),
            ]),
        );
    }
    Ok(Some(v))
}

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
    let request = request_from_args(args, out)?;
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
    let mut handle = match request {
        // the export screen's own path: stems, formats, loop, normalisation (`project.export`)
        Some(v) => {
            let request = lm_session::export::ExportRequest::from_value(&v)?;
            workers.start(move |cancel, progress| lm_session::export::export_request(job, &request, cancel, progress))
        }
        None => workers.start(move |cancel, progress| export(job, &target, cancel, progress)),
    };
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
        config.device = platform_device(device);
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

fn slice(args: &[String]) -> Result<()> {
    let pos = positional(args);
    let Some(folder) = pos.first() else { usage() };
    let mut config = SessionConfig::headless(std::env::temp_dir().join("linuxmusic-headless-slice"))?;
    match flag(args, "--device") {
        Some(device) => config.device = platform_device(device),
        None => config.dummy_source = Some(|| Box::new(lm_session::FanNoise::default())),
    }
    if let Some(block) = flag(args, "--block") {
        config.block = block.parse().map_err(|_| Error::new("headless.block").arg("value", block.as_str()))?;
    }
    let mut o = lm_session::slice::SliceOptions::new(PathBuf::from(folder));
    if let Some(r) = number(args, "--seconds") {
        o.record_seconds = r?;
    }
    if let Some(r) = number(args, "--input") {
        o.input = r? as i64;
    }
    let mut session = Session::new(config)?;
    let report = lm_session::slice::run_slice(&mut session, &o)?;
    session.close();
    println!("{}", to_json(&report)?);
    let yes = |k: &str| report.get(k).and_then(Value::as_bool) == Some(true);
    if !yes("identicalAfterReopen") || !yes("sameTreeAfterReopen") {
        return Err(Error::new("headless.slice.not_identical"));
    }
    Ok(())
}

fn main() {
    lm_session::crashreport::install(lm_session::prefs::data_dir().join("logs"));
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
        "slice" => slice(rest),
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// `--device NAME` on this system: ALSA on Linux, CoreAudio (output only) on macOS
fn platform_device(device: String) -> DeviceSetup {
    #[cfg(feature = "jack")]
    if device == "jack" || device.starts_with("jack:") {
        return DeviceSetup::jack(&device);
    }
    if cfg!(target_os = "macos") {
        DeviceSetup::CoreAudio { output: device, input: None }
    } else {
        DeviceSetup::Alsa { device }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_plain_wav_is_the_reference_render() {
        assert_eq!(request_from_args(&args(&["p", "/o/a.wav", "--tail", "1"]), "/o/a.wav").unwrap(), None);
    }

    #[test]
    fn the_flags_make_the_export_request() {
        let a = args(&[
            "p",
            "/o",
            "--stems",
            "tracks",
            "--formats",
            "wav,flac",
            "--depth",
            "24",
            "--loop",
            "--normalize",
            "lufs:-14",
        ]);
        let v = request_from_args(&a, "/o").unwrap().unwrap();
        assert_eq!(v.get("folder").and_then(Value::as_text), Some("/o"));
        assert_eq!(v.get("formats").map(Value::len), Some(2));
        assert_eq!(v.get("asLoop").and_then(Value::as_bool), Some(true));
        assert_eq!(v.get("stems").and_then(|s| s.get("by")).and_then(Value::as_text), Some("tracks"));
        // the request is one the session reads
        lm_session::export::ExportRequest::from_value(&v).unwrap();
        assert_eq!(positional(&a), ["p", "/o"], "switches take no value");
    }

    #[test]
    fn a_file_of_another_format_is_the_full_export() {
        let v = request_from_args(&args(&["p", "/o/a.flac"]), "/o/a.flac").unwrap().unwrap();
        assert_eq!(v.get("name").and_then(Value::as_text), Some("a"));
        assert_eq!(v.get("formats").and_then(|l| l.items().next()).and_then(Value::as_text), Some("flac"));
    }

    #[test]
    fn a_bad_normalize_is_refused() {
        assert!(request_from_args(&args(&["p", "o", "--normalize", "loud"]), "o").is_err());
    }
}
