//! The server's file side (I-20.3, I-20.6): uploads for importing (chunked, resumable, checked by their BLAKE3) and
//! the project folders a client may name. A client never gives a path of the server: it sends a file, which lands in
//! the uploads folder, or names a project inside the projects folder.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use lm_base::{Error, Result};

/// An upload being received
#[derive(Debug)]
pub(crate) struct Upload {
    pub hash: String,
    pub size: u64,
    pub ext: String,
    pub have: u64,
    file: File,
    pub part: PathBuf,
}

fn io(what: &'static str, path: &Path, e: &std::io::Error) -> Error {
    Error::new(what).arg("path", path.display().to_string()).arg("error", e.to_string())
}

/// A file extension a client may give: 1 to 8 ASCII letters or digits
pub(crate) fn valid_ext(ext: &str) -> bool {
    (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// What the uploads folder may take from the server's disk (A6). The per-client limits bound one client; these bound
/// all of them together, which is what fills a disk
#[derive(Clone, Copy, Debug)]
pub struct UploadQuota {
    /// Bytes still to arrive over every open upload of every client at once
    pub max_outstanding: u64,
    /// Everything in the uploads folder (partial files and finished ones not yet removed)
    pub max_stored: u64,
    /// Free space that must remain on the disk after an upload's remaining bytes
    pub min_free: u64,
    /// A partial file untouched this long is not coming back: it is removed
    pub stale_after: std::time::Duration,
}

impl Default for UploadQuota {
    fn default() -> Self {
        Self {
            max_outstanding: 32 << 30,
            max_stored: 64 << 30,
            min_free: 1 << 30,
            stale_after: std::time::Duration::from_secs(7 * 24 * 3600),
        }
    }
}

/// The quota's verdict for a new upload that still needs `remaining` bytes. Pure, so the policy can be tested
pub(crate) fn quota_verdict(q: &UploadQuota, outstanding: u64, remaining: u64, stored: u64, free: u64) -> Result<()> {
    let refuse = |what: &'static str, limit: u64| {
        Err(Error::new("remote.upload.quota").arg("what", what).arg("limit", i64::try_from(limit).unwrap_or(i64::MAX)))
    };
    if outstanding.saturating_add(remaining) > q.max_outstanding {
        return refuse("outstanding", q.max_outstanding);
    }
    if stored.saturating_add(remaining) > q.max_stored {
        return refuse("stored", q.max_stored);
    }
    if free < remaining.saturating_add(q.min_free) {
        return refuse("disk", q.min_free);
    }
    Ok(())
}

/// Removes partial uploads nobody has touched for `quota.stale_after`, except those in `open` (hashes), and returns
/// the bytes the folder holds afterwards
pub(crate) fn prune_and_measure(dir: &Path, quota: &UploadQuota, open: &[String]) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let now = std::time::SystemTime::now();
    let mut total = 0u64;
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let path = e.path();
        let stale = path.extension().is_some_and(|x| x == "part")
            && meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > quota.stale_after)
            && !path.file_stem().and_then(|s| s.to_str()).is_some_and(|h| open.iter().any(|o| o == h));
        if stale && std::fs::remove_file(&path).is_ok() {
            continue;
        }
        total = total.saturating_add(meta.len());
    }
    total
}

/// The bytes the partial file of this hash already holds (an upload resumes from them), capped at `size`
pub(crate) fn part_len(dir: &Path, hash: &str, size: u64) -> u64 {
    match std::fs::metadata(dir.join(format!("{hash}.part"))) {
        Ok(m) if m.len() <= size => m.len(),
        _ => 0,
    }
}

/// Free bytes of the disk holding `dir` (its nearest existing parent); `u64::MAX` when it cannot be read, so an
/// unreadable disk never blocks an upload by itself
pub(crate) fn free_space(dir: &Path) -> u64 {
    let mut at = dir;
    while !at.exists() {
        match at.parent() {
            Some(p) => at = p,
            None => break,
        }
    }
    rustix::fs::statvfs(at).map_or(u64::MAX, |s| s.f_bavail.saturating_mul(s.f_frsize))
}

impl Upload {
    /// Starts or resumes the upload of a file with this hash: what is already there (`<hash>.part`) counts
    pub fn begin(dir: &Path, hash: &str, size: u64, ext: &str) -> Result<Upload> {
        if !lm_state::is_valid_resource_key(hash) {
            return Err(Error::new("remote.upload.hash"));
        }
        if !valid_ext(ext) {
            return Err(Error::new("remote.upload.ext"));
        }
        std::fs::create_dir_all(dir).map_err(|e| io("remote.upload.io", dir, &e))?;
        let part = dir.join(format!("{hash}.part"));
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&part)
            .map_err(|e| io("remote.upload.io", &part, &e))?;
        let mut have = file.metadata().map_err(|e| io("remote.upload.io", &part, &e))?.len();
        if have > size {
            file.set_len(0).map_err(|e| io("remote.upload.io", &part, &e))?;
            have = 0;
        }
        file.seek(SeekFrom::Start(have)).map_err(|e| io("remote.upload.io", &part, &e))?;
        Ok(Upload { hash: hash.to_owned(), size, ext: ext.to_owned(), have, file, part })
    }

    /// The next bytes, at `offset` (which must be where the file ends: chunks come in order)
    pub fn write(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        if offset != self.have {
            return Err(Error::new("remote.upload.offset").arg("expected", self.have as i64).arg("got", offset as i64));
        }
        if self.have + data.len() as u64 > self.size {
            return Err(Error::new("remote.upload.too_long"));
        }
        self.file.write_all(data).map_err(|e| io("remote.upload.io", &self.part, &e))?;
        self.have += data.len() as u64;
        Ok(())
    }

    /// Checks the whole file against its hash on a thread (it may be large: I-2.1), then names it
    /// `<hash>.<ext>`: the path to import comes back on the receiver
    pub fn finish(self) -> Result<Receiver<Result<PathBuf>>> {
        if self.have != self.size {
            return Err(Error::new("remote.upload.incomplete")
                .arg("have", self.have as i64)
                .arg("size", self.size as i64));
        }
        let Upload { hash, ext, file, part, .. } = self;
        drop(file);
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("lm-remote-verify".into())
            .spawn(move || {
                let check = || -> Result<PathBuf> {
                    let mut f = File::open(&part).map_err(|e| io("remote.upload.io", &part, &e))?;
                    let mut h = blake3::Hasher::new();
                    let mut buf = vec![0u8; 1 << 20];
                    loop {
                        let n = f.read(&mut buf).map_err(|e| io("remote.upload.io", &part, &e))?;
                        if n == 0 {
                            break;
                        }
                        h.update(&buf[..n]);
                    }
                    let got = h.finalize().to_hex().to_string();
                    if got != hash {
                        let _ = std::fs::remove_file(&part);
                        return Err(Error::new("remote.upload.hash_mismatch"));
                    }
                    let done = part.with_file_name(format!("{hash}.{ext}"));
                    std::fs::rename(&part, &done).map_err(|e| io("remote.upload.io", &done, &e))?;
                    Ok(done)
                };
                let _ = tx.send(check());
            })
            .map_err(|e| Error::new("remote.io").arg("error", e.to_string()))?;
        Ok(rx)
    }
}

/// The folder of the project named `name` inside the projects folder (`~` is the home folder). `name` is one path
/// component: no separator, no `.` or `..` (I-20.6)
pub(crate) fn project_folder(projects: &str, name: &str) -> Result<PathBuf> {
    let valid = !name.is_empty()
        && name.len() <= 255
        && !name.contains(['/', '\\', '\0'])
        && matches!(Path::new(name).components().next(), Some(Component::Normal(_)))
        && Path::new(name).components().count() == 1;
    if !valid {
        return Err(Error::new("remote.project.name").arg("name", name));
    }
    Ok(projects_dir(projects)?.join(name))
}

pub(crate) fn projects_dir(projects: &str) -> Result<PathBuf> {
    if projects.is_empty() {
        return Err(Error::new("remote.project.no_folder"));
    }
    Ok(match projects.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => PathBuf::from(projects),
    })
}

/// Whether `folder` (absolute, maybe not existing yet) lies right inside the projects folder
pub(crate) fn inside_projects(projects: &str, folder: &str) -> bool {
    let Ok(dir) = projects_dir(projects) else { return false };
    let f = Path::new(folder);
    let (Some(parent), Some(name)) = (f.parent(), f.file_name().and_then(|n| n.to_str())) else { return false };
    if project_folder(projects, name).is_err() {
        return false;
    }
    match (parent.canonicalize(), dir.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => parent == dir,
    }
}

/// Whether `path` (absolute) lies somewhere under the projects folder, with no `..` in it (an export's file)
pub(crate) fn inside_tree(projects: &str, path: &str) -> bool {
    let Ok(dir) = projects_dir(projects) else { return false };
    let p = Path::new(path);
    if !(p.is_absolute()
        && p.components().all(|c| !matches!(c, Component::ParentDir | Component::CurDir))
        && p.starts_with(&dir)
        && p != dir)
    {
        return false;
    }
    // A symbolic link inside the folder must not lead out of it: resolve the deepest part that exists
    let Ok(root) = dir.canonicalize() else { return true };
    let mut probe = p;
    loop {
        if let Ok(real) = probe.canonicalize() {
            return real.starts_with(&root);
        }
        match probe.parent() {
            Some(up) => probe = up,
            None => return false,
        }
    }
}

/// The projects in the projects folder (folders holding a `projeto.json`), by name, sorted; on a thread
pub(crate) fn list_projects(projects: &str) -> Receiver<Result<Vec<String>>> {
    let (tx, rx) = channel();
    let dir = projects_dir(projects);
    let spawned = std::thread::Builder::new().name("lm-remote-list".into()).spawn({
        let tx = tx.clone();
        move || {
            let r = dir.and_then(|dir| {
                let mut out = Vec::new();
                for e in std::fs::read_dir(&dir).map_err(|e| io("remote.project.list", &dir, &e))?.flatten() {
                    if e.path().join("projeto.json").is_file()
                        && let Some(n) = e.file_name().to_str()
                    {
                        out.push(n.to_owned());
                    }
                }
                out.sort();
                Ok(out)
            });
            let _ = tx.send(r);
        }
    });
    if let Err(e) = spawned {
        let _ = tx.send(Err(Error::new("remote.io").arg("error", e.to_string())));
    }
    rx
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_quota_bounds_outstanding_stored_and_free_space() {
        use super::{UploadQuota, quota_verdict};
        let q = UploadQuota { max_outstanding: 100, max_stored: 200, min_free: 10, ..UploadQuota::default() };
        assert!(quota_verdict(&q, 50, 50, 100, 1000).is_ok());
        let what = |r: lm_base::Result<()>| r.unwrap_err().label.args.iter().find(|(k, _)| *k == "what").map(|(_, a)| format!("{a:?}"));
        assert!(what(quota_verdict(&q, 60, 50, 0, 1000)).unwrap().contains("outstanding"));
        assert!(what(quota_verdict(&q, 0, 50, 160, 1000)).unwrap().contains("stored"));
        assert!(what(quota_verdict(&q, 0, 50, 0, 55)).unwrap().contains("disk"));
    }

    #[test]
    fn stale_partials_are_removed_and_the_rest_counted() {
        use super::{UploadQuota, prune_and_measure};
        let dir = std::env::temp_dir().join(format!("lmr-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("aa.part"), [0u8; 10]).unwrap();
        std::fs::write(dir.join("bb.part"), [0u8; 20]).unwrap();
        std::fs::write(dir.join("cc.wav"), [0u8; 30]).unwrap();
        let none = UploadQuota { stale_after: std::time::Duration::from_secs(3600), ..UploadQuota::default() };
        assert_eq!(prune_and_measure(&dir, &none, &[]), 60);
        // Everything is older than a zero age; the open one and the finished file stay
        let all = UploadQuota { stale_after: std::time::Duration::ZERO, ..UploadQuota::default() };
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(prune_and_measure(&dir, &all, &["bb".to_owned()]), 50);
        assert!(!dir.join("aa.part").exists() && dir.join("bb.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_tree_is_outside() {
        let base = std::env::temp_dir().join(format!("lm-tree-{}", std::process::id()));
        let (proj, out) = (base.join("projects"), base.join("elsewhere"));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::os::unix::fs::symlink(&out, proj.join("link")).unwrap();
        let p = proj.to_str().unwrap();
        assert!(inside_tree(p, proj.join("song/a.mid").to_str().unwrap()));
        assert!(!inside_tree(p, proj.join("link/a.mid").to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn browser_actions_are_admin_only() {
        assert!(crate::proto::admin_action("browser.reveal"));
        assert!(!crate::proto::admin_action("midi.export"));
    }

    #[test]
    fn names_stay_inside() {
        assert!(project_folder("/p", "song").is_ok());
        for bad in ["", "..", ".", "a/b", "/etc", "a\\b", "x\0"] {
            assert!(project_folder("/p", bad).is_err(), "{bad:?}");
        }
        assert!(inside_projects("/p", "/p/song"));
        assert!(!inside_projects("/p", "/q/song"));
        assert!(!inside_projects("/p", "/p/../etc"));
        // A file whose bytes do not match its declared hash is refused and removed
        let dir = std::env::temp_dir().join(format!("lmr-files-{}", std::process::id()));
        let wrong = blake3::hash(b"other").to_hex().to_string();
        let mut u = Upload::begin(&dir, &wrong, 4, "wav").unwrap();
        u.write(0, b"data").unwrap();
        assert_eq!(u.write(0, b"x").unwrap_err().key(), "remote.upload.offset");
        let part = u.part.clone();
        let r = u.finish().unwrap().recv().unwrap();
        assert_eq!(r.unwrap_err().key(), "remote.upload.hash_mismatch");
        assert!(!part.exists());
        let right = blake3::hash(b"data").to_hex().to_string();
        let mut u = Upload::begin(&dir, &right, 4, "wav").unwrap();
        u.write(0, b"da").unwrap();
        // Resumed: the part already has two bytes
        let mut u2 = Upload::begin(&dir, &right, 4, "wav").unwrap();
        assert_eq!(u2.have, 2);
        drop(u);
        u2.write(2, b"ta").unwrap();
        let done = u2.finish().unwrap().recv().unwrap().unwrap();
        assert_eq!(std::fs::read(&done).unwrap(), b"data");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            inside_tree("/p", "/p/song/mix.wav") && !inside_tree("/p", "/p/../x.wav") && !inside_tree("/p", "/q/x")
        );
        assert!(valid_ext("wav") && !valid_ext("") && !valid_ext("../x"));
    }
}
