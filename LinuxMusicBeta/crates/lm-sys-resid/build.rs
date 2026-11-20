//! Compiles the vendored reSID 1.0 (`vendor/resid`, GPL-2.0-or-later, by Dag Lem; the files the legacy program's
//! `plugins/Sid/resid` compiled, with its configured `siddefs.h` and the waveform tables its `samp2src.pl` made from
//! the `.dat` samples) and the C interface `src/shim.cc`, as the legacy build compiled them: GNU C++20, `-O2`, but
//! without floating-point contraction, so the filter tables made in double at start-up are the same on every platform
//! (the A/B references come from the legacy plugin rebuilt the same way).

fn main() {
    const SOURCES: [&str; 9] =
        ["sid.cc", "voice.cc", "wave.cc", "envelope.cc", "filter.cc", "dac.cc", "extfilt.cc", "pot.cc", "version.cc"];
    let mut build = cc::Build::new();
    build
        .cpp(true)
        // (GNU mode as the legacy build: GCC contracts a * b + c only in it, so ISO mode would round otherwise)
        .flag("-std=gnu++20")
        // No fused multiply-add anywhere: the filter's tables are made in double at start-up, and a contraction the
        // compiler may or may not do (clang on arm64 does, GCC on x86-64 cannot) would make them differ between
        // platforms. The A/B references are made from the legacy plugin rebuilt the same way
        .flag("-ffp-contract=off")
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
