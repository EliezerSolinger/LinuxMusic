//! Discovery: the plugins installed here, and the test bundle's data read back exactly

mod common;
use common::bundle::{self, GAIN, REFUSES, STATELESS, UNOBTAINIUM, port};

use lm_sys_lilv::{Direction, InstanceConfig, PortKind, World, uri, version};

#[test]
fn the_library_is_linked_and_new_enough() {
    let mut parts = version().split('.').map(|p| p.parse::<u32>().unwrap());
    let (major, minor, micro) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap_or(0));
    assert!((major, minor, micro) >= (0, 24, 10), "{}", version());
}

#[test]
fn the_plugins_installed_on_this_machine_are_listed() {
    // Whatever is installed (possibly nothing): every plugin read must be coherent. Discovery runs no plugin code
    let world = World::new().unwrap();
    world.load_all();
    for plugin in world.plugins() {
        assert!(!plugin.uri.is_empty());
        for (i, p) in plugin.ports.iter().enumerate() {
            assert_eq!(p.index as usize, i, "{}: ports in index order", plugin.uri);
        }
        if let Some(latency) = plugin.latency_port {
            assert!((latency as usize) < plugin.ports.len(), "{}", plugin.uri);
        }
    }
}

#[test]
fn an_explicit_lv2_path_finds_the_test_bundle() {
    let world = World::new().unwrap();
    world.load_path(bundle::path().parent().unwrap().as_os_str()).unwrap();
    let uris: Vec<String> = world.plugins().into_iter().map(|p| p.uri).collect();
    for uri in [GAIN, UNOBTAINIUM, REFUSES, STATELESS] {
        assert!(uris.iter().any(|u| u == uri), "{uri} not in {uris:?}");
    }
}

#[test]
fn the_test_plugin_is_described_exactly() {
    let world = bundle::world();
    let info = world.plugin(GAIN).expect("the gain is loaded");
    assert_eq!(info.uri, GAIN);
    assert_eq!(info.name, "LM Test Gain");
    assert_eq!(info.author.as_deref(), Some("LinuxMusic tests"));
    assert!(info.verified);
    assert_eq!(info.bundle_path.canonicalize().unwrap(), bundle::path().canonicalize().unwrap());
    assert!(info.library_path.ends_with(if cfg!(target_os = "macos") { "lm_test.dylib" } else { "lm_test.so" }));
    assert!(info.library_path.is_file());
    assert_eq!(info.ports.len(), 16);
    assert_eq!(info.latency_port, Some(port::LATENCY));

    let has = |list: &[String], u: &std::ffi::CStr| list.iter().any(|f| f.as_bytes() == u.to_bytes());
    assert!(has(&info.required_features, uri::URID_MAP));
    assert!(has(&info.required_features, uri::OPTIONS));
    assert!(has(&info.required_features, uri::WORKER_SCHEDULE));
    assert!(has(&info.optional_features, uri::HARD_RT_CAPABLE));
    assert!(has(&info.extension_data, uri::WORKER_INTERFACE));
    assert!(has(&info.extension_data, uri::STATE_INTERFACE));
    assert!(has(&info.required_options, uri::PARAM_SAMPLE_RATE));
    assert!(has(&info.required_options, uri::MAX_BLOCK_LENGTH));
    assert!(info.missing(&InstanceConfig::new(48_000.0, 256)).is_empty());

    let p = |i: u32| &info.ports[i as usize];
    assert_eq!((p(port::IN).kind, p(port::IN).direction), (PortKind::Audio, Direction::Input));
    assert_eq!((p(port::OUT).kind, p(port::OUT).direction), (PortKind::Audio, Direction::Output));
    let gain = p(port::GAIN);
    assert_eq!((gain.kind, gain.direction), (PortKind::Control, Direction::Input));
    assert_eq!(gain.symbol, "gain");
    assert_eq!(gain.name, "Gain");
    assert_eq!((gain.default, gain.minimum, gain.maximum), (Some(1.0), Some(0.0), Some(2.0)));
    assert!(gain.logarithmic && !gain.toggled && !gain.integer);
    assert!(p(port::LATENCY).reports_latency && p(port::LATENCY).integer);
    assert!(p(port::TRIGGER).toggled);
    assert!(p(port::REPORT).integer);
    let midi = p(port::MIDI_IN);
    assert_eq!((midi.kind, midi.direction, midi.midi), (PortKind::Atom, Direction::Input, true));
    assert_eq!(midi.minimum_size, Some(16384));
    assert_eq!((p(port::EVENTS_OUT).kind, p(port::EVENTS_OUT).direction), (PortKind::Atom, Direction::Output));
    assert_eq!(info.port("work_ms").map(|p| p.index), Some(port::WORK_MS));
}

#[test]
fn a_required_feature_the_host_lacks_is_reported() {
    let world = bundle::world();
    let info = world.plugin(UNOBTAINIUM).unwrap();
    assert_eq!(info.missing(&InstanceConfig::new(48_000.0, 256)), vec!["urn:linuxmusic:test#unobtainium".to_owned()]);
    assert!(world.plugin("urn:linuxmusic:test:nothing").is_none());
}

#[test]
fn the_world_is_shared_across_threads() {
    let world = bundle::world();
    std::thread::scope(|s| {
        for _ in 0..4 {
            let w = world.clone();
            s.spawn(move || {
                for _ in 0..20 {
                    assert_eq!(w.plugins().len(), 4);
                    assert!(w.urid_map().map(GAIN) > 0);
                }
            });
        }
    });
    let id = world.urid_map().map("urn:linuxmusic:test#x");
    assert_eq!(world.urid_map().map("urn:linuxmusic:test#x"), id, "the same URI has the same id");
    assert_eq!(world.urid_map().unmap(id).as_deref(), Some("urn:linuxmusic:test#x"));
    assert_eq!(world.urid_map().unmap(0), None);
}
