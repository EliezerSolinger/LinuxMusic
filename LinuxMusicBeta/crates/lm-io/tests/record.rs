//! Recording (interfaces.md, I-11; U1, U3, U4, U5; C2, C10; H3; "pronto quando" of lm-io: an hour without loss, a
//! full ring is an explicit error). The whole device callback runs under the allocation trap (rule 5)

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use lm_base::{Cancel, Progress};
use lm_io::*;
use lm_render::*;
use lm_sys_sndfile::Reader;

// The trap exists in debug builds only (assert_no_alloc compiles it out of release)
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

fn plan(folder: &Path, rate: u32, inputs: &[&[u16]]) -> RecordPlan {
    let inputs = inputs
        .iter()
        .enumerate()
        .map(|(k, ch)| RecordInput {
            input: InputIndex(k as u32),
            channels: ch.to_vec(),
            tracks: vec![format!("t:{k}")],
        })
        .collect();
    let mut p = RecordPlan::new(folder.to_path_buf(), inputs, rate);
    p.latency = 123;
    p.pdc = 45;
    p
}

fn read_all(path: &Path) -> (Vec<f32>, usize) {
    let mut r = Reader::open(path).expect("open take");
    let ch = r.info().channels;
    let mut out = vec![0.0f32; r.info().frames as usize * ch];
    let mut at = 0;
    while at < out.len() {
        let n = r.read_f32(&mut out[at..]).expect("read");
        if n == 0 {
            break;
        }
        at += n * ch;
    }
    out.truncate(at);
    (out, ch)
}

/// Waits for `count` closed takes, collecting every event
fn takes(disk: &mut DiskWriter, count: usize, secs: u64) -> (Vec<Take>, Vec<DiskEvent>) {
    let mut takes = Vec::new();
    let mut others = Vec::new();
    wait_for(secs, "closed takes", || {
        disk.poll(|e| match e {
            DiskEvent::TakeClosed(t) => takes.push(t),
            other => others.push(other),
        });
        takes.len() >= count
    });
    (takes, others)
}

/// Checks that every frame of a ramp take came from consecutive device frames, except inside `gaps`, which must be
/// exact silence. Returns the device frame of file frame 0
fn check_ramp(samples: &[f32], channels: usize, gaps: &[TakeGap]) -> u64 {
    let first = ramp_frame(samples[0]);
    for (k, frame) in samples.chunks_exact(channels).enumerate() {
        let k = k as u64;
        if let Some(g) = gaps.iter().find(|g| k >= g.file_frame && k < g.file_frame + g.frames) {
            assert!(frame.iter().all(|x| *x == 0.0), "gap frame {k} is not silence");
            let _ = g;
            continue;
        }
        // The lost frames advanced the device clock exactly as much as the file: the grid holds (C2)
        let expected = (first + k) % (1 << 23);
        for x in frame {
            assert_eq!(ramp_frame(*x), expected, "file frame {k}");
        }
    }
    first
}

/// A take of a few seconds: the file, the record, the spans and the U5 data
#[test]
fn records_a_take() {
    let dir = TempDir::new("take");
    let options = DummyOptions { pace: Pace::Free { speed: Some(50.0) }, ..DummyOptions::default() };
    let backend = DummyBackend::new(options).with_source(Box::new(Ramp));
    let (mut device, mut engine, control, info) =
        dummy_device_with(backend, &DeviceConfig::duplex("dummy", 48_000, 256));
    let mut disk = DiskWriter::start().expect("disk");
    let (sender, receiver) = recording_channel(plan(&dir.0, 48_000, &[&[0, 1], &[1]])).expect("channel");
    disk.record(receiver).expect("record");
    device.arm(sender).expect("arm");
    wait_for(5, "the device took the recorder", || device.is_armed());
    engine.commands().send(Command::Record { on: true }).expect("record");
    engine.commands().send(Command::Play).expect("play");
    wait_for(20, "3 s recorded", || engine.frame().position > 3 * 48_000);
    engine.commands().send(Command::Stop).expect("stop");
    let (t, events) = takes(&mut disk, 2, 20);
    assert!(events.iter().any(|e| matches!(e, DiskEvent::TakeStarted { .. })));
    for take in &t {
        assert!(take.frames >= 3 * 48_000, "{} frames", take.frames);
        assert_eq!(take.start_sample, 0);
        assert_eq!(take.latency, 123);
        assert_eq!(take.pdc, 45);
        assert!(take.path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rec-")));
        let (samples, ch) = read_all(&take.path);
        assert_eq!(ch, take.channels);
        assert_eq!(samples.len() / ch, take.frames as usize);
        check_ramp(&samples, ch, &take.gaps);
        // Without a punch the whole take is recorded: one span from frame 0
        assert_eq!(take.spans.len(), 1);
        assert_eq!(take.spans[0].file_frame, 0);
        assert_eq!(take.spans[0].sample, 0);
        let (header, lines) = read_record(&take.record).expect("record");
        assert_eq!(header.start_sample, 0);
        assert_eq!(header.tracks, vec![format!("t:{}", take.input.0)]);
        assert!(lines.iter().any(|l| matches!(l, RecordLine::End { .. })));
    }
    let r = device.stop().expect("stop");
    drop(r);
    let _ = control;
    let _ = info;
}

/// The "pronto quando" of lm-io: one hour of recording, simulated fast with the Dummy and its fake clock, without
/// loss, with a capture xrun provoked in the middle that leaves the whole take on the grid (C2)
#[test]
fn one_hour_without_loss_and_an_xrun_on_the_grid() {
    let dir = TempDir::new("hour");
    let rate = 8_000;
    let hour = u64::from(rate) * 3600;
    let speed = 200.0;
    let options =
        DummyOptions { inputs: 1, outputs: 1, pace: Pace::Free { speed: Some(speed) }, ..DummyOptions::default() };
    let backend = DummyBackend::new(options).with_source(Box::new(Ramp));
    let mut cfg = DeviceConfig::duplex("dummy", rate, 256);
    cfg.playback_channels = Some(1);
    cfg.capture_channels = Some(1);
    let (mut device, mut engine, control, _info) = dummy_device_with(backend, &cfg);
    let mut disk = DiskWriter::start().expect("disk");
    // C4: the ring holds 10 s of the disk thread's (real) time; at `speed` times real time that is 10 s × speed of
    // simulated audio. The fsyncs every 2 s of audio come `speed` times as often as in a real session
    let mut p = plan(&dir.0, rate, &[&[0]]);
    p.seconds = (f64::from(DEFAULT_SECONDS) * speed) as f32;
    let (sender, receiver) = recording_channel(p).expect("channel");
    disk.record(receiver).expect("record");
    device.arm(sender).expect("arm");
    wait_for(5, "the device took the recorder", || device.is_armed());
    engine.commands().send(Command::Record { on: true }).expect("record");
    engine.commands().send(Command::Play).expect("play");
    let mut stopped = Vec::new();
    let mut xrun_sent = false;
    let lost = 1_234u32;
    wait_for(120, "an hour", || {
        disk.poll(|e| stopped.push(e));
        live_drain(&mut disk);
        let p = engine.frame().position;
        if !xrun_sent && p as u64 > hour / 2 {
            control.inject_capture_lost(lost);
            xrun_sent = true;
        }
        p as u64 >= hour
    });
    engine.commands().send(Command::Stop).expect("stop");
    let (t, events) = takes(&mut disk, 1, 30);
    stopped.extend(events);
    assert!(
        !stopped.iter().any(|e| matches!(e, DiskEvent::InputStopped { .. } | DiskEvent::Failed { .. })),
        "loss: {stopped:?}"
    );
    let take = &t[0];
    assert!(take.frames as u64 >= hour, "{} frames", take.frames);
    assert_eq!(take.gaps.len(), 1, "one gap");
    assert_eq!(take.gaps[0].frames, u64::from(lost));
    let (samples, ch) = read_all(&take.path);
    assert_eq!(samples.len() as i64, take.frames);
    // The take is on the grid: file frame k is device frame first + k, the lost frames silent where reported
    check_ramp(&samples, ch, &take.gaps);
    // And the timeline: the playhead advanced by the same lost frames, so one span covers everything
    assert_eq!(take.spans.len(), 1, "spans {:?}", take.spans);
    assert_eq!(take.spans[0].frames as i64, take.frames);
    assert!(take.cuts.is_empty());
    drop(samples);
    device.stop().expect("stop");
}

fn live_drain(disk: &mut DiskWriter) {
    disk.live_peaks(|_| {});
}

/// I-11: a full ring is an explicit error. The disk is held back while the device records; the input stops at the
/// last block that fit, the file ends there, and the exact time is reported
#[test]
fn full_ring_is_an_explicit_error() {
    let dir = TempDir::new("full");
    let rate = 48_000;
    let options = DummyOptions { pace: Pace::Free { speed: Some(20.0) }, ..DummyOptions::default() };
    let backend = DummyBackend::new(options).with_source(Box::new(Ramp));
    let (mut device, mut engine, _control, _) = dummy_device_with(backend, &DeviceConfig::duplex("dummy", rate, 256));
    let mut p = plan(&dir.0, rate, &[&[0]]);
    p.seconds = 0.5;
    let (sender, receiver) = recording_channel(p).expect("channel");
    device.arm(sender).expect("arm");
    wait_for(5, "the device took the recorder", || device.is_armed());
    engine.commands().send(Command::Record { on: true }).expect("record");
    engine.commands().send(Command::Play).expect("play");
    wait_for(20, "two seconds", || engine.frame().position > 2 * i64::from(rate));
    engine.commands().send(Command::Stop).expect("stop");
    std::thread::sleep(Duration::from_millis(50));
    // Only now the disk starts reading
    let mut disk = DiskWriter::start().expect("disk");
    disk.record(receiver).expect("record");
    let (t, events) = takes(&mut disk, 1, 20);
    let stop = events
        .iter()
        .find_map(|e| match e {
            DiskEvent::InputStopped { reason, file_frame, seconds, .. } => Some((*reason, *file_frame, *seconds)),
            _ => None,
        })
        .expect("an explicit stop");
    assert_eq!(stop.0, StopReason::Overflow);
    let ring = ring_frames(rate, 0.5) as u64;
    assert!(stop.1 <= ring && stop.1 + 256 > ring, "stopped at {} of a ring of {ring}", stop.1);
    assert!((stop.2 - stop.1 as f64 / f64::from(rate)).abs() < 1e-9);
    let take = &t[0];
    assert_eq!(take.stopped.map(|s| s.file_frame), Some(stop.1));
    let (samples, ch) = read_all(&take.path);
    assert_eq!(samples.len() as u64, stop.1, "the file ends at the last good block");
    check_ramp(&samples, ch, &[]);
    device.stop().expect("stop");
}

/// U1: recording in a loop keeps one file with the cuts; U4: a punch keeps the handles, and the spans say where
/// the take proper is
#[test]
fn loop_cuts_and_punch_handles() {
    let dir = TempDir::new("loop");
    let rate = 48_000;
    let options = DummyOptions { pace: Pace::Free { speed: Some(50.0) }, ..DummyOptions::default() };
    let backend = DummyBackend::new(options).with_source(Box::new(Ramp));
    let (mut device, mut engine, control, info) = dummy_device_with(backend, &DeviceConfig::duplex("dummy", rate, 256));
    let spec = info.spec();
    let mut b = ProgramBuilder::new(ProgramGen(1), spec, &limits(), tempo(rate));
    b.node(NodeDesc::sum(n(0), 2));
    b.master(n(0));
    b.transport(TransportSettings {
        loop_range: Some((10_000, 30_000)),
        punch: Some((15_000, 25_000)),
        ..Default::default()
    });
    engine.install(Install::new(b.build().expect("program"))).expect("install");
    let mut disk = DiskWriter::start().expect("disk");
    let (sender, receiver) = recording_channel(plan(&dir.0, rate, &[&[0]])).expect("channel");
    disk.record(receiver).expect("record");
    // Wait for the program to be in
    wait_for(5, "program", || engine.frame().generation == ProgramGen(1));
    engine.commands().send(Command::Locate { sample: 5_000 }).expect("locate");
    device.arm(sender).expect("arm");
    wait_for(5, "the device took the recorder", || device.is_armed());
    engine.commands().send(Command::Record { on: true }).expect("record");
    engine.commands().send(Command::Play).expect("play");
    // Three passes of the loop (5 000 to 30 000, then 10 000 to 30 000 twice more)
    let from = control.frames();
    wait_for(20, "three passes", || control.frames() > from + 70_000);
    engine.commands().send(Command::Stop).expect("stop");
    let (t, _) = takes(&mut disk, 1, 20);
    let take = &t[0];
    // The file starts at the play position (pre-roll handle, U4)
    assert_eq!(take.start_sample, 5_000);
    assert!(take.spans.len() >= 3, "spans {:?}", take.spans);
    for s in &take.spans {
        assert!(s.sample >= 15_000 && s.sample + s.frames as i64 <= 25_000, "span outside the punch: {s:?}");
    }
    assert_eq!(take.spans[0].file_frame, 10_000, "15 000 is 10 000 frames into a file that starts at 5 000");
    // Each pass of the loop is a cut from the punch-out... the loop end back to the start
    assert!(!take.cuts.is_empty());
    for c in &take.cuts {
        assert_eq!(c.to_sample, 15_000);
    }
    // The handles: frames before and after the punch are in the file
    assert!(take.frames as u64 > take.spans.iter().map(|s| s.frames).sum::<u64>());
    device.stop().expect("stop");
}

/// C10, P-12: the finalisation names the file by its BLAKE3 hash, writes its peaks and renames the record;
/// converting to FLAC 24 keeps the signal
#[test]
fn finalize_names_by_hash_and_writes_peaks() {
    let dir = TempDir::new("final");
    let rate = 48_000;
    let options = DummyOptions { pace: Pace::Free { speed: Some(50.0) }, ..DummyOptions::default() };
    let backend = DummyBackend::new(options).with_source(Box::new(Ramp));
    let (mut device, mut engine, _control, _) = dummy_device_with(backend, &DeviceConfig::duplex("dummy", rate, 256));
    let mut disk = DiskWriter::start().expect("disk");
    let (sender, receiver) = recording_channel(plan(&dir.0, rate, &[&[0], &[1]])).expect("channel");
    disk.record(receiver).expect("record");
    device.arm(sender).expect("arm");
    wait_for(5, "the device took the recorder", || device.is_armed());
    engine.commands().send(Command::Record { on: true }).expect("record");
    engine.commands().send(Command::Play).expect("play");
    let mut live = 0usize;
    wait_for(20, "a second", || {
        disk.live_peaks(|_| live += 1);
        engine.frame().position > i64::from(rate)
    });
    engine.commands().send(Command::Stop).expect("stop");
    let (t, _) = takes(&mut disk, 2, 20);
    assert!(t.iter().all(|t| t.frames >= i64::from(rate)));
    disk.live_peaks(|_| live += 1);
    assert!(
        live > 2 * 700,
        "U3: the growing waveform ({live} live pairs, {} dropped, frames {} {})",
        disk.live_dropped(),
        t[0].frames,
        t[1].frames
    );
    let cancel = Cancel::new();
    let progress = Progress::new();
    let a = finalize_take(&t[0], TakeFormat::Float32, &cancel, &progress).expect("finalize");
    assert_eq!(a.key, hash_file(&a.take.path, &cancel, &progress).expect("hash"));
    assert_eq!(a.take.path, dir.0.join(format!("{}.wav", a.key)));
    assert!(!t[0].path.exists(), "the rec- name is gone");
    assert_eq!(Peaks::read(&a.peaks_path).expect("peaks"), *a.peaks);
    assert_eq!(a.peaks.frames() as i64, t[0].frames);
    let record_name = a.take.record.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
    assert!(record_name.starts_with(&a.key) && record_name.ends_with(".recording.json"), "{record_name}");
    let (_, lines) = read_record(&a.take.record).expect("record");
    assert!(lines.iter().any(|l| matches!(l, RecordLine::Final { .. })));
    forget_record(&a.take.record).expect("forget");
    let b = finalize_take(&t[1], TakeFormat::Flac24, &cancel, &progress).expect("flac");
    assert!(b.take.path.extension().is_some_and(|e| e == "flac"));
    let (flac, _) = read_all(&b.take.path);
    assert_eq!(flac.len() as i64, t[1].frames);
    // 24 bits keep the ramp within one step of 2^-23
    let first = ramp_frame(flac[0]);
    for (k, x) in flac.iter().enumerate() {
        assert!((ramp_frame(*x) as i64 - ((first + k as u64) % (1 << 23)) as i64).abs() <= 1);
    }
    device.stop().expect("stop");
}

/// U5, C10: a crash leaves a file readable up to the last sync and a record that puts the take back. The "crash"
/// is a copy of the files while the recording runs (what the disk holds at that moment)
#[test]
fn interrupted_take_is_recovered() {
    let dir = TempDir::new("crash");
    let copy = TempDir::new("crash-copy");
    let rate = 48_000;
    let options = DummyOptions { pace: Pace::Free { speed: Some(4.0) }, ..DummyOptions::default() };
    let backend = DummyBackend::new(options).with_source(Box::new(Ramp));
    let (mut device, mut engine, _control, _) = dummy_device_with(backend, &DeviceConfig::duplex("dummy", rate, 256));
    let mut disk = DiskWriter::start().expect("disk");
    let (sender, receiver) = recording_channel(plan(&dir.0, rate, &[&[0, 1]])).expect("channel");
    disk.record(receiver).expect("record");
    device.arm(sender).expect("arm");
    wait_for(5, "the device took the recorder", || device.is_armed());
    engine.commands().send(Command::Record { on: true }).expect("record");
    engine.commands().send(Command::Play).expect("play");
    let mut started: Option<(PathBuf, PathBuf)> = None;
    wait_for(20, "the take started", || {
        disk.poll(|e| {
            if let DiskEvent::TakeStarted { path, record, .. } = e {
                started = Some((path, record));
            }
        });
        started.is_some()
    });
    wait_for(20, "5 s", || engine.frame().position > 5 * i64::from(rate));
    let (path, record) = started.expect("started");
    let crashed_audio = copy.0.join(path.file_name().expect("name"));
    let crashed_record = copy.0.join(record.file_name().expect("name"));
    std::fs::copy(&record, &crashed_record).expect("copy record");
    std::fs::copy(&path, &crashed_audio).expect("copy audio");
    engine.commands().send(Command::Stop).expect("stop");
    let _ = takes(&mut disk, 1, 20);
    let found = find_interrupted(&copy.0).expect("scan");
    assert_eq!(found, vec![crashed_record.clone()]);
    let take = recover_take(&crashed_record, &Cancel::new(), &Progress::new()).expect("recover");
    // At least the last sync (every 2 s) before 5 s
    assert!(take.frames >= 4 * i64::from(rate) - 256, "{} frames", take.frames);
    assert_eq!(take.start_sample, 0);
    // U5: the span is in the record in pieces of a second, so the crash keeps where the take proper is
    assert_eq!(take.spans.len(), 1, "{:?}", take.spans);
    assert_eq!((take.spans[0].file_frame, take.spans[0].sample), (0, 0));
    assert!(take.spans[0].frames >= 3 * u64::from(rate), "{:?}", take.spans);
    let (samples, ch) = read_all(&take.path);
    check_ramp(&samples, ch, &[]);
    assert!(find_interrupted(&copy.0).expect("scan").is_empty(), "recovered takes are not offered again");
    device.stop().expect("stop");
}

/// H3: room for 10 minutes warns; under 30 s, arming is refused
#[test]
fn disk_space_rules() {
    let bps = record_bytes_per_second(2, 48_000);
    assert!(matches!(space_for(bps * 3600, 2, 48_000), SpaceCheck::Enough { .. }));
    assert!(matches!(space_for(bps * 300, 2, 48_000), SpaceCheck::Low { seconds } if (seconds - 300.0).abs() < 1e-9));
    assert!(matches!(space_for(bps * 29, 2, 48_000), SpaceCheck::Refused { .. }));
    let here = check_record_space(&std::env::temp_dir(), 2, 48_000).expect("statvfs");
    assert!(!matches!(here, SpaceCheck::Refused { seconds } if seconds < 0.0));
}

/// Two takes of identical content (two silent inputs) finalised at the same time: one audio file and one peaks file
/// for both, and each keeps its own record
#[test]
fn identical_takes_finalise_concurrently() {
    let dir = TempDir::new("twins");
    let rate = 48_000;
    let options = DummyOptions { pace: Pace::Free { speed: Some(50.0) }, ..DummyOptions::default() };
    let (mut device, mut engine, _control, _) =
        dummy_device_with(DummyBackend::new(options), &DeviceConfig::duplex("dummy", rate, 256));
    let mut disk = DiskWriter::start().expect("disk");
    let (sender, receiver) = recording_channel(plan(&dir.0, rate, &[&[0], &[1]])).expect("channel");
    disk.record(receiver).expect("record");
    device.arm(sender).expect("arm");
    wait_for(5, "armed", || device.is_armed());
    engine.commands().send(Command::Record { on: true }).expect("record");
    engine.commands().send(Command::Play).expect("play");
    wait_for(20, "half a second", || engine.frame().position > i64::from(rate) / 2);
    engine.commands().send(Command::Stop).expect("stop");
    let (t, _) = takes(&mut disk, 2, 20);
    let handles: Vec<_> = t
        .into_iter()
        .map(|take| {
            std::thread::spawn(move || finalize_take(&take, TakeFormat::Float32, &Cancel::new(), &Progress::new()))
        })
        .collect();
    let done: Vec<FinalizedTake> = handles.into_iter().map(|h| h.join().expect("thread").expect("finalize")).collect();
    assert_eq!(done[0].key, done[1].key, "identical content, one key");
    assert_ne!(done[0].take.record, done[1].take.record, "each take keeps its own record");
    for d in &done {
        let (header, _) = read_record(&d.take.record).expect("record");
        assert_eq!(header.tracks, vec![format!("t:{}", d.take.input.0)]);
        assert!(d.take.path.exists() && d.peaks_path.exists());
    }
    forget_record(&done[0].take.record).expect("forget");
    assert!(done[1].take.record.exists(), "forgetting one does not touch the other");
    device.stop().expect("stop");
}
