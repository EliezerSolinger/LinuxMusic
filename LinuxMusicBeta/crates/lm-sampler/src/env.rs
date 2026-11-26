//! The AHDSR envelope: delay, a linear attack, a hold, an exponential decay to the sustain level and an exponential
//! release (the SoundFont's shape: decay and release times are the time to fall 100 dB). One `Env` per voice and per
//! use (amplitude, filter), stepped a sample at a time with no allocation.

use crate::model::Ahdsr;

const FLOOR_DB: f32 = -100.0;
const FLOOR: f32 = 1.0e-5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

/// An envelope in progress
#[derive(Clone, Copy, Debug)]
pub struct Env {
    stage: Stage,
    level: f32,
    /// Samples left in the timed stage (delay, hold)
    left: u32,
    attack_step: f32,
    decay_mul: f32,
    release_mul: f32,
    sustain: f32,
    hold: u32,
}

/// The per-sample factor that falls 100 dB in `seconds`
fn fall(seconds: f32, rate: f32) -> f32 {
    let n = (seconds * rate).max(1.0);
    libm::powf(10.0, FLOOR_DB / 20.0 / n)
}

impl Env {
    pub const fn idle() -> Self {
        Self {
            stage: Stage::Idle,
            level: 0.0,
            left: 0,
            attack_step: 1.0,
            decay_mul: 0.0,
            release_mul: 0.0,
            sustain: 1.0,
            hold: 0,
        }
    }

    /// Starts the envelope `a` at `rate`
    pub fn start(&mut self, a: &Ahdsr, rate: f32) {
        self.sustain = a.sustain.clamp(0.0, 1.0);
        self.attack_step = if a.attack <= 0.0 { 1.0 } else { 1.0 / (a.attack * rate).max(1.0) };
        self.decay_mul = fall(a.decay, rate);
        self.release_mul = fall(a.release.max(0.0), rate);
        self.hold = (a.hold.max(0.0) * rate) as u32;
        self.level = 0.0;
        if a.delay > 0.0 {
            self.stage = Stage::Delay;
            self.left = (a.delay * rate) as u32;
        } else {
            self.enter_attack();
        }
    }

    fn enter_attack(&mut self) {
        self.stage = Stage::Attack;
    }

    fn enter_hold(&mut self) {
        if self.hold > 0 {
            self.stage = Stage::Hold;
            self.left = self.hold;
        } else {
            self.enter_decay();
        }
    }

    fn enter_decay(&mut self) {
        if self.sustain >= 1.0 {
            self.stage = Stage::Sustain;
            self.level = 1.0;
        } else {
            self.stage = Stage::Decay;
        }
    }

    /// The release from the level it is at, in `seconds`
    pub fn release(&mut self, seconds: f32, rate: f32) {
        if matches!(self.stage, Stage::Idle | Stage::Release) {
            return;
        }
        self.release_mul = fall(seconds.max(0.0), rate);
        self.stage = Stage::Release;
    }

    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
    }

    pub fn level(&self) -> f32 {
        self.level
    }

    /// The next sample of the envelope, 0 to 1
    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Delay => {
                if self.left == 0 {
                    self.enter_attack();
                } else {
                    self.left -= 1;
                }
            }
            Stage::Attack => {
                self.level += self.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.enter_hold();
                }
            }
            Stage::Hold => {
                if self.left == 0 {
                    self.enter_decay();
                } else {
                    self.left -= 1;
                }
            }
            Stage::Decay => {
                self.level *= self.decay_mul;
                if self.level <= self.sustain {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {}
            Stage::Release => {
                self.level *= self.release_mul;
                if self.level < FLOOR {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 1000.0;

    #[test]
    fn attack_reaches_one_then_sustains() {
        let mut e = Env::idle();
        e.start(&Ahdsr { delay: 0.0, attack: 0.01, hold: 0.0, decay: 0.1, sustain: 0.5, release: 0.1 }, RATE);
        let mut last = 0.0;
        for _ in 0..10 {
            last = e.next();
        }
        assert!((last - 1.0).abs() < 1.0e-6);
        for _ in 0..2000 {
            last = e.next();
        }
        assert!((last - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn release_ends_idle() {
        let mut e = Env::idle();
        e.start(&Ahdsr::SHORT, RATE);
        for _ in 0..5 {
            e.next();
        }
        e.release(0.05, RATE);
        for _ in 0..200 {
            e.next();
        }
        assert!(e.is_idle());
        assert_eq!(e.level(), 0.0);
    }

    #[test]
    fn delay_holds_zero() {
        let mut e = Env::idle();
        e.start(&Ahdsr { delay: 0.01, ..Ahdsr::SHORT }, RATE);
        for _ in 0..10 {
            assert_eq!(e.next(), 0.0);
        }
    }
}
