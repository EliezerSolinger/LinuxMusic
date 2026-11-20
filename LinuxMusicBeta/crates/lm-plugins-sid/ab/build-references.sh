#!/bin/sh
# Builds legacy-harness against the legacy program's build and writes the A/B references of a chip instrument
# (interfaces.md, U34): the legacy plugins themselves (build/plugins/lib<plugin>.so), as the program loads them.
# Needs the legacy build: cmake -S legado -B build && cmake --build build --target MixerTest sid freeboy opulenz
# Usage: crates/lm-plugins-sid/ab/build-references.sh <the crate's ab directory> [legacy build directory]
#   e.g. crates/lm-plugins-sid/ab/build-references.sh crates/lm-plugins-freeboy/ab
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cases=$(cd "${1:?the ab directory of the crate}" && pwd)
# (the legacy tree and its build: this tree, or LEGACY_ROOT when this is a worktree without them)
root=${LEGACY_ROOT:-$(cd "$here/../../../.." && pwd)}
build=$(cd "${2:-$root/build}" && pwd)
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

mkdir -p "$cases/reference"
LMMS_PLUGIN_DIR="$build/plugins" "$work/legacy-harness" "$cases/cases.json" "$cases/reference"
