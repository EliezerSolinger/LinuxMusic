#!/usr/bin/env bash
# Runs the tests of the miri crates (ci/miri-crates.txt and `[package.metadata.lm-ci] miri = true`, filtered to the
# crates that exist) under miri, on the nightly toolchain with the miri component.
#
# A test that miri cannot run (FFI, a real device, a very long loop) is marked `#[cfg_attr(miri, ignore)]` in the
# crate; MIRIFLAGS from the environment is passed through.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here/.."

mapfile -t crates < <(python3 "$here/workspace.py" miri)
if [ "${#crates[@]}" -eq 0 ]; then
	echo "no miri crate exists yet: nothing to check"
	exit 0
fi
echo "miri crates: ${crates[*]}"

packages=()
for crate in "${crates[@]}"; do packages+=(-p "$crate"); done

cargo miri setup
cargo miri test --locked "${packages[@]}"
