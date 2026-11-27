use super::*;

fn le16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn le32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    le32(&mut v, body.len() as u32);
    v.extend_from_slice(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut inner = kind.to_vec();
    inner.extend_from_slice(body);
    chunk(b"LIST", &inner)
}

fn name(s: &str) -> [u8; 20] {
    let mut n = [0u8; 20];
    n[..s.len()].copy_from_slice(s.as_bytes());
    n
}

fn put(v: &mut Vec<u8>, oper: u16, amount: i16) {
    le16(v, oper);
    v.extend_from_slice(&amount.to_le_bytes());
}

fn put_range(v: &mut Vec<u8>, oper: u16, lo: u8, hi: u8) {
    le16(v, oper);
    v.push(lo);
    v.push(hi);
}

/// A SoundFont with one 200-frame sample (a ramp) at 22050 Hz, looped 50..150, one preset "Test" (bank 0, program 5)
/// with an instrument of a global zone (attenuation 100 cB) and two zones: keys 0-59 and 60-127, the second with a
/// loop, a filter and a pan
pub(super) fn test_font() -> Vec<u8> {
    let mut smpl = Vec::new();
    for i in 0..200i16 {
        smpl.extend_from_slice(&(i * 100).to_le_bytes());
    }
    // (46 terminal samples of padding, as the specification asks)
    smpl.extend_from_slice(&[0u8; 92]);
    let sdta = list(b"sdta", &chunk(b"smpl", &smpl));

    let mut phdr = Vec::new();
    phdr.extend_from_slice(&name("Test"));
    le16(&mut phdr, 5);
    le16(&mut phdr, 0);
    le16(&mut phdr, 0);
    phdr.extend_from_slice(&[0u8; 12]);
    phdr.extend_from_slice(&name("EOP"));
    le16(&mut phdr, 0);
    le16(&mut phdr, 0);
    le16(&mut phdr, 1);
    phdr.extend_from_slice(&[0u8; 12]);

    let mut pbag = Vec::new();
    le16(&mut pbag, 0);
    le16(&mut pbag, 0);
    le16(&mut pbag, 1);
    le16(&mut pbag, 0);
    let mut pgen = Vec::new();
    put(&mut pgen, 41, 0);
    put(&mut pgen, 0, 0);
    let pmod = vec![0u8; 10];

    let mut inst = Vec::new();
    inst.extend_from_slice(&name("Inst"));
    le16(&mut inst, 0);
    inst.extend_from_slice(&name("EOI"));
    le16(&mut inst, 3);

    let mut ibag = Vec::new();
    for g in [0u16, 1, 5, 12] {
        le16(&mut ibag, g);
        le16(&mut ibag, 0);
    }
    let mut igen = Vec::new();
    // Global zone
    put(&mut igen, 48, 100);
    // Zone 1: keys 0-59, the sample
    put_range(&mut igen, 43, 0, 59);
    put(&mut igen, 58, 48);
    put(&mut igen, 53, 0);
    put(&mut igen, 0, 0);
    // (padding to keep the counts easy: zone 1 has 4 generators, so bag 2 starts at 5)
    // Zone 2: keys 60-127, loop, filter, pan
    put_range(&mut igen, 43, 60, 127);
    put(&mut igen, 54, 1);
    put(&mut igen, 8, 6000);
    put(&mut igen, 17, -500);
    put(&mut igen, 58, 60);
    put(&mut igen, 57, 3);
    put(&mut igen, 53, 0);
    put(&mut igen, 0, 0);
    let imod = vec![0u8; 10];

    let mut shdr = Vec::new();
    shdr.extend_from_slice(&name("Ramp"));
    le32(&mut shdr, 0);
    le32(&mut shdr, 200);
    le32(&mut shdr, 50);
    le32(&mut shdr, 150);
    le32(&mut shdr, 22_050);
    shdr.push(60);
    shdr.push(0);
    le16(&mut shdr, 0);
    le16(&mut shdr, 1);
    shdr.extend_from_slice(&name("EOS"));
    shdr.extend_from_slice(&[0u8; 26]);

    let mut pdta = Vec::new();
    for (id, body) in [
        (b"phdr", &phdr),
        (b"pbag", &pbag),
        (b"pmod", &pmod),
        (b"pgen", &pgen),
        (b"inst", &inst),
        (b"ibag", &ibag),
        (b"imod", &imod),
        (b"igen", &igen),
        (b"shdr", &shdr),
    ] {
        pdta.extend_from_slice(&chunk(id, body));
    }
    let pdta = list(b"pdta", &pdta);
    let mut body = b"sfbk".to_vec();
    body.extend_from_slice(&sdta);
    body.extend_from_slice(&pdta);
    chunk(b"RIFF", &body)
}

#[test]
fn lists_the_presets() {
    let bytes = test_font();
    let p = list_presets(&bytes).expect("parses");
    assert_eq!(p, vec![PresetInfo { bank: 0, program: 5, name: "Test".into() }]);
}

#[test]
fn imports_the_zones_with_their_generators() {
    let bytes = test_font();
    let i = import(&bytes, 0, 5, None).expect("imports");
    assert_eq!(i.zones.len(), 2);
    let (a, b) = (&i.zones[0], &i.zones[1]);
    assert_eq!(a.keys, (0, 59));
    assert_eq!(a.root_key, 48);
    assert_eq!(a.loop_mode, LoopMode::Off);
    // The global attenuation (100 cB: 10 dB)
    assert!((a.gain + 10.0).abs() < 1.0e-4);
    assert_eq!(b.keys, (60, 127));
    assert_eq!(b.loop_mode, LoopMode::Forward);
    assert_eq!((b.loop_start, b.loop_end), (50, 150));
    assert_eq!(b.group, 3);
    assert!((b.pan + 1.0).abs() < 1.0e-6);
    let f = b.filter.expect("a filter");
    // 6000 cents above 8.176 Hz
    assert!((f.cutoff - 8.175_798 * 2.0f32.powf(5.0)).abs() < 1.0);
    // The two zones share one decoded sample
    assert!(Arc::ptr_eq(&a.sample, &b.sample));
    assert_eq!(a.sample.frames(), 200);
    assert_eq!(a.sample.rate, 22_050);
}

#[test]
fn an_unknown_preset_is_an_error() {
    let bytes = test_font();
    assert!(import(&bytes, 1, 1, None).is_err());
}

#[test]
fn garbage_is_refused() {
    assert!(SoundFont::parse(b"RIFF....WAVE").is_err());
    assert!(SoundFont::parse(&[]).is_err());
}

/// The offset of a chunk's body in the font's bytes
fn body_of(bytes: &[u8], id: &[u8; 4]) -> usize {
    bytes.windows(4).position(|w| w == id).expect("the chunk is there") + 8
}

#[test]
fn bag_indices_that_go_backwards_are_refused() {
    let mut bytes = test_font();
    // The second instrument bag starts its generators after the third one
    let at = body_of(&bytes, b"ibag") + 4;
    bytes[at..at + 2].copy_from_slice(&20u16.to_le_bytes());
    assert!(SoundFont::parse(&bytes).is_err());
}

#[test]
fn a_sample_of_no_frames_is_an_error() {
    let mut bytes = test_font();
    // end = start
    let at = body_of(&bytes, b"shdr") + 24;
    bytes[at..at + 4].copy_from_slice(&0u32.to_le_bytes());
    assert!(import(&bytes, 0, 5, None).is_err());
}
