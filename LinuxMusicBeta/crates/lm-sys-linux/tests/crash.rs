//! The native-crash note (U21): a child process (this test binary, by an environment variable) arms the handlers,
//! marks itself a render thread inside a node, and faults; the parent finds the note and a process that died of the
//! signal (the handlers chain to the default action)

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;

use lm_sys_linux::crash::{CrashNotes, RenderThreadMark, arm, dereference_null};

const CHILD: &str = "LM_SYS_LINUX_CRASH_CHILD";
const DIR: &str = "LM_SYS_LINUX_CRASH_DIR";

fn child(how: &'static str, dir: PathBuf) {
    let file = arm(&dir, "suspeito.json").expect("arm");
    // Render thread 1 is in slot 7 (the table is lm_render::Watch's: slot + 1)
    file.set_notes(CrashNotes::new(
        std::sync::Arc::new([AtomicU32::new(0), AtomicU32::new(8)]),
        vec![(3, b"three".to_vec()), (7, b"{\"node\":\"seven\"}".to_vec())],
    ));
    let render = how != "main-thread";
    let thread = std::thread::spawn(move || {
        let _mark = render.then(|| RenderThreadMark::enter(1));
        match how {
            "abort" => std::process::abort(),
            "ill" | "bus" => {
                let sig = if how == "ill" { libc::SIGILL } else { libc::SIGBUS };
                // SAFETY: raising a signal at ourselves
                unsafe { libc::raise(sig) };
            }
            _ => {
                std::hint::black_box(dereference_null());
            }
        }
    });
    let _ = thread.join();
    drop(file);
    std::process::exit(0);
}

fn run(how: &'static str) -> (std::process::ExitStatus, Option<String>, bool) {
    let dir = std::env::temp_dir().join(format!("lm-sys-linux-crash-{how}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let status = std::process::Command::new(std::env::current_exe().expect("exe"))
        .args(["--exact", "a_native_crash_leaves_the_node_it_was_in", "--nocapture"])
        .env(CHILD, how)
        .env(DIR, &dir)
        .stderr(std::process::Stdio::null())
        .status()
        .expect("child");
    let note = std::fs::read_to_string(dir.join("suspeito.json")).ok();
    let tmp_left = dir.join(".suspeito.json.crash").exists();
    let _ = std::fs::remove_dir_all(&dir);
    (status, note, tmp_left)
}

#[test]
fn a_native_crash_leaves_the_node_it_was_in() {
    if let (Some(how), Some(dir)) = (std::env::var(CHILD).ok(), std::env::var_os(DIR)) {
        let how: &'static str = lm_base::intern(&how);
        child(how, dir.into());
        return;
    }
    for (how, signals) in [
        ("null", &[libc::SIGSEGV, libc::SIGBUS][..]),
        ("abort", &[libc::SIGABRT][..]),
        ("ill", &[libc::SIGILL][..]),
        ("bus", &[libc::SIGBUS][..]),
    ] {
        let (status, note, tmp_left) = run(how);
        assert!(!status.success(), "{how}: the child died");
        assert_eq!(note.as_deref(), Some("{\"node\":\"seven\"}"), "{how}");
        assert!(!tmp_left, "{how}: the temporary became the note");
        // Chained to the default action: killed by the same signal (a sanitizer's own handler exits instead)
        if let Some(sig) = status.signal() {
            assert!(signals.contains(&sig), "{how}: {status:?}");
        }
    }
    // A crash outside the render threads accuses nobody
    let (status, note, _) = run("main-thread");
    assert!(!status.success());
    assert_eq!(note, None);
}

#[test]
fn a_name_that_leaves_the_folder_is_refused() {
    let dir = std::env::temp_dir();
    for name in ["", ".", "..", "a/b", "../x", "a\0b"] {
        let e = arm(&dir, name).expect_err(name);
        assert_eq!(e.key(), "linux.invalid_argument", "{name:?}");
    }
}
