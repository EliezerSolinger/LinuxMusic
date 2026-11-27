//! The scan of third-party plugins (I-9.6, H15), run in an `Operation`: the helper lists a format's plugins from
//! their metadata (no plugin code runs for that), then each plugin is **probed in a helper of its own** with a
//! deadline (instantiated, run, its state saved and restored, freed). One that crashes or hangs the probe is listed
//! as blocked — the session keeps that list in the preferences and does not try it again until the user asks.

use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lm_base::{Cancel, Error, Progress, Result};
use lm_state::Value;

use crate::hosted::HostConfig;
use crate::protocol::{error_from, read_message};

/// One parameter of a scanned plugin
#[derive(Clone, Debug, PartialEq)]
pub struct ScannedParam {
    /// Its permanent key in the format (an LV2 port symbol, a CLAP param id, a LADSPA `p<port index>`)
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
    /// The file it was found in (CLAP, VST3, LADSPA): tried first when it is loaded; the id is what identifies it
    pub file: Option<String>,
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

/// How many messages one helper run may send before it is stopped
const MAX_MESSAGES: usize = 100_000;
/// How many parameters of one plugin are taken (each key is interned for good)
const MAX_PARAMS: usize = 2048;
/// The longest parameter key taken; a longer one is skipped
const MAX_KEY: usize = 160;
/// The largest magnitude taken for a parameter's range or default
const MAX_RANGE: f64 = 1.0e9;
/// The longest name, id or path taken as text from a helper
const MAX_TEXT: usize = 1024;

fn get_f(v: &Value, k: &str, d: f64) -> f64 {
    let x = v.get(k).and_then(|x| x.as_real().or_else(|| x.as_int().map(|i| i as f64))).unwrap_or(d);
    if x.is_finite() { x.clamp(-MAX_RANGE, MAX_RANGE) } else { d }
}

fn limited(t: &str) -> String {
    t.chars().take(MAX_TEXT).collect()
}

fn get_b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn get_t(v: &Value, k: &str) -> String {
    limited(v.get(k).and_then(Value::as_text).unwrap_or(""))
}

impl Scanned {
    /// As the helper lists it (and the session keeps it in the preferences); [`scanned_from`] reads it back
    pub fn to_value(&self) -> Value {
        let r = |x: f64| Value::real(x).unwrap_or(Value::NULL);
        let params = self.params.iter().map(|p| {
            Value::object([
                ("key", Value::text(p.key.as_str())),
                ("name", Value::text(p.name.as_str())),
                ("min", r(p.min)),
                ("max", r(p.max)),
                ("default", r(p.default)),
                ("integer", Value::bool(p.integer)),
                ("toggled", Value::bool(p.toggled)),
                ("logarithmic", Value::bool(p.logarithmic)),
            ])
        });
        Value::object([
            ("t", Value::text("plugin")),
            ("format", Value::text(self.format.as_str())),
            ("uri", Value::text(self.uri.as_str())),
            ("name", Value::text(self.name.as_str())),
            ("instrument", Value::bool(self.instrument)),
            ("inputs", Value::int(self.inputs as i64)),
            ("outputs", Value::int(self.outputs as i64)),
            ("midi_in", Value::bool(self.midi_in)),
            ("params", Value::list(params)),
            ("latency", Value::bool(self.latency)),
            ("has_window", Value::bool(self.has_window)),
            ("file", self.file.as_deref().map_or(Value::NULL, Value::text)),
        ])
    }
}

/// A plugin from the helper's listing
pub fn scanned_from(v: &Value) -> Option<Scanned> {
    Some(Scanned {
        // The id is the format and the uri as they are: one that is too long is not a plugin of ours
        format: v.get("format")?.as_text().filter(|t| t.len() <= MAX_TEXT)?.to_owned(),
        uri: v.get("uri")?.as_text().filter(|t| t.len() <= MAX_TEXT)?.to_owned(),
        name: get_t(v, "name"),
        instrument: get_b(v, "instrument"),
        inputs: v.get("inputs").and_then(Value::as_int).unwrap_or(0).clamp(0, 64) as usize,
        outputs: v.get("outputs").and_then(Value::as_int).unwrap_or(0).clamp(0, 64) as usize,
        midi_in: get_b(v, "midi_in"),
        params: v
            .get("params")
            .into_iter()
            .flat_map(Value::items)
            .take(MAX_PARAMS)
            .filter_map(|p| {
                let key = p.get("key")?.as_text()?;
                if key.len() > MAX_KEY {
                    return None;
                }
                Some(ScannedParam {
                    key: key.to_owned(),
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
        file: v.get("file").and_then(Value::as_text).map(limited),
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
    // A helper that talks without end is stopped: the reader keeps no more than `MAX_MESSAGES` and says so
    let flooded = Arc::new(AtomicBool::new(false));
    let reader = {
        let flooded = Arc::clone(&flooded);
        std::thread::spawn(move || {
            let mut r = BufReader::new(out);
            let mut v = Vec::new();
            while let Ok(Some(m)) = read_message(&mut r) {
                if v.len() >= MAX_MESSAGES {
                    flooded.store(true, Ordering::Release);
                    break;
                }
                v.push(m);
            }
            v
        })
    };
    let until = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(Error::new("plugin_host.spawn").arg("error", e.to_string()));
            }
        }
        if flooded.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(Error::new("plugin_host.flood"));
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
/// each in a helper of its own within `probe_timeout`.
///
/// For the formats whose listing runs plugin code (CLAP, VST3, LADSPA) the helper first names the files, without
/// running any; then each file is listed in a helper of its own within `probe_timeout`. A file that crashes or hangs
/// its listing is blocked as `file:<path>` (and skipped next time)
pub fn scan(
    config: &HostConfig,
    format: &str,
    extra: &[PathBuf],
    blocked_uris: &[String],
    probe_timeout: Duration,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<ScanReport> {
    let paths: Vec<PathBuf> = config.paths_for(Some(format)).into_iter().chain(extra.iter().cloned()).collect();
    let with_paths = |mut args: Vec<String>| {
        for p in &paths {
            args.push("--path".into());
            args.push(p.display().to_string());
        }
        if !config.system_paths {
            args.push("--no-system".into());
        }
        args
    };
    let plugins_of = |messages: &[Value]| -> Vec<Scanned> {
        messages
            .iter()
            .filter(|m| m.get("t").and_then(Value::as_text) == Some("plugin"))
            .filter_map(scanned_from)
            .collect()
    };
    let mut report = ScanReport::default();
    let named =
        run_helper(config, &with_paths(vec!["files".into(), format.to_owned()]), Duration::from_secs(60), cancel)?;
    let files: Vec<String> = named
        .iter()
        .filter(|m| m.get("t").and_then(Value::as_text) == Some("file"))
        .filter_map(|m| m.get("path").and_then(Value::as_text).map(str::to_owned))
        .collect();
    let by_file = !named.iter().any(|m| m.get("t").and_then(Value::as_text) == Some("no_files"));
    let found: Vec<Scanned> = if by_file {
        let mut found = Vec::new();
        for (i, f) in files.iter().enumerate() {
            cancel.check()?;
            progress.set(0.5 * i as f32 / files.len().max(1) as f32);
            let uri = format!("file:{f}");
            if blocked_uris.contains(&uri) {
                report.skipped.push(uri);
                continue;
            }
            let args = with_paths(vec!["list".into(), format.to_owned(), "--file".into(), f.clone()]);
            match run_helper(config, &args, probe_timeout, cancel) {
                Ok(m) => found.extend(plugins_of(&m)),
                Err(e) if e.is_cancelled() => return Err(e),
                Err(reason) => report.blocked.push(Blocked { format: format.to_owned(), uri, reason }),
            }
        }
        found
    } else {
        plugins_of(&run_helper(
            config,
            &with_paths(vec!["list".into(), format.to_owned()]),
            Duration::from_secs(60),
            cancel,
        )?)
    };
    let total = found.len().max(1);
    for (i, p) in found.into_iter().enumerate() {
        cancel.check()?;
        progress.set(if by_file { 0.5 + 0.5 * i as f32 / total as f32 } else { i as f32 / total as f32 });
        if blocked_uris.contains(&p.uri) {
            report.skipped.push(p.uri);
            continue;
        }
        let mut args = vec!["probe".to_owned(), p.format.clone(), p.uri.clone()];
        if let Some(f) = &p.file {
            args.push("--file".into());
            args.push(f.clone());
        }
        let args = with_paths(args);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_format_searches_its_own_folders_and_the_shared_ones() {
        let mut config = HostConfig::find();
        config.paths = vec!["/shared".into()];
        config.format_paths.insert("lv2".into(), vec!["/lv2".into()]);
        config.format_paths.insert("clap".into(), vec!["/clap".into()]);
        assert_eq!(config.paths_for(Some("lv2")), [PathBuf::from("/shared"), PathBuf::from("/lv2")]);
        assert_eq!(config.paths_for(Some("vst3")), [PathBuf::from("/shared")]);
        // a group's helper serves any format: every folder
        assert_eq!(config.paths_for(None).len(), 3);
    }

    fn plugin(params: Vec<Value>) -> Value {
        Value::object([
            ("format", Value::text("test")),
            ("uri", Value::text("x")),
            ("inputs", Value::int(1_000_000)),
            ("params", Value::list(params)),
        ])
    }

    #[test]
    fn what_the_helper_lists_is_limited() {
        let long = "k".repeat(MAX_KEY + 1);
        let many = (0..MAX_PARAMS + 10).map(|i| Value::object([("key", Value::text(format!("p{i}")))]));
        let mut params: Vec<Value> = vec![Value::object([("key", Value::text(long))])];
        params.extend(many);
        let s = scanned_from(&plugin(params)).unwrap();
        // The long key is skipped, and no more than the cap are taken
        assert!(s.params.len() < MAX_PARAMS + 10 && s.params.iter().all(|p| p.key.len() <= MAX_KEY));
        assert_eq!(s.inputs, 64);
        // A range beyond the cap is brought back into it
        let p = Value::object([("key", Value::text("a")), ("min", Value::int(i64::MIN / 2)), ("max", Value::int(i64::MAX / 2))]);
        let s = scanned_from(&plugin(vec![p])).unwrap();
        assert!(s.params[0].min >= -MAX_RANGE && s.params[0].max <= MAX_RANGE);
        // An id too long to be one is not a plugin
        let v = Value::object([("format", Value::text("test")), ("uri", Value::text("u".repeat(MAX_TEXT + 1)))]);
        assert!(scanned_from(&v).is_none());
    }
}
