//! SlicerT (F-INS: legacy `plugins/SlicerT`, Daniel Kauss Serna's): a sample cut into slices, the base note (A4)
//! playing it whole and each key above it one slice, in time with the song if synced, faded out at the slice's end
//! (a cosine over `fadeOut`), or looped while held and faded out after the note.
//!
//! State, version 1: parameters `noteThreshold` (0–2, 0.6: the onset threshold the slices are found with), `fadeOut`
//! (0–100 ms, 10), `originalBpm` (1–999, 1: the sample's tempo) and the switch `sync` (off); fields `sample` (an
//! audio resource, none), `snap` (`off`, `1/1`, `1/2`, `1/4`, `1/8`, `1/16`, `1/32`; `off`: the notes the slices
//! snap to when found), `loop` (`off`, `on`, `pingPong`; `off`), `slices` (the slice points, fractions of the sample
//! from 0 to 1, as the legacy saved them: empty); and the instrument functions.
//!
//! The legacy found the tempo and the slices when a sample was loaded; here that is [`analyze`], run by whoever sets
//! the sample (a command, off the audio thread), which gives the `originalBpm` and the `slices` to put in the state;
//! "find slices" again with other `noteThreshold` or `snap` is [`find_slices`]. The notes play through libsamplerate's
//! linear converter, here its port ([`crate::src_port`], so a sounding note can be copied without allocating). The
//! song's tempo comes from the host.

use std::sync::{Arc, LazyLock};

use lm_base::Result;
use lm_dsp::sample_player::Loop;
use lm_dsp::slicert::{SlicerSample, SlicerSettings, SlicerVoice, find_bpm, release_ms, slice_for};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::audio::Audio;
use crate::src_port::{Converter, Kind};
use crate::stereo_out;

pub const ID: &str = "slicert";
pub const VOICES: usize = 32;
/// The parameters, in this order
pub const KEYS: [&str; 4] = ["noteThreshold", "fadeOut", "originalBpm", "sync"];
pub const SNAP_KEYS: [&str; 7] = ["off", "1/1", "1/2", "1/4", "1/8", "1/16", "1/32"];
pub const LOOP_KEYS: [&str; 3] = ["off", "on", "pingPong"];
static SNAPS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| SNAP_KEYS.into());
static LOOPS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| LOOP_KEYS.into());
/// The key of the base note, A4: the whole sample (the legacy track's default base note)
pub const BASE_KEY: i32 = 69;
/// The song's tempo until the host gives it, BPM
const DEFAULT_BPM: f64 = 120.0;

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.slicert.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: a slice plays at its own pitch, the keys choose slices; its ratios are tested against the
    // legacy note)
    tests: TestHints { voices: VOICES, ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    let s = NodeSchema::new("plugin:slicert", 1)
        .param(ParamSpec::new(KEYS[0], 0.0, 2.0, 0.6))
        .param(ParamSpec::new(KEYS[1], 0.0, 100.0, 10.0).unit("ms"))
        .param(ParamSpec::new(KEYS[2], 1.0, 999.0, 1.0).scale(Scale::Stepped))
        .param(ParamSpec::new(KEYS[3], 0.0, 1.0, 0.0).scale(Scale::Stepped))
        .field(FieldSpec::new("sample", FieldKind::Resource("audio")).nullable().default(Value::NULL))
        .field(FieldSpec::choice("snap", &SNAPS, "off"))
        .field(FieldSpec::choice("loop", &LOOPS, "off"))
        .field(
            FieldSpec::new("slices", FieldKind::List(Box::new(FieldKind::Real { min: 0.0, max: 1.0 })))
                .default(Value::empty_list()),
        );
    vec![add_instrument_functions(s)]
}

/// What loading a sample finds: its tempo (for `originalBpm`) and its slices (for `slices`)
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub bpm: i32,
    pub slices: Vec<f32>,
}

/// The tempo and the slices of a sample, as the legacy found them on loading (`noteThreshold` as its knob, `snap`
/// the index in [`SNAP_KEYS`]). Away from the audio thread
pub fn analyze(audio: &Audio, threshold: f32, snap: usize) -> Analysis {
    let rate = audio.rate as u32;
    let bpm = find_bpm(&audio.frames, rate);
    Analysis { bpm, slices: find_slices(audio, threshold, bpm, snap) }
}

/// The slices again, with the sample's tempo @p bpm (the legacy "find slices")
pub fn find_slices(audio: &Audio, threshold: f32, bpm: i32, snap: usize) -> Vec<f32> {
    lm_dsp::slicert::find_slices(&audio.frames, audio.rate as u32, threshold, bpm, snap as i32)
}

/// The synth's knobs as the notes read them
pub fn settings(k: &[f32; 4], looping: Loop) -> SlicerSettings {
    SlicerSettings { fade_out: k[1], original_bpm: k[2] as i32, sync: k[3] >= 0.5, looping }
}

/// The pitch of a key, Hz
fn key_freq(key: u8) -> f32 {
    440.0 * libm::exp2f((f32::from(key) - 69.0) / 12.0)
}

#[derive(Clone, Debug)]
pub struct SlicerT {
    rate: u32,
    audio: Option<Arc<Audio>>,
    points: Arc<[f32]>,
    knobs: [f32; 4],
    looping: Loop,
    /// The song's tempo
    bpm: f64,
}

/// One note: its slice, and whether it still plays. Copied into its own buffers (the crossfade copies voices on the
/// audio thread)
#[derive(Debug)]
pub struct Voice {
    sound: SlicerVoice<Converter>,
    playing: bool,
    key: u8,
}

impl Clone for Voice {
    fn clone(&self) -> Self {
        Self { sound: self.sound.clone(), playing: self.playing, key: self.key }
    }

    fn clone_from(&mut self, source: &Self) {
        self.sound.clone_from(&source.sound);
        self.playing = source.playing;
        self.key = source.key;
    }
}

impl Synth for SlicerT {
    type Voice = Voice;
    const PARAMS: usize = 4;

    fn voice(&self) -> Voice {
        Voice { sound: SlicerVoice::new((0.0, 1.0), 0, Converter::new(Kind::Linear)), playing: false, key: 0 }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(k) = self.knobs.get_mut(index) {
            *k = value;
        }
    }

    fn start(&self, voice: &mut Voice, note: &NoteStart, _: &mut Rng) {
        voice.playing = false;
        voice.key = note.key;
        let Some(audio) = self.audio.as_ref().filter(|a| a.frames.len() > 1) else { return };
        if let Some(slice) = slice_for(&self.points, i32::from(note.key) - BASE_KEY) {
            voice.sound.restart(slice, audio.frames.len());
            voice.playing = true;
        }
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let Some(audio) = self.audio.as_ref().filter(|a| voice.playing && voice.sound.fits(a.frames.len())) else {
            voice.playing = false;
            left.fill(0.0);
            right.fill(0.0);
            return;
        };
        // The track's pitch, in cents: the note's pitch (with bend and glide) against its key's
        let pitch = 1200.0 * libm::log2f(freq / key_freq(voice.key));
        let pitch = if pitch.is_finite() { pitch } else { 0.0 };
        let sample = SlicerSample { frames: &audio.frames, sample_rate: audio.rate as u32 };
        let settings = settings(&self.knobs, self.looping);
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            if voice.playing {
                voice.playing =
                    voice.sound.render(&mut buf[..n], &sample, &settings, self.bpm as f32, pitch, self.rate as f32, n);
            } else {
                buf[..n].fill([0.0; 2]);
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
        release_ms(&settings(&self.knobs, self.looping))
    }

    fn set_tempo(&mut self, bpm: f64) {
        if bpm > 0.0 {
            self.bpm = bpm;
        }
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut knobs = [0.0f32; 4];
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
    let points: Arc<[f32]> = s
        .value()
        .get("slices")
        .map(|l| l.items().filter_map(Value::as_real).map(|p| p.clamp(0.0, 1.0) as f32).collect())
        .unwrap_or_default();
    let synth = SlicerT { rate: cx.spec.sample_rate, audio, points, knobs, looping, bpm: DEFAULT_BPM };
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
