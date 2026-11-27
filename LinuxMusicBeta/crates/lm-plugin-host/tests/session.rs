//! Third-party plugins in a session (I-9.6, I-9.4): the scan as an operation (its plugins in the catalog, the
//! blocked ones and the scan in the preferences), a plugin played from a track, its own state into the document
//! (when it says it changed, and before a save), a state the document changes given to the running plugin, and a
//! helper that dies: a notice with "recreate", and a new helper from the document's state

use std::path::PathBuf;
use std::time::{Duration, Instant};

use lm_base::Label;
use lm_plugin_proxy::{Deadline, HostConfig, plugin_id};
use lm_session::{Session, SessionConfig};
use lm_state::{NodeId, PathStep, ROOT, Value};

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp(name: &str) -> TempDir {
    let p = std::env::temp_dir().join(format!("lm-plugin-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    TempDir(p)
}

fn host() -> HostConfig {
    HostConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_lm-plugin-host")),
        deadline: Deadline::Live(0.6),
        load_timeout: Duration::from_secs(20),
        paths: Vec::new(),
        system_paths: true,
    }
}

fn settle(s: &mut Session) {
    assert!(s.settle(Duration::from_secs(30)), "not settled: {:?} {:?}", s.busy(), s.notices_keys());
}

/// Pumps until `f` holds, at most 20 s
fn until(s: &mut Session, what: &str, mut f: impl FnMut(&mut Session) -> bool) {
    let start = Instant::now();
    while !f(s) {
        assert!(start.elapsed() < Duration::from_secs(20), "never: {what} ({:?})", s.notices_keys());
        s.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn chunk_of(s: &Session, id: NodeId) -> Option<String> {
    let n = s.document().find(id)?;
    n.get("state")?.get("chunk")?.as_text().map(str::to_owned)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn third_party_plugins_in_a_session() {
    let dir = temp("all");
    let mut s = Session::new(SessionConfig::headless(dir.0.join("untitled")).unwrap()).unwrap();
    s.set_plugin_host(host());
    s.create(dir.0.join("song")).unwrap();
    settle(&mut s);

    // The scan: an operation; the plugins join the catalog, the blocked ones and the scan go to the preferences
    s.scan_plugins("test").unwrap();
    let synth = plugin_id("test", "synth");
    until(&mut s, "the scan", |s| s.notices_keys().contains(&"plugins.scan.done"));
    assert!(s.registry().kind(&synth).is_some(), "the synth is in the catalog: {:?}", s.notices().all());
    assert!(s.notices_keys().contains(&"plugins.scan.done"));
    let prefs = s.preferences().get(&["plugins"]).unwrap().clone();
    let blocked: Vec<&str> =
        prefs.get("blocked").unwrap().items().filter_map(|b| b.get("uri")?.as_text()).collect::<Vec<_>>();
    assert_eq!(blocked, ["bomb", "stall"]);
    assert_eq!(prefs.get("scanned").unwrap().len(), 6);
    // A crash is blocked at once; a timeout is a strike, and the plugin is tried again by the next scan
    let strikes = |s: &Session, uri: &str| {
        s.preferences()
            .get(&["plugins", "blocked"])
            .unwrap()
            .items()
            .find(|b| b.get("uri").and_then(Value::as_text) == Some(uri))
            .and_then(|b| b.get("strikes")?.as_int())
    };
    assert_eq!(strikes(&s, "bomb"), Some(1));
    assert_eq!(strikes(&s, "stall"), Some(1));
    // A second scan skips the crash, tries the stall again (its second timeout blocks it) and adds nothing twice
    let n = s.notices().all().len();
    s.scan_plugins("test").unwrap();
    until(&mut s, "the second scan", |s| {
        s.notices().all().iter().find(|x| x.key == "plugins.scan.done").is_some_and(|x| x.repeats > 0)
            || s.notices().all().len() > n
    });
    assert_eq!(s.preferences().get(&["plugins", "blocked"]).unwrap().len(), 2);
    assert_eq!(strikes(&s, "bomb"), Some(1), "skipped, not counted again");
    assert_eq!(strikes(&s, "stall"), Some(2));
    // The user lifts a block: the entry goes, and the next scan tries the plugin again
    s.run_action("plugin.unblock", &Value::object([("format", Value::text("test")), ("uri", Value::text("stall"))]))
        .unwrap();
    assert_eq!(strikes(&s, "stall"), None);
    assert_eq!(s.preferences().get(&["plugins", "blocked"]).unwrap().len(), 1);

    // A track playing the scanned synth: the session keeps its helper
    let mut pid = None;
    s.document_mut()
        .transact(Label::new("test"), |e| {
            let inst = Value::object([
                ("plugin", Value::text(synth.as_str())),
                ("version", Value::int(1)),
                ("state", Value::empty_object()),
            ]);
            let t = e.insert(ROOT, "tracks", 0, Value::object([("name", Value::text("S")), ("instrument", inst)]))?;
            let id = e.find(t).and_then(|n| n.get("instrument")?.get("id")?.as_text().map(str::to_owned));
            pid = Some(id.unwrap().parse::<NodeId>()?);
            let item =
                Value::object([("kind", Value::text("midi")), ("start", Value::int(0)), ("length", Value::int(960))]);
            let it = e.insert(t, "items", 0, item)?;
            e.add_note(
                it,
                Value::object([
                    ("key", Value::int(60)),
                    ("start", Value::int(0)),
                    ("length", Value::int(480)),
                    ("velocity", Value::real(0.8)?),
                ]),
            )?;
            Ok(())
        })
        .unwrap();
    let pid = pid.unwrap();
    settle(&mut s);
    let first = s.plugin_host_of(pid).expect("a helper runs the synth");
    assert_eq!(chunk_of(&s, pid), None);

    // The plugin says its state changed (the test synth does on a note): the session asks for it, into the document
    s.play();
    until(&mut s, "the plugin's state in the document", |s| chunk_of(s, pid).is_some());
    s.stop();
    let undo_steps = s.document().history_len();

    // The document's chunk changed (an undo, a preset): given to the running plugin, which is not built again
    let level = hex(&0.25f32.to_le_bytes());
    s.document_mut()
        .transact(Label::new("test.preset"), |e| {
            e.set(pid, &[PathStep::key("state"), PathStep::key("chunk")], Value::text(level.as_str()))
        })
        .unwrap();
    settle(&mut s);
    let now = s.plugin_host_of(pid).unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &now), "the same helper, not built again");
    assert_eq!(now.save_state(Duration::from_secs(5)).unwrap(), Some(0.25f32.to_le_bytes().to_vec()));
    assert!(s.document().history_len() > undo_steps);

    // A save asks every plugin for its state first (here one it took without saying so)
    let other = hex(&0.75f32.to_le_bytes());
    now.load_state(&0.75f32.to_le_bytes(), Duration::from_secs(5)).unwrap();
    s.save().unwrap();
    settle(&mut s);
    assert_eq!(chunk_of(&s, pid).as_deref(), Some(other.as_str()));
    assert_eq!(s.document().param(pid, "level"), Some(0.75), "the parameters the state carries");
    let level = other;
    let saved = std::fs::read_to_string(dir.0.join("song").join("project.json"))
        .or_else(|_| {
            // Whatever the project file is called, it has the chunk
            let f = std::fs::read_dir(dir.0.join("song"))?
                .filter_map(|e| e.ok())
                .find(|e| e.path().extension().is_some_and(|x| x == "json" || x == "lmp"));
            std::fs::read_to_string(f.map(|e| e.path()).unwrap_or_default())
        })
        .unwrap_or_default();
    assert!(saved.contains(&level), "the saved project has the plugin's state");

    // The helper dies: the node goes silent, a notice offers "recreate"; recreating starts a new helper with the
    // document's state
    first.kill();
    drop(first);
    drop(now);
    until(&mut s, "the crash notice", |s| s.notices_keys().contains(&"plugin_host.crashed"));
    let notice = s.notices().all().into_iter().find(|n| n.key == "plugin_host.crashed").unwrap();
    let (action, params) = notice.actions.first().cloned().expect("an action");
    assert_eq!(&*action.0, "plugin.recreate");
    s.run_action(&action.0, &params).unwrap();
    until(&mut s, "a new helper", |s| s.plugin_host_of(pid).is_some_and(|h| !h.is_dead()));
    let again = s.plugin_host_of(pid).unwrap();
    assert_eq!(again.save_state(Duration::from_secs(5)).unwrap(), Some(0.75f32.to_le_bytes().to_vec()));
}
