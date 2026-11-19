//! Errors are values with stable keys

mod common;
use common::TempDir;
use lm_sys_sndfile::{Existing, FlacDepth, Reader, Sample, WriteFormat, WriteSpec, Writer, is_supported};

#[test]
fn opening_what_is_not_audio() {
    let dir = TempDir::new("errors");
    assert_eq!(Reader::open(&dir.path("missing.wav")).unwrap_err().key(), "sndfile.open_failed");
    let text = dir.path("text.wav");
    std::fs::write(&text, b"this is not a sound file, only some text that is long enough to be looked at").unwrap();
    let e = Reader::open(&text).unwrap_err();
    assert!(["sndfile.unrecognised_format", "sndfile.malformed"].contains(&e.key()), "{e}");
    assert_eq!(e.label.args[0].0, "path");
    let empty = dir.path("empty.wav");
    std::fs::write(&empty, b"").unwrap();
    assert!(Reader::open(&empty).unwrap_err().key().starts_with("sndfile."));
}

#[test]
fn writing_where_it_must_not() {
    let dir = TempDir::new("werrors");
    let spec = WriteSpec { format: WriteFormat::Wav(Sample::Float32), rate: 48_000, channels: 2 };
    let path = dir.path("x.wav");
    Writer::create(&path, spec, Existing::Fail).unwrap().close().unwrap();
    assert_eq!(Writer::create(&path, spec, Existing::Fail).unwrap_err().key(), "sndfile.exists");
    Writer::create(&path, spec, Existing::Replace).unwrap();
    assert_eq!(
        Writer::create(&dir.path("no/such/dir.wav"), spec, Existing::Fail).unwrap_err().key(),
        "sndfile.open_failed"
    );
    let mut w = Writer::create(&dir.path("y.wav"), spec, Existing::Fail).unwrap();
    assert_eq!(w.write_f32(&[0.0; 3]).unwrap_err().key(), "sndfile.bad_length");
    let bad = WriteSpec {
        format: WriteFormat::Flac { depth: FlacDepth::Int16, compression: 0.5 },
        rate: 48_000,
        channels: 99,
    };
    assert!(!is_supported(bad.format, bad.rate, bad.channels));
    assert_eq!(
        Writer::create(&dir.path("z.flac"), bad, Existing::Fail).unwrap_err().key(),
        "sndfile.unsupported_format"
    );
    let bad = WriteSpec { format: WriteFormat::OggVorbis { quality: 2.0 }, rate: 48_000, channels: 2 };
    assert_eq!(Writer::create(&dir.path("q.ogg"), bad, Existing::Fail).unwrap_err().key(), "sndfile.bad_setting");
}

#[test]
fn opens_from_many_threads_at_once() {
    let dir = TempDir::new("parallel");
    let spec = WriteSpec { format: WriteFormat::Rf64(Sample::Int24), rate: 44_100, channels: 1 };
    let good = dir.path("good.rf64");
    let mut w = Writer::create(&good, spec, Existing::Fail).unwrap();
    w.write_f32(&[0.25; 100]).unwrap();
    w.close().unwrap();
    let bad = dir.path("bad.wav");
    std::fs::write(&bad, vec![7_u8; 4096]).unwrap();
    std::thread::scope(|s| {
        for i in 0..8 {
            let (good, bad) = (&good, &bad);
            s.spawn(move || {
                for _ in 0..50 {
                    if i % 2 == 0 {
                        assert_eq!(Reader::open(good).unwrap().info().frames, 100);
                    } else {
                        assert_ne!(Reader::open(bad).unwrap_err().key(), "sndfile.open_failed");
                    }
                }
            });
        }
    });
}
