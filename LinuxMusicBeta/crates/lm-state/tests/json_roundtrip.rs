//! Property: random trees go through the JSON and come back bit for bit, and the text is a fixed point.
//! Seeded (`LM_TEST_SEED=<n>` replays another seed); see `common/rng.rs`.

mod common;

use common::rng::Rng;
use lm_state::{Value, from_json, to_json, to_json_compact};

#[test]
fn round_trip_is_bit_exact() {
    let seed = Rng::seed(0x5eed_0001);
    let mut rng = Rng::new(seed);
    for case in 0..20_000 {
        let v = rng.tree(6);
        let json = to_json(&v).unwrap_or_else(|e| panic!("seed {seed} case {case}: {e} for {v:?}"));
        let back = from_json(&json).unwrap_or_else(|e| panic!("seed {seed} case {case}: {e} reading {json}"));
        // Exact equality: deep, and bit for bit for reals
        assert_eq!(back, v, "seed {seed} case {case}");
        // The text is a fixed point: writing what was read gives the same bytes
        assert_eq!(to_json(&back).unwrap(), json, "seed {seed} case {case}");
        let compact = to_json_compact(&v).unwrap();
        assert_eq!(from_json(&compact).unwrap(), v, "seed {seed} case {case}");
    }
}

#[test]
fn reals_keep_their_bits() {
    let seed = Rng::seed(0x5eed_0002);
    let mut rng = Rng::new(seed);
    for case in 0..200_000 {
        let x = rng.real();
        let json = to_json_compact(&Value::real(x).unwrap()).unwrap();
        assert!(json.contains('.') || json.contains('e'), "{json}");
        let back = from_json(&json).unwrap().as_real().unwrap();
        assert_eq!(back.to_bits(), x.to_bits(), "seed {seed} case {case}: {json}");
    }
}

#[test]
fn negative_zero_survives_in_a_tree() {
    let v = Value::object([("a", Value::real(-0.0).unwrap()), ("b", Value::list([Value::real(-0.0).unwrap()]))]);
    let back = from_json(&to_json(&v).unwrap()).unwrap();
    assert!(back.get("a").unwrap().as_real().unwrap().is_sign_negative());
    assert_eq!(back, v);
}
