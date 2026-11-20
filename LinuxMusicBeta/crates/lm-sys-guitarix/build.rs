//! Compiles Guitarix's units (vendor/gx, as the legacy plugin compiles them: legado/plugins/Guitarix/CMakeLists.txt)
//! with the fork's glue (glue/) and the C interface (glue/shim.cpp) into one static archive, and links FFTW (single
//! precision, pkg-config module `fftw3f`: the Detune's FFT, as in the legacy program).
//!
//! The C++ is compiled at `-O2` in every profile and **without fused multiply-adds** (`-ffp-contract=off`): the same
//! numbers in a debug and a release build, on arm64 and on x86-64, and the same as the legacy plugin rebuilt the
//! same way for the A/B (`crates/lm-plugins-guitarix/ab/build-references.sh`). No debug information (5 MB of
//! generated DSP code would weigh hundreds of MB in the target folder).

// A build script talks to Cargo by printing to stdout
#![allow(clippy::print_stdout)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Guitarix's hand-written units (gx/plugins), compiled one file each
const UNITS: &[&str] = &[
    "ts9sim",
    "aclipper",
    "mxrdist",
    "bossds1",
    "bmp",
    "flanger_gx",
    "mbc",
    "mbd",
    "mbe",
    "mbdel",
    "mbclipper",
    "bassboom",
    "mbchor",
    "gcb_95",
    "fuzzfacefm",
    "fuzzfacerm",
    "hornet",
    "susta",
    "hfb",
    "muff",
    "scream",
    "lpbboost",
    "foxeylady",
    "hogsfoot",
    "cstb",
    "coverd",
    "rangem",
    "ruin",
    "ffreak",
    "fumaster",
    "fuzzdrive",
    "rolandwah",
    "astrofuzz",
    "buffb",
    "eldist",
    "mole",
    "buzz",
    "bfuzz",
    "axface",
    "mbcs",
    "zita_rev1",
    "abgate",
    "reversedelay",
];

/// Guitarix's Faust units (gx/faust-generated), each in its namespace of `gx_engine` (`gx_engine/<name>`: in
/// `gx_engine` itself); the `gx_amps` in one file, as Guitarix has them (they share the tube tables of valve.h)
const FAUST_UNITS: &[&str] = &[
    "gx_effects/gain",
    "gx_effects/low_high_pass",
    "gx_effects/highbooster",
    "gx_effects/selecteq",
    "gx_effects/gx_distortion",
    "gx_effects/bitdowner",
    "gx_effects/thick_distortion",
    "gx_effects/impulseresponse",
    "gx_effects/compressor",
    "gx_effects/expander",
    "gx_effects/overdrive",
    "gx_effects/echo",
    "gx_effects/delay",
    "gx_effects/freeverb",
    "gx_effects/biquad",
    "gx_effects/tremolo",
    "gx_effects/phaser_mono",
    "gx_effects/chorus_mono",
    "gx_effects/flanger_mono",
    "gx_effects/gx_feedback",
    "gx_effects/peak_eq",
    "gx_effects/digital_delay",
    "gx_effects/ring_modulator",
    "gx_effects/graphiceq",
    "gx_effects/duck_delay",
    "gx_effects/baxandall",
    "gx_effects/distortion2",
    "gx_effects/fuzzface",
    "gx_effects/trbuff",
    "gx_effects/chorus",
    "gx_effects/flanger",
    "gx_effects/phaser",
    "gx_effects/stereodelay",
    "gx_effects/stereoecho",
    "gx_effects/moog",
    "gx_amps/gx_ampmodul",
    "gx_effects/tonecontroll",
    "gx_effects/digital_delay_st",
    "gx_effects/stereoverb",
    "gx_effects/dattorros_progenitor",
    "gx_effects/panoram_enhancer",
    "gx_effects/bass_enhancer",
    "gx_effects/ring_modulator_st",
    "gx_effects/duck_delay_st",
    "gx_effects/crybaby",
    "gx_effects/autowah",
    "gx_effects/colwah",
    "gx_effects/dallaswah",
    "gx_effects/foxwah",
    "gx_effects/jenwah",
    "gx_effects/maestrowah",
    "gx_effects/selwah",
    "gx_effects/voxwah",
    "gx_effects/rolwah",
    "gx_effects/colbwah",
    "gx_effects/jenbasswah",
    "gx_tonestacks/tonestack_default",
    "gx_tonestacks/tonestack_bassman",
    "gx_tonestacks/tonestack_twin",
    "gx_tonestacks/tonestack_princeton",
    "gx_tonestacks/tonestack_jcm800",
    "gx_tonestacks/tonestack_jcm2000",
    "gx_tonestacks/tonestack_mlead",
    "gx_tonestacks/tonestack_m2199",
    "gx_tonestacks/tonestack_ac30",
    "gx_tonestacks/tonestack_soldano",
    "gx_tonestacks/tonestack_mesa",
    "gx_tonestacks/tonestack_jtm45",
    "gx_tonestacks/tonestack_ac15",
    "gx_tonestacks/tonestack_peavey",
    "gx_tonestacks/tonestack_ibanez",
    "gx_tonestacks/tonestack_roland",
    "gx_tonestacks/tonestack_ampeg",
    "gx_tonestacks/tonestack_ampeg_rev",
    "gx_tonestacks/tonestack_sovtek",
    "gx_tonestacks/tonestack_bogner",
    "gx_tonestacks/tonestack_groove",
    "gx_tonestacks/tonestack_crunch",
    "gx_tonestacks/tonestack_fender_blues",
    "gx_tonestacks/tonestack_fender_default",
    "gx_tonestacks/tonestack_fender_deville",
    "gx_tonestacks/tonestack_gibsen",
    "gx_tonestacks/tonestack_engl",
    "gx_poweramps/champ",
    "gx_poweramps/epiphone",
    "gx_poweramps/princeton",
    "gx_poweramps/orangedarkterror",
    "gx_poweramps/plexiel34",
    "gx_poweramps/supersonic",
    "gx_effects/noise_shaper",
    "gx_effects/softclip",
    "gx_effects/bassbooster",
    "gx_effects/gx_ampout",
    "gx_effects/gxfeed",
    "gx_effects/gx_outputlevel",
    "gx_engine/balance",
    "gx_engine/fizz_remover",
    "gx_engine/hardlim",
    "gx_amps/gxamp",
    "gx_amps/gxamp2",
    "gx_amps/gxamp3",
    "gx_amps/gxamp4",
    "gx_amps/gxamp5",
    "gx_amps/gxamp6",
    "gx_amps/gxamp7",
    "gx_amps/gxamp8",
    "gx_amps/gxamp9",
    "gx_amps/gxamp10",
    "gx_amps/gxamp11",
    "gx_amps/gxamp12",
    "gx_amps/gxamp13",
    "gx_amps/gxamp14",
    "gx_amps/gxamp15",
    "gx_amps/gxamp16",
    "gx_amps/gxamp17",
    "gx_amps/gxamp18",
    "gx_amps/gxnoamp",
];

/// The fork's glue (glue/) and the C interface
const GLUE: &[&str] = &[
    "GuitarixUnits.cpp",
    "GuitarixAmp.cpp",
    "GuitarixConvolution.cpp",
    "GuitarixDetune.cpp",
    "GuitarixVibe.cpp",
    "GuitarixSelectors.cpp",
    "shim.cpp",
];

/// The support Guitarix's units run on
const SUPPORT: &[&str] = &[
    "gx_resampler.cpp",
    "zita-resampler/resampler.cc",
    "zita-resampler/resampler-table.cc",
    "zita-convolver/zita-convolver.cc",
];

const FFTW: &str = "fftw3f";
const INSTALL_HINT: &str = "install it with `apt install libfftw3-dev` (Debian, Ubuntu) or `brew install fftw` (macOS)";

fn pkg_config(args: &[&str]) -> Result<String, String> {
    let program = env::var("PKG_CONFIG").unwrap_or_else(|_| "pkg-config".to_owned());
    let output = Command::new(&program)
        .args(args)
        .output()
        .map_err(|e| format!("could not run `{program}` ({e}); pkg-config is needed to find {FFTW}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{program} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("`{program}` printed something that is not UTF-8: {e}"))
}

fn main() {
    for var in ["PKG_CONFIG", "PKG_CONFIG_PATH", "PKG_CONFIG_LIBDIR", "PKG_CONFIG_SYSROOT_DIR"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=glue");
    println!("cargo:rerun-if-changed=vendor");

    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let gx = here.join("vendor/gx");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    // FFTW: its include folder and its library
    let cflags =
        pkg_config(&["--cflags", FFTW]).unwrap_or_else(|e| panic!("{FFTW} was not found: {e}; {INSTALL_HINT}"));
    let libs = pkg_config(&["--libs", FFTW]).unwrap_or_else(|e| panic!("{FFTW} was not found: {e}; {INSTALL_HINT}"));

    // The Faust units' wrappers, as CMake writes them from faust_unit.cpp.in
    let template = fs::read_to_string(gx.join("faust_unit.cpp.in")).expect("faust_unit.cpp.in");
    let wrappers = out.join("gx_faust");
    fs::create_dir_all(&wrappers).expect("the wrappers' folder");
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut amps = String::new();
    for unit in FAUST_UNITS {
        let (namespace, name) = unit.split_once('/').expect("namespace/name");
        let wrapped = gx.join("faust-generated").join(format!("{name}.cc"));
        let include = format!("#include \"{}\"", wrapped.display());
        if namespace == "gx_amps" {
            amps.push_str(&include);
            amps.push('\n');
            continue;
        }
        let (open, close) =
            if namespace == "gx_engine" { (String::new(), "") } else { (format!("namespace {namespace} {{"), "}") };
        sources.push(write_wrapper(&wrappers, &template, &format!("{namespace}_{name}"), name, &open, close, &include));
    }
    sources.push(write_wrapper(&wrappers, &template, "gx_amps", "amps", "namespace gx_amps {", "}", &amps));
    sources.extend(UNITS.iter().map(|u| gx.join("plugins").join(format!("{u}.cc"))));
    sources.extend(SUPPORT.iter().map(|s| gx.join(s)));
    sources.extend(GLUE.iter().map(|g| here.join("glue").join(g)));

    // The archive is made again only when a source, a flag or the target changed (the workspace's rule touches every
    // `.rs` before a build, which reruns this script: recompiling 5 MB of C++ each time would cost minutes)
    let fingerprint = fingerprint(&sources, &cflags);
    let stamp = out.join("lm_guitarix.fingerprint");
    let archive = out.join("liblm_guitarix.a");
    if archive.exists() && fs::read_to_string(&stamp).ok().as_deref() == Some(fingerprint.as_str()) {
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static=lm_guitarix");
        link_cpp_and_fftw(&libs);
        return;
    }
    let _ = fs::remove_file(&stamp);

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("gnu++20")
        .opt_level(2)
        .debug(false)
        .warnings(false)
        .flag("-w")
        .flag("-ffp-contract=off")
        .flag("-fvisibility=hidden")
        .define("NDEBUG", None)
        .include(here.join("glue"))
        .include(gx.join("headers"))
        .include(gx.join("plugins"))
        .include(gx.join("zita-resampler"))
        .include(gx.join("engine"))
        .include(gx.join("faust"))
        .include(gx.join("zita-convolver"))
        .files(&sources);
    for flag in cflags.split_whitespace() {
        if let Some(dir) = flag.strip_prefix("-I") {
            build.include(dir);
        }
    }
    build.compile("lm_guitarix");
    fs::write(&stamp, &fingerprint).expect("the fingerprint");
    link_fftw(&libs);
}

/// The C++ standard library (what `cc::Build::compile` prints for a C++ archive) and FFTW
fn link_cpp_and_fftw(libs: &str) {
    let target = env::var("TARGET").unwrap_or_default();
    let stdlib = if target.contains("apple") || target.contains("freebsd") { "c++" } else { "stdc++" };
    println!("cargo:rustc-link-lib={stdlib}");
    link_fftw(libs);
}

fn link_fftw(libs: &str) {
    // (its planner made thread safe: glue/shim.cpp)
    println!("cargo:rustc-link-lib=fftw3f_threads");
    for flag in libs.split_whitespace() {
        if let Some(dir) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
}

/// A hash of every source's text, of the headers' (vendor/gx and glue), of this script and of the target and flags
fn fingerprint(sources: &[PathBuf], cflags: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> = sources.to_vec();
    let mut dirs = vec![here.join("vendor"), here.join("glue")];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.push(here.join("build.rs"));
    files.sort();
    files.dedup();
    let mut h = DefaultHasher::new();
    for f in &files {
        f.hash(&mut h);
        fs::read(f).unwrap_or_default().hash(&mut h);
    }
    for var in ["TARGET", "HOST", "CXX", "CXXFLAGS"] {
        env::var(var).unwrap_or_default().hash(&mut h);
    }
    cflags.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn write_wrapper(
    dir: &Path,
    template: &str,
    file: &str,
    name: &str,
    open: &str,
    close: &str,
    include: &str,
) -> PathBuf {
    let text = template
        .replace("@UNIT_NAME@", name)
        .replace("@UNIT_OPEN@", open)
        .replace("@UNIT_CLOSE@", close)
        .replace("@WRAPPED_INCLUDES@", include);
    let path = dir.join(format!("{file}.cpp"));
    // (written only when it changed: cc recompiles what is newer than its object)
    if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        fs::write(&path, text).expect("a wrapper");
    }
    path
}
