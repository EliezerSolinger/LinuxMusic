//! The scan of third-party plugins (I-9.6, H15), run in an `Operation`: the helper lists a format's plugins from
//! their metadata (no plugin code runs for that), then each plugin is **probed in a helper of its own** with a
//! deadline (instantiated, run, its state saved and restored, freed). One that crashes or hangs the probe is listed
//! as blocked — the session keeps that list in the preferences and does not try it again until the user asks.

use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lm_base::{Cancel, Error, Progress, Result};
use lm_state::Value;

use crate::hosted::HostConfig;
use crate::protocol::{error_from, read_message};

/// One parameter of a scanned plugin
#[derive(Clone, Debug, PartialEq)]
pub struct ScannedParam {
    /// Its permanent key in the format (an LV2 port symbol, a CLAP param id, a LADSPA port name)
    pub key: String,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub integer: bool,
    pub toggled: bool,
    pub logarithmic: bool,
}

/// A plugin the scan found and the probe ran
#[derive(Clone, Debug, PartialEq)]
pub struct Scanned {
    pub format: String,
    pub uri: String,
    pub name: String,
    /// Takes notes (an instrument) rather than audio (an effect)
    pub instrument: bool,
    pub inputs: usize,
    pub outputs: usize,
    pub midi_in: bool,
    pub params: Vec<ScannedParam>,
    /// It reports a latency
    pub latency: bool,
    pub has_window: bool,
}

/// A plugin the probe could not run, and why (`plugin_host.crashed`, `plugin_host.timeout`, or the plugin's error)
#[derive(Clone, Debug, PartialEq)]
pub struct Blocked {
    pub format: String,
    pub uri: String,
    pub reason: Error,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanReport {
    pub plugins: Vec<Scanned>,
    pub blocked: Vec<Blocked>,
    /// Skipped because the block list had them
    pub skipped: Vec<String>,
}

fn get_f(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(|x| x.as_real().or_else(|| x.as_int().map(|i| i as f64))).unwrap_or(d)
}

fn get_b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn get_t(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_text).unwrap_or("").to_owned()
}

/// A plugin from the helper's listing
pub fn scanned_from(v: &Value) -> Option<Scanned> {
    Some(Scanned {
        format: v.get("format")?.as_text()?.to_owned(),
        uri: v.get("uri")?.as_text()?.to_owned(),
        name: get_t(v, "name"),
        instrument: get_b(v, "instrument"),
        inputs: v.get("inputs").and_then(Value::as_int).unwrap_or(0).clamp(0, 64) as usize,
        outputs: v.get("outputs").and_then(Value::as_int).unwrap_or(0).clamp(0, 64) as usize,
        midi_in: get_b(v, "midi_in"),
        params: v
            .get("params")
            .into_iter()
            .flat_map(Value::items)
            .filter_map(|p| {
                Some(ScannedParam {
                    key: p.get("key")?.as_text()?.to_owned(),
                    name: get_t(p, "name"),
                    min: get_f(p, "min", 0.0),
                    max: get_f(p, "max", 1.0),
                    default: get_f(p, "default", 0.0),
                    integer: get_b(p, "integer"),
                    toggled: get_b(p, "toggled"),
                    logarithmic: get_b(p, "logarithmic"),
                })
            })
            .collect(),
        latency: get_b(v, "latency"),
        has_window: get_b(v, "has_window"),
    })
}

/// Runs the helper with `args`, collecting its messages, within `timeout`. `Err` with `plugin_host.timeout` (it
/// was killed) or `plugin_host.crashed` (it died by a signal or a failure status)
fn run_helper(config: &HostConfig, args: &[String], timeout: Duration, cancel: &Cancel) -> Result<Vec<Value>> {
    let mut child = Command::new(&config.binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Error::new("plugin_host.spawn").arg("error", e.to_string()))?;
    let out = child.stdout.take().ok_or_else(|| Error::new("plugin_host.spawn"))?;
    let reader = std::thread::spawn(move || {
        let mut r = BufReader::new(out);
        let mut v = Vec::new();
        while let Ok(Some(m)) = read_message(&mut r) {
            v.push(m);
        }
        v
    });
    let until = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| Error::new("plugin_host.spawn").arg("error", e.to_string()))? {
            break s;
        }
        if Instant::now() >= until || cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            cancel.check()?;
            return Err(Error::new("plugin_host.timeout"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let messages = reader.join().unwrap_or_default();
    if !status.success() {
        // A failure the helper reported is its message; anything else (a signal) is a crash
        if let Some(e) = messages.iter().find(|m| m.get("t").and_then(Value::as_text) == Some("error")) {
            return Err(error_from(e.get("error").unwrap_or(&Value::NULL)));
        }
        return Err(Error::new("plugin_host.crashed").arg("status", status.to_string()));
    }
    Ok(messages)
}

/// Scans a format: lists its plugins (`paths` adds search folders), then probes each one not in `blocked_uris`,
/// each in a helper of its own within `probe_timeout`
pub fn scan(
    config: &HostConfig,
    format: &str,
    extra: &[PathBuf],
    blocked_uris: &[String],
    probe_timeout: Duration,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<ScanReport> {
    let paths: Vec<PathBuf> = config.paths.iter().chain(extra).cloned().collect();
    let mut args = vec!["list".to_owned(), format.to_owned()];
    for p in &paths {
        args.push("--path".into());
        args.push(p.display().to_string());
    }
    let listed = run_helper(config, &args, Duration::from_secs(60), cancel)?;
    let found: Vec<Scanned> = listed
        .iter()
        .filter(|m| m.get("t").and_then(Value::as_text) == Some("plugin"))
        .filter_map(scanned_from)
        .collect();
    let mut report = ScanReport::default();
    let total = found.len().max(1);
    for (i, p) in found.into_iter().enumerate() {
        cancel.check()?;
        progress.set(i as f32 / total as f32);
        if blocked_uris.contains(&p.uri) {
            report.skipped.push(p.uri);
            continue;
        }
        let mut args = vec!["probe".to_owned(), p.format.clone(), p.uri.clone()];
        for path in &paths {
            args.push("--path".into());
            args.push(path.display().to_string());
        }
        match run_helper(config, &args, probe_timeout, cancel) {
            Ok(m) if m.iter().any(|x| x.get("t").and_then(Value::as_text) == Some("probed")) => report.plugins.push(p),
            Ok(_) => {
                report.blocked.push(Blocked { format: p.format, uri: p.uri, reason: Error::new("plugin_host.probe") })
            }
            Err(e) if e.is_cancelled() => return Err(e),
            Err(reason) => report.blocked.push(Blocked { format: p.format, uri: p.uri, reason }),
        }
    }
    progress.set(1.0);
    Ok(report)
}
