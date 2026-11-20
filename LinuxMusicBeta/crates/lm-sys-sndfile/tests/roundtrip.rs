//! Writing and reading back with the real libsndfile

mod common;
use common::{TempDir, int16_values, signal};
use lm_sys_sndfile::{
    Container, Encoding, Existing, FlacDepth, Reader, Sample, WriteFormat, WriteSpec, Writer, is_supported, version,
};

fn write_f32(path: &std::path::Path, format: WriteFormat, channels: usize, data: &[f32]) {
    let spec = WriteSpec { format, rate: 48_000, channels };
    let mut w = Writer::create(path, spec, Existing::Fail).unwrap();
    // In blocks, like the recorder
    for block in data.chunks(1024 * channels) {
        w.write_f32(block).unwrap();
    }
    assert_eq!(w.close().unwrap(), (data.len() / channels) as u64);
}

fn read_all_f32(path: &std::path::Path) -> (Reader, Vec<f32>) {
    let mut r = Reader::open(path).unwrap();
    let ch = r.info().channels;
    let mut out = vec![0.0_f32; r.info().frames as usize * ch];
    let mut at = 0;
    loop {
        let end = (at + 777 * ch).min(out.len());
        let got = r.read_f32(&mut out[at..end]).unwrap();
        at += got * ch;
        if got == 0 {
            break;
        }
    }
    assert_eq!(at, out.len());
    (r, out)
}

#[test]
fn the_library_is_at_least_1_1() {
    let v = version();
    assert!(v.starts_with("libsndfile-1."), "{v}");
    let minor: u32 = v["libsndfile-1.".len()..].split('.').next().unwrap().parse().unwrap();
    assert!(minor >= 1, "{v}: MP3 needs 1.1");
}

#[test]
fn float32_wav_and_rf64_are_bit_exact() {
    let dir = TempDir::new("f32");
    let mut data = signal(48_000, 2);
    // The awkward values too: zero, negative zero, subnormals, beyond full scale, the extremes
    data.extend_from_slice(&[0.0, -0.0, f32::from_bits(1), -f32::from_bits(1), 1.5, -2.0, f32::MAX, f32::MIN]);
    for (name, format, container) in [
        ("a.wav", WriteFormat::Wav(Sample::Float32), Container::Wav),
        ("a.rf64", WriteFormat::Rf64(Sample::Float32), Container::Rf64),
        ("a.aiff", WriteFormat::Aiff(Sample::Float32), Container::Aiff),
    ] {
        let path = dir.path(name);
        write_f32(&path, format, 2, &data);
        let (r, back) = read_all_f32(&path);
        let info = *r.info();
        assert_eq!((info.rate, info.channels, info.frames), (48_000, 2, 48_004));
        assert_eq!((info.container, info.encoding), (container, Encoding::Float32));
        assert!(info.seekable);
        let same = data.iter().zip(&back).all(|(a, b)| a.to_bits() == b.to_bits());
        assert!(same, "{name}: float samples changed");
    }
}

#[test]
fn integer_formats_are_bit_exact_at_their_depth() {
    let dir = TempDir::new("int");
    let values = int16_values(65_536 * 2 + 10);
    for (name, format) in [
        ("16.wav", WriteFormat::Wav(Sample::Int16)),
        ("16.rf64", WriteFormat::Rf64(Sample::Int16)),
        ("16.aiff", WriteFormat::Aiff(Sample::Int16)),
        ("16.flac", WriteFormat::Flac { depth: FlacDepth::Int16, compression: 0.5 }),
    ] {
        let path = dir.path(name);
        let mut w = Writer::create(&path, WriteSpec { format, rate: 44_100, channels: 2 }, Existing::Fail).unwrap();
        w.write_i16(&values).unwrap();
        w.close().unwrap();
        let mut r = Reader::open(&path).unwrap();
        assert_eq!(r.info().encoding, Encoding::Pcm16, "{name}");
        let mut back = vec![0_i16; values.len()];
        assert_eq!(r.read_i16(&mut back).unwrap(), values.len() / 2);
        assert_eq!(back, values, "{name}");
    }
    // 24 bits: every value that fits in the top 24 bits of an i32
    let values: Vec<i32> = (0..200_000_i64).map(|i| (((i * 83) % (1 << 24)) - (1 << 23)) as i32 * 256).collect();
    for (name, format) in [
        ("24.wav", WriteFormat::Wav(Sample::Int24)),
        ("24.rf64", WriteFormat::Rf64(Sample::Int24)),
        ("24.flac", WriteFormat::Flac { depth: FlacDepth::Int24, compression: 1.0 }),
    ] {
        let path = dir.path(name);
        let mut w = Writer::create(&path, WriteSpec { format, rate: 96_000, channels: 1 }, Existing::Fail).unwrap();
        w.write_i32(&values).unwrap();
        w.close().unwrap();
        let mut r = Reader::open(&path).unwrap();
        assert_eq!((r.info().encoding, r.info().rate), (Encoding::Pcm24, 96_000), "{name}");
        let mut back = vec![0_i32; values.len()];
        assert_eq!(r.read_i32(&mut back).unwrap(), values.len());
        assert_eq!(back, values, "{name}");
    }
}

#[test]
fn float_into_integer_files_scales_by_powers_of_two_and_clips() {
    // A float k/32768 written to 16 bits reads back exactly; beyond full scale it clips instead of wrapping
    let dir = TempDir::new("scale");
    let path = dir.path("s.wav");
    let mut data: Vec<f32> = (-32_768..32_768).step_by(7).map(|k| k as f32 / 32_768.0).collect();
    data.extend_from_slice(&[1.5, -1.5]);
    write_f32(&path, WriteFormat::Wav(Sample::Int16), 1, &data);
    let (_, back) = read_all_f32(&path);
    let n = data.len() - 2;
    let worst = data[..n].iter().zip(&back[..n]).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
    assert!(worst <= 1.0 / 32_768.0, "worst {worst}");
    assert!(back[n] > 0.99 && back[n + 1] < -0.99, "clipped, not wrapped: {:?}", &back[n..]);
}

#[test]
fn lossy_formats_keep_the_signal() {
    let dir = TempDir::new("lossy");
    let data = signal(48_000, 2);
    let mut formats = vec![("a.ogg", WriteFormat::OggVorbis { quality: 0.8 }, Container::Ogg, Encoding::Vorbis)];
    if is_supported(WriteFormat::Mp3 { quality: 0.8 }, 48_000, 2) {
        formats.push(("a.mp3", WriteFormat::Mp3 { quality: 0.8 }, Container::Mpeg, Encoding::Mp3));
    } else {
        panic!("this libsndfile cannot write MP3: the import test of MP3 needs a file");
    }
    for (name, format, container, encoding) in formats {
        let path = dir.path(name);
        write_f32(&path, format, 2, &data);
        let (r, back) = read_all_f32(&path);
        let info = *r.info();
        assert_eq!(
            (info.container, info.encoding, info.channels, info.rate),
            (container, encoding, 2, 48_000),
            "{name}"
        );
        assert!(!encoding.is_lossless());
        // The length within one codec frame, and the signal there after the codec's own delay
        assert!((info.frames as i64 - 48_000).abs() <= 2_304, "{name}: {} frames", info.frames);
        let best = (0..3_000)
            .map(|lag| {
                let n = 40_000.min(back.len() / 2 - lag);
                let (mut err, mut sig) = (0.0_f64, 0.0_f64);
                for i in 2_000..n {
                    let a = f64::from(data[i * 2]);
                    let b = f64::from(back[(i + lag) * 2]);
                    err += (a - b).powi(2);
                    sig += a * a;
                }
                10.0 * (err / sig).log10()
            })
            .fold(f64::INFINITY, f64::min);
        assert!(best < -20.0, "{name}: the decoded signal is {best:.1} dB from the original");
    }
}

#[test]
fn seek_and_read_from_the_middle() {
    let dir = TempDir::new("seek");
    let path = dir.path("s.flac");
    let data: Vec<i16> = (0..100_000).map(|i| (i % 30_000) as i16).collect();
    let spec = WriteSpec {
        format: WriteFormat::Flac { depth: FlacDepth::Int16, compression: 0.0 },
        rate: 48_000,
        channels: 1,
    };
    let mut w = Writer::create(&path, spec, Existing::Fail).unwrap();
    w.write_i16(&data).unwrap();
    w.close().unwrap();
    let mut r = Reader::open(&path).unwrap();
    for at in [0_u64, 1, 4_095, 4_096, 77_777, 99_999] {
        r.seek(at).unwrap();
        assert_eq!(r.position(), at);
        let mut one = [0_i16; 1];
        assert_eq!(r.read_i16(&mut one).unwrap(), 1);
        assert_eq!(one[0], data[at as usize]);
    }
    r.seek(100_000).unwrap();
    assert_eq!(r.read_i16(&mut [0; 10]).unwrap(), 0, "the end reads nothing");
    assert_eq!(r.seek(100_001).unwrap_err().key(), "sndfile.seek_failed");
}

#[test]
fn readers_and_writers_move_between_threads() {
    let dir = TempDir::new("threads");
    let path = dir.path("t.rf64");
    let spec = WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate: 48_000, channels: 2 };
    let w = Writer::create(&path, spec, Existing::Fail).unwrap();
    let w = std::thread::spawn(move || {
        let mut w = w;
        w.write_f32(&[0.5; 2_000]).unwrap();
        w
    })
    .join()
    .unwrap();
    w.close().unwrap();
    let r = Reader::open(&path).unwrap();
    let frames = std::thread::spawn(move || r.info().frames).join().unwrap();
    assert_eq!(frames, 1_000);
}

#[test]
fn the_same_samples_give_the_same_bytes_a_second_later() {
    // libsndfile's PEAK chunk carries the wall-clock time: with it, two exports of the same song differed whenever
    // they straddled a second (T19)
    let dir = TempDir::new("same-bytes");
    let data = signal(4_800, 2);
    for format in
        [WriteFormat::Wav(Sample::Float32), WriteFormat::Rf64(Sample::Float32), WriteFormat::Aiff(Sample::Float32)]
    {
        let a = dir.path(&format!("a-{format:?}"));
        let b = dir.path(&format!("b-{format:?}"));
        write_f32(&a, format, 2, &data);
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        write_f32(&b, format, 2, &data);
        assert!(std::fs::read(&a).unwrap() == std::fs::read(&b).unwrap(), "{format:?}");
    }
}
