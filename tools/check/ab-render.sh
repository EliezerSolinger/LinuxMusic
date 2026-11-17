#!/usr/bin/env bash
# ab-render.sh <build dir> <output dir>
# Renders every project in ab-projects/ as a 32-bit float WAV (<output dir>/<name>.wav, with its log beside it),
# headless and with a configuration of its own, so that two builds can be compared with ab-compare.py.
set -u
if [ $# -ne 2 ]; then echo "usage: $0 <build dir> <output dir>" >&2; exit 2; fi
build=$(cd "$1" && pwd) || exit 2
out=$2
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out/cfg"
out=$(cd "$out" && pwd)
export QT_QPA_PLATFORM=offscreen LINUXMUSIC_CONFIG_DIR="$out/cfg"
failed=0
for project in "$here"/ab-projects/*.mmp; do
	name=$(basename "$project" .mmp)
	if ! "$build/linuxmusic" render "$project" -a -o "$out/$name.wav" > "$out/$name.log" 2>&1; then
		echo "FAILED $name (see $out/$name.log)"
		failed=1
	fi
done
exit $failed
