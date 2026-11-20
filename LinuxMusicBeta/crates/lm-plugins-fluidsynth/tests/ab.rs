//! A/B of the plugins against the legacy ones (interfaces.md, U34): `ab/build-references.sh` loads the legacy plugins
//! from the legacy build and writes what they render for each case of `ab/cases.json`; here the plugin renders the
//! same events at the same rate and period, and the difference must stay below the case's limit (−90 dBFS unless
//! the case says why not: `INTENTIONAL`).

// (the results are printed: they are the A/B's report)
#![allow(clippy::print_stdout)]

use lm_plugin::testing::{Bench, Timed, note_off, note_on};
use lm_plugin::{BuildContext, Loaded, Registry, ResourceView};
use lm_render::{EventKind, ProcessSpec};
use lm_state::{Value, from_json};

/// The default limit of a case's difference (dBFS, the peak of the difference)
const LIMIT_DB: f64 = -90.0;

/// Cases that differ on purpose, with the reason and the limit they keep
const INTENTIONAL: &[(&str, f64, &str)] = &[];

fn cases() -> Vec<Value> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ab/cases.json")).expect("cases");
    let v = from_json(&text).expect("json");
    v.get("cases").expect("cases list").items().cloned().collect()
}

fn reference(name: &str) -> Vec<f32> {
    let path = format!("{}/ab/reference/{name}.f32", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
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
                // (the earliest note of that key still held)
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

/// The plugin's state for a case's legacy settings
fn state(registry: &Registry, c: &Value) -> (String, Value) {
    let plugin = c.get("plugin").and_then(Value::as_text).expect("plugin").to_owned();
    let s = c.get("settings").expect("settings");
    let get = |k: &str| s.get(k).map(num);
    let state = match plugin.as_str() {
        "avldrums" => {
            let kit = lm_plugins_fluidsynth::avldrums::KITS[get("Drum kit").unwrap_or(0.0) as usize].key;
            Value::empty_object()
                .with("kit", Value::text(kit))
                .with("multiOutput", Value::bool(get("Output per drum piece").unwrap_or(0.0) >= 0.5))
        }
        "sf2player" => {
            // The legacy knobs by their names, at the legacy's values (a new instrument opening the file: bank 0,
            // patch 0 when the case does not say)
            let mut params = Value::empty_object();
            for (legacy, key) in [
                ("Bank", "bank"),
                ("Patch", "patch"),
                ("Gain", "gain"),
                ("Reverb", "reverbOn"),
                ("Reverb room size", "reverbRoomSize"),
                ("Reverb damping", "reverbDamping"),
                ("Reverb width", "reverbWidth"),
                ("Reverb level", "reverbLevel"),
                ("Chorus", "chorusOn"),
                ("Chorus voices", "chorusNum"),
                ("Chorus level", "chorusLevel"),
                ("Chorus speed", "chorusSpeed"),
                ("Chorus depth", "chorusDepth"),
            ] {
                if let Some(v) = get(legacy) {
                    // (the legacy models are floats)
                    params = params.with(key, Value::real(f64::from(v as f32)).expect("finite"));
                }
            }
            Value::empty_object().with("soundfont", Value::resource(gm())).with(lm_state::PARAMS_KEY, params)
        }
        p => panic!("plugin {p}"),
    };
    let complete = registry.complete(&plugin, &state).expect("complete");
    (plugin, complete)
}

/// The legacy's default SoundFont, the file the Sf2 Player cases open
fn gm() -> lm_state::Resource {
    lm_state::Resource::new("soundfont", &"67".repeat(32)).expect("key")
}

fn gm_path() -> std::path::PathBuf {
    let root = std::env::var_os("LEGACY_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."));
    root.join("legado/data/samples/soundfonts/FluidR3Mono_GM.sf3")
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1.0e-12).log10()
}

#[test]
fn the_plugins_render_as_the_legacy_ones() {
    let registry =
        Registry::new(&[&lm_plugins_fluidsynth::avldrums::DESCRIPTOR, &lm_plugins_fluidsynth::sf2player::DESCRIPTOR])
            .expect("registry");
    let mut failures = Vec::new();
    for c in cases() {
        let name = c.get("name").and_then(Value::as_text).expect("name").to_owned();
        let rate = num(c.get("rate").expect("rate")) as u32;
        let block = num(c.get("block").expect("block")) as usize;
        let frames = num(c.get("frames").expect("frames")) as usize;
        let (plugin, state) = state(&registry, &c);
        let spec = ProcessSpec { sample_rate: rate, max_block: block };
        let resources = ResourceView::new().with(gm(), Loaded::File(std::sync::Arc::from(gm_path().as_path())));
        let cx = BuildContext { spec, resources: &resources, channels: 0 };
        let ports = registry.ports(&plugin, &state).expect("ports");
        let channels: usize = ports.outputs.iter().map(|p| p.channels as usize).sum();
        let processor = registry.build(&plugin, &state, &cx).expect("build");
        let out = Bench::instrument(processor, spec, channels, 1).render(frames, block, &events(&c), false);
        let want = reference(&name);
        assert_eq!(want.len(), frames * channels, "{name}: the reference has another layout");
        let (mut peak, mut diff) = (0.0f64, 0.0f64);
        for f in 0..frames {
            for (ch, o) in out.iter().enumerate() {
                let w = f64::from(want[f * channels + ch]);
                peak = peak.max(w.abs());
                diff = diff.max((f64::from(o[f]) - w).abs());
            }
        }
        if std::env::var_os("AB_DEBUG").is_some() {
            for (ch, o) in out.iter().enumerate() {
                let (mut d, mut p, mut first) = (0.0f64, 0.0f64, None);
                for f in 0..frames {
                    let w = f64::from(want[f * channels + ch]);
                    p = p.max(w.abs());
                    let e = (f64::from(o[f]) - w).abs();
                    if e > 1.0e-6 && first.is_none() {
                        first = Some(f);
                    }
                    d = d.max(e);
                }
                println!("  channel {ch}: peak {:.1}, difference {:.1}, first at {first:?}", db(p), db(d));
            }
        }
        let limit = INTENTIONAL.iter().find(|(n, ..)| *n == name).map_or(LIMIT_DB, |(_, l, _)| *l);
        println!("{name}: peak {:.1} dBFS, difference {:.1} dBFS (limit {limit})", db(peak), db(diff));
        assert!(peak > 1.0e-3, "{name}: the reference is silent");
        if db(diff) > limit {
            failures.push(format!("{name}: {:.1} dBFS", db(diff)));
        }
    }
    assert!(failures.is_empty(), "differences above the limit: {failures:?}");
}
