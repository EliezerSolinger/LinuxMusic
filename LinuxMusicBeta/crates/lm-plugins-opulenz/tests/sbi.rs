//! `.sbi` files read as the legacy `loadFile` read them

use lm_plugins_opulenz::sbi;
use lm_state::{PARAMS_KEY, Value};

fn file(regs: [u8; 11]) -> Vec<u8> {
    let mut b = b"SBI\x1a".to_vec();
    let mut name = b"Brass 1".to_vec();
    name.resize(32, 0);
    b.extend(name);
    b.extend(regs);
    b.resize(52, 0);
    b
}

#[test]
fn a_patch_sets_every_knob_as_the_legacy_did() {
    // modulator: AM, EG (sustained), mult 1; carrier: VIB, KSR, mult 2; KSL 2 and TL 0x15 / KSL 0, TL 0;
    // AD f2 / 54; SR 13 / 26; waves 1 / 3; feedback 5, additive
    let p = sbi::read(&file([0xA1, 0x52, 0x95, 0x00, 0xF2, 0x54, 0x13, 0x26, 0x01, 0x03, 0x0B])).unwrap();
    assert_eq!(p.name, "Brass 1");
    let get = |k: &str| p.params.iter().find(|(key, _)| *key == k).unwrap().1;
    assert_eq!(get("op1Tremolo"), 1.0);
    assert_eq!(get("op1Vibrato"), 0.0);
    assert_eq!(get("op1Percussive"), 0.0);
    assert_eq!(get("op1KeyScalingRate"), 0.0);
    assert_eq!(get("op1Multiplier"), 1.0);
    assert_eq!(get("op2Vibrato"), 1.0);
    assert_eq!(get("op2Percussive"), 1.0);
    assert_eq!(get("op2KeyScalingRate"), 1.0);
    assert_eq!(get("op2Multiplier"), 2.0);
    assert_eq!(get("op1LevelScaling"), 2.0);
    assert_eq!(get("op1Level"), 63.0 - 0x15 as f64);
    assert_eq!(get("op2Level"), 63.0);
    assert_eq!(get("op1Attack"), 0.0);
    assert_eq!(get("op1Decay"), 13.0);
    assert_eq!(get("op2Attack"), 10.0);
    assert_eq!(get("op1Sustain"), 14.0);
    assert_eq!(get("op1Release"), 12.0);
    assert_eq!(get("fm"), 0.0);
    assert_eq!(get("feedback"), 5.0);
    assert_eq!(p.fields, vec![("op1Waveform", "half-sine"), ("op2Waveform", "quarter-sine")]);
    assert_eq!(p.params.len(), 24);
    let state = p.apply(&Value::empty_object().with("other", Value::bool(true))).unwrap();
    assert_eq!(state.get("other"), Some(&Value::bool(true)));
    assert_eq!(state.get(PARAMS_KEY).and_then(|v| v.get("feedback")).and_then(Value::as_real), Some(5.0));
}

#[test]
fn not_an_sbi_is_refused() {
    assert_eq!(sbi::read(b"RIFF....").unwrap_err().key(), "opulenz.sbi.signature");
    assert_eq!(sbi::read(b"SBI\x1a short").unwrap_err().key(), "opulenz.sbi.short");
}
