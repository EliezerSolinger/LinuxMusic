//! Killing the process and opening the project again comes back to the last action (I-4): a child process (this
//! same test binary, by an environment variable) makes edits and aborts without saving; the parent opens the folder

mod common;

use std::time::Duration;

use common::*;
use lm_base::Label;
use lm_state::Value;

const CHILD: &str = "LM_SESSION_CRASH_CHILD";

#[test]
fn killed_and_reopened_comes_back_to_the_last_action() {
    if let Some(folder) = std::env::var_os(CHILD) {
        // The child: open, edit, the journal on the disk, then a crash
        let dir = TempDir::new("crash-child");
        let mut s = session(&dir);
        open(&mut s, folder.into());
        let t = add_synth_track(&mut s, "Before the crash");
        set(&mut s, t, "volume", Value::real(0.25).expect("real"));
        s.document_mut()
            .transact(Label::new("test.rename"), |e| {
                e.set(t, &[lm_state::PathStep::key("name")], Value::text("Last action"))
            })
            .expect("rename");
        s.pump();
        assert!(s.sync_journal(Duration::from_secs(5)));
        std::process::abort();
    }
    let dir = TempDir::new("crash");
    let folder = dir.join("song");
    {
        let mut s = session(&dir);
        s.create(folder.clone()).expect("create");
        settle(&mut s);
    }
    let status = std::process::Command::new(std::env::current_exe().expect("exe"))
        .args(["--exact", "killed_and_reopened_comes_back_to_the_last_action", "--nocapture"])
        .env(CHILD, &folder)
        .status()
        .expect("child");
    assert!(!status.success(), "the child crashed");
    // The lock of the dead process is taken over; the journal brings the edits back
    let mut s = session(&dir);
    open(&mut s, folder);
    assert!(!s.project().read_only, "{:?}", s.notices().keys());
    let tracks = s.document().root().get("tracks").expect("tracks").clone();
    assert_eq!(tracks.len(), 1);
    let t = tracks.at(0).expect("track");
    assert_eq!(t.get("name").and_then(Value::as_text), Some("Last action"));
    assert_eq!(t.get("volume").and_then(Value::as_real), Some(0.25));
    assert!(s.modified(), "the recovered edits are not saved yet");
    assert!(s.notices().keys().contains(&"session.journal.recovered"));
}
