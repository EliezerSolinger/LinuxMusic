//! Links the system's Rubber Band library (pkg-config module `rubberband`, at least 3.0.0, the first with the R3
//! "finer" engine and the start-delay calls): Homebrew on the Mac, the distribution's package on Linux
//! (`librubberband-dev` on Debian and Ubuntu; Ubuntu 22.04 ships 2.0, too old: 24.04 or later). No vendored copy and no
//! `-sys` crate: the library is the one the user's system ships and updates.
//!
//! Only the `pkg-config` program is used (the `pkg-config` crate would be one more dependency of the workspace for a
//! dozen lines). `PKG_CONFIG` names another program, as the `pkg-config` crate allows too. Homebrew's `.pc` file links
//! Apple's Accelerate framework (`-framework Accelerate`), which is passed on as a framework link.

// A build script talks to Cargo by printing to stdout
#![allow(clippy::print_stdout)]

use std::env;
use std::process::Command;

const MODULE: &str = "rubberband";
const MIN_VERSION: &str = "3.0.0";
const INSTALL_HINT: &str = "install it with `apt install librubberband-dev` (Debian 12, Ubuntu 24.04 or later) or `brew install rubberband` (macOS)";

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

/// "2.6.1" → [2, 6, 1]; anything after a number's digits (a "-rc1") is ignored
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

    let version = match pkg_config(&["--modversion", MODULE]) {
        Ok(version) => version.trim().to_owned(),
        Err(e) => panic!("{MODULE} was not found: {e}; {INSTALL_HINT}"),
    };
    // (compared here: pkgconf refuses `--atleast-version`, and `--libs`, when a `.pc` the library's own
    // `Requires.private` names is missing, as on a macOS runner, although the library is there)
    if !at_least(&version, MIN_VERSION) {
        panic!("{MODULE} {version} is too old: at least {MIN_VERSION} is needed; {INSTALL_HINT}");
    }

    let libs = match pkg_config(&["--libs", MODULE]) {
        Ok(libs) => libs,
        Err(e) => {
            println!("cargo:warning={MODULE}: `--libs` failed ({e}); linking -lrubberband from its libdir");
            let libdir = pkg_config(&["--variable=libdir", MODULE]).unwrap_or_else(|e| panic!("{e}"));
            format!("-L{} -lrubberband", libdir.trim())
        }
    };
    let mut linked = false;
    let mut flags = libs.split_whitespace();
    while let Some(flag) = flags.next() {
        if let Some(dir) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
            linked = linked || lib == "rubberband";
        } else if flag == "-framework" {
            match flags.next() {
                Some(framework) => println!("cargo:rustc-link-lib=framework={framework}"),
                None => panic!("`pkg-config --libs {MODULE}` ends with `-framework` and no name ({libs:?})"),
            }
        }
    }
    if !linked {
        panic!("`pkg-config --libs {MODULE}` does not link librubberband ({libs:?})");
    }
    println!("cargo:rustc-env=LM_SYS_LINKED_VERSION={version}");
}
