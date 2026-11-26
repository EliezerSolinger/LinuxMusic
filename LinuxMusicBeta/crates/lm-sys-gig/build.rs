//! Compiles the C++ shim (`src/shim.cpp`) against the system's libgig (pkg-config module `gig`, at least 4.0.0:
//! Homebrew's `libgig` on the Mac, `libgig-dev` on Debian and Ubuntu) and links it. No vendored copy: the library is the
//! one the user's system ships and updates.
//!
//! Only the `pkg-config` program is used (`PKG_CONFIG` names another). When libgig is missing the build stops here
//! with a message that says what to install, and only then: a machine that has the library builds as any other crate.

// A build script talks to Cargo by printing to stdout
#![allow(clippy::print_stdout)]

use std::env;
use std::path::PathBuf;
use std::process::Command;

const MODULE: &str = "gig";
const MIN_VERSION: &str = "4.0.0";
const INSTALL_HINT: &str = "install it with `apt install libgig-dev` (Debian, Ubuntu) or `brew install libgig` (macOS)";

fn pkg_config(args: &[&str]) -> Result<String, String> {
    let program = env::var("PKG_CONFIG").unwrap_or_else(|_| "pkg-config".to_owned());
    let output = Command::new(&program)
        .args(args)
        .output()
        .map_err(|e| format!("could not run `{program}` ({e}); pkg-config is needed to find {MODULE}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{program} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("`{program}` printed something that is not UTF-8: {e}"))
}

/// "4.6.0" → [4, 6, 0]; anything after a number's digits is ignored
fn numbers(version: &str) -> Vec<u64> {
    version
        .trim()
        .split('.')
        .map(|part| part.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))
        .collect()
}

fn at_least(version: &str, min: &str) -> bool {
    let (mut v, mut m) = (numbers(version), numbers(min));
    let len = v.len().max(m.len());
    v.resize(len, 0);
    m.resize(len, 0);
    v >= m
}

fn main() {
    for var in ["PKG_CONFIG", "PKG_CONFIG_PATH", "PKG_CONFIG_LIBDIR", "PKG_CONFIG_SYSROOT_DIR"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=src/shim.h");

    let version = match pkg_config(&["--modversion", MODULE]) {
        Ok(version) => version.trim().to_owned(),
        Err(e) => panic!("libgig ({MODULE}) was not found: {e}; {INSTALL_HINT}"),
    };
    if !at_least(&version, MIN_VERSION) {
        panic!("libgig {version} is too old: at least {MIN_VERSION} is needed; {INSTALL_HINT}");
    }

    let mut build = cc::Build::new();
    build.cpp(true).flag("-std=c++17").warnings(false).file("src/shim.cpp");
    match pkg_config(&["--cflags", MODULE]) {
        Ok(flags) => {
            for flag in flags.split_whitespace() {
                if let Some(dir) = flag.strip_prefix("-I") {
                    build.include(dir);
                } else if let Some(def) = flag.strip_prefix("-D") {
                    build.define(def, None);
                }
            }
        }
        Err(e) => {
            println!("cargo:warning={MODULE}: `--cflags` failed ({e}); the headers are looked for in the default path")
        }
    }
    build.compile("lm_gig_shim");

    let mut linked = false;
    match pkg_config(&["--libs", MODULE]) {
        Ok(libs) => {
            for flag in libs.split_whitespace() {
                if let Some(dir) = flag.strip_prefix("-L") {
                    println!("cargo:rustc-link-search=native={dir}");
                } else if let Some(lib) = flag.strip_prefix("-l") {
                    println!("cargo:rustc-link-lib={lib}");
                    linked = true;
                }
            }
        }
        Err(e) => println!("cargo:warning={MODULE}: `--libs` failed ({e}); linking -l{MODULE} from its libdir"),
    }
    if !linked {
        if let Ok(libdir) = pkg_config(&["--variable=libdir", MODULE]) {
            let libdir = PathBuf::from(libdir.trim());
            if !libdir.as_os_str().is_empty() {
                println!("cargo:rustc-link-search=native={}", libdir.display());
            }
        }
        println!("cargo:rustc-link-lib={MODULE}");
    }
    println!("cargo:rustc-env=LM_SYS_GIG_LINKED_VERSION={version}");
}
