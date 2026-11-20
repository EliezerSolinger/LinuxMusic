//! Serves the interface's fake core (lm-port-fake) over the protocol, for trying the RemotePort from another
//! machine before `lm-headless serve` has the session behind it:
//!
//! `serve_fake [--socket PATH] [--tcp ADDR] [--token TOKEN:ROLE]... [--seconds N]`
//!
//! From another machine: `ssh -N -L 7400:127.0.0.1:7400 host`, then the client connects to `127.0.0.1:7400`.

#![allow(clippy::print_stderr)]

use std::time::{Duration, Instant};

use lm_port::CorePort;
use lm_port_fake::{Config, FakePort};
use lm_remote::{Role, Server, ServerConfig};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = ServerConfig::new("serve_fake");
    let mut seconds = 600u64;
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--socket" => cfg.socket = Some(v.into()),
            "--tcp" => cfg.tcp = Some(v.parse().expect("--tcp ADDR")),
            "--token" => {
                let (t, r) = v.split_once(':').expect("--token TOKEN:ROLE");
                cfg.tokens.push((t.to_owned(), Role::parse(r).expect("observe, control or admin")));
            }
            "--seconds" => seconds = v.parse().expect("--seconds N"),
            other => panic!("unknown argument {other}"),
        }
        i += 2;
    }
    let mut core = FakePort::new(Config { tracks: 64, slow_delay: None, slow_track: 0 }).expect("fake core");
    let mut server = Server::start(cfg).expect("server");
    eprintln!("serving: tcp {:?}", server.tcp_addr());
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut clients = 0;
    while Instant::now() < end {
        core.pump();
        server.pump(&mut core);
        if server.clients() != clients {
            clients = server.clients();
            eprintln!("clients: {clients}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
