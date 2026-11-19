#!/usr/bin/env bash
# corrupt-all.sh <build dir> <output dir>
# Renders damaged copies (corrupt-projects.py) of the A/B projects. Refusing a damaged project is fine; crashing,
# hanging for more than two minutes, or (with a sanitizer build) a sanitizer report is a problem.
set -u
if [ $# -ne 2 ]; then echo "usage: $0 <build dir> <output dir>" >&2; exit 2; fi
build=$(cd "$1" && pwd) || exit 2
out=$2
here=$(cd "$(dirname "$0")" && pwd)
rm -rf "$out"; mkdir -p "$out/projects" "$out/logs"
out=$(cd "$out" && pwd)
python3 "$here/corrupt-projects.py" "$out/projects" "$here"/ab-projects/*.mmp || exit 2
export ASAN_OPTIONS=detect_leaks=0:halt_on_error=1 UBSAN_OPTIONS=print_stacktrace=1
export QT_QPA_PLATFORM=offscreen
problems=0
total=0
for project in "$out"/projects/*.mmp; do
	name=$(basename "$project" .mmp)
	log="$out/logs/$name.log"
	mkdir -p "$out/cfg-$name"
	LINUXMUSIC_CONFIG_DIR="$out/cfg-$name" "$build/linuxmusic" render "$project" -o "$out/out.wav" > "$log" 2>&1 &
	pid=$!
	( sleep 120; kill -9 $pid 2>/dev/null ) & watchdog=$!
	wait $pid; status=$?
	kill $watchdog 2>/dev/null; wait $watchdog 2>/dev/null
	rm -rf "$out/cfg-$name"
	total=$((total + 1))
	if [ $status -ge 128 ] || grep -qE 'runtime error|ERROR: AddressSanitizer|WARNING: ThreadSanitizer' "$log"; then
		echo "PROBLEM $name (exit $status, see $log)"
		problems=$((problems + 1))
	fi
done
echo "$total damaged projects, $problems with problems"
[ $problems -eq 0 ]
