//! Safe mode (U21): a plugin that hangs the callback is found by the watchdog; reopening the project starts with it
//! bypassed, a notice, and `status().safeMode`

mod common;

use std::any::Any;
use std::time::Duration;

use common::*;
use lm_base::{Label, Result};
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, Registry, TestHints, Velocity};
use lm_render::{ProcessContext, Processor};
use lm_session::Session;
use lm_state::{NodeSchema, ROOT, Value};

/// An instrument that blocks its thread for 4 s on its 50th call: an endless loop, as far as the watchdog knows
struct Hang(u32);

impl Processor for Hang {
    fn process(&mut self, _: &mut ProcessContext<'_>) {
        self.0 += 1;
        if self.0 == 50 {
            std::thread::sleep(Duration::from_secs(4));
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn build(_: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    Ok(Box::new(Hang(0)))
}

static HANG: Descriptor = Descriptor {
    id: "hang",
    name: "plugin.hang.name",
    kind: PluginKind::Instrument,
    schemas: || vec![NodeSchema::new("plugin:hang", 1)],
    ports: |_| Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() },
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn config_with_hang(dir: &TempDir) -> lm_session::SessionConfig {
    let mut c = config(dir);
    c.registry = Registry::new(&[&lm_plugins_tripleoscillator::DESCRIPTOR, &HANG]).expect("registry");
    c
}

#[test]
fn a_hanging_plugin_opens_the_project_in_safe_mode() {
    let dir = TempDir::new("safe");
    let folder = dir.join("song");
    let config = config_with_hang(&dir);
    let f2 = folder.clone();
    // The first session: the plugin hangs; the process is "killed" (the session is leaked, its lock removed)
    std::thread::spawn(move || {
        let mut s = Session::new(config).expect("session");
        s.create(f2.clone()).expect("create");
        s.document_mut()
            .transact(Label::new("test"), |e| {
                let inst = Value::object([
                    ("plugin", Value::text("hang")),
                    ("version", Value::int(1)),
                    ("state", Value::empty_object()),
                ]);
                e.insert(ROOT, "tracks", 0, Value::object([("instrument", inst)]))?;
                Ok(())
            })
            .expect("track");
        s.save().expect("save");
        let start = std::time::Instant::now();
        while !lm_session::watchdog::suspect_path(&f2).exists() {
            assert!(start.elapsed() < Duration::from_secs(20), "the watchdog never saw it");
            s.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(s.sync_journal(Duration::from_secs(5)));
        std::mem::forget(s);
    })
    .join()
    .expect("first");
    let _ = std::fs::remove_file(lm_session::folder::ProjectFolder::new(folder.clone()).lock());
    let config = config_with_hang(&dir);
    std::thread::spawn(move || {
        let mut s = Session::new(config).expect("session");
        open(&mut s, folder.clone());
        assert_eq!(s.status().get("safeMode").and_then(Value::as_bool), Some(true));
        let plugins = s.status().get("safeModePlugins").expect("list").clone();
        assert_eq!(plugins.at(0).and_then(|p| p.get("plugin")).and_then(Value::as_text), Some("hang"));
        let track = s.document().root().get("tracks").and_then(|t| t.at(0).cloned()).expect("track");
        assert_eq!(
            track
                .at_path(&[lm_state::PathStep::key("instrument"), lm_state::PathStep::key("bypass")])
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(s.notices_keys().contains(&"session.safe_mode"));
        assert!(!lm_session::watchdog::suspect_path(&folder).exists(), "used once");
    })
    .join()
    .expect("second");
}

/// An instrument with a null-pointer bug that fires after half a second of audio (the notes of the program are set
/// by then: they follow its install on the main thread)
struct Crash(u64);

impl Processor for Crash {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        self.0 += cx.buffers.frames() as u64;
        if self.0 > 24_000 {
            std::hint::black_box(lm_sys_linux::crash::dereference_null());
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

static CRASH: Descriptor = Descriptor {
    id: "crash",
    name: "plugin.crash.name",
    kind: PluginKind::Instrument,
    schemas: || vec![NodeSchema::new("plugin:crash", 1)],
    ports: |_| Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() },
    velocity: Velocity::Ignored,
    display: None,
    build: |_, _| Ok(Box::new(Crash(0))),
    tests: TestHints::new(),
};

const CRASH_CHILD: &str = "LM_SESSION_NATIVE_CRASH_CHILD";

#[cfg(unix)]
#[test]
fn a_native_crash_in_a_plugin_opens_the_project_in_safe_mode() {
    let registry = || Registry::new(&[&lm_plugins_tripleoscillator::DESCRIPTOR, &CRASH]).expect("registry");
    if let Some(folder) = std::env::var_os(CRASH_CHILD) {
        // The child: the plugin dereferences null inside the callback; the process dies of it
        let dir = TempDir::new("native-crash-child");
        let mut c = config(&dir);
        c.registry = registry();
        let mut s = Session::new(c).expect("session");
        open(&mut s, folder.into());
        s.document_mut()
            .transact(Label::new("test"), |e| {
                let inst = Value::object([("plugin", Value::text("crash")), ("version", Value::int(1))]);
                e.insert(ROOT, "tracks", 0, Value::object([("instrument", inst)]))?;
                Ok(())
            })
            .expect("track");
        s.save().expect("save");
        loop {
            s.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let dir = TempDir::new("native-crash");
    let folder = dir.join("song");
    {
        let mut s = session(&dir);
        s.create(folder.clone()).expect("create");
        settle(&mut s);
    }
    let status = std::process::Command::new(std::env::current_exe().expect("exe"))
        .args(["--exact", "a_native_crash_in_a_plugin_opens_the_project_in_safe_mode", "--nocapture"])
        .env(CRASH_CHILD, &folder)
        .stderr(std::process::Stdio::null())
        .status()
        .expect("child");
    assert!(!status.success(), "the child crashed: {status:?}");
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            assert!(sig == libc_sigsegv() || sig == libc_sigbus(), "{status:?}");
        }
    }
    let suspect = lm_session::watchdog::read_suspect(&folder).expect("the handler wrote the suspect");
    assert_eq!(suspect.reason, "crash");
    let mut c = config(&dir);
    c.registry = registry();
    let mut s = Session::new(c).expect("session");
    open(&mut s, folder.clone());
    assert_eq!(s.status().get("safeMode").and_then(Value::as_bool), Some(true));
    let plugins = s.status().get("safeModePlugins").expect("list").clone();
    assert_eq!(plugins.at(0).and_then(|p| p.get("plugin")).and_then(Value::as_text), Some("crash"));
    let node = s.document().find(suspect.node).expect("the suspect is a node of the document");
    assert_eq!(node.get("plugin").and_then(Value::as_text), Some("crash"));
    assert_eq!(node.get("bypass").and_then(Value::as_bool), Some(true));
    assert!(s.notices_keys().contains(&"session.safe_mode"));
    // Bypassed: it plays on without crashing this process
    settle(&mut s);
    std::thread::sleep(Duration::from_millis(800));
    s.pump();
}

// SIGSEGV and SIGBUS, without libc in this crate's dependencies
fn libc_sigsegv() -> i32 {
    11
}
fn libc_sigbus() -> i32 {
    if cfg!(target_os = "linux") { 7 } else { 10 }
}
