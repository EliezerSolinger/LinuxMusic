//! Compiles the vendored Neural Amp Modeler core (`vendor/nam`, MIT) with its vendored Eigen (`vendor/eigen`, the
//! headers its code includes, MPL-2.0) and nlohmann/json (`vendor/nlohmann`, MIT), and the C interface
//! (`src/shim.cpp`), into one static archive linked whole.
//!
//! - **Whole archive:** NAM's architectures (WaveNet, LSTM, ConvNet, Linear, the containers) register themselves
//!   from static objects in their own files, which nothing references by name; a plain static archive would let
//!   the linker drop them and every model would be "unknown architecture" (the legacy build used an object library
//!   for the same reason).
//! - **Flags:** as the legacy program built it (`-O2 -DNDEBUG`, C++20, `NAM_SAMPLE_FLOAT`), whatever the Cargo
//!   profile: the model is unusable unoptimised, and the A/B compares against the legacy build's code. No
//!   `-march`: the program runs on any CPU of its targets.

// A build script talks to Cargo by printing to stdout
#![allow(clippy::print_stdout)]

use std::fs;
use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "cpp") {
            out.push(path);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor = root.join("vendor");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=vendor");

    let mut files = Vec::new();
    sources(&vendor.join("nam/NAM"), &mut files);
    files.push(root.join("src/shim.cpp"));

    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .opt_level(2)
        .debug(false)
        .define("NAM_SAMPLE_FLOAT", None)
        .define("NDEBUG", None)
        .include(vendor.join("nam"))
        .include(vendor.join("eigen"))
        .include(vendor.join("nlohmann"))
        .warnings(false)
        .flag_if_supported("-w")
        .files(&files)
        .link_lib_modifier("+whole-archive")
        .compile("lm_nam");
}
