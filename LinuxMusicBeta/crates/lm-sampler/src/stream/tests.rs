use std::io::Write;

use super::*;
use crate::engine::{Engine, EngineConfig};
use crate::model::{Instrument, LoopMode, Zone};
use crate::pcm::Pcm;

const RATE: u32 = 48_000;
const HEAD: usize = 10_000;

/// A sample held in memory that is served as if it were in a file
#[derive(Debug)]
struct VecTail {
    left: Vec<f32>,
}

impl TailSource for VecTail {
    fn frames(&self) -> usize {
        self.left.len()
    }

    fn stereo(&self) -> bool {
        false
    }

    fn read(&self, start: usize, left: &mut [f32], _right: &mut [f32]) -> usize {
        let n = left.len().min(self.left.len().saturating_sub(start));
        left[..n].copy_from_slice(&self.left[start..start + n]);
        n
    }
}

fn wave(frames: usize) -> Vec<f32> {
    (0..frames).map(|i| (i as f32 * 0.013).sin() * 0.5 + (i as f32 * 0.0007).cos() * 0.25).collect()
}

fn engine(sample: Pcm, looped: Option<(u32, u32)>) -> Engine {
    let mut z = Zone::new(Arc::new(sample));
    z.root_key = 60;
    if let Some((a, b)) = looped {
        z.loop_mode = LoopMode::Forward;
        z.loop_start = a;
        z.loop_end = b;
    }
    let mut i = Instrument::new("stream");
    i.zones = vec![z];
    i.validate().expect("valid");
    Engine::new(Arc::new(i), EngineConfig::new(RATE, 256))
}

fn resident(frames: &[f32], looped: Option<(u32, u32)>) -> Engine {
    engine(Pcm::mono(RATE, frames.to_vec()), looped)
}

fn streamed(frames: &[f32], looped: Option<(u32, u32)>) -> Engine {
    let head = Pcm::mono(RATE, frames[..HEAD].to_vec());
    engine(Pcm::streamed(head, Arc::new(VecTail { left: frames.to_vec() })), looped)
}

/// What `total` frames of a note at the root key render to, in blocks of 256, with `between` called after each
fn play(e: &mut Engine, total: usize, mut between: impl FnMut()) -> Vec<f32> {
    e.note_on(1, 60, 1.0);
    let mut out = Vec::with_capacity(total);
    let mut left = total;
    while left > 0 {
        let n = left.min(256);
        e.render(n);
        out.extend_from_slice(e.bus(0).0);
        between();
        left -= n;
    }
    out
}

#[test]
fn a_streamed_sample_read_by_the_engine_itself_is_the_resident_one() {
    let frames = wave(60_000);
    let mut a = resident(&frames, None);
    let mut b = streamed(&frames, None);
    assert!(b.streams() && !a.streams());
    assert_eq!(play(&mut a, 58_000, || {}), play(&mut b, 58_000, || {}));
    assert_eq!(b.stream_misses(), 0);
}

#[test]
fn a_streamed_sample_fed_by_the_thread_side_is_the_resident_one_without_misses() {
    let frames = wave(60_000);
    let mut a = resident(&frames, None);
    let mut b = streamed(&frames, None);
    let (link, mut feeder) = link(QUEUE);
    b.set_link(link);
    let want = play(&mut a, 58_000, || {});
    // (the feeder answers between the calls, as its thread does within a block's time)
    let got = play(&mut b, 58_000, || {
        feeder.serve();
    });
    assert_eq!(want, got);
    assert_eq!(b.stream_misses(), 0);
}

#[test]
fn a_block_that_has_not_arrived_is_silence_and_a_miss() {
    let frames = wave(60_000);
    let mut b = streamed(&frames, None);
    let (link, _feeder) = link(QUEUE);
    b.set_link(link);
    let got = play(&mut b, 30_000, || {});
    assert!(b.stream_misses() > 0);
    // the head plays; past it nothing does
    assert!(got[..HEAD - 100].iter().any(|v| v.abs() > 0.1));
    assert!(got[HEAD + 2_000..].iter().all(|v| *v == 0.0));
}

#[test]
fn a_loop_far_past_the_head_wraps_in_the_ring() {
    let frames = wave(60_000);
    let looped = Some((30_000, 34_000));
    let mut a = resident(&frames, looped);
    let mut b = streamed(&frames, looped);
    let (link, mut feeder) = link(QUEUE);
    b.set_link(link);
    let want = play(&mut a, 120_000, || {});
    let got = play(&mut b, 120_000, || {
        feeder.serve();
    });
    assert_eq!(want, got);
    assert_eq!(b.stream_misses(), 0);
}

#[test]
fn the_rings_are_given_back_when_the_voices_end() {
    let frames = wave(30_000);
    let mut b = streamed(&frames, None);
    play(&mut b, 29_000, || {});
    // The voice ended; the next render gives its ring back, and a new note takes one again
    b.render(16);
    for _ in 0..(STREAM_VOICES + 8) {
        b.note_on(2, 60, 1.0);
        b.render(64);
        b.all_sound_off();
    }
    assert_eq!(b.stream_misses(), 0);
}

#[test]
fn a_file_tail_reads_the_frames_it_describes() {
    let path = std::env::temp_dir().join(format!("lm-stream-{}.raw", std::process::id()));
    // 3 bytes of junk, then 5 stereo frames of 24-bit and 4 mono frames of 16-bit
    let mut bytes = vec![9u8, 9, 9];
    let stereo24: [(i32, i32); 5] = [(0, 1), (8_388_607, -8_388_608), (4_194_304, -4_194_304), (1_000, -1_000), (-1, 1)];
    for (l, r) in stereo24 {
        bytes.extend_from_slice(&l.to_le_bytes()[..3]);
        bytes.extend_from_slice(&r.to_le_bytes()[..3]);
    }
    let mono16: [i16; 4] = [0, 16_384, -16_384, 32_767];
    for v in mono16 {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::File::create(&path).and_then(|mut f| f.write_all(&bytes)).expect("a temporary file");
    let file = Arc::new(std::fs::File::open(&path).expect("open"));
    let tail = FileTail::new(Arc::clone(&file), 3, 5, 2, Raw::I24);
    let (mut l, mut r) = ([0.0f32; 8], [0.0f32; 8]);
    assert_eq!(tail.read(1, &mut l, &mut r), 4, "four frames from frame 1 to the end");
    assert!((l[0] - 8_388_607.0 / 8_388_608.0).abs() < 1e-6 && (r[0] + 1.0).abs() < 1e-6);
    assert!((l[1] - 0.5).abs() < 1e-6 && (r[1] + 0.5).abs() < 1e-6);
    assert_eq!(tail.read(5, &mut l, &mut r), 0);
    let tail = FileTail::new(file, 3 + 5 * 6, 4, 1, Raw::I16).scaled(0.5);
    let mut m = [0.0f32; 4];
    assert_eq!(tail.read(0, &mut m, &mut r), 4);
    assert!((m[1] - 0.25).abs() < 1e-6 && (m[2] + 0.25).abs() < 1e-6);
    std::fs::remove_file(&path).ok();
}

#[test]
fn offset_in_finds_a_piece_of_the_whole() {
    let whole = [0u8; 100];
    assert_eq!(offset_in(&whole, &whole[40..60]), Some(40));
    assert_eq!(offset_in(&whole, &[1, 2, 3]), None);
}
