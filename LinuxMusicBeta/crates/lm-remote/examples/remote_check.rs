//! Connects to a server as the interface would and checks the round trip: the mirror, an edit and its diff, undo,
//! a gesture, telemetry, peaks; prints the timings.
//!
//! `remote_check (--socket PATH | --tcp HOST:PORT) [--token TOKEN]`

#![allow(clippy::print_stdout)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use lm_base::Label;
use lm_doc::Topic;
use lm_port::{CorePort, EditOp, Finished, NodeId, Pending, RequestId, Value};
use lm_remote::{ClientConfig, Endpoint, RemotePort};

fn wait(port: &mut RemotePort, id: RequestId) -> Finished {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        port.pump();
        if let Some((_, f)) = port.finished().into_iter().find(|(r, _)| *r == id) {
            return f;
        }
        assert!(Instant::now() < end, "no answer");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut endpoint = None;
    let mut token = None;
    let mut fake = false;
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--socket" => endpoint = Some(Endpoint::Unix(v.into())),
            "--tcp" => endpoint = Some(Endpoint::Tcp(v)),
            "--token" => token = Some(v),
            // The schemas of the fake core (serve_fake) instead of the session's (lm-headless serve)
            "--fake" => {
                fake = true;
                i += 1;
                continue;
            }
            other => panic!("unknown argument {other}"),
        }
        i += 2;
    }
    let schemas = if fake {
        Arc::new(lm_port_fake::demo::registry().expect("registry"))
    } else {
        Arc::new(lm_session::internal_plugins().and_then(|p| p.project_schemas()).expect("the session's schemas"))
    };
    let mut cfg = ClientConfig::new(endpoint.expect("--socket or --tcp"), schemas);
    cfg.token = token;
    let t0 = Instant::now();
    let mut port = RemotePort::connect(cfg).expect("connect");
    let root_bytes = lm_state::to_json_compact(port.root()).map(|s| s.len()).unwrap_or(0);
    println!("connected in {:?}: {:?}; tree {} KB", t0.elapsed(), port.connection(), root_bytes / 1024);
    let mut feed = port.feed(Topic::All);
    let mut rtt = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        let Pending::Waiting(id) = port.ping() else { panic!("ping") };
        wait(&mut port, id);
        rtt.push(t.elapsed());
    }
    rtt.sort();
    println!("round trip: median {:?}, worst {:?}", rtt[rtt.len() / 2], rtt[rtt.len() - 1]);
    let first =
        NodeId::parse(port.root().get("tracks").unwrap().at(0).unwrap().get("id").unwrap().as_text().unwrap()).unwrap();
    let t = Instant::now();
    let Pending::Waiting(id) =
        port.apply(Label::new("check.rename"), vec![EditOp::set(first, &["name"], Value::text("remote"))])
    else {
        panic!("apply")
    };
    let Finished::Applied(r) = wait(&mut port, id) else { panic!("applied") };
    r.expect("rename");
    println!("edit applied and mirrored in {:?}", t.elapsed());
    assert_eq!(port.find(first).unwrap().get("name").unwrap().as_text(), Some("remote"));
    let d = port.drain(&mut feed).expect("its diff");
    println!("diff: {} changed node(s)", d.changed.len());
    let Pending::Waiting(id) = port.undo() else { panic!("undo") };
    let Finished::Done(r) = wait(&mut port, id) else { panic!("undo") };
    r.expect("undo");
    assert_ne!(port.find(first).unwrap().get("name").unwrap().as_text(), Some("remote"));
    let g = port.begin_gesture(Label::new("check.drag"));
    let t = Instant::now();
    for k in 0..60 {
        port.gesture_apply(&g, vec![EditOp::set(first, &["volume"], Value::real(f64::from(k) / 60.0).unwrap())]);
        port.param_write(first, "volume", k as f32 / 60.0);
        port.pump();
    }
    port.gesture_commit(g);
    println!("60 gesture updates sent in {:?} (never waiting)", t.elapsed());
    let Pending::Waiting(id) = port.ping() else { panic!("ping") };
    wait(&mut port, id);
    println!("volume after the drag: {:?}", port.find(first).unwrap().get("volume"));
    let t = Instant::now();
    while port.telemetry().position.sample_rate == 0 && t.elapsed() < Duration::from_secs(5) {
        port.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("telemetry: rate {} Hz, {} meters", port.telemetry().position.sample_rate, port.telemetry().meters.len());
    println!("ok");
}
