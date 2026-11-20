#!/bin/sh
# Builds legacy-harness against the legacy program's build and writes the A/B references of lm-plugins-stk
# (interfaces.md, U34): the legacy Mallets plugin's object (MalletsSynth) and the STK 5.0.1 classes it plays, linked
# as the legacy test MixerTest is linked (the program's objects, for what Mallets.cpp needs of them).
# Needs the legacy build (cmake -S legado -B build && cmake --build build --target MixerTest malletsstk) and STK 5.0.1
# with its sources (Homebrew's `stk` keeps them in share/stk/src).
# Usage: crates/lm-plugins-stk/ab/build-references.sh [legacy build directory]
# Environment: LEGACY_ROOT (the tree with legado/ and build/, from a worktree), STK_PREFIX (default: Homebrew's),
# LIBRARY_STK=1 to link the installed libstk.a as it is (with the fused multiply-adds its build chose) instead of the
# library's sources compiled without them.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=${LEGACY_ROOT:-$(cd "$here/../../../.." && pwd)}
build=$(cd "${1:-$root/build}" && pwd)
stk=${STK_PREFIX:-$(brew --prefix stk 2>/dev/null || echo /opt/homebrew/opt/stk)}
stk=$(cd "$stk" && pwd -P)
work=$(mktemp -d)
[ -n "${KEEP:-}" ] || trap 'rm -rf "$work"' EXIT; echo "$work" >&2

cd "$build"
mallets=plugins/Stk/Mallets/CMakeFiles/malletsstk.dir
strip_deps() { sed -e "s# -MD -MT [^ ]*##" -e "s# -MF [^ ]*##"; }

# The harness, compiled as Mallets.cpp is, without contracting a * b + c into one rounding (Rust never does)
ninja -t commands "$mallets/Mallets.cpp.o" | tail -1 | strip_deps \
	| sed -e "s# -o [^ ]*# -o $work/harness.o#" \
		-e "s# -c [^ ]*Mallets.cpp# -I$root/legado/plugins/Stk/Mallets -ffp-contract=off -c $here/legacy-harness.cpp#" \
	> "$work/compile-harness.sh"
sh "$work/compile-harness.sh"

# The plugin's objects (MalletsSynth, its moc and resources), without fused multiply-adds
objects=""
for name in Mallets.cpp moc_Mallets.cpp qrc_malletsstk.cpp; do
	ninja -t commands "$mallets/$name.o" | tail -1 | strip_deps \
		| sed -e "s# -o [^ ]*# -o $work/$name.o#" -e "s# -c # -ffp-contract=off -c #" > "$work/compile-$name.sh"
	sh "$work/compile-$name.sh"
	objects="$objects $work/$name.o"
done

# STK: its sources, without fused multiply-adds, or the installed library as it is
if [ -n "${LIBRARY_STK:-}" ]; then
	objects="$objects $stk/lib/libstk.a"
else
	for unit in Stk ModalBar Modal FileWvIn FileRead FileLoop SineWave Envelope ADSR BiQuad OnePole TwoZero FM TubeBell \
		BandedWG DelayL; do
		c++ -std=gnu++17 -O2 -ffp-contract=off -arch "$(uname -m)" -I"$stk/include/stk" -c "$stk/share/stk/src/$unit.cpp" \
			-o "$work/stk-$unit.o"
		objects="$objects $work/stk-$unit.o"
	done
fi

# Link it as MixerTest is linked, with the harness and the objects in place of the test's own
ninja -t commands tests/MixerTest | tail -1 | sed -e 's#^: && ##' -e 's# && :$##' \
	-e "s# tests/CMakeFiles/MixerTest.dir/MixerTest_autogen/mocs_compilation.cpp.o##" \
	-e "s# tests/CMakeFiles/MixerTest.dir/src/core/MixerTest.cpp.o# $work/harness.o $objects#" \
	-e "s# -o tests/MixerTest# -o $work/legacy-harness#" > "$work/link.sh"
sh "$work/link.sh"

out=${OUT:-$here/reference}
mkdir -p "$out"
"$work/legacy-harness" "$here/cases.txt" "$stk/share/stk/rawwaves/" "$out"
