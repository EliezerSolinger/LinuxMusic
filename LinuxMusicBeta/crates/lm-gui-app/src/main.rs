//! LinuxMusicBeta, the program: the interface (lm-gui) on a real core.
//!
//! ```text
//! linuxmusicbeta [<project folder>]
//! linuxmusicbeta --connect <host:port | unix socket path> [--token <token>]
//! ```
//!
//! - Local (the default): lm-session's `LocalPort`, with the real engine, the device of the preferences (ALSA on
//!   Linux; the Dummy device where the session has no backend for the system yet, or when the card cannot be opened,
//!   with a notice), real projects on disk and the program's own plugins with their panels. A project folder given
//!   opens at once (an operation: the 200 ms loading rule from the first window); without one, the recent projects
//!   are shown. The session's notices (safe mode, interrupted takes, a newer project read-only...) show as any other.
//! - Client (`--connect`): lm-remote's `RemotePort` to a server (`lm-headless serve`), over a Unix socket (a path)
//!   or TCP (`host:port`, through an SSH tunnel, with a token).
//!
//! The interface pumps the port every 10 ms on its own clock (H2), so the session needs no clock thread here.

#![forbid(unsafe_code)]
#![allow(clippy::print_stderr)]

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(not(unix))]
use lm_base::Error;
use lm_base::Result;
use lm_port::CorePort;
use lm_session::{DeviceSetup, LocalPort, Session, SessionConfig, internal_plugins};

const USAGE: &str = "usage:\n  linuxmusicbeta [<project folder>]\n  linuxmusicbeta --connect <host:port | socket path> [--token <token>]";

/// What the command line asked for
#[derive(Debug, PartialEq)]
enum Mode {
    Local { project: Option<PathBuf> },
    Remote { endpoint: String, token: Option<String> },
}

fn parse(args: &[String]) -> std::result::Result<Mode, String> {
    let mut project = None;
    let mut connect = None;
    let mut token = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--connect" => {
                connect = Some(args.get(i + 1).ok_or("--connect needs an address")?.clone());
                i += 2;
            }
            "--token" => {
                token = Some(args.get(i + 1).ok_or("--token needs a value")?.clone());
                i += 2;
            }
            "-h" | "--help" => return Err(String::new()),
            a if a.starts_with("--") => return Err(format!("unknown option {a}")),
            a => {
                if project.is_some() {
                    return Err("one project at a time".into());
                }
                project = Some(PathBuf::from(a));
                i += 1;
            }
        }
    }
    match connect {
        Some(endpoint) if project.is_none() => Ok(Mode::Remote { endpoint, token }),
        Some(_) => Err("a client opens the server's project, not one of its own".into()),
        None if token.is_some() => Err("--token goes with --connect".into()),
        None => Ok(Mode::Local { project }),
    }
}

/// The machine's data folder: where an untitled project lives until it is saved (`LINUXMUSIC_DATA`, portable mode)
fn data_dir() -> PathBuf {
    lm_session::prefs::data_dir()
}

/// The device the preferences name (I-10). The session has backends for ALSA (Linux), CoreAudio (macOS) and the
/// Dummy device; any other combination keeps the Dummy device running in real time (the headless configuration's)
fn device(prefs: &lm_session::prefs::Preferences) -> Option<DeviceSetup> {
    let system = prefs.text(&["audio", "system"]).unwrap_or("");
    let output = prefs.text(&["audio", "output"]).unwrap_or("");
    let output = if output.is_empty() { "default".to_owned() } else { output.to_owned() };
    if cfg!(target_os = "linux") && system == "alsa" {
        return Some(DeviceSetup::Alsa { device: output });
    }
    if cfg!(target_os = "macos") && system == "coreaudio" {
        let input = prefs.text(&["audio", "input"]).filter(|i| !i.is_empty()).map(str::to_owned);
        return Some(DeviceSetup::CoreAudio { output, input });
    }
    #[cfg(feature = "jack")]
    if system == "jack" {
        // `audio.output` may name a server (`jack:<server>`); anything else is the default server
        let device = if output.starts_with("jack:") { output } else { "jack".to_owned() };
        return Some(DeviceSetup::jack(&device));
    }
    #[cfg(not(feature = "jack"))]
    if system == "jack" {
        eprintln!("audio.system is \"jack\", but this build has no JACK support (build with --features jack)");
    }
    None
}

/// The local core: a session with the machine's preferences and device, the program's plugins, and the project
fn local(project: Option<PathBuf>) -> Result<Box<dyn CorePort>> {
    let mut config = SessionConfig::headless(data_dir().join("untitled"))?;
    let location = lm_session::prefs::location();
    let (prefs, error) = lm_session::prefs::Preferences::load(location);
    if let Some(e) = error {
        eprintln!("preferences: {e}");
    }
    if let Some(d) = device(&prefs) {
        config.device = d;
    }
    if let Some(block) = prefs.int(&["audio", "block"]) {
        config.block = u32::try_from(block).unwrap_or(256).clamp(16, 4096);
    }
    config.workers = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(2).clamp(1, 8));
    config.preferences = prefs;
    let mut session = Session::new(config)?;
    if let Some(folder) = project {
        // an operation: the interface shows it past 200 ms, with its stage and Cancel
        // its failure is a notice of the session, shown like any other
        let _operation = session.open(folder);
    }
    Ok(Box::new(LocalPort::new(session)))
}

/// The client: a RemotePort to a server, its schemas the program's own (the same program on both ends)
fn remote(endpoint: String, token: Option<String>) -> Result<Box<dyn CorePort>> {
    let schemas = Arc::new(internal_plugins()?.project_schemas()?);
    let endpoint = if endpoint.contains('/') {
        #[cfg(unix)]
        {
            lm_remote::Endpoint::Unix(PathBuf::from(endpoint))
        }
        #[cfg(not(unix))]
        {
            return Err(Error::new("app.unix_socket_unavailable"));
        }
    } else {
        lm_remote::Endpoint::Tcp(endpoint)
    };
    let mut config = lm_remote::ClientConfig::new(endpoint, schemas);
    config.token = token;
    config.name = "LinuxMusicBeta".into();
    config.telemetry_hz = 60.0;
    config.peaks_cache = Some(data_dir().join("peaks"));
    Ok(Box::new(lm_remote::RemotePort::connect(config)?))
}

fn main() {
    lm_session::crashreport::install(lm_session::prefs::data_dir().join("logs"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match parse(&args) {
        Ok(m) => m,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    // What the machine's preferences ask of a start without a project: the first run (no preferences file yet) makes
    // the default folders and opens Settings; `general.reopenLast` opens the project the last session ended on
    let mut project_arg = None;
    let mut first_run = false;
    if let Mode::Local { project } = &mode {
        let location = lm_session::prefs::location();
        first_run = !location.exists();
        let (prefs, _) = lm_session::prefs::Preferences::load(location);
        if first_run && let Err(e) = lm_session::prefs::create_default_folders(&prefs) {
            eprintln!("LinuxMusicBeta: the default folders: {e}");
        }
        if project.is_none() {
            project_arg = prefs.last_project();
        }
    }
    let mode = match mode {
        Mode::Local { project: None } if project_arg.is_some() => Mode::Local { project: project_arg },
        other => other,
    };
    let recents = matches!(mode, Mode::Local { project: None });
    let options = lm_gui::Options { title: Some("LinuxMusicBeta".into()), recents, first_run };
    lm_gui::run_with(
        move || {
            let made = match mode {
                Mode::Local { project } => local(project),
                Mode::Remote { endpoint, token } => remote(endpoint, token),
            };
            match made {
                Ok(port) => port,
                Err(e) => {
                    eprintln!("LinuxMusicBeta: {e}");
                    std::process::exit(1);
                }
            }
        },
        options,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_command_line() {
        assert_eq!(parse(&args(&[])), Ok(Mode::Local { project: None }));
        assert_eq!(parse(&args(&["/x/song"])), Ok(Mode::Local { project: Some(PathBuf::from("/x/song")) }));
        assert_eq!(
            parse(&args(&["--connect", "127.0.0.1:7890", "--token", "T"])),
            Ok(Mode::Remote { endpoint: "127.0.0.1:7890".into(), token: Some("T".into()) })
        );
        assert!(parse(&args(&["--connect"])).is_err());
        assert!(parse(&args(&["--token", "T"])).is_err());
        assert!(parse(&args(&["/x", "--connect", "a:1"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
    }
}
