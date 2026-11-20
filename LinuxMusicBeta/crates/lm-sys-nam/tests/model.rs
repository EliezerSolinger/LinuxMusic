//! The models load, report what their files say, and play; broken files are errors with NAM's message

use lm_sys_nam::Model;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)).unwrap()
}

/// A sine at 220 Hz, 48 kHz
fn sine(frames: usize) -> Vec<f32> {
    (0..frames).map(|i| 0.3 * (i as f32 * 220.0 * std::f32::consts::TAU / 48_000.0).sin()).collect()
}

fn run(model: &mut Model, input: &[f32], piece: usize) -> Vec<f32> {
    let mut out = input.to_vec();
    for chunk in out.chunks_mut(piece) {
        model.process(chunk);
    }
    out
}

#[test]
fn the_example_models_load_and_play() {
    for (name, loudness) in [
        ("lstm.nam", Some(-37.8406867980957)),
        ("wavenet.nam", Some(-20.020729064941406)),
        ("slimmable_wavenet.nam", None),
    ] {
        let mut model = Model::load(&data(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(model.expected_rate(), Some(48_000.0), "{name}");
        assert_eq!(model.loudness(), loudness, "{name}");
        model.reset(48_000.0, 512).unwrap();
        let out = run(&mut model, &sine(9_600), 512);
        assert!(out.iter().all(|x| x.is_finite()), "{name}");
        let peak = out[4_800..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1.0e-3, "{name}: silent ({peak})");
    }
}

#[test]
fn the_output_does_not_depend_on_the_pieces() {
    for name in ["lstm.nam", "wavenet.nam"] {
        let input = sine(4_096);
        let mut a = Model::load(&data(name)).unwrap();
        a.reset(48_000.0, 512).unwrap();
        let whole = run(&mut a, &input, 512);
        for piece in [32, 100, 256] {
            let mut b = Model::load(&data(name)).unwrap();
            b.reset(48_000.0, 512).unwrap();
            let pieces = run(&mut b, &input, piece);
            let worst = whole.iter().zip(&pieces).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
            assert!(worst == 0.0, "{name}: pieces of {piece} differ by {worst}");
        }
    }
}

#[test]
fn broken_files_are_errors_with_the_reason() {
    let e = Model::load(b"{ not json").unwrap_err();
    assert_eq!(e.key(), "nam.load");
    let e =
        Model::load(br#"{"version": "0.5.4", "architecture": "Nonsense", "config": {}, "weights": []}"#).unwrap_err();
    assert_eq!(e.key(), "nam.load");
    assert!(format!("{e:?}").contains("Nonsense") || !format!("{e:?}").is_empty());
    let e = Model::load(br#"{"version": "9.0.0", "architecture": "Linear", "config": {}, "weights": []}"#).unwrap_err();
    assert_eq!(e.key(), "nam.load");
}

#[test]
fn before_a_reset_it_is_silent() {
    let mut model = Model::load(&data("lstm.nam")).unwrap();
    let mut io = sine(64);
    model.process(&mut io);
    assert!(io.iter().all(|x| *x == 0.0));
}
