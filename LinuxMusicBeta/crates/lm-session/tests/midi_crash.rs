//! I-21.6: `kill -9` in the middle of a MIDI recording gives back everything up to the last second. A child process
//! (this same test binary, by an environment variable) records notes on an armed track and writes down when it
//! played each one; the parent kills it with SIGKILL, opens the project and recovers the take

mod common;

use std::io::Write;
use std::time::{Duration, Instant};

use common::*;
use lm_base::HostTime;
use lm_render::{Command, PortIndex};
use lm_session::Session;
use lm_state::Value;

const CHILD: &str = "LM_MIDI_CRASH_CHILD";

fn notes_in_midi_items(s: &Session) -> usize {
    s.document()
        .root()
        .get("tracks")
        .map(|l| {
            l.items()
                .flat_map(|t| t.get("items").map(|i| i.items().cloned().collect::<Vec<_>>()).unwrap_or_default())
                .filter(|i| i.get("kind").and_then(Value::as_text) == Some("midi"))
                .map(|i| i.get("notes").map_or(0, Value::len))
                .sum()
        })
        .unwrap_or(0)
}

#[test]
fn kill_9_while_recording_midi_keeps_everything_up_to_the_last_second() {
    if let Some(folder) = std::env::var_os(CHILD) {
        let folder = std::path::PathBuf::from(folder);
        let dir = TempDir::new("midi-crash-child");
        let mut s = session(&dir);
        open(&mut s, folder.clone());
        s.record(None).expect("record");
        let start = Instant::now();
        while !s.telemetry().position.recording {
            s.pump();
            std::thread::sleep(Duration::from_millis(2));
            assert!(start.elapsed() < Duration::from_secs(20), "recording did not start");
        }
        let mut log = std::fs::File::create(folder.join("played.txt")).expect("log");
        let begun = Instant::now();
        for n in 0u64..u64::MAX {
            let key = 36 + (n % 48) as u8;
            for bytes in [[0x90, key, 100], [0x80, key, 0]] {
                s.engine()
                    .commands()
                    .send(Command::Midi { port: PortIndex(0), at: HostTime::now(), bytes })
                    .expect("midi");
            }
            // Played: once the renderer has it and the session has read it (the next pumps)
            let until = Instant::now() + Duration::from_millis(40);
            while Instant::now() < until {
                s.pump();
                std::thread::sleep(Duration::from_millis(2));
            }
            writeln!(log, "{} {}", begun.elapsed().as_millis(), n + 1).expect("write");
            log.flush().expect("flush");
        }
    }
    let dir = TempDir::new("midi-crash");
    let folder = dir.join("song");
    {
        let mut s = session(&dir);
        s.create(folder.clone()).expect("create");
        let t = add_synth_track(&mut s, "Keys");
        set(&mut s, t, "armed", Value::bool(true));
        settle(&mut s);
        s.save().expect("save");
    }
    let mut child = std::process::Command::new(std::env::current_exe().expect("exe"))
        .args(["--exact", "kill_9_while_recording_midi_keeps_everything_up_to_the_last_second", "--nocapture"])
        .env(CHILD, &folder)
        .spawn()
        .expect("child");
    // Let it play for a while, then SIGKILL (kill -9)
    let played = folder.join("played.txt");
    let start = Instant::now();
    loop {
        let n = std::fs::read_to_string(&played).map(|t| t.lines().count()).unwrap_or(0);
        if n >= 60 || start.elapsed() > Duration::from_secs(60) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().expect("kill -9");
    let _ = child.wait();
    let lines: Vec<(u64, usize)> = std::fs::read_to_string(&played)
        .expect("played")
        .lines()
        .filter_map(|l| {
            let mut f = l.split(' ');
            Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
        })
        .collect();
    let (last_ms, _) = *lines.last().expect("the child played");
    // Everything played more than a second (plus a pump) before the kill was on the disk
    let safe = lines.iter().filter(|(ms, _)| *ms + 1100 <= last_ms).map(|(_, n)| *n).max().unwrap_or(0);
    assert!(safe >= 10, "the child played too little before the kill: {lines:?}");
    let mut s = session(&dir);
    open(&mut s, folder);
    s.recover_takes().expect("recover");
    let start = Instant::now();
    while notes_in_midi_items(&s) < safe {
        assert!(start.elapsed() < Duration::from_secs(20), "recovered {} of {safe}", notes_in_midi_items(&s));
        s.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    // A recovered take, named so, and one undo removes it
    let recovered = notes_in_midi_items(&s);
    assert!(recovered >= safe, "{recovered} notes back, at least {safe} expected");
    let undo = s.document().undo_label().map(|l| l.key);
    assert_eq!(undo, Some("midi.take.recovered"));
    s.document_mut().undo().expect("undo");
    assert_eq!(notes_in_midi_items(&s), 0);
}
