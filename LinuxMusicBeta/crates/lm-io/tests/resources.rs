//! Resources (interfaces.md, I-12; C8, C9, C10; H3, H17; U14; I-2: loading with an injected delay shows the loading
//! state and never a half-loaded resource)

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use lm_base::{Cancel, HostTime, ManualClock, Progress, Role, Workers};
use lm_io::*;
use lm_sys_sndfile::{Existing, Sample, WriteFormat, WriteSpec, Writer};

fn write_wav(path: &Path, rate: u32, channels: usize, frames: usize) {
    let mut w = Writer::create(path, WriteSpec { format: WriteFormat::Wav(Sample::Float32), rate, channels }, Existing::Replace)
        .expect("create");
    let samples: Vec<f32> = (0..frames * channels).map(|i| ((i as f32) * 0.001).sin() * 0.5).collect();
    w.write_f32(&samples).expect("write");
    w.close().expect("close");
}

fn ops() -> (Cancel, Progress) {
    (Cancel::new(), Progress::new())
}

#[test]
fn import_copies_hashes_and_writes_peaks() {
    let dir = TempDir::new("import");
    let src = dir.0.join("in.wav");
    write_wav(&src, 48_000, 2, 30_000);
    let project = dir.0.join("project");
    let (c, p) = ops();
    let a = import_file(&src, &project, 48_000, &c, &p).expect("import");
    assert!(!a.converted);
    assert_eq!(a.key, hash_file(&src, &c, &p).expect("hash"), "same rate: the bytes are copied");
    assert_eq!(a.path, project.join("audio").join(format!("{}.wav", a.key)));
    assert_eq!(Peaks::read(&a.peaks_path).expect("peaks").frames(), 30_000);
    // Twice: the same file is reused
    let b = import_file(&src, &project, 48_000, &c, &p).expect("again");
    assert_eq!(a.key, b.key);
    assert_eq!(std::fs::read_dir(project.join("audio")).expect("dir").count(), 2, "one audio file and its peaks");
    assert_eq!(resource_path(&project, &a.key).expect("path"), Some(a.path.clone()));
}

#[test]
fn import_converts_to_the_project_rate() {
    let dir = TempDir::new("convert");
    let src = dir.0.join("in.wav");
    write_wav(&src, 44_100, 1, 44_100);
    let project = dir.0.join("project");
    let (c, p) = ops();
    let a = import_file(&src, &project, 48_000, &c, &p).expect("import");
    assert!(a.converted);
    assert_eq!(a.frames, 48_000);
    let data = load_file(&a.path, &c, &p).expect("load");
    assert_eq!(data.rate(), 48_000);
    assert_eq!(data.frames(), 48_000);
    assert_eq!(a.key, hash_file(&a.path, &c, &p).expect("hash"));
}

/// U14: a cancelled import leaves no file behind; leftovers of a crash are cleaned when the project opens
#[test]
fn cancelled_import_leaves_nothing() {
    let dir = TempDir::new("cancel");
    let src = dir.0.join("in.wav");
    write_wav(&src, 44_100, 2, 200_000);
    let project = dir.0.join("project");
    let (c, p) = ops();
    c.cancel();
    assert!(import_file(&src, &project, 48_000, &c, &p).expect_err("cancelled").is_cancelled());
    let left: Vec<_> = std::fs::read_dir(project.join("audio")).expect("dir").collect();
    assert!(left.is_empty(), "{left:?}");
    std::fs::write(project.join("audio").join(".import-x.tmp"), b"half").expect("leftover");
    assert_eq!(clean_import_leftovers(&project).expect("clean"), 1);
}

/// The 200 ms rule (I-2) with an injected delay: the resource is Loading, absent from the view, until it is whole
#[test]
fn delayed_load_shows_loading_and_never_half_loaded() {
    let dir = TempDir::new("loading");
    let src = dir.0.join("in.wav");
    write_wav(&src, 48_000, 2, 48_000);
    let project = dir.0.join("project");
    let (c, p) = ops();
    let key = import_file(&src, &project, 48_000, &c, &p).expect("import").key;
    let clock = ManualClock::new(HostTime(1_000_000_000));
    let mut store = ResourceStore::new(project.clone(), Arc::new(clock.clone()));
    store.faults = IoFaults { load_delay: Duration::from_millis(400) };
    let job = store.begin_load(&key).expect("job");
    assert!(matches!(store.state(&key), ResourceState::Loading { since } if since == HostTime(1_000_000_000)));
    assert!(matches!(store.begin_load(&key), Err(ResourceState::Loading { .. })), "asked twice, loaded once");
    let workers = Workers::new("test-load", 1, Role::Worker);
    let mut handle = workers.start(move |c, p| job.run(c, p));
    let mut polled = 0;
    let result = loop {
        // While it runs: loading, and nothing of it in the view
        assert!(store.view().get(&key).is_none());
        assert!(store.peaks(&key).is_none());
        if let Some(r) = handle.poll() {
            break r;
        }
        clock.advance_millis(10);
        polled += 1;
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(polled >= 30, "the delay was injected ({polled} polls)");
    // Past 200 ms of the session's clock the interface draws the loading: the state says since when
    if let ResourceState::Loading { since } = store.state(&key) {
        assert!(clock.now_ns() - since.0 >= 200_000_000);
    } else {
        panic!("not loading");
    }
    store.finish(&key, result);
    assert_eq!(store.state(&key), ResourceState::Ready);
    let data = store.view().get(&key).cloned().expect("in the view");
    assert_eq!(data.frames(), 48_000, "whole");
    assert_eq!(store.peaks(&key).expect("peaks").frames(), 48_000);
}

trait NowNs {
    fn now_ns(&self) -> u64;
}

impl NowNs for ManualClock {
    fn now_ns(&self) -> u64 {
        lm_base::Clock::now(self).0
    }
}

/// H17, C9: a missing file is Missing; a bad key is refused; a link out of the project is not followed
#[test]
fn missing_bad_keys_and_links_out() {
    let dir = TempDir::new("missing");
    let project = dir.0.join("project");
    std::fs::create_dir_all(project.join("audio")).expect("audio");
    let mut store = ResourceStore::new(project.clone(), Arc::new(ManualClock::new(HostTime(0))));
    let key = "0".repeat(64);
    assert!(matches!(store.begin_load(&key), Err(ResourceState::Missing)));
    assert_eq!(store.state(&key), ResourceState::Missing);
    assert!(matches!(store.begin_load("../../etc/passwd"), Err(ResourceState::Failed(e)) if e.key() == "io.resource.invalid_key"));
    let outside = dir.0.join("secret.wav");
    write_wav(&outside, 48_000, 1, 100);
    let k2 = "1".repeat(64);
    std::os::unix::fs::symlink(&outside, project.join("audio").join(format!("{k2}.wav"))).expect("link");
    assert!(matches!(store.begin_load(&k2), Err(ResourceState::Failed(e)) if e.key() == "io.resource.outside_project"));
    // External files only inside allowed folders
    assert_eq!(resolve_external(&outside, &[project.clone()]).expect_err("not allowed").key(), "io.resource.not_allowed");
    assert!(resolve_external(&outside, &[dir.0.clone()]).is_ok());
    // A located file is the resource only if its hash is the key
    let (c, p) = ops();
    assert!(!verify_located(&outside, &key, &c, &p).expect("hash"));
}

/// C8: unused for 5 minutes and in no program: evicted; asked again, loaded again
#[test]
fn eviction_after_five_minutes() {
    let dir = TempDir::new("evict");
    let src = dir.0.join("in.wav");
    write_wav(&src, 48_000, 1, 1000);
    let project = dir.0.join("project");
    let (c, p) = ops();
    let key = import_file(&src, &project, 48_000, &c, &p).expect("import").key;
    let clock = ManualClock::new(HostTime(0));
    let mut store = ResourceStore::new(project, Arc::new(clock.clone()));
    let job = store.begin_load(&key).expect("job");
    store.finish(&key, job.run(&c, &p));
    clock.advance_millis(299_000);
    assert!(store.evict(&|_| false).is_empty(), "not yet");
    clock.advance_millis(2_000);
    assert!(store.evict(&|_| true).is_empty(), "a live program uses it");
    let gone = store.evict(&|_| false);
    assert_eq!(gone.len(), 1);
    assert!(store.view().get(&key).is_none());
    assert_eq!(store.state(&key), ResourceState::Absent);
    assert!(store.begin_load(&key).is_ok(), "loads again");
}

/// P-32: a long resource is not decoded into memory; it plays from disk
#[test]
fn long_resources_play_from_disk() {
    let dir = TempDir::new("long");
    let src = dir.0.join("in.wav");
    write_wav(&src, 8_000, 1, 8_000 * 3);
    let project = dir.0.join("project");
    let (c, p) = ops();
    let key = import_file(&src, &project, 8_000, &c, &p).expect("import").key;
    let mut store = ResourceStore::new(project, Arc::new(ManualClock::new(HostTime(0))));
    store.stream_above = Duration::from_secs(2);
    let job = store.begin_load(&key).expect("job");
    store.finish(&key, job.run(&c, &p));
    assert_eq!(store.state(&key), ResourceState::OnDisk);
    let view = store.view();
    assert!(view.get(&key).is_none());
    assert_eq!(view.on_disk(&key).map(|o| o.frames), Some(24_000));
    assert!(view.is_ready(&key));
}

/// P-32: the disk streamer fills a stream from the file, nearest the playhead first, and drops it when the program
/// side is gone
#[test]
fn disk_reader_streams_a_file() {
    let dir = TempDir::new("stream");
    let path = dir.0.join("long.wav");
    let frames = 48_000 * 10;
    write_wav(&path, 48_000, 2, frames);
    let (stream, feeder) = lm_render::stream(lm_render::StreamSpec::new(48_000, 2, frames as u64)).expect("stream");
    stream.set_playhead(Some(100_000));
    let mut reader = DiskReader::start().expect("reader");
    reader.serve(path.clone(), feeder).expect("serve");
    wait_for(10, "the window around the playhead", || !stream.pending());
    let mut out = vec![0.0f32; 2 * 1000];
    assert_eq!(stream.read(100_000, 1000, &mut out), 0, "nothing missing");
    let (c, p) = ops();
    let data = load_file(&path, &c, &p).expect("load");
    assert_eq!(&out[..1000], &data.channel(0)[100_000..101_000]);
    assert_eq!(&out[1000..], &data.channel(1)[100_000..101_000]);
    // A wrong rate is refused, not played out of tune
    let (s2, f2) = lm_render::stream(lm_render::StreamSpec::new(44_100, 2, frames as u64)).expect("stream");
    s2.set_playhead(Some(0));
    reader.serve(path, f2).expect("serve");
    let mut failed = None;
    wait_for(5, "the refusal", || {
        reader.poll(|e| {
            let StreamEvent::Failed { error, .. } = e;
            failed = Some(error.key());
        });
        failed.is_some()
    });
    assert_eq!(failed, Some("io.stream.rate_mismatch"));
    drop(stream);
}
