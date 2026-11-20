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
    let mut w = Writer::create(
        path,
        WriteSpec { format: WriteFormat::Wav(Sample::Float32), rate, channels },
        Existing::Replace,
    )
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
    // The load is held until the test lets it go: what happens in between depends on the injected clock only
    let hold = Arc::new(std::sync::atomic::AtomicBool::new(true));
    store.faults = IoFaults { load_delay: Duration::ZERO, hold: Some(hold.clone()) };
    let job = store.begin_load(&key).expect("job");
    assert!(matches!(store.state(&key), ResourceState::Loading { since } if since == HostTime(1_000_000_000)));
    assert!(matches!(store.begin_load(&key), Err(ResourceState::Loading { .. })), "asked twice, loaded once");
    let workers = Workers::new("test-load", 1, Role::Worker);
    let mut handle = workers.start(move |c, p| job.run(c, p));
    // 300 ms of the session's clock while the load is held: always loading, nothing of it in the view
    for _ in 0..30 {
        clock.advance_millis(10);
        assert!(handle.poll().is_none(), "the held load cannot finish");
        assert!(store.view().get(&key).is_none());
        assert!(store.peaks(&key).is_none());
        assert!(matches!(store.state(&key), ResourceState::Loading { .. }));
    }
    // Past 200 ms the interface draws the loading: the state says since when
    match store.state(&key) {
        ResourceState::Loading { since } => assert_eq!(clock.now_ns() - since.0, 300_000_000),
        other => panic!("not loading: {other:?}"),
    }
    hold.store(false, std::sync::atomic::Ordering::Release);
    let result = loop {
        assert!(store.view().get(&key).is_none(), "nothing half loaded while it finishes");
        if let Some(r) = handle.poll() {
            break r;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
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
    assert!(
        matches!(store.begin_load("../../etc/passwd"), Err(ResourceState::Failed(e)) if e.key() == "io.resource.invalid_key")
    );
    let outside = dir.0.join("secret.wav");
    write_wav(&outside, 48_000, 1, 100);
    let k2 = "1".repeat(64);
    std::os::unix::fs::symlink(&outside, project.join("audio").join(format!("{k2}.wav"))).expect("link");
    assert!(matches!(store.begin_load(&k2), Err(ResourceState::Failed(e)) if e.key() == "io.resource.outside_project"));
    // External files only inside allowed folders
    assert_eq!(
        resolve_external(&outside, std::slice::from_ref(&project)).expect_err("not allowed").key(),
        "io.resource.not_allowed"
    );
    assert!(resolve_external(&outside, std::slice::from_ref(&dir.0)).is_ok());
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

/// Resources of other kinds (U24): a NAM model is imported into `state/` by hash and loaded as bytes; a SoundFont
/// stays a checked file; kinds and keys are validated (C9)
#[test]
fn resources_by_kind() {
    let dir = TempDir::new("kinds");
    let project = dir.0.join("project");
    let model = dir.0.join("amp.nam");
    std::fs::write(&model, b"{\"version\": \"0.5\", \"weights\": [1, 2, 3]}").expect("model");
    let font = dir.0.join("piano.SF2");
    std::fs::write(&font, vec![7u8; 4096]).expect("font");
    let (c, p) = ops();
    let a = import_file_as(&model, &project, "nam", &c, &p).expect("nam");
    assert_eq!(a.path, project.join("state").join(format!("{}.nam", a.key)));
    assert_eq!(a.key, hash_file(&model, &c, &p).expect("hash"));
    let b = import_file_as(&font, &project, "sf2", &c, &p).expect("sf2");
    assert!(b.path.to_str().is_some_and(|s| s.ends_with(".sf2")), "{:?}", b.path);
    assert_eq!(import_file_as(&model, &project, "nam", &c, &p).expect("again").path, a.path, "reused");
    assert_eq!(import_file_as(&model, &project, "../x", &c, &p).expect_err("kind").key(), "io.resource.invalid_kind");
    assert_eq!(kind_policy("nam"), KindPolicy::Bytes);
    assert_eq!(kind_policy("sf2"), KindPolicy::File);
    assert_eq!(kind_policy("pat"), KindPolicy::Bytes);
    assert_eq!(kind_policy("render"), KindPolicy::Audio);

    let clock = ManualClock::new(HostTime(0));
    let mut store = ResourceStore::new(project.clone(), Arc::new(clock));
    let job = store.begin_load_kind("nam", &a.key).expect("job");
    assert!(matches!(store.state_kind("nam", &a.key), ResourceState::Loading { .. }));
    assert!(store.view().content("nam", &a.key).is_none(), "nothing half loaded");
    store.finish_kind("nam", &a.key, job.run(&c, &p));
    assert_eq!(store.state_kind("nam", &a.key), ResourceState::Ready);
    assert_eq!(
        store.view().content("nam", &a.key),
        Some(&Content::Bytes(Arc::from(std::fs::read(&model).expect("bytes").into_boxed_slice())))
    );
    let job = store.begin_load_kind("sf2", &b.key).expect("job");
    store.finish_kind("sf2", &b.key, job.run(&c, &p));
    assert_eq!(store.view().content("sf2", &b.key), Some(&Content::File(b.path.clone())));
    // Missing and invalid
    assert!(matches!(store.begin_load_kind("nam", &"0".repeat(64)), Err(ResourceState::Missing)));
    assert!(
        matches!(store.begin_load_kind("nam", "../../etc"), Err(ResourceState::Failed(e)) if e.key() == "io.resource.invalid_key")
    );
    // A link out of state/ is refused
    let k = "2".repeat(64);
    std::os::unix::fs::symlink(&model, project.join("state").join(format!("{k}.nam"))).expect("link");
    assert!(
        matches!(store.begin_load_kind("nam", &k), Err(ResourceState::Failed(e)) if e.key() == "io.resource.outside_project")
    );
}

/// F-REC-090: a tempo render lives in `render/<plan key>.wav`, apart from the content-hashed `audio/`, and loads like
/// audio (decoded, with peaks), into its own place in the view
#[test]
fn renders_load_like_audio_from_their_folder() {
    let dir = TempDir::new("render");
    let project = dir.0.join("project");
    std::fs::create_dir_all(render_folder(&project)).expect("render folder");
    let key = "3".repeat(64);
    write_wav(&render_folder(&project).join(format!("{key}.wav")), 48_000, 2, 9_000);
    std::fs::write(render_folder(&project).join(format!("{key}.x.tmp")), b"half").expect("leftover");
    let (c, p) = ops();
    let mut store = ResourceStore::new(project.clone(), Arc::new(ManualClock::new(HostTime(0))));
    assert!(
        matches!(store.begin_load_kind("render", &key), Err(ResourceState::Failed(e)) if e.key() == "io.resource.use_begin_load")
    );
    let job = store.begin_load_render(&key).expect("job");
    assert!(matches!(store.state_kind("render", &key), ResourceState::Loading { .. }));
    store.finish_render(&key, job.run(&c, &p));
    assert_eq!(store.state_kind("render", &key), ResourceState::Ready);
    let view = store.view();
    assert_eq!(view.render(&key).map(|d| d.frames()), Some(9_000));
    assert!(view.get(&key).is_none(), "not an audio resource");
    assert_eq!(store.render_peaks(&key).map(|p| p.frames()), Some(9_000));
    assert!(matches!(store.begin_load_render(&"4".repeat(64)), Err(ResourceState::Missing)));
    assert_eq!(clean_import_leftovers(&project).expect("clean"), 1);
}
