//! The recorder's file survives the process dying (I-11, U5, C10), and the disk space is reserved ahead (H3)

mod common;
use common::{TempDir, signal};
use lm_sys_sndfile::{Container, Existing, Reader, Sample, WriteFormat, WriteSpec, Writer};
use std::os::unix::fs::MetadataExt;

fn rf64(path: &std::path::Path) -> Writer {
    let spec = WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate: 48_000, channels: 2 };
    Writer::create(path, spec, Existing::Fail).unwrap()
}

#[test]
fn an_rf64_left_without_closing_reads_up_to_the_last_sync() {
    let dir = TempDir::new("crash");
    let path = dir.path("rec-0001.rf64");
    let data = signal(48_000 * 5, 2);
    let mut w = rf64(&path);
    // Two seconds, a checkpoint, two more, a checkpoint, then half a second more and the "crash"
    for (i, block) in data.chunks(512 * 2).enumerate() {
        w.write_f32(block).unwrap();
        let written = (i + 1) * 512;
        if written == 96_256 || written == 192_000 {
            w.sync().unwrap();
        }
        if written >= 216_000 {
            break;
        }
    }
    let synced = 192_000_u64;
    assert!(w.frames() > synced);
    // The process dies: no sf_close, no final header, the descriptor never closed
    std::mem::forget(w);

    let mut r = Reader::open(&path).unwrap();
    assert_eq!(r.info().container, Container::Rf64);
    assert_eq!(r.info().frames, synced, "the header says what was synced");
    let mut back = vec![0.0_f32; synced as usize * 2];
    assert_eq!(r.read_f32(&mut back).unwrap() as u64, synced);
    assert!(back.iter().zip(&data).all(|(a, b)| a.to_bits() == b.to_bits()), "the synced audio is intact");
}

#[test]
fn a_file_dropped_without_close_is_still_finished() {
    let dir = TempDir::new("drop");
    let path = dir.path("d.rf64");
    let mut w = rf64(&path);
    w.write_f32(&signal(10_000, 2)).unwrap();
    drop(w);
    assert_eq!(Reader::open(&path).unwrap().info().frames, 10_000);
}

#[test]
fn a_crash_before_any_sync_leaves_a_file_that_opens() {
    // The recorder syncs after its first block; before that, the file must still not be garbage
    let dir = TempDir::new("early");
    let path = dir.path("e.rf64");
    let mut w = rf64(&path);
    w.write_f32(&signal(1_000, 2)).unwrap();
    std::mem::forget(w);
    let r = Reader::open(&path);
    match r {
        Ok(r) => assert!(r.info().frames <= 1_000),
        Err(e) => assert!(e.key().starts_with("sndfile."), "{e}"),
    }
}

#[test]
fn space_is_reserved_ahead_without_changing_the_size() {
    let dir = TempDir::new("reserve");
    let path = dir.path("r.rf64");
    let mut w = rf64(&path);
    w.write_f32(&signal(4_800, 2)).unwrap();
    w.sync().unwrap();
    let size = w.file().metadata().unwrap().len();
    // 30 s of stereo float at 48 kHz ahead
    let ahead = 30 * 48_000 * 2 * 4;
    // APFS (macOS) and ext4 (the Linux runners' temp folder) both reserve
    w.reserve(size + ahead).unwrap();
    let meta = w.file().metadata().unwrap();
    assert_eq!(meta.len(), size, "the size is unchanged");
    let allocated = meta.blocks() * 512;
    assert!(allocated >= size + ahead, "{allocated} bytes allocated");
    // Calling again for the same total reserves nothing more
    w.reserve(size + ahead).unwrap();
    assert_eq!(w.file().metadata().unwrap().blocks() * 512, allocated);
    w.write_f32(&signal(4_800, 2)).unwrap();
    assert_eq!(w.close().unwrap(), 9_600);
    let r = Reader::open(&path).unwrap();
    assert_eq!(r.info().frames, 9_600, "the reservation does not show as audio");
    // An impossible reservation is the disk full, not a silent success
    let mut w = rf64(&dir.path("big.rf64"));
    let e = w.reserve(1 << 62).unwrap_err();
    assert!(["sndfile.disk_full", "sndfile.reserve_failed", "sndfile.reserve_unsupported"].contains(&e.key()), "{e}");
}
