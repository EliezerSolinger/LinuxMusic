//! The actions the interface sends that the session answers (I-19): each one is a transaction, or an operation when
//! it may pass 200 ms (I-2). A child module of `session`, so it works on the session's own fields.
//!
//! ```text
//! project.new           { folder, rate?: int, template?: text }  a template is a name in the templates folder, or a folder
//! project.save_template                                          the project as a template for new ones (operation)
//! project.export_zip                                             the project and the files it cites in one .zip (operation)
//! project.clean_unused                                           the audio files nothing cites, to the system trash; the undo is cleared
//! project.open_entry    { folder | path }                        a project of the browser (the port turns its entry into the path)
//! takes.discard         { index?: int }                          the interrupted takes thrown away (absent: all)
//! safe_mode.enable      { node }                                 the plugin the safe mode bypassed, on again
//! device.reopen, device.calibrate, devices.scan                  (operations on the device)
//! folder.allow          { folder }                               a folder system files may be followed in (C9)
//! resource.locate       { path, resource | key, kind? }          the file of a missing resource, by its hash (H17)
//! resource.locate_folder{ folder }                               every missing resource found in a folder
//! plugin.load_preset    { node, path }                           (the port turns `entry` into `path`)
//! plugin.save_preset    { node, name }
//! browser.reveal        { path }                                 the file in the system's file manager
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lm_base::{Cancel, Error, Label, Progress, Result};
use lm_io::{Device, DeviceConfig, DummyOptions};
use lm_port::{ActionId, Severity, Target};
use lm_state::{NodeId, PathStep, Resource, Value, from_json, to_json};

use super::{DeviceSetup, Session, open_device, project_options, project_rate};
use crate::actions::ActionSpec;
use crate::activity::OpList;

/// The unused files and the templates are looked for again this often (host ns)
const SLOW_STATUS: u64 = 3_000_000_000;
/// A file this recent may belong to an import or a take not yet in the tree: never cleaned
const RECENT_SECONDS: u64 = 60;
/// The largest preset file read
const PRESET_MAX: u64 = 16 << 20;

fn params() -> Error {
    Error::new("action.params")
}

fn text(p: &Value, k: &str) -> Option<String> {
    p.get(k).and_then(Value::as_text).map(str::to_owned)
}

fn need(p: &Value, k: &str) -> Result<String> {
    text(p, k).filter(|t| !t.trim().is_empty()).ok_or_else(params)
}

fn node(p: &Value, k: &str) -> Result<NodeId> {
    p.get(k).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)
}

fn always(_: &Session) -> bool {
    true
}

fn writable(s: &Session) -> bool {
    !s.project().read_only
}

/// The session's actions of this module (chained into `actions::all`)
pub(crate) static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "project.save_template",
        label: "action.project.save_template",
        run: |s, _| s.save_template(),
        enabled: always,
    },
    ActionSpec {
        id: "project.save_version",
        label: "action.project.save_version",
        run: |s, _| s.save_version(),
        enabled: always,
    },
    ActionSpec {
        id: "clip.export_sample",
        label: "action.clip.export_sample",
        run: |s, p| {
            let item = need(p, "item")?;
            let item = lm_state::NodeId::parse(&item).map_err(|_| params())?;
            s.export_clip_sample(item, PathBuf::from(need(p, "path")?)).map(|_| ())
        },
        enabled: always,
    },
    ActionSpec {
        id: "project.export_zip",
        label: "action.project.export_zip",
        run: |s, _| s.export_zip(),
        enabled: always,
    },
    ActionSpec {
        id: "project.clean_unused",
        label: "action.project.clean_unused",
        run: |s, _| s.clean_unused(),
        enabled: writable,
    },
    ActionSpec {
        id: "project.open_entry",
        label: "action.project.open_entry",
        run: |s, p| {
            let folder = text(p, "folder").or_else(|| text(p, "path")).ok_or_else(params)?;
            let _ = s.open(PathBuf::from(folder));
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "takes.discard",
        label: "action.takes.discard",
        run: |s, p| s.discard_takes(p.get("index").and_then(Value::as_int)),
        enabled: writable,
    },
    ActionSpec {
        id: "safe_mode.enable",
        label: "action.safe_mode.enable",
        run: |s, p| s.safe_mode_enable(node(p, "node")?),
        enabled: writable,
    },
    ActionSpec { id: "device.reopen", label: "action.device.reopen", run: |s, _| s.device_reopen(), enabled: always },
    ActionSpec {
        id: "device.calibrate",
        label: "action.device.calibrate",
        run: |s, _| s.device_calibrate(),
        enabled: always,
    },
    ActionSpec { id: "devices.scan", label: "action.devices.scan", run: |s, _| s.devices_scan(), enabled: always },
    ActionSpec {
        id: "folder.allow",
        label: "action.folder.allow",
        run: |s, p| s.allow_folder(PathBuf::from(need(p, "folder")?)),
        enabled: always,
    },
    ActionSpec {
        id: "resource.locate",
        label: "action.resource.locate",
        run: |s, p| {
            let (kind, key) = match p.get("resource").and_then(Value::as_resource) {
                Some(r) => (r.kind().to_owned(), r.key().to_owned()),
                None => (
                    text(p, "kind").unwrap_or_else(|| "audio".to_owned()),
                    text(p, "key").or_else(|| text(p, "resource")).ok_or_else(params)?,
                ),
            };
            s.locate_resource(&kind, &key, PathBuf::from(need(p, "path")?))
        },
        enabled: writable,
    },
    ActionSpec {
        id: "resource.locate_folder",
        label: "action.resource.locate_folder",
        run: |s, p| s.locate_folder(PathBuf::from(text(p, "folder").or_else(|| text(p, "path")).ok_or_else(params)?)),
        enabled: writable,
    },
    ActionSpec {
        id: "plugin.load_preset",
        label: "action.plugin.load_preset",
        // `entry` (a browser entry) is the port's: it is turned into `path` before it gets here
        run: |s, p| s.load_preset(node(p, "node")?, Path::new(&need(p, "path")?)),
        enabled: writable,
    },
    ActionSpec {
        id: "plugin.save_preset",
        label: "action.plugin.save_preset",
        run: |s, p| s.save_preset(node(p, "node")?, &need(p, "name")?).map(|_| ()),
        enabled: always,
    },
    ActionSpec {
        id: "browser.reveal",
        label: "action.browser.reveal",
        run: |_, p| reveal(Path::new(&need(p, "path")?)),
        enabled: always,
    },
];

// ---- small helpers ----

/// A file name part: no separator, no control, never empty
fn file_part(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() { '_' } else { c })
        .collect::<String>()
        .trim()
        .trim_start_matches('.')
        .to_owned();
    if s.is_empty() { "_".to_owned() } else { s }
}

fn io_err(key: &'static str, path: &Path, e: &std::io::Error) -> Error {
    crate::folder::io_error(key, path, e)
}

/// Every resource the tree cites (kind and key), once
fn cited(root: &Value) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    root.walk(&mut |_, v| {
        if let Some(r) = v.as_resource() {
            out.insert((r.kind().to_owned(), r.key().to_owned()));
        }
    });
    out
}

/// Copies a folder's files (and its folders') into another, in order, checking the cancel between files
fn copy_dir(from: &Path, to: &Path, cancel: &Cancel) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| io_err("session.template.copy", to, &e))?;
    let Ok(read) = std::fs::read_dir(from) else { return Ok(()) };
    for entry in read.flatten() {
        cancel.check()?;
        let path = entry.path();
        let dest = to.join(entry.file_name());
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_dir() {
            copy_dir(&path, &dest, cancel)?;
        } else if meta.is_file() {
            std::fs::copy(&path, &dest).map_err(|e| io_err("session.template.copy", &dest, &e))?;
        }
    }
    Ok(())
}

// ---- the zip (stored, with zip64 records so a recording over 4 GiB fits) ----

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for (i, slot) in t.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    t
}

fn crc_update(table: &[u32; 256], crc: u32, bytes: &[u8]) -> u32 {
    let mut c = !crc;
    for b in bytes {
        c = table[((c ^ u32::from(*b)) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// What goes into the archive
enum ZipSource {
    Bytes(String, Vec<u8>),
    File(String, PathBuf),
}

struct ZipEntry {
    name: String,
    crc: u32,
    size: u64,
    offset: u64,
}

struct ZipOut<W: Write> {
    out: W,
    at: u64,
}

impl<W: Write> ZipOut<W> {
    fn put(&mut self, bytes: &[u8], path: &Path) -> Result<()> {
        self.out.write_all(bytes).map_err(|e| io_err("session.export_zip.write", path, &e))?;
        self.at += bytes.len() as u64;
        Ok(())
    }
}

fn le16(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
fn le64(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

/// A time and date for every entry: 1980-01-01 (the archive does not depend on when it was made)
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 0x21;

/// Writes the archive to `dest` (through a temporary file, renamed when whole)
fn write_zip(
    dest: &Path,
    sources: Vec<ZipSource>,
    faults: &crate::folder::DiskFaults,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<()> {
    let tmp = dest.with_extension("zip.tmp");
    let file = std::fs::File::create(&tmp).map_err(|e| io_err("session.export_zip.write", &tmp, &e))?;
    let result = (|| -> Result<()> {
        let mut z = ZipOut { out: std::io::BufWriter::new(file), at: 0 };
        let table = crc_table();
        let total = sources.len().max(1);
        let mut entries = Vec::new();
        for (i, source) in sources.into_iter().enumerate() {
            cancel.check()?;
            let (name, data): (String, Option<PathBuf>) = match &source {
                ZipSource::Bytes(n, _) => (n.clone(), None),
                ZipSource::File(n, p) => (n.clone(), Some(p.clone())),
            };
            // The size and the checksum first (the header carries them)
            let (crc, size) = match &source {
                ZipSource::Bytes(_, b) => (crc_update(&table, 0, b), b.len() as u64),
                ZipSource::File(_, p) => {
                    let mut f = std::fs::File::open(p).map_err(|e| io_err("session.export_zip.read", p, &e))?;
                    let mut buf = vec![0u8; 1 << 16];
                    let (mut crc, mut size) = (0u32, 0u64);
                    loop {
                        cancel.check()?;
                        let n = f.read(&mut buf).map_err(|e| io_err("session.export_zip.read", p, &e))?;
                        if n == 0 {
                            break;
                        }
                        crc = crc_update(&table, crc, &buf[..n]);
                        size += n as u64;
                    }
                    (crc, size)
                }
            };
            faults.check(size)?;
            let offset = z.at;
            let name_bytes = name.as_bytes();
            let mut header = Vec::with_capacity(30 + name_bytes.len() + 20);
            header.extend_from_slice(&le32(0x0403_4b50));
            header.extend_from_slice(&le16(45));
            header.extend_from_slice(&le16(0x0800)); // names are UTF-8
            header.extend_from_slice(&le16(0)); // stored
            header.extend_from_slice(&le16(DOS_TIME));
            header.extend_from_slice(&le16(DOS_DATE));
            header.extend_from_slice(&le32(crc));
            header.extend_from_slice(&le32(0xFFFF_FFFF));
            header.extend_from_slice(&le32(0xFFFF_FFFF));
            header.extend_from_slice(&le16(name_bytes.len() as u16));
            header.extend_from_slice(&le16(20));
            header.extend_from_slice(name_bytes);
            header.extend_from_slice(&le16(1));
            header.extend_from_slice(&le16(16));
            header.extend_from_slice(&le64(size));
            header.extend_from_slice(&le64(size));
            z.put(&header, &tmp)?;
            match (&source, data) {
                (ZipSource::Bytes(_, b), _) => z.put(b, &tmp)?,
                (_, Some(p)) => {
                    let mut f = std::fs::File::open(&p).map_err(|e| io_err("session.export_zip.read", &p, &e))?;
                    let mut buf = vec![0u8; 1 << 16];
                    let mut copied = 0u64;
                    loop {
                        cancel.check()?;
                        let n = f.read(&mut buf).map_err(|e| io_err("session.export_zip.read", &p, &e))?;
                        if n == 0 {
                            break;
                        }
                        z.put(&buf[..n], &tmp)?;
                        copied += n as u64;
                    }
                    if copied != size {
                        return Err(Error::new("session.export_zip.changed").arg("path", p.display().to_string()));
                    }
                }
                _ => {}
            }
            entries.push(ZipEntry { name, crc, size, offset });
            progress.set((i + 1) as f32 / total as f32);
        }
        // The central directory
        let directory = z.at;
        for e in &entries {
            let name_bytes = e.name.as_bytes();
            let mut c = Vec::with_capacity(46 + name_bytes.len() + 28);
            c.extend_from_slice(&le32(0x0201_4b50));
            c.extend_from_slice(&le16(45));
            c.extend_from_slice(&le16(45));
            c.extend_from_slice(&le16(0x0800));
            c.extend_from_slice(&le16(0));
            c.extend_from_slice(&le16(DOS_TIME));
            c.extend_from_slice(&le16(DOS_DATE));
            c.extend_from_slice(&le32(e.crc));
            c.extend_from_slice(&le32(0xFFFF_FFFF));
            c.extend_from_slice(&le32(0xFFFF_FFFF));
            c.extend_from_slice(&le16(name_bytes.len() as u16));
            c.extend_from_slice(&le16(28));
            c.extend_from_slice(&le16(0)); // comment
            c.extend_from_slice(&le16(0)); // disk
            c.extend_from_slice(&le16(0)); // internal attributes
            c.extend_from_slice(&le32(0o100_644 << 16)); // a regular file, rw-r--r--
            c.extend_from_slice(&le32(0xFFFF_FFFF));
            c.extend_from_slice(name_bytes);
            c.extend_from_slice(&le16(1));
            c.extend_from_slice(&le16(24));
            c.extend_from_slice(&le64(e.size));
            c.extend_from_slice(&le64(e.size));
            c.extend_from_slice(&le64(e.offset));
            z.put(&c, &tmp)?;
        }
        let size = z.at - directory;
        let record = z.at;
        let mut end = Vec::new();
        // zip64 end of central directory, its locator, the classic end record
        end.extend_from_slice(&le32(0x0606_4b50));
        end.extend_from_slice(&le64(44));
        end.extend_from_slice(&le16(45));
        end.extend_from_slice(&le16(45));
        end.extend_from_slice(&le32(0));
        end.extend_from_slice(&le32(0));
        end.extend_from_slice(&le64(entries.len() as u64));
        end.extend_from_slice(&le64(entries.len() as u64));
        end.extend_from_slice(&le64(size));
        end.extend_from_slice(&le64(directory));
        end.extend_from_slice(&le32(0x0706_4b50));
        end.extend_from_slice(&le32(0));
        end.extend_from_slice(&le64(record));
        end.extend_from_slice(&le32(1));
        end.extend_from_slice(&le32(0x0605_4b50));
        end.extend_from_slice(&le16(0));
        end.extend_from_slice(&le16(0));
        end.extend_from_slice(&le16(0xFFFF));
        end.extend_from_slice(&le16(0xFFFF));
        end.extend_from_slice(&le32(0xFFFF_FFFF));
        end.extend_from_slice(&le32(0xFFFF_FFFF));
        end.extend_from_slice(&le16(0));
        z.put(&end, &tmp)?;
        z.out.flush().map_err(|e| io_err("session.export_zip.write", &tmp, &e))?;
        z.out.get_ref().sync_all().map_err(|e| io_err("session.export_zip.write", &tmp, &e))?;
        Ok(())
    })();
    match result {
        Ok(()) => std::fs::rename(&tmp, dest).map_err(|e| io_err("session.export_zip.write", dest, &e)),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

// ---- the system trash ----

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// A place in `dir` for a file named `name` that is not taken
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..).map(|n| dir.join(format!("{stem} {n}{ext}"))).find(|p| std::fs::symlink_metadata(p).is_err()).unwrap_or(first)
}

/// Moves a file or renames across devices (copy, then remove)
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
    }
}

/// The date of a day number since 1970-01-01 (the civil calendar, proleptic Gregorian)
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn percent_encode(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Moves a file to the system's trash: `~/.Trash` on macOS, the freedesktop trash (`files/` and `info/`) elsewhere
fn trash(path: &Path) -> Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let fail = |e: std::io::Error| io_err("session.trash.failed", path, &e);
    if cfg!(target_os = "macos") {
        let dir = home().join(".Trash");
        std::fs::create_dir_all(&dir).map_err(fail)?;
        return move_file(path, &free_name(&dir, &name)).map_err(fail);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"))
        .join("Trash");
    let (files, info) = (base.join("files"), base.join("info"));
    std::fs::create_dir_all(&files).map_err(fail)?;
    std::fs::create_dir_all(&info).map_err(fail)?;
    let target = free_name(&files, &name);
    let stored = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let secs = crate::lock::now_seconds();
    let (y, m, d) = civil(i64::try_from(secs / 86_400).unwrap_or(0));
    let day = secs % 86_400;
    let when = format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60);
    let note = format!("[Trash Info]\nPath={}\nDeletionDate={when}\n", percent_encode(&absolute.display().to_string()));
    std::fs::write(info.join(format!("{stored}.trashinfo")), note).map_err(fail)?;
    move_file(path, &target).map_err(fail)
}

/// Shows a file in the system's file manager (it does not wait for it)
fn reveal(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err(Error::new("session.reveal.missing").arg("path", path.display().to_string()));
    }
    let mut command;
    if cfg!(target_os = "macos") {
        command = std::process::Command::new("open");
        command.arg("-R").arg(path);
    } else {
        command = std::process::Command::new("xdg-open");
        command.arg(if path.is_dir() { path } else { path.parent().unwrap_or(path) });
    }
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| io_err("session.reveal.failed", path, &e))?;
    // Something has to wait for it, or it stays a zombie
    let _ = std::thread::Builder::new().name("lm-reveal".to_owned()).spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Puts the file of a located resource into the project (checked by the caller); nothing when it is there already
fn adopt(project: &Path, kind: &str, key: &str, source: &Path, faults: &crate::folder::DiskFaults) -> Result<()> {
    let ext = source.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let (dir, ext) = if kind == "audio" {
        if !lm_io::AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            return Err(Error::new("session.locate.format").arg("path", source.display().to_string()));
        }
        (lm_io::audio_folder(project), ext)
    } else {
        let ext =
            if !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_alphanumeric()) { ext } else { "bin".to_owned() };
        (lm_io::state_folder(project), ext)
    };
    std::fs::create_dir_all(&dir).map_err(|e| io_err("session.locate.copy", &dir, &e))?;
    let dest = dir.join(format!("{key}.{ext}"));
    if dest.exists() {
        return Ok(());
    }
    let tmp = dir.join(format!("{key}.{ext}.tmp"));
    faults.check(std::fs::metadata(source).map(|m| m.len()).unwrap_or(0))?;
    crate::folder::copy_via_tmp(source, &tmp, &dest, "session.locate.copy")
}

/// A device on the Dummy backend started with the renderer (when nothing else opens)
fn dummy_device(
    renderer: lm_render::Renderer,
    rate: u32,
    block: u32,
    source: Option<fn() -> Box<dyn lm_io::DummySource>>,
) -> Option<Device> {
    let (mut device, _) = open_device(&DeviceSetup::Dummy(DummyOptions::default()), rate, block, source).ok()?;
    device.start(renderer).ok()?;
    Some(device)
}

fn put_device(slot: &Mutex<Option<Device>>, device: Device) {
    if let Ok(mut g) = slot.lock() {
        *g = Some(device);
    }
}

impl Session {
    // ---- the slow parts of the status ----

    /// `project.unused` (`{files, mb}`) and the templates, looked for on the disk every few seconds
    pub(super) fn slow_status(&mut self) -> (Value, Value) {
        let now = self.clock.now().0;
        let fresh = self.slow_status_at != 0 && now.saturating_sub(self.slow_status_at) < SLOW_STATUS;
        if fresh
            && let (Some(u), Some(t)) = (
                self.status.get("project").and_then(|p| p.get("unused")).cloned(),
                self.status.get("templates").cloned(),
            )
        {
            return (u, t);
        }
        self.slow_status_at = now.max(1);
        let unused = {
            let found = self.unused_files();
            let files = found.iter().filter(|(_, _, audio)| *audio).count() as i64;
            let bytes: u64 = found.iter().map(|(_, size, _)| *size).sum();
            Value::object([
                ("files", Value::int(files)),
                ("mb", Value::real(bytes as f64 / 1_048_576.0).unwrap_or(Value::NULL)),
            ])
        };
        (unused, self.template_list())
    }

    /// The audio files of the project's `audio/` that neither the tree nor the saved file cites (nor a recent one: it
    /// may be on its way into the tree), with their peaks: path, size, and whether it is the audio itself
    fn unused_files(&self) -> Vec<(PathBuf, u64, bool)> {
        let mut keys = BTreeSet::new();
        for root in [self.document.root(), &self.project.saved_root] {
            keys.extend(cited(root).into_iter().map(|(_, key)| key));
        }
        let mut out = Vec::new();
        let Ok(read) = std::fs::read_dir(self.project.folder.audio()) else { return out };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some((stem, ext)) = name.split_once('.') else { continue };
            let audio = lm_io::AUDIO_EXTENSIONS.contains(&ext);
            if !lm_io::is_valid_key(stem) || !(audio || ext == "peaks") || keys.contains(stem) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let recent =
                meta.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|a| a.as_secs() < RECENT_SECONDS);
            if meta.is_file() && !recent {
                out.push((entry.path(), meta.len(), audio));
            }
        }
        out.sort();
        out
    }

    /// Where the templates live: beside the preferences (none for a session without a preferences file)
    fn templates_dir(&self) -> Option<PathBuf> {
        self.preferences.file().and_then(|f| f.parent()).map(|d| d.join("templates"))
    }

    fn template_list(&self) -> Value {
        let Some(dir) = self.templates_dir() else { return Value::empty_list() };
        let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                !e.file_name().to_string_lossy().starts_with('.')
                    && e.path().join(crate::folder::PROJECT_FILE).is_file()
            })
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect();
        found.sort();
        Value::list(found.into_iter().map(|(name, path)| {
            Value::object([("name", Value::text(name)), ("folder", Value::text(path.display().to_string()))])
        }))
    }

    /// `status().device`: what the open device negotiated; null for the Dummy (no real device is open)
    pub(super) fn device_status(&self) -> Value {
        let Some(info) = self.device.as_ref().and_then(Device::info) else { return Value::NULL };
        if info.name == "dummy" {
            return Value::NULL;
        }
        let pref = |k: &str| self.preferences.text(&["audio", k]).unwrap_or("").to_owned();
        Value::object([
            ("input", Value::text(pref("input"))),
            ("output", Value::text(pref("output"))),
            ("rate", Value::int(i64::from(info.rate))),
            ("block", Value::int(info.max_block as i64)),
            ("latencyIn", Value::int(i64::from(info.input_latency))),
            ("latencyOut", Value::int(i64::from(info.output_latency))),
            ("edgeConverter", Value::bool(info.edge_resampler)),
        ])
    }

    // ---- the project: new, template, zip, clean ----

    /// A new project in `folder`, at `rate` (the default's when absent), from a template when one is named
    pub(crate) fn create_with(&mut self, folder: PathBuf, rate: Option<u32>, template: Option<String>) -> Result<()> {
        if let Some(t) = template.filter(|t| !t.trim().is_empty()) {
            return self.new_from_template(folder, &t);
        }
        let mut document = lm_doc::Document::new_desk_project(
            self.schemas.clone(),
            project_options(&self.plugins, &self.schemas, self.limits),
        )?;
        if let Some(rate) = rate
            && rate != project_rate(document.root())
        {
            if !(8_000..=384_000).contains(&rate) {
                return Err(Error::new("session.project.rate").arg("rate", i64::from(rate)));
            }
            let root = document
                .root()
                .with_path(&[PathStep::key("project"), PathStep::key("sampleRate")], Value::int(i64::from(rate)))?;
            document = lm_doc::Document::from_root(
                self.schemas.clone(),
                lm_doc::PROJECT,
                root,
                project_options(&self.plugins, &self.schemas, self.limits),
            )?
            .document;
        }
        self.replace_document(document, None)?;
        self.save_as(folder)
    }

    /// A new project made of a template (a name in the templates folder, or a folder): its files copied into `folder`
    /// with a new identity, in an operation; the project opens when they are there
    fn new_from_template(&mut self, folder: PathBuf, template: &str) -> Result<()> {
        let source = if Path::new(template).is_absolute() {
            PathBuf::from(template)
        } else {
            self.templates_dir().ok_or_else(|| Error::new("session.template.no_folder"))?.join(file_part(template))
        };
        if !source.join(crate::folder::PROJECT_FILE).is_file() {
            return Err(Error::new("session.template.missing").arg("template", template));
        }
        let to = crate::folder::ProjectFolder::new(folder.clone());
        if to.project().exists() {
            return Err(Error::new("session.template.exists").arg("folder", folder.display().to_string()));
        }
        let faults = self.faults.clone();
        self.start(
            Label::new("session.template.creating"),
            Target::Project,
            true,
            "new_from_template",
            move |cancel, progress| {
                to.create(&faults)?;
                let from = crate::folder::ProjectFolder::new(source.clone());
                let bytes =
                    std::fs::read(from.project()).map_err(|e| io_err("session.template.read", &from.project(), &e))?;
                let text = std::str::from_utf8(&bytes).map_err(|_| Error::new("session.open.not_utf8"))?;
                let tree = from_json(text)?;
                // A project of its own: its identity is new
                let tree = tree
                    .with_path(&[PathStep::key("meta"), PathStep::key("uuid")], Value::text(lm_doc::new_uuid()))
                    .unwrap_or(tree);
                progress.set(0.1);
                for dir in [crate::folder::AUDIO_DIR, "state"] {
                    copy_dir(&source.join(dir), &to.root().join(dir), cancel)?;
                }
                progress.set(0.9);
                crate::folder::write_atomic(&to.project(), to_json(&tree)?.as_bytes(), &faults)?;
                Ok(Value::object([("folder", Value::text(to.root().display().to_string()))]))
            },
        );
        Ok(())
    }

    /// The files the project cites that are in it, with their place inside the project's folder
    fn cited_files(&self) -> Vec<(PathBuf, String)> {
        let project = self.project.folder.root();
        let mut keys = cited(self.document.root());
        keys.retain(|(kind, _)| kind != "render");
        let mut out = Vec::new();
        for (kind, key) in keys {
            let Ok(Some(path)) = lm_io::resource_path_of(project, &kind, &key) else { continue };
            for file in [path.clone(), path.with_extension("peaks")] {
                if file.is_file()
                    && let Ok(rel) = file.strip_prefix(project)
                {
                    out.push((file.clone(), rel.to_string_lossy().replace('\\', "/")));
                }
            }
        }
        out
    }

    /// `project.save_version` (F-JAN-020): saves as a new folder beside the project, the name with the next version
    /// number (`song` -> `song-02` -> `song-03`)
    pub(crate) fn save_version(&mut self) -> Result<()> {
        if self.project.untitled {
            return Err(Error::new("session.project.untitled"));
        }
        let root = self.project.folder.root().to_path_buf();
        let parent = root.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let next = next_version_name(&name, |n| parent.join(n).exists());
        self.save_as(parent.join(next))
    }

    /// `project.save_template`: the project (as it is now) and the files it cites, as a template for new projects
    pub(crate) fn save_template(&mut self) -> Result<()> {
        if self.project.untitled {
            return Err(Error::new("session.project.untitled"));
        }
        let dir = self.templates_dir().ok_or_else(|| Error::new("session.template.no_folder"))?;
        let name = file_part(&self.project.folder.name());
        let tree = self.document.saved();
        let files = self.cited_files();
        let faults = self.faults.clone();
        self.start(
            Label::new("session.template.saving"),
            Target::Project,
            true,
            "save_template",
            move |cancel, progress| {
                let tmp = dir.join(format!(".{name}.saving"));
                let _ = std::fs::remove_dir_all(&tmp);
                let result = (|| -> Result<()> {
                    std::fs::create_dir_all(&tmp).map_err(|e| io_err("session.template.write", &tmp, &e))?;
                    crate::folder::write_atomic(
                        &tmp.join(crate::folder::PROJECT_FILE),
                        to_json(&tree)?.as_bytes(),
                        &faults,
                    )?;
                    for (i, (from, rel)) in files.iter().enumerate() {
                        cancel.check()?;
                        let dest = tmp.join(rel);
                        if let Some(parent) = dest.parent() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| io_err("session.template.write", parent, &e))?;
                        }
                        faults.check(std::fs::metadata(from).map(|m| m.len()).unwrap_or(0))?;
                        std::fs::copy(from, &dest).map_err(|e| io_err("session.template.write", &dest, &e))?;
                        progress.set((i + 1) as f32 / files.len().max(1) as f32);
                    }
                    let dest = dir.join(&name);
                    let _ = std::fs::remove_dir_all(&dest);
                    std::fs::rename(&tmp, &dest).map_err(|e| io_err("session.template.write", &dest, &e))
                })();
                if result.is_err() {
                    let _ = std::fs::remove_dir_all(&tmp);
                }
                result?;
                Ok(Value::object([("name", Value::text(name.as_str()))]))
            },
        );
        Ok(())
    }

    /// `project.export_zip`: the project (as it is now) and the files it cites in `<name>.zip` beside its folder
    pub(crate) fn export_zip(&mut self) -> Result<()> {
        let name = file_part(&self.project.folder.name());
        let dir = if self.project.untitled {
            PathBuf::from(self.preferences.text(&["projectsFolder"]).unwrap_or("."))
        } else {
            self.project.folder.root().parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        };
        let dest = free_name(&dir, &format!("{name}.zip"));
        let tree = to_json(&self.document.saved())?;
        let mut sources = vec![ZipSource::Bytes(format!("{name}/{}", crate::folder::PROJECT_FILE), tree.into_bytes())];
        sources
            .extend(self.cited_files().into_iter().map(|(path, rel)| ZipSource::File(format!("{name}/{rel}"), path)));
        let faults = self.faults.clone();
        self.start(
            Label::new("session.export_zip.packing"),
            Target::Project,
            true,
            "export_zip",
            move |cancel, progress| {
                std::fs::create_dir_all(&dir).map_err(|e| io_err("session.export_zip.write", &dir, &e))?;
                write_zip(&dest, sources, &faults, cancel, progress)?;
                Ok(Value::object([("file", Value::text(dest.display().to_string()))]))
            },
        );
        Ok(())
    }

    /// `project.clean_unused` (H4, P-13): the audio nothing cites goes to the system's trash, in an operation. The
    /// undo history is cleared first: what it could bring back is no longer there
    pub(crate) fn clean_unused(&mut self) -> Result<()> {
        let found = self.unused_files();
        if found.is_empty() {
            self.notices.info("session.clean_unused.none", Vec::new());
            return Ok(());
        }
        self.document.clear_history();
        self.slow_status_at = 0;
        let files = found.iter().filter(|(_, _, audio)| *audio).count() as i64;
        let bytes: u64 = found.iter().map(|(_, size, _)| *size).sum();
        self.start(
            Label::new("session.clean_unused.moving"),
            Target::Project,
            true,
            "clean_unused",
            move |cancel, progress| {
                for (i, (path, _, _)) in found.iter().enumerate() {
                    cancel.check()?;
                    trash(path)?;
                    progress.set((i + 1) as f32 / found.len() as f32);
                }
                Ok(Value::object([
                    ("files", Value::int(files)),
                    ("mb", Value::int((bytes as f64 / 1_048_576.0).round() as i64)),
                ]))
            },
        );
        Ok(())
    }

    // ---- takes and the safe mode ----

    /// `takes.discard`: the interrupted takes thrown away: the one at `index` of the status list, or all of them
    pub(crate) fn discard_takes(&mut self, index: Option<i64>) -> Result<()> {
        let dir = self.project.folder.audio();
        let records = lm_io::find_interrupted(&dir)?;
        match index {
            Some(i) => {
                let record = usize::try_from(i).ok().and_then(|i| records.get(i)).ok_or_else(params)?;
                lm_io::forget_record(record)?;
            }
            None => {
                for record in &records {
                    lm_io::forget_record(record)?;
                }
                // The MIDI takes' logs too
                for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    if entry.path().extension().is_some_and(|x| x == "midilog") {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
        Ok(())
    }

    /// `safe_mode.enable`: the plugin the safe mode bypassed is on again (one undoable step); the safe mode ends when
    /// none is left
    pub(crate) fn safe_mode_enable(&mut self, plugin: NodeId) -> Result<()> {
        if self.document.find(plugin).is_none() {
            return Err(Error::new("action.node.missing").arg("node", plugin.to_string()));
        }
        self.document.transact(Label::new("session.safe_mode.enable"), |e| {
            e.set(plugin, &[PathStep::key("bypass")], Value::bool(false))
        })?;
        let id = plugin.to_string();
        let rest: Vec<Value> = self
            .status
            .get("safeModePlugins")
            .map(|l| {
                l.items().filter(|x| x.get("node").and_then(Value::as_text) != Some(id.as_str())).cloned().collect()
            })
            .unwrap_or_default();
        if rest.is_empty() {
            self.safe_mode_on = false;
        }
        self.status =
            self.status.with("safeModePlugins", Value::list(rest)).with("safeMode", Value::bool(self.safe_mode_on));
        Ok(())
    }

    // ---- the devices ----

    /// Takes the device for an operation that gives it back through `calibrating` (`restore_device`)
    fn take_device(&mut self) -> Result<(Device, Arc<Mutex<Option<Device>>>)> {
        if self.calibrating.is_some() {
            return Err(Error::new("session.device.busy"));
        }
        let position = self.telemetry.position;
        if position.recording || position.playing || self.record_start.is_some() {
            return Err(Error::new("session.device.running"));
        }
        let device = self.device.take().ok_or_else(|| Error::new("session.device.none"))?;
        let slot = Arc::new(Mutex::new(None));
        self.calibrating = Some(slot.clone());
        Ok((device, slot))
    }

    /// The device an operation took comes back (or the Dummy the operation fell back to)
    fn restore_device(&mut self) {
        if let Some(slot) = self.calibrating.take()
            && let Ok(mut g) = slot.lock()
            && let Some(device) = g.take()
        {
            self.device = Some(device);
        }
    }

    /// `device.reopen`: the device of the preferences opens again, in an operation (the barrier, the open, the start
    /// with the same renderer). Another rate or a bigger block than the engine's is refused; the Dummy plays on
    pub(crate) fn device_reopen(&mut self) -> Result<()> {
        let audio = |k: &str| self.preferences.text(&["audio", k]).unwrap_or("").to_owned();
        let name = if audio("output").is_empty() { audio("input") } else { audio("output") };
        // The audio system of the preferences wins when this build can open it (the screen's choice of backend)
        let system = self.preferences.text(&["audio", "system"]).unwrap_or("").to_owned();
        let real = !matches!(self.device_setup, DeviceSetup::Dummy(_));
        let from_system = match system.as_str() {
            _ if !real => None,
            "alsa" if cfg!(target_os = "linux") && !matches!(self.device_setup, DeviceSetup::Alsa { .. }) => {
                Some(DeviceSetup::Alsa { device: if name.is_empty() { "default".to_owned() } else { name.clone() } })
            }
            "coreaudio" if cfg!(target_os = "macos") && !matches!(self.device_setup, DeviceSetup::CoreAudio { .. }) => {
                Some(DeviceSetup::CoreAudio {
                    output: if audio("output").is_empty() { "default".to_owned() } else { audio("output") },
                    input: Some(audio("input")).filter(|i| !i.is_empty()),
                })
            }
            #[cfg(feature = "jack")]
            "jack" if !matches!(self.device_setup, DeviceSetup::Jack { .. }) => {
                Some(DeviceSetup::jack(if name.starts_with("jack") { &name } else { "jack" }))
            }
            _ => None,
        };
        let setup = if let Some(s) = from_system {
            s
        } else {
            match &self.device_setup {
                DeviceSetup::Dummy(o) => DeviceSetup::Dummy(*o),
                DeviceSetup::Alsa { .. } if name.is_empty() => self.device_setup.clone(),
                DeviceSetup::Alsa { .. } => DeviceSetup::Alsa { device: name },
                DeviceSetup::CoreAudio { .. } => DeviceSetup::CoreAudio {
                    output: if audio("output").is_empty() { "default".to_owned() } else { audio("output") },
                    input: Some(audio("input")).filter(|i| !i.is_empty()),
                },
                // The ports and the MIDI state stay; the server may change (`jack:<server>` in the preferences)
                #[cfg(feature = "jack")]
                DeviceSetup::Jack { device, client, inputs, outputs, midi } => DeviceSetup::Jack {
                    device: if name.starts_with("jack") { name } else { device.clone() },
                    client: client.clone(),
                    inputs: *inputs,
                    outputs: *outputs,
                    midi: midi.clone(),
                },
            }
        };
        self.device_setup = setup.clone();
        let (spec, source) = (self.spec, self.dummy_source);
        let block =
            self.preferences.int(&["audio", "block"]).map_or(self.block, |b| u32::try_from(b).unwrap_or(self.block));
        let block = block.clamp(32, u32::try_from(spec.max_block).unwrap_or(u32::MAX));
        let (mut old, slot) = self.take_device()?;
        self.start(Label::new("session.device.opening"), Target::Device, false, "device.reopen", move |_, _| {
            let mut renderer = Some(match old.stop() {
                Ok(r) => r,
                // D11: the stuck device is abandoned, never dropped under its callback; the session builds a new engine
                Err(t) => {
                    std::mem::forget(old);
                    return Err(t.to_error());
                }
            });
            drop(old.poll(|_| {}));
            old.close();
            drop(old);
            let outcome: Result<Device> = match open_device(&setup, spec.sample_rate, block, source) {
                Ok((mut device, info)) => {
                    if info.rate != spec.sample_rate || info.max_block > spec.max_block {
                        Err(Error::new("io.device.spec_mismatch").arg("rate", i64::from(info.rate)))
                    } else {
                        match device.start(renderer.take().expect("the renderer")) {
                            Ok(()) => Ok(device),
                            Err(failed) => {
                                let (e, back) = failed.into_parts();
                                renderer = back;
                                Err(e)
                            }
                        }
                    }
                }
                Err(e) => Err(e),
            };
            match outcome {
                Ok(device) => {
                    put_device(&slot, device);
                    Ok(Value::NULL)
                }
                Err(e) => {
                    if let Some(r) = renderer
                        && let Some(d) = dummy_device(r, spec.sample_rate, spec.max_block as u32, source)
                    {
                        put_device(&slot, d);
                    }
                    Err(e)
                }
            }
        });
        Ok(())
    }

    /// I-10.1 / D11: the device did not stop in time, so its callback holds the renderer. The device is abandoned
    /// (not killed), and the session gets a new engine pair, a new compiler (recompiling from the tree: every
    /// processor is built again) and the Dummy device, so the program goes on; the preferences keep the real device
    /// for a later `device.reopen`
    pub(crate) fn replace_stuck_device(&mut self) {
        use std::sync::atomic::Ordering;
        self.watchdog.shared.running.store(false, Ordering::Release);
        self.device = None;
        self.record_start = None;
        self.builds_waiting.clear();
        self.build_ops.cancel_all();
        let rate = self.spec.sample_rate;
        let opened = open_device(&DeviceSetup::Dummy(DummyOptions::default()), rate, self.block, self.dummy_source);
        let (mut device, info) = match opened {
            Ok(d) => d,
            Err(e) => {
                self.notices.error(&e, Some(Target::Device), None);
                return;
            }
        };
        let spec = info.spec();
        let (engine, renderer) = lm_render::engine_with(spec, self.limits, crate::crashnote::render_threads());
        if let Err(failed) = device.start(renderer) {
            self.notices.error(failed.error(), Some(Target::Device), None);
            return;
        }
        self.engine = engine;
        self.compiler = lm_compile::Compiler::new(lm_compile::CompilerConfig {
            spec,
            limits: self.limits,
            schemas: self.schemas.clone(),
            plugins: self.plugins.clone(),
        });
        self.table = Arc::new(lm_compile::InstalledTable::default());
        self.spec = spec;
        self.device = Some(device);
        self.telemetry = crate::session::empty_telemetry(rate);
        if let Ok(mut w) = self.watchdog.shared.watch.lock() {
            *w = self.engine.watch();
        }
        self.crash.update(&self.table, &self.engine.watch());
        self.watchdog.shared.running.store(true, Ordering::Release);
        self.needs_compile = true;
        self.notices.push(
            "session.device.abandoned",
            Vec::new(),
            Severity::Warning,
            Some(Target::Device),
            vec![(ActionId::from("device.reopen"), Value::NULL)],
        );
        // A7: at the cap, hardware devices are refused until a stuck callback returns; say so up front
        let stuck = lm_io::stuck_devices();
        if stuck >= lm_io::MAX_STUCK_DEVICES {
            self.notices.warn(
                "session.device.abandoned_limit",
                vec![("stuck", Value::int(stuck as i64)), ("max", Value::int(lm_io::MAX_STUCK_DEVICES as i64))],
            );
        }
    }

    /// `device.calibrate` (I-10.4, H19): the round trip of the device measured with bursts, in an operation; the
    /// residual (beyond what the device reports) goes to the preferences, by device pair, rate and edge converter
    pub(crate) fn device_calibrate(&mut self) -> Result<()> {
        let config: DeviceConfig =
            self.device.as_ref().and_then(|d| d.config().cloned()).ok_or_else(|| Error::new("session.device.none"))?;
        let (spec, source, block) = (self.spec, self.dummy_source, self.block);
        let threads = crate::crashnote::render_threads();
        let (mut old, slot) = self.take_device()?;
        self.start(
            Label::new("session.device.calibrating"),
            Target::Device,
            true,
            "device.calibrate",
            move |cancel, progress| {
                let renderer = match old.stop() {
                    Ok(r) => r,
                    Err(t) => {
                        std::mem::forget(old);
                        return Err(t.to_error());
                    }
                };
                drop(old.poll(|_| {}));
                old.close();
                let Some(backend) = old.into_backend() else {
                    if let Some(d) = dummy_device(renderer, spec.sample_rate, block, source) {
                        put_device(&slot, d);
                    }
                    return Err(Error::new("session.device.calibrate.backend"));
                };
                let (result, backend) = lm_io::calibrate(
                    backend,
                    &config,
                    lm_io::CalibrationOptions::default(),
                    threads.clone(),
                    cancel,
                    progress,
                );
                // The device goes on as it was
                let mut renderer = Some(renderer);
                let mut device = Device::new(backend, threads);
                let restarted =
                    device.open(&config).and_then(|_| match device.start(renderer.take().expect("the renderer")) {
                        Ok(()) => Ok(()),
                        Err(failed) => {
                            let (e, back) = failed.into_parts();
                            renderer = back;
                            Err(e)
                        }
                    });
                match restarted {
                    Ok(()) => put_device(&slot, device),
                    Err(_) => {
                        if let Some(r) = renderer
                            && let Some(d) = dummy_device(r, spec.sample_rate, block, source)
                        {
                            put_device(&slot, d);
                        }
                    }
                }
                let c = result?;
                let (lo, hi) = c.measures.iter().fold((i64::MAX, i64::MIN), |(lo, hi), m| (lo.min(*m), hi.max(*m)));
                Ok(Value::object([
                    ("rate", Value::int(i64::from(c.key.rate))),
                    ("edgeConverter", Value::bool(c.key.edge_resampler)),
                    ("residual", Value::int(c.residual)),
                    ("spread", Value::int(if hi >= lo { hi - lo } else { 0 })),
                ]))
            },
        );
        Ok(())
    }

    /// `devices.scan`: the MIDI ports are looked for now, and the audio devices listed, in an operation on the scan
    pub(crate) fn devices_scan(&mut self) -> Result<()> {
        if let Some(link) = self.midi.devices.link.as_mut() {
            link.rescan_raw();
        }
        self.start(
            Label::new("session.devices.scanning"),
            Target::Scan(lm_port::ScanKind::MidiPorts),
            true,
            "devices.scan",
            move |cancel, progress| {
                #[cfg(target_os = "linux")]
                let count = lm_io::list_devices(cancel, progress)?.len() as i64;
                #[cfg(target_os = "macos")]
                let count = lm_io::coreaudio_list_devices(cancel, progress)?.len() as i64;
                #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                let count = {
                    let _ = (cancel, progress);
                    0
                };
                #[cfg(feature = "jack")]
                let count = count + lm_io::jack_list_devices(cancel, progress)?.len() as i64;
                Ok(Value::object([("devices", Value::int(count))]))
            },
        );
        Ok(())
    }

    // ---- folders and files the project cannot find ----

    /// `folder.allow` (C9): system files may be followed inside this folder; what failed for lack of it is asked for
    /// again
    pub(crate) fn allow_folder(&mut self, folder: PathBuf) -> Result<()> {
        if !folder.is_absolute() || !folder.is_dir() {
            return Err(Error::new("session.folder.not_a_folder").arg("folder", folder.display().to_string()));
        }
        let text = folder.display().to_string();
        let mut list: Vec<Value> =
            self.preferences.get(&["folders", "allowed"]).map(|l| l.items().cloned().collect()).unwrap_or_default();
        if !list.iter().any(|v| v.as_text() == Some(text.as_str())) {
            list.push(Value::text(text));
            self.set_preference(&[PathStep::key("folders"), PathStep::key("allowed")], Value::list(list))?;
        }
        self.resources.set_allowed(crate::session::allowed_folders(&self.preferences));
        self.reload_resources();
        Ok(())
    }

    /// The resources the tree cites that the project does not have (H17)
    fn missing_resources(&self) -> Vec<(String, String)> {
        let project = self.project.folder.root();
        cited(self.document.root())
            .into_iter()
            .filter(|(kind, key)| kind != "render" && matches!(lm_io::resource_path_of(project, kind, key), Ok(None)))
            .collect()
    }

    /// `resource.locate` (H17): this file is the missing resource when its hash is the key; it is copied into the
    /// project, in an operation
    pub(crate) fn locate_resource(&mut self, kind: &str, key: &str, path: PathBuf) -> Result<()> {
        let resource = Resource::new(kind, key)?;
        let project = self.project.folder.root().to_path_buf();
        let faults = self.faults.clone();
        let (kind, key) = (kind.to_owned(), key.to_owned());
        self.start(
            Label::new("session.locate.checking"),
            Target::Resource(resource.clone()),
            true,
            "locate",
            move |cancel, progress| {
                if !lm_io::verify_located(&path, &key, cancel, progress)? {
                    return Err(Error::new("session.locate.not_the_file").arg("path", path.display().to_string()));
                }
                adopt(&project, &kind, &key, &path, &faults)?;
                Ok(Value::list([Value::resource(resource)]))
            },
        );
        Ok(())
    }

    /// `resource.locate_folder` (H17): every missing resource whose file is in this folder (to four levels down)
    pub(crate) fn locate_folder(&mut self, folder: PathBuf) -> Result<()> {
        if !folder.is_dir() {
            return Err(Error::new("session.folder.not_a_folder").arg("folder", folder.display().to_string()));
        }
        let missing = self.missing_resources();
        if missing.is_empty() {
            self.notices.info("session.locate.none_missing", Vec::new());
            return Ok(());
        }
        let mut wanted: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (kind, key) in missing {
            wanted.entry(key).or_default().push(kind);
        }
        let only_audio = wanted.values().all(|kinds| kinds.iter().all(|k| k == "audio"));
        let project = self.project.folder.root().to_path_buf();
        let faults = self.faults.clone();
        self.start(Label::new("session.locate.searching"), Target::Project, true, "locate", move |cancel, progress| {
            let mut found = Vec::new();
            let mut stack = vec![(folder, 0usize)];
            let mut seen = 0usize;
            while let Some((dir, depth)) = stack.pop() {
                let mut entries: Vec<PathBuf> =
                    std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
                entries.sort();
                for path in entries {
                    cancel.check()?;
                    let hidden = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
                    let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
                    if hidden {
                        continue;
                    }
                    if meta.is_dir() {
                        if depth < 4 {
                            stack.push((path, depth + 1));
                        }
                        continue;
                    }
                    if !meta.is_file() || meta.len() == 0 {
                        continue;
                    }
                    if only_audio {
                        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                        if !lm_io::AUDIO_EXTENSIONS.contains(&ext.as_str()) {
                            continue;
                        }
                    }
                    seen += 1;
                    progress.set(1.0 - 1.0 / (1.0 + seen as f32 / 50.0));
                    let key = lm_io::hash_file(&path, cancel, &Progress::new())?;
                    if let Some(kinds) = wanted.get(&key) {
                        for kind in kinds {
                            adopt(&project, kind, &key, &path, &faults)?;
                            found.push(Value::resource(Resource::new(kind, &key)?));
                        }
                    }
                }
            }
            Ok(Value::list(found))
        });
        Ok(())
    }

    // ---- presets ----

    /// Where the presets are: `<projects folder>/Presets/<plugin>/<name>.json` (the browser's Presets page)
    pub(crate) fn presets_dir(&self) -> PathBuf {
        PathBuf::from(self.preferences.text(&["projectsFolder"]).unwrap_or(".")).join("Presets")
    }

    /// The node whose state a preset is: a track's instrument, or the plugin node itself. Its plugin id, and the
    /// path of its state under the node
    fn preset_target(&self, id: NodeId) -> Result<(String, Vec<PathStep>, Value)> {
        let found =
            self.document.find(id).ok_or_else(|| Error::new("action.node.missing").arg("node", id.to_string()))?;
        let (plugin, path) = match found.get("instrument") {
            Some(instrument) => (instrument.clone(), vec![PathStep::key("instrument"), PathStep::key("state")]),
            None => (found, vec![PathStep::key("state")]),
        };
        let id_text = plugin
            .get("plugin")
            .and_then(Value::as_text)
            .ok_or_else(|| Error::new("plugin.preset.not_a_plugin"))?
            .to_owned();
        Ok((id_text, path, plugin))
    }

    /// `plugin.load_preset`: the preset file's state on the plugin, one undoable step. Error:
    /// `plugin.preset.other_plugin`
    pub(crate) fn load_preset(&mut self, id: NodeId, file: &Path) -> Result<()> {
        let (plugin, path, _) = self.preset_target(id)?;
        let read = |e: std::io::Error| io_err("plugin.preset.read", file, &e);
        let mut bytes = Vec::new();
        std::fs::File::open(file).map_err(read)?.take(PRESET_MAX).read_to_end(&mut bytes).map_err(read)?;
        let saved = from_json(std::str::from_utf8(&bytes).map_err(|_| Error::new("plugin.preset.invalid"))?)?;
        if saved.get("plugin").and_then(Value::as_text) != Some(plugin.as_str()) {
            return Err(Error::new("plugin.preset.other_plugin").arg("plugin", plugin));
        }
        let state = saved.get("state").cloned().ok_or_else(|| Error::new("plugin.preset.invalid"))?;
        let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.document
            .transact(Label::new("action.plugin.load_preset").arg("name", name), |e| e.set(id, &path, state))?;
        Ok(())
    }

    /// `plugin.save_preset`: the plugin's state as a preset file with this name (it replaces one of the same name).
    /// The file's path
    pub(crate) fn save_preset(&mut self, id: NodeId, name: &str) -> Result<PathBuf> {
        let (plugin, _, node) = self.preset_target(id)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(params());
        }
        let state = node.get("state").cloned().unwrap_or_else(Value::empty_object);
        let dir = self.presets_dir().join(file_part(&plugin));
        std::fs::create_dir_all(&dir).map_err(|e| io_err("plugin.preset.write", &dir, &e))?;
        let file = dir.join(format!("{}.json", file_part(name)));
        let body = Value::object([("plugin", Value::text(plugin)), ("name", Value::text(name)), ("state", state)]);
        crate::folder::write_atomic(&file, to_json(&body)?.as_bytes(), &self.faults)?;
        Ok(file)
    }

    // ---- what the operations say when they finish ----

    /// The end of an operation of this module (called by the session's pump). The device comes back first, whatever
    /// happened
    pub(crate) fn core_op_finished(&mut self, kind: &'static str, target: Target, result: Result<Value>) {
        if matches!(kind, "device.calibrate" | "device.reopen") {
            self.restore_device();
        }
        match result {
            Ok(report) => self.core_op_done(kind, &report),
            Err(e) if e.is_cancelled() => {}
            Err(e) => {
                let retry = match kind {
                    "device.reopen" | "device.calibrate" => Some((ActionId::from(kind), Value::NULL)),
                    _ => None,
                };
                let stuck = matches!(e.key(), "io.device.stop_timed_out" | "io.device.start_lost") && retry.is_some();
                self.notices.error(&e, Some(target.clone()), retry);
                self.activity.fail(target, e);
                if stuck {
                    self.replace_stuck_device();
                }
            }
        }
    }

    fn core_op_done(&mut self, kind: &'static str, report: &Value) {
        match kind {
            "locate" => {
                let located: Vec<Resource> = report.items().filter_map(Value::as_resource).cloned().collect();
                if located.is_empty() {
                    self.notices.warn("session.locate.nothing", Vec::new());
                }
                for r in located {
                    self.resources.forget(r.key());
                    self.request(&r);
                }
                self.needs_compile = true;
            }
            "export_zip" => {
                let file = report.get("file").and_then(Value::as_text).unwrap_or("").to_owned();
                self.notices.info("session.export_zip.done", vec![("file", Value::text(file))]);
            }
            "save_template" => {
                let name = report.get("name").and_then(Value::as_text).unwrap_or("").to_owned();
                self.slow_status_at = 0;
                self.notices.info("session.template.saved", vec![("name", Value::text(name))]);
            }
            "clean_unused" => {
                self.slow_status_at = 0;
                let n = |k: &str| report.get(k).and_then(Value::as_int).unwrap_or(0);
                self.notices.info(
                    "session.clean_unused.done",
                    vec![("files", Value::int(n("files"))), ("mb", Value::int(n("mb")))],
                );
            }
            "new_from_template" => {
                if let Some(folder) = report.get("folder").and_then(Value::as_text) {
                    let _ = self.open(PathBuf::from(folder));
                }
            }
            "device.calibrate" => self.calibration_done(report),
            _ => {}
        }
    }

    /// The residual of a calibration into the preferences, replacing the entry of the same device pair, rate and
    /// edge converter
    fn calibration_done(&mut self, report: &Value) {
        let pref = |s: &Session, k: &str| s.preferences.text(&["audio", k]).unwrap_or("").to_owned();
        let pair = format!("{}|{}", pref(self, "input"), pref(self, "output"));
        let rate = report.get("rate").and_then(Value::as_int).unwrap_or(0);
        let edge = report.get("edgeConverter").and_then(Value::as_bool).unwrap_or(false);
        let residual = report.get("residual").and_then(Value::as_int).unwrap_or(0);
        let entry = Value::object([
            ("devices", Value::text(pair.as_str())),
            ("rate", Value::int(rate)),
            ("edgeConverter", Value::bool(edge)),
            ("residual", Value::int(residual)),
            ("spread", Value::int(report.get("spread").and_then(Value::as_int).unwrap_or(0))),
            ("measured", Value::text(crate::lock::now_seconds().to_string())),
        ]);
        let mut list: Vec<Value> = self
            .preferences
            .get(&["audio", "calibrations"])
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
        list.push(entry);
        if let Err(e) = self.set_preference(&[PathStep::key("audio"), PathStep::key("calibrations")], Value::list(list))
        {
            self.notices.error(&e, Some(Target::Device), None);
            return;
        }
        self.notices.push(
            "session.calibration.done",
            vec![("devices", Value::text(pair)), ("rate", Value::int(rate)), ("residual", Value::int(residual))],
            Severity::Info,
            Some(Target::Device),
            Vec::new(),
        );
    }
}

/// The name of the next version of a project folder: the trailing `-NN` (or `_NN`) number counted up, or `-02` added;
/// the first name `exists` does not know
pub(crate) fn next_version_name(name: &str, exists: impl Fn(&str) -> bool) -> String {
    let digits = name.len() - name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (stem, mut n) = match name.as_bytes().get(name.len().saturating_sub(digits + 1)) {
        Some(b'-' | b'_') if digits > 0 => {
            (name[..name.len() - digits - 1].to_string(), name[name.len() - digits..].parse::<u32>().unwrap_or(1))
        }
        _ => (name.to_string(), 1),
    };
    loop {
        n += 1;
        let candidate = format!("{stem}-{n:02}");
        if !exists(&candidate) {
            return candidate;
        }
    }
}

#[cfg(test)]
mod version_tests {
    use super::next_version_name;

    #[test]
    fn versions_count_up_and_skip_existing() {
        assert_eq!(next_version_name("song", |_| false), "song-02");
        assert_eq!(next_version_name("song-02", |_| false), "song-03");
        assert_eq!(next_version_name("song", |n| n == "song-02"), "song-03");
        assert_eq!(next_version_name("take_9", |_| false), "take-10");
        assert_eq!(next_version_name("2024", |_| false), "2024-02");
    }
}
