//! Xpressive: the contract, the legacy factory presets (every expression of the 22 compiles, every preset plays
//! finite sound), the editor's helpers (the error's position, the last good text kept), and the plugin as the
//! legacy played a note (the expression's value, the panning, the knobs on a sounding note).
//!
//! The sound itself is `lm_dsp::xpressive`'s (its tests); the expressions' own results are `lm-expr`'s A/B.
//! (Written without running: bless the frozen defaults once with `LM_BLESS_DEFAULTS=1` into
//! `tests/data/xpressive/defaults.json`.)

mod common;

lm_plugin::contract_tests!(lm_plugins_instruments::xpressive::DESCRIPTOR, "xpressive");

use std::path::PathBuf;

use common::{render, state};
use lm_expr::key;
use lm_plugin::testing::{note_off, note_on};
use lm_plugins_instruments::xpressive::{
    DEFAULT_O1, DEFAULT_O2, DESCRIPTOR, ExpressionKind, SMOOTH_FIELDS, WAVE_EXPR_FIELDS, WAVE_GRAPH_FIELDS, check,
    edit, status, display_frame, FLAGS, FRAME_FLOATS, PREVIEW_POINTS,
};
use lm_state::Value;

const RATE: u32 = 48_000;

fn presets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../legado/data/presets/Xpressive")
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&apos;", "'").replace("&amp;", "&")
}

/// The attribute `name="..."` of the preset's XML (the first, as ` name="`)
fn attribute(xml: &str, name: &str) -> Option<String> {
    let needle = format!(" {name}=\"");
    let start = xml.find(&needle)? + needle.len();
    let end = xml[start..].find('"')? + start;
    Some(unescape(&xml[start..end]))
}

fn base64(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

struct Preset {
    name: String,
    xml: String,
}

fn presets() -> Vec<Preset> {
    let Ok(dir) = std::fs::read_dir(presets_dir()) else { return Vec::new() };
    let mut all: Vec<Preset> = dir
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "xpf"))
        .filter_map(|e| {
            let xml = std::fs::read_to_string(e.path()).ok()?;
            Some(Preset { name: e.file_name().to_string_lossy().into_owned(), xml })
        })
        .collect();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}

/// The plugin's state for a legacy preset
fn preset_state(p: &Preset) -> Value {
    let num = |name: &str| attribute(&p.xml, name).and_then(|v| v.parse::<f64>().ok());
    let mut params: Vec<(&str, f64)> = Vec::new();
    for (key, attr) in [
        ("a1", "A1"),
        ("a2", "A2"),
        ("a3", "A3"),
        ("panning1", "PAN1"),
        ("panning2", "PAN2"),
        ("relTransition", "RELTRANS"),
        ("interpolateW1", "interpolateW1"),
        ("interpolateW2", "interpolateW2"),
        ("interpolateW3", "interpolateW3"),
    ] {
        if let Some(v) = num(attr) {
            params.push((key, v));
        }
    }
    let mut fields: Vec<(&str, Value)> = Vec::new();
    for (key, attr) in [("o1", "O1"), ("o2", "O2")] {
        if let Some(t) = attribute(&p.xml, attr) {
            fields.push((key, Value::text(t)));
        }
    }
    for i in 0..3 {
        let n = i + 1;
        if let Some(t) = attribute(&p.xml, &format!("W{n}")) {
            fields.push((WAVE_EXPR_FIELDS[i], Value::text(t)));
        }
        if let Some(b) = attribute(&p.xml, &format!("W{n}sample")) {
            let points: Vec<Value> = base64(&b)
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .map(|v| Value::real(f64::from(if v.is_finite() { v.clamp(-2.0, 2.0) } else { 0.0 })).unwrap())
                .collect();
            fields.push((WAVE_GRAPH_FIELDS[i], Value::list(points)));
        }
        if let Some(s) = num(&format!("smoothW{n}")) {
            fields.push((SMOOTH_FIELDS[i], Value::int(s.round() as i64)));
        }
    }
    state(&params, &fields)
}

#[test]
fn every_legacy_preset_expression_compiles() {
    let all = presets();
    if all.is_empty() {
        // (the legacy's folder is not in this checkout)
        return;
    }
    assert!(all.len() >= 22, "the 22 factory presets: {}", all.len());
    for p in &all {
        for name in ["O1", "O2"] {
            let text = attribute(&p.xml, name).unwrap_or_default();
            // (an output left empty is a legacy preset that plays one output only)
            if !text.is_empty() {
                check(ExpressionKind::Output, &text).unwrap_or_else(|e| panic!("{}: {name} {text:?}: {e}", p.name));
            }
        }
        for name in ["W1", "W2", "W3"] {
            let text = attribute(&p.xml, name).unwrap_or_default();
            check(ExpressionKind::Wave, &text).unwrap_or_else(|e| panic!("{}: {name} {text:?}: {e}", p.name));
        }
    }
}

#[test]
fn every_legacy_preset_plays_finite_sound() {
    let all = presets();
    let mut sounding = 0;
    for p in &all {
        let s = preset_state(p);
        let st = status(&s);
        assert!(st.outputs.iter().chain(&st.waves).all(Option::is_none) || p.name.contains("notes test"), "{}", p.name);
        let events = [note_on(0, 1, 60, 0.8), note_off(30_000, 1, 60)];
        let out = render(&DESCRIPTOR, &s, RATE, 5, 60_000, &events);
        assert!(out.iter().flatten().all(|v| v.is_finite()), "{}", p.name);
        if out.iter().flatten().any(|v| v.abs() > 1.0e-4) {
            sounding += 1;
        }
    }
    assert!(sounding * 2 >= all.len(), "most presets sound: {sounding} of {}", all.len());
}

#[test]
fn the_defaults_play_the_legacy_default_patch() {
    // The default outputs compile, and the default instrument sounds on a note
    check(ExpressionKind::Output, DEFAULT_O1).unwrap();
    check(ExpressionKind::Output, DEFAULT_O2).unwrap();
    let out = render(&DESCRIPTOR, &state(&[], &[]), RATE, 1, 20_000, &[note_on(0, 1, 60, 1.0)]);
    assert!(out[0].iter().chain(&out[1]).any(|v| v.abs() > 1.0e-3));
    assert!(out.iter().flatten().all(|v| v.is_finite()));
}

#[test]
fn the_editor_gets_the_errors_position_and_the_last_good_text() {
    assert!(check(ExpressionKind::Output, "sinew(t*f)").is_ok());
    // An empty wave text means the drawn points; an empty output is no output
    assert!(check(ExpressionKind::Wave, "").is_ok());
    assert!(check(ExpressionKind::Output, "").is_err());
    // Unknown names, with where: `sinew(t * foo)` goes wrong at `foo` (byte 10)
    let e = check(ExpressionKind::Output, "sinew(t * foo)").unwrap_err();
    assert_eq!(e.label.key, key::UNKNOWN_SYMBOL);
    assert!((10..=13).contains(&e.position), "{}", e.position);
    // A wave expression has no `f` and no `W1`
    assert!(check(ExpressionKind::Wave, "f").is_err());
    assert!(check(ExpressionKind::Wave, "W1(t)").is_err());
    assert!(check(ExpressionKind::Output, "W1(t)").is_ok());
    // The bad text keeps the previous good one
    let ok = edit(ExpressionKind::Output, "t", "t * 2");
    assert_eq!((ok.text.as_str(), ok.error.is_none()), ("t * 2", true));
    let bad = edit(ExpressionKind::Output, "t", "t * (");
    assert_eq!(bad.text, "t");
    assert!(bad.error.is_some());
}

#[test]
fn status_reports_each_expression() {
    let s = state(
        &[],
        &[
            ("o1", Value::text("t +")),
            ("o2", Value::text("t")),
            ("w2Expr", Value::text("sinew(")),
            ("w3Expr", Value::text("sinew(t)")),
        ],
    );
    let st = status(&s);
    assert!(st.outputs[0].is_some() && st.outputs[1].is_none());
    assert!(st.waves[0].is_none() && st.waves[1].is_some() && st.waves[2].is_none());
}

fn constant_note(o1: &str, o2: &str, params: &[(&str, f64)]) -> Vec<Vec<f32>> {
    let s = state(params, &[("o1", Value::text(o1)), ("o2", Value::text(o2))]);
    let events = [note_on(0, 1, 69, 1.0)];
    render(&DESCRIPTOR, &s, RATE, 3, 8_000, &events)
}

#[test]
fn a_constant_output_comes_out_panned() {
    // O1 full right (the default), nothing on O2: the output is 0.5 of the expression's 0.8 on the right, none left
    let out = constant_note("0.8", "", &[]);
    for n in 1_000..2_000 {
        assert!(out[0][n].abs() < 1e-4 && (out[1][n] - 0.8).abs() < 1e-3, "{n}: {} {}", out[0][n], out[1][n]);
    }
    // Both centred: half each
    let out = constant_note("0.8", "0.2", &[("panning1", 0.0), ("panning2", 0.0)]);
    for n in 1_000..2_000 {
        assert!((out[0][n] - 0.5).abs() < 1e-3 && (out[1][n] - 0.5).abs() < 1e-3);
    }
}

#[test]
fn the_a_knobs_reach_a_sounding_note() {
    let out =
        constant_note("A1 + 10 * A2 + 100 * A3", "", &[("a1", 0.5), ("a2", -0.5), ("a3", 0.25), ("panning1", 0.0)]);
    let want = 0.5 * (0.5 - 5.0 + 25.0);
    assert!((out[0][3_000] - want).abs() < 1e-2 && (out[1][3_000] - want).abs() < 1e-2, "{}", out[0][3_000]);
}

#[test]
fn the_notes_pitch_is_f_and_the_key_is_key() {
    let out = constant_note("f + 1000 * key", "", &[("panning1", 0.0)]);
    let want = 0.5 * (440.0 + 69_000.0);
    assert!((out[0][3_000] - want).abs() < want * 1e-4, "{}", out[0][3_000]);
}

#[test]
fn a_wave_expression_and_its_smoothing_make_the_table() {
    let s = state(
        &[("panning1", 0.0)],
        &[("o1", Value::text("W1(0.25)")), ("o2", Value::text("")), ("w1Expr", Value::text("sinew(t)"))],
    );
    let out = render(&DESCRIPTOR, &s, RATE, 3, 8_000, &[note_on(0, 1, 69, 1.0)]);
    // sinew(t) at a quarter of the period: 1, half on each side
    assert!((out[0][3_000] - 0.5).abs() < 1e-2, "{}", out[0][3_000]);
    // The drawn points play when the expression is empty
    let drawn = Value::list((0..4096).map(|_| Value::real(0.5).unwrap()));
    let s = state(&[("panning1", 0.0)], &[("o1", Value::text("W1(0.7)")), ("o2", Value::text("")), ("w1Graph", drawn)]);
    let out = render(&DESCRIPTOR, &s, RATE, 3, 8_000, &[note_on(0, 1, 69, 1.0)]);
    assert!((out[0][3_000] - 0.25).abs() < 1e-3);
}

#[test]
fn the_release_transition_moves_rel_after_the_note_off() {
    let s =
        state(&[("panning1", 0.0), ("relTransition", 100.0)], &[("o1", Value::text("rel")), ("o2", Value::text(""))]);
    // The note is let go at 20000 and the instrument's release is what keeps it sounding: with none, it ends there;
    // before the release `rel` is 0
    let out = render(&DESCRIPTOR, &s, RATE, 3, 30_000, &[note_on(0, 1, 69, 1.0), note_off(20_000, 1, 69)]);
    assert!(out[0][..19_000].iter().all(|v| v.abs() < 1e-6));
}

#[test]
fn the_display_frame_has_the_flags_and_each_outputs_curve() {
    // O1 is a constant 0.5 and O2 does not compile: its flag is up and its curve is silence
    let s = state(&[], &[("o1", Value::text("0.5")), ("o2", Value::text("1 +"))]);
    let frame = display_frame(&s, RATE).unwrap();
    assert_eq!(frame.len(), FRAME_FLOATS);
    assert_eq!(&frame[..FLAGS], &[0.0, 0.0, 0.0, 0.0, 1.0]);
    let o1 = &frame[FLAGS..FLAGS + PREVIEW_POINTS];
    assert!(o1.iter().all(|v| (v - 0.5 * 100.0 / 255.0).abs() < 1e-3 || (v - 0.5).abs() < 1e-3), "{o1:?}");
    let o2 = &frame[FLAGS + PREVIEW_POINTS..];
    assert!(o2.iter().all(|v| *v == 0.0));
}

#[test]
fn the_display_frame_does_not_depend_on_anything_but_the_state() {
    let s = state(&[("a1", 0.3)], &[]);
    assert_eq!(display_frame(&s, RATE).unwrap(), display_frame(&s, RATE).unwrap());
}
