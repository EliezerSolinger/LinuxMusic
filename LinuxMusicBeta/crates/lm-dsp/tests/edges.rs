//! Every index lm-dsp works out from a float, driven to its edges: a phase a hair below a whole period, a delay at
//! its longest and just above a whole number of frames, a knob past its range, a cursor at a sample's last frame.
//! None may read outside its buffer (a panic on the audio thread), and none may turn into NaN where the input was
//! finite.

use lm_dsp::compressor::{CompressorPlugin, CompressorPluginSettings};
use lm_dsp::delay::{Delay, DelaySettings, MAX_TIME, StereoDelay};
use lm_dsp::distortion_effects::{default_wave_shape, wave_shaper};
use lm_dsp::dynamics_processor::{DynamicsProcessor, DynamicsProcessorSettings};
use lm_dsp::frequency_shifter::{FrequencyShifter, FrequencyShifterSettings};
use lm_dsp::modulation_effects::{Flanger, FlangerSettings, MonoDelay, MultitapEcho, MultitapSettings};
use lm_dsp::oscillator::user_wave_sample;
use lm_dsp::reverb_sc::{ReverbSc, ReverbScSettings};
use lm_dsp::sample_player::{Bounds, Cursor, Interpolator, Loop, PlaybackState, Sample};
use lm_dsp::small_effects::{Ramp, StereoEnhancer};

/// The largest f32 below 1
const ONE_MINUS: f32 = 1.0 - f32::EPSILON / 2.0;

fn noise(frames: usize, seed: u32) -> Vec<[f32; 2]> {
    let mut x = seed.max(1);
    (0..frames)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let v = (x as i32) as f32 / i32::MAX as f32 * 0.5;
            [v, -v]
        })
        .collect()
}

fn all_finite(frames: &[[f32; 2]]) -> bool {
    frames.iter().all(|f| f[0].is_finite() && f[1].is_finite())
}

/// The delay of the frequency shifter at a length a hair above a whole number of frames: the read position a hair
/// below 0 rounds to exactly the ring's length in single precision (the oscillator's bug, in a ring)
#[test]
fn frequency_shifter_delay_just_above_whole_frames_and_at_its_longest() {
    let sr = 48_000.0_f32;
    // A delay (in ms) whose length in frames is 100 plus less than half the spacing of f32 at the ring's length
    let mut delay_ms = 100.0 / sr * 1000.0;
    let len = |ms: f32| (ms + 0.0) * 0.001 * sr;
    while !(len(delay_ms) > 100.0 && len(delay_ms) - 100.0 < 0.003) {
        delay_ms = f32::from_bits(delay_ms.to_bits() + 1);
    }
    for (delay, fine) in [(delay_ms, 0.0), (2000.0, 20.0), (0.0, 0.0)] {
        let mut shifter = FrequencyShifter::new(sr);
        let s = FrequencyShifterSettings {
            delay,
            fine_delay: fine,
            delay_glide: 0.0,
            feedback: 0.5,
            shift: 100.0,
            ..FrequencyShifterSettings::default()
        };
        // Past the ring's length (2.02 s) twice, so the write position passes the delay again and again
        let mut frames = noise(48_000 * 5, 7);
        for block in frames.chunks_mut(256) {
            shifter.process(block, &s);
        }
        assert!(all_finite(&frames), "delay {delay} ms");
    }
}

#[test]
fn stereo_enhancer_at_every_width() {
    for width in [0.0, 0.5, ONE_MINUS, 179.999, 180.0, 255.0, 255.9, 256.0, 300.0, 1000.0, -0.5, -300.0, f32::NAN] {
        let mut e = StereoEnhancer::default();
        let mut frames = noise(1000, 3);
        e.process(&mut frames, width);
        if width.is_finite() {
            assert!(all_finite(&frames), "width {width}");
        }
    }
    // Within its range the right side is the input delayed by the width (truncated)
    let mut e = StereoEnhancer::default();
    let mut frames: Vec<[f32; 2]> = (0..300).map(|n| [0.0, n as f32]).collect();
    e.process(&mut frames, 180.0);
    // l = 0 + r_delayed * sin(90°) = r_delayed; at frame 200 the right input 180 frames back is 20
    assert!((frames[200][0] - 20.0).abs() < 1e-4, "{:?}", frames[200]);
}

#[test]
fn compressor_lookahead_at_and_beyond_its_longest() {
    for sr in [8_000.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for lookahead_ms in [0.0, 0.001, 19.999, 20.0, 25.0, 1000.0, f32::INFINITY] {
            let mut c = CompressorPlugin::new(sr);
            let s = CompressorPluginSettings {
                lookahead: true,
                lookahead_length: lookahead_ms,
                ..CompressorPluginSettings::default()
            };
            let mut frames = noise(4096, 11);
            c.process(&mut frames, &s);
            assert!(all_finite(&frames), "{sr} Hz, lookahead {lookahead_ms} ms");
        }
    }
}

#[test]
fn delay_at_its_longest_and_swept_past_it() {
    let sr = 1_000.0;
    // The line at its full length: an impulse comes back after exactly MAX_TIME
    let mut line = StereoDelay::new(sr);
    line.set_length(MAX_TIME * sr);
    let total = (MAX_TIME * sr) as usize + 10;
    let mut out = vec![[0.0f32; 2]; total];
    for (n, frame) in out.iter_mut().enumerate() {
        *frame = if n == 0 { [1.0, 1.0] } else { [0.0, 0.0] };
        line.tick(frame);
    }
    let at = out.iter().position(|f| f[0] == 1.0).unwrap();
    assert_eq!(at, (MAX_TIME * sr) as usize);
    // Lengths beyond the line, negative or NaN are ignored (the line keeps its last length)
    for length in [MAX_TIME * sr + 1.0, -1.0, f32::NAN, f32::INFINITY] {
        line.set_length(length);
        let mut f = [0.5, 0.5];
        line.tick(&mut f);
    }
    // The effect: the longest time with the LFO sweeping beyond the line's end and before its start
    let mut delay = Delay::new(sr);
    let s = DelaySettings { time: MAX_TIME, feedback: 0.9, lfo_time: 0.01, lfo_amount: 0.5, out_gain: 0.0 };
    let mut frames = noise(60_000, 5);
    for block in frames.chunks_mut(128) {
        delay.process(block, &s);
    }
    assert!(all_finite(&frames));
    let s = DelaySettings { time: 0.0, lfo_amount: 0.5, lfo_time: 0.01, ..s };
    delay.process(&mut frames, &s);
    assert!(all_finite(&frames));
}

#[test]
fn flanger_and_mono_delay_at_their_longest() {
    let sr = 44_100.0;
    let mut d = MonoDelay::new(1.0, sr);
    d.set_length(sr);
    let mut x = 1.0;
    d.tick(&mut x);
    for _ in 0..(sr as usize) * 3 {
        let mut x = 0.25;
        d.tick(&mut x);
    }
    // The knobs' tops: the delay plus the whole sweep reach past the 1 s line (ignored lengths)
    let mut f = Flanger::new(sr, 1);
    let s = FlangerSettings {
        delay: 0.05,
        lfo_period: 0.01,
        lfo_amount: 0.0025,
        feedback: 1.0,
        noise: 0.05,
        ..FlangerSettings::default()
    };
    let mut frames = noise(100_000, 9);
    for block in frames.chunks_mut(64) {
        f.process(block, &s);
    }
    assert!(frames.iter().all(|f| !f[0].is_nan() && !f[1].is_nan()));
}

#[test]
fn multitap_at_its_longest_and_with_odd_blocks() {
    let period = 256;
    let mut echo = MultitapEcho::new(48_000.0, period);
    // 32 steps of 500 ms: 16 s, the ring's length
    let s = MultitapSettings { steps: 32, step_length: 500.0, stages: 4, ..MultitapSettings::default() };
    for block in 0..4000 {
        let mut frames = noise(period, block + 1);
        echo.process(&mut frames, &s);
        assert!(all_finite(&frames));
    }
    let mut frames = noise(period, 1);
    // A step length that is not finite, and a block shorter than the period
    let s = MultitapSettings { step_length: f32::INFINITY, ..s };
    echo.process(&mut frames, &s);
    let mut short = noise(period / 2, 2);
    echo.process(&mut short, &MultitapSettings::default());
}

#[test]
fn reverb_at_every_rate_for_a_long_time() {
    for sr in [8_000.0, 22_050.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let mut r = ReverbSc::new(sr);
        let s = ReverbScSettings { size: 1.0, color: 15_000.0, ..ReverbScSettings::default() };
        // Long enough for every line's random segments to wrap their reads around many times
        let mut frames = noise((sr as usize) * 4, 13);
        for block in frames.chunks_mut(512) {
            r.process(block, &s);
        }
        assert!(all_finite(&frames), "{sr} Hz");
    }
}

#[test]
fn shaping_curves_at_their_ends() {
    let shape = default_wave_shape();
    let edges = [0.0, 0.004_999, 0.005, ONE_MINUS / 200.0 * 199.0, 199.0 / 200.0, 0.999_999, ONE_MINUS, 1.0, 1.5, 1e30];
    let mut frames: Vec<[f32; 2]> = edges.iter().map(|&v| [v, -v]).collect();
    wave_shaper(&mut frames, &shape, Ramp::constant(1.0), Ramp::constant(1.0), false);
    assert!(all_finite(&frames));
    // The straight line gives the input back (to within its 200 points)
    for (f, &v) in frames.iter().zip(&edges).take(8) {
        assert!((f[0] - v).abs() < 1e-5 && (f[1] + v).abs() < 1e-5, "{v}: {f:?}");
    }
    let mut frames = vec![[f32::NAN, f32::INFINITY], [f32::NEG_INFINITY, 0.0]];
    wave_shaper(&mut frames, &shape, Ramp::constant(1.0), Ramp::constant(1.0), true);

    let mut p = DynamicsProcessor::new(48_000.0);
    let s = DynamicsProcessorSettings::default();
    let mut frames: Vec<[f32; 2]> = (0..20_000)
        .map(|n| {
            let v = edges[n % edges.len()].min(1e3);
            [v, -v]
        })
        .collect();
    p.process(&mut frames, &s);
    assert!(all_finite(&frames));
}

#[test]
fn a_user_wave_at_a_phase_just_below_a_period() {
    let wave = [0.0, 1.0, 0.0, -1.0];
    for phase in [ONE_MINUS, -f32::EPSILON / 4.0, -1e-9, 1.0, 0.0, 3.0 - 1e-7, -0.25] {
        let v = user_wave_sample(&wave, phase);
        assert!(v.is_finite() && v.abs() <= 1.0, "{phase}: {v}");
    }
    // A hair below a period is the period's start (the wave's first point), not an index past the end
    assert!(user_wave_sample(&wave, -1e-9).abs() < 1e-3);
}

/// A plain linear interpolator: consumes the input at `ratio` frames per output frame, as the real one does
struct Linear {
    pos: f64,
}
impl Interpolator for Linear {
    fn process(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]], ratio: f64) -> (usize, usize) {
        let step = 1.0 / ratio;
        let mut made = 0;
        while made < output.len() {
            let i = self.pos.floor() as usize;
            if i + 1 >= input.len() {
                break;
            }
            let t = (self.pos - i as f64) as f32;
            output[made] =
                [input[i][0] + (input[i + 1][0] - input[i][0]) * t, input[i][1] + (input[i + 1][1] - input[i][1]) * t];
            made += 1;
            self.pos += step;
        }
        let used = (self.pos.floor() as usize).min(input.len().saturating_sub(1));
        self.pos -= used as f64;
        (used, made)
    }
    fn reset(&mut self) {
        self.pos = 0.0;
    }
}

fn sample(frames: &[[f32; 2]], bounds: Bounds, declick: bool, reversed: bool) -> Sample<'_> {
    Sample {
        frames,
        sample_rate: 48_000.0,
        bounds,
        amplification: 1.0,
        frequency: lm_dsp::sample_player::DEFAULT_BASE_FREQ,
        reversed,
        declick,
    }
}

#[test]
fn a_sample_played_to_its_last_frame_at_any_pitch() {
    let frames: Vec<[f32; 2]> = (0..1000).map(|n| [n as f32, -(n as f32)]).collect();
    let whole = Bounds { start: 0, end: 1000, loop_start: 0, loop_end: 1000 };
    // Ratios whose steps land exactly on the last frame (1000 = 8 x 125, 4 x 250...) and ones that do not
    for ratio in [0.125, 0.25, 0.5, 2.0, 4.0, 8.0, 1.0 / 3.0, 3.0, 1.000_001, 0.999_999] {
        for looping in [Loop::Off, Loop::On, Loop::PingPong] {
            for (declick, reversed) in [(false, false), (true, false), (true, true)] {
                let s = sample(&frames, whole, declick, reversed);
                let mut state = PlaybackState::new(Linear { pos: 0.0 }, 0);
                let mut out = vec![[0.0f32; 2]; 333];
                for _ in 0..40 {
                    s.play(&mut out, &mut state, looping, ratio, 48_000.0, 256);
                    assert!(all_finite(&out));
                }
            }
        }
    }
}

#[test]
fn a_sample_rendered_with_bounds_and_cursors_outside_it() {
    let frames: Vec<[f32; 2]> = (0..100).map(|n| [n as f32, 0.0]).collect();
    let wild = [
        Bounds { start: -50, end: 500, loop_start: -10, loop_end: 300 },
        Bounds { start: 90, end: 10, loop_start: 80, loop_end: 20 },
        Bounds { start: 100, end: 100, loop_start: 100, loop_end: 100 },
        Bounds { start: 0, end: 100, loop_start: 99, loop_end: 100 },
        Bounds { start: i32::MIN, end: i32::MAX, loop_start: i32::MIN, loop_end: i32::MAX },
    ];
    for b in wild {
        for looping in [Loop::Off, Loop::On, Loop::PingPong] {
            for index in [-1000, -1, 0, 50, 99, 100, 101, 5000] {
                for backwards in [false, true] {
                    for (declick, reversed) in [(false, false), (true, false), (true, true)] {
                        let s = sample(&frames, b, declick, reversed);
                        let mut cursor = Cursor { frame_index: index, backwards, wrapped: false };
                        let mut out = vec![[0.0f32; 2]; 300];
                        let n = s.render(&mut out, &mut cursor, looping, &b);
                        assert!(n <= out.len());
                        let mut state = PlaybackState::new(Linear { pos: 0.0 }, index);
                        s.play(&mut out, &mut state, looping, 1.0, 48_000.0, 256);
                        s.play(&mut out, &mut state, looping, 0.37, 48_000.0, 256);
                    }
                }
            }
        }
    }
    assert_eq!(
        Bounds { start: -5, end: 500, loop_start: 60, loop_end: 40 }.within(100),
        Bounds { start: 0, end: 100, loop_start: 60, loop_end: 60 }
    );
}
