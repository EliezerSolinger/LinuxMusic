//! Drum kits from pieces: the DrumMachines' kits. A piece is a sample on a MIDI note with a choke group; it plays to
//! its end at the note's velocity (a note-off does not stop a drum) and the pieces of a choke group end one
//! another (the hi-hats) in the legacy's 64 frames. The `.ds` pieces are DrumSynth files rendered here with
//! `lm_dsp::drumsynth`; the recordings come already decoded (the plugin reads the FLAC through its libraries).

use std::sync::Arc;

use lm_base::{Error, Result};
use lm_dsp::drum_machines::CHOKE_FRAMES;
use lm_dsp::drumsynth;
use lm_dsp::math::LegacyRand;

use crate::import::avl::assign_outputs;
use crate::model::{Ahdsr, Instrument, LoopMode, VelCurve, Zone};
use crate::pcm::Pcm;

/// A piece ready to play
#[derive(Clone, Debug)]
pub struct Piece {
    /// Its General MIDI note
    pub note: u8,
    /// Its choke group (0: none)
    pub choke: u32,
    pub pcm: Arc<Pcm>,
}

/// A DrumSynth piece at `rate`: its mono frames (the legacy decoder's 16 bits over 32768). The noise comes from
/// `rand`, drawn in the order the pieces are rendered: one generator for a kit's pieces, seeded with 1, as the legacy
/// loaded a kit. An empty result is a file that is not a DrumSynth file
pub fn render_ds(text: &[u8], rate: u32, rand: &mut LegacyRand) -> Option<Arc<Pcm>> {
    let frames = drumsynth::render(text, rate, rand);
    if frames.is_empty() {
        return None;
    }
    Some(Arc::new(Pcm::mono(rate, frames.into_iter().map(|s| f32::from(s) / 32768.0).collect())))
}

/// The generator a kit's `.ds` pieces share
pub fn kit_rand() -> LegacyRand {
    LegacyRand::new(1)
}

/// The kit as an instrument: a one-shot zone for each piece that has sound (a piece that could not be read is
/// silent: the caller leaves it out), the choke groups, and, with `groups`, an output for each group of pieces
pub fn import(pieces: &[Piece], groups: bool) -> Result<Instrument> {
    let mut out = Instrument::new("drums");
    for p in pieces {
        if p.pcm.is_empty() || p.note > 127 {
            continue;
        }
        let mut z = Zone::new(Arc::clone(&p.pcm));
        z.keys = (p.note, p.note);
        z.root_key = p.note;
        // A drum has no pitch: every note of its key plays the piece as it is
        z.key_track = 0.0;
        z.loop_mode = LoopMode::OneShot;
        z.vel_curve = VelCurve::Linear;
        z.vel_track = 1.0;
        z.gain = if p.pcm.is_stereo() { 0.0 } else { crate::import::pat::MONO_CENTER_DB };
        z.amp = Ahdsr::SHORT;
        z.group = p.choke;
        z.off_by = p.choke;
        z.off_fast = p.choke != 0;
        out.zones.push(z);
    }
    if out.zones.is_empty() {
        return Err(Error::new("sampler.drums.empty"));
    }
    assign_outputs(&mut out, groups);
    out.validate()?;
    Ok(out)
}

/// The legacy's choke time, in frames, for the engine's fast release to be compared with
pub const LEGACY_CHOKE_FRAMES: i32 = CHOKE_FRAMES;

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(note: u8, choke: u32) -> Piece {
        Piece { note, choke, pcm: Arc::new(Pcm::mono(44100, vec![0.5; 100])) }
    }

    #[test]
    fn pieces_become_one_shot_zones_with_chokes() {
        let i = import(&[piece(36, 0), piece(42, 1), piece(46, 1)], false).expect("imports");
        assert_eq!(i.zones.len(), 3);
        assert!(i.zones.iter().all(|z| z.loop_mode == LoopMode::OneShot && z.key_track == 0.0));
        assert_eq!((i.zones[1].group, i.zones[1].off_by), (1, 1));
        assert!(i.zones[1].off_fast && !i.zones[0].off_fast);
    }

    #[test]
    fn groups_give_an_output_each() {
        let i = import(&[piece(36, 0), piece(38, 0), piece(49, 0)], true).expect("imports");
        assert_eq!(i.outputs.len(), 7);
        assert_eq!(i.zones[0].output, 0);
        assert_eq!(i.zones[1].output, 1);
        assert_eq!(i.zones[2].output, 5);
    }

    #[test]
    fn no_pieces_is_an_error() {
        assert!(import(&[], false).is_err());
    }

    #[test]
    fn a_non_drumsynth_file_renders_nothing() {
        assert!(render_ds(b"not a drumsynth file", 44100, &mut kit_rand()).is_none());
    }
}
