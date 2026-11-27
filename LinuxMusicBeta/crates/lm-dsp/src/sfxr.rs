//! The legacy program's sfxr (Tomas Pettersson's sound-effect generator, Wong Cho Ching's port): a square, saw,
//! sine or noise wave with slides, arpeggio, vibrato, an envelope, a resonant low-pass and a high-pass, a phaser
//! and repeats, 8 times supersampled, rendered at 44.1 kHz at the note's pitch.

use crate::math::{LegacyRand, f32c::PI, powf, sinf};

/// The waves
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SfxrWave {
    Square,
    Saw,
    Sine,
    Noise,
}

impl SfxrWave {
    pub const ALL: [SfxrWave; 4] = [SfxrWave::Square, SfxrWave::Saw, SfxrWave::Sine, SfxrWave::Noise];
}

/// The sfxr's knobs: 0 to 1, or -1 to 1 for the slides, sweeps and changes
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfxrSettings {
    pub attack: f32,
    pub sustain: f32,
    pub punch: f32,
    pub decay: f32,
    pub start_freq: f32,
    pub min_freq: f32,
    pub slide: f32,
    pub delta_slide: f32,
    pub vibrato_depth: f32,
    pub vibrato_speed: f32,
    pub change_amount: f32,
    pub change_speed: f32,
    pub square_duty: f32,
    pub duty_sweep: f32,
    pub repeat_speed: f32,
    pub phaser_offset: f32,
    pub phaser_sweep: f32,
    pub lp_cutoff: f32,
    pub lp_cutoff_sweep: f32,
    pub lp_resonance: f32,
    pub hp_cutoff: f32,
    pub hp_cutoff_sweep: f32,
    pub wave: SfxrWave,
}

impl Default for SfxrSettings {
    fn default() -> Self {
        Self {
            attack: 0.0,
            sustain: 0.3,
            punch: 0.0,
            decay: 0.4,
            start_freq: 0.3,
            min_freq: 0.0,
            slide: 0.0,
            delta_slide: 0.0,
            vibrato_depth: 0.0,
            vibrato_speed: 0.0,
            change_amount: 0.0,
            change_speed: 0.0,
            square_duty: 0.0,
            duty_sweep: 0.0,
            repeat_speed: 0.0,
            phaser_offset: 0.0,
            phaser_sweep: 0.0,
            lp_cutoff: 1.0,
            lp_cutoff_sweep: 0.0,
            lp_resonance: 0.0,
            hp_cutoff: 0.0,
            hp_cutoff_sweep: 0.0,
            wave: SfxrWave::Square,
        }
    }
}

/// The frames rendered at once at the synth's own pitch
const BLOCK: usize = 512;
/// The range a note's pitch ratio is kept to (six octaves either way): bounds the work per period
const MIN_PITCH: f64 = 1.0 / 64.0;
const MAX_PITCH: f64 = 64.0;

/// The sfxr's synth, at its own rate. A note restarts it in place ([`SfxrSynth::restart`]) and `clone_from` copies
/// into it: neither allocates (for the audio thread)
#[derive(Debug)]
pub struct SfxrSynth {
    s: SfxrSettings,
    rand: LegacyRand,
    playing: bool,
    phase: i32,
    fperiod: f64,
    fmaxperiod: f64,
    fslide: f64,
    fdslide: f64,
    period: i32,
    square_duty: f32,
    square_slide: f32,
    env_stage: usize,
    env_time: i32,
    env_length: [i32; 3],
    env_vol: f32,
    fphase: f32,
    fdphase: f32,
    iphase: i32,
    phaser: Box<[f32; 1024]>,
    ipp: i32,
    noise: [f32; 32],
    fltp: f32,
    fltdp: f32,
    fltw: f32,
    fltw_d: f32,
    fltdmp: f32,
    fltphp: f32,
    flthp: f32,
    flthp_d: f32,
    vib_phase: f32,
    vib_speed: f32,
    vib_amp: f32,
    rep_time: i32,
    rep_limit: i32,
    arp_time: i32,
    arp_limit: i32,
    arp_mod: f64,
}

impl Clone for SfxrSynth {
    fn clone(&self) -> Self {
        Self {
            s: self.s,
            rand: self.rand,
            playing: self.playing,
            phase: self.phase,
            fperiod: self.fperiod,
            fmaxperiod: self.fmaxperiod,
            fslide: self.fslide,
            fdslide: self.fdslide,
            period: self.period,
            square_duty: self.square_duty,
            square_slide: self.square_slide,
            env_stage: self.env_stage,
            env_time: self.env_time,
            env_length: self.env_length,
            env_vol: self.env_vol,
            fphase: self.fphase,
            fdphase: self.fdphase,
            iphase: self.iphase,
            phaser: self.phaser.clone(),
            ipp: self.ipp,
            noise: self.noise,
            fltp: self.fltp,
            fltdp: self.fltdp,
            fltw: self.fltw,
            fltw_d: self.fltw_d,
            fltdmp: self.fltdmp,
            fltphp: self.fltphp,
            flthp: self.flthp,
            flthp_d: self.flthp_d,
            vib_phase: self.vib_phase,
            vib_speed: self.vib_speed,
            vib_amp: self.vib_amp,
            rep_time: self.rep_time,
            rep_limit: self.rep_limit,
            arp_time: self.arp_time,
            arp_limit: self.arp_limit,
            arp_mod: self.arp_mod,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.s = source.s;
        self.rand = source.rand;
        self.playing = source.playing;
        self.phase = source.phase;
        self.fperiod = source.fperiod;
        self.fmaxperiod = source.fmaxperiod;
        self.fslide = source.fslide;
        self.fdslide = source.fdslide;
        self.period = source.period;
        self.square_duty = source.square_duty;
        self.square_slide = source.square_slide;
        self.env_stage = source.env_stage;
        self.env_time = source.env_time;
        self.env_length = source.env_length;
        self.env_vol = source.env_vol;
        self.fphase = source.fphase;
        self.fdphase = source.fdphase;
        self.iphase = source.iphase;
        self.phaser.clone_from(&source.phaser);
        self.ipp = source.ipp;
        self.noise = source.noise;
        self.fltp = source.fltp;
        self.fltdp = source.fltdp;
        self.fltw = source.fltw;
        self.fltw_d = source.fltw_d;
        self.fltdmp = source.fltdmp;
        self.fltphp = source.fltphp;
        self.flthp = source.flthp;
        self.flthp_d = source.flthp_d;
        self.vib_phase = source.vib_phase;
        self.vib_speed = source.vib_speed;
        self.vib_amp = source.vib_amp;
        self.rep_time = source.rep_time;
        self.rep_limit = source.rep_limit;
        self.arp_time = source.arp_time;
        self.arp_limit = source.arp_limit;
        self.arp_mod = source.arp_mod;
    }
}

impl SfxrSynth {
    /// The sound @p s (taken when the note starts); its noise from @p seed
    pub fn new(s: &SfxrSettings, seed: u64) -> Self {
        let mut v = Self {
            s: *s,
            rand: LegacyRand::new(seed),
            playing: true,
            phase: 0,
            fperiod: 0.0,
            fmaxperiod: 0.0,
            fslide: 0.0,
            fdslide: 0.0,
            period: 0,
            square_duty: 0.0,
            square_slide: 0.0,
            env_stage: 0,
            env_time: 0,
            env_length: [0; 3],
            env_vol: 0.0,
            fphase: 0.0,
            fdphase: 0.0,
            iphase: 0,
            phaser: Box::new([0.0; 1024]),
            ipp: 0,
            noise: [0.0; 32],
            fltp: 0.0,
            fltdp: 0.0,
            fltw: 0.0,
            fltw_d: 0.0,
            fltdmp: 0.0,
            fltphp: 0.0,
            flthp: 0.0,
            flthp_d: 0.0,
            vib_phase: 0.0,
            vib_speed: 0.0,
            vib_amp: 0.0,
            rep_time: 0,
            rep_limit: 0,
            arp_time: 0,
            arp_limit: 0,
            arp_mod: 0.0,
        };
        v.reset_sample(false);
        v
    }

    /// A new note of the sound @p s, as [`SfxrSynth::new`] makes one, without allocating
    pub fn restart(&mut self, s: &SfxrSettings, seed: u64) {
        self.s = *s;
        self.rand = LegacyRand::new(seed);
        self.playing = true;
        self.reset_sample(false);
    }

    fn noise_draw(&mut self) -> f32 {
        -1.0 + self.rand.inclusive(2.0)
    }

    fn reset_sample(&mut self, restart: bool) {
        let s = self.s;
        if !restart {
            self.phase = 0;
        }
        self.fperiod = 100.0 / (f64::from(s.start_freq * s.start_freq) + 0.001);
        self.period = self.fperiod as i32;
        self.fmaxperiod = 100.0 / (f64::from(s.min_freq * s.min_freq) + 0.001);
        let sv = f64::from(s.slide);
        let dsv = f64::from(s.delta_slide);
        self.fslide = 1.0 - sv * sv * sv * 0.01;
        self.fdslide = -dsv * dsv * dsv * 0.000001;
        self.square_duty = 0.5 - s.square_duty * 0.5;
        self.square_slide = -s.duty_sweep * 0.00005;
        let cha = f64::from(s.change_amount);
        self.arp_mod = if cha >= 0.0 { 1.0 - cha * cha * 0.9 } else { 1.0 + cha * cha * 10.0 };
        self.arp_time = 0;
        let chs = 1.0 - s.change_speed;
        self.arp_limit = if chs == 0.0 { 0 } else { (chs * chs * 20000.0 + 32.0) as i32 };
        if !restart {
            self.fltp = 0.0;
            self.fltdp = 0.0;
            self.fltw = powf(s.lp_cutoff, 3.0) * 0.1;
            self.fltw_d = 1.0 + s.lp_cutoff_sweep * 0.0001;
            self.fltdmp = (5.0 / (1.0 + powf(s.lp_resonance, 2.0) * 20.0) * (0.01 + self.fltw)).min(0.8);
            self.fltphp = 0.0;
            self.flthp = powf(s.hp_cutoff, 2.0) * 0.1;
            self.flthp_d = (1.0 + f64::from(s.hp_cutoff_sweep * 0.0003)) as f32;
            self.vib_phase = 0.0;
            self.vib_speed = powf(s.vibrato_speed, 2.0) * 0.01;
            self.vib_amp = s.vibrato_depth * 0.5;
            self.env_vol = 0.0;
            self.env_stage = 0;
            self.env_time = 0;
            self.env_length = [
                (s.attack * s.attack * 99999.0) as i32 + 1,
                (s.sustain * s.sustain * 99999.0) as i32 + 1,
                (s.decay * s.decay * 99999.0) as i32 + 1,
            ];
            self.fphase = powf(s.phaser_offset, 2.0) * 1020.0;
            if s.phaser_offset < 0.0 {
                self.fphase = -self.fphase;
            }
            self.fdphase = powf(s.phaser_sweep, 2.0) * 1.0;
            if s.phaser_sweep < 0.0 {
                self.fdphase = -self.fdphase;
            }
            self.iphase = (self.fphase as i32).abs();
            self.ipp = 0;
            self.phaser.fill(0.0);
            for i in 0..32 {
                self.noise[i] = self.noise_draw();
            }
            self.rep_time = 0;
            self.rep_limit = (powf(1.0 - s.repeat_speed, 2.0) * 20000.0 + 32.0) as i32;
            if s.repeat_speed == 0.0 {
                self.rep_limit = 0;
            }
        }
    }

    /// Whether it still sounds (once silent, the note is let go)
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The synth's next frames, at its own rate (44.1 kHz)
    pub fn update(&mut self, out: &mut [[f32; 2]]) {
        let s = self.s;
        for frame in out.iter_mut() {
            if !self.playing {
                *frame = [0.0; 2];
                continue;
            }
            self.rep_time += 1;
            if self.rep_limit != 0 && self.rep_time >= self.rep_limit {
                self.rep_limit = 0;
                self.reset_sample(true);
            }
            // The frequency's slides and arpeggio
            self.arp_time += 1;
            if self.arp_limit != 0 && self.arp_time >= self.arp_limit {
                self.arp_limit = 0;
                self.fperiod *= self.arp_mod;
            }
            self.fslide += self.fdslide;
            self.fperiod *= self.fslide;
            if self.fperiod > self.fmaxperiod {
                self.fperiod = self.fmaxperiod;
                if s.min_freq > 0.0 {
                    self.playing = false;
                }
            }
            let mut rfperiod = self.fperiod as f32;
            if self.vib_amp > 0.0 {
                self.vib_phase += self.vib_speed;
                rfperiod = (self.fperiod * (1.0 + f64::from(sinf(self.vib_phase) * self.vib_amp))) as f32;
            }
            self.period = (rfperiod as i32).max(8);
            self.square_duty = (self.square_duty + self.square_slide).clamp(0.0, 0.5);

            // The volume envelope
            self.env_time += 1;
            if self.env_time > self.env_length[self.env_stage] {
                self.env_time = 0;
                self.env_stage += 1;
                if self.env_stage == 3 {
                    self.playing = false;
                }
            }
            match self.env_stage {
                0 => self.env_vol = self.env_time as f32 / self.env_length[0] as f32,
                1 => {
                    self.env_vol = 1.0 + (1.0 - self.env_time as f32 / self.env_length[1] as f32) * 2.0 * s.punch;
                }
                2 => self.env_vol = 1.0 - self.env_time as f32 / self.env_length[2] as f32,
                _ => {}
            }

            // The phaser
            self.fphase += self.fdphase;
            self.iphase = (self.fphase as i32).abs().min(1023);

            if self.flthp_d != 0.0 {
                self.flthp = (self.flthp * self.flthp_d).clamp(0.00001, 0.1);
            }

            let mut ssample = 0.0f32;
            // 8 times supersampled
            for _ in 0..8 {
                self.phase += 1;
                if self.phase >= self.period {
                    self.phase %= self.period;
                    if s.wave == SfxrWave::Noise {
                        for i in 0..32 {
                            self.noise[i] = self.noise_draw();
                        }
                    }
                }
                let fp = self.phase as f32 / self.period as f32;
                let mut sample = match s.wave {
                    SfxrWave::Square => {
                        if fp < self.square_duty {
                            0.5
                        } else {
                            -0.5
                        }
                    }
                    SfxrWave::Saw => 1.0 - fp * 2.0,
                    SfxrWave::Sine => sinf(fp * 2.0 * PI),
                    SfxrWave::Noise => self.noise[(self.phase * 32 / self.period) as usize],
                };
                // The low-pass
                let pp = self.fltp;
                self.fltw = (self.fltw * self.fltw_d).clamp(0.0, 0.1);
                if s.lp_cutoff != 1.0 {
                    self.fltdp += (sample - self.fltp) * self.fltw;
                    self.fltdp -= self.fltdp * self.fltdmp;
                } else {
                    self.fltp = sample;
                    self.fltdp = 0.0;
                }
                self.fltp += self.fltdp;
                // The high-pass
                self.fltphp += self.fltp - pp;
                self.fltphp -= self.fltphp * self.flthp;
                sample = self.fltphp;
                // The phaser
                self.phaser[(self.ipp & 1023) as usize] = sample;
                sample += self.phaser[((self.ipp - self.iphase + 1024) & 1023) as usize];
                self.ipp = (self.ipp + 1) & 1023;
                ssample += sample * self.env_vol;
            }
            ssample = (ssample * 0.025).clamp(-1.0, 1.0);
            *frame = [ssample, ssample];
        }
    }
}

/// One note of the sfxr: its synth, played at the note's pitch. Restarted in place and copied into its own room
/// (`clone_from`), without allocating
#[derive(Debug)]
pub struct SfxrVoice {
    pub synth: SfxrSynth,
    fraction_due: f64,
    block: Box<[[f32; 2]; BLOCK]>,
}

impl Clone for SfxrVoice {
    fn clone(&self) -> Self {
        Self { synth: self.synth.clone(), fraction_due: self.fraction_due, block: self.block.clone() }
    }

    fn clone_from(&mut self, source: &Self) {
        self.synth.clone_from(&source.synth);
        self.fraction_due = source.fraction_due;
        self.block.clone_from(&source.block);
    }
}

impl SfxrVoice {
    /// A note of the sound @p s; its noise from @p seed
    pub fn new(s: &SfxrSettings, seed: u64) -> Self {
        Self { synth: SfxrSynth::new(s, seed), fraction_due: 0.0, block: Box::new([[0.0; 2]; BLOCK]) }
    }

    /// A new note of the sound @p s in this voice, without allocating
    pub fn restart(&mut self, s: &SfxrSettings, seed: u64) {
        self.synth.restart(s, seed);
        self.fraction_due = 0.0;
    }

    /// A period of the note into @p out: the synth's frames at the note's pitch (@p pitch: the note's frequency
    /// over the instrument's base one), each output frame the one rendered at its place (the fraction of a frame
    /// carried to the next period). False once it is silent (@p out then silent: the host lets the note go)
    pub fn render(&mut self, out: &mut [[f32; 2]], pitch: f64, sample_rate: f32) -> bool {
        if !self.synth.is_playing() {
            out.fill([0.0; 2]);
            return false;
        }
        // A tuning far out of range would make the loop below run for ages (or the pitch be NaN/infinite)
        if !pitch.is_finite() {
            out.fill([0.0; 2]);
            return true;
        }
        let pitch = pitch.clamp(MIN_PITCH, MAX_PITCH);
        let frames = out.len();
        let due = pitch * frames as f64 * 44100.0 / f64::from(sample_rate) + self.fraction_due;
        let whole = due as i32;
        self.fraction_due = due - f64::from(whole);
        let pitched = whole.max(0) as usize;
        let (mut rendered, mut frame) = (0usize, 0usize);
        while rendered < pitched {
            let count = BLOCK.min(pitched - rendered);
            self.synth.update(&mut self.block[..count]);
            while frame < frames {
                let source = frame * pitched / frames;
                if source >= rendered + count {
                    break;
                }
                out[frame] = self.block[source - rendered];
                frame += 1;
            }
            rendered += count;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absurd_pitch_is_bounded_and_a_non_finite_one_is_silent() {
        let s = SfxrSettings { wave: SfxrWave::Saw, ..SfxrSettings::default() };
        let mut voice = SfxrVoice::new(&s, 3);
        let mut out = vec![[1.0; 2]; 256];
        assert!(voice.render(&mut out, f64::INFINITY, 44100.0));
        assert!(out.iter().all(|f| *f == [0.0; 2]));
        assert!(voice.render(&mut out, f64::NAN, 44100.0));
        assert!(voice.render(&mut out, 1.0e300, 44100.0));
        assert!(out.iter().all(|f| f[0].is_finite()));
    }

    #[test]
    fn a_note_at_the_base_pitch_and_rate_plays_the_synth_as_it_renders() {
        let s = SfxrSettings { wave: SfxrWave::Saw, ..SfxrSettings::default() };
        let mut voice = SfxrVoice::new(&s, 3);
        let mut synth = SfxrSynth::new(&s, 3);
        let mut a = vec![[0.0; 2]; 700];
        let mut b = vec![[0.0; 2]; 700];
        assert!(voice.render(&mut a, 1.0, 44100.0));
        synth.update(&mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn an_octave_up_takes_every_other_frame() {
        let s = SfxrSettings::default();
        let mut voice = SfxrVoice::new(&s, 0);
        let mut synth = SfxrSynth::new(&s, 0);
        let mut a = vec![[0.0; 2]; 256];
        let mut b = vec![[0.0; 2]; 512];
        voice.render(&mut a, 2.0, 44100.0);
        synth.update(&mut b);
        for (n, frame) in a.iter().enumerate() {
            assert_eq!(*frame, b[n * 2]);
        }
    }
}
