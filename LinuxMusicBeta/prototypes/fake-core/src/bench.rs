//! The measurement harness shared by the three prototypes, so the numbers come from the same code.
//!
//! `LM_BENCH=scroll|play|idle` turns it on; `LM_BENCH_SECS` (default 15) is the measured time after 2 s of warm-up;
//! `LM_BENCH_OUT` (default `results`) is where `<lib>-<mode>.json` goes. Each library calls `frame()` once per drawn
//! frame (from its own "a frame was presented / is being built" hook) and `tick()` from the 10 ms pump timer, and quits
//! when either says so.
//! - `scroll`: transport playing (all meters and the cursor move), and the view scrolls and zooms by a fixed script of
//!   time: horizontal scroll, zoom from 256 to 8192 samples per pixel, vertical scroll over the 64 tracks. Every frame
//!   redraws every visible waveform.
//! - `play`: transport playing, the view still (meters at 60 Hz, cursor moving).
//! - `idle`: transport stopped, nothing moves.
//!
//! CPU and memory are read from outside (`measure.sh`), so the harness costs nothing there.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Scroll,
    Play,
    Idle,
}

#[derive(Debug)]
pub struct Bench {
    pub mode: Mode,
    lib: &'static str,
    start: Instant,
    warmup: Duration,
    length: Duration,
    frames: Vec<Instant>,
    /// CPU time of each frame, from the start of building it to the end of presenting it, when the library says
    work: Rc<RefCell<Vec<f64>>>,
    done: bool,
}

/// What the script asks the view to show at a moment
#[derive(Debug, Clone, Copy)]
pub struct ScriptedView {
    /// first visible sample at the left edge of the timeline
    pub scroll_samples: f64,
    pub samples_per_px: f64,
    /// 0..1 of the vertical scroll range
    pub scroll_y: f64,
}

impl Bench {
    pub fn from_env(lib: &'static str) -> Bench {
        let mode = match std::env::var("LM_BENCH").as_deref() {
            Ok("scroll") => Mode::Scroll,
            Ok("play") => Mode::Play,
            Ok("idle") => Mode::Idle,
            _ => Mode::Off,
        };
        let secs = std::env::var("LM_BENCH_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(15.0);
        Bench {
            mode,
            lib,
            start: Instant::now(),
            warmup: Duration::from_secs(2),
            length: Duration::from_secs_f64(secs),
            frames: Vec::with_capacity(8192),
            work: Rc::new(RefCell::new(Vec::with_capacity(8192))),
            done: false,
        }
    }

    pub fn active(&self) -> bool {
        self.mode != Mode::Off
    }

    /// Whether the transport should be playing for this mode
    pub fn wants_play(&self) -> bool {
        matches!(self.mode, Mode::Scroll | Mode::Play)
    }

    /// The view the scroll script wants now (None outside `scroll`)
    pub fn view(&self, now: Instant) -> Option<ScriptedView> {
        if self.mode != Mode::Scroll {
            return None;
        }
        let t = now.duration_since(self.start).as_secs_f64();
        let tau = std::f64::consts::TAU;
        let zoom = 8.0 + 5.0 * (0.5 - 0.5 * (tau * t / 6.0).cos()); // 2^8 .. 2^13 samples per pixel
        Some(ScriptedView {
            scroll_samples: (t * 48_000.0 * 6.0) % (150.0 * 48_000.0),
            samples_per_px: 2f64.powf(zoom),
            scroll_y: 0.5 - 0.5 * (tau * t / 9.0).cos(),
        })
    }

    /// Call once per frame. Returns true when the run is over and the app should quit
    pub fn frame(&mut self) -> bool {
        if self.mode == Mode::Off || self.done {
            return self.done;
        }
        let now = Instant::now();
        if now.duration_since(self.start) >= self.warmup {
            self.frames.push(now);
        }
        self.check(now)
    }

    /// A probe for the CPU-side time of one frame: call it with the frame's start from the last thing the library
    /// paints (build, layout, prepaint and paint on the CPU; not the GPU)
    pub fn work_probe(&self) -> impl Fn(Instant) + 'static {
        let sink = self.work.clone();
        let (on, start, warmup) = (self.mode != Mode::Off, self.start, self.warmup);
        move |started: Instant| {
            if on && started.duration_since(start) >= warmup {
                sink.borrow_mut().push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }

    /// Call from the pump timer: ends `idle` and `play` runs, which may draw few frames
    pub fn tick(&mut self) -> bool {
        if self.mode == Mode::Off || self.done {
            return self.done;
        }
        self.check(Instant::now())
    }

    fn check(&mut self, now: Instant) -> bool {
        if now.duration_since(self.start) < self.warmup + self.length {
            return false;
        }
        self.done = true;
        self.write();
        true
    }

    fn write(&self) {
        let mut iv: Vec<f64> =
            self.frames.windows(2).map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1000.0).collect();
        iv.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |p: f64| if iv.is_empty() { 0.0 } else { iv[((iv.len() - 1) as f64 * p).round() as usize] };
        let mean = if iv.is_empty() { 0.0 } else { iv.iter().sum::<f64>() / iv.len() as f64 };
        // a frame "missed" when it took more than 1.5 display periods (120 Hz here: 8.33 ms)
        let period = std::env::var("LM_DISPLAY_HZ").ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or(120.0);
        let missed = iv.iter().filter(|&&x| x > 1.5 * 1000.0 / period).count();
        let mut work = self.work.borrow().clone();
        work.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let wpct = |p: f64| if work.is_empty() { 0.0 } else { work[((work.len() - 1) as f64 * p).round() as usize] };
        let mode = match self.mode {
            Mode::Scroll => "scroll",
            Mode::Play => "play",
            Mode::Idle => "idle",
            Mode::Off => "off",
        };
        let json = format!(
            "{{\"lib\":\"{}\",\"mode\":\"{}\",\"frames\":{},\"seconds\":{:.2},\"fps\":{:.1},\
             \"interval_ms\":{{\"median\":{:.2},\"p95\":{:.2},\"p99\":{:.2},\"max\":{:.2},\"mean\":{:.2}}},\
             \"missed_over_1_5_periods\":{},\"work_ms\":{{\"median\":{:.2},\"p99\":{:.2},\"max\":{:.2}}}}}\n",
            self.lib,
            mode,
            self.frames.len(),
            self.length.as_secs_f64(),
            self.frames.len() as f64 / self.length.as_secs_f64(),
            pct(0.5),
            pct(0.95),
            pct(0.99),
            iv.last().copied().unwrap_or(0.0),
            mean,
            missed,
            wpct(0.5),
            wpct(0.99),
            work.last().copied().unwrap_or(0.0)
        );
        let dir = std::env::var("LM_BENCH_OUT").unwrap_or_else(|_| "results".into());
        let _ = std::fs::create_dir_all(&dir);
        let path = format!("{dir}/{}-{mode}.json", self.lib);
        if let Err(e) = std::fs::write(&path, &json) {
            eprintln!("bench: cannot write {path}: {e}");
        }
        eprint!("bench: {json}");
    }
}
