//! The session's services the settings, export and project screens use (owner: agent D): opening, saving and
//! creating a project folder, exporting, recovering interrupted takes, safe mode, the device's calibration, reopening
//! and listing, and MIDI port aliases. Each long one is an operation (I-2) whose effect lands in the pump; the
//! results go to `status()` and `preferences()` with the shapes of `lm_port::trees`.
//!
//! The prototype's cases come from the folder's name: "Podcast" is locked by another program (H10), "Remix" was saved
//! by a newer version (H11), "Ensaio" has takes a crash interrupted (U5), "Trilha" opens in safe mode (U21), "sumiu"
//! is a folder that is not there.

use std::time::Duration;

use lm_base::{Label, Result};
use lm_port::{ActionId, Severity, Target};
use lm_state::{PathStep, Value};

use crate::{FakePort, err};

fn text(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(Value::as_text).map(str::to_string)
}

fn name_of(folder: &str) -> String {
    folder.trim_end_matches('/').rsplit('/').next().unwrap_or(folder).trim_end_matches(".zip").to_string()
}

fn o(pairs: Vec<(&str, Value)>) -> Value {
    Value::object(pairs)
}

fn real(x: f64) -> Value {
    Value::real(x).unwrap_or_default()
}

impl FakePort {
    fn set_status(&mut self, key: &str, v: Value) {
        self.status = self.status.with(key, v);
    }

    fn set_pref(&mut self, path: &[&str], v: Value) {
        let path: Vec<PathStep> = path.iter().map(|k| PathStep::key(k)).collect();
        if let Ok(p) = self.preferences.with_path(&path, v) {
            self.preferences = p;
        }
    }

    /// The project's status after opening or saving into `folder`
    fn project_status(name: &str, folder: &str, read_only: bool, locked_by: Value, newer: Value) -> Value {
        o(vec![
            ("name", Value::text(name)),
            ("folder", Value::text(folder)),
            ("readOnly", Value::bool(read_only)),
            ("lockedBy", locked_by),
            ("newerVersion", newer),
            ("migratedFrom", Value::NULL),
            ("unused", o(vec![("files", Value::int(9)), ("mb", real(412.0))])),
        ])
    }

    fn push_recent(&mut self, name: &str, folder: &str) {
        let others: Vec<Value> = self
            .status
            .get("recent")
            .map(|l| l.items().filter(|r| r.get("folder").and_then(Value::as_text) != Some(folder)).cloned().collect())
            .unwrap_or_default();
        let first =
            o(vec![("name", Value::text(name)), ("folder", Value::text(folder)), ("opened", Value::text("agora"))]);
        self.set_status("recent", Value::list(std::iter::once(first).chain(others)));
    }
}

/// Runs one of these actions; `None` when the action is not one of them
pub(crate) fn run(p: &mut FakePort, action: &str, params: &Value) -> Option<Result<()>> {
    Some(match action {
        "project.open" => {
            let Some(folder) = text(params, "folder") else { return Some(Err(err("port.params"))) };
            let name = name_of(&folder);
            p.start(Label::new("Opening the project"), Target::Project, true, Duration::from_millis(900), move |p| {
                if folder.contains("sumiu") {
                    p.push_notice(
                        "Could not open {0}: its folder is not there",
                        vec![("project", Value::text(name))],
                        Severity::Error,
                        Some(Target::Project),
                        vec![(ActionId::from("project.open"), o(vec![("folder", Value::text(folder))]))],
                    );
                    return;
                }
                let locked = folder.contains("Podcast");
                let newer = folder.contains("Remix");
                let locked_by = if locked {
                    o(vec![
                        ("host", Value::text("estudio-b")),
                        ("pid", Value::int(48213)),
                        ("since", Value::text("09:41")),
                    ])
                } else {
                    Value::NULL
                };
                let newer_v = if newer {
                    o(vec![("version", Value::text("1.3")), ("format", Value::int(5))])
                } else {
                    Value::NULL
                };
                p.set_status("project", FakePort::project_status(&name, &folder, locked || newer, locked_by, newer_v));
                let takes = if folder.contains("Ensaio") {
                    Value::list([
                        o(vec![
                            ("track", Value::text("Voz")),
                            ("input", Value::text("Entrada 1")),
                            ("bar", Value::int(33)),
                            ("file", Value::text("audio/rec-voz.rf64")),
                            ("seconds", real(74.5)),
                        ]),
                        o(vec![
                            ("track", Value::text("Guitarra")),
                            ("input", Value::text("Entrada 2")),
                            ("bar", Value::int(33)),
                            ("file", Value::text("audio/rec-gtr.rf64")),
                            ("seconds", real(74.5)),
                        ]),
                    ])
                } else {
                    Value::empty_list()
                };
                p.set_status("interruptedTakes", takes);
                let safe = folder.contains("Trilha");
                p.set_status("safeMode", Value::bool(safe));
                p.set_status(
                    "safeModePlugins",
                    if safe {
                        Value::list([o(vec![
                            ("node", Value::text("p:7")),
                            ("plugin", Value::text("Vital (VST3)")),
                            ("reason", Value::text("stopped answering")),
                        ])])
                    } else {
                        Value::empty_list()
                    },
                );
                p.push_recent(&name, &folder);
                if locked {
                    p.push_notice(
                        "{0} is open in another program; it opened read-only",
                        vec![("project", Value::text(name))],
                        Severity::Warning,
                        Some(Target::Project),
                        Vec::new(),
                    );
                } else if newer {
                    p.push_notice(
                        "{0} was saved by a newer version; it opened read-only and is never saved over",
                        vec![("project", Value::text(name))],
                        Severity::Warning,
                        Some(Target::Project),
                        Vec::new(),
                    );
                }
            });
            Ok(())
        }
        "project.save_as" => {
            let Some(folder) = text(params, "folder") else { return Some(Err(err("port.params"))) };
            let name = name_of(&folder);
            p.start(Label::new("Saving"), Target::Project, false, Duration::from_millis(250), move |p| {
                p.set_status("project", FakePort::project_status(&name, &folder, false, Value::NULL, Value::NULL));
                p.saved_serial = p.serial;
                p.push_recent(&name, &folder);
            });
            Ok(())
        }
        "project.new" => {
            let Some(folder) = text(params, "folder") else { return Some(Err(err("port.params"))) };
            let name = name_of(&folder);
            p.start(Label::new("Creating the project"), Target::Project, false, Duration::from_millis(400), move |p| {
                let mut st = FakePort::project_status(&name, &folder, false, Value::NULL, Value::NULL);
                st = st.with("unused", o(vec![("files", Value::int(0)), ("mb", real(0.0))]));
                p.set_status("project", st);
                p.set_status("interruptedTakes", Value::empty_list());
                p.set_status("safeModePlugins", Value::empty_list());
                p.push_recent(&name, &folder);
            });
            Ok(())
        }
        "project.export" => {
            let folder = text(params, "folder").unwrap_or_default();
            let name = text(params, "name").unwrap_or_else(|| "export".into());
            let formats: Vec<String> = params
                .get("formats")
                .map(|l| l.items().filter_map(|f| f.as_text().map(str::to_string)).collect())
                .unwrap_or_else(|| vec!["wav".into()]);
            let fast = params.get("range").and_then(|r| r.get("fast")).and_then(Value::as_bool).unwrap_or(false);
            let full = folder.contains("cheio");
            p.start(Label::new("Exporting"), Target::Export(1), true, Duration::from_millis(1500), move |p| {
                let file = format!("{folder}/{name}.{}", formats.first().map_or("wav", String::as_str));
                let report = if full {
                    o(vec![
                        ("file", Value::text(file)),
                        ("ok", Value::bool(false)),
                        ("error", Value::text("The export failed: there is not enough space on the disk")),
                    ])
                } else {
                    o(vec![
                        ("file", Value::text(file)),
                        ("files", Value::int(formats.len() as i64)),
                        ("seconds", real(192.0)),
                        ("peakDb", real(-0.3)),
                        ("lufs", real(-13.8)),
                        ("ok", Value::bool(true)),
                        ("identical", Value::bool(!fast)),
                    ])
                };
                p.set_status("lastExport", report);
            });
            Ok(())
        }
        "project.export_zip" => {
            let folder = p
                .status
                .get("project")
                .and_then(|x| x.get("folder"))
                .and_then(Value::as_text)
                .unwrap_or("projeto")
                .to_string();
            p.start(Label::new("Packing the project"), Target::Project, true, Duration::from_millis(2000), move |p| {
                p.push_notice(
                    "Exported the project as {0}",
                    vec![("file", Value::text(format!("{folder}.zip")))],
                    Severity::Info,
                    None,
                    Vec::new(),
                );
            });
            Ok(())
        }
        "project.clean_unused" => {
            let unused = p.status.get("project").and_then(|x| x.get("unused")).cloned().unwrap_or(Value::NULL);
            let files = unused.get("files").and_then(Value::as_int).unwrap_or(0);
            let mb = unused.get("mb").and_then(Value::as_real).unwrap_or(0.0);
            if let Some(proj) = p.status.get("project").cloned() {
                p.set_status("project", proj.with("unused", o(vec![("files", Value::int(0)), ("mb", real(0.0))])));
            }
            p.push_notice(
                "Moved {0} unused files ({1} MB) to the trash; the undo history was cleared",
                vec![("files", Value::int(files)), ("mb", Value::int(mb as i64))],
                Severity::Info,
                Some(Target::Project),
                Vec::new(),
            );
            Ok(())
        }
        "project.save_template" => {
            let name =
                p.status.get("project").and_then(|x| x.get("name")).and_then(Value::as_text).unwrap_or("").to_string();
            p.push_notice(
                "Saved {0} as a template for new projects",
                vec![("project", Value::text(name))],
                Severity::Info,
                None,
                Vec::new(),
            );
            Ok(())
        }
        "takes.recover" | "takes.discard" => {
            // one take by its index, or all of them
            let keep = action == "takes.recover";
            let takes: Vec<Value> =
                p.status.get("interruptedTakes").map(|l| l.items().cloned().collect()).unwrap_or_default();
            let index = params.get("index").and_then(Value::as_int);
            let rest: Vec<Value> = takes
                .into_iter()
                .enumerate()
                .filter(|(i, _)| index.is_some_and(|k| k as usize != *i))
                .map(|(_, t)| t)
                .collect();
            p.set_status("interruptedTakes", Value::list(rest));
            if keep {
                p.serial += 1;
            }
            Ok(())
        }
        "safe_mode.enable" => {
            let node = text(params, "node").unwrap_or_default();
            let rest: Vec<Value> = p
                .status
                .get("safeModePlugins")
                .map(|l| {
                    l.items()
                        .filter(|x| x.get("node").and_then(Value::as_text) != Some(node.as_str()))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let empty = rest.is_empty();
            p.set_status("safeModePlugins", Value::list(rest));
            if empty {
                p.set_status("safeMode", Value::bool(false));
            }
            Ok(())
        }
        "device.reopen" => {
            p.start(Label::new("Opening the audio device"), Target::Device, false, Duration::from_millis(450), |p| {
                let a = p.preferences.get("audio").cloned().unwrap_or(Value::NULL);
                let (input, output) = (a.get("input").and_then(Value::as_text).unwrap_or(""), a.get("output").and_then(Value::as_text).unwrap_or(""));
                if input != output && !a.get("driftCorrection").and_then(Value::as_bool).unwrap_or(false) && !input.is_empty() {
                    p.push_notice(
                        "The audio device did not open: the input and the output are on different cards; turn on correcting the drift between cards, or choose one card",
                        Vec::new(),
                        Severity::Error,
                        Some(Target::Device),
                        Vec::new(),
                    );
                    p.set_status("device", Value::NULL);
                    return;
                }
                let block = a.get("block").and_then(Value::as_int).unwrap_or(256).max(32);
                let negotiated = (block as u64).next_power_of_two() as i64;
                let rate = a.get("rate").and_then(Value::as_int).unwrap_or(48_000);
                let device = o(vec![
                    ("input", Value::text(input)),
                    ("output", Value::text(output)),
                    ("rate", Value::int(rate)),
                    ("block", Value::int(negotiated)),
                    ("latencyIn", Value::int(negotiated + 58)),
                    ("latencyOut", Value::int(negotiated + 58)),
                    ("edgeConverter", Value::bool(rate != 48_000)),
                ]);
                p.set_status("device", device);
            });
            Ok(())
        }
        "device.calibrate" => {
            p.start(Label::new("Measuring the latency"), Target::Device, true, Duration::from_millis(3000), |p| {
                let d = p.status.get("device").cloned().unwrap_or(Value::NULL);
                let pair = format!(
                    "{}|{}",
                    d.get("input").and_then(Value::as_text).unwrap_or(""),
                    d.get("output").and_then(Value::as_text).unwrap_or("")
                );
                let rate = d.get("rate").and_then(Value::as_int).unwrap_or(48_000);
                let edge = d.get("edgeConverter").and_then(Value::as_bool).unwrap_or(false);
                let entry = o(vec![
                    ("devices", Value::text(pair.clone())),
                    ("rate", Value::int(rate)),
                    ("edgeConverter", Value::bool(edge)),
                    ("residual", Value::int(23)),
                    ("spread", Value::int(1)),
                    ("measured", Value::text("agora")),
                ]);
                let others: Vec<Value> = p
                    .preferences
                    .get("audio")
                    .and_then(|a| a.get("calibrations"))
                    .map(|l| {
                        l.items()
                            .filter(|c| {
                                !(c.get("devices").and_then(Value::as_text) == Some(pair.as_str())
                                    && c.get("rate").and_then(Value::as_int) == Some(rate)
                                    && c.get("edgeConverter").and_then(Value::as_bool) == Some(edge))
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                p.set_pref(&["audio", "calibrations"], Value::list(others.into_iter().chain(std::iter::once(entry))));
                p.push_notice(
                    "Latency calibrated on {0} at {1} Hz: {2} frames beyond what the device reports",
                    vec![("devices", Value::text(pair)), ("rate", Value::int(rate)), ("residual", Value::int(23))],
                    Severity::Info,
                    Some(Target::Device),
                    Vec::new(),
                );
            });
            Ok(())
        }
        "devices.scan" => {
            p.start(
                Label::new("Looking for devices"),
                Target::Scan(lm_port::ScanKind::MidiPorts),
                true,
                Duration::from_millis(700),
                |_| {},
            );
            Ok(())
        }
        "midi.assign_alias" => {
            // { alias, client, port }: the port of this machine that plays the role of the project's alias
            let (Some(alias), Some(client), Some(port)) =
                (text(params, "alias"), text(params, "client"), text(params, "port"))
            else {
                return Some(Err(err("port.params")));
            };
            let ports: Vec<Value> = p
                .preferences
                .get("midi")
                .and_then(|m| m.get("ports"))
                .map(|l| {
                    l.items()
                        .map(|x| {
                            let hit = x.get("client").and_then(Value::as_text) == Some(client.as_str())
                                && x.get("port").and_then(Value::as_text) == Some(port.as_str());
                            if hit { x.with("alias", Value::text(alias.clone())) } else { x.clone() }
                        })
                        .collect()
                })
                .unwrap_or_default();
            p.set_pref(&["midi", "ports"], Value::list(ports));
            Ok(())
        }
        "midi.import" => {
            let Some(path) = text(params, "path") else { return Some(Err(err("port.params"))) };
            let at = params.get("at").and_then(Value::as_int).unwrap_or(0);
            midi_import(p, &path, at)
        }
        "midi.export" => {
            let Some(path) = text(params, "path") else { return Some(Err(err("port.params"))) };
            let root = p.doc.root().clone();
            p.start(
                Label::new("session.midi.exporting"),
                Target::Project,
                false,
                Duration::from_millis(300),
                move |_| {
                    let _ = std::fs::write(&path, midi_bytes(&root));
                },
            );
            Ok(())
        }
        _ => return None,
    })
}

/// An audio file into the fake's store: its resource at once (the peaks are synthesized)
pub(crate) fn import(_: &mut FakePort, path: &std::path::Path) -> lm_port::Pending<lm_state::Resource> {
    if !path.exists() {
        return lm_port::Pending::Ready(Err(err("port.import.missing")));
    }
    lm_port::Pending::Ready(Ok(crate::demo::audio(&path.display().to_string())))
}

/// The fake's MIDI import: one track per file track with notes, each note as the file has it (the session's plan,
/// channels and automation are lm-session's)
fn midi_import(p: &mut FakePort, path: &str, at: i64) -> Result<()> {
    let bytes = std::fs::read(path).map_err(|_| err("session.midi.read"))?;
    let smf = lm_smf::read(&bytes).map_err(|_| err("session.midi.read"))?;
    let ppq = i64::from(smf.ppq.max(1));
    let tracks = p.doc.root().get("tracks").map_or(0, Value::len);
    p.doc.transact(Label::new("session.midi.import"), |e| {
        let mut n = tracks;
        for (i, t) in smf.tracks.iter().enumerate() {
            let mut notes = Vec::new();
            let mut open: Vec<(u8, i64, u8)> = Vec::new();
            for ev in &t.events {
                let tick = ev.tick as i64 * 960 / ppq;
                match ev.event {
                    lm_smf::Event::NoteOn { key, velocity, .. } => open.push((key, tick, velocity)),
                    lm_smf::Event::NoteOff { key, .. } => {
                        if let Some(k) = open.iter().position(|o| o.0 == key) {
                            let (key, start, vel) = open.remove(k);
                            notes.push((key, start, (tick - start).max(1), vel));
                        }
                    }
                    _ => {}
                }
            }
            if notes.is_empty() {
                continue;
            }
            let end = notes.iter().map(|n| n.1 + n.2).max().unwrap_or(960);
            let name = t.name().map_or_else(|| format!("MIDI {}", i + 1), str::to_string);
            let track = e.insert(
                lm_state::ROOT,
                "tracks",
                n,
                o(vec![("name", Value::text(name)), ("instrument", crate::demo::plugin("tripleoscillator"))]),
            )?;
            n += 1;
            let item = e.insert(
                track,
                "items",
                0,
                o(vec![("kind", Value::text("midi")), ("start", Value::int(at)), ("length", Value::int(end))]),
            )?;
            for (key, start, len, vel) in notes {
                e.add_note(
                    item,
                    o(vec![
                        ("key", Value::int(i64::from(key))),
                        ("start", Value::int(start)),
                        ("length", Value::int(len)),
                        ("velocity", real(f64::from(vel) / 127.0)),
                    ]),
                )?;
            }
        }
        Ok(())
    })?;
    Ok(())
}

fn midi_bytes(root: &Value) -> Vec<u8> {
    let mut tracks = Vec::new();
    for t in root.get("tracks").into_iter().flat_map(Value::items) {
        let mut tr = lm_smf::Track::default();
        for item in t.get("items").into_iter().flat_map(Value::items) {
            let start = item.get("start").and_then(Value::as_int).unwrap_or(0);
            for (_, n) in item.get("notes").map(|v| v.entries().collect::<Vec<_>>()).unwrap_or_default() {
                let g = |k: &str| n.get(k).and_then(Value::as_int).unwrap_or(0);
                let on = (start + g("start")).max(0) as u64;
                let key = g("key").clamp(0, 127) as u8;
                tr.events
                    .push(lm_smf::Timed { tick: on, event: lm_smf::Event::NoteOn { channel: 0, key, velocity: 100 } });
                tr.events.push(lm_smf::Timed {
                    tick: on + g("length").max(1) as u64,
                    event: lm_smf::Event::NoteOff { channel: 0, key, velocity: 64 },
                });
            }
        }
        if !tr.events.is_empty() {
            tracks.push(tr);
        }
    }
    lm_smf::write(&lm_smf::Smf { format: 1, ppq: 960, tracks })
}
