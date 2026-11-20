//! The program's sequencer client (I-21.1) on Linux, with two clients of our own routed to each other: scheduled
//! output lands at its time (measured by the receiving port's stamps), SysEx round trips, the waker coalesces, and
//! announcements name what changed. Needs `/dev/snd/seq`; without it the tests say so and pass, unless
//! `LM_TEST_ALSA_SEQ` is set.
#![cfg(target_os = "linux")]
#![allow(clippy::print_stderr)]

use lm_base::HostTime;
use lm_sys_alsa::{Addr, MidiClient, MidiMessage, PortChange, ReadOutcome, SYSEX_MAX, SeqEvent, Stamp};
use std::time::{Duration, Instant};

fn client(name: &str, watch: bool) -> Option<MidiClient> {
    match MidiClient::open(name, watch) {
        Ok(c) => Some(c),
        Err(e) if std::env::var_os("LM_TEST_ALSA_SEQ").is_none() => {
            eprintln!("SKIPPED: no ALSA sequencer here ({e}); load snd-seq or set LM_TEST_ALSA_SEQ to require it");
            None
        }
        Err(e) => panic!("LM_TEST_ALSA_SEQ is set but the sequencer cannot be opened: {e}"),
    }
}

/// A sender with an output port connected to a receiver's input port
fn pair() -> Option<(MidiClient, Addr, MidiClient, Addr)> {
    let mut tx = client("lm-test-sender", false)?;
    let mut rx = client("lm-test-receiver", false)?;
    let out = tx.create_output_port("out").expect("output port");
    let inp = rx.create_input_port("in").expect("input port");
    tx.seq().connect(out, inp).expect("connect");
    Some((tx, out, rx, inp))
}

#[derive(Debug, Clone, PartialEq)]
enum Got {
    Midi(HostTime, Stamp, Vec<u8>),
    Sysex(HostTime, Vec<u8>),
}

fn collect(rx: &mut MidiClient, want: usize, within: Duration) -> Vec<Got> {
    let mut got = Vec::new();
    let deadline = Instant::now() + within;
    while got.len() < want {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let outcome = rx
            .read(Some(left), &mut |e| match e {
                SeqEvent::Midi(m) => got.push(Got::Midi(m.time, m.stamp, m.message.as_slice().to_vec())),
                SeqEvent::Sysex(s) => got.push(Got::Sysex(s.time, s.data.to_vec())),
                SeqEvent::PortsChanged(_) => {}
            })
            .expect("read");
        if outcome == ReadOutcome::TimedOut {
            break;
        }
    }
    got
}

#[test]
fn scheduled_output_leaves_at_its_time_within_a_millisecond() {
    let Some((mut tx, out, mut rx, _)) = pair() else { return };
    if tx.stamps_on_arrival() || rx.stamps_on_arrival() {
        panic!("no real-time queue: output cannot be scheduled");
    }
    let start = HostTime(HostTime::now().0 + 50_000_000);
    let targets: Vec<HostTime> = (0..20u64).map(|i| HostTime(start.0 + i * 7_300_000)).collect();
    // Sent in one burst, long before their times: the queue holds them
    for (i, at) in targets.iter().enumerate() {
        tx.send_at(out, None, &MidiMessage::new3(0x90, i as u8, 100), *at).expect("send_at");
    }
    let got = collect(&mut rx, targets.len(), Duration::from_secs(3));
    assert_eq!(got.len(), targets.len(), "{got:?}");
    let mut worst = 0i64;
    for (i, (g, at)) in got.iter().zip(&targets).enumerate() {
        let Got::Midi(time, stamp, bytes) = g else { panic!("{g:?}") };
        assert_eq!(*stamp, Stamp::Queue);
        assert_eq!(bytes, &vec![0x90, i as u8, 100]);
        let err = time.0 as i64 - at.0 as i64;
        worst = worst.max(err.abs());
    }
    eprintln!("scheduled output: worst timing error {} us", worst / 1000);
    assert!(worst < 1_000_000, "worst timing error {worst} ns");
    // A time already past leaves at once
    let now = HostTime::now();
    tx.send_at(out, None, &MidiMessage::new3(0x80, 1, 0), HostTime(now.0 - 1_000_000_000)).expect("past");
    let got = collect(&mut rx, 1, Duration::from_secs(1));
    assert_eq!(got.len(), 1);
    assert_eq!(tx.stats().unscheduled, 0);
}

#[test]
fn sysex_round_trips_up_to_the_limit() {
    let Some((mut tx, out, mut rx, inp)) = pair() else { return };
    for len in [2usize, 6, 300, 1500, SYSEX_MAX] {
        let mut data: Vec<u8> = (0..len).map(|i| (i % 128) as u8).collect();
        data[0] = 0xf0;
        data[len - 1] = 0xf7;
        let at = HostTime(HostTime::now().0 + 5_000_000);
        tx.send_sysex_at(out, Some(inp), &data, at).expect("send sysex");
        let got = collect(&mut rx, 1, Duration::from_secs(2));
        match got.as_slice() {
            [Got::Sysex(time, back)] => {
                assert_eq!(back, &data, "{len} bytes");
                assert!((time.0 as i64 - at.0 as i64).abs() < 1_000_000, "{len} bytes at {time}, wanted {at}");
            }
            other => panic!("{len} bytes: {other:?}"),
        }
    }
    let too_long = vec![0xf0; SYSEX_MAX + 1];
    assert_eq!(tx.send_sysex_at(out, None, &too_long, HostTime::now()).unwrap_err().key(), "alsa.invalid_argument");
    assert_eq!(tx.send_sysex_at(out, None, &[0x90, 1, 2], HostTime::now()).unwrap_err().key(), "alsa.invalid_argument");
    // SysEx and short messages scheduled at one time leave in the order they were sent
    let at = HostTime(HostTime::now().0 + 2_000_000);
    tx.send_at(out, None, &MidiMessage::new3(0xb0, 7, 1), at).expect("cc");
    tx.send_sysex_at(out, None, &[0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7], at).expect("identity request");
    tx.send_at(out, None, &MidiMessage::new3(0xb0, 7, 2), at).expect("cc");
    let got = collect(&mut rx, 3, Duration::from_secs(2));
    assert!(matches!(got.as_slice(), [Got::Midi(..), Got::Sysex(..), Got::Midi(..)]), "{got:?}");
    assert_eq!(rx.stats().sysex_dropped, 0);
}

#[test]
fn many_wakes_give_one_woken_and_the_stopper_ends_the_reader() {
    let Some(mut c) = client("lm-test-waker", false) else { return };
    let waker = c.waker();
    for _ in 0..10_000 {
        waker.wake();
    }
    assert_eq!(c.read(Some(Duration::from_millis(100)), &mut |_| {}).expect("read"), ReadOutcome::Woken);
    assert_eq!(c.read(Some(Duration::from_millis(50)), &mut |_| {}).expect("read"), ReadOutcome::TimedOut);
    // A wake from another thread ends a blocked read
    let waking = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        waker.wake();
    });
    assert_eq!(c.read(None, &mut |_| {}).expect("read"), ReadOutcome::Woken);
    waking.join().expect("join");
    let stopper = c.stopper();
    let reader = std::thread::spawn(move || c.read(None, &mut |_| {}).expect("read"));
    std::thread::sleep(Duration::from_millis(50));
    stopper.stop();
    assert_eq!(reader.join().expect("reader"), ReadOutcome::Stopped);
}

#[test]
fn announcements_name_the_port_that_came_and_went() {
    let Some(mut watcher) = client("lm-test-watcher", true) else { return };
    let mut other = lm_sys_alsa::Seq::open("lm-test-hotplug").expect("client");
    let port = other.create_output_port("comes and goes").expect("port");
    let mut seen = Vec::new();
    let mut wait_for = |watcher: &mut MidiClient, want: PortChange| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !seen.contains(&want) && Instant::now() < deadline {
            watcher
                .read(Some(Duration::from_millis(200)), &mut |e| {
                    if let SeqEvent::PortsChanged(c) = e {
                        seen.push(c);
                    }
                })
                .expect("read");
        }
        assert!(seen.contains(&want), "{want:?} not in {seen:?}");
    };
    wait_for(&mut watcher, PortChange::PortStart(port));
    other.delete_port(port).expect("delete");
    wait_for(&mut watcher, PortChange::PortExit(port));
    let client = other.client_id();
    drop(other);
    wait_for(&mut watcher, PortChange::ClientExit(client));
}
