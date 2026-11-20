//! The channel strip run in segments (`process_stages`, for a rack with inserts between its sections) is bit for
//! bit the strip run whole, for every split point, the meters and the tail included, and allocates nothing

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_dsp::channel_strip::{ChannelStrip, InsertPoint, STRIP_STAGES, StripSettings, UNITY_POSITION, stage_after};
use lm_dsp::dynamics::{CompressorModel, CompressorSettings, CompressorSpeed, GateMode, GateSettings};
use lm_dsp::gain_stage::GainModel;
use lm_dsp::math::sinf;
use lm_dsp::tolerance::ChannelTolerance;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const BLOCK: usize = 256;

fn input(block: usize) -> Vec<[f32; 2]> {
    // Loud, then quiet, then silent stretches, so the gate, the compressor and the tail all move
    let level = match block % 12 {
        0..=4 => 0.8,
        5..=7 => 0.01,
        _ => 0.0,
    };
    (0..BLOCK)
        .map(|n| {
            let t = (block * BLOCK + n) as f32 / 48_000.0;
            let tau = std::f32::consts::TAU;
            let v = level * (sinf(t * 220.0 * tau) + 0.3 * sinf(t * 3_100.0 * tau));
            [v, -0.7 * v + 0.1 * level]
        })
        .collect()
}

/// The settings of block @p b: every section on, and changing as the blocks go (the ramps, the switches)
fn settings(b: usize, drive: GainModel) -> StripSettings {
    let phase = (b % 16) as f32 / 16.0;
    StripSettings {
        gain: UNITY_POSITION + 0.3 * phase,
        drive,
        realistic: b.is_multiple_of(2),
        low_cut: b % 10 < 7,
        high_pass: 20.0 + 200.0 * phase,
        low_pass: if b % 9 < 6 { 20_000.0 - 15_000.0 * phase } else { 20_000.0 },
        mono_bass: if b % 7 < 5 { 100.0 * phase } else { 0.0 },
        mono_bass_frequency: 150.0,
        compressor: CompressorSettings {
            model: [CompressorModel::Ssl, CompressorModel::Urei1176, CompressorModel::La2a][b / 20 % 3],
            amount: 0.7,
            ratio: if b % 11 < 8 { 0.6 } else { 0.0 },
            speed: CompressorSpeed::Fast,
            release: 0.4,
        },
        gate: GateSettings { threshold: 0.0, range: if b % 13 < 9 { 40.0 } else { 0.0 }, mode: GateMode::Vox },
        eq: [6.0 * phase, -9.0 + 18.0 * phase, if b % 5 < 3 { -4.0 } else { 4.0 }, 3.0],
    }
}

#[test]
fn every_split_is_bit_for_bit_the_whole_strip() {
    let tolerance = ChannelTolerance::for_channel(3);
    for drive in [GainModel::Flat, GainModel::OpAmp4558, GainModel::Tube12AX7, GainModel::Tube12AU7] {
        for split in 0..=STRIP_STAGES {
            let mut whole = ChannelStrip::new(48_000.0);
            let mut parts = ChannelStrip::new(48_000.0);
            // The test means something only if the sections really work: the dynamics reduce, the tail rings on
            let (mut reduced, mut gated, mut tail_without_input) = (false, false, false);
            for b in 0..120 {
                let s = settings(b, drive);
                let had_input = b % 12 < 8;
                let mut a = input(b);
                let mut c = a.clone();
                // The insert between the segments: the same in both
                let mut touch = |f: &mut [[f32; 2]]| {
                    for x in f.iter_mut() {
                        x[0] *= 0.9;
                    }
                };
                let point = [
                    InsertPoint::PreGain,
                    InsertPoint::PostGain,
                    InsertPoint::PostFilter,
                    InsertPoint::PostDynamics,
                    InsertPoint::PostEq,
                ][split];
                assert_eq!(stage_after(point), split);
                let (mut sat_a, mut sat_c) = (0.0, 0.0);
                assert_no_alloc(|| {
                    whole.process(&mut a, &s, had_input, &tolerance, point, Some(&mut touch));
                    parts.process_stages(&mut c, &s, had_input, &tolerance, 0..split);
                    touch(&mut c);
                    parts.process_stages(&mut c, &s, had_input, &tolerance, split..STRIP_STAGES);
                    sat_a = whole.gain_stage().take_saturation();
                    sat_c = parts.gain_stage().take_saturation();
                });
                let same =
                    a.iter().zip(&c).all(|(x, y)| x[0].to_bits() == y[0].to_bits() && x[1].to_bits() == y[1].to_bits());
                assert!(same, "{drive:?}, split at {split}, block {b}");
                assert_eq!(sat_a.to_bits(), sat_c.to_bits(), "saturation meter");
                assert_eq!(whole.gain_stage().take_clip(), parts.gain_stage().take_clip());
                let (ra, rc) = (whole.reductions(), parts.reductions());
                assert_eq!((ra.0.to_bits(), ra.1.to_bits()), (rc.0.to_bits(), rc.1.to_bits()), "reductions");
                assert_eq!(whole.holds_signal(), parts.holds_signal(), "tail, block {b}");
                reduced |= ra.1.abs() > 1.0;
                gated |= ra.0.abs() > 1.0;
                tail_without_input |= !had_input && whole.holds_signal();
            }
            assert!(reduced && gated && tail_without_input, "{reduced} {gated} {tail_without_input}");
        }
    }
}

#[test]
fn segments_of_one_stage_each_and_ranges_past_the_end() {
    let tolerance = ChannelTolerance::NONE;
    let mut whole = ChannelStrip::new(44_100.0);
    let mut parts = ChannelStrip::new(44_100.0);
    for b in 0..60 {
        let s = settings(b, GainModel::Tube12AX7Hot);
        let mut a = input(b);
        let mut c = a.clone();
        whole.process(&mut a, &s, true, &tolerance, InsertPoint::PostEq, None);
        for stage in 0..STRIP_STAGES {
            parts.process_stages(&mut c, &s, true, &tolerance, stage..stage + 1);
        }
        // Empty and out-of-range ranges do nothing
        parts.process_stages(&mut c, &s, true, &tolerance, 4..9);
        parts.process_stages(&mut c, &s, true, &tolerance, 2..2);
        assert!(a.iter().zip(&c).all(|(x, y)| x[0].to_bits() == y[0].to_bits() && x[1].to_bits() == y[1].to_bits()));
    }
    assert_eq!(stage_after(InsertPoint::PrePan), STRIP_STAGES);
}
