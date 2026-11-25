//! Against a real JACK server: each test starts `jackd` with the dummy driver under a name of its own (no sound card,
//! no other program disturbed). Without `jackd` on the PATH the tests say so and pass.

#![allow(clippy::print_stderr)]

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lm_sys_jack::{Client, Cycle, Direction, LatencyMode, Notify, Port, PortFilter, PortKind, Process};

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// A jackd of the test's own, killed when dropped
struct Server {
    name: String,
    child: Child,
}

impl Server {
    fn start(tag: &str) -> Option<Server> {
        let name = format!("lmtest-{}-{tag}", std::process::id());
        let child = Command::new("jackd")
            .args(["--no-realtime", "-n", &name, "-d", "dummy", "-r", "48000", "-p", "256", "-C", "2", "-P", "2"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(child) = child else {
            eprintln!("jackd is not on the PATH: the JACK tests are skipped");
            return None;
        };
        let server = Server { name, child };
        // Until it answers
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if Client::open("lm-probe", Some(&server.name)).is_ok() {
                return Some(server);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("jackd did not start: the JACK tests are skipped");
        None
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
    }
}

fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Default)]
struct Heard {
    xruns: AtomicU32,
    shutdown: AtomicBool,
    registrations: AtomicU32,
    connections: AtomicU32,
}

struct Notes(Arc<Heard>);

impl Notify for Notes {
    fn xrun(&self, _delayed_usecs: f32) {
        self.0.xruns.fetch_add(1, Ordering::AcqRel);
    }
    fn shutdown(&self, _reason: &str) {
        self.0.shutdown.store(true, Ordering::Release);
    }
    fn port_registration(&self, _name: &str, _registered: bool) {
        self.0.registrations.fetch_add(1, Ordering::AcqRel);
    }
    fn port_connection(&self, _source: &str, _destination: &str, _connected: bool) {
        self.0.connections.fetch_add(1, Ordering::AcqRel);
    }
}

/// Sends a constant and a note every cycle, and records what comes back
struct Loop {
    audio_out: Port,
    audio_in: Port,
    midi_out: Port,
    midi_in: Port,
    scratch: Vec<f32>,
    cycles: Arc<AtomicU64>,
    heard_level: Arc<AtomicU32>,
    heard_notes: Arc<AtomicU32>,
    times_ok: Arc<AtomicBool>,
}

impl Process for Loop {
    fn process(&mut self, cycle: &mut Cycle<'_>) {
        assert_no_alloc::assert_no_alloc(|| {
            let n = cycle.frames() as usize;
            self.scratch[..n].fill(0.25);
            cycle.write_audio(self.audio_out, &self.scratch[..n]);
            assert!(cycle.write_midi(self.midi_out, 5, &[0x90, 60, 100]));
            self.scratch[..n].fill(0.0);
            cycle.read_audio(self.audio_in, &mut self.scratch[..n]);
            self.heard_level.store(self.scratch[n - 1].to_bits(), Ordering::Release);
            let notes = &self.heard_notes;
            cycle.read_midi(self.midi_in, |time, bytes| {
                if time == 5 && bytes == [0x90, 60, 100] {
                    notes.fetch_add(1, Ordering::AcqRel);
                }
            });
            if let Some(t) = cycle.times()
                && t.next_usecs > t.usecs
            {
                self.times_ok.store(true, Ordering::Release);
            }
            self.cycles.fetch_add(1, Ordering::AcqRel);
        });
    }
}

#[test]
fn audio_and_midi_go_round_a_loop_of_connections() {
    let Some(server) = Server::start("loop") else { return };
    let mut client = Client::open("lm-loop", Some(&server.name)).unwrap();
    assert_eq!(client.sample_rate(), 48000);
    assert_eq!(client.buffer_size(), 256);
    let audio_out = client.register("out", PortKind::Audio, Direction::Output).unwrap();
    let audio_in = client.register("in", PortKind::Audio, Direction::Input).unwrap();
    let midi_out = client.register("midi_out", PortKind::Midi, Direction::Output).unwrap();
    let midi_in = client.register("midi_in", PortKind::Midi, Direction::Input).unwrap();
    let out_name = client.port_name(audio_out);
    assert_eq!(out_name, format!("{}:out", client.name()));

    let heard = Arc::new(Heard::default());
    let cycles = Arc::new(AtomicU64::new(0));
    let level = Arc::new(AtomicU32::new(0));
    let notes = Arc::new(AtomicU32::new(0));
    let times_ok = Arc::new(AtomicBool::new(false));
    let process = Loop {
        audio_out,
        audio_in,
        midi_out,
        midi_in,
        scratch: vec![0.0; 8192],
        cycles: cycles.clone(),
        heard_level: level.clone(),
        heard_notes: notes.clone(),
        times_ok: times_ok.clone(),
    };
    let (in_name, midi_out_name, midi_in_name) =
        (client.port_name(audio_in), client.port_name(midi_out), client.port_name(midi_in));
    let active = client.activate(Box::new(process), Arc::new(Notes(heard.clone()))).map_err(|(e, _)| e).unwrap();
    // Its own output to its own input: the buffers are the same memory, which the copies make safe
    active.client().connect(&out_name, &in_name).unwrap();
    active.client().connect(&midi_out_name, &midi_in_name).unwrap();
    // Connecting twice is fine
    active.client().connect(&out_name, &in_name).unwrap();

    wait_for("the cycles", || cycles.load(Ordering::Acquire) > 20);
    wait_for("the level", || f32::from_bits(level.load(Ordering::Acquire)) == 0.25);
    wait_for("the notes", || notes.load(Ordering::Acquire) > 5);
    assert!(times_ok.load(Ordering::Acquire), "the cycle times");
    wait_for("the connection notifications", || heard.connections.load(Ordering::Acquire) >= 2);
    assert!(!active.panicked());

    // The server's own ports: the dummy driver's two captures and two playbacks
    let physical = active.client().ports(PortFilter {
        kind: Some(PortKind::Audio),
        direction: Some(Direction::Output),
        physical: true,
    });
    assert_eq!(physical.len(), 2, "{physical:?}");
    assert!(active.client().port_latency(&physical[0], LatencyMode::Capture).is_some());
    assert!(active.client().port_latency("nobody:nothing", LatencyMode::Capture).is_none());

    let mut active = active;
    active.deactivate();
    let after = cycles.load(Ordering::Acquire);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(cycles.load(Ordering::Acquire), after, "no cycle after deactivating");
    let _process = active.close();
}

struct Panics;

impl Process for Panics {
    fn process(&mut self, _cycle: &mut Cycle<'_>) {
        panic!("a bug in the process handler");
    }
}

struct Nothing;

impl Notify for Nothing {}

#[test]
fn a_panic_in_the_process_handler_never_reaches_jack() {
    let Some(server) = Server::start("panic") else { return };
    let client = Client::open("lm-panic", Some(&server.name)).unwrap();
    let active = client.activate(Box::new(Panics), Arc::new(Nothing)).map_err(|(e, _)| e).unwrap();
    wait_for("the panic", || active.panicked());
    drop(active);
}

struct Slow {
    once: bool,
}

impl Process for Slow {
    fn process(&mut self, _cycle: &mut Cycle<'_>) {
        if !self.once {
            self.once = true;
            std::thread::sleep(Duration::from_millis(60));
        }
    }
}

#[test]
fn xruns_and_the_server_going_away_are_heard() {
    let Some(mut server) = Server::start("shutdown") else { return };
    let client = Client::open("lm-shutdown", Some(&server.name)).unwrap();
    let heard = Arc::new(Heard::default());
    let active =
        client.activate(Box::new(Slow { once: false }), Arc::new(Notes(heard.clone()))).map_err(|(e, _)| e).unwrap();
    wait_for("an xrun", || heard.xruns.load(Ordering::Acquire) > 0);
    server.kill();
    wait_for("the shutdown", || heard.shutdown.load(Ordering::Acquire));
    // Closing a client whose server went away does not hang
    drop(active);
}

#[test]
fn no_server_is_a_clear_error() {
    let e = Client::open("lm-alone", Some("lm-no-such-server")).unwrap_err();
    assert_eq!(e.key(), "jack.server_not_running");
}
