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

# The chip plugins, rebuilt without fused multiply-adds into a plugin directory of their own: the emulators' tables
# are made in double at start-up, and the new program builds them without contraction on every platform (the same
# tables on x86-64, arm64 and any compiler), so the references must too
mkdir -p "$work/plugins"
for plugin in sid freeboy opulenz; do
	for object in $(ninja -t query "plugins/lib$plugin.so" | sed -n '/input:/,/outputs:/p' | grep '\.o$'); do
		objectcompile=$(ninja -t commands "$object" | tail -1 \
			| sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" -e "s# -o [^ ]*# -o $work/$(basename "$object")#" \
				-e "s# -c # -ffp-contract=off -c #")
		sh -c "$objectcompile"
	done
	# (the emulator's static library, gme or adplug, rebuilt likewise)
	archives=""
	for archive in $(ninja -t query "plugins/lib$plugin.so" | sed -n '/input:/,/outputs:/p' | grep -o 'plugins/[^ ]*\.a$' | sort -u); do
		members=""
		for object in $(ninja -t query "$archive" | sed -n '/input:/,/outputs:/p' | grep '\.o$'); do
			objectcompile=$(ninja -t commands "$object" | tail -1 \
				| sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" -e "s# -o [^ ]*# -o $work/a-$(basename "$object")#" \
					-e "s# -c # -ffp-contract=off -c #")
			sh -c "$objectcompile"
			members="$members $work/a-$(basename "$object")"
		done
		ar rcs "$work/$(basename "$archive")" $members
		archives="$archives -e s#$archive#$work/$(basename "$archive")#g"
	done
	pluginlink=$(ninja -t commands "plugins/lib$plugin.so" | tail -1 | sed -e 's#^: && ##' -e 's# && :$##' \
		-e "s# [^ ]*CMakeFiles/[^ ]*/\([^/ ]*\.o\)# $work/\1#g" \
		-e "s# -o plugins/lib$plugin.so# -o $work/plugins/lib$plugin.so#" $archives)
	sh -c "$pluginlink"
done

mkdir -p "$cases/reference"
LMMS_PLUGIN_DIR="$work/plugins" "$work/legacy-harness" "$cases/cases.json" "$cases/reference"
