//! The project folder: create, save, open, the lock (H10), a newer schema (H11), the journal and crash recovery
//! (I-4), a full disk (H3)

mod common;

use std::time::Duration;

use common::*;
use lm_port::Target;
use lm_session::folder::ProjectFolder;
use lm_session::lock::{LockOwner, write_foreign_lock};
use lm_session::{Session, project_file};
use lm_state::{Value, from_json, to_json};

#[test]
fn a_new_project_plays_its_instrument_once_built() {
    let dir = TempDir::new("play");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let t = add_synth_track(&mut s, "Synth");
    settle(&mut s);
    let inst = s.document().find(t).and_then(|n| n.get("instrument").cloned()).expect("instrument");
    let pid = lm_state::NodeId::parse(inst.get("id").and_then(Value::as_text).expect("id")).expect("p:");
    assert!(s.table().node(lm_compile::NodeKey::Instrument(pid)).is_some(), "the instrument is in the program");
    assert_eq!(s.loading(&Target::Node(pid)), lm_port::Loading::Ready);
    assert!(project_file(&dir.join("song")).exists());
}

#[test]
fn save_open_save_gives_the_same_file() {
    let dir = TempDir::new("roundtrip");
    let folder = dir.join("song");
    let first;
    {
        let mut s = session(&dir);
        s.create(folder.clone()).expect("create");
        add_synth_track(&mut s, "A");
        add_synth_track(&mut s, "B");
        s.save().expect("save");
        settle(&mut s);
        assert!(!s.modified());
        first = std::fs::read(project_file(&folder)).expect("read");
    }
    let mut s = session(&dir);
    open(&mut s, folder.clone());
    assert!(!s.project().read_only, "{:?}", s.notices().keys());
    assert!(!s.modified());
    s.save().expect("save again");
    settle(&mut s);
    let second = std::fs::read(project_file(&folder)).expect("read");
    assert_eq!(String::from_utf8_lossy(&first), String::from_utf8_lossy(&second));
}

/// F-PRJ-010: each save keeps the file it replaces in `backup/`, numbered up, and the oldest go past `backup.kept`
#[test]
fn a_save_keeps_the_previous_file_in_backup() {
    let dir = TempDir::new("keep");
    let folder = dir.join("song");
    let mut s = session(&dir);
    s.set_preference(&[lm_state::PathStep::key("backup"), lm_state::PathStep::key("kept")], Value::int(2)).expect("pref");
    s.create(folder.clone()).expect("create");
    let mut versions = Vec::new();
    for name in ["A", "B", "C", "D"] {
        add_synth_track(&mut s, name);
        s.save().expect("save");
        settle(&mut s);
        versions.push(std::fs::read_to_string(project_file(&folder)).expect("read"));
    }
    let mut kept: Vec<(String, String)> = std::fs::read_dir(folder.join("backup"))
        .expect("backup")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("projeto-") && n.ends_with(".json"))
        .map(|n| (std::fs::read_to_string(folder.join("backup").join(&n)).expect("read"), n))
        .collect();
    kept.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(kept.len(), 2, "the oldest are pruned: {kept:?}");
    // the last save replaced the file the one before wrote, and that one is the newest copy
    assert_eq!(kept[1].0, versions[2]);
    assert_eq!(kept[0].0, versions[1]);
}

#[test]
fn a_project_open_elsewhere_opens_read_only() {
    let dir = TempDir::new("lock");
    let folder = dir.join("song");
    {
        let mut s = session(&dir);
        s.create(folder.clone()).expect("create");
        settle(&mut s);
    }
    // A living process on another machine holds it
    let other = LockOwner { host: "another-machine".into(), pid: 1, started: "x".into(), since: 1 };
    write_foreign_lock(&ProjectFolder::new(folder.clone()), &other).expect("lock");
    let mut s = session(&dir);
    open(&mut s, folder.clone());
    assert!(s.project().read_only);
    assert!(s.save().is_err(), "a read-only project is never saved");
    assert_eq!(s.status().get("project").and_then(|p| p.get("readOnly")).and_then(Value::as_bool), Some(true));
    drop(s);
    // A process of this machine that is gone: the lock is taken over, with a notice
    let dead = LockOwner { host: lm_session::lock::host_name(), pid: 999_999, started: "never".into(), since: 1 };
    write_foreign_lock(&ProjectFolder::new(folder.clone()), &dead).expect("lock");
    let mut s = session(&dir);
    open(&mut s, folder);
    assert!(!s.project().read_only);
    assert!(s.notices().keys().contains(&"session.project.stale_lock"));
}

#[test]
fn a_newer_schema_opens_read_only_and_is_never_written() {
    let dir = TempDir::new("newer");
    let folder = dir.join("song");
    {
        let mut s = session(&dir);
        s.create(folder.clone()).expect("create");
        settle(&mut s);
    }
    let path = project_file(&folder);
    let saved = from_json(&std::fs::read_to_string(&path).expect("read")).expect("json");
    let versions = saved.get("versions").expect("versions").with("project", Value::int(99));
    let newer = to_json(&saved.with("versions", versions)).expect("json");
    std::fs::write(&path, &newer).expect("write");
    let mut s = session(&dir);
    open(&mut s, folder);
    assert!(s.project().read_only);
    assert!(s.notices().keys().contains(&"session.project.newer"));
    assert!(s.save().is_err());
    drop(s);
    assert_eq!(std::fs::read_to_string(&path).expect("read"), newer, "never saved over");
}

#[test]
fn a_full_disk_on_save_keeps_the_old_file_and_the_journal_keeps_the_change() {
    let dir = TempDir::new("fullsave");
    let folder = dir.join("song");
    let mut s = session(&dir);
    s.create(folder.clone()).expect("create");
    settle(&mut s);
    let before = std::fs::read(project_file(&folder)).expect("read");
    add_synth_track(&mut s, "After");
    s.pump();
    assert!(s.sync_journal(Duration::from_secs(5)));
    s.faults().set_full(true);
    s.save().expect("asked");
    settle(&mut s);
    assert!(s.modified(), "not saved");
    assert_eq!(std::fs::read(project_file(&folder)).expect("read"), before, "the old file is intact");
    assert!(s.notices().keys().contains(&"session.disk.full"));
    // The journal still holds the change: reopening (after a crash) gets it back
    let journal = lm_session::journal::read_journal(&ProjectFolder::new(folder.clone()).journal()).expect("journal");
    assert!(!journal.patches.is_empty());
    s.faults().set_full(false);
    s.save().expect("save");
    settle(&mut s);
    assert!(!s.modified());
}

const FULL_JOURNAL_CHILD: &str = "LM_SESSION_FULL_JOURNAL_CHILD";

#[cfg(unix)]
#[test]
fn a_full_disk_on_the_journal_makes_an_autosave_that_recovery_uses() {
    if let Some(dir) = std::env::var_os(FULL_JOURNAL_CHILD) {
        // The child: the journal line is lost to a full disk, the autosave keeps the edit, then the process crashes
        // (no save, the lock stays); it leaves the tracks it had for the parent to compare
        let dir = std::path::PathBuf::from(dir);
        let mut s =
            Session::new(lm_session::SessionConfig::headless(dir.join("untitled")).expect("config")).expect("session");
        s.create(dir.join("song")).expect("create");
        settle(&mut s);
        // The next journal line fails; then the disk has room again for the autosave
        s.faults().full_after(0);
        add_synth_track(&mut s, "Lost line");
        for _ in 0..50 {
            s.pump();
            std::thread::sleep(Duration::from_millis(2));
            if s.notices().keys().contains(&"session.disk.full") {
                break;
            }
        }
        s.faults().set_full(false);
        for _ in 0..200 {
            s.pump();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(s.sync_journal(Duration::from_secs(5)));
        let tracks = s.document().root().get("tracks").cloned().expect("tracks");
        std::fs::write(dir.join("tracks.json"), to_json(&tracks).expect("json")).expect("write the tracks");
        std::process::abort();
    }
    let dir = TempDir::new("fulljournal");
    let folder = dir.join("song");
    run_crashing_child(
        "a_full_disk_on_the_journal_makes_an_autosave_that_recovery_uses",
        FULL_JOURNAL_CHILD,
        dir.0.as_os_str(),
    );
    let tracks =
        from_json(&std::fs::read_to_string(dir.join("tracks.json")).expect("the child's tracks")).expect("json");
    std::fs::remove_file(ProjectFolder::new(folder.clone()).lock()).expect("unlock as after a reboot");
    let mut r = session(&dir);
    open(&mut r, folder);
    let keys = r.notices().keys();
    assert_eq!(r.document().root().get("tracks"), Some(&tracks), "{keys:?}");
}
