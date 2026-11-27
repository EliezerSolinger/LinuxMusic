//! An LV2 plugin through the helper: `lm-sys-lilv`'s test bundle (its gain), compiled here with the system's C
//! compiler and the LV2 headers, listed, probed and run out of process

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use lm_base::{Cancel, Progress};
use lm_plugin_proxy::{Deadline, HostConfig, LoadRequest, scan, start};
use lm_render::{BlockShape, Buffers, HostOutbox, ProcessContext, Processor, Rng, TransportInfo};

const GAIN: &str = "urn:linuxmusic:test:gain";

fn output(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().unwrap_or_else(|e| panic!("{program}: {e}"));
    assert!(out.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// The bundle, built once per run
fn bundle() -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../lm-sys-lilv/tests/plugin");
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("lm-plugin-host-{}", std::process::id()));
    let bundle = root.join("lm-test.lv2");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&bundle).unwrap();
    let library = if cfg!(target_os = "macos") { "lm_test.dylib" } else { "lm_test.so" };
    let cflags = output("pkg-config", &["--cflags", "lv2"]);
    let mut args: Vec<String> =
        ["-std=c99", "-D_POSIX_C_SOURCE=200809L", "-O1", "-fPIC", "-shared"].iter().map(|s| s.to_string()).collect();
    args.extend(cflags.split_whitespace().map(str::to_owned));
    args.push(source.join("lm_test.c").display().to_string());
    args.push("-o".into());
    args.push(bundle.join(library).display().to_string());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    output(&std::env::var("CC").unwrap_or_else(|_| "cc".into()), &refs);
    std::fs::copy(source.join("lm_test.ttl"), bundle.join("lm_test.ttl")).unwrap();
    let manifest = format!(
        "@prefix lv2: <http://lv2plug.in/ns/lv2core#> .\n@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\n<{GAIN}> a lv2:Plugin ;\n\tlv2:binary <{library}> ;\n\trdfs:seeAlso <lm_test.ttl> .\n"
    );
    std::fs::write(bundle.join("manifest.ttl"), manifest).unwrap();
    bundle
}

#[test]
fn an_lv2_plugin_out_of_process() {
    let bundle = bundle();
    let config = HostConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_lm-plugin-host")),
        deadline: Deadline::Wait,
        load_timeout: Duration::from_secs(20),
        paths: vec![bundle.clone()],
        format_paths: Default::default(),
        system_paths: false,
    };
    // The scan: listed from the Turtle, probed in a helper
    let report = scan(&config, "lv2", &[], &[], Duration::from_secs(10), &Cancel::new(), &Progress::new()).unwrap();
    assert!(report.blocked.is_empty(), "{:?}", report.blocked);
    let gain = report.plugins.iter().find(|p| p.uri == GAIN).expect("the gain");
    assert_eq!((gain.inputs, gain.outputs, gain.instrument, gain.midi_in, gain.latency), (1, 1, false, true, true));
    let g = gain.params.iter().find(|p| p.key == "gain").expect("its gain port as a parameter");
    assert_eq!((g.min, g.max, g.default), (0.0, 2.0, 1.0));

    // Run: the gain at 0.5, the output is the input halved
    let req = LoadRequest {
        format: "lv2".into(),
        uri: GAIN.into(),
        file: None,
        rate: 48_000,
        max_block: 256,
        inputs: 1,
        outputs: 1,
        sidechain: 0,
        chunk: None,
        params: vec![("gain".into(), 0.5)],
    };
    let (mut proxy, hosted) = start(&config, &req).unwrap();
    let shape = BlockShape { inputs: 1, outputs: 1, sidechain: 0, in_place: true };
    let mut data = vec![0.0f32; 256];
    let mut rng = Rng::new(1);
    let mut host = HostOutbox::default();
    for b in 0..50 {
        for (k, x) in data.iter_mut().enumerate() {
            *x = ((b * 256 + k) % 101) as f32 / 101.0 - 0.5;
        }
        let input = data.clone();
        let transport = TransportInfo {
            sample_rate: 48_000,
            playing: true,
            recording: false,
            position: b as i64 * 256,
            tick: 0,
            ppq: 960,
            bpm: 120.0,
            time_signature: lm_time::TimeSignature::COMMON,
            frame: b as u64 * 256,
        };
        assert_no_alloc::assert_no_alloc(|| {
            let buffers = Buffers::from_block(&mut data, 256, 256, shape);
            let mut cx = ProcessContext {
                frames: 256,
                buffers,
                events: &[],
                transport: &transport,
                rng: &mut rng,
                host: &mut host,
                display: None,
            };
            proxy.process(&mut cx);
        });
        for (k, (y, x)) in data.iter().zip(&input).enumerate() {
            assert_eq!(*y, x * 0.5, "block {b}, sample {k}");
        }
    }
    // Its state (the control values and lilv's chunk) goes and comes back
    let chunk = hosted.save_state(Duration::from_secs(5)).unwrap().expect("a state");
    drop(proxy);
    drop(hosted);
    let req = LoadRequest { chunk: Some(chunk), params: Vec::new(), ..req };
    let (_proxy, hosted) = start(&config, &req).unwrap();
    assert!(!hosted.is_dead());
    let _ = std::fs::remove_dir_all(bundle.parent().unwrap());
}
