//! The chip: a voice sounds at its pitch, the models differ, a copy plays on as its original

use lm_sys_resid::{ChipModel, PAL_CLOCK, Sid};

const RATE: f64 = 48_000.0;

/// Writes a register and clocks 9 cycles, as a 6502 would between two writes (on the MOS8580 a write lands on the
/// next cycle, so two writes with no cycle between keep only the second)
fn write(sid: &mut Sid, reg: u8, value: u8) {
    sid.write(reg, value);
    let mut cycles = 9;
    let mut scratch = [0i16; 16];
    let _ = sid.clock(&mut cycles, &mut scratch);
}

/// Voice 1 at `hz`, triangle (or `wave`), gate on, full volume, no filter, its envelope fast
fn play(sid: &mut Sid, hz: f64, wave: u8) {
    let reg = (hz / f64::from(PAL_CLOCK) * 16_777_216.0) as u32;
    write(sid, 0x00, (reg & 0xff) as u8);
    write(sid, 0x01, (reg >> 8) as u8);
    write(sid, 0x02, 0x00);
    write(sid, 0x03, 0x08);
    write(sid, 0x05, 0x00);
    write(sid, 0x06, 0xf0);
    write(sid, 0x18, 0x0f);
    write(sid, 0x04, wave | 1);
}

fn render(sid: &mut Sid, frames: usize) -> Vec<i16> {
    let mut out = vec![0i16; frames];
    let mut at = 0;
    while at < frames {
        let mut cycles = 1_000_000;
        at += sid.clock(&mut cycles, &mut out[at..]);
    }
    out
}

/// Crossings of the mean going up, per second
fn pitch(x: &[i16]) -> f64 {
    let mean = x.iter().map(|&v| f64::from(v)).sum::<f64>() / x.len() as f64;
    let ups: Vec<usize> = (1..x.len()).filter(|&i| f64::from(x[i - 1]) < mean && f64::from(x[i]) >= mean).collect();
    (ups.len() - 1) as f64 / ((ups[ups.len() - 1] - ups[0]) as f64 / RATE)
}

#[test]
fn a_voice_sounds_at_its_pitch() {
    let mut sid = Sid::new(f64::from(PAL_CLOCK), RATE).unwrap();
    play(&mut sid, 440.0, 0x10);
    let out = render(&mut sid, 48_000);
    let hz = pitch(&out[4_800..]);
    assert!((hz - 440.0).abs() < 0.5, "{hz} Hz");
    assert!(out.iter().map(|v| v.unsigned_abs()).max().unwrap() > 1000);
    assert!(Sid::version().starts_with('1'));
}

#[test]
fn the_models_differ() {
    let mut a = Sid::new(f64::from(PAL_CLOCK), RATE).unwrap();
    let mut b = Sid::new(f64::from(PAL_CLOCK), RATE).unwrap();
    b.set_chip_model(ChipModel::Mos6581);
    for sid in [&mut a, &mut b] {
        play(sid, 220.0, 0x20);
        write(sid, 0x17, 0xf1);
        write(sid, 0x18, 0x1f);
    }
    assert_ne!(render(&mut a, 4800), render(&mut b, 4800));
}

#[test]
fn a_copy_plays_on_as_its_original() {
    let mut a = Sid::new(f64::from(PAL_CLOCK), RATE).unwrap();
    // Sync and ring modulation use the other voices: the copy's must be its own
    play(&mut a, 330.0, 0x10 | 0x04 | 0x02);
    write(&mut a, 0x07, 0x40);
    write(&mut a, 0x08, 0x05);
    write(&mut a, 0x0c, 0x00);
    write(&mut a, 0x0d, 0xf0);
    write(&mut a, 0x0b, 0x41);
    write(&mut a, 0x17, 0x83);
    let _ = render(&mut a, 1000);
    let mut b = Sid::new(f64::from(PAL_CLOCK), 44_100.0).unwrap();
    b.copy_from(&a);
    let c = a.clone();
    let from_a = render(&mut a, 6000);
    let from_b = render(&mut b, 6000);
    let mut c = c;
    let from_c = render(&mut c, 6000);
    assert_eq!(from_a, from_b);
    assert_eq!(from_a, from_c);
    // The copy's voices are its own: playing a changed copy leaves the original as a twin made before the change
    let twin = a.clone();
    write(&mut b, 0x0e, 0x99);
    write(&mut b, 0x0f, 0x10);
    let _ = render(&mut b, 3000);
    let mut twin = twin;
    assert_eq!(render(&mut a, 3000), render(&mut twin, 3000));
}
