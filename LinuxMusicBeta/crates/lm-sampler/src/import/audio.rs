//! A single audio file as an instrument: the AudioFileProcessor. One zone over the whole keyboard, the sample sounding
//! as recorded at A4, between a start and an end point, looped from a loop point (forward or back and forth),
//! reversed, amplified, with microfades against clicks. With `stutter` each note starts where the last one stopped.
//!
//! The start, end and loop points are the plugin's knobs (fractions of the sample): the zone is *windowed*, so the
//! engine reads them from its parameters while it plays and the same instrument serves every setting. The options
//! here are the initial ones for [`Zone`]'s own fields.

use std::sync::Arc;

use lm_base::{Error, Result};

use crate::model::{Ahdsr, Instrument, LoopMode, VelCurve, Window, Zone};
use crate::pcm::Pcm;

/// The AudioFileProcessor's release (legacy `desiredReleaseTimeMs`), seconds
pub const RELEASE: f32 = 0.003;

/// How the file plays
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioOptions {
    pub loop_mode: LoopMode,
    pub window: Window,
    pub reverse: bool,
    pub stutter: bool,
    /// The legacy amplification (1 = 100 %)
    pub amplify: f32,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self { loop_mode: LoopMode::Off, window: Window::default(), reverse: false, stutter: false, amplify: 1.0 }
    }
}

/// The instrument for `pcm`. Keys: `sampler.audio.empty`
pub fn import(pcm: Arc<Pcm>, options: AudioOptions) -> Result<Instrument> {
    if pcm.is_empty() {
        return Err(Error::new("sampler.audio.empty"));
    }
    let mut z = Zone::new(pcm);
    // A4 is the sample's own pitch
    z.root_key = 69;
    z.windowed = true;
    z.declick = true;
    z.loop_mode = options.loop_mode;
    z.reverse = options.reverse;
    z.stutter = options.stutter;
    z.vel_curve = VelCurve::Linear;
    z.amp = Ahdsr { release: RELEASE, ..Ahdsr::SHORT };
    // (a mono file plays on both sides at unity, not under the constant-power pan: +3 dB at the center)
    let mono = if z.sample.is_stereo() { 0.0 } else { crate::import::pat::MONO_CENTER_DB };
    z.gain = 20.0 * libm::log10f(options.amplify.max(1.0e-6)) + mono;
    let mut out = Instrument::new("audio");
    out.zones.push(z);
    out.validate()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_windowed_zone_over_the_keyboard() {
        let i = import(Arc::new(Pcm::mono(44100, vec![0.1; 100])), AudioOptions { amplify: 2.0, ..Default::default() })
            .expect("imports");
        assert_eq!(i.zones.len(), 1);
        let z = &i.zones[0];
        assert_eq!((z.keys, z.root_key, z.windowed), ((0, 127), 69, true));
        // 2x is +6 dB, and +3 dB for the mono file
        assert!((z.gain - (6.0206 + 3.0103)).abs() < 1.0e-3);
    }

    #[test]
    fn an_empty_file_is_refused() {
        assert!(import(Arc::new(Pcm::mono(44100, Vec::new())), AudioOptions::default()).is_err());
    }
}
