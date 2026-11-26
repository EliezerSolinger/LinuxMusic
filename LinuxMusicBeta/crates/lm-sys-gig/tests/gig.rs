//! lm-sys-gig against a `.gig` the test writes (`test_file`): listing, loading, the data of a dimension region, the
//! selection, and a file that is not a `.gig`

use lm_sys_gig::{Patch, instruments, test_file};
use std::path::PathBuf;

fn file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("lm-sys-gig-{}-{name}.gig", std::process::id()))
}

#[test]
fn the_instruments_of_a_file_are_listed() {
    let path = file("list");
    test_file(&path).expect("the test file");
    let list = instruments(&path).expect("the list");
    assert_eq!(list.len(), 2);
    assert_eq!((list[0].bank, list[0].program), (0, 0));
    assert_eq!(list[0].name, "Sine");
    assert_eq!((list[1].bank, list[1].program), (0, 1));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_patch_loads_with_its_sample_loop_and_envelope() {
    let path = file("load");
    test_file(&path).expect("the test file");
    let patch = Patch::load(&path, 0, 0).expect("the patch");
    assert_eq!(patch.regions().len(), 1);
    let region = &patch.regions()[0];
    assert_eq!((region.key_low, region.key_high), (0, 127));
    let mut values = [0u32; 8];
    values[0] = 100;
    let chosen = patch.select(0, &values).expect("a dimension region");
    let dimregion = patch.dimregion(chosen).expect("its data");
    assert!(dimregion.pitch_track);
    assert_eq!(dimregion.unity_note, 69);
    let looped = dimregion.loop_.expect("a loop");
    assert_eq!((looped.start, looped.length), (0, 2000));
    assert!(dimregion.envelope.infinite_sustain);
    assert!((dimregion.envelope.sustain - 1000.0).abs() < 1.0);
    let sample = patch.sample(dimregion.sample.expect("a sample")).expect("its info");
    assert_eq!((sample.rate, sample.channels, sample.frames), (44_000, 1, 2000));
    let data = patch.sample_data(dimregion.sample.unwrap());
    assert_eq!(data.len(), 2000);
    let peak = data.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.1 && peak < 0.5, "the sine is half of full scale at most: {peak}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_missing_instrument_and_a_file_that_is_not_a_gig_are_errors() {
    let path = file("errors");
    test_file(&path).expect("the test file");
    assert_eq!(Patch::load(&path, 3, 99).unwrap_err().key(), "gig.no_instrument");
    let _ = std::fs::remove_file(&path);
    let junk = file("junk");
    std::fs::write(&junk, b"not a gig file at all").expect("write");
    assert_eq!(instruments(&junk).unwrap_err().key(), "gig.open_failed");
    assert_eq!(Patch::load(&junk, 0, 0).unwrap_err().key(), "gig.open_failed");
    let _ = std::fs::remove_file(junk);
}
