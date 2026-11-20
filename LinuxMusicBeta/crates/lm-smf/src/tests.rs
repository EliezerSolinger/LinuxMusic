use super::*;

fn t(tick: u64, event: Event) -> Timed {
    Timed { tick, event }
}

fn song() -> Smf {
    Smf {
        format: 1,
        ppq: 960,
        tracks: vec![
            Track {
                events: vec![
                    t(0, Event::Tempo(500_000)),
                    t(0, Event::TimeSignature(3, 4)),
                    t(3840, Event::Tempo(400_000)),
                ],
            },
            Track {
                events: vec![
                    t(0, Event::TrackName("Piano".into())),
                    t(0, Event::Program { channel: 0, program: 4 }),
                    t(0, Event::Controller { channel: 0, number: 7, value: 100 }),
                    t(0, Event::NoteOn { channel: 0, key: 60, velocity: 90 }),
                    t(480, Event::NoteOff { channel: 0, key: 60, velocity: 64 }),
                    t(480, Event::NoteOn { channel: 0, key: 60, velocity: 80 }),
                    t(960, Event::PitchBend { channel: 0, value: 12_000 }),
                    t(1000, Event::NoteOff { channel: 0, key: 60, velocity: 64 }),
                ],
            },
        ],
    }
}

#[test]
fn a_round_trip_keeps_every_event() {
    let s = song();
    let back = read(&write(&s)).unwrap();
    assert_eq!(back, s);
}

#[test]
fn a_hand_written_format_0_file_with_running_status() {
    // MThd, format 0, 1 track, 96 ppq; a track: note on 60, then (running status) note on 64 with velocity 0
    let mut f = b"MThd\0\0\0\x06\0\0\0\x01\0\x60".to_vec();
    let body: &[u8] = &[0x00, 0x90, 60, 100, 0x60, 60, 0x00, 0x00, 0xff, 0x2f, 0x00];
    f.extend_from_slice(b"MTrk");
    f.extend_from_slice(&(body.len() as u32).to_be_bytes());
    f.extend_from_slice(body);
    let s = read(&f).unwrap();
    assert_eq!((s.format, s.ppq), (0, 96));
    assert_eq!(
        s.tracks[0].events,
        vec![
            t(0, Event::NoteOn { channel: 0, key: 60, velocity: 100 }),
            t(96, Event::NoteOff { channel: 0, key: 60, velocity: 64 }),
        ]
    );
}

#[test]
fn rmid_wraps_an_smf() {
    let smf = write(&song());
    let mut f = b"RIFF".to_vec();
    f.extend_from_slice(&((smf.len() + 12) as u32).to_le_bytes());
    f.extend_from_slice(b"RMIDdata");
    f.extend_from_slice(&(smf.len() as u32).to_le_bytes());
    f.extend_from_slice(&smf);
    assert_eq!(read(&f).unwrap(), song());
}

#[test]
fn broken_files_are_refused_with_keys() {
    assert_eq!(read(b"hello").unwrap_err().key, "smf.not_midi");
    let mut f = write(&song());
    f.truncate(f.len() - 5);
    assert_eq!(read(&f).unwrap_err().key, "smf.truncated");
    let smpte = b"MThd\0\0\0\x06\0\0\0\x01\xe7\x28";
    assert_eq!(read(smpte).unwrap_err().key, "smf.smpte_time");
}

#[test]
fn variable_length_quantities() {
    for v in [0u32, 0x40, 0x7f, 0x80, 0x2000, 0x3fff, 0x4000, 0x0fff_ffff] {
        let mut b = Vec::new();
        put_vlq(&mut b, v);
        let mut r = Reader { data: &b, at: 0 };
        assert_eq!(r.vlq().unwrap(), v);
    }
}
