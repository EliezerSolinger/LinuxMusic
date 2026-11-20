//! SoundBlaster instrument files (`.sbi`), as the legacy OpulenZ loaded them into its knobs (`loadFile`): the
//! signature `SBI\x1a`, the name in bytes 4–35, and the 11 register bytes from 36 (modulator then carrier: sound
//! characteristic, scaling and output level, attack and decay, sustain and release, wave select; then feedback and
//! connection). Files of 47 to 52 bytes are read (the reserved bytes at the end may be missing).

use lm_base::{Error, Result};
use lm_state::Value;

use crate::WAVEFORM_KEYS;

/// What an `.sbi` file sets: the instrument's name, its parameters (keys of the plugin's schema) and its fields
#[derive(Clone, Debug, PartialEq)]
pub struct SbiPatch {
    pub name: String,
    pub params: Vec<(&'static str, f64)>,
    pub fields: Vec<(&'static str, &'static str)>,
}

/// Reads an `.sbi` file. Keys: `opulenz.sbi.signature` (not an SBI file), `opulenz.sbi.short` (under 47 bytes)
pub fn read(bytes: &[u8]) -> Result<SbiPatch> {
    if !bytes.starts_with(b"SBI\x1a") {
        return Err(Error::new("opulenz.sbi.signature"));
    }
    if bytes.len() < 47 {
        return Err(Error::new("opulenz.sbi.short").arg("bytes", bytes.len() as i64));
    }
    let name_bytes = &bytes[4..36];
    let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
    let name = String::from_utf8_lossy(&name_bytes[..end]).trim().to_owned();
    let b = |i: usize| i32::from(bytes[i]);
    let flag = |v: i32, bit: i32| if v & bit == bit { 1.0 } else { 0.0 };
    let mut params = Vec::new();
    for (op, prefix) in ["op1", "op2"].iter().enumerate() {
        let character = b(36 + op);
        let level = b(38 + op);
        let ad = b(40 + op);
        let sr = b(42 + op);
        let key = |k: &str| -> &'static str {
            crate::KEYS.iter().find(|key| **key == format!("{prefix}{k}")).copied().unwrap_or("")
        };
        params.push((key("Tremolo"), flag(character, 0x80)));
        params.push((key("Vibrato"), flag(character, 0x40)));
        // (the sustained envelope bit set is "not percussive")
        params.push((key("Percussive"), 1.0 - flag(character, 0x20)));
        params.push((key("KeyScalingRate"), flag(character, 0x10)));
        params.push((key("Multiplier"), f64::from(character & 0x0f)));
        params.push((key("LevelScaling"), f64::from((level & 0xc0) >> 6)));
        params.push((key("Level"), f64::from(63 - (level & 0x3f))));
        params.push((key("Attack"), f64::from(15 - ((ad & 0xf0) >> 4))));
        params.push((key("Decay"), f64::from(15 - (ad & 0x0f))));
        params.push((key("Sustain"), f64::from(15 - ((sr & 0xf0) >> 4))));
        params.push((key("Release"), f64::from(15 - (sr & 0x0f))));
    }
    let connection = b(46);
    params.push(("fm", if connection & 0x01 == 0x01 { 0.0 } else { 1.0 }));
    params.push(("feedback", f64::from((connection & 0x0e) >> 1)));
    let fields = vec![
        ("op1Waveform", WAVEFORM_KEYS[(b(44) & 0x03) as usize]),
        ("op2Waveform", WAVEFORM_KEYS[(b(45) & 0x03) as usize]),
    ];
    Ok(SbiPatch { name, params, fields })
}

impl SbiPatch {
    /// The plugin's state with the patch applied (`state`: the instrument's current state, its other fields kept)
    pub fn apply(&self, state: &Value) -> Result<Value> {
        let mut params = state.get(lm_state::PARAMS_KEY).cloned().unwrap_or_else(Value::empty_object);
        for (key, value) in &self.params {
            params = params.with(key, Value::real(*value)?);
        }
        let mut out = state.with(lm_state::PARAMS_KEY, params);
        for (key, value) in &self.fields {
            out = out.with(key, Value::text(*value));
        }
        Ok(out)
    }
}
