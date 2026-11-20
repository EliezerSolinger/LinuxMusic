//! PatMan (F-INS-013: legacy `plugins/Patman`): a Gravis Ultrasound patch (`.pat`), each note playing the patch's
//! sample nearest its pitch, tuned to the note or at the sample's own pitch, looped by the patch's loop or not.
//!
//! State, version 1: parameters `looped` and `tuned` (switches, on); field `patch` (a resource of kind `pat`, none);
//! and the instrument functions. The patch is read in the build (off the audio thread), as the legacy read it when
//! loaded; a file that is not a one-instrument, one-layer GUS patch fails the build with the legacy's reason. As the
//! legacy, `tuned` is read when a note starts and `looped` as it plays; the samples play through libsamplerate's
//! linear converter, here its port ([`crate::src_port`], so a sounding note can be copied without allocating).

use std::sync::Arc;

use lm_base::{Error, Result};
use lm_dsp::patman::{PatchSample, PatmanVoice, RELEASE_MS, parse_patch};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, Loaded, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::src_port::{Converter, Kind};
use crate::stereo_out;

pub const ID: &str = "patman";
pub const VOICES: usize = 32;
/// The resource kind of a patch
pub const PATCH_KIND: &str = "pat";

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.patman.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: a patch sounds whatever its samples hold; the pitch is tested against the legacy note)
    tests: TestHints { voices: VOICES, ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    let switch = |k| ParamSpec::new(k, 0.0, 1.0, 1.0).scale(Scale::Stepped);
    let s = NodeSchema::new("plugin:patman", 1)
        .param(switch("looped"))
        .param(switch("tuned"))
        .field(FieldSpec::new("patch", FieldKind::Resource(PATCH_KIND)).nullable().default(Value::NULL));
    vec![add_instrument_functions(s)]
}

/// The samples of a patch's bytes; the legacy's reason as the error's key
pub fn read_patch(bytes: &[u8]) -> Result<Vec<PatchSample>> {
    let samples = parse_patch(bytes).map_err(|e| Error::new(e.key()))?;
    if samples.is_empty() {
        return Err(Error::new("patman.empty"));
    }
    Ok(samples)
}

#[derive(Clone, Debug)]
pub struct Patman {
    rate: u32,
    samples: Option<Arc<Vec<PatchSample>>>,
    looped: bool,
    tuned: bool,
}

/// One note: the patch's voice, and whether it still plays
#[derive(Debug)]
pub struct Voice {
    sound: PatmanVoice<Converter>,
    playing: bool,
}

impl Clone for Voice {
    fn clone(&self) -> Self {
        Self { sound: self.sound.clone(), playing: self.playing }
    }

    fn clone_from(&mut self, source: &Self) {
        self.sound.clone_from(&source.sound);
        self.playing = source.playing;
    }
}

impl Synth for Patman {
    type Voice = Voice;
    const PARAMS: usize = 2;

    fn voice(&self) -> Voice {
        let samples = self.samples.as_deref().map_or(&[][..], |s| s.as_slice());
        let sound = PatmanVoice::new(samples, 440.0, self.tuned, Converter::new(Kind::Linear));
        Voice { sound, playing: false }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.looped = value >= 0.5,
            1 => self.tuned = value >= 0.5,
            _ => {}
        }
    }

    fn start(&self, voice: &mut Voice, note: &NoteStart, _: &mut Rng) {
        voice.playing = false;
        if let Some(samples) = &self.samples {
            voice.sound.restart(samples, note.freq, self.tuned);
            voice.playing = true;
        }
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let Some(samples) = self.samples.as_deref().filter(|_| voice.playing) else {
            left.fill(0.0);
            right.fill(0.0);
            return;
        };
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            if voice.playing {
                voice.playing = voice.sound.render(&mut buf[..n], samples, freq, self.looped, self.rate as f32, n);
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
        RELEASE_MS
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let samples = match s.resource("patch")? {
        Some(r) => {
            let bytes: Arc<[u8]> = match cx.resources.get(r) {
                Some(Loaded::Bytes(b)) => b.clone(),
                Some(Loaded::File(path)) => std::fs::read(path)
                    .map_err(|e| Error::new("patman.read_failed").arg("error", e.to_string()))?
                    .into(),
                _ => cx.resources.bytes(r)?,
            };
            Some(Arc::new(read_patch(&bytes)?))
        }
        None => None,
    };
    let synth = Patman {
        rate: cx.spec.sample_rate,
        samples,
        looped: s.param_f32("looped")? >= 0.5,
        tuned: s.param_f32("tuned")? >= 0.5,
    };
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
