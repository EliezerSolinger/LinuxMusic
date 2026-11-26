//! A track's keys, base key, base velocity and microtuner (F-IJN-017/019/021), read from the tree into the
//! renderer's [`LiveKeys`]. The microtuner's math is the legacy's `Microtuner::keyToFreq`, `Keymap::getDegree` and
//! `Keymap::getOctave` ported as they are (the ratios as `f32`, the octave arithmetic in `f64`).

use lm_render::LiveKeys;
use lm_state::Value;

use crate::read::{flag, int, items};

/// The legacy's `DefaultKey`: the key that sounds 440 Hz when the track's base key is not changed
pub(crate) const DEFAULT_KEY: i64 = 69;
/// The velocity that plays at 100 % (the legacy's `MidiDefaultVelocity`)
pub(crate) const DEFAULT_VELOCITY: i64 = 63;
const DEFAULT_BASE_FREQUENCY: f64 = 440.0;

/// A scale's ratios: the unison first (the legacy keeps the implicit 1/1), then the intervals; the last one is the
/// period. An interval that is not one is left out
pub(crate) fn scale_ratios(scale: Option<&Value>) -> Vec<f32> {
    let mut r = vec![1.0f32];
    for t in scale.into_iter().flat_map(|s| items(s, "intervals")).filter_map(Value::as_text) {
        let Some(t) = t.split_whitespace().next() else { continue };
        if t.contains('.') {
            if let Ok(c) = t.parse::<f32>()
                && c.is_finite()
            {
                r.push((c / 1200.0).exp2());
            }
        } else {
            let (a, b) = t.split_once('/').unwrap_or((t, "1"));
            if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>())
                && a > 0
            {
                r.push(a as f32 / b.max(1) as f32);
            }
        }
    }
    r
}

/// A keymap of the project's tuning (`Keymap.cpp`)
pub(crate) struct Keymap {
    degrees: Vec<i64>,
    first: i32,
    last: i32,
    middle: i32,
    pub base_key: i32,
    base_freq: f64,
}

impl Keymap {
    pub(crate) fn read(node: Option<&Value>) -> Self {
        let Some(n) = node else { return Self::empty() };
        Self {
            degrees: items(n, "degrees").filter_map(Value::as_int).collect(),
            first: int(n, "first", 0).clamp(0, 127) as i32,
            last: int(n, "last", 127).clamp(0, 127) as i32,
            middle: int(n, "middle", 60).clamp(0, 127) as i32,
            base_key: int(n, "baseKey", 69).clamp(0, 127) as i32,
            base_freq: n.get("baseFrequency").and_then(Value::as_real).unwrap_or(DEFAULT_BASE_FREQUENCY),
        }
    }

    fn empty() -> Self {
        Self { degrees: Vec::new(), first: 0, last: 127, middle: 60, base_key: 69, base_freq: DEFAULT_BASE_FREQUENCY }
    }

    pub(crate) fn first(&self) -> u8 {
        self.first as u8
    }

    pub(crate) fn last(&self) -> u8 {
        self.last as u8
    }

    /// The scale degree of a key, −1 when the key is not mapped (`Keymap::getDegree`)
    fn degree(&self, key: i32) -> i64 {
        if key < self.first || key > self.last {
            return -1;
        }
        if self.degrees.is_empty() {
            return i64::from(key); // an empty table is the linear map
        }
        let size = self.degrees.len() as i32;
        let rem = (key - self.middle) % size;
        let m = if rem >= 0 { rem } else { rem + size };
        self.degrees[m as usize]
    }

    /// How many times the keymap repeated at this key (`Keymap::getOctave`)
    fn octave(&self, key: i32) -> i32 {
        if self.degrees.is_empty() || self.degree(key) == -1 {
            return 0;
        }
        let size = self.degrees.len() as i32;
        let offset = key - self.middle;
        if offset >= 0 { offset / size } else { (offset + 1) / size - 1 }
    }

    /// The frequency of a key, 0 for a key that is not mapped (`Microtuner::keyToFreq`). `import`: the base key is
    /// the keymap's, not the track's
    pub(crate) fn frequency(&self, ratios: &[f32], key: i32, user_base: i32, import: bool) -> f64 {
        if !(0..128).contains(&key) {
            return 0.0;
        }
        let degree = self.degree(key);
        if degree == -1 {
            return 0.0;
        }
        let octave = self.octave(key);
        let octave_degree = ratios.len() as i64 - 1;
        if octave_degree == 0 {
            return self.base_freq; // an octave of 1/1: a constant frequency
        }
        let scale_octave = (degree / octave_degree) as i32;
        let rem = degree % octave_degree;
        let scale_degree = if rem >= 0 { rem } else { rem + octave_degree };

        let base = if import { self.base_key } else { user_base };
        let base_degree = self.degree(base);
        if base_degree == -1 {
            return 0.0;
        }
        let base_octave = self.octave(base);
        let base_scale_octave = (base_degree / octave_degree) as i32;
        let brem = base_degree % octave_degree;
        let base_scale_degree = if brem >= 0 { brem } else { brem + octave_degree };

        let octave_ratio = f64::from(ratios[octave_degree as usize]);
        let middle = (self.base_freq / octave_ratio.powi(base_scale_octave + base_octave))
            / f64::from(ratios[base_scale_degree as usize]);
        middle * f64::from(ratios[scale_degree as usize]) * octave_ratio.powi(octave + scale_octave)
    }
}

fn slot<'a>(tuning: Option<&'a Value>, list: &str, n: i64) -> Option<&'a Value> {
    items(tuning?, list).nth(n.max(0) as usize)
}

/// The shaping of a track's notes: `transpose` is the project's (0 when the track does not follow it)
pub(crate) fn track_keys(track: &Value, tuning: Option<&Value>, transpose: i32) -> LiveKeys {
    let tuning = tuning.filter(|t| !t.is_null());
    let keys = track.get("keys").filter(|k| !k.is_null());
    let mut first = keys.map_or(0, |k| int(k, "first", 0)).clamp(0, 127);
    let mut last = keys.map_or(127, |k| int(k, "last", 127)).clamp(0, 127);
    let mut base = keys.map_or(DEFAULT_KEY, |k| int(k, "base", DEFAULT_KEY)).clamp(0, 127);
    let velocity = int(track, "baseVelocity", DEFAULT_VELOCITY).clamp(1, 127);
    let mut out = LiveKeys {
        transpose: transpose.clamp(-12, 12) as i8,
        velocity_scale: velocity as f32 / DEFAULT_VELOCITY as f32,
        ..LiveKeys::default()
    };
    let micro = track.get("microtuner").filter(|m| !m.is_null() && flag(m, "on", false));
    if let Some(m) = micro {
        let ratios = scale_ratios(slot(tuning, "scales", int(m, "scale", 0)));
        let keymap = Keymap::read(slot(tuning, "keymaps", int(m, "keymap", 0)));
        let import = flag(m, "importRanges", true);
        if import {
            first = i64::from(keymap.first());
            last = i64::from(keymap.last());
            base = i64::from(keymap.base_key);
        }
        let mut table = Box::new([f32::NAN; 128]);
        for (k, t) in table.iter_mut().enumerate() {
            let hz = keymap.frequency(&ratios, k as i32, base as i32, import);
            if hz > 0.0 {
                // Semitones from the key's own 12-TET pitch
                *t = (12.0 * (hz / DEFAULT_BASE_FREQUENCY).log2() - (k as f64 - DEFAULT_KEY as f64)) as f32;
            }
        }
        out.tuning = Some(table);
    } else {
        out.shift = (DEFAULT_KEY - base) as i8;
    }
    out.first = first as u8;
    out.last = last.max(first) as u8;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale(intervals: &[&str]) -> Value {
        Value::object([("intervals", Value::list(intervals.iter().map(|t| Value::text(*t))))])
    }

    #[test]
    fn twelve_tone_equal_matches_the_default_pitch() {
        let ints: Vec<String> = (1..=12).map(|i| format!("{}.0", i * 100)).collect();
        let refs: Vec<&str> = ints.iter().map(String::as_str).collect();
        let ratios = scale_ratios(Some(&scale(&refs)));
        let km = Keymap::empty();
        for key in [0, 21, 60, 69, 81, 127] {
            let hz = km.frequency(&ratios, key, 69, true);
            let want = 440.0 * ((key as f64 - 69.0) / 12.0).exp2();
            assert!((hz - want).abs() / want < 1e-4, "key {key}: {hz} vs {want}");
        }
    }

    #[test]
    fn an_empty_scale_is_a_constant_frequency() {
        let ratios = scale_ratios(None);
        assert_eq!(ratios, vec![1.0]);
        assert_eq!(Keymap::empty().frequency(&ratios, 40, 69, true), 440.0);
    }

    #[test]
    fn a_keymap_leaves_keys_silent_and_wraps_octaves() {
        // 5 notes to the octave, a map of 5 keys from middle 60: 0 1 x 2 3, then the octave repeats
        let ratios = scale_ratios(Some(&scale(&["200.0", "400.0", "700.0", "900.0", "2/1"])));
        let km =
            Keymap { degrees: vec![0, 1, -1, 2, 3], first: 0, last: 127, middle: 60, base_key: 60, base_freq: 261.625 };
        assert_eq!(km.frequency(&ratios, 62, 60, true), 0.0, "the silent key");
        assert!((km.frequency(&ratios, 60, 60, true) - 261.625).abs() < 1e-3);
        assert!((km.frequency(&ratios, 65, 60, true) - 2.0 * 261.625).abs() < 1e-2, "an octave up");
        assert!((km.frequency(&ratios, 55, 60, true) - 261.625 / 2.0).abs() < 1e-2, "an octave down");
        assert_eq!(km.octave(59), -1);
        assert_eq!(km.octave(55), -1);
        assert_eq!(km.octave(54), -2);
    }

    #[test]
    fn a_track_without_settings_plays_everything_unchanged() {
        let k = track_keys(&Value::object([] as [(&str, Value); 0]), None, 0);
        assert_eq!(k, LiveKeys::default());
        assert_eq!(k.map(60), Some((60, 0.0)));
        assert_eq!(k.velocity(100), 100);
    }

    #[test]
    fn keys_base_key_and_velocity() {
        let t = Value::object([
            ("keys", Value::object([("first", Value::int(48)), ("last", Value::int(72)), ("base", Value::int(60))])),
            ("baseVelocity", Value::int(126)),
        ]);
        let k = track_keys(&t, None, 2);
        assert_eq!(k.map(47 - 2), None);
        assert_eq!(k.map(60), Some((71, 0.0)), "60 + 2 transpose + 9 for the base key 60");
        assert_eq!(k.map(71), None, "the transposed 73 is past the last key");
        assert_eq!(k.velocity(40), 80);
        assert_eq!(k.velocity(100), 127);
    }

    #[test]
    fn a_microtuned_track_takes_the_keymaps_ranges() {
        let tuning = Value::object([
            ("scales", Value::list([scale(&["1200.0"])])),
            (
                "keymaps",
                Value::list([Value::object([
                    ("degrees", Value::list([Value::int(0)])),
                    ("first", Value::int(50)),
                    ("last", Value::int(70)),
                    ("middle", Value::int(60)),
                    ("baseKey", Value::int(60)),
                    ("baseFrequency", Value::real(220.0).unwrap()),
                ])]),
            ),
        ]);
        let t = Value::object([(
            "microtuner",
            Value::object([("on", Value::bool(true)), ("scale", Value::int(0)), ("keymap", Value::int(0))]),
        )]);
        let k = track_keys(&t, Some(&tuning), 0);
        assert_eq!((k.first, k.last), (50, 70));
        assert_eq!(k.map(40), None);
        // One note per octave, 220 Hz at key 60: 3 semitones under that key's own 12-TET pitch (9 + 12 below 440 Hz)
        let (key, off) = k.map(60).unwrap();
        assert_eq!(key, 60);
        assert!((off - (12.0 * (220.0f32 / 440.0).log2() + 9.0)).abs() < 1e-3);
    }
}
