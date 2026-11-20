#!/bin/sh
# Builds legacy-harness against the legacy program's build and writes the A/B references of Amp & IR: the legacy
# plugin as it was built (build/plugins/libampandir.so, unchanged), run on cases.json with the files of data/.
# Needs the legacy build: cmake -S legado -B build && cmake --build build --target MixerTest ampandir
# Usage: crates/lm-plugins-ampandir/ab/build-references.sh [legacy build directory]
# From a worktree without legado/: LEGACY_ROOT=/Users/castrot1/Desktop/LinuxMusic crates/lm-plugins-ampandir/ab/...
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=${LEGACY_ROOT:-$(cd "$here/../../../.." && pwd)}
build=$(cd "${1:-$root/build}" && pwd)
work=$(mktemp -d)
[ -n "${KEEP:-}" ] || trap 'rm -rf "$work"' EXIT; echo "$work" >&2

cd "$build"
commands=$(ninja -t commands tests/MixerTest)

# Compile the harness as MixerTest.cpp is compiled (the signals with + and * only, never fused)
compile=$(printf '%s\n' "$commands" | grep 'MixerTest.cpp.o -c' \
	| sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" -e "s# -o [^ ]*# -o $work/harness.o#" \
		-e "s# -c [^ ]*MixerTest.cpp# -ffp-contract=off -c $here/legacy-harness.cpp#")
sh -c "$compile"

# Link it as MixerTest is linked, with the harness in place of the test's own objects
link=$(printf '%s\n' "$commands" | tail -1 | sed -e 's#^: && ##' -e 's# && :$##' \
	-e "s# tests/CMakeFiles/MixerTest.dir/MixerTest_autogen/mocs_compilation.cpp.o##" \
	-e "s# tests/CMakeFiles/MixerTest.dir/src/core/MixerTest.cpp.o# $work/harness.o#" \
	-e "s# -o tests/MixerTest# -o $work/legacy-harness#")
sh -c "$link"

# The plugin as the legacy program built it, alone in a plugin directory
mkdir -p "$work/plugins"
cp "$build/plugins/libampandir.so" "$work/plugins/"
mkdir -p "$here/reference"
LMMS_PLUGIN_DIR="$work/plugins" "$work/legacy-harness" "$here/cases.json" "$here/data" "$here/reference"
