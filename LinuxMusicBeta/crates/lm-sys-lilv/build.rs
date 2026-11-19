//! Links the system's libsamplerate (pkg-config module `samplerate`, at least 0.1.9): Homebrew on the Mac, the
//! distribution's package on Linux (`libsamplerate0-dev` on Debian and Ubuntu). No vendored copy and no `-sys` crate: the
//! library is the one the user's system ships and updates.
//!
//! Only the `pkg-config` program is used (the `pkg-config` crate would be one more dependency of the workspace for a
//! dozen lines). `PKG_CONFIG` names another program, as the `pkg-config` crate allows too.

// A build script talks to Cargo by printing to stdout
#![allow(clippy::print_stdout)]

use std::env;
use std::process::Command;

const MODULE: &str = "lilv-0";
const MIN_VERSION: &str = "0.24.10";
const INSTALL_HINT: &str =
    "install it with `apt install libsamplerate0-dev` (Debian, Ubuntu) or `brew install libsamplerate` (macOS)";

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

fn main() {
    for var in ["PKG_CONFIG", "PKG_CONFIG_PATH", "PKG_CONFIG_LIBDIR", "PKG_CONFIG_SYSROOT_DIR"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=build.rs");

    let version = match pkg_config(&["--modversion", MODULE]) {
        Ok(version) => version.trim().to_owned(),
        Err(e) => panic!("{MODULE} was not found: {e}; {INSTALL_HINT}"),
    };
    if pkg_config(&["--atleast-version", MIN_VERSION, MODULE]).is_err() {
        panic!("{MODULE} {version} is too old: at least {MIN_VERSION} is needed; {INSTALL_HINT}");
    }

    let libs = pkg_config(&["--libs", MODULE]).unwrap_or_else(|e| panic!("{e}"));
    let mut linked = false;
    for flag in libs.split_whitespace() {
        if let Some(dir) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
            linked = true;
        }
    }
    if !linked {
        panic!("`pkg-config --libs {MODULE}` named no library to link ({libs:?})");
    }
    println!("cargo:rustc-env=LM_SYS_LINKED_VERSION={version}");
}
