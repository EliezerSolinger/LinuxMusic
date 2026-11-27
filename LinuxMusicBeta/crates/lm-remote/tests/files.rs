//! I-20.3: files over the protocol — an upload for importing (chunked, resumable, checked by its hash, imported by
//! the core) and the projects by name (open, new, save, save as, close, list), each an operation of the core.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use common::TestCore;
use lm_port::{ActionId, CorePort, Finished, PathStep, Pending, RequestId, Resource, Value};
use lm_remote::{ClientConfig, Connection, Endpoint, RemotePort, Server, ServerConfig};

fn temp(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!("lmr-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

struct Setup {
    core: TestCore,
    server: Server,
    client: RemotePort,
    uploads: PathBuf,
    done: Vec<(RequestId, Finished)>,
}

impl Setup {
    fn new() -> Setup {
        Setup::with_quota(lm_remote::UploadQuota::default())
    }

    fn with_quota(quota: lm_remote::UploadQuota) -> Setup {
        let socket = temp("sock");
        let uploads = temp("uploads");
        let mut cfg = ServerConfig::new("files");
        cfg.socket = Some(socket.clone());
        cfg.uploads = uploads.clone();
        cfg.quota = quota;
        let mut server = Server::start(cfg).unwrap();
        let mut core = TestCore::new(3);
        let mut c = ClientConfig::new(Endpoint::Unix(socket), Arc::new(lm_port_fake::demo::registry().unwrap()));
        c.telemetry_hz = 0.0;
        let mut connecting = RemotePort::start(c);
        let client = loop {
            core.pump();
            server.pump(&mut core);
            if let Some(r) = connecting.poll() {
                break r.unwrap();
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        Setup { core, server, client, uploads, done: Vec::new() }
    }

    fn step(&mut self) {
        self.core.pump();
        self.server.pump(&mut self.core);
        self.client.pump();
        self.done.extend(self.client.finished());
    }

    fn wait(&mut self, id: RequestId) -> Finished {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(i) = self.done.iter().position(|(r, _)| *r == id) {
                return self.done.remove(i).1;
            }
            assert!(Instant::now() < end, "no answer to {id}");
            self.step();
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    fn unit(&mut self, p: Pending<()>) -> lm_base::Result<()> {
        match p {
            Pending::Waiting(id) => match self.wait(id) {
                Finished::Done(r) => r,
                other => panic!("{other:?}"),
            },
            Pending::Ready(r) => r,
        }
    }
}

fn noise(bytes: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..bytes)
        .map(|_| {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (x >> 56) as u8
        })
        .collect()
}

#[test]
fn an_upload_lands_hashed_and_imported() {
    let mut s = Setup::new();
    let dir = temp("source");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("take.wav");
    let bytes = noise(5 << 20, 1);
    std::fs::write(&file, &bytes).unwrap();
    let Pending::Waiting(id) = s.client.import(file.clone()) else { panic!("remote imports wait") };
    let Finished::Imported(r) = s.wait(id) else { panic!() };
    assert_eq!(r.expect("imported"), Resource::of_content("audio", &bytes).unwrap());
    // The core imported a file of the uploads folder, named by its hash, and it is gone afterwards
    let path = s.core.log.imports.last().unwrap().clone();
    assert!(path.starts_with(&s.uploads));
    assert!(path.extension().is_some_and(|e| e == "wav"));
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A6: an upload past the global quota is refused, and the import fails instead of waiting
#[test]
fn an_upload_over_the_quota_is_refused() {
    let quota = lm_remote::UploadQuota { max_outstanding: 1 << 20, ..lm_remote::UploadQuota::default() };
    let mut s = Setup::with_quota(quota);
    let file = temp("big.wav");
    std::fs::write(&file, noise(2 << 20, 3)).unwrap();
    let Pending::Waiting(id) = s.client.import(file.clone()) else { panic!("remote imports wait") };
    let Finished::Imported(r) = s.wait(id) else { panic!() };
    assert_eq!(r.unwrap_err().key(), "remote.upload.quota");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn an_upload_resumes_after_a_dropped_connection() {
    let mut s = Setup::new();
    let file = temp("long.flac");
    let bytes = noise(48 << 20, 2);
    std::fs::write(&file, &bytes).unwrap();
    let Pending::Waiting(id) = s.client.import(file.clone()) else { panic!() };
    // Part of it goes, then the connection drops
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        s.step();
        if s.client.upload_progress(id).is_some_and(|(acked, _)| acked >= 12 << 20) {
            break;
        }
        assert!(Instant::now() < end);
    }
    s.client.disconnect();
    let before = s.client.upload_progress(id).unwrap().0;
    // It comes back and the upload goes on from what the server has, not from zero
    let mut lowest_after = u64::MAX;
    let mut reconnected = false;
    let Finished::Imported(r) = ({
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(i) = s.done.iter().position(|(r, _)| *r == id) {
                break s.done.remove(i).1;
            }
            s.step();
            if matches!(s.client.connection(), Connection::Reconnecting { .. }) {
                reconnected = true;
            }
            if reconnected && let Some((acked, _)) = s.client.upload_progress(id) {
                lowest_after = lowest_after.min(acked);
            }
            assert!(Instant::now() < end, "the upload did not finish");
            std::thread::sleep(Duration::from_micros(200));
        }
    }) else {
        panic!()
    };
    assert!(reconnected);
    assert_eq!(r.expect("imported after the reconnection"), Resource::of_content("audio", &bytes).unwrap());
    assert!(lowest_after >= before.min(12 << 20), "resumed from {lowest_after}, had {before}");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn a_file_that_cannot_be_read_fails_cleanly() {
    let mut s = Setup::new();
    let Pending::Waiting(id) = s.client.import(temp("missing.wav")) else { panic!() };
    let Finished::Imported(r) = s.wait(id) else { panic!() };
    assert_eq!(r.unwrap_err().key(), "remote.upload.read");
}

#[test]
fn projects_by_name_inside_the_projects_folder() {
    let mut s = Setup::new();
    let dir = temp("projects");
    for name in ["alpha", "beta"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
        std::fs::write(dir.join(name).join("projeto.json"), "{}").unwrap();
    }
    std::fs::create_dir_all(dir.join("not-a-project")).unwrap();
    let p = s.core.set_preference(vec![PathStep::key("projectsFolder")], Value::text(dir.display().to_string()));
    assert!(matches!(p, Pending::Ready(Ok(()))));
    // The list: the folder's projects (a thread on the server) and the recent ones
    let Pending::Waiting(id) = s.client.list_projects() else { panic!() };
    let list = loop {
        s.step();
        if let Some((_, r)) = s.client.values().into_iter().find(|(r, _)| *r == id) {
            break r.expect("list");
        }
    };
    let names: Vec<&str> = list.get("projects").unwrap().items().filter_map(Value::as_text).collect();
    assert_eq!(names, ["alpha", "beta"]);
    assert!(list.get("recent").is_some());
    // Each request is the session's action with the folder inside the projects folder; each one an operation
    let p = s.client.open_project("alpha");
    s.unit(p).unwrap();
    let p = s.client.new_project("gamma");
    s.unit(p).unwrap();
    let p = s.client.save_project();
    s.unit(p).unwrap();
    let p = s.client.save_project_as("delta");
    s.unit(p).unwrap();
    let p = s.client.close_project();
    s.unit(p).unwrap();
    let folder = |n: &str| Value::object([("folder", Value::text(dir.join(n).display().to_string()))]);
    let actions: Vec<(String, Value)> = s.core.log.actions.clone();
    assert_eq!(
        actions,
        vec![
            ("project.open".to_owned(), folder("alpha")),
            ("project.new".to_owned(), folder("gamma")),
            ("project.save".to_owned(), Value::NULL),
            ("project.save_as".to_owned(), folder("delta")),
            ("project.close".to_owned(), Value::NULL),
        ]
    );
    // Never a path outside: a name with a separator, or a raw action with another folder
    for bad in ["../etc", "a/b", "", ".."] {
        let p = s.client.open_project(bad);
        assert_eq!(s.unit(p).unwrap_err().key(), "remote.project.name", "{bad:?}");
    }
    let p = s.client.run_action(ActionId::from("project.open"), Value::object([("folder", Value::text("/etc"))]));
    assert_eq!(s.unit(p).unwrap_err().key(), "remote.project.outside");
    let p = s.client.run_action(
        ActionId::from("project.open"),
        Value::object([("folder", Value::text(dir.join("alpha").display().to_string()))]),
    );
    s.unit(p).unwrap();
    assert_eq!(s.core.log.actions.len(), 6);
    let _ = std::fs::remove_dir_all(&dir);
}
