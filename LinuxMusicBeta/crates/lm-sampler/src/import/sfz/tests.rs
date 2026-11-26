use std::path::Path;

use super::*;

struct Fake;

impl SampleLoader for Fake {
    fn load(&mut self, path: &Path) -> Result<Pcm> {
        let frames = if path.ends_with("long.wav") { 1000 } else { 100 };
        Ok(Pcm::mono(44100, vec![0.5; frames]))
    }
}

#[test]
fn note_names() {
    assert_eq!(parse_key("c4"), Some(60));
    assert_eq!(parse_key("a4"), Some(69));
    assert_eq!(parse_key("f#3"), Some(54));
    assert_eq!(parse_key("eb2"), Some(39));
    assert_eq!(parse_key("c-1"), Some(0));
    assert_eq!(parse_key("72"), Some(72));
    assert_eq!(parse_key("x"), None);
}

#[test]
fn headers_inherit_down_to_the_region() {
    let text = "
        <global> volume=-3 ampeg_release=0.5
        <group> lokey=c3 hikey=c4 loop_mode=loop_continuous
        <region> sample=a.wav pitch_keycenter=c3 loop_start=10 loop_end=89
        <region> sample=sub\\long.wav lokey=c5 hikey=c5 volume=-6 tune=12
    ";
    let i = import(text, Path::new("/x"), &mut Fake).expect("imports");
    assert_eq!(i.zones.len(), 2);
    let (a, b) = (&i.zones[0], &i.zones[1]);
    assert_eq!(a.keys, (48, 60));
    assert_eq!(a.root_key, 48);
    assert_eq!(a.loop_mode, LoopMode::Forward);
    assert_eq!((a.loop_start, a.loop_end), (10, 90));
    assert!((a.gain + 3.0).abs() < 1e-6);
    assert!((a.amp.release - 0.5).abs() < 1e-6);
    assert_eq!(b.keys, (72, 72));
    assert!((b.gain + 6.0).abs() < 1e-6);
    assert_eq!(b.tune, 12.0);
    // The group's loop mode is inherited too
    assert_eq!(b.loop_mode, LoopMode::Forward);
}

#[test]
fn defines_comments_and_default_path() {
    let text = "
        // a comment
        #define $KEY c4
        <control> default_path=samples/
        <region> sample=one.wav key=$KEY /* inline */ lovel=64
    ";
    let i = import(text, Path::new("/x"), &mut Fake).expect("imports");
    assert_eq!(i.zones[0].keys, (60, 60));
    assert_eq!(i.zones[0].vels.0, 64);
}

#[test]
fn groups_triggers_and_sequences() {
    let text = "
        <region> sample=a.wav key=36 group=1 off_by=1 off_mode=fast seq_length=2 seq_position=1
        <region> sample=a.wav key=36 group=1 off_by=1 seq_length=2 seq_position=2
        <region> sample=r.wav key=36 trigger=release rt_decay=3 lorand=0.5 hirand=1
    ";
    let i = import(text, Path::new("/x"), &mut Fake).expect("imports");
    assert!(i.zones[0].off_fast && !i.zones[1].off_fast);
    assert_eq!(i.zones[0].sequence.map(|s| (s.length, s.position)), Some((2, 1)));
    assert_eq!(i.zones[2].trigger, Trigger::Release);
    assert_eq!(i.zones[2].random, Some((0.5, 1.0)));
    assert!((i.zones[2].release_decay - 3.0).abs() < 1e-6);
    // Two regions read the same file: one shared sample
    assert!(Arc::ptr_eq(&i.zones[0].sample, &i.zones[1].sample));
}

#[test]
fn a_filter_and_pan() {
    let text = "<region> sample=a.wav fil_type=hpf_2p cutoff=500 resonance=6 pan=-50 fileg_depth=1200 fileg_decay=0.3";
    let i = import(text, Path::new("/x"), &mut Fake).expect("imports");
    let f = i.zones[0].filter.expect("filter");
    assert_eq!(f.kind, FilterKind::HighPass);
    assert_eq!(f.cutoff, 500.0);
    assert_eq!(f.env_depth, 1200.0);
    assert!((i.zones[0].pan + 0.5).abs() < 1e-6);
}

#[test]
fn no_sample_regions_is_an_error() {
    assert!(import("<region> key=60", Path::new("/x"), &mut Fake).is_err());
}
