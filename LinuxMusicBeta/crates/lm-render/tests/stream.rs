//! Streaming from the disk (P-32): a 10-minute item plays bit for bit as from memory; seeking and looping work; a
//! slow disk gives reported underruns of exact silence, with nothing allocated on the audio thread. Not under loom
//! (real threads and the disk; the engine's loom models are in the library).
#![cfg(not(loom))]

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lm_render::*;
use lm_time::{Bpm, Ppq, TempoMap, TimeSignature};

const RATE: u32 = 48_000;
const BLOCK: usize = 4096;

fn spec() -> ProcessSpec {
    ProcessSpec { sample_rate: RATE, max_block: BLOCK }
}

fn limits() -> Limits {
    Limits { max_nodes: 16, max_params: 16, ..Limits::default() }
}

/// Noise that is the same every run
fn source(frames: usize) -> Arc<SampleData> {
    let mut x = 0x1234_5678u32;
    let data: Vec<f32> = (0..frames)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        })
        .collect();
    Arc::new(SampleData::from_channels(RATE, vec![data]).expect("data"))
}

struct Side {
    control: EngineControl,
    dummy: Dummy,
}

/// An engine playing one item into device channel 0
fn side(item: AudioItem, loop_range: Option<(i64, i64)>) -> Side {
    install_panic_hook();
    let (mut control, renderer) = engine(spec(), limits());
    let tempo = TempoMap::constant(RATE, Ppq::DEFAULT, Bpm::from_integer(120).unwrap(), TimeSignature::COMMON).unwrap();
    let node = NodeIndex::new(1, 1);
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(), tempo);
    b.node(NodeDesc::sum(node, 1));
    b.audio_items(node, vec![item]);
    b.output(OutputRoute { node, channel: 0, device_channel: 0 });
    b.transport(TransportSettings { loop_range, ..TransportSettings::default() });
    control.install(Install::new(b.build().expect("program"))).expect("install");
    Side { control, dummy: Dummy::new(renderer, 0, 1) }
}

impl Side {
    fn block(&mut self) -> &[f32] {
        let d = &mut self.dummy;
        assert_no_alloc::assert_no_alloc(|| {
            d.block(BLOCK);
        });
        &self.dummy.outputs()[0][..BLOCK]
    }

    fn send(&mut self, c: Command) {
        self.control.commands().send(c).expect("command");
    }

    fn underruns(&mut self) -> u64 {
        let mut n = 0u64;
        self.control.events(|e| {
            if let Telemetry::StreamUnderrun { frames, .. } = e {
                n += u64::from(frames);
            }
        });
        n
    }
}

fn item(source: AudioSource, frames: usize) -> AudioItem {
    AudioItem { start: 1000, length: frames as i64 - 500, source, offset: 500, gain: 0.75, fade_in: 64, fade_out: 64, fade_in_curve: 0.0, fade_out_curve: 0.0, fade_in_equal: false, fade_out_equal: false }
}

/// Serves every chunk the stream wants, from the data (the "file"), now
fn serve_all(f: &StreamFeeder, data: &SampleData) {
    let chunk = f.chunk_frames();
    let mut buf = Vec::new();
    while let Some(c) = f.next_request() {
        let from = c as usize * chunk;
        let to = (from + chunk).min(data.frames());
        buf.clear();
        buf.extend_from_slice(&data.channel(0)[from..to]);
        f.write(c, &buf).expect("write");
    }
}

/// A disk thread: serves the feeder until it is abandoned, waiting `delay` per chunk
fn disk(f: StreamFeeder, data: Arc<SampleData>, delay: Duration, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let chunk = f.chunk_frames();
        let mut buf = Vec::with_capacity(chunk);
        while !f.is_abandoned() && !stop.load(Ordering::Relaxed) {
            match f.next_request() {
                Some(c) => {
                    std::thread::sleep(delay);
                    let from = c as usize * chunk;
                    let to = (from + chunk).min(data.frames());
                    buf.clear();
                    buf.extend_from_slice(&data.channel(0)[from..to]);
                    let _ = f.write(c, &buf);
                }
                None => std::thread::sleep(Duration::from_micros(200)),
            }
        }
    })
}

#[test]
fn ten_minutes_streamed_are_bit_identical_to_memory() {
    let frames = 10 * 60 * RATE as usize;
    let data = source(frames);
    let (stream, feeder) = stream(StreamSpec::new(RATE, 1, frames as u64)).expect("stream");
    let stop = Arc::new(AtomicBool::new(false));
    let reader = disk(feeder, data.clone(), Duration::ZERO, stop.clone());
    let mut mem = side(item(AudioSource::Memory(data.clone()), frames), None);
    let mut disk_side = side(item(AudioSource::Stream(stream.clone()), frames), None);
    mem.send(Command::Play);
    disk_side.send(Command::Play);
    let blocks = (frames + 2000).div_ceil(BLOCK) + 1;
    let mut nonzero = 0usize;
    for b in 0..blocks {
        // The test renders much faster than real time: it waits for the disk the way time would
        while stream.pending() {
            std::thread::sleep(Duration::from_micros(100));
        }
        let a = mem.block().to_vec();
        let s = disk_side.block();
        for (i, (x, y)) in a.iter().zip(s).enumerate() {
            assert_eq!(x.to_bits(), y.to_bits(), "block {b}, sample {i}");
        }
        nonzero += a.iter().filter(|x| **x != 0.0).count();
    }
    assert!(nonzero > frames - 10_000, "the item played ({nonzero} samples)");
    assert_eq!(stream.underruns(), 0);
    assert_eq!(disk_side.underruns(), 0);
    stop.store(true, Ordering::Relaxed);
    drop(disk_side);
    reader.join().unwrap();
}

#[test]
fn seeking_and_looping() {
    let frames = 30 * RATE as usize;
    let data = source(frames);
    let (stream, feeder) = stream(StreamSpec::new(RATE, 1, frames as u64)).expect("stream");
    // A loop of 6 s, longer than the 3 s kept ahead: the wrap is served by the cue region
    let lp = Some((2 * RATE as i64, 8 * RATE as i64));
    let mut mem = side(item(AudioSource::Memory(data.clone()), frames), lp);
    let mut st = side(item(AudioSource::Stream(stream.clone()), frames), lp);
    let compare = |mem: &mut Side, st: &mut Side, blocks: usize, feeder: &StreamFeeder| {
        for _ in 0..blocks {
            serve_all(feeder, &data);
            let a = mem.block().to_vec();
            let s = st.block();
            assert!(a.iter().zip(s).all(|(x, y)| x.to_bits() == y.to_bits()));
        }
    };
    // Stopped: the playhead's chunks are read before playing
    st.block();
    mem.block();
    mem.send(Command::Play);
    st.send(Command::Play);
    // Three wraps of the loop, with the disk serving only between blocks
    compare(&mut mem, &mut st, 3 * 6 * RATE as usize / BLOCK + 10, &feeder);
    assert_eq!(st.underruns(), 0, "the loop's start came from the cue");

    // A seek while playing, the disk not keeping up: one block of reported silence, then the music again
    mem.send(Command::Stop);
    st.send(Command::Stop);
    mem.block();
    st.block();
    let far = 20 * RATE as i64;
    for s in [&mut mem, &mut st] {
        s.send(Command::Locate { sample: far });
    }
    mem.block();
    st.block();
    // Stopped at the new place, the disk reads it: playing from there is exact at once
    mem.send(Command::Play);
    st.send(Command::Play);
    serve_all(&feeder, &data);
    compare(&mut mem, &mut st, 4, &feeder);
    // Now a jump while playing without the disk: silence, reported
    let near = 6 * RATE as i64;
    mem.send(Command::Locate { sample: near });
    st.send(Command::Locate { sample: near });
    let a = mem.block().to_vec();
    let s = st.block().to_vec();
    assert!(a.iter().any(|x| *x != 0.0));
    let missing = st.underruns();
    assert!(missing > 0, "the underrun is reported");
    assert_eq!(s.iter().filter(|x| **x == 0.0).count() as u64, missing.min(BLOCK as u64));
    // Served again: exact
    compare(&mut mem, &mut st, 4, &feeder);
}

#[test]
fn a_slow_disk_gives_reported_silence_and_never_allocates() {
    let frames = 20 * RATE as usize;
    let data = source(frames);
    let (stream, feeder) = stream(StreamSpec::new(RATE, 1, frames as u64)).expect("stream");
    let stop = Arc::new(AtomicBool::new(false));
    // 20 ms per chunk of 8192 frames (170 ms of audio): slower than the render below, which never waits
    let reader = disk(feeder, data.clone(), Duration::from_millis(20), stop.clone());
    let mut mem = side(item(AudioSource::Memory(data.clone()), frames), None);
    let mut st = side(item(AudioSource::Stream(stream.clone()), frames), None);
    mem.send(Command::Play);
    st.send(Command::Play);
    let mut exact = 0usize;
    let mut silent = 0usize;
    for _ in 0..frames / BLOCK {
        let a = mem.block().to_vec();
        let s = st.block();
        for (x, y) in a.iter().zip(s) {
            if x.to_bits() == y.to_bits() {
                exact += 1;
            } else {
                assert_eq!(*y, 0.0, "what is not exact is silence");
                silent += 1;
            }
        }
    }
    let reported = st.underruns();
    assert!(silent > 0 && reported > 0);
    assert!(reported >= silent as u64, "every silent sample was reported ({reported} for {silent})");
    assert!(exact > 0);
    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();
}
