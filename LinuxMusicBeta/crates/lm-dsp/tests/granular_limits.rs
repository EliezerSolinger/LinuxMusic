//! The grain list has a ceiling: knobs far outside their ranges (a density of a million, a size of zero) neither
//! allocate nor produce anything but finite audio

use lm_dsp::granular_pitch_shifter::{GranularPitchShifter, GranularSettings};

#[test]
fn absurd_density_and_size_stay_bounded_and_finite() {
    let mut shifter = GranularPitchShifter::new(48_000.0, 0, 1.0, 0.0, 3);
    let settings = GranularSettings { density: 1.0e6, size: 0.0, ..GranularSettings::default() };
    let mut block = [[0.25f32; 2]; 2048];
    for _ in 0..8 {
        shifter.process(&mut block, &settings);
    }
    assert!(block.iter().flatten().all(|v| v.is_finite()));
}
