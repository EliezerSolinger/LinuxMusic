//! PCM tests on Linux: the ALSA `null` PCM always (it is part of alsa-lib's configuration), and the `snd-aloop`
//! loopback card when the runner has it (`LM_TEST_ALSA_LOOPBACK`, set by ci/setup-alsa.sh). When the CI says a device
//! exists (`LM_TEST_ALSA_NULL`, `LM_TEST_ALSA_LOOPBACK`), failing to open it fails the test; otherwise the test says
//! why it skips and passes.
#![cfg(target_os = "linux")]
#![allow(clippy::print_stderr)]

use lm_base::HostTime;
use lm_sys_alsa::{
    AccessChoice, HwRequest, Pcm, PcmIoError, PcmPoll, PcmState, SampleFormat, Stream, Waker, lost_capture_frames,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SECOND: u64 = 1_000_000_000;

/// The null device, or None (with the reason on stderr) when it cannot be used and the CI did not promise it
fn open_null(stream: Stream) -> Option<Pcm> {
    let promised = std::env::var("LM_TEST_ALSA_NULL").ok();
    let name = promised.clone().unwrap_or_else(|| "null".to_owned());
    match Pcm::open(&name, stream) {
        Ok(pcm) => Some(pcm),
        Err(e) if promised.is_none() => {
            eprintln!("SKIPPED: the ALSA null PCM cannot be opened here ({e}); set LM_TEST_ALSA_NULL to require it");
            None
        }
        Err(e) => panic!("LM_TEST_ALSA_NULL={name} but opening it failed: {e}"),
    }
}

fn loopback() -> Option<String> {
    match std::env::var("LM_TEST_ALSA_LOOPBACK") {
        Ok(card) => Some(card),
        Err(_) => {
            eprintln!("SKIPPED: no snd-aloop loopback card on this machine (LM_TEST_ALSA_LOOPBACK unset)");
            None
        }
    }
}

fn near_now(t: HostTime) -> bool {
    let now = HostTime::now();
    t.0 != 0 && now.since(t) < 10 * SECOND && t.since(now) < SECOND
}

#[test]
fn null_playback_runs_in_every_access_and_format() {
    for access in [AccessChoice::MmapOnly, AccessChoice::ReadWriteOnly] {
        for format in SampleFormat::PREFERENCE {
            let Some(mut pcm) = open_null(Stream::Playback) else { return };
            let mut req = HwRequest::new(48_000, 2, 256, 2);
            req.access = access;
            req.formats = vec![format];
            let config = pcm.configure(&req).unwrap_or_else(|e| panic!("{access:?} {format:?}: {e}"));
            assert_eq!(config.rate, 48_000);
            assert_eq!(config.channels, 2);
            assert_eq!(config.format, format);
            assert_eq!(config.access.is_mmap(), access == AccessChoice::MmapOnly);
            assert!(config.period_frames > 0 && config.buffer_frames >= config.period_frames);
            assert_eq!(pcm.state(), PcmState::Prepared);

            let prefill = pcm.write_silence(config.buffer_frames as usize).expect("prefill");
            assert!(prefill > 0, "{access:?} {format:?}: nothing prefilled");
            pcm.start().expect("start");
            assert_eq!(pcm.state(), PcmState::Running);

            let block: Vec<f32> = (0..config.period_frames as usize * 2).map(|i| (i as f32 * 0.01).sin()).collect();
            let left: Vec<f32> = block.iter().step_by(2).copied().collect();
            let right = left.clone();
            let mut written = 0;
            let deadline = Instant::now() + Duration::from_secs(2);
            while written < 4 * config.period_frames as usize && Instant::now() < deadline {
                written += pcm.write_interleaved(&block).expect("write interleaved");
                written += pcm.write_planar(&[&left, &right]).expect("write planar");
            }
            assert!(written > 0, "{access:?} {format:?}: the null device took nothing");

            let status = pcm.status().expect("status");
            assert_eq!(status.state, PcmState::Running);
            assert!(
                near_now(status.now),
                "status time {} is not CLOCK_MONOTONIC now ({})",
                status.now,
                HostTime::now()
            );
            let (_, stamp) = pcm.htimestamp().expect("htimestamp");
            assert!(stamp.0 == 0 || near_now(stamp), "htimestamp {stamp} is not CLOCK_MONOTONIC");
            pcm.avail().expect("avail");
            pcm.avail_update().expect("avail_update");
            pcm.delay().expect("delay");
            pcm.stop_now().expect("drop");
            assert_eq!(pcm.state(), PcmState::Setup);
        }
    }
}

#[test]
fn null_capture_reads_samples_in_range() {
    for access in [AccessChoice::MmapOnly, AccessChoice::ReadWriteOnly] {
        let Some(mut pcm) = open_null(Stream::Capture) else { return };
        let mut req = HwRequest::new(44_100, 3, 128, 3);
        req.access = access;
        let config = pcm.configure(&req).expect("configure capture");
        assert_eq!(config.channels, 3);
        pcm.start().expect("start");
        let mut interleaved = vec![9.0f32; config.period_frames as usize * 3];
        let mut a = vec![9.0f32; 64];
        let mut b = vec![9.0f32; 64];
        let mut c = vec![9.0f32; 64];
        let mut read = 0;
        let deadline = Instant::now() + Duration::from_secs(2);
        while read == 0 && Instant::now() < deadline {
            read += pcm.read_interleaved(&mut interleaved).expect("read interleaved");
            read += pcm.read_planar(&mut [&mut a, &mut b, &mut c]).expect("read planar");
        }
        assert!(read > 0, "{access:?}: the null capture gave nothing");
        assert!(interleaved.iter().chain(&a).all(|x| x.is_finite() && (-1.0..=1.0).contains(x) || *x == 9.0));
        // The wrong direction is refused without touching the device
        assert_eq!(pcm.write_silence(1), Err(PcmIoError::BadState));
    }
}

#[test]
fn recover_from_an_xrun_leaves_the_stream_prepared() {
    let Some(mut pcm) = open_null(Stream::Playback) else { return };
    let config = pcm.configure(&HwRequest::new(48_000, 2, 64, 2)).expect("configure");
    pcm.write_silence(config.buffer_frames as usize).expect("prefill");
    pcm.start().expect("start");
    pcm.recover(PcmIoError::Xrun).expect("recover");
    assert_eq!(pcm.state(), PcmState::Prepared);
    pcm.write_silence(config.buffer_frames as usize).expect("prefill again");
    pcm.start().expect("start again");
    let info = pcm.xrun_info().expect("xrun info");
    assert!(near_now(info.at) || info.at.0 == 0);
    // A disconnected device cannot be recovered
    assert_eq!(pcm.recover(PcmIoError::Disconnected), Err(PcmIoError::Disconnected));
}

#[test]
fn transfers_before_configure_are_refused() {
    let Some(mut pcm) = open_null(Stream::Playback) else { return };
    assert_eq!(pcm.write_silence(16), Err(PcmIoError::BadState));
    assert!(pcm.config().is_none());
}

#[test]
fn impossible_configurations_name_the_problem() {
    let Some(mut pcm) = open_null(Stream::Playback) else { return };
    let mut req = HwRequest::new(48_000, 2, 64, 2);
    req.formats = vec![];
    assert_eq!(pcm.configure(&req).map(drop).unwrap_err().key(), "alsa.format_unsupported");
    let e = Pcm::open("hw:LinuxMusicNoSuchCard", Stream::Playback).unwrap_err();
    assert_eq!(e.key(), "alsa.device_not_found", "{e}");
    assert_eq!(Pcm::open("bad\0name", Stream::Playback).unwrap_err().key(), "alsa.invalid_argument");
}

#[test]
fn poll_reports_ready_and_the_waker_stops_a_wait() {
    let Some(mut pcm) = open_null(Stream::Playback) else { return };
    let config = pcm.configure(&HwRequest::new(48_000, 2, 256, 2)).expect("configure");
    pcm.write_silence(config.buffer_frames as usize).expect("prefill");
    pcm.start().expect("start");
    let waker = Arc::new(Waker::new().expect("waker"));
    let mut poll = PcmPoll::new(&[&pcm], Some(waker.clone())).expect("poll set");
    let outcome = poll.wait(&[&pcm], Some(Duration::from_secs(2))).expect("wait");
    assert!(outcome.is_ready(0) || outcome.has_error(0), "{outcome:?}");

    // A wait on the waker alone returns when another thread wakes it, not at the timeout
    let mut only_waker = PcmPoll::new(&[], Some(waker.clone())).expect("poll set");
    let started = Instant::now();
    let thread = std::thread::spawn({
        let waker = waker.clone();
        move || {
            std::thread::sleep(Duration::from_millis(50));
            waker.wake();
        }
    });
    let outcome = only_waker.wait(&[], Some(Duration::from_secs(10))).expect("wait");
    thread.join().expect("waking thread");
    assert!(outcome.woken);
    assert!(started.elapsed() < Duration::from_secs(5));
    // The wake-up was consumed
    let outcome = only_waker.wait(&[], Some(Duration::from_millis(10))).expect("wait");
    assert!(outcome.is_empty());
}

#[test]
fn loopback_links_capture_and_playback_on_one_clock() {
    let Some(card) = loopback() else { return };
    let mut playback = Pcm::open(&format!("{card},0,0"), Stream::Playback).expect("open loopback playback");
    let mut capture = Pcm::open(&format!("{card},1,0"), Stream::Capture).expect("open loopback capture");
    let req = HwRequest::new(48_000, 2, 256, 2);
    let pc = playback.configure(&req).expect("configure playback");
    capture.configure(&req).expect("configure capture");
    capture.link(&mut playback).expect("link");
    assert!(capture.is_linked() && playback.is_linked());
    playback.write_silence(pc.buffer_frames as usize).expect("prefill");
    playback.start().expect("start both");
    assert_eq!(capture.state(), PcmState::Running, "the linked capture did not start with the playback");

    let mut poll = PcmPoll::new(&[&playback, &capture], None).expect("poll set");
    let tone: Vec<f32> = (0..pc.period_frames as usize * 2).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
    let mut heard = vec![0.0f32; pc.period_frames as usize * 2];
    let mut loud = false;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !loud && Instant::now() < deadline {
        let o = poll.wait(&[&playback, &capture], Some(Duration::from_millis(500))).expect("wait");
        if o.is_ready(0) {
            playback.write_interleaved(&tone).expect("write");
        }
        if o.is_ready(1) {
            let n = capture.read_interleaved(&mut heard).expect("read");
            loud = heard[..n * 2].iter().any(|x| (x.abs() - 0.5).abs() < 0.01);
        }
    }
    assert!(loud, "what was played never came back on the loopback's capture");
    capture.unlink().expect("unlink");
}

#[test]
fn loopback_capture_xrun_counts_the_lost_frames() {
    let Some(card) = loopback() else { return };
    let mut capture = Pcm::open(&format!("{card},1,1"), Stream::Capture).expect("open loopback capture");
    let config = capture.configure(&HwRequest::new(48_000, 2, 256, 2)).expect("configure");
    capture.start().expect("start");
    let started = HostTime::now();
    std::thread::sleep(Duration::from_millis(300));
    let mut buf = vec![0.0f32; 512];
    let err = loop {
        match capture.read_interleaved(&mut buf) {
            Err(e) => break e,
            Ok(_) if HostTime::now().since(started) > 3 * SECOND => panic!("no overrun after 3 s of not reading"),
            Ok(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    assert_eq!(err, PcmIoError::Xrun);
    let xrun = capture.xrun_info().expect("xrun info");
    assert!(near_now(xrun.at));
    assert!(xrun.pending >= u64::from(config.buffer_frames), "pending {} < buffer", xrun.pending);
    std::thread::sleep(Duration::from_millis(100));
    capture.recover(err).expect("recover");
    capture.start().expect("restart");
    let restarted = capture.trigger_time().expect("trigger time");
    let lost = lost_capture_frames(xrun, restarted, config.rate);
    // At least the 100 ms gap plus the full buffer; at most the whole time since the start
    let total = HostTime::nanos_to_samples(restarted.since(started), config.rate);
    assert!(lost >= 4_800 + u64::from(config.buffer_frames), "lost {lost}");
    assert!(lost <= total + u64::from(config.buffer_frames), "lost {lost} > elapsed {total}");
}
