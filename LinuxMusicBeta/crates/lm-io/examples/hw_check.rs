//! The real-machine check of lm-io (interfaces.md, Parte 4 step 8: "tested on a real Linux machine, not only in
//! CI"). Each mode prints `CHECK <name>: PASS|FAIL <detail>` lines and exits non-zero on a failure.
//!
//! ```text
//! hw_check duplex  <device> <seconds> <dir>   open duplex, play a tone, record the input, provoke an xrun, check the take
//! hw_check crash   <device> <dir>             record until killed (kill -9 it), printing the take's files
//! hw_check recover <dir>                      recover what `crash` left, check it, remove it
//! hw_check calibrate <device>                 acoustic round trip: speaker into microphone, five measures
//! hw_check reserve <card>                     ReserveDevice1 against the running PipeWire, then hand the card back
//! hw_check midi                               a virtual sequencer port into a MidiInput, stamps on the host clock
//! ```
//!
//! Run it from the user's graphical session (`systemd-run --user --pipe ...`) for rtkit and logind to answer: polkit
//! refuses both to a remote (SSH) session.

#![allow(clippy::print_stdout, clippy::print_stderr)]

#[cfg(target_os = "linux")]
fn main() {
    std::process::exit(linux::main());
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("hw_check runs on Linux only");
}

#[cfg(target_os = "linux")]
mod linux {
    use std::any::Any;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use lm_base::{Cancel, HostTime, Progress};
    use lm_io::*;
    use lm_render::*;
    use lm_sys_sndfile::Reader;
    use lm_time::{Bpm, Ppq, TempoMap, TimeSignature};

    const RATE: u32 = 48_000;
    /// The period asked (`LMIO_PERIOD` overrides it)
    static PERIOD_CELL: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    fn period() -> u32 {
        *PERIOD_CELL.get_or_init(|| std::env::var("LMIO_PERIOD").ok().and_then(|v| v.parse().ok()).unwrap_or(256))
    }

    static FAILED: AtomicBool = AtomicBool::new(false);

    fn check(name: &str, ok: bool, detail: impl std::fmt::Display) {
        println!("CHECK {name}: {} {detail}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            FAILED.store(true, Ordering::Relaxed);
        }
    }

    pub fn main() -> i32 {
        let args: Vec<String> = std::env::args().collect();
        let r = match args.get(1).map(String::as_str) {
            Some("duplex") if args.len() == 5 => duplex(&args[2], args[3].parse().unwrap_or(60), Path::new(&args[4])),
            Some("crash") if args.len() == 4 => crash(&args[2], Path::new(&args[3])),
            Some("recover") if args.len() == 3 => recover(Path::new(&args[2])),
            Some("calibrate") if args.len() == 3 => calibrate_mode(&args[2]),
            Some("reserve") if args.len() == 3 => reserve(args[2].parse().unwrap_or(0)),
            Some("midi") => midi(),
            Some("aloop") if args.len() == 4 => aloop(args[2].parse().unwrap_or(20), Path::new(&args[3])),
            _ => {
                eprintln!("usage: see the top of examples/hw_check.rs");
                return 2;
            }
        };
        if let Err(e) = r {
            println!("CHECK run: FAIL {e}");
            return 1;
        }
        i32::from(FAILED.load(Ordering::Relaxed))
    }

    fn limits() -> Limits {
        Limits { workers: 0, max_nodes: 64, max_params: 64, ..Limits::default() }
    }

    fn tempo() -> TempoMap {
        TempoMap::constant(RATE, Ppq::DEFAULT, Bpm::from_integer(120).expect("bpm"), TimeSignature::COMMON)
            .expect("map")
    }

    /// A quiet tone on both outputs; when `stall` is set, the next call sleeps that long once (a provoked xrun)
    struct Tone {
        phase: f64,
        stall: Arc<std::sync::atomic::AtomicU64>,
        level: f32,
        freq: f64,
    }

    impl Processor for Tone {
        fn process(&mut self, cx: &mut ProcessContext<'_>) {
            let ms = self.stall.swap(0, Ordering::Relaxed);
            if ms > 0 {
                std::thread::sleep(Duration::from_millis(ms));
            }
            let step = self.freq / f64::from(cx.transport.sample_rate);
            for k in 0..cx.frames {
                let v = (self.phase * std::f64::consts::TAU).sin() as f32 * self.level;
                for ch in 0..cx.buffers.outputs() {
                    cx.buffers.output(ch)[k] = v;
                }
                self.phase = (self.phase + step).fract();
            }
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn tone_program(
        spec: ProcessSpec,
        outputs: usize,
        stall: Arc<std::sync::atomic::AtomicU64>,
        level: f32,
    ) -> Install {
        let n = |i| NodeIndex::new(i, 1);
        let mut b = ProgramBuilder::new(ProgramGen(1), spec, &limits(), tempo());
        b.node(NodeDesc::sum(n(0), 2));
        b.master(n(0));
        for c in 0..outputs.min(2) {
            b.output(OutputRoute { node: n(0), channel: c as u16, device_channel: c as u16 });
        }
        b.node(NodeDesc::processor(n(1), 0, 2));
        b.feed(Feed::new(n(1), n(0), 2));
        let mut i = Install::new(b.build().expect("program"));
        i.add(n(1), NodeCore::new(Box::new(Tone { phase: 0.0, stall, level, freq: 440.0 }), 1, None));
        i
    }

    struct Session {
        bus: Arc<lm_sys_linux::SystemBus>,
        device: Device,
        engine: EngineControl,
        report: Arc<RtReport>,
        info: DeviceInfo,
        stall: Arc<std::sync::atomic::AtomicU64>,
    }

    fn open(device: &str) -> lm_base::Result<Session> {
        open_with(&DeviceConfig::duplex(device, RATE, period()), 0.03)
    }

    fn open_with(config: &DeviceConfig, level: f32) -> lm_base::Result<Session> {
        let bus = Arc::new(lm_sys_linux::SystemBus::start()?);
        let report = Arc::new(RtReport::default());
        let period = Duration::from_nanos(u64::from(period()) * 1_000_000_000 / u64::from(RATE));
        let threads = realtime_hook(Arc::new(SystemRealtime::new(Some(bus.clone()))), period, report.clone());
        let mut dev = Device::new(Box::new(AlsaBackend::new(Some(bus.clone()))), threads.clone());
        let t0 = Instant::now();
        let info = dev.open(config)?;
        println!("INFO device {info:?} (opened in {:?})", t0.elapsed());
        check(
            "negotiated",
            info.max_block == info.period as usize && info.linked,
            format!("period {} linked {}", info.period, info.linked),
        );
        let (mut engine, renderer) = engine_with(info.spec(), limits(), threads);
        dev.start(renderer).map_err(|(e, _)| e)?;
        let stall = Arc::new(std::sync::atomic::AtomicU64::new(0));
        engine.install(tone_program(info.spec(), info.outputs, stall.clone(), level))?;
        Ok(Session { bus, device: dev, engine, report, info, stall })
    }

    fn wait(secs: f64, mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    fn rms_windows(samples: &[f32], channels: usize, window: usize) -> Vec<f64> {
        samples
            .chunks(window * channels)
            .map(|w| (w.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / w.len().max(1) as f64).sqrt())
            .collect()
    }

    fn read_take(path: &Path) -> lm_base::Result<(Vec<f32>, usize)> {
        let mut r = Reader::open(path)?;
        let ch = r.info().channels;
        let mut out = vec![0.0f32; r.info().frames as usize * ch];
        let mut at = 0;
        while at < out.len() {
            let n = r.read_f32(&mut out[at..])?;
            if n == 0 {
                break;
            }
            at += n * ch;
        }
        out.truncate(at);
        Ok((out, ch))
    }

    /// The fan's noise: present everywhere, stable over 100 ms windows; gaps exactly where reported
    fn check_continuity(samples: &[f32], ch: usize, gaps: &[TakeGap], label: &str) {
        let window = (RATE / 10) as usize;
        let rms = rms_windows(samples, ch, window);
        let in_gap = |w: usize| {
            let (a, b) = ((w * window) as u64, ((w + 1) * window) as u64);
            gaps.iter().any(|g| g.file_frame < b && g.file_frame + g.frames > a)
        };
        let mut steady: Vec<f64> = rms
            .iter()
            .enumerate()
            .filter(|(w, _)| *w >= 5 && !in_gap(*w) && *w + 1 < rms.len())
            .map(|(_, v)| *v)
            .collect();
        steady.sort_by(|a, b| a.total_cmp(b));
        let median = steady.get(steady.len() / 2).copied().unwrap_or(0.0);
        check(
            &format!("{label}.not_silent"),
            median > 1e-5,
            format!("median RMS per 100 ms {median:.6} ({:.1} dBFS)", 20.0 * median.max(1e-12).log10()),
        );
        let low = steady.first().copied().unwrap_or(0.0);
        let high = steady.last().copied().unwrap_or(0.0);
        check(
            &format!("{label}.rms_stable"),
            low > median / 4.0 && high < median * 8.0,
            format!("{} windows, min {low:.6} max {high:.6} (allowed median/4 .. median×8)", steady.len()),
        );
        // No run of exact zeros outside the reported gaps (a hole), and no repeated block (a duplicate)
        let frames = samples.len() / ch;
        let mut zero_run = 0usize;
        let mut worst = 0usize;
        for f in 0..frames {
            let inside = gaps.iter().any(|g| (f as u64) >= g.file_frame && (f as u64) < g.file_frame + g.frames);
            if !inside && samples[f * ch..(f + 1) * ch].iter().all(|x| *x == 0.0) {
                zero_run += 1;
                worst = worst.max(zero_run);
            } else {
                zero_run = 0;
            }
        }
        check(
            &format!("{label}.no_holes"),
            worst < 32,
            format!("longest run of exact zeros outside gaps: {worst} frames"),
        );
        let block = period() as usize * ch;
        let mut repeats = 0;
        for k in 1..samples.len() / block {
            if samples[k * block..(k + 1) * block] == samples[(k - 1) * block..k * block]
                && samples[k * block..(k + 1) * block].iter().any(|x| *x != 0.0)
            {
                repeats += 1;
            }
        }
        check(&format!("{label}.no_repeats"), repeats == 0, format!("{repeats} periods identical to the one before"));
        for g in gaps {
            let a = g.file_frame as usize * ch;
            let b = ((g.file_frame + g.frames) as usize * ch).min(samples.len());
            let silent = samples[a..b].iter().all(|x| *x == 0.0);
            check(
                &format!("{label}.gap_silent"),
                silent,
                format!("gap at {} of {} frames is exact silence", g.file_frame, g.frames),
            );
        }
    }

    fn duplex(device: &str, seconds: u64, dir: &Path) -> lm_base::Result<()> {
        std::fs::create_dir_all(dir).map_err(|e| lm_base::Error::new("io").arg("m", e.to_string()))?;
        let mut s = open(device)?;
        let mut sleep = SleepGuard::new(Some(s.bus.clone()));
        sleep.set(true, "Recording");
        let held = wait(5.0, || {
            if let Some(e) = sleep.poll() {
                println!("INFO inhibitor: {e}");
                return true;
            }
            sleep.is_held()
        }) && sleep.is_held();
        check("logind_inhibitor", held, "sleep:idle held while recording (H2)");
        let inputs: Vec<u16> = (0..s.info.inputs as u16).collect();
        let space = check_record_space(dir, inputs.len(), RATE)?;
        check("disk_space", !matches!(space, SpaceCheck::Refused { .. }), format!("{space:?}"));
        let mut disk = DiskWriter::start()?;
        let plan = RecordPlan::new(
            dir.to_path_buf(),
            vec![RecordInput { input: InputIndex(0), channels: inputs, tracks: vec!["t:1".into()] }],
            RATE,
        );
        let (sender, receiver) = recording_channel(plan)?;
        disk.record(receiver)?;
        s.device.arm(sender).map_err(|_| lm_base::Error::new("arm"))?;
        check("armed", wait(2.0, || s.device.is_armed()), "the callback took the recorder");
        s.engine.commands().send(Command::Record { on: true }).map_err(|_| lm_base::Error::new("cmd"))?;
        s.engine.commands().send(Command::Play).map_err(|_| lm_base::Error::new("cmd"))?;
        let started = wait(5.0, || s.engine.frame().recording);
        check("recording", started, "the transport records");
        std::thread::sleep(Duration::from_secs(1));
        let f0 = (s.engine.frame().position, s.engine.frame().host_time);
        let mut xruns = Vec::new();
        let mut events = Vec::new();
        let t_end = Instant::now() + Duration::from_secs(seconds);
        let mut provoked = false;
        let half = Instant::now() + Duration::from_secs(seconds / 2);
        while Instant::now() < t_end {
            let _ = s.device.poll(|e| match e {
                DeviceEvent::Xrun { capture_lost, late_by } => xruns.push((capture_lost, late_by)),
                other => events.push(format!("{other:?}")),
            });
            disk.poll(|e| events.push(format!("{e:?}")));
            disk.live_peaks(|_| {});
            if !provoked && Instant::now() > half {
                // Three buffers' worth of a stuck plugin: an xrun on both streams
                let buffer_ms = u64::from(s.info.buffer) * 1000 / u64::from(RATE);
                s.stall.store(buffer_ms * 3 + 20, Ordering::Relaxed);
                provoked = true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let f1 = (s.engine.frame().position, s.engine.frame().host_time);
        s.engine.commands().send(Command::Stop).map_err(|_| lm_base::Error::new("cmd"))?;
        let mut takes = Vec::new();
        wait(10.0, || {
            disk.poll(|e| match e {
                DiskEvent::TakeClosed(t) => takes.push(t),
                other => events.push(format!("{other:?}")),
            });
            !takes.is_empty()
        });
        let _ = s.device.poll(|e| match e {
            DeviceEvent::Xrun { capture_lost, late_by } => xruns.push((capture_lost, late_by)),
            other => events.push(format!("{other:?}")),
        });
        for e in &events {
            println!("INFO event {e}");
        }
        let rt = s.report.last_method();
        check(
            "realtime",
            s.report.failed() == 0 && rt.is_some(),
            format!("{rt:?}, failure {:?}", s.report.take_failure()),
        );
        // The device clock against CLOCK_MONOTONIC over the run (C6): within 500 ppm
        let dpos = (f1.0 - f0.0) as f64;
        let dt = f1.1.since(f0.1) as f64 / 1e9;
        let lost: u64 = xruns.iter().map(|(l, _)| u64::from(*l)).sum();
        let measured = dpos / dt;
        check(
            "device_clock",
            (measured / f64::from(RATE) - 1.0).abs() < 5e-4,
            format!("{measured:.1} frames/s over {dt:.2} s (positions include {lost} lost frames)"),
        );
        check("xrun_provoked", !xruns.is_empty(), format!("{xruns:?}"));
        let renderer = s.device.stop().map_err(|t| t.to_error())?;
        drop(renderer);
        let Some(take) = takes.into_iter().next() else {
            check("take", false, "no take closed");
            return Ok(());
        };
        println!("INFO take {} frames, {} gaps {:?}, spans {:?}", take.frames, take.gaps.len(), take.gaps, take.spans);
        let lost_in_take: u64 = take.gaps.iter().map(|g| g.frames).sum();
        check(
            "gaps_match_xruns",
            lost_in_take == lost,
            format!("gaps {lost_in_take} frames, xruns reported {lost} capture frames lost"),
        );
        check("take_stopped", take.stopped.is_none(), format!("{:?}", take.stopped));
        // Duration: the take holds every frame the transport ran while recording (C2: lost ones as silence)
        let expected = f1.0 - f0.0;
        check(
            "duration",
            take.frames >= expected,
            format!("take {} frames ≥ {} frames between the two telemetry samples", take.frames, expected),
        );
        let (samples, ch) = read_take(&take.path)?;
        check(
            "rf64_readable",
            samples.len() as i64 == take.frames * ch as i64,
            format!("{} frames read", samples.len() / ch.max(1)),
        );
        check_continuity(&samples, ch, &take.gaps, "take");
        let fin = finalize_take(&take, TakeFormat::Float32, &Cancel::new(), &Progress::new())?;
        check("finalized", fin.take.path.exists() && fin.peaks_path.exists(), &fin.key);
        for p in [&fin.take.path, &fin.peaks_path, &fin.take.record] {
            let _ = std::fs::remove_file(p);
        }
        sleep.set(false, "");
        let _ = sleep.poll();
        Ok(())
    }

    fn crash(device: &str, dir: &Path) -> lm_base::Result<()> {
        std::fs::create_dir_all(dir).map_err(|e| lm_base::Error::new("io").arg("m", e.to_string()))?;
        let mut s = open(device)?;
        let mut disk = DiskWriter::start()?;
        let inputs: Vec<u16> = (0..s.info.inputs as u16).collect();
        let plan = RecordPlan::new(
            dir.to_path_buf(),
            vec![RecordInput { input: InputIndex(0), channels: inputs, tracks: vec!["t:1".into()] }],
            RATE,
        );
        let (sender, receiver) = recording_channel(plan)?;
        disk.record(receiver)?;
        s.device.arm(sender).map_err(|_| lm_base::Error::new("arm"))?;
        wait(2.0, || s.device.is_armed());
        s.engine.commands().send(Command::Record { on: true }).map_err(|_| lm_base::Error::new("cmd"))?;
        s.engine.commands().send(Command::Play).map_err(|_| lm_base::Error::new("cmd"))?;
        let start = HostTime::now();
        loop {
            disk.poll(|e| {
                if let DiskEvent::TakeStarted { path, .. } = e {
                    println!("RECORDING {} at {}", path.display(), start.0);
                }
            });
            let _ = s.device.poll(|_| {});
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn recover(dir: &Path) -> lm_base::Result<()> {
        let found = find_interrupted(dir)?;
        check("interrupted_found", found.len() == 1, format!("{found:?}"));
        for record in found {
            let (header, lines) = read_record(&record)?;
            println!("INFO record header {header:?}, {} lines", lines.len());
            let take = recover_take(&record, &Cancel::new(), &Progress::new())?;
            let modified = std::fs::metadata(&take.path).and_then(|m| m.modified()).ok();
            println!(
                "INFO recovered {} frames ({:.2} s), file modified {modified:?}",
                take.frames,
                take.frames as f64 / f64::from(RATE)
            );
            let (samples, ch) = read_take(&take.path)?;
            check(
                "recovered_readable",
                samples.len() as i64 == take.frames * ch as i64 && take.frames > 0,
                format!("{} frames", take.frames),
            );
            check_continuity(&samples, ch, &take.gaps, "recovered");
            check("not_offered_again", find_interrupted(dir)?.is_empty(), "the recovered take has its end");
            let fin = finalize_take(&take, TakeFormat::Float32, &Cancel::new(), &Progress::new())?;
            for p in [&fin.take.path, &fin.peaks_path, &fin.take.record] {
                let _ = std::fs::remove_file(p);
            }
        }
        Ok(())
    }

    fn calibrate_mode(device: &str) -> lm_base::Result<()> {
        let bus = Arc::new(lm_sys_linux::SystemBus::start()?);
        let backend = Box::new(AlsaBackend::new(Some(bus)));
        let options = CalibrationOptions { level: 0.5, min_snr: 6.0, ..CalibrationOptions::default() };
        let (result, _) = calibrate(
            backend,
            &DeviceConfig::duplex(device, RATE, period()),
            options,
            RenderThreads::default(),
            &Cancel::new(),
            &Progress::new(),
        );
        match result {
            Ok(c) => {
                check(
                    "calibration",
                    true,
                    format!(
                        "measures {:?}, round trip {} frames ({:.2} ms), reported {}, residual {} ({:.2} ms), weakest SNR {:.1}",
                        c.measures,
                        c.round_trip,
                        c.round_trip as f64 * 1000.0 / f64::from(RATE),
                        c.reported,
                        c.residual,
                        c.residual as f64 * 1000.0 / f64::from(RATE),
                        c.snr
                    ),
                );
            }
            Err(e) => check("calibration", false, e),
        }
        Ok(())
    }

    fn reserve(card: u32) -> lm_base::Result<()> {
        let bus = lm_sys_linux::SystemBus::start()?;
        let name = format!("hw:{card},0");
        let before = lm_sys_alsa::Pcm::open(&name, lm_sys_alsa::Stream::Playback).err().map(|e| e.key());
        println!("INFO opening {name} before reserving: {before:?}");
        let request = lm_sys_linux::ReserveRequest {
            card,
            application_name: "LinuxMusic".into(),
            device_name: "check".into(),
            priority: 20,
        };
        let reservation = bus.reserve_audio_device(request).wait(Duration::from_secs(10));
        match &reservation {
            Ok(r) => println!("INFO reserved card {}", r.card()),
            Err(e) => println!("INFO reserve: {e}"),
        }
        check("reserve", reservation.is_ok(), "ReserveDevice1 granted by the running audio server");
        let opened = wait(3.0, || lm_sys_alsa::Pcm::open(&name, lm_sys_alsa::Stream::Playback).is_ok());
        check("open_after_reserve", opened, format!("{name} opens while reserved"));
        drop(reservation);
        // Handed back: the server can take the card again (it reopens it when it plays)
        std::thread::sleep(Duration::from_millis(500));
        let shut = bus.shutdown(Duration::from_secs(3));
        check("handed_back", shut, "the reservation was released and the bus thread ended");
        Ok(())
    }

    /// snd-aloop: what is played on device 0 comes back exactly on device 1 of the same card. The tone recorded must
    /// be the tone played (frequency and level), and the xrun's gap exact silence; then the calibration measures the
    /// digital round trip
    fn aloop(seconds: u64, dir: &Path) -> lm_base::Result<()> {
        std::fs::create_dir_all(dir).map_err(|e| lm_base::Error::new("io").arg("m", e.to_string()))?;
        let mut cfg = DeviceConfig::duplex("hw:CARD=Loopback,DEV=0", RATE, period());
        cfg.capture = Some("hw:CARD=Loopback,DEV=1".into());
        let mut s = open_with(&cfg, 0.5)?;
        let mut disk = DiskWriter::start()?;
        let plan = RecordPlan::new(
            dir.to_path_buf(),
            vec![RecordInput { input: InputIndex(0), channels: vec![0], tracks: vec![] }],
            RATE,
        );
        let (sender, receiver) = recording_channel(plan)?;
        disk.record(receiver)?;
        s.device.arm(sender).map_err(|_| lm_base::Error::new("arm"))?;
        wait(2.0, || s.device.is_armed());
        s.engine.commands().send(Command::Record { on: true }).map_err(|_| lm_base::Error::new("cmd"))?;
        s.engine.commands().send(Command::Play).map_err(|_| lm_base::Error::new("cmd"))?;
        let mut xruns = Vec::new();
        let end = Instant::now() + Duration::from_secs(seconds);
        let half = Instant::now() + Duration::from_secs(seconds / 2);
        let mut provoked = false;
        while Instant::now() < end {
            let _ = s.device.poll(|e| {
                if let DeviceEvent::Xrun { capture_lost, late_by } = e {
                    xruns.push((capture_lost, late_by));
                }
            });
            disk.poll(|_| {});
            if !provoked && Instant::now() > half {
                s.stall.store(u64::from(s.info.buffer) * 3000 / u64::from(RATE) + 20, Ordering::Relaxed);
                provoked = true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        s.engine.commands().send(Command::Stop).map_err(|_| lm_base::Error::new("cmd"))?;
        let mut takes = Vec::new();
        wait(10.0, || {
            disk.poll(|e| {
                if let DiskEvent::TakeClosed(t) = e {
                    takes.push(t);
                }
            });
            !takes.is_empty()
        });
        let _ = s.device.poll(|e| {
            if let DeviceEvent::Xrun { capture_lost, late_by } = e {
                xruns.push((capture_lost, late_by));
            }
        });
        drop(s.device.stop().map_err(|t| t.to_error())?);
        let take = takes.into_iter().next().ok_or_else(|| lm_base::Error::new("no take"))?;
        let lost: u64 = xruns.iter().map(|x| u64::from(x.0)).sum();
        let gap: u64 = take.gaps.iter().map(|g| g.frames).sum();
        check("aloop.xrun", !xruns.is_empty() && gap == lost, format!("xruns {xruns:?}, gaps {:?}", take.gaps));
        let (samples, _) = read_take(&take.path)?;
        // Each whole second outside the gap: 440 rising zero crossings and a peak of 0.5
        let mut bad = Vec::new();
        let rate = RATE as usize;
        for (k, sec) in samples.chunks(rate).enumerate() {
            if sec.len() < rate || k == 0 {
                continue;
            }
            let start = (k * rate) as u64;
            // The gap, and the prefilled silence that comes back through the loop after the restart
            if take.gaps.iter().any(|g| g.file_frame < start + rate as u64 && g.file_frame + g.frames + 2048 > start) {
                continue;
            }
            let crossings = sec.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
            let peak = sec.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            if !(439..=441).contains(&crossings) || (peak - 0.5).abs() > 0.01 {
                bad.push((k, crossings, peak));
            }
        }
        check("aloop.tone", bad.is_empty(), format!("seconds off: {bad:?}"));
        check_holes_only(&samples, &take.gaps);
        for p in [&take.path, &take.record] {
            let _ = std::fs::remove_file(p);
        }
        let bus = Arc::new(lm_sys_linux::SystemBus::start()?);
        let (result, _) = calibrate(
            Box::new(AlsaBackend::new(Some(bus))),
            &cfg,
            CalibrationOptions::default(),
            RenderThreads::default(),
            &Cancel::new(),
            &Progress::new(),
        );
        match result {
            Ok(c) => check("aloop.calibration", c.measures.iter().all(|m| *m == c.round_trip), format!("{c:?}")),
            Err(e) => check("aloop.calibration", false, e),
        }
        Ok(())
    }

    /// A tone through a digital loopback is never exactly zero for 32 frames, except in the gaps
    fn check_holes_only(samples: &[f32], gaps: &[TakeGap]) {
        let mut run = 0;
        let mut worst = 0;
        for (f, x) in samples.iter().enumerate() {
            let inside = gaps.iter().any(|g| (f as u64) >= g.file_frame && (f as u64) < g.file_frame + g.frames);
            if *x == 0.0 && !inside {
                run += 1;
                worst = worst.max(run);
            } else {
                run = 0;
            }
        }
        // The first periods are the prefill's silence coming back through the loop
        check("aloop.no_holes", worst <= (period() as usize) * 4, format!("longest zero run outside gaps {worst}"));
    }

    struct Collect(std::sync::mpsc::Sender<(HostTime, [u8; 3])>);

    impl MidiSink for Collect {
        fn deliver(&mut self, _: PortIndex, at: HostTime, bytes: [u8; 3]) -> bool {
            self.0.send((at, bytes)).is_ok()
        }
    }

    fn midi() -> lm_base::Result<()> {
        let mut seq = lm_sys_alsa::Seq::open("hw_check keyboard")?;
        let out = seq.create_output_port("keys")?;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut input =
            MidiInput::open_seq("hw_check in", Some((out.client, out.port)), PortIndex(3), Box::new(Collect(tx)))?;
        let ports = list_midi_ports(&Cancel::new())?;
        check(
            "ports_listed",
            ports.iter().any(|p| p.name.contains("hw_check keyboard")),
            format!("{} ports", ports.len()),
        );
        std::thread::sleep(Duration::from_millis(100));
        let mut sent = Vec::new();
        for k in 0..200u8 {
            let t = HostTime::now();
            seq.send(out, None, &lm_sys_alsa::MidiMessage::new3(0x90, k % 128, 100))?;
            sent.push(t);
            std::thread::sleep(Duration::from_micros(2_000));
        }
        let mut got = Vec::new();
        wait(2.0, || {
            while let Ok(m) = rx.try_recv() {
                got.push(m);
            }
            got.len() >= 200
        });
        check("midi_count", got.len() == 200, format!("{} of 200", got.len()));
        let mut worst = 0u64;
        let mut order = true;
        for (k, (at, bytes)) in got.iter().enumerate() {
            worst = worst.max(at.since(sent[k]).max(sent[k].since(*at)));
            order &= bytes[1] == k as u8 % 128;
        }
        check("midi_order", order, "in order, unchanged");
        check(
            "midi_stamps",
            worst < 2_000_000,
            format!("worst |stamp − send| {:.3} ms, queue stamps: {}", worst as f64 / 1e6, !input.stamps_on_arrival()),
        );
        input.stop();
        check("midi_stats", input.stats().lost == 0, format!("{:?}", input.stats()));
        Ok(())
    }

    #[allow(dead_code)]
    fn _unused(_: PathBuf) {}
}
