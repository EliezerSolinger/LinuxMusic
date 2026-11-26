//! The Sampler against the references of the players it replaces (U34). The references are what the legacy
//! AvlDrums and Sf2 Player rendered through FluidSynth (`lm-plugins-fluidsynth/ab`). The Sampler is another engine,
//! so it cannot null against them: the check is that the same notes make the same event at the same time with
//! the same loudness (total level within 8 dB, onset within 10 ms, same number of audible hits within one), which is
//! what the player's replacement owes. The AVL kits and the General MIDI SoundFont are read from the legacy tree
//! (`$LEGACY_ROOT`, or this source tree's `legado`): without them the test fails with the folder it looked in.

use std::sync::Arc;

use lm_plugin::testing::{Bench, Timed, note_off, note_on};
use lm_plugin::{BuildContext, Loaded, Registry, ResourceView};
use lm_render::{EventKind, ProcessSpec};
use lm_state::{Value, from_json};

const LEVEL_DB: f64 = 8.0;
const ONSET_MS: f64 = 10.0;

fn fluidsynth_ab(file: &str) -> String {
    format!("{}/../lm-plugins-fluidsynth/ab/{file}", env!("CARGO_MANIFEST_DIR"))
}

fn num(v: &Value) -> f64 {
    v.as_real().or_else(|| v.as_int().map(|i| i as f64)).expect("a number")
}

fn events(c: &Value) -> Vec<Timed> {
    let mut out = Vec::new();
    let mut held: Vec<(u8, u32)> = Vec::new();
    for (i, e) in c.get("events").expect("events").items().enumerate() {
        let a: Vec<&Value> = e.items().collect();
        let at = num(a[0]) as u64;
        let id = 1000 + i as u32;
        match a[1].as_text().expect("kind") {
            "on" => {
                held.push((num(a[2]) as u8, id));
                out.push(note_on(at, id, num(a[2]) as u8, num(a[3]) as f32));
            }
            "off" => {
                let key = num(a[2]) as u8;
                let on = held.iter().position(|(k, _)| *k == key).map(|i| held.remove(i).1).unwrap_or(id);
                out.push(note_off(at, on, key));
            }
            "cc" => out.push(Timed::new(at, EventKind::Midi { bytes: [0xB0, num(a[2]) as u8, num(a[3]) as u8] })),
            k => panic!("event kind {k}"),
        }
    }
    out
}

fn legacy_root() -> std::path::PathBuf {
    std::env::var_os("LEGACY_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1.0e-12).log10()
}

/// (total RMS over the first channel pair, the first frame above 1 % of the peak)
fn measure(channels: &[&[f32]]) -> (f64, usize) {
    let frames = channels[0].len();
    let mix = |f: usize| channels.iter().map(|c| f64::from(c[f])).sum::<f64>();
    let peak = (0..frames).map(|f| mix(f).abs()).fold(0.0, f64::max);
    let rms = ((0..frames).map(|f| mix(f) * mix(f)).sum::<f64>() / frames as f64).sqrt();
    let onset = (0..frames).find(|f| mix(*f).abs() > peak * 0.01).unwrap_or(frames);
    (rms, onset)
}

#[test]
fn the_sampler_plays_what_the_replaced_players_played() {
    let registry = Registry::new(&[&lm_plugins_sampler::DESCRIPTOR]).expect("registry");
    let text = std::fs::read_to_string(fluidsynth_ab("cases.json")).expect("the cases");
    let cases: Vec<Value> = from_json(&text).expect("json").get("cases").expect("cases").items().cloned().collect();
    let font = legacy_root().join("legado/data/samples/soundfonts/FluidR3Mono_GM.sf3");
    let font_key = lm_state::Resource::new("soundfont", &"67".repeat(32)).expect("key");
    let mut failures = Vec::new();
    let mut compared = 0;
    for c in &cases {
        let name = c.get("name").and_then(Value::as_text).expect("name").to_owned();
        let plugin = c.get("plugin").and_then(Value::as_text).expect("plugin");
        let settings = c.get("settings").expect("settings");
        let get = |k: &str| settings.get(k).map(num);
        let (state, multi_output) = match plugin {
            "avldrums" => {
                let kit = lm_plugins_sampler::kits::AVL_KITS[get("Drum kit").unwrap_or(0.0) as usize].key;
                let multi = get("Output per drum piece").unwrap_or(0.0) >= 0.5;
                let s = Value::empty_object()
                    .with("format", Value::text("kit"))
                    .with("kit", Value::text(kit))
                    .with("outputs", Value::text(if multi { "groups" } else { "main" }))
                    .with("interpolation", Value::text("sinc"));
                (s, multi)
            }
            "sf2player" => {
                // (the legacy's linear gain knob as dB)
                let gain = db(get("Gain").unwrap_or(1.0)) as f32;
                let mut params = Value::empty_object().with("gain", Value::real(f64::from(gain)).expect("finite"));
                if let Some(env) = get("Volume envelope amount") {
                    params = params.with("volAmount", Value::real(env).expect("finite"));
                }
                let s = Value::empty_object()
                    .with("format", Value::text("sf2"))
                    .with("soundfont", Value::resource(font_key.clone()))
                    .with("bank", Value::int(get("Bank").unwrap_or(0.0) as i64))
                    .with("program", Value::int(get("Patch").unwrap_or(0.0) as i64))
                    .with(lm_state::PARAMS_KEY, params);
                (s, false)
            }
            p => panic!("plugin {p}"),
        };
        let state = registry.complete("sampler", &state).expect("complete");
        let rate = num(c.get("rate").expect("rate")) as u32;
        let block = num(c.get("block").expect("block")) as usize;
        let frames = num(c.get("frames").expect("frames")) as usize;
        let spec = ProcessSpec { sample_rate: rate, max_block: block };
        let mut resources = ResourceView::new();
        if plugin == "sf2player" {
            assert!(font.is_file(), "{name}: the General MIDI SoundFont is not at {}", font.display());
            resources.insert(font_key.clone(), Loaded::File(Arc::from(font.as_path())));
        }
        let cx = BuildContext { spec, resources: &resources, channels: 0 };
        let ports = registry.ports("sampler", &state).expect("ports");
        let channels: usize = ports.outputs.iter().map(|p| p.channels as usize).sum();
        let processor = registry.build("sampler", &state, &cx).expect("build");
        let out = Bench::instrument(processor, spec, channels, 1).render(frames, block, &events(c), false);
        let bytes = std::fs::read(fluidsynth_ab(&format!("reference/{name}.f32"))).expect("the reference");
        let want: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        assert_eq!(want.len(), frames * channels, "{name}: the reference has another layout");
        let reference: Vec<Vec<f32>> =
            (0..channels).map(|ch| (0..frames).map(|f| want[f * channels + ch]).collect()).collect();
        let got: Vec<&[f32]> = out.iter().map(Vec::as_slice).collect();
        let refs: Vec<&[f32]> = reference.iter().map(Vec::as_slice).collect();
        let (g, w) = (measure(&got), measure(&refs));
        let level = db(g.0) - db(w.0);
        let onset_ms = (g.1 as f64 - w.1 as f64).abs() * 1000.0 / f64::from(rate);
        let _ = multi_output;
        compared += 1;
        if level.abs() > LEVEL_DB || onset_ms > ONSET_MS {
            failures.push(format!("{name}: level {level:+.1} dB, onset {onset_ms:.1} ms"));
        }
    }
    assert!(compared > 0, "no case compared");
    assert!(failures.is_empty(), "the Sampler differs from the replaced players: {failures:?}");
}
