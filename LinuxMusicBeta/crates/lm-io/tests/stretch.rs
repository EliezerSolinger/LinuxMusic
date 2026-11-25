//! The rendering of a take that follows the tempo (F-REC-090): exactly the frames asked for, the pitch kept, each key
//! frame where the plan says, silence around, the same bytes twice, nothing left by a cancel

mod common;

use std::path::{Path, PathBuf};

use common::TempDir;
use lm_base::{Cancel, Progress};
use lm_io::stretch::{StretchJob, render_folder, render_path, render_stretch};
use lm_sys_sndfile::{Existing, Reader, Sample, WriteFormat, WriteSpec, Writer};

const RATE: u32 = 48_000;
const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A stereo take: a 440 Hz sine on the left, clicks every half second on the right
fn source(project: &Path) -> PathBuf {
    let path = project.join("take.wav");
    let spec = WriteSpec { format: WriteFormat::Wav(Sample::Float32), rate: RATE, channels: 2 };
    let mut w = Writer::create(&path, spec, Existing::Fail).unwrap();
    let frames: Vec<f32> = (0..96_000)
        .flat_map(|n| {
            let s = 0.5 * (std::f64::consts::TAU * 440.0 * n as f64 / f64::from(RATE)).sin() as f32;
            [s, if n % 24_000 == 12_000 { 1.0 } else { 0.0 }]
        })
        .collect();
    w.write_f32(&frames).unwrap();
    w.close().unwrap();
    path
}

fn read(path: &Path) -> (usize, Vec<f32>, Vec<f32>) {
    let mut r = Reader::open(path).unwrap();
    let frames = r.info().frames as usize;
    let mut buf = vec![0.0f32; frames * 2];
    assert_eq!(r.read_f32(&mut buf).unwrap(), frames);
    let left = buf.iter().step_by(2).copied().collect();
    let right = buf.iter().skip(1).step_by(2).copied().collect();
    (frames, left, right)
}

fn frequency(x: &[f32]) -> f64 {
    let ups: Vec<f64> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(n, w)| n as f64 + f64::from(w[0] / (w[0] - w[1])))
        .collect();
    (ups.len() - 1) as f64 / (ups[ups.len() - 1] - ups[0]) * f64::from(RATE)
}

fn job(project: &Path, key_frames: Vec<(u64, u64)>, output_frames: u64) -> StretchJob {
    let take = project.join("take.wav");
    StretchJob {
        source: if take.exists() { take } else { source(project) },
        project: project.to_owned(),
        key: KEY.to_owned(),
        key_frames,
        output_frames,
        rate: RATE,
    }
}

#[test]
fn a_rendering_is_exactly_as_long_as_asked_keeps_the_pitch_and_puts_each_key_frame_in_place() {
    let dir = TempDir::new("stretch");
    // 1000 frames of silence, the first second squeezed into half a second, the second stretched over 1.5 s
    let j = job(&dir.0, vec![(0, 1_000), (48_000, 25_000), (96_000, 97_000)], 100_000);
    let path = render_stretch(&j, &Cancel::new(), &Progress::new()).unwrap();
    assert_eq!(path, render_path(&dir.0, KEY));
    let (frames, left, right) = read(&path);
    assert_eq!(frames, 100_000, "exactly the frames asked");
    assert!(left[..1_000].iter().chain(&left[97_000..]).all(|v| *v == 0.0), "silence around");
    // The pitch of every stretch between the clicks (a click is a transient: the engine's channels go together)
    for (from, to) in
        [(1_500, 6_000), (8_000, 18_000), (20_000, 24_500), (26_000, 42_000), (44_000, 78_000), (80_000, 96_500)]
    {
        let f = frequency(&left[from..to]);
        assert!((f / 440.0 - 1.0).abs() < 0.005, "{from}..{to}: the pitch kept, {f} Hz");
    }
    // No dip where the pieces meet (the key frame at 25000, crossfaded)
    let rms = |x: &[f32]| (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    for w in (24_000..26_000).step_by(100) {
        let level = rms(&left[w..w + 200]) / rms(&left[20_000..24_000]);
        assert!((level - 1.0).abs() < 0.12, "the level at {w}: {level}");
    }
    // The clicks at 0.25 s, 0.75 s, 1.25 s, 1.75 s of the take, where the key frames put them
    // (source 0..48000 to 1000..25000: half speed; 48000..96000 to 25000..97000: 1.5 times)
    for at in [1_000usize + 6_000, 1_000 + 18_000, 25_000 + 18_000, 25_000 + 54_000] {
        let found = (at.saturating_sub(1_500)..(at + 1_500).min(frames))
            .max_by(|a, b| right[*a].abs().total_cmp(&right[*b].abs()))
            .unwrap();
        assert!(found.abs_diff(at) <= 200, "a click at {at}, found at {found}");
    }
}

#[test]
fn the_same_plan_twice_makes_the_same_bytes() {
    let (a, b) = (TempDir::new("stretch-a"), TempDir::new("stretch-b"));
    let frames = vec![(0, 0), (30_000, 50_000), (96_000, 150_000)];
    let pa = render_stretch(&job(&a.0, frames.clone(), 150_000), &Cancel::new(), &Progress::new()).unwrap();
    let pb = render_stretch(&job(&b.0, frames, 150_000), &Cancel::new(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(pa).unwrap(), std::fs::read(pb).unwrap());
}

#[test]
fn a_cancel_leaves_nothing_and_a_rendering_there_is_kept() {
    let dir = TempDir::new("stretch-cancel");
    let j = job(&dir.0, vec![(0, 0), (96_000, 192_000)], 192_000);
    let cancel = Cancel::new();
    cancel.cancel();
    assert!(render_stretch(&j, &cancel, &Progress::new()).unwrap_err().is_cancelled());
    let left: Vec<_> = std::fs::read_dir(render_folder(&dir.0)).map(|d| d.flatten().collect()).unwrap_or_default();
    assert!(left.is_empty(), "nothing left: {left:?}");
    let path = render_stretch(&j, &Cancel::new(), &Progress::new()).unwrap();
    let made = std::fs::metadata(&path).unwrap().modified().unwrap();
    render_stretch(&j, &Cancel::new(), &Progress::new()).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), made, "not made again");
    // A plan outside the take is refused
    let bad = job(&dir.0, vec![(0, 0), (200_000, 1_000)], 1_000);
    let bad = StretchJob { key: KEY.replace('0', "f"), ..bad };
    assert_eq!(render_stretch(&bad, &Cancel::new(), &Progress::new()).unwrap_err().key(), "io.stretch.bad_plan");
}

#[test]
fn a_ramp_of_many_pieces_keeps_the_pitch_and_the_level() {
    let dir = TempDir::new("stretch-ramp");
    // Twelve pieces of 8000 frames, their ratios from 0.8 to 1.35 (a tempo ramp cut into key frames)
    let mut points = vec![(0u64, 0u64)];
    for k in 0..12u64 {
        let r = 0.8 + 0.05 * k as f64;
        let last = *points.last().unwrap();
        points.push((last.0 + 8_000, last.1 + (8_000.0 * r).round() as u64));
    }
    let out = points.last().unwrap().1;
    let path = render_stretch(&job(&dir.0, points.clone(), out), &Cancel::new(), &Progress::new()).unwrap();
    let (frames, left, _) = read(&path);
    assert_eq!(frames as u64, out);
    let rms = |x: &[f32]| (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    let reference = rms(&left[2_000..6_000]);
    for w in (1_000..frames - 1_200).step_by(150) {
        let level = rms(&left[w..w + 200]) / reference;
        assert!((level - 1.0).abs() < 0.12, "the level at {w}: {level}");
    }
    // (the pieces without a click: a click is a transient the engine spreads over both channels)
    for w in points.windows(2).skip(1).filter(|w| (w[0].0..w[1].0).all(|x| x % 24_000 != 12_000)) {
        let (from, to) = (w[0].1 as usize + 800, w[1].1 as usize - 800);
        let f = frequency(&left[from..to]);
        assert!((f / 440.0 - 1.0).abs() < 0.005, "{from}..{to}: {f} Hz");
    }
}
