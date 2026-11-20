//! A scanned plugin as an external plugin of the registry: its schema and ports, built through its helper, and the
//! generated checks that apply to a third-party plugin (round trips, the parameter sweep under the allocation trap,
//! the same state twice gives the same audio)

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lm_base::{Cancel, Progress};
use lm_plugin::{PluginKind, Registry};
use lm_plugin_proxy::{Deadline, HostConfig, external, plugin_id, scan};

#[test]
fn a_scanned_plugin_in_the_registry() {
    let config = Arc::new(HostConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_lm-plugin-host")),
        deadline: Deadline::Wait,
        load_timeout: Duration::from_secs(20),
        paths: Vec::new(),
    });
    let report = scan(&config, "test", &[], &[], Duration::from_secs(2), &Cancel::new(), &Progress::new()).unwrap();
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
