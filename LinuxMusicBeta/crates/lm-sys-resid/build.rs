//! Compiles the vendored reSID 1.0 (`vendor/resid`, GPL-2.0-or-later, by Dag Lem; the files the legacy program's
//! `plugins/Sid/resid` compiled, with its configured `siddefs.h` and the waveform tables its `samp2src.pl` made from
//! the `.dat` samples) and the C interface `src/shim.cc`, as the legacy build compiled them: GNU C++20, `-O2`, the
//! compiler's own floating-point contraction (the filter tables are made in double at start-up: the same compiler
//! and flags give the same tables, bit for bit, as the legacy plugin's).

fn main() {
    const SOURCES: [&str; 9] =
        ["sid.cc", "voice.cc", "wave.cc", "envelope.cc", "filter.cc", "dac.cc", "extfilt.cc", "pot.cc", "version.cc"];
    let mut build = cc::Build::new();
    build
        .cpp(true)
        // (GNU mode as the legacy build: GCC contracts a * b + c only in it, so ISO mode would round otherwise)
        .flag("-std=gnu++20")
        .opt_level(2)
        .warnings(false)
        .include("vendor/resid")
        .define("VERSION", "\"1.0\"")
        .file("src/shim.cc");
    for source in SOURCES {
        build.file(format!("vendor/resid/{source}"));
    }
    build.compile("lm_resid");
    println!("cargo:rerun-if-changed=src/shim.cc");
    println!("cargo:rerun-if-changed=vendor/resid");
}
