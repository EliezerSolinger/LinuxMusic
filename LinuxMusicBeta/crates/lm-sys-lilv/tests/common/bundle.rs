//! The test LV2 bundle (`tests/plugin`), compiled once per test binary with the system's C compiler (`CC`, else
//! `cc`) and the LV2 headers found by pkg-config (module `lv2`, installed with lilv). The library never needs a C
//! compiler: only these tests do.

#![allow(dead_code)]

use lm_sys_lilv::{InstanceConfig, World};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub const GAIN: &str = "urn:linuxmusic:test:gain";
pub const UNOBTAINIUM: &str = "urn:linuxmusic:test:unobtainium";
pub const REFUSES: &str = "urn:linuxmusic:test:refuses";
pub const STATELESS: &str = "urn:linuxmusic:test:stateless";

/// The gain's ports
pub mod port {
    pub const IN: u32 = 0;
    pub const OUT: u32 = 1;
    pub const GAIN: u32 = 2;
    pub const LATENCY: u32 = 3;
    pub const RATE: u32 = 4;
    pub const MAX_BLOCK: u32 = 5;
    pub const TRIGGER: u32 = 6;
    pub const WORKED: u32 = 7;
    pub const REPORT: u32 = 8;
    pub const WORK_MS: u32 = 9;
    pub const SEED: u32 = 10;
    pub const MIDI_IN: u32 = 11;
    pub const NOTES: u32 = 12;
    pub const EVENTS_OUT: u32 = 13;
    pub const RESTORED: u32 = 14;
    pub const END_RUNS: u32 = 15;
}

fn output(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().unwrap_or_else(|e| panic!("could not run {program}: {e}"));
    assert!(out.status.success(), "{program} {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn build() -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/plugin");
    let binary = std::env::current_exe().unwrap().file_stem().unwrap().to_string_lossy().into_owned();
    let root =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("lm-sys-lilv").join(format!("{binary}-{}", std::process::id()));
    let bundle = root.join("lm-test.lv2");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&bundle).unwrap();

    let library = if cfg!(target_os = "macos") { "lm_test.dylib" } else { "lm_test.so" };
    let pkg_config = std::env::var("PKG_CONFIG").unwrap_or_else(|_| "pkg-config".into());
    let cflags = output(&pkg_config, &["--cflags", "lv2"]);
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut args: Vec<String> =
        ["-std=c99", "-D_POSIX_C_SOURCE=200809L", "-O1", "-g", "-fPIC", "-shared", "-Wall", "-Wextra", "-Werror"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    args.extend(cflags.split_whitespace().map(str::to_owned));
    args.push(source.join("lm_test.c").to_string_lossy().into_owned());
    args.push("-o".into());
    args.push(bundle.join(library).to_string_lossy().into_owned());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    output(&cc, &refs);

    std::fs::copy(source.join("lm_test.ttl"), bundle.join("lm_test.ttl")).unwrap();
    let mut manifest = String::from(
        "@prefix lv2: <http://lv2plug.in/ns/lv2core#> .\n@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n",
    );
    for uri in [GAIN, UNOBTAINIUM, REFUSES, STATELESS] {
        manifest.push_str(&format!(
            "\n<{uri}> a lv2:Plugin ;\n\tlv2:binary <{library}> ;\n\trdfs:seeAlso <lm_test.ttl> .\n"
        ));
    }
    std::fs::write(bundle.join("manifest.ttl"), manifest).unwrap();
    bundle
}

/// The bundle folder (built on first use)
pub fn path() -> &'static Path {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE.get_or_init(build)
}

/// A world with only the test bundle loaded
pub fn world() -> World {
    let world = World::new().unwrap();
    world.load_bundle(path()).unwrap();
    world
}

/// 48 kHz, blocks up to 256
pub fn config() -> InstanceConfig {
    InstanceConfig::new(48_000.0, 256)
}

/// The log the gain writes for report id `id` (work-begin, work-end, cleanup), one entry per line
pub fn report(id: u32) -> Vec<String> {
    let text = std::fs::read_to_string(path().join(format!("report-{id}.txt"))).unwrap_or_default();
    text.lines().map(str::to_owned).collect()
}
