#!/bin/sh
# Builds legacy-harness against the legacy program's build and writes the A/B references of lm-plugins-fluidsynth
# (interfaces.md, U34): the legacy AvlDrums and Sf2 Player plugins of that build, loaded and driven as the program does.
# Needs the legacy build: cmake -S legado -B build && cmake --build build --target MixerTest avldrums sf2player
# Usage: crates/lm-plugins-fluidsynth/ab/build-references.sh [legacy build directory]
# From a worktree without legado/, LEGACY_ROOT names the tree that has it (read only).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=${LEGACY_ROOT:-$(cd "$here/../../../.." && pwd)}
build=$(cd "${1:-$root/build}" && pwd)
work=$(mktemp -d)
[ -n "${KEEP:-}" ] || trap 'rm -rf "$work"' EXIT; echo "$work" >&2

cd "$build"
commands=$(ninja -t commands tests/MixerTest)
# Compile the harness as MixerTest.cpp is compiled
compile=$(printf '%s\n' "$commands" | grep 'MixerTest.cpp.o -c' \
	| sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" -e "s# -o [^ ]*# -o $work/harness.o#" \
		-e "s# -c [^ ]*MixerTest.cpp# -c $here/legacy-harness.cpp#")
sh -c "$compile"
# Link it as MixerTest is linked, with the harness in place of the test's own objects
link=$(printf '%s\n' "$commands" | tail -1 | sed -e 's#^: && ##' -e 's# && :$##' \
	-e "s# tests/CMakeFiles/MixerTest.dir/MixerTest_autogen/mocs_compilation.cpp.o##" \
	-e "s# tests/CMakeFiles/MixerTest.dir/src/core/MixerTest.cpp.o# $work/harness.o#" \
	-e "s# -o tests/MixerTest# -o $work/legacy-harness#")
sh -c "$link"

mkdir -p "$here/reference"
# The plugins as the legacy build made them; the kits from the legacy data
HARNESS_ROOT="$root" LMMS_PLUGIN_DIR="$build/plugins" LMMS_DATA_DIR="$root/legado/data" \
	"$work/legacy-harness" "$here/cases.json" "$here/reference"
