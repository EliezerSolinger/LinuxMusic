//! I-20.8: a 64-track project of one hour opens in a remote client with an empty cache and shows its first screen
//! with less than 2 MB received; reopened, it receives no waveform at all.

#![allow(clippy::print_stderr)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use common::{HOUR, TestCore};
use lm_port::{CorePort, Finished, NodeId, Pending, Resource};
use lm_remote::{ClientConfig, Endpoint, RemotePort, Server, ServerConfig};

fn temp(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!("lmr-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

fn connect(server: &mut Server, core: &mut TestCore, cfg: ClientConfig) -> RemotePort {
    let mut c = RemotePort::start(cfg);
    loop {
        core.pump();
        server.pump(core);
        if let Some(r) = c.poll() {
            return r.expect("connect");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The first screen: every track's waveform across 2000 pixels, the hour fitted in
fn first_screen(server: &mut Server, core: &mut TestCore, client: &mut RemotePort) -> usize {
    let tracks: Vec<NodeId> = client
        .root()
        .get("tracks")
        .unwrap()
        .items()
        .map(|t| NodeId::parse(t.get("id").unwrap().as_text().unwrap()).unwrap())
        .collect();
    // 2000 pixels for the hour: 86,400 samples a pixel, so level 5 (65,536 samples a pair) and its 2637 pairs
    let level = 5u8;
    let pairs = HOUR.div_ceil(64 * 4u64.pow(5));
    let mut waiting = Vec::new();
    let mut ready = 0;
    for t in &tracks {
        let res = Resource::of_content("audio", t.to_string().as_bytes()).unwrap();
        match client.peaks(&res, level, 0, pairs) {
            Pending::Ready(Ok(b)) => {
                assert_eq!(b.pairs.len() as u64, pairs);
                ready += 1;
            }
            Pending::Waiting(id) => waiting.push(id),
            Pending::Ready(Err(e)) => panic!("{e}"),
        }
    }
    let end = Instant::now() + Duration::from_secs(20);
    while !waiting.is_empty() {
        core.pump();
        server.pump(core);
        client.pump();
        for (id, f) in client.finished() {
            if let Finished::Peaks(r) = f {
                assert_eq!(r.expect("peaks").pairs.len() as u64, pairs);
                waiting.retain(|w| *w != id);
            }
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_micros(200));
    }
    assert_eq!(tracks.len(), 64);
    ready
}

#[test]
fn an_hour_of_64_tracks_opens_under_2_mb_and_reopens_without_waveforms() {
    let socket = temp("sock");
    let cache = temp("peaks");
    let mut cfg = ServerConfig::new("measure");
    cfg.socket = Some(socket.clone());
    let mut server = Server::start(cfg).unwrap();
    let mut core = TestCore::new(64);
    let schemas = Arc::new(lm_port_fake::demo::registry().unwrap());
    let client_cfg = || {
        let mut c = ClientConfig::new(Endpoint::Unix(socket.clone()), Arc::clone(&schemas));
        c.peaks_cache = Some(cache.clone());
        // The first screen only: no meters running
        c.telemetry_hz = 0.0;
        c
    };

    // Empty cache
    let mut client = connect(&mut server, &mut core, client_cfg());
    let from_cache = first_screen(&mut server, &mut core, &mut client);
    assert_eq!(from_cache, 0);
    let t = client.traffic();
    eprintln!(
        "first open: tree {} KB, waveforms {} KB, other {} KB; total {:.2} MB",
        t.snapshots / 1024,
        t.peaks / 1024,
        t.messages / 1024,
        t.total() as f64 / (1 << 20) as f64
    );
    assert!(t.total() < 2 << 20, "the first screen took {} bytes", t.total());
    assert!(t.peaks > 0);
    let served = core.log.peaks_served;
    drop(client);

    // Reopened by the same client: the waveforms come from its cache, none over the network
    let mut client = connect(&mut server, &mut core, client_cfg());
    let from_cache = first_screen(&mut server, &mut core, &mut client);
    let t = client.traffic();
    eprintln!("reopened: tree {} KB, waveforms {} B", t.snapshots / 1024, t.peaks);
    assert_eq!(from_cache, 64);
    assert_eq!(t.peaks, 0);
    assert_eq!(core.log.peaks_served, served);
    let _ = std::fs::remove_dir_all(&cache);
}
