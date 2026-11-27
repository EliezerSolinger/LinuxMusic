//! The ALSA backend (interfaces.md, I-10; U7, U8, U10, U11, U17; C2, C6, C7), over `lm-sys-alsa`.
//!
//! **One duplex `hw` device** in F1 (U7): capture and playback of the same card, opened exclusively, configured
//! alike and linked (`snd_pcm_link`), so they start, stop and run on one clock. Capture and playback on different
//! cards only with the user's explicit drift correction: then the playback is the clock and the capture goes through
//! the adaptive resampler of [`Edge`]. Nothing is ever joined behind the user's back.
//!
//! **The loop** (one thread, real time through the render-thread hook, H1): `poll` on both PCMs and the stop waker;
//! for every whole period available on both sides: read the capture, stamp the block with the capture's host time
//! (`snd_pcm_htimestamp` on `CLOCK_MONOTONIC`, C6), call the renderer, write the playback. The block is the
//! **negotiated** period (C7).
//!
//! **Xruns:** the capture's pending frames are read before recovering, the stream is prepared, prefilled and started
//! again, and the lost frames (pending + the time until the restart, C2) go to the next block's `capture_lost`, so
//! the take keeps the grid. **A removed card** (`ENODEV`, U11) ends the thread with `Disconnected`; the session
//! falls back to the Dummy. **Suspend** is resumed without blocking the loop.
//!
//! **A busy card** (U8): ReserveDevice1 asks PipeWire or PulseAudio to let it go (as JACK does), and the reservation
//! is held while the device is open; closing hands the card back.

use std::fmt;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use lm_base::{Error, HostTime, Result, Role};
use lm_render::{DeviceBlock, ThreadKind};
use lm_sys_alsa::enumerate::{self, Probe};
use lm_sys_alsa::{HwConfig, HwRequest, Pcm, PcmIoError, PcmPoll, PcmState, Stream, Waker, lost_capture_frames};

use crate::backend::{Backend, CallbackHandle, DeviceConfig, DeviceEvent, DeviceInfo};
use crate::choice::{ChoiceCaps, DeviceChoice};
use crate::edge::{Edge, MAX_CHANNELS};
use crate::rt::RtBuffer;
use crate::system::device_error;

/// Rates tried, in order, when the device cannot run at the project's and the edge resampler is allowed
const FALLBACK_RATES: [u32; 6] = [48_000, 44_100, 96_000, 88_200, 192_000, 176_400];

/// The ALSA backend
pub struct AlsaBackend {
    bus: Option<Arc<lm_sys_linux::SystemBus>>,
    open: Option<Opened>,
    waker: Option<Arc<Waker>>,
    reservations: Vec<lm_sys_linux::Reservation>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for AlsaBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AlsaBackend")
            .field("open", &self.open.as_ref().map(|o| &o.info))
            .field("reservations", &self.reservations.len())
            .field("running", &self.thread.is_some())
            .finish()
    }
}

/// What `open` made, moved to the thread by `start`
struct Opened {
    info: DeviceInfo,
    playback: Option<Pcm>,
    capture: Option<Pcm>,
    poll: PcmPoll,
    linked: bool,
    edge: Option<Edge>,
    play_config: Option<HwConfig>,
    cap_config: Option<HwConfig>,
}

impl AlsaBackend {
    /// `bus`: the session's D-Bus thread, for ReserveDevice1 (U8). Without it a busy card is just reported busy
    pub fn new(bus: Option<Arc<lm_sys_linux::SystemBus>>) -> Self {
        Self { bus, open: None, waker: None, reservations: Vec::new(), thread: None }
    }

    /// Opens one PCM, asking its owner to let the card go if it is busy (U8)
    fn open_pcm(&mut self, name: &str, stream: Stream) -> Result<Pcm> {
        match Pcm::open(name, stream) {
            Ok(p) => Ok(p),
            Err(e) if e.key() == "alsa.device_busy" => {
                let Some(bus) = self.bus.clone() else { return Err(device_error(e, name)) };
                let Some(card) = card_index(name) else { return Err(device_error(e, name)) };
                if !self.reservations.iter().any(|r| r.card() == card as u32) {
                    let request = lm_sys_linux::ReserveRequest {
                        card: card as u32,
                        application_name: "LinuxMusic".to_owned(),
                        device_name: card_name(card).unwrap_or_else(|| name.to_owned()),
                        priority: 20,
                    };
                    match bus.reserve_audio_device(request).wait(Duration::from_secs(5)) {
                        Ok(r) => self.reservations.push(r),
                        Err(re) if re.key() == "linux.reserve.refused" => return Err(device_error(re, name)),
                        // No session bus, or a failure: nobody can be asked; the card stays busy
                        Err(_) => return Err(device_error(e, name)),
                    }
                }
                // The owner closes the card asynchronously after agreeing: try for a moment
                let mut last = e;
                for _ in 0..50 {
                    match Pcm::open(name, stream) {
                        Ok(p) => return Ok(p),
                        Err(e2) if e2.key() == "alsa.device_busy" => {
                            last = e2;
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        Err(e2) => return Err(device_error(e2, name)),
                    }
                }
                Err(device_error(last, name))
            }
            Err(e) => Err(device_error(e, name)),
        }
    }

    /// The rate to run the device at: the project's when it can, else (with the edge resampler) the first fallback
    /// it has
    fn choose_rate(names: &[(&str, Stream)], config: &DeviceConfig) -> Result<u32> {
        let mut caps = Vec::new();
        for (name, stream) in names {
            match enumerate::probe(name, *stream) {
                Ok(c) => caps.push(c),
                // Busy now (it may be ours to take): the configuration will say
                Err(_) => return Ok(config.rate),
            }
        }
        let fits = |rate: u32| {
            caps.iter()
                .all(|c| rate >= c.rate_min && rate <= c.rate_max && (c.rates.is_empty() || c.rates.contains(&rate)))
        };
        if fits(config.rate) {
            return Ok(config.rate);
        }
        if !config.allow_edge_resampler {
            return Err(Error::new("io.device.rate_unsupported").arg("rate", i64::from(config.rate)));
        }
        FALLBACK_RATES
            .iter()
            .copied()
            .find(|r| fits(*r))
            .ok_or_else(|| Error::new("io.device.rate_unsupported").arg("rate", i64::from(config.rate)))
    }

    fn channels_for(name: &str, stream: Stream, wanted: Option<u32>) -> Result<u32> {
        if let Some(c) = wanted {
            return Ok(c);
        }
        match enumerate::probe(name, stream) {
            Ok(c) => Ok(c.channels_max.min(MAX_CHANNELS as u32).max(c.channels_min)),
            // Busy: the usual stereo; a wrong guess fails the configuration with the device's range
            Err(_) => Ok(2),
        }
    }
}

/// The card number of an ALSA device name (`hw:1`, `hw:1,0`, `plughw:CARD=PCH,DEV=0`, `hw:CARD=1`)
pub fn card_index(name: &str) -> Option<i32> {
    let rest = name.split_once(':')?.1;
    let first = rest.split(',').next()?;
    let card = first.strip_prefix("CARD=").unwrap_or(first);
    if let Ok(n) = card.parse::<i32>() {
        return Some(n);
    }
    enumerate::cards(false).ok()?.into_iter().find(|c| c.id == card).map(|c| c.index)
}

fn card_name(index: i32) -> Option<String> {
    enumerate::card(index, false).ok().map(|c| c.name)
}

impl Backend for AlsaBackend {
    fn open(&mut self, config: &DeviceConfig) -> Result<DeviceInfo> {
        if self.thread.is_some() {
            return Err(Error::new("io.device.running"));
        }
        self.open = None;
        if config.playback.is_none() && config.capture.is_none() {
            return Err(Error::new("io.device.bad_config"));
        }
        let two_cards = match (&config.playback, &config.capture) {
            (Some(p), Some(c)) => card_index(p) != card_index(c) || card_index(p).is_none() && p != c,
            _ => false,
        };
        // U7: different cards only with the explicit drift correction
        if two_cards && !config.drift_correction {
            return Err(Error::new("io.device.two_cards")
                .arg("playback", config.playback.clone().unwrap_or_default())
                .arg("capture", config.capture.clone().unwrap_or_default()));
        }
        let mut streams = Vec::new();
        if let Some(p) = &config.playback {
            streams.push((p.as_str(), Stream::Playback));
        }
        if let Some(c) = &config.capture {
            streams.push((c.as_str(), Stream::Capture));
        }
        let device_rate = Self::choose_rate(&streams, config)?;
        let period = config.period.max(16);
        let periods = config.periods.clamp(2, 8);

        let mut playback = None;
        let mut play_config = None;
        if let Some(name) = config.playback.clone() {
            let channels = Self::channels_for(&name, Stream::Playback, config.playback_channels)?;
            let mut pcm = self.open_pcm(&name, Stream::Playback)?;
            let cfg = pcm
                .configure(&HwRequest::new(device_rate, channels, period, periods))
                .map_err(|e| device_error(e, &name))?;
            play_config = Some(cfg);
            playback = Some(pcm);
        }
        let mut capture = None;
        let mut cap_config = None;
        if let Some(name) = config.capture.clone() {
            let channels = Self::channels_for(&name, Stream::Capture, config.capture_channels)?;
            let mut pcm = self.open_pcm(&name, Stream::Capture)?;
            // The capture gets the playback's negotiated period, so the linked streams move in the same steps
            let want = play_config.map_or(period, |c| c.period_frames);
            let cfg = pcm
                .configure(&HwRequest::new(device_rate, channels, want, periods))
                .map_err(|e| device_error(e, &name))?;
            cap_config = Some(cfg);
            capture = Some(pcm);
        }
        let mut linked = false;
        if let (Some(p), Some(c)) = (playback.as_mut(), capture.as_mut())
            && !two_cards
        {
            let same_period = play_config.map(|x| x.period_frames) == cap_config.map(|x| x.period_frames);
            if !same_period {
                return Err(Error::new("io.device.period_mismatch")
                    .arg("playback", i64::from(play_config.map_or(0, |x| x.period_frames)))
                    .arg("capture", i64::from(cap_config.map_or(0, |x| x.period_frames))));
            }
            match c.link(p) {
                Ok(()) => linked = true,
                Err(e) if config.drift_correction => {
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        }
        let drift = playback.is_some() && capture.is_some() && !linked;
        let negotiated = play_config.or(cap_config).map_or(period, |c| c.period_frames);
        let inputs = cap_config.map_or(0, |c| c.channels as usize);
        let outputs = play_config.map_or(0, |c| c.channels as usize);
        if inputs > MAX_CHANNELS || outputs > MAX_CHANNELS {
            return Err(Error::new("io.device.too_many_channels").arg("max", MAX_CHANNELS as i64));
        }
        let cap_period = cap_config.map_or(negotiated, |c| c.period_frames);
        let edge = if device_rate != config.rate || drift {
            Some(Edge::new(config.rate, device_rate, inputs, outputs, negotiated.max(cap_period) as usize * 2, drift)?)
        } else {
            None
        };
        let waker = Arc::new(Waker::new()?);
        let mut pcms: Vec<&Pcm> = Vec::new();
        if let Some(p) = playback.as_ref() {
            pcms.push(p);
        }
        if let Some(c) = capture.as_ref() {
            pcms.push(c);
        }
        let poll = PcmPoll::new(&pcms, Some(waker.clone()))?;
        let scale = |frames: u32| (f64::from(frames) * f64::from(config.rate) / f64::from(device_rate)).round() as u32;
        let edge_latency = edge.as_ref().map_or(0, Edge::latency);
        let info = DeviceInfo {
            name: config.name(),
            rate: config.rate,
            device_rate,
            period: negotiated,
            periods: play_config.or(cap_config).map_or(periods, |c| c.periods),
            buffer: play_config.or(cap_config).map_or(negotiated * periods, |c| c.buffer_frames),
            max_block: edge.as_ref().map_or(negotiated as usize, Edge::block),
            inputs,
            outputs,
            // A captured frame waits up to a period to be read; a written one waits the prefilled buffer
            input_latency: cap_config.map_or(0, |c| scale(c.period_frames)) + edge_latency / 2,
            output_latency: play_config.map_or(0, |c| scale(c.buffer_frames)) + edge_latency - edge_latency / 2,
            edge_resampler: device_rate != config.rate,
            drift_correction: drift,
            linked,
        };
        self.waker = Some(waker);
        self.open = Some(Opened { info: info.clone(), playback, capture, poll, linked, edge, play_config, cap_config });
        Ok(info)
    }

    fn start(&mut self, mut callback: CallbackHandle) -> Result<()> {
        if self.thread.is_some() {
            return Err(Error::new("io.device.running"));
        }
        let Some(opened) = self.open.take() else {
            return Err(Error::new("io.device.not_open"));
        };
        let thread = std::thread::Builder::new()
            .name("lm-audio-alsa".to_owned())
            .stack_size(4 << 20)
            .spawn(move || {
                lm_base::set_role(Role::Audio);
                let threads = callback.threads();
                let mut opened = opened;
                threads.run(ThreadKind::Audio, &mut || {
                    callback.report(DeviceEvent::Started { realtime: None });
                    let r = std::panic::catch_unwind(AssertUnwindSafe(|| run(&mut opened, &mut callback)));
                    if let Err(payload) = r {
                        lm_render::note_caught_panic(&*payload);
                        callback.report(DeviceEvent::Panicked);
                    }
                });
                if let Some(p) = opened.playback.as_mut() {
                    let _ = p.stop_now();
                }
                if let Some(c) = opened.capture.as_mut() {
                    let _ = c.stop_now();
                }
                // The PCMs close here, before the callback goes back: when `stop` returns, the card is free
                drop(opened);
                drop(callback);
            })
            .map_err(|e| Error::new("io.device.thread_failed").arg("detail", e.to_string()))?;
        self.thread = Some(thread);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(w) = &self.waker {
            w.wake();
        }
    }

    fn join(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    fn close(&mut self) {
        self.open = None;
        self.waker = None;
        // Hands the card back to its previous owner (U8)
        self.reservations.clear();
    }

    fn kind(&self) -> &'static str {
        "alsa"
    }
}

/// The loop's buffers: one locked buffer per channel (V6)
struct Buffers {
    inputs: Vec<RtBuffer>,
    outputs: Vec<RtBuffer>,
}

/// Why the loop ended
enum Exit {
    Stopped,
    Disconnected,
    /// The errno, and where it happened
    Failed(i32, &'static str),
}

fn io_errno(e: PcmIoError) -> i32 {
    match e {
        PcmIoError::Other(n) => n,
        other => -other.code(),
    }
}

/// The device thread's loop
fn run(o: &mut Opened, handle: &mut CallbackHandle) {
    let exit = if o.linked || o.capture.is_none() || o.playback.is_none() {
        run_locked(o, handle)
    } else {
        run_drift(o, handle)
    };
    match exit {
        Exit::Stopped => {}
        Exit::Disconnected => handle.report(DeviceEvent::Disconnected),
        Exit::Failed(errno, call) => handle.report(DeviceEvent::Failed { errno, call }),
    }
}

/// Prefills the playback with a full buffer of silence and starts (linked: both streams)
fn start_streams(o: &mut Opened) -> std::result::Result<(), PcmIoError> {
    start_streams_at(o).map_err(|(e, _)| e)
}

/// The same, saying which step failed
fn start_streams_at(o: &mut Opened) -> std::result::Result<(), (PcmIoError, &'static str)> {
    if let Some(p) = o.playback.as_mut() {
        let buffer = o.play_config.map_or(0, |c| c.buffer_frames as usize);
        let mut left = buffer;
        while left > 0 {
            let n = p.write_silence(left).map_err(|e| (e, "playback prefill"))?;
            if n == 0 {
                break;
            }
            left -= n;
        }
        p.start().map_err(|e| (e, "playback start"))?;
        if !o.linked
            && let Some(c) = o.capture.as_mut()
        {
            c.start().map_err(|e| (e, "capture start"))?;
        }
    } else if let Some(c) = o.capture.as_mut() {
        c.start().map_err(|e| (e, "capture start"))?;
    }
    Ok(())
}

/// After an xrun on either stream: what the capture lost (C2), prepared, prefilled and started again. Returns the
/// lost capture frames and the playback's lateness
fn recover(o: &mut Opened, err: PcmIoError) -> std::result::Result<(u32, u32), PcmIoError> {
    if err == PcmIoError::Disconnected {
        return Err(err);
    }
    let rate = o.cap_config.or(o.play_config).map_or(48_000, |c| c.rate);
    let xrun = match o.capture.as_mut() {
        Some(c) if c.state() == PcmState::Xrun => c.xrun_info().ok(),
        _ => None,
    };
    let late = match o.playback.as_mut() {
        Some(p) if p.state() == PcmState::Xrun => {
            p.xrun_info().map(|x| x.pending.min(u64::from(u32::MAX)) as u32).unwrap_or(1)
        }
        _ => 0,
    };
    if !o.linked && o.playback.is_some() && o.capture.is_some() {
        // Two cards (the drift correction): each stream is recovered alone, and only the one that stopped. Preparing
        // a running stream fails (EBUSY)
        if let Some(p) = o.playback.as_mut()
            && p.state() == PcmState::Xrun
        {
            match p.recover(PcmIoError::Xrun) {
                Ok(()) | Err(PcmIoError::BadState) => {}
                Err(e) => return Err(e),
            }
            let mut left = o.play_config.map_or(0, |c| c.buffer_frames as usize);
            while left > 0 {
                let n = p.write_silence(left)?;
                if n == 0 {
                    break;
                }
                left -= n;
            }
            p.start()?;
        }
        if let Some(c) = o.capture.as_mut()
            && c.state() == PcmState::Xrun
        {
            match c.recover(PcmIoError::Xrun) {
                Ok(()) | Err(PcmIoError::BadState) => {}
                Err(e) => return Err(e),
            }
            c.start()?;
        }
    } else {
        if let Some(p) = o.playback.as_mut() {
            match p.recover(err) {
                Ok(()) | Err(PcmIoError::BadState) => {}
                Err(e) => return Err(e),
            }
            let _ = p.prepare();
        }
        if let Some(c) = o.capture.as_mut() {
            if !o.linked {
                match c.recover(err) {
                    Ok(()) | Err(PcmIoError::BadState) => {}
                    Err(e) => return Err(e),
                }
            }
            let _ = c.prepare();
        }
        start_streams(o)?;
    }
    let lost = match (xrun, o.capture.as_mut()) {
        (Some(x), Some(c)) => {
            let restarted = c.trigger_time().unwrap_or_else(|_| HostTime::now());
            lost_capture_frames(x, restarted, rate).min(u64::from(u32::MAX)) as u32
        }
        _ => 0,
    };
    Ok((lost, late.max(u32::from(xrun.is_none() && late == 0))))
}

/// The stream's state after a poll error or a timeout: what to do
fn classify(pcm: &Pcm) -> Option<PcmIoError> {
    match pcm.state() {
        PcmState::Xrun => Some(PcmIoError::Xrun),
        PcmState::Suspended => Some(PcmIoError::Suspended),
        PcmState::Disconnected => Some(PcmIoError::Disconnected),
        PcmState::Setup | PcmState::Open => Some(PcmIoError::BadState),
        _ => None,
    }
}

/// Linked duplex, playback only or capture only: one period in, one render, one period out
fn run_locked(o: &mut Opened, handle: &mut CallbackHandle) -> Exit {
    let period = o.info.period as usize;
    let ch_in = o.cap_config.map_or(0, |c| c.channels as usize);
    let ch_out = o.play_config.map_or(0, |c| c.channels as usize);
    let rate = o.cap_config.or(o.play_config).map_or(48_000, |c| c.rate);
    let mut bufs = Buffers {
        inputs: (0..ch_in).map(|_| RtBuffer::zeroed(period)).collect(),
        outputs: (0..ch_out).map(|_| RtBuffer::zeroed(period)).collect(),
    };
    let period_time = Duration::from_nanos(period as u64 * 1_000_000_000 / u64::from(rate));
    let timeout = period_time * 4 + Duration::from_millis(100);
    if let Err((e, call)) = start_streams_at(o) {
        return if e == PcmIoError::Disconnected { Exit::Disconnected } else { Exit::Failed(io_errno(e), call) };
    }
    let mut lost: u32 = 0;
    let mut late: u32 = 0;
    let mut timeouts = 0u32;
    loop {
        if handle.should_stop() {
            return Exit::Stopped;
        }
        let outcome = {
            match (o.playback.as_ref(), o.capture.as_ref()) {
                (Some(p), Some(c)) => o.poll.wait(&[p, c], Some(timeout)),
                (Some(one), None) | (None, Some(one)) => o.poll.wait(&[one], Some(timeout)),
                // `open` refuses a config without a stream; this is only defensive
                (None, None) => return Exit::Failed(22, "no stream open"),
            }
        };
        if handle.should_stop() {
            return Exit::Stopped;
        }
        let problem = match outcome {
            Err(e) => Some(e),
            Ok(out) if out.is_empty() => {
                timeouts = timeouts.saturating_add(1);
                let state = o.playback.as_ref().and_then(classify).or_else(|| o.capture.as_ref().and_then(classify));
                if state.is_none() && timeouts.saturating_mul(timeout.as_millis() as u32) > 2_000 {
                    // Two seconds without a period: the device stalled
                    return Exit::Failed(110, "poll: no period for 2 s");
                }
                state
            }
            Ok(_) => {
                timeouts = 0;
                o.playback.as_ref().and_then(classify).or_else(|| o.capture.as_ref().and_then(classify))
            }
        };
        if let Some(err) = problem {
            match handle_problem(o, err, handle, &mut lost, &mut late) {
                Ok(()) => continue,
                Err(exit) => return exit,
            }
        }
        // Every whole period ready on both sides
        loop {
            let cap_avail = match o.capture.as_mut() {
                Some(c) => match c.avail_update() {
                    Ok(a) => a as usize,
                    Err(e) => {
                        if let Err(exit) = handle_problem(o, e, handle, &mut lost, &mut late) {
                            return exit;
                        }
                        break;
                    }
                },
                None => usize::MAX,
            };
            let play_avail = match o.playback.as_mut() {
                Some(p) => match p.avail_update() {
                    Ok(a) => a as usize,
                    Err(e) => {
                        if let Err(exit) = handle_problem(o, e, handle, &mut lost, &mut late) {
                            return exit;
                        }
                        break;
                    }
                },
                None => usize::MAX,
            };
            if cap_avail < period || play_avail < period {
                break;
            }
            // C6: the host time of the block's first sample. Capture: it was captured `avail` frames before the
            // stamp. Playback only: the moment the room for it appeared
            let stamp = match o.capture.as_mut().or(o.playback.as_mut()) {
                Some(pcm) => pcm.htimestamp().ok(),
                None => None,
            };
            let host_time = match stamp {
                Some((avail, t)) if t.0 > 0 => HostTime(t.0.saturating_sub(avail * 1_000_000_000 / u64::from(rate))),
                _ => HostTime::now(),
            };
            if let Some(c) = o.capture.as_mut() {
                let mut got = 0;
                while got < period {
                    let r = {
                        let mut dst: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
                        for (d, b) in dst.iter_mut().zip(bufs.inputs.iter_mut()) {
                            *d = &mut b[got..period];
                        }
                        c.read_planar(&mut dst[..ch_in])
                    };
                    match r {
                        Ok(0) => break,
                        Ok(n) => got += n,
                        Err(e) => {
                            if let Err(exit) = handle_problem(o, e, handle, &mut lost, &mut late) {
                                return exit;
                            }
                            break;
                        }
                    }
                }
                if got < period {
                    // An xrun in the middle of the read: the rest of the period is part of the lost frames
                    continue;
                }
            }
            if let Some(edge) = o.edge.as_mut() {
                let mut src: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                for (s, b) in src.iter_mut().zip(bufs.inputs.iter()) {
                    *s = &b[..period];
                }
                if lost > 0 {
                    edge.add_capture_lost(std::mem::take(&mut lost));
                }
                if late > 0 {
                    edge.add_late(std::mem::take(&mut late));
                }
                edge.push_capture(&src[..ch_in], period);
                let mut dst: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
                for (d, b) in dst.iter_mut().zip(bufs.outputs.iter_mut()) {
                    *d = &mut b[..period];
                }
                edge.period(host_time, &mut dst[..ch_out], period, &mut |b| handle.call(b));
            } else {
                let mut src: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                for (s, b) in src.iter_mut().zip(bufs.inputs.iter()) {
                    *s = &b[..period];
                }
                let mut dst: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
                for (d, b) in dst.iter_mut().zip(bufs.outputs.iter_mut()) {
                    *d = &mut b[..period];
                }
                handle.call(DeviceBlock {
                    frames: period,
                    host_time,
                    inputs: &src[..ch_in],
                    outputs: &mut dst[..ch_out],
                    capture_lost: std::mem::take(&mut lost),
                    late_by: std::mem::take(&mut late),
                });
            }
            if let Some(p) = o.playback.as_mut() {
                let mut put = 0;
                while put < period {
                    let r = {
                        let mut src: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                        for (s, b) in src.iter_mut().zip(bufs.outputs.iter()) {
                            *s = &b[put..period];
                        }
                        p.write_planar(&src[..ch_out])
                    };
                    match r {
                        Ok(0) => break,
                        Ok(n) => put += n,
                        Err(e) => {
                            if let Err(exit) = handle_problem(o, e, handle, &mut lost, &mut late) {
                                return exit;
                            }
                            break;
                        }
                    }
                }
            }
            if handle.should_stop() {
                return Exit::Stopped;
            }
        }
    }
}

/// An error of a transfer or a state found by poll: recover (xrun, suspend) or end (removed card)
fn handle_problem(
    o: &mut Opened,
    err: PcmIoError,
    handle: &mut CallbackHandle,
    lost: &mut u32,
    late: &mut u32,
) -> std::result::Result<(), Exit> {
    match err {
        PcmIoError::Disconnected => Err(Exit::Disconnected),
        PcmIoError::Suspended => {
            // One non-blocking resume per poll round; a device still waking up says so and is tried again
            let mut resumed = true;
            for pcm in [o.playback.as_mut(), o.capture.as_mut()].into_iter().flatten() {
                match pcm.recover(PcmIoError::Suspended) {
                    Ok(()) => {}
                    Err(PcmIoError::Suspended) => resumed = false,
                    Err(PcmIoError::Disconnected) => return Err(Exit::Disconnected),
                    Err(_) => {}
                }
            }
            if resumed {
                match start_streams(o) {
                    Ok(()) => handle.report(DeviceEvent::Resumed),
                    Err(PcmIoError::Disconnected) => return Err(Exit::Disconnected),
                    Err(_) => {}
                }
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        }
        PcmIoError::Xrun | PcmIoError::BadState => match recover(o, PcmIoError::Xrun) {
            Ok((l, t)) => {
                *lost = lost.saturating_add(l);
                *late = late.saturating_add(t);
                handle.report(DeviceEvent::Xrun { capture_lost: l, late_by: t });
                Ok(())
            }
            Err(PcmIoError::Disconnected) => Err(Exit::Disconnected),
            Err(e) => Err(Exit::Failed(io_errno(e), "recover after an xrun")),
        },
        PcmIoError::Other(n) => {
            // ENODEV can come as a plain errno from a call that does not map it
            if n == 19 || n == 108 { Err(Exit::Disconnected) } else { Err(Exit::Failed(n, "transfer")) }
        }
    }
}

/// Capture and playback on different cards, with the drift correction (U7): the playback is the clock, the capture
/// is read as it comes into the edge's adaptive resampler
fn run_drift(o: &mut Opened, handle: &mut CallbackHandle) -> Exit {
    let period = o.info.period as usize;
    let cap_period = o.cap_config.map_or(period, |c| c.period_frames as usize);
    let ch_in = o.cap_config.map_or(0, |c| c.channels as usize);
    let ch_out = o.play_config.map_or(0, |c| c.channels as usize);
    let rate = o.play_config.map_or(48_000, |c| c.rate);
    let chunk = cap_period.max(period) * 2;
    let mut inputs: Vec<RtBuffer> = (0..ch_in).map(|_| RtBuffer::zeroed(chunk)).collect();
    let mut outputs: Vec<RtBuffer> = (0..ch_out).map(|_| RtBuffer::zeroed(period)).collect();
    let timeout = Duration::from_nanos(period as u64 * 4_000_000_000 / u64::from(rate)) + Duration::from_millis(100);
    if let Err((e, call)) = start_streams_at(o) {
        return if e == PcmIoError::Disconnected { Exit::Disconnected } else { Exit::Failed(io_errno(e), call) };
    }
    let mut lost = 0u32;
    let mut late = 0u32;
    loop {
        if handle.should_stop() {
            return Exit::Stopped;
        }
        let outcome = match (o.playback.as_ref(), o.capture.as_ref()) {
            (Some(p), Some(c)) => o.poll.wait(&[p, c], Some(timeout)),
            _ => return Exit::Failed(22, "drift: needs both streams"),
        };
        if handle.should_stop() {
            return Exit::Stopped;
        }
        if let Err(e) = outcome {
            if let Err(exit) = handle_problem(o, e, handle, &mut lost, &mut late) {
                return exit;
            }
            continue;
        }
        if let Some(err) = o.playback.as_ref().and_then(classify) {
            if let Err(exit) = handle_problem(o, err, handle, &mut lost, &mut late) {
                return exit;
            }
            continue;
        }
        // The capture: everything available, into the edge
        if let Some(c) = o.capture.as_mut() {
            if c.state() == PcmState::Xrun {
                let x = c.xrun_info().ok();
                let _ = c.recover(PcmIoError::Xrun);
                let _ = c.start();
                if let Some(x) = x {
                    let restarted = c.trigger_time().unwrap_or_else(|_| HostTime::now());
                    let l = lost_capture_frames(x, restarted, o.cap_config.map_or(rate, |k| k.rate))
                        .min(u64::from(u32::MAX)) as u32;
                    if let Some(edge) = o.edge.as_mut() {
                        edge.add_capture_lost(l);
                    }
                    handle.report(DeviceEvent::Xrun { capture_lost: l, late_by: 0 });
                }
            } else if c.state() == PcmState::Disconnected {
                return Exit::Disconnected;
            }
            loop {
                let r = {
                    let mut dst: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
                    for (d, b) in dst.iter_mut().zip(inputs.iter_mut()) {
                        *d = &mut b[..chunk];
                    }
                    c.read_planar(&mut dst[..ch_in])
                };
                match r {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Some(edge) = o.edge.as_mut() {
                            let mut src: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                            for (s, b) in src.iter_mut().zip(inputs.iter()) {
                                *s = &b[..n];
                            }
                            edge.push_capture(&src[..ch_in], n);
                        }
                    }
                    Err(PcmIoError::Disconnected) => return Exit::Disconnected,
                    // The capture's xrun is handled at the next round
                    Err(_) => break,
                }
            }
        }
        // The playback: every whole period of room
        loop {
            let avail = match o.playback.as_mut().map(Pcm::avail_update) {
                Some(Ok(a)) => a as usize,
                Some(Err(e)) => {
                    if let Err(exit) = handle_problem(o, e, handle, &mut lost, &mut late) {
                        return exit;
                    }
                    break;
                }
                None => 0,
            };
            if avail < period {
                break;
            }
            let host_time = match o.playback.as_mut().map(Pcm::htimestamp) {
                Some(Ok((a, t))) if t.0 > 0 => HostTime(t.0.saturating_sub(a * 1_000_000_000 / u64::from(rate))),
                _ => HostTime::now(),
            };
            if let Some(edge) = o.edge.as_mut() {
                if late > 0 {
                    edge.add_late(std::mem::take(&mut late));
                }
                let mut dst: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
                for (d, b) in dst.iter_mut().zip(outputs.iter_mut()) {
                    *d = &mut b[..period];
                }
                edge.period(host_time, &mut dst[..ch_out], period, &mut |b| handle.call(b));
            }
            lost = 0;
            if let Some(p) = o.playback.as_mut() {
                let mut src: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                for (s, b) in src.iter_mut().zip(outputs.iter()) {
                    *s = &b[..period];
                }
                if let Err(e) = p.write_planar(&src[..ch_out])
                    && let Err(exit) = handle_problem(o, e, handle, &mut lost, &mut late)
                {
                    return exit;
                }
            }
        }
    }
}

/// Lists the devices (U17): every card's PCM devices, probed. Enumerating can take time with USB devices: run it
/// in an `Operation` on the target `Device`'s list. Cancellable between cards
pub fn list_devices(cancel: &lm_base::Cancel, progress: &lm_base::Progress) -> Result<Vec<DeviceChoice>> {
    let indices = enumerate::card_indices()?;
    let mut out = Vec::new();
    for (k, index) in indices.iter().enumerate() {
        cancel.check()?;
        progress.set(k as f32 / indices.len().max(1) as f32);
        let Ok(card) = enumerate::card(*index, true) else { continue };
        for d in card.devices {
            let caps = |s: &Option<enumerate::StreamInfo>| match s.as_ref().map(|s| &s.probe) {
                Some(Probe::Capabilities(c)) => Some(ChoiceCaps {
                    channels_min: c.channels_min,
                    channels_max: c.channels_max,
                    rates: if c.rates.is_empty() {
                        FALLBACK_RATES.iter().copied().filter(|r| *r >= c.rate_min && *r <= c.rate_max).collect()
                    } else {
                        c.rates.clone()
                    },
                }),
                _ => None,
            };
            let state = |p: &Probe| (matches!(p, Probe::Busy), matches!(p, Probe::Denied));
            let (pb, pd) = d.playback.as_ref().map_or((false, false), |s| state(&s.probe));
            let (cb, cd) = d.capture.as_ref().map_or((false, false), |s| state(&s.probe));
            out.push(DeviceChoice {
                name: d.hw_name.clone(),
                card: card.name.clone(),
                description: format!("{} — {}", card.name, d.name),
                playback: caps(&d.playback),
                capture: caps(&d.capture),
                busy: pb || cb,
                denied: pd || cd,
            });
        }
    }
    progress.set(1.0);
    Ok(out)
}

/// Whether a card is present (U11: the periodic scan that offers to reconnect)
pub fn device_present(name: &str) -> bool {
    match card_index(name) {
        Some(i) => enumerate::card_indices().map(|v| v.contains(&i)).unwrap_or(false),
        None => false,
    }
}
