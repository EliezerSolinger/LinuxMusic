#!/bin/sh
# Builds legacy-harness against the legacy program's build and writes the A/B references of lm-dsp (interfaces.md,
# U34). Needs the legacy build: cmake -S legado -B build && cmake --build build --target MixerTest
# Usage: crates/lm-dsp/ab/build-references.sh [legacy build directory]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
build=$(cd "${1:-$root/build}" && pwd)
work=$(mktemp -d)
[ -n "${KEEP:-}" ] || trap 'rm -rf "$work"' EXIT; echo "$work" >&2

cd "$build"
commands=$(ninja -t commands tests/MixerTest)

# Compile the harness as MixerTest.cpp is compiled, without contracting a * b + c into one rounding (the signals must
# be the same bit for bit as the ones tests/ab.rs makes)
compile=$(printf '%s\n' "$commands" | grep 'MixerTest.cpp.o -c' \
	| sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" -e "s# -o [^ ]*# -o $work/harness.o#" \
		-e "s# -c [^ ]*MixerTest.cpp# -I$root/legado/plugins/Delay -I$root/legado/plugins/ReverbSC -ffp-contract=off -c $here/legacy-harness.cpp#")
sh -c "$compile"

# The legacy objects to rebuild without fused multiply-adds (CONTRACT_OFF="TubeStage GainStage ..."), to tell
# rounding apart from a real difference
objects=""
for unit in ${CONTRACT_OFF:-}; do
	unitcompile=$(ninja -t commands "src/CMakeFiles/lmmsobjs.dir/core/$unit.cpp.o" | tail -1 \
		| sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" -e "s# -o [^ ]*# -o $work/$unit.o#" -e "s# -c # -ffp-contract=off -c #")
	sh -c "$unitcompile"
	objects="$objects -e s#src/CMakeFiles/lmmsobjs.dir/core/$unit.cpp.o#$work/$unit.o#"
done

# The plugins' DSP the harness uses, from the plugins' own objects' commands, without fused multiply-adds
for target in plugins/Delay/CMakeFiles/delay.dir/Lfo.cpp.o plugins/Delay/CMakeFiles/delay.dir/StereoDelay.cpp.o \
	plugins/ReverbSC/CMakeFiles/reverbsc.dir/base.c.o plugins/ReverbSC/CMakeFiles/reverbsc.dir/revsc.c.o \
	plugins/ReverbSC/CMakeFiles/reverbsc.dir/dcblock.c.o; do
	name=$(basename "$target" .o)
	plugincompile=$(ninja -t commands "$target" | tail -1 \
		| sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" -e "s# -o [^ ]*# -o $work/$name.o#" -e "s# -c # -ffp-contract=off -c #")
	sh -c "$plugincompile"
	plugins="${plugins:-} $work/$name.o"
done

# Link it as MixerTest is linked, with the harness in place of the test's own objects
link=$(printf '%s\n' "$commands" | tail -1 | sed -e 's#^: && ##' -e 's# && :$##' \
	-e "s# tests/CMakeFiles/MixerTest.dir/MixerTest_autogen/mocs_compilation.cpp.o##" \
	-e "s# tests/CMakeFiles/MixerTest.dir/src/core/MixerTest.cpp.o# $work/harness.o $plugins#" \
	-e "s# -o tests/MixerTest# -o $work/legacy-harness#" $objects)
sh -c "$link"

# The effect plugins the harness loads (plugin-effect cases), rebuilt without fused multiply-adds into a plugin
# directory of their own: their discrete logic (an attack or a hold that starts or not) turns one rounding into an
# audible difference
mkdir -p "$work/plugins"
for plugin in compressor eq amplifier stereomatrix stereoenhancer bassbooster; do
	for object in $(ninja -t query "plugins/lib$plugin.so" | sed -n '/input:/,/outputs:/p' | grep '\.o$'); do
		objectcompile=$(ninja -t commands "$object" | tail -1 \
			| sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##" -e "s# -o [^ ]*# -o $work/$(basename "$object")#" \
				-e "s# -c # -ffp-contract=off -c #")
		sh -c "$objectcompile"
	done
	pluginlink=$(ninja -t commands "plugins/lib$plugin.so" | tail -1 | sed -e 's#^: && ##' -e 's# && :$##' \
		-e "s# [^ ]*CMakeFiles/$plugin.dir/[^ ]*/\([^/ ]*\.o\)# $work/\1#g" \
		-e "s# [^ ]*CMakeFiles/$plugin.dir/\([^/ ]*\.o\)# $work/\1#g" \
		-e "s# -o plugins/lib$plugin.so# -o $work/plugins/lib$plugin.so#")
	sh -c "$pluginlink"
done

mkdir -p "$here/reference"
LMMS_PLUGIN_DIR="$work/plugins" "$work/legacy-harness" "$here/cases.json" "$here/reference"
