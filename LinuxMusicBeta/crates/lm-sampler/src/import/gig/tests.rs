//! The importer's tests, on `.gig` files written here by [`write_gig`] (the same layout libgig reads: version 3, the
//! 1092-byte `3lnk`, the 148-byte `3ewa`, 32-bit wave pool offsets).

use super::*;
use crate::engine::{Engine, EngineConfig};

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

/// The seconds as the file stores them (0 is the most negative value)
fn enc(seconds: f64) -> i32 {
    if seconds <= 0.0 { i32::MIN } else { (libm::log(seconds) / libm::log(1.000_000_008_813_822)) as i32 }
}

/// A dimension of a region
#[derive(Clone, Copy)]
struct DimSpec {
    kind: u8,
    bits: u8,
    zones: u8,
}

/// A dimension region
#[derive(Clone, Copy)]
struct SlotSpec {
    wave: u32,
    unity: u16,
    fine: i16,
    /// The `3ewa`'s zone limit of dimension 0 (0: none, even zones)
    upper0: u8,
    release_decay: u8,
    /// A loop: start and length
    looped: Option<(u32, u32)>,
}

impl SlotSpec {
    fn new(wave: u32) -> Self {
        Self { wave, unity: 69, fine: 0, upper0: 0, release_decay: 0, looped: None }
    }
}

struct RegionSpec {
    keys: (u16, u16),
    dims: Vec<DimSpec>,
    slots: Vec<SlotSpec>,
}

struct InsSpec {
    name: &'static str,
    bank: u32,
    program: u32,
    regions: Vec<RegionSpec>,
}

fn wsmp(s: &SlotSpec) -> Vec<u8> {
    let mut v = Vec::new();
    le32(&mut v, 20);
    le16(&mut v, s.unity);
    v.extend_from_slice(&s.fine.to_le_bytes());
    le32(&mut v, 0);
    le32(&mut v, 0);
    le32(&mut v, u32::from(s.looped.is_some()));
    if let Some((start, length)) = s.looped {
        le32(&mut v, 16);
        le32(&mut v, 0);
        le32(&mut v, start);
        le32(&mut v, length);
    }
    v
}

fn ewa(s: &SlotSpec) -> Vec<u8> {
    let mut b = vec![0u8; 148];
    let mut put32 = |at: usize, x: i32| b[at..at + 4].copy_from_slice(&x.to_le_bytes());
    put32(28, enc(0.0));
    put32(32, enc(0.0));
    put32(40, enc(0.1));
    b[38..40].copy_from_slice(&1000u16.to_le_bytes());
    b[96] = 5; // linear, depth 0
    b[128] = s.release_decay;
    b[140] = s.upper0;
    b[141..148].fill(127);
    b
}

/// A `.gig` of mono 16-bit waves at 44 100 Hz (the flag marks a wave compressed)
fn write_gig(waves: &[(Vec<i16>, bool)], instruments: &[InsSpec]) -> Vec<u8> {
    let mut wvpl = Vec::new();
    let mut offsets = Vec::new();
    for (pcm, compressed) in waves {
        offsets.push(wvpl.len() as u32);
        let mut fmt = Vec::new();
        le16(&mut fmt, 1);
        le16(&mut fmt, 1);
        le32(&mut fmt, 44_100);
        le32(&mut fmt, 88_200);
        le16(&mut fmt, 2);
        le16(&mut fmt, 16);
        let data: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut body = chunk(b"fmt ", &fmt);
        body.extend(chunk(b"data", &data));
        if *compressed {
            body.extend(chunk(b"EWAV", &[0u8; 8]));
        }
        wvpl.extend(list(b"wave", &body));
    }
    let mut ptbl = Vec::new();
    le32(&mut ptbl, 8);
    le32(&mut ptbl, offsets.len() as u32);
    offsets.iter().for_each(|o| le32(&mut ptbl, *o));
    let mut lins = Vec::new();
    for ins in instruments {
        let mut insh = Vec::new();
        le32(&mut insh, ins.regions.len() as u32);
        le32(&mut insh, ins.bank);
        le32(&mut insh, ins.program);
        let mut name = ins.name.as_bytes().to_vec();
        name.push(0);
        let mut lrgn = Vec::new();
        for r in &ins.regions {
            let mut rgnh = Vec::new();
            le16(&mut rgnh, r.keys.0);
            le16(&mut rgnh, r.keys.1);
            le16(&mut rgnh, 0);
            le16(&mut rgnh, 127);
            le16(&mut rgnh, 0);
            le16(&mut rgnh, 0);
            le16(&mut rgnh, 0);
            let mut lnk = vec![0u8; 1092];
            lnk[..4].copy_from_slice(&(r.slots.len() as u32).to_le_bytes());
            for (i, d) in r.dims.iter().enumerate() {
                lnk[4 + i * 8] = d.kind;
                lnk[5 + i * 8] = d.bits;
                lnk[8 + i * 8] = d.zones;
            }
            let mut prg = Vec::new();
            for (i, s) in r.slots.iter().enumerate() {
                lnk[68 + i * 4..72 + i * 4].copy_from_slice(&s.wave.to_le_bytes());
                let mut ewl = chunk(b"3ewa", &ewa(s));
                ewl.extend(chunk(b"wsmp", &wsmp(s)));
                prg.extend(list(b"3ewl", &ewl));
            }
            let mut body = chunk(b"rgnh", &rgnh);
            body.extend(chunk(b"3lnk", &lnk));
            body.extend(list(b"3prg", &prg));
            lrgn.extend(list(b"rgn ", &body));
        }
        let mut body = chunk(b"insh", &insh);
        body.extend(list(b"INFO", &chunk(b"INAM", &name)));
        body.extend(list(b"lrgn", &lrgn));
        lins.extend(list(b"ins ", &body));
    }
    let mut vers = Vec::new();
    for x in [0u16, 3, 0, 0] {
        le16(&mut vers, x);
    }
    let mut colh = Vec::new();
    le32(&mut colh, instruments.len() as u32);
    let mut body = b"DLS ".to_vec();
    body.extend(chunk(b"vers", &vers));
    body.extend(chunk(b"colh", &colh));
    body.extend(list(b"lins", &lins));
    body.extend(chunk(b"ptbl", &ptbl));
    body.extend(list(b"wvpl", &wvpl));
    let mut file = b"RIFF".to_vec();
    le32(&mut file, body.len() as u32);
    file.extend(body);
    file
}

fn sine(freq: f32, frames: usize) -> Vec<i16> {
    (0..frames).map(|i| (16_384.0 * libm::sinf(std::f32::consts::TAU * freq * i as f32 / 44_100.0)) as i16).collect()
}

fn region(dims: Vec<DimSpec>, slots: Vec<SlotSpec>) -> RegionSpec {
    RegionSpec { keys: (0, 127), dims, slots }
}

const VELOCITY: DimSpec = DimSpec { kind: 0x82, bits: 1, zones: 2 };

fn one(name: &'static str, program: u32, regions: Vec<RegionSpec>) -> InsSpec {
    InsSpec { name, bank: 0, program, regions }
}

fn two_waves() -> Vec<(Vec<i16>, bool)> {
    vec![(sine(440.0, 2000), false), (sine(880.0, 2000), false)]
}

#[test]
fn the_instruments_are_listed() {
    let file = write_gig(
        &two_waves(),
        &[
            one("Sine", 0, vec![region(vec![], vec![SlotSpec::new(0)])]),
            InsSpec { bank: 1, ..one("Sine up", 5, vec![region(vec![], vec![SlotSpec::new(1)])]) },
        ],
    );
    let presets = list_presets(&file).expect("presets");
    assert_eq!(
        presets,
        vec![
            PresetInfo { bank: 0, program: 0, name: "Sine".into() },
            PresetInfo { bank: 1, program: 5, name: "Sine up".into() },
        ]
    );
}

#[test]
fn a_single_region_keeps_its_pitch_loop_and_envelope() {
    let mut slot = SlotSpec::new(0);
    slot.fine = 10;
    slot.looped = Some((10, 1000));
    let file = write_gig(&two_waves(), &[one("Sine", 0, vec![region(vec![], vec![slot])])]);
    let i = import(&file, 0, 0).expect("import");
    assert_eq!(i.zones.len(), 1);
    let z = &i.zones[0];
    assert_eq!((z.keys, z.vels), ((0, 127), (1, 127)));
    assert_eq!(z.root_key, 69);
    assert_eq!(z.key_track, 100.0);
    assert_eq!(z.tune, 10.0);
    assert_eq!(z.loop_mode, LoopMode::Forward);
    assert_eq!((z.loop_start, z.loop_end), (10, 1010));
    assert_eq!(z.sample.frames(), 2000);
    assert!((z.amp.sustain - 1.0).abs() < 1.0e-6);
    assert!(z.amp.attack < 1.0e-6);
    // The half-scale decode: 16384 / 65536 at the peak
    let peak = z.sample.left().iter().fold(0.0f32, |a, x| a.max(x.abs()));
    assert!((peak - 0.25).abs() < 0.01);
}

#[test]
fn velocity_zones_follow_the_files_limits_or_split_evenly() {
    let mut low = SlotSpec::new(0);
    low.upper0 = 40;
    let high = SlotSpec::new(1);
    let file = write_gig(&two_waves(), &[one("Vel", 0, vec![region(vec![VELOCITY], vec![low, high])])]);
    let i = import(&file, 0, 0).expect("import");
    let mut ranges: Vec<(u8, u8)> = i.zones.iter().map(|z| z.vels).collect();
    ranges.sort_unstable();
    assert_eq!(ranges, vec![(1, 40), (41, 127)]);
    // No limits: even zones of 64
    let file = write_gig(
        &two_waves(),
        &[one("Vel", 0, vec![region(vec![VELOCITY], vec![SlotSpec::new(0), SlotSpec::new(1)])])],
    );
    let i = import(&file, 0, 0).expect("import");
    let mut ranges: Vec<(u8, u8)> = i.zones.iter().map(|z| z.vels).collect();
    ranges.sort_unstable();
    assert_eq!(ranges, vec![(1, 63), (64, 127)]);
}

#[test]
fn the_velocity_curve_is_the_files() {
    let file = write_gig(&two_waves(), &[one("Sine", 0, vec![region(vec![], vec![SlotSpec::new(0)])])]);
    let i = import(&file, 0, 0).expect("import");
    let t = i.zones[0].vel_table.as_ref().expect("table");
    // Linear, depth 0: the level is the velocity
    assert!((t[127] - 1.0).abs() < 1.0e-3);
    assert!((t[64] - 64.0 / 127.0).abs() < 1.0e-2);
    assert_eq!(t[0], 0.0);
}

#[test]
fn the_release_trigger_dimension_makes_release_zones() {
    let release = DimSpec { kind: 0x84, bits: 1, zones: 2 };
    let mut r = SlotSpec::new(1);
    r.release_decay = 2;
    let file = write_gig(&two_waves(), &[one("Rel", 0, vec![region(vec![release], vec![SlotSpec::new(0), r])])]);
    let i = import(&file, 0, 0).expect("import");
    assert_eq!(i.zones.len(), 2);
    let attack = i.zones.iter().find(|z| z.trigger == Trigger::Attack).expect("attack");
    let rel = i.zones.iter().find(|z| z.trigger == Trigger::Release).expect("release");
    assert_eq!(attack.sample.frames(), 2000);
    assert_eq!(rel.loop_mode, LoopMode::Off);
    assert!(rel.release_decay > 0.0);
}

#[test]
fn the_sustain_pedal_dimension_selects_by_the_pedal() {
    let pedal = DimSpec { kind: 0x40, bits: 1, zones: 2 };
    let file =
        write_gig(&two_waves(), &[one("Ped", 0, vec![region(vec![pedal], vec![SlotSpec::new(0), SlotSpec::new(1)])])]);
    let i = import(&file, 0, 0).expect("import");
    let up = i.zones.iter().find(|z| z.pedal == Some(false)).expect("up");
    let down = i.zones.iter().find(|z| z.pedal == Some(true)).expect("down");
    assert!(!Arc::ptr_eq(&up.sample, &down.sample));
    // The engine plays the pedal-up zone, and the pedal-down one once the pedal is down
    let mut e = Engine::new(Arc::new(i), EngineConfig::new(44_100, 256));
    e.note_on(1, 69, 0.8);
    e.render(256);
    assert!(e.bus(0).0.iter().any(|x| x.abs() > 1.0e-4));
}

#[test]
fn round_robin_zones_take_turns() {
    let rr = DimSpec { kind: 0x86, bits: 1, zones: 2 };
    let file =
        write_gig(&two_waves(), &[one("RR", 0, vec![region(vec![rr], vec![SlotSpec::new(0), SlotSpec::new(1)])])]);
    let i = import(&file, 0, 0).expect("import");
    let mut seq: Vec<(u16, u16, u16)> =
        i.zones.iter().filter_map(|z| z.sequence).map(|s| (s.group, s.length, s.position)).collect();
    seq.sort_unstable();
    assert_eq!(seq, vec![(0, 2, 1), (0, 2, 2)]);
}

#[test]
fn a_compressed_sample_is_refused_with_a_clear_key() {
    let waves = vec![(sine(440.0, 100), true)];
    let file = write_gig(&waves, &[one("Packed", 0, vec![region(vec![], vec![SlotSpec::new(0)])])]);
    // The list still reads (the names need no samples)
    assert_eq!(list_presets(&file).expect("presets").len(), 1);
    assert_eq!(import(&file, 0, 0).expect_err("compressed").key(), "sampler.gig.compressed");
}

#[test]
fn bad_files_and_missing_presets_are_errors() {
    assert_eq!(import(b"not a gig at all", 0, 0).expect_err("riff").key(), "sampler.gig.invalid");
    assert_eq!(import(&[], 0, 0).expect_err("empty").key(), "sampler.gig.invalid");
    let file = write_gig(&two_waves(), &[one("Sine", 0, vec![region(vec![], vec![SlotSpec::new(0)])])]);
    assert_eq!(import(&file, 3, 3).expect_err("preset").key(), "sampler.gig.no_preset");
}

#[test]
fn only_the_chosen_instruments_samples_are_decoded() {
    // The second instrument's wave is compressed, the first's is not: choosing the first works
    let waves = vec![(sine(440.0, 100), false), (sine(440.0, 100), true)];
    let file = write_gig(
        &waves,
        &[
            one("Good", 0, vec![region(vec![], vec![SlotSpec::new(0)])]),
            one("Packed", 1, vec![region(vec![], vec![SlotSpec::new(1)])]),
        ],
    );
    assert!(import(&file, 0, 0).is_ok());
    assert!(import(&file, 0, 1).is_err());
}

#[test]
fn zones_beyond_the_dimensions_bits_do_not_explode() {
    // Five one-bit dimensions that claim 255 zones each: 255^5 combinations if believed
    let dim = DimSpec { kind: DIM_LAYER, bits: 1, zones: 255 };
    let file = write_gig(&two_waves(), &[one("Many", 0, vec![region(vec![dim; 5], vec![SlotSpec::new(0)])])]);
    let i = import(&file, 0, 0).expect("import");
    assert!(i.zones.len() <= 256 * 127);
}

#[test]
fn the_last_velocity_zone_takes_the_top_velocities() {
    // Three zones of 42 on two bits: velocities 126 and 127 would be zone 3 without the clamp
    let dim = DimSpec { kind: 0x82, bits: 2, zones: 3 };
    let slots = vec![SlotSpec::new(0), SlotSpec::new(1), SlotSpec::new(0)];
    let file = write_gig(&two_waves(), &[one("Vel3", 0, vec![region(vec![dim], slots)])]);
    let i = import(&file, 0, 0).expect("import");
    assert!(i.zones.iter().any(|z| z.vels.1 == 127));
}
