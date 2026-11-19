//! ReserveDevice1 against a private session bus (a `dbus-daemon` started by the test, so it runs on CI machines
//! without a desktop), with a fake audio server playing PipeWire's part. Skips with a message when `dbus-daemon` is
//! not installed. One test in its own binary: it sets DBUS_SESSION_BUS_ADDRESS for the process.
#![cfg(target_os = "linux")]
#![allow(clippy::print_stderr)]

use lm_sys_linux::{ReserveRequest, SystemBus};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn private_bus() -> Option<(Daemon, String)> {
    let child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIPPED: no dbus-daemon to run a private session bus ({e}); install the `dbus` package");
            return None;
        }
    };
    let mut line = String::new();
    BufReader::new(child.stdout.take()?).read_line(&mut line).ok()?;
    let daemon = Daemon(child);
    let address = line.trim().to_owned();
    if address.is_empty() {
        eprintln!("SKIPPED: dbus-daemon printed no address");
        return None;
    }
    Some((daemon, address))
}

/// A fake audio server's ReserveDevice1 object: answers RequestRelease with `agree`
struct FakeServer {
    agree: bool,
    asked: Arc<AtomicU32>,
}

#[zbus::interface(name = "org.freedesktop.ReserveDevice1")]
impl FakeServer {
    async fn request_release(&self, priority: i32) -> bool {
        assert!(priority > 0);
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.agree
    }

    #[zbus(property)]
    async fn priority(&self) -> i32 {
        0
    }

    #[zbus(property)]
    async fn application_name(&self) -> String {
        "FakeAudioServer".to_owned()
    }

    #[zbus(property)]
    async fn application_device_name(&self) -> String {
        "Fake card".to_owned()
    }
}

/// The fake server holds card `card`'s name, allowing replacement as PipeWire does
async fn fake_server(address: &str, card: u32, agree: bool) -> (zbus::Connection, Arc<AtomicU32>) {
    let conn = zbus::connection::Builder::address(address).build().await.expect("fake server connects");
    let asked = Arc::new(AtomicU32::new(0));
    conn.object_server()
        .at(format!("/org/freedesktop/ReserveDevice1/Audio{card}"), FakeServer { agree, asked: asked.clone() })
        .await
        .expect("fake object");
    let reply = conn
        .request_name_with_flags(
            format!("org.freedesktop.ReserveDevice1.Audio{card}"),
            RequestNameFlags::ALLOW_REPLACEMENT | RequestNameFlags::DO_NOT_QUEUE,
        )
        .await
        .expect("fake server takes the name");
    assert_eq!(reply, RequestNameReply::PrimaryOwner);
    (conn, asked)
}

fn request(card: u32) -> ReserveRequest {
    ReserveRequest {
        card,
        application_name: "LinuxMusic".to_owned(),
        device_name: "Test card".to_owned(),
        priority: 20,
    }
}

const WAIT: Duration = Duration::from_secs(15);

#[test]
fn reserve_device_takes_a_card_from_a_willing_owner_and_keeps_it() {
    let Some((_daemon, address)) = private_bus() else { return };
    // SAFETY: the only test of this binary sets it before any thread of ours reads the environment
    unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &address) };

    // A free card is simply taken
    let bus = SystemBus::start().expect("bus thread");
    let free = bus.reserve_audio_device(request(3)).wait(WAIT).expect("a free card");
    assert_eq!(free.card(), 3);
    let again = bus.reserve_audio_device(request(3)).wait(WAIT).expect_err("held twice");
    assert_eq!(again.key(), "linux.reserve.already_held");

    // An owner that agrees gives the card up
    let (_server, asked) = zbus::block_on(fake_server(&address, 5, true));
    let taken = bus.reserve_audio_device(request(5)).wait(WAIT).expect("the willing owner releases the card");
    assert_eq!(asked.load(Ordering::SeqCst), 1);

    // Another program asking us is refused, and counted
    let other = SystemBus::start().expect("second bus thread");
    let refused = other.reserve_audio_device(request(5)).wait(WAIT).expect_err("we keep the card");
    assert_eq!(refused.key(), "linux.reserve.refused", "{refused}");
    assert!(
        refused.label.args.iter().any(|(n, v)| *n == "owner" && *v == lm_base::Arg::from("LinuxMusic")),
        "{refused}"
    );
    assert_eq!(taken.release_requests(), 1);

    // Dropping the reservation gives the name back
    drop(taken);
    let mut regained = None;
    for _ in 0..100 {
        match other.reserve_audio_device(request(5)).wait(WAIT) {
            Ok(r) => {
                regained = Some(r);
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    assert!(regained.is_some(), "the card was not given back on drop");

    // An owner that refuses keeps it, and the error names it
    let (_stubborn, asked) = zbus::block_on(fake_server(&address, 6, false));
    let e = bus.reserve_audio_device(request(6)).wait(WAIT).expect_err("the owner refuses");
    assert_eq!(e.key(), "linux.reserve.refused");
    assert!(e.label.args.iter().any(|(n, v)| *n == "owner" && *v == lm_base::Arg::from("FakeAudioServer")), "{e}");
    assert_eq!(asked.load(Ordering::SeqCst), 1);

    drop(free);
    drop(regained);
    assert!(bus.shutdown(WAIT));
    assert!(other.shutdown(WAIT));
}
