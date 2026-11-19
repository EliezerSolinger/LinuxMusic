//! Sequencer tests on Linux, with our own ports (snd-seq-dummy is usually missing on CI runners): a sending client
//! routed to a `MidiInput`. They need `/dev/snd/seq` (the `snd-seq` module); without it they say so and pass, unless
//! `LM_TEST_ALSA_SEQ` is set, which makes a missing sequencer a failure.
#![cfg(target_os = "linux")]
#![allow(clippy::print_stderr)]

use lm_base::HostTime;
use lm_sys_alsa::{MidiInput, MidiInputEvent, MidiMessage, ReadOutcome, Seq, SeqInput, Stamp};
use std::time::Duration;

fn input(watch: bool) -> Option<MidiInput> {
    match MidiInput::open("lm-test-input", "in", watch) {
        Ok(i) => Some(i),
        Err(e) if std::env::var_os("LM_TEST_ALSA_SEQ").is_none() => {
            eprintln!("SKIPPED: no ALSA sequencer here ({e}); load snd-seq or set LM_TEST_ALSA_SEQ to require it");
            None
        }
        Err(e) => panic!("LM_TEST_ALSA_SEQ is set but the sequencer cannot be opened: {e}"),
    }
}

/// Reads until `want` MIDI messages arrived or 2 s passed
fn collect(input: &mut MidiInput, want: usize) -> Vec<MidiInputEvent> {
    let mut got = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while got.len() < want {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let outcome = input
            .read(Some(left), &mut |item| {
                if let SeqInput::Midi(e) = item {
                    got.push(e);
                }
            })
            .expect("read");
        if outcome == ReadOutcome::TimedOut {
            break;
        }
    }
    got
}

#[test]
fn messages_cross_the_kernel_unchanged_and_on_the_host_clock() {
    let Some(mut input) = input(false) else { return };
    let mut out = Seq::open("lm-test-output").expect("open output client");
    let port = out.create_output_port("out").expect("output port");
    out.connect(port, input.addr()).expect("connect");

    let messages: [&[u8]; 14] = [
        &[0x90, 60, 100],
        &[0x80, 60, 64],
        &[0xa1, 61, 3],
        &[0xb2, 7, 99],
        &[0xc3, 5],
        &[0xd4, 77],
        &[0xe5, 0x12, 0x34],
        &[0xf2, 1, 2],
        &[0xf3, 4],
        &[0xf8],
        &[0xfa],
        &[0xfb],
        &[0xfc],
        &[0xfe],
    ];
    let before = HostTime::now();
    for m in messages {
        out.send(port, None, &MidiMessage::from_bytes(m).expect("valid")).expect("send");
    }
    let got = collect(&mut input, messages.len());
    let after = HostTime::now();
    let bytes: Vec<&[u8]> = got.iter().map(|e| e.message.as_slice()).collect();
    assert_eq!(bytes, messages.to_vec());
    for e in &got {
        assert_eq!(e.source, port);
        assert_eq!(e.port, input.addr());
        let expected = if input.stamps_on_arrival() { Stamp::Arrival } else { Stamp::Queue };
        assert_eq!(e.stamp, expected);
        // C6: the stamp is on CLOCK_MONOTONIC, between the send and the read (2 ms of slack for the conversion)
        assert!(
            e.time.0 + 2_000_000 >= before.0 && e.time.0 <= after.0 + 2_000_000,
            "{} not in [{before}, {after}]",
            e.time
        );
    }
    assert_eq!(input.stats().overruns, 0);
}

#[test]
fn a_disconnected_port_sends_nothing_and_direct_sends_reach_one_port() {
    let Some(mut input) = input(false) else { return };
    let mut out = Seq::open("lm-test-output").expect("open output client");
    let port = out.create_output_port("out").expect("output port");
    out.connect(port, input.addr()).expect("connect");
    out.connect(port, input.addr()).expect("connecting twice is not an error");
    out.disconnect(port, input.addr()).expect("disconnect");
    out.disconnect(port, input.addr()).expect("disconnecting twice is not an error");
    out.send(port, None, &MidiMessage::new3(0x90, 1, 2)).expect("send to no subscriber");
    assert!(collect(&mut input, 1).is_empty());
    out.send(port, Some(input.addr()), &MidiMessage::new3(0x91, 3, 4)).expect("direct send");
    let got = collect(&mut input, 1);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].message.as_slice(), &[0x91, 3, 4]);
}

#[test]
fn ports_are_listed_with_their_directions() {
    let Some(input) = input(false) else { return };
    let mut out = Seq::open("lm-test-lister").expect("open client");
    let port = out.create_output_port("lister out").expect("port");
    let ports = out.ports().expect("ports");
    let ours = ports.iter().find(|p| p.addr == port).expect("our output port is listed");
    assert!(ours.readable && !ours.writable && ours.midi && !ours.hardware);
    assert_eq!(ours.port_name, "lister out");
    assert_eq!(ours.client_name, "lm-test-lister");
    let theirs = ports.iter().find(|p| p.addr == input.addr()).expect("the input port is listed");
    assert!(theirs.writable && !theirs.readable);
    assert!(ports.iter().any(|p| p.system));
    assert_eq!(out.connect(port, lm_sys_alsa::Addr::new(250, 250)).unwrap_err().key(), "alsa.seq_port_not_found");
}

#[test]
fn the_stopper_wakes_a_blocked_reader() {
    let Some(mut input) = input(false) else { return };
    let stopper = input.stopper();
    let reader = std::thread::spawn(move || input.read(None, &mut |_| {}).expect("read"));
    std::thread::sleep(Duration::from_millis(100));
    stopper.stop();
    assert_eq!(reader.join().expect("reader thread"), ReadOutcome::Stopped);
}

#[test]
fn port_changes_are_announced_when_watched() {
    let Some(mut input) = input(true) else { return };
    let mut other = Seq::open("lm-test-hotplug").expect("open client");
    other.create_output_port("appears").expect("port");
    let mut changed = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !changed && std::time::Instant::now() < deadline {
        input
            .read(Some(Duration::from_millis(200)), &mut |item| changed |= item == SeqInput::PortsChanged)
            .expect("read");
    }
    assert!(changed, "a new port was not announced");
}
