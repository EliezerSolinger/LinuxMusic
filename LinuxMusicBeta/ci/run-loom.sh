#!/usr/bin/env bash
# Runs the loom model checks of every crate that has `cfg(loom)` code (found by ci/workspace.py loom), in release
# (interfaces.md, I-17): RUSTFLAGS="--cfg loom" cargo test -p <crate> --release.
#
# Only the library and the integration tests run: doctests are compiled without `--cfg loom` but link the library
# built with it, so loom's types would be used outside a model. Loom's own knobs (LOOM_MAX_PREEMPTIONS,
# LOOM_MAX_BRANCHES, LOOM_LOG...) pass through from the environment.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here/.."

mapfile -t crates < <(python3 "$here/workspace.py" loom)
if [ "${#crates[@]}" -eq 0 ]; then
	echo "no crate has cfg(loom) code: nothing to check"
	exit 0
fi
echo "loom crates: ${crates[*]}"

packages=()
for crate in "${crates[@]}"; do packages+=(-p "$crate"); done

export RUSTFLAGS="--cfg loom ${RUSTFLAGS:-}"
cargo test --locked --release "${packages[@]}" --lib --tests -- --nocapture
