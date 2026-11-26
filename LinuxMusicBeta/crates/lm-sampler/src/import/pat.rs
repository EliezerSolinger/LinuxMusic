//! PAT: a Gravis Ultrasound patch, read by `lm_dsp::patman`'s parser. Each of the patch's samples gets the keys
//! nearest its pitch (the PatMan's rule: the sample whose frequency is nearest the note's, by ratio), at the root key
//! its recorded frequency makes. `tuned` plays the samples at the notes' pitch; untuned, every note of a sample plays
//! it at its own pitch. A patch that loops loops forward.

use std::sync::Arc;

use lm_base::{Error, Result};
use lm_dsp::patman::{RELEASE_MS, parse_patch, select_sample};

use crate::model::{Ahdsr, Instrument, LoopMode, VelCurve, Zone};
use crate::pcm::Pcm;

/// The unity-at-center gain a mono sample needs under the constant-power pan law, dB
pub const MONO_CENTER_DB: f32 = 3.0103;

/// How a patch plays
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatOptions {
    /// The samples follow the keyboard (otherwise every note of a sample plays at its own pitch)
    pub tuned: bool,
    /// A looping patch loops; off plays every sample once through
    pub looped: bool,
}

impl Default for PatOptions {
    fn default() -> Self {
        Self { tuned: true, looped: true }
    }
}

/// The key a frequency sits at (A4 = 69 at 440 Hz) and the cents past it
fn key_of(freq: f32) -> (u8, f32) {
    let exact = 69.0 + 12.0 * libm::log2f(freq / 440.0);
    let key = exact.round().clamp(0.0, 127.0);
    (key as u8, (exact - key) * 100.0)
}

/// Reads a patch's bytes. Keys: the legacy's (`patman.not_gus`, `patman.instruments`, `patman.layers`,
/// `patman.truncated`, `patman.empty`)
pub fn import(bytes: &[u8], options: PatOptions) -> Result<Instrument> {
    let samples = parse_patch(bytes).map_err(|e| Error::new(e.key()))?;
    if samples.is_empty() {
        return Err(Error::new("patman.empty"));
    }
    let pcms: Vec<Arc<Pcm>> =
        samples.iter().map(|s| Arc::new(Pcm::from_frames(s.sample_rate.max(1), &s.frames))).collect();
    let mut out = Instrument::new("patch");
    // Which sample each key plays: the nearest by frequency ratio; runs of one sample are one zone
    let choice: Vec<usize> =
        (0u32..128).map(|k| select_sample(&samples, 440.0 * libm::exp2f((k as f32 - 69.0) / 12.0))).collect();
    let mut from = 0usize;
    for k in 1..=128usize {
        if k == 128 || choice[k] != choice[from] {
            let i = choice[from];
            let s = &samples[i];
            let mut z = Zone::new(Arc::clone(&pcms[i]));
            z.keys = (from as u8, (k - 1) as u8);
            let usable = s.root_freq.is_finite() && s.root_freq > 0.0;
            let (root, cents) = if usable { key_of(s.root_freq) } else { (60, 0.0) };
            z.root_key = root;
            // (a sample's frequency is not a key: its cents past the key are a tuning)
            z.tune = -cents;
            z.key_track = if options.tuned || !usable { 100.0 } else { 0.0 };
            if let Some((a, b)) = s.looping.filter(|_| options.looped) {
                let frames = s.frames.len() as i32;
                let (a, b) = (a.clamp(0, frames), b.clamp(0, frames));
                if b > a {
                    z.loop_mode = LoopMode::Forward;
                    z.loop_start = a as u32;
                    z.loop_end = b as u32;
                }
            }
            z.vel_curve = VelCurve::Linear;
            z.vel_track = 1.0;
            z.gain = MONO_CENTER_DB;
            z.amp = Ahdsr { release: RELEASE_MS / 1000.0, ..Ahdsr::SHORT };
            out.zones.push(z);
            from = k;
        }
    }
    out.validate()?;
    Ok(out)
}

#[cfg(test)]
mod tests;
