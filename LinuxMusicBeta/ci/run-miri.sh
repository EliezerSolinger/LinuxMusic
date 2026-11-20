#!/usr/bin/env bash
# Runs the tests of the miri crates (ci/miri-crates.txt and `[package.metadata.lm-ci] miri = true`, filtered to the
# crates that exist) under miri, on the nightly toolchain with the miri component.
#
# A test that miri cannot run (FFI, a real device, a very long loop) is marked `#[cfg_attr(miri, ignore)]` in the
# crate; MIRIFLAGS from the environment is passed through. The tests run one at a time, so the log shows which one
# was running when the time limit (LM_MIRI_MINUTES, 50 by default) was reached.
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
# Under the job's own limit, so that a test too slow for miri is named in the log instead of the job just being
# cancelled; `-- --nocapture --test-threads=1` would hide nothing, but the default runner already prints each test
# as it ends, and the one still running when time is up is the one to mark #[cfg_attr(miri, ignore)]
status=0
timeout --signal=INT "${LM_MIRI_MINUTES:-50}m" cargo miri test "${packages[@]}" -- --test-threads=1 || status=$?
if [ "$status" -eq 124 ] || [ "$status" -eq 130 ]; then
	echo "error: miri ran out of time (${LM_MIRI_MINUTES:-50} min); the test after the last one listed is too slow for miri"
fi
exit "$status"
