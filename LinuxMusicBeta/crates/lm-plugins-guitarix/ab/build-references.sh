#!/bin/sh
# Builds legacy-harness against the legacy program's build and writes the A/B references of lm-plugins-guitarix: the
# legacy Guitarix plugin itself (libguitarix.so), rebuilt without fused multiply-adds as lm-sys-guitarix compiles
# Guitarix's code, run as the legacy mixer runs an effect. Needs the legacy build:
#   cmake -S legado -B build && cmake --build build --target MixerTest guitarix
# Usage: crates/lm-plugins-guitarix/ab/build-references.sh [legacy build directory]
# From a worktree without legado/: LEGACY_ROOT=/Users/castrot1/Desktop/LinuxMusic crates/.../build-references.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=${LEGACY_ROOT:-$(cd "$here/../../../.." && pwd)}
build=$(cd "${1:-$root/build}" && pwd)
work=$(mktemp -d)
[ -n "${KEEP:-}" ] || trap 'rm -rf "$work"' EXIT; echo "$work" >&2

cd "$build"
commands=$(ninja -t commands tests/MixerTest)

# The harness, compiled as MixerTest.cpp is compiled, without fused multiply-adds
compile=$(printf '%s\n' "$commands" | grep 'MixerTest.cpp.o -c' \
	| sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" -e "s# -o [^ ]*# -o $work/harness.o#" \
		-e "s# -c [^ ]*MixerTest.cpp# -ffp-contract=off -c $here/legacy-harness.cpp#")
sh -c "$compile"

# Linked as MixerTest is linked, with the harness in place of the test's own objects
link=$(printf '%s\n' "$commands" | tail -1 | sed -e 's#^: && ##' -e 's# && :$##' \
	-e "s# tests/CMakeFiles/MixerTest.dir/MixerTest_autogen/mocs_compilation.cpp.o##" \
	-e "s# tests/CMakeFiles/MixerTest.dir/src/core/MixerTest.cpp.o# $work/harness.o#" \
	-e "s# -o tests/MixerTest# -o $work/legacy-harness#")
sh -c "$link"

# The Guitarix plugin, every object rebuilt without fused multiply-adds, into a plugin directory of its own
mkdir -p "$work/plugins" "$work/objects"
plugin=guitarix
objects=$(ninja -t query "plugins/lib$plugin.so" | sed -n '/input:/,/outputs:/p' | grep '\.o$')
mkdir -p "$work/commands"
for object in $objects; do
	flat=$(printf '%s' "$object" | tr '/' '_')
	ninja -t commands "$object" | tail -1 | sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" \
		-e "s# -o [^ ]*# -o $work/objects/$flat#" -e "s# -c # -ffp-contract=off -c #" > "$work/commands/$flat.sh"
done
# (in parallel: Guitarix is hundreds of files)
ls "$work/commands"/*.sh | xargs -P "$(sysctl -n hw.ncpu 2>/dev/null || nproc)" -n 1 sh
pluginlink=$(ninja -t commands "plugins/lib$plugin.so" | tail -1 | sed -e 's#^: && ##' -e 's# && :$##' \
	-e "s# -o plugins/lib$plugin.so# -o $work/plugins/lib$plugin.so#")
for object in $objects; do
	flat=$(printf '%s' "$object" | tr '/' '_')
	pluginlink=$(printf '%s' "$pluginlink" | sed "s# $object # $work/objects/$flat #")
done
sh -c "$pluginlink"

mkdir -p "$here/reference"
for period in $(grep -o '"block": *[0-9]*' "$here/cases.json" | grep -o '[0-9]*$' | sort -un); do
	LM_PERIOD=$period LMMS_PLUGIN_DIR="$work/plugins" "$work/legacy-harness" "$here/cases.json" "$here/reference"
done
