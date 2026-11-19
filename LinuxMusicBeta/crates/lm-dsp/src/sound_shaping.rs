//! An instrument's sound shaping (legacy `InstrumentSoundShaping`): its filter, whose cutoff and resonance its
//! envelopes and LFOs move, and its volume envelope, on each note.

use crate::basic_filters::{BasicFilter, FilterType};
use crate::envelope::{EnvelopeAndLfo, EnvelopeSettings, exp_knob_val};

const CUT_FREQ_MULTIPLIER: f32 = 6000.0;
const RES_MULTIPLIER: f32 = 2.0;
const RES_PRECISION: f32 = 1000.0;

/// The sound shaping's controls
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundShapingSettings {
    pub filter_enabled: bool,
    pub filter: FilterType,
    /// Hz, 1 to 14000
    pub cutoff: f32,
    /// 0.01 to 10
    pub resonance: f32,
    pub volume: EnvelopeSettings,
    pub cutoff_envelope: EnvelopeSettings,
    pub resonance_envelope: EnvelopeSettings,
}

impl Default for SoundShapingSettings {
    fn default() -> Self {
        Self {
            filter_enabled: false,
            filter: FilterType::LowPass,
            cutoff: 14000.0,
            resonance: 0.5,
            volume: EnvelopeSettings::default(),
            cutoff_envelope: EnvelopeSettings::default(),
            resonance_envelope: EnvelopeSettings::default(),
        }
    }
}

/// The instrument's three envelopes and LFOs, and the scratch the notes use (one at a time)
#[derive(Clone, Debug)]
pub struct SoundShaping {
    pub volume: EnvelopeAndLfo,
    pub cutoff: EnvelopeAndLfo,
    pub resonance: EnvelopeAndLfo,
    cut_buffer: Vec<f32>,
    res_buffer: Vec<f32>,
    vol_buffer: Vec<f32>,
}

/// Where a note is in its envelope
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotePosition {
    /// The frames the note has played before these
    pub frame: i32,
    /// The frame (since the note started) where its release begins (or past these frames while it is held)
    pub release_begin: i32,
}

impl SoundShaping {
    /// Prepared for @p sample_rate and periods of up to @p max_period frames; @p seed is the random LFOs'
    pub fn new(sample_rate: f32, max_period: usize, seed: u64) -> Self {
        Self {
            volume: EnvelopeAndLfo::new(1.0, sample_rate, max_period, seed),
            cutoff: EnvelopeAndLfo::new(0.0, sample_rate, max_period, seed.wrapping_add(1)),
            resonance: EnvelopeAndLfo::new(0.0, sample_rate, max_period, seed.wrapping_add(2)),
            cut_buffer: vec![0.0; max_period],
            res_buffer: vec![0.0; max_period],
            vol_buffer: vec![0.0; max_period],
        }
    }

    /// The envelopes' settings (and the period the LFOs' shapes are worked out for)
    pub fn set(&mut self, settings: &SoundShapingSettings, period: usize) {
        self.volume.set(&settings.volume, None, period);
        self.cutoff.set(&settings.cutoff_envelope, None, period);
        self.resonance.set(&settings.resonance_envelope, None, period);
    }

    /// The frames of the longest envelope until its release (only the volume's: @p only_volume)
    pub fn env_frames(&self, only_volume: bool) -> i32 {
        let mut frames = self.volume.pahd_frames();
        if !only_volume {
            if self.cutoff.is_used() {
                frames = frames.max(self.cutoff.pahd_frames());
            }
            if self.resonance.is_used() {
                frames = frames.max(self.resonance.pahd_frames());
            }
        }
        frames
    }

    /// The frames of the release: the instrument's own (@p desired), the volume envelope's when it is used, or the
    /// longest of the filter's; a single-streamed instrument keeps its own
    pub fn release_frames(&self, desired: i32, single_streamed: bool) -> i32 {
        if single_streamed {
            return desired;
        }
        if self.volume.is_used() {
            return self.volume.release_frames();
        }
        let mut frames = desired;
        if self.cutoff.is_used() {
            frames = frames.max(self.cutoff.release_frames());
        }
        if self.resonance.is_used() {
            frames = frames.max(self.resonance.release_frames());
        }
        frames
    }

    /// The volume envelope's level of a note at one frame
    pub fn volume_level(&self, frame: i32, release_begin: i32) -> f32 {
        let mut level = [0.0f32];
        self.volume.fill_level(&mut level, frame, release_begin);
        level[0]
    }

    /// Shapes a note's @p buffer: its filter (@p filter, the note's own), then its volume envelope
    pub fn process(
        &mut self,
        buffer: &mut [[f32; 2]],
        filter: &mut BasicFilter,
        position: NotePosition,
        settings: &SoundShapingSettings,
    ) {
        let frames = buffer.len();
        let NotePosition { frame: env_frame, release_begin } = position;
        if settings.filter_enabled {
            let mut old_cut = 0i32;
            let mut old_res = 0i32;
            filter.set_filter_type(settings.filter);
            let cut_used = self.cutoff.is_used();
            let res_used = self.resonance.is_used();
            let cut = &mut self.cut_buffer[..frames];
            let res = &mut self.res_buffer[..frames];
            if cut_used {
                self.cutoff.fill_level(cut, env_frame, release_begin);
            }
            if res_used {
                self.resonance.fill_level(res, env_frame, release_begin);
            }
            let (fcv, frv) = (settings.cutoff, settings.resonance);
            let run = |frame: &mut [f32; 2], filter: &mut BasicFilter| {
                frame[0] = filter.update(frame[0], 0);
                frame[1] = filter.update(frame[1], 1);
            };
            if cut_used && res_used {
                for (n, frame) in buffer.iter_mut().enumerate() {
                    let new_cut = exp_knob_val(cut[n]) * CUT_FREQ_MULTIPLIER + fcv;
                    let new_res = frv + RES_MULTIPLIER * res[n];
                    if new_cut as i32 != old_cut || (new_res * RES_PRECISION) as i32 != old_res {
                        filter.calc_filter_coeffs(new_cut, new_res);
                        old_cut = new_cut as i32;
                        old_res = (new_res * RES_PRECISION) as i32;
                    }
                    run(frame, filter);
                }
            } else if cut_used {
                for (n, frame) in buffer.iter_mut().enumerate() {
                    let new_cut = exp_knob_val(cut[n]) * CUT_FREQ_MULTIPLIER + fcv;
                    if new_cut as i32 != old_cut {
                        filter.calc_filter_coeffs(new_cut, frv);
                        old_cut = new_cut as i32;
                    }
                    run(frame, filter);
                }
            } else if res_used {
                for (n, frame) in buffer.iter_mut().enumerate() {
                    let new_res = frv + RES_MULTIPLIER * res[n];
                    if (new_res * RES_PRECISION) as i32 != old_res {
                        filter.calc_filter_coeffs(fcv, new_res);
                        old_res = (new_res * RES_PRECISION) as i32;
                    }
                    run(frame, filter);
                }
            } else {
                filter.calc_filter_coeffs(fcv, frv);
                for frame in buffer.iter_mut() {
                    run(frame, filter);
                }
            }
        }

        if self.volume.is_used() {
            let vol = &mut self.vol_buffer[..frames];
            self.volume.fill_level(vol, env_frame, release_begin);
            for (frame, level) in buffer.iter_mut().zip(vol.iter()) {
                let level = level * level;
                frame[0] *= level;
                frame[1] *= level;
            }
        }
    }
}
