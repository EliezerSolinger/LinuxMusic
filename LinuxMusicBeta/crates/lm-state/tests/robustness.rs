//! Robustness: garbage, mutated JSON and hostile shapes never panic the reader nor the validation (the long run is
//! the cargo-fuzz target in `fuzz/`, on the CI)

mod common;

use common::rng::Rng;
use lm_state::{MAX_DEPTH, Value, from_json, from_json_bytes, to_json};

const SEED_DOCUMENTS: &[&str] = &[
    r#"{"id": "t:17", "name": "Baixo", "parent": null, "volume": 0.8, "mute": false, "solo": false,
        "outputs": [{"id": "o:20", "port": "main", "channels": 2, "to": "m:3"}],
        "items": [{"id": "c:52", "kind": "midi", "start": 0, "length": 3840,
                   "notes": {"812": {"key": 57, "start": 0, "length": 480, "velocity": 0.79}}}]}"#,
    r#"{"wave": "saw", "params": {"vol1": 0.5, "cutoff": 1000.0, "octave": -1.0}, "chunk": {"a": [1, 2.5e-3]}}"#,
    r#"{"id": "l:61", "sourceOffset": 0,
        "source": {"kind": "audio", "key": "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"}}"#,
    r#"["\u00e9\ud83d\ude00\n", -0.0, 1e308, -9007199254740991, true, null, {}, []]"#,
];

const TYPES: &[&str] = &["track", "plugin:tripleoscillator", "lane", "item.midi", "note", "channel", "output"];

fn exercise(registry: &lm_state::SchemaRegistry, bytes: &[u8]) {
    if let Ok(v) = from_json_bytes(bytes) {
        for t in TYPES {
            let _ = registry.validate(t, &v);
            let _ = registry.refs(t, &v);
            let _ = registry.ids(t, &v);
            let _ = registry.resources(t, &v);
            let _ = registry.load(t, 1, v.clone());
        }
        // What was read can always be written, and reads back the same
        let json = to_json(&v).unwrap();
        assert_eq!(from_json(&json).unwrap(), v);
    }
}

/// One random change to a document, biased towards the bytes that matter to JSON
fn mutate(rng: &mut Rng, doc: &mut Vec<u8>) {
    const INTERESTING: &[u8] = b"{}[]\",:\\-+.eE0123456789tfnu \n\xff\xc3\x80";
    let len = doc.len().max(1);
    let at = rng.below(len).min(doc.len());
    match rng.below(5) {
        0 => {
            if let Some(x) = doc.get_mut(at) {
                *x = if rng.chance(2) { INTERESTING[rng.below(INTERESTING.len())] } else { rng.byte() };
            }
        }
        1 => {
            let end = (at + 1 + rng.below(8)).min(doc.len());
            doc.drain(at..end);
        }
        2 => {
            let n = 1 + rng.below(6);
            let bytes: Vec<u8> = (0..n)
                .map(|_| if rng.chance(2) { INTERESTING[rng.below(INTERESTING.len())] } else { rng.byte() })
                .collect();
            doc.splice(at..at, bytes);
        }
        3 => doc.truncate(at),
        _ => {
            let end = (at + 1 + rng.below(32)).min(doc.len());
            let piece = doc[at..end].to_vec();
            doc.splice(end..end, piece);
        }
    }
}

#[test]
fn garbage_never_panics() {
    let registry = common::registry();
    let seed = Rng::seed(0x5eed_0003);
    let mut rng = Rng::new(seed);
    for _ in 0..20_000 {
        let bytes = rng.bytes(512);
        exercise(&registry, &bytes);
    }
}

#[test]
fn mutated_documents_never_panic() {
    let registry = common::registry();
    let seed = Rng::seed(0x5eed_0004);
    let mut rng = Rng::new(seed);
    let mut read = 0usize;
    for _ in 0..50_000 {
        let mut doc = SEED_DOCUMENTS[rng.below(SEED_DOCUMENTS.len())].as_bytes().to_vec();
        for _ in 0..1 + rng.below(5) {
            mutate(&mut rng, &mut doc);
        }
        if from_json_bytes(&doc).is_ok() {
            read += 1;
        }
        exercise(&registry, &doc);
    }
    // The mutations must also reach the validation, not only the reader's errors
    assert!(read > 1_000, "only {read} mutated documents were still JSON");
}

#[test]
fn random_trees_never_panic_the_validation() {
    let registry = common::registry();
    let seed = Rng::seed(0x5eed_0005);
    let mut rng = Rng::new(seed);
    let base = common::track();
    for _ in 0..20_000 {
        // A valid track with one random subtree grafted somewhere
        let mut paths = Vec::new();
        base.walk(&mut |p, _| paths.push(p.to_vec()));
        let path = &paths[rng.below(paths.len())];
        let v = if path.is_empty() { rng.tree(4) } else { base.with_path(path, rng.tree(4)).unwrap() };
        exercise(&registry, to_json(&v).unwrap().as_bytes());
    }
}

#[test]
fn seed_documents_are_valid() {
    let r = common::registry();
    r.validate("track", &from_json(SEED_DOCUMENTS[0]).unwrap()).unwrap();
    r.validate("plugin:tripleoscillator", &from_json(SEED_DOCUMENTS[1]).unwrap()).unwrap();
    r.validate("lane", &from_json(SEED_DOCUMENTS[2]).unwrap()).unwrap();
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    for open in ["[", "{\"a\":"] {
        let doc = open.repeat(1_000_000);
        assert_eq!(from_json(&doc).unwrap_err().key(), "state.json.too_deep");
    }
    let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    from_json(&ok).unwrap();
    let too_deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
    assert_eq!(from_json(&too_deep).unwrap_err().key(), "state.json.too_deep");
}

#[test]
fn syntax_errors_have_keys_and_positions() {
    let cases: &[(&str, &str)] = &[
        ("", "state.json.unexpected_end"),
        ("   ", "state.json.unexpected_end"),
        ("{", "state.json.unexpected_end"),
        ("[1,", "state.json.unexpected_end"),
        ("\"abc", "state.json.unexpected_end"),
        ("[1,]", "state.json.syntax"),
        ("{,}", "state.json.syntax"),
        ("{\"a\" 1}", "state.json.syntax"),
        ("{\"a\":1,}", "state.json.syntax"),
        ("{1:2}", "state.json.syntax"),
        ("01", "state.json.trailing"),
        ("1.", "state.json.syntax"),
        (".5", "state.json.syntax"),
        ("1e", "state.json.syntax"),
        ("+1", "state.json.syntax"),
        ("-", "state.json.unexpected_end"),
        ("NaN", "state.json.syntax"),
        ("Infinity", "state.json.syntax"),
        ("-Infinity", "state.json.syntax"),
        ("1e400", "state.json.real_out_of_range"),
        ("-1e400", "state.json.real_out_of_range"),
        ("nul", "state.json.syntax"),
        ("truex", "state.json.trailing"),
        ("[1] [2]", "state.json.trailing"),
        ("\"a\nb\"", "state.json.control_character"),
        ("\"\\x\"", "state.json.bad_escape"),
        ("\"\\u12g4\"", "state.json.bad_escape"),
        ("\"\\ud800\"", "state.json.lone_surrogate"),
        ("\"\\udc00\"", "state.json.lone_surrogate"),
        ("\"\\ud800\\u0041\"", "state.json.lone_surrogate"),
        ("{\"a\":1,\"a\":2}", "state.json.duplicate_key"),
        ("\u{feff}{}", "state.json.syntax"),
        ("// comment\n{}", "state.json.syntax"),
    ];
    for (doc, key) in cases {
        let e = from_json(doc).unwrap_err();
        assert_eq!(e.key(), *key, "{doc:?}: {e}");
        assert!(e.to_string().contains("line="), "{e}");
    }
    let e = from_json("{\n  \"a\": 1,\n  \"a\": 2\n}").unwrap_err();
    assert!(e.to_string().contains("line=3 column=3"), "{e}");
    assert_eq!(from_json_bytes(b"[\"\xff\"]").unwrap_err().key(), "state.json.not_utf8");
    // Underflow is not an error: the nearest real is 0
    assert_eq!(from_json("1e-400").unwrap(), Value::real(0.0).unwrap());
    // Long digit strings are read exactly
    let long = format!("0.{}1", "0".repeat(2000));
    from_json(&long).unwrap();
}
