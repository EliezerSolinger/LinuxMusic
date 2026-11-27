//! A scanned plugin as an external plugin of the registry: its schema and ports, built through its helper, and the
//! generated checks that apply to a third-party plugin (round trips, the parameter sweep under the allocation trap,
//! the same state twice gives the same audio)

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lm_base::{Cancel, Progress};
use lm_plugin::{BuildContext, PluginKind, Registry, ResourceView};
use lm_plugin_proxy::{Deadline, HostConfig, external, plugin_id, scan};
use lm_render::{BlockShape, Buffers, HostOutbox, ProcessContext, ProcessSpec, Rng, TransportInfo};
use lm_time::TimeSignature;

#[test]
fn a_scanned_plugin_in_the_registry() {
    let config = Arc::new(HostConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_lm-plugin-host")),
        deadline: Deadline::Wait,
        load_timeout: Duration::from_secs(20),
        paths: Vec::new(),
        format_paths: Default::default(),
        system_paths: true,
    });
    let report =
        scan(&config, "test", &[], &["stall".to_owned()], Duration::from_secs(10), &Cancel::new(), &Progress::new())
            .unwrap();
    let started = Arc::new(Mutex::new(0usize));
    let counter = Arc::clone(&started);
    let on_start: lm_plugin_proxy::OnStart = Arc::new(move |_, _| *counter.lock().unwrap() += 1);
    let externals = report
        .plugins
        .iter()
        .filter(|p| p.uri == "gain" || p.uri == "synth")
        .map(|p| external(p, Arc::clone(&config), Some(Arc::clone(&on_start))))
        .collect();
    let registry = Registry::with_external(&[], externals).unwrap();
    let gain = plugin_id("test", "gain");
    let synth = plugin_id("test", "synth");
    assert_eq!(registry.kind(&gain), Some(PluginKind::Effect));
    assert_eq!(registry.kind(&synth), Some(PluginKind::Instrument));
    let keys: Vec<&str> = registry.params(&gain).unwrap().iter().map(|p| p.key).collect();
    assert_eq!(keys, ["gain", "latency"]);
    let names: Vec<String> = registry.entries().map(|e| e.name.to_owned()).collect();
    assert!(names.contains(&"Test gain (TEST)".to_owned()));
    lm_plugin::testing::external_contract(&registry, &gain, true);
    lm_plugin::testing::external_contract(&registry, &synth, true);
    assert!(*started.lock().unwrap() > 0, "the session hears of each helper");
}

/// The session builds an effect with the channel's channel count (2), not the plugin's own (1): a mono plugin on a
/// stereo channel runs as two instances, so both channels are heard (one instance alone would leave the second silent)
#[test]
fn a_mono_plugin_on_a_stereo_channel_runs_as_two_instances() {
    let config = Arc::new(HostConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_lm-plugin-host")),
        deadline: Deadline::Wait,
        load_timeout: Duration::from_secs(20),
        paths: Vec::new(),
        format_paths: Default::default(),
        system_paths: true,
    });
    let report =
        scan(&config, "test", &[], &["stall".to_owned()], Duration::from_secs(10), &Cancel::new(), &Progress::new())
            .unwrap();
    let mono = report.plugins.iter().find(|p| p.uri == "mono").expect("the mono test plugin");
    assert_eq!((mono.inputs, mono.outputs), (1, 1));
    let registry = Registry::with_external(&[], vec![external(mono, Arc::clone(&config), None)]).unwrap();
    let id = plugin_id("test", "mono");
    let view = ResourceView::new();
    let cx = BuildContext { spec: ProcessSpec { sample_rate: 48_000, max_block: 256 }, resources: &view, channels: 2 };
    let state = registry.defaults(&id).unwrap();
    let mut p = registry.build(&id, &state, &cx).unwrap();
    let shape = BlockShape { inputs: 2, outputs: 2, sidechain: 0, in_place: true };
    let frames = 256;
    let mut data = vec![0.5f32; 2 * frames];
    let transport = TransportInfo {
        sample_rate: 48_000,
        playing: false,
        recording: false,
        position: 0,
        tick: 0,
        ppq: 960,
        bpm: 120.0,
        time_signature: TimeSignature::COMMON,
        frame: 0,
    };
    let (mut rng, mut host) = (Rng::new(1), HostOutbox::default());
    let buffers = Buffers::from_block(&mut data, frames, frames, shape);
    let mut pcx = ProcessContext {
        frames,
        buffers,
        events: &[],
        transport: &transport,
        rng: &mut rng,
        host: &mut host,
        display: None,
    };
    p.process(&mut pcx);
    assert!(data[..frames].iter().all(|x| *x == 0.5), "the left channel through the first instance");
    assert!(data[frames..].iter().all(|x| *x == 0.5), "the right channel through the second");
}
