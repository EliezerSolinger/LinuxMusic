//! The legacy program's Drum Machines: each kit's pieces (synthesized from DrumSynth files, or recordings brought to
//! the engine's rate) on their notes of the General MIDI drum map, played by 64 voices made beforehand. A piece plays to
//! its end at its velocity, its first half millisecond fading in; a hi-hat chokes the other hi-hats that ring (a fade
//! of 64 frames), as on the machines.

/// The voices
pub const VOICES: usize = 64;
/// The frames of a choke's fade
pub const CHOKE_FRAMES: i32 = 64;

/// A piece of a kit, made away from the audio thread
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrumSound {
    /// At the engine's rate
    pub frames: Vec<[f32; 2]>,
    /// Its General MIDI note
    pub note: usize,
    /// Pieces in the same group (not 0) choke each other: the hi-hats
    pub choke: i32,
}

/// A kit: its sounds, and which one each note plays
#[derive(Clone, Debug)]
pub struct DrumKit {
    pub sounds: Vec<DrumSound>,
    of_note: [i32; 128],
}

impl DrumKit {
    pub fn new(sounds: Vec<DrumSound>) -> Self {
        let mut of_note = [-1; 128];
        for (i, s) in sounds.iter().enumerate() {
            if s.note < 128 {
                of_note[s.note] = i as i32;
            }
        }
        Self { sounds, of_note }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    /// -1: free
    sound: i32,
    position: usize,
    gain: f32,
    /// Frames left of a choke's fade (0: none)
    fade: i32,
}

const FREE: Voice = Voice { sound: -1, position: 0, gain: 0.0, fade: 0 };

/// The machine playing a kit (one for the instrument)
#[derive(Clone, Debug)]
pub struct DrumMachine {
    voices: [Voice; VOICES],
    /// Half a millisecond, one frame at least
    fade_in: usize,
}

impl DrumMachine {
    pub fn new(sample_rate: u32) -> Self {
        Self { voices: [FREE; VOICES], fade_in: ((sample_rate / 2000) as usize).max(1) }
    }

    /// Every voice silent (a new kit, a reset, a note choke)
    pub fn silence(&mut self) {
        self.voices = [FREE; VOICES];
    }

    /// The piece of @p note hit at @p velocity (0 to 1). Returns which sound it plays (for the pads), if any
    pub fn note_on(&mut self, kit: &DrumKit, note: i32, velocity: f32) -> Option<usize> {
        if !(0..128).contains(&note) {
            return None;
        }
        let sound = kit.of_note[note as usize];
        if sound < 0 || kit.sounds[sound as usize].frames.is_empty() {
            return None;
        }
        // A hi-hat chokes the other hi-hats that ring (fading out, not to click)
        let choke = kit.sounds[sound as usize].choke;
        if choke != 0 {
            for v in &mut self.voices {
                if v.sound >= 0 && v.sound != sound && kit.sounds[v.sound as usize].choke == choke && v.fade == 0 {
                    v.fade = CHOKE_FRAMES;
                }
            }
        }
        // A free voice, else the one that has played longest
        let mut chosen = 0;
        for (i, v) in self.voices.iter().enumerate() {
            if v.sound < 0 {
                chosen = i;
                break;
            }
            if v.position > self.voices[chosen].position {
                chosen = i;
            }
        }
        self.voices[chosen] = Voice { sound, position: 0, gain: velocity, fade: 0 };
        Some(sound as usize)
    }

    /// Adds the voices' next frames to @p out
    pub fn render(&mut self, kit: &DrumKit, out: &mut [[f32; 2]]) {
        let fade_in = self.fade_in;
        for v in &mut self.voices {
            if v.sound < 0 {
                continue;
            }
            let frames = &kit.sounds[v.sound as usize].frames;
            for frame in out.iter_mut() {
                if v.position >= frames.len() {
                    *v = FREE;
                    break;
                }
                let mut gain = v.gain;
                // (its first half millisecond fades in: DrumSynth's pieces start on a step, which clicks)
                if v.position < fade_in {
                    gain *= (v.position + 1) as f32 / fade_in as f32;
                }
                if v.fade > 0 {
                    gain *= v.fade as f32 / CHOKE_FRAMES as f32;
                    v.fade -= 1;
                    if v.fade == 0 {
                        *v = FREE;
                        break;
                    }
                }
                let s = frames[v.position];
                frame[0] += s[0] * gain;
                frame[1] += s[1] * gain;
                v.position += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> DrumKit {
        DrumKit::new(vec![
            DrumSound { frames: vec![[1.0; 2]; 100], note: 36, choke: 0 },
            DrumSound { frames: vec![[0.5; 2]; 1000], note: 42, choke: 1 },
            DrumSound { frames: vec![[0.25; 2]; 1000], note: 46, choke: 1 },
        ])
    }

    #[test]
    fn a_hit_fades_in_and_plays_to_its_end() {
        let k = kit();
        let mut m = DrumMachine::new(44100);
        assert_eq!(m.note_on(&k, 36, 0.5), Some(0));
        let mut out = vec![[0.0; 2]; 200];
        m.render(&k, &mut out);
        // 22 frames of fade in at 44.1 kHz
        assert!((out[0][0] - 0.5 / 22.0).abs() < 1e-6);
        assert_eq!(out[50][0], 0.5);
        assert_eq!(out[150][0], 0.0);
    }

    #[test]
    fn a_hi_hat_chokes_the_other() {
        let k = kit();
        let mut m = DrumMachine::new(44100);
        m.note_on(&k, 46, 1.0);
        let mut out = vec![[0.0; 2]; 100];
        m.render(&k, &mut out);
        m.note_on(&k, 42, 1.0);
        let mut out = vec![[0.0; 2]; 200];
        m.render(&k, &mut out);
        // The open hat fades out over 64 frames, the closed one goes on
        assert!(out[100][0] > 0.49 && out[100][0] < 0.51);
    }

    #[test]
    fn a_note_without_a_piece_plays_nothing() {
        let k = kit();
        let mut m = DrumMachine::new(44100);
        assert_eq!(m.note_on(&k, 60, 1.0), None);
        assert_eq!(m.note_on(&k, 200, 1.0), None);
    }
}
