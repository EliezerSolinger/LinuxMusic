//! AudioFileProcessor (F-INS-001: legacy `plugins/AudioFileProcessor`): a sample played at the note's pitch (A4 as
//! recorded), between a start and an end point, looping from a loop point (on or ping-pong), reversed, amplified,
//! with microfades, through an interpolator; "stutter" starts each note where the last one stopped.
//!
//! State, version 1: parameters `amplify` (0–500 %, 100), `startPoint` (0–1, 0), `endPoint` (0–1, 1), `loopPoint`
//! (0–1, 0) — fractions of the sample —, the switches `reverse` (off), `stutter` (off) and `declick` (on); fields
//! `sample` (an audio resource, none), `loop` (`off`, `on`, `pingPong`; `off`) and `interpolation` (`none`, `linear`,
//! `sinc`; `linear`); and the instrument functions.
//!
//! The points are put in order whatever the parameters say, as the legacy `pointChanged`: the start before the end,
//! at least a frame apart, the loop point between them. (The legacy knobs also pushed each other while dragged: that
//! is the panel's gesture, not the plugin's.) A change of a point, of `reverse` or of `stutter` starts the next note
//! at the start point again. With stutter on, a note below 20 Hz plays nothing and sends the next note back to the
//! start (the legacy "magic key").
//!
//! The interpolation of a note is chosen when it starts (the legacy made its converter then). `none` and `linear` are
//! libsamplerate's converters ported sample for sample ([`crate::src_port`]); `sinc` is the library's medium sinc.
//! A sinc converter cannot be copied without allocating, so an instrument playing sinc notes is not adopted by a
//! changed one: its notes end on the old instrument (U32).

use std::cell::Cell;
use std::sync::{Arc, LazyLock};

use lm_base::Result;
use lm_dsp::sample_player::{Bounds, DEFAULT_BASE_FREQ, Interpolator, Loop, PlaybackState, Sample};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};
use lm_sys_samplerate::{Quality, Resampler};

use crate::src_port::{Converter, Kind};
use crate::stereo_out;

pub const ID: &str = "audiofileprocessor";
pub const VOICES: usize = 32;
/// The parameters, in this order
pub const KEYS: [&str; 7] = ["amplify", "startPoint", "endPoint", "loopPoint", "reverse", "stutter", "declick"];
pub const LOOP_KEYS: [&str; 3] = ["off", "on", "pingPong"];
pub const INTERPOLATION_KEYS: [&str; 3] = ["none", "linear", "sinc"];
static LOOPS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| LOOP_KEYS.into());
static INTERPOLATIONS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| INTERPOLATION_KEYS.into());
/// The release, ms (legacy `desiredReleaseTimeMs`)
pub const RELEASE_MS: f32 = 3.0;
/// Below this pitch, with stutter on, a note only sends the next one back to the start (Hz)
pub const MAGIC_FREQ: f32 = 20.0;

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.audiofileprocessor.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: a sample's pitch is whatever it holds; its playback ratio is tested against the legacy note)
    // The instrument functions' fields change voices while they sound, and a linear converter copies; an
    // interpolation change to sinc needs converters the old voices lack
    tests: TestHints { voices: VOICES, no_adopt: &["interpolation"], ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    let switch = |k, on: bool| ParamSpec::new(k, 0.0, 1.0, if on { 1.0 } else { 0.0 }).scale(Scale::Stepped);
    let s = NodeSchema::new("plugin:audiofileprocessor", 1)
        .param(ParamSpec::new(KEYS[0], 0.0, 500.0, 100.0).unit("%"))
        .param(ParamSpec::new(KEYS[1], 0.0, 1.0, 0.0))
        .param(ParamSpec::new(KEYS[2], 0.0, 1.0, 1.0))
        .param(ParamSpec::new(KEYS[3], 0.0, 1.0, 0.0))
        .param(switch(KEYS[4], false))
        .param(switch(KEYS[5], false))
        .param(switch(KEYS[6], true))
        .field(FieldSpec::new("sample", FieldKind::Resource("audio")).nullable().default(Value::NULL))
        .field(FieldSpec::choice("loop", &LOOPS, "off"))
        .field(FieldSpec::choice("interpolation", &INTERPOLATIONS, "linear"));
    vec![add_instrument_functions(s)]
}

/// The points of a sample of `size` frames from the parameters (legacy `AudioFileProcessor::pointChanged`)
pub fn bounds(size: usize, start: f32, end: f32, loop_point: f32) -> Bounds {
    let size = i32::try_from(size).unwrap_or(i32::MAX);
    let frame = |position: f32| ((position * size as f32) as i32).clamp(0, size);
    let (mut f_start, mut f_end) = (frame(start), frame(end));
    if f_start > f_end {
        std::mem::swap(&mut f_start, &mut f_end);
    }
    if f_end - f_start < 1 && size > 0 {
        if f_end < size {
            f_end += 1;
        } else {
            f_start = f_end - 1;
        }
    }
    let f_loop = frame(loop_point).clamp(f_start, if f_end > f_start { f_end - 1 } else { f_start });
    Bounds { start: f_start, end: f_end, loop_start: f_loop, loop_end: f_end }
}

/// The converter of a note: libsamplerate's, ported (none, linear) or the library's (sinc)
#[derive(Debug)]
pub struct Interp {
    port: Converter,
    /// Made in the build only when the instrument plays sinc
    sinc: Option<Resampler>,
    use_sinc: bool,
}

impl Interp {
    fn new(interpolation: usize) -> Self {
        let kind = if interpolation == 0 { Kind::ZeroOrderHold } else { Kind::Linear };
        let sinc = if interpolation == 2 { Resampler::new(Quality::Medium, 2, 1.0).ok() } else { None };
        Self { port: Converter::new(kind), sinc, use_sinc: interpolation == 2 }
    }

    /// The converter a note starts with (`interpolation`: its index in [`INTERPOLATION_KEYS`])
    fn choose(&mut self, interpolation: usize) {
        self.use_sinc = interpolation == 2 && self.sinc.is_some();
        if !self.use_sinc {
            self.port = Converter::new(if interpolation == 0 { Kind::ZeroOrderHold } else { Kind::Linear });
        }
    }
}

impl Clone for Interp {
    /// In the build only: a sinc converter is made anew (its state is not copied); the instrument never copies a
    /// voice playing sinc ([`Synth::copies_voices`])
    fn clone(&self) -> Self {
        let sinc = self.sinc.as_ref().and_then(|s| Resampler::new(s.quality(), 2, 1.0).ok());
        Self { port: self.port, sinc, use_sinc: self.use_sinc && self.sinc.is_some() }
    }

    fn clone_from(&mut self, source: &Self) {
        self.port = source.port;
        self.use_sinc = source.use_sinc && self.sinc.is_some();
    }
}

impl Interpolator for Interp {
    fn process(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]], ratio: f64) -> (usize, usize) {
        match &mut self.sinc {
            Some(sinc) if self.use_sinc => {
                if sinc.set_ratio(ratio).is_err() {
                    return (0, 0);
                }
                sinc.process(input.as_flattened(), output.as_flattened_mut())
                    .map_or((0, 0), |p| (p.input_frames, p.output_frames))
            }
            _ => self.port.process(input, output, ratio),
        }
    }

    fn reset(&mut self) {
        self.port.reset();
        if let Some(sinc) = &mut self.sinc {
            // (resetting a valid state cannot fail)
            let _ = sinc.reset();
        }
    }
}

/// One note: where it plays in the sample, and whether it still does. Copied into its own buffers (the crossfade
/// copies voices on the audio thread)
#[derive(Debug)]
pub struct Voice {
    state: PlaybackState<Interp>,
    playing: bool,
}

impl Clone for Voice {
    fn clone(&self) -> Self {
        Self { state: self.state.clone(), playing: self.playing }
    }

    fn clone_from(&mut self, source: &Self) {
        self.state.clone_from(&source.state);
        self.playing = source.playing;
    }
}

/// The sample: its frames as stereo (a mono sample on both sides; past two channels, the first two), and its rate
#[derive(Debug)]
pub struct Audio {
    pub frames: Vec<[f32; 2]>,
    pub rate: f32,
}

impl Audio {
    /// The resource's audio as the player reads it
    pub fn of(data: &lm_render::SampleData) -> Self {
        let left = data.channel(0);
        let right = if data.channels() > 1 { data.channel(1) } else { left };
        Self { frames: left.iter().zip(right).map(|(l, r)| [*l, *r]).collect(), rate: data.rate() as f32 }
    }
}

#[derive(Clone, Debug)]
pub struct AudioFileProcessor {
    rate: u32,
    audio: Option<Arc<Audio>>,
    looping: Loop,
    interpolation: usize,
    knobs: [f32; 7],
    bounds: Bounds,
    /// Where the next note starts, and which way (stutter: where the last one stopped)
    next: Cell<(i32, bool)>,
}

impl AudioFileProcessor {
    fn frames(&self) -> usize {
        self.audio.as_ref().map_or(0, |a| a.frames.len())
    }

    fn update(&mut self) {
        self.bounds = bounds(self.frames(), self.knobs[1], self.knobs[2], self.knobs[3]);
        self.next.set((self.bounds.start, false));
    }

    fn stutter(&self) -> bool {
        self.knobs[5] >= 0.5
    }
}

impl Synth for AudioFileProcessor {
    type Voice = Voice;
    const PARAMS: usize = 7;

    fn voice(&self) -> Voice {
        Voice { state: PlaybackState::new(Interp::new(self.interpolation), 0), playing: false }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(k) = self.knobs.get_mut(index) else { return };
        if *k == value {
            return;
        }
        let was = *k;
        *k = value;
        match index {
            1..=3 => self.update(),
            // (a switch moved: legacy reverseModelChanged, stutterModelChanged)
            4 | 5 if (was >= 0.5) != (value >= 0.5) => self.next.set((self.bounds.start, false)),
            _ => {}
        }
    }

    fn start(&self, voice: &mut Voice, note: &NoteStart, _: &mut Rng) {
        voice.playing = false;
        if self.stutter() && note.freq < MAGIC_FREQ {
            self.next.set((self.bounds.start, false));
            return;
        }
        if self.audio.is_none() {
            return;
        }
        if self.stutter() && self.next.get().0 >= self.bounds.end {
            // At the end of the sample: from the start again
            self.next.set((self.bounds.start, false));
        }
        let (at, backwards) = self.next.get();
        voice.state.interpolator_mut().choose(self.interpolation);
        voice.state.restart(at);
        voice.state.set_backwards(backwards);
        voice.playing = true;
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let Some(audio) = self.audio.as_deref().filter(|_| voice.playing) else {
            left.fill(0.0);
            right.fill(0.0);
            return;
        };
        let sample = Sample {
            frames: &audio.frames,
            sample_rate: audio.rate,
            bounds: self.bounds,
            amplification: self.knobs[0] / 100.0,
            frequency: DEFAULT_BASE_FREQ,
            reversed: self.knobs[4] >= 0.5,
            declick: self.knobs[6] >= 0.5,
        };
        // (in single precision, as the legacy `DefaultBaseFreq / frequency()`)
        let ratio = f64::from(DEFAULT_BASE_FREQ / freq);
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            if voice.playing && !sample.play(&mut buf[..n], &mut voice.state, self.looping, ratio, self.rate as f32, n)
            {
                voice.playing = false;
            }
            if !voice.playing {
                buf[..n].fill([0.0; 2]);
            }
            if self.stutter() {
                // (the last note to play sets where the next starts)
                self.next.set((voice.state.frame_index(), voice.state.backwards()));
            }
            for (i, f) in buf[..n].iter().enumerate() {
                left[at + i] = f[0];
                right[at + i] = f[1];
            }
            at += n;
        }
    }

    fn finished(&self, voice: &Voice) -> bool {
        !voice.playing
    }

    fn release_ms(&self) -> f32 {
        RELEASE_MS
    }

    fn can_adopt(&self, old: &Self) -> bool {
        // The old voices have sinc converters only if the old instrument played sinc
        self.interpolation != 2 || old.interpolation == 2
    }

    fn copies_voices(&self) -> bool {
        self.interpolation != 2
    }

    fn adopt(&mut self, old: &mut Self) {
        // The same sample and points: stutter goes on from where the old one stopped
        let same = match (&self.audio, &old.audio) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same && self.bounds == old.bounds {
            self.next.set(old.next.get());
        }
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut knobs = [0.0f32; 7];
    for (k, key) in knobs.iter_mut().zip(KEYS) {
        *k = s.param_f32(key)?;
    }
    let audio = match s.resource("sample")? {
        Some(r) => Some(Arc::new(Audio::of(&*cx.resources.audio(r)?))),
        None => None,
    };
    let looping = s.choice("loop", |k| match k {
        "off" => Some(Loop::Off),
        "on" => Some(Loop::On),
        "pingPong" => Some(Loop::PingPong),
        _ => None,
    })?;
    let interpolation = s.choice("interpolation", |k| INTERPOLATION_KEYS.iter().position(|i| *i == k))?;
    if interpolation == 2 {
        // (the voices' converters are made below; one made here first gives the library's error, if any)
        Resampler::new(Quality::Medium, 2, 1.0)?;
    }
    let mut synth = AudioFileProcessor {
        rate: cx.spec.sample_rate,
        audio,
        looping,
        interpolation,
        knobs,
        bounds: Bounds::default(),
        next: Cell::new((0, false)),
    };
    synth.update();
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
