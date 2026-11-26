use super::*;

/// A GUS patch of `samples`: (root frequency in milli-Hz, 16-bit frames, looped)
fn patch(samples: &[(u32, Vec<i16>, bool)]) -> Vec<u8> {
    let mut v = vec![0u8; 239];
    v[..22].copy_from_slice(b"GF1PATCH110\0ID#000002\0");
    v[82] = 1;
    v[151] = 1;
    v[198] = samples.len() as u8;
    for (root, frames, looped) in samples {
        let mut head = vec![0u8; 7 + 1];
        let data_len = (frames.len() * 2) as u32;
        head.extend_from_slice(&data_len.to_le_bytes());
        head.extend_from_slice(&(20u32).to_le_bytes());
        head.extend_from_slice(&(data_len - 8).to_le_bytes());
        head.extend_from_slice(&22_050u16.to_le_bytes());
        head.extend_from_slice(&[0u8; 8]);
        head.extend_from_slice(&root.to_le_bytes());
        head.extend_from_slice(&[0u8; 2 + 1 + 12 + 3 + 3]);
        head.push(1 | if *looped { 4 } else { 0 });
        head.extend_from_slice(&[0u8; 2 + 2 + 36]);
        v.extend_from_slice(&head);
        for f in frames {
            v.extend_from_slice(&f.to_le_bytes());
        }
    }
    v
}

#[test]
fn each_sample_gets_the_keys_nearest_its_pitch() {
    // 261.626 Hz (C4) and 523.251 Hz (C5)
    let bytes = patch(&[(261_626, vec![1000; 64], false), (523_251, vec![2000; 64], true)]);
    let i = import(&bytes, PatOptions::default()).expect("imports");
    assert_eq!(i.zones.len(), 2);
    assert_eq!(i.zones[0].keys.0, 0);
    assert_eq!(i.zones[1].keys.1, 127);
    // Contiguous, covering the keyboard
    assert_eq!(i.zones[0].keys.1 + 1, i.zones[1].keys.0);
    assert_eq!(i.zones[0].root_key, 60);
    assert_eq!(i.zones[1].root_key, 72);
    assert_eq!(i.zones[0].loop_mode, LoopMode::Off);
    assert_eq!(i.zones[1].loop_mode, LoopMode::Forward);
}

#[test]
fn untuned_samples_do_not_follow_the_keys() {
    let bytes = patch(&[(261_626, vec![1000; 64], false)]);
    let i = import(&bytes, PatOptions { tuned: false, looped: true }).expect("imports");
    assert_eq!(i.zones[0].key_track, 0.0);
}

#[test]
fn a_looped_off_plays_through() {
    let bytes = patch(&[(261_626, vec![1000; 64], true)]);
    let i = import(&bytes, PatOptions { tuned: true, looped: false }).expect("imports");
    assert_eq!(i.zones[0].loop_mode, LoopMode::Off);
}

#[test]
fn not_a_patch_is_the_legacys_error() {
    let e = import(&[0u8; 300], PatOptions::default()).expect_err("fails");
    assert_eq!(e.key(), "patman.not_gus");
}
