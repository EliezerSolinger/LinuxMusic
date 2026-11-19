#!/usr/bin/env bash
# sanitize-all.sh <sanitizer build dir> <output dir> [<more projects>...]
# Renders the A/B projects, the bundled projects and any others given with a build configured with
# -DWANT_DEBUG_ASAN=ON (AddressSanitizer and UndefinedBehaviorSanitizer) or -DWANT_DEBUG_TSAN=ON, and reports
# each project whose render failed, took longer than five minutes, or made the sanitizer report anything.
set -u
if [ $# -lt 2 ]; then echo "usage: $0 <sanitizer build dir> <output dir> [<projects>...]" >&2; exit 2; fi
build=$(cd "$1" && pwd) || exit 2
out=$2
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
rm -rf "$out"; mkdir -p "$out/cfg"
out=$(cd "$out" && pwd)
export ASAN_OPTIONS=detect_leaks=0:halt_on_error=1 UBSAN_OPTIONS=print_stacktrace=1
export TSAN_OPTIONS="suppressions=$root/tests/tsan.supp"
export QT_QPA_PLATFORM=offscreen LINUXMUSIC_CONFIG_DIR="$out/cfg"
projects=("$here"/ab-projects/*.mmp)
while IFS= read -r -d '' f; do projects+=("$f"); done < <(find "$root/data/projects" \( -name '*.mmp' -o -name '*.mmpz' \) -print0)
projects+=("${@:3}")
problems=0
for project in "${projects[@]}"; do
	name=$(basename "$project")
	log="$out/$name.log"
	"$build/linuxmusic" render "$project" -o "$out/out.wav" > "$log" 2>&1 &
	pid=$!
	( sleep 300; kill -9 $pid 2>/dev/null ) & watchdog=$!
	wait $pid; status=$?
	kill $watchdog 2>/dev/null; wait $watchdog 2>/dev/null
	if [ $status -ne 0 ] || grep -qE 'runtime error|ERROR: AddressSanitizer|WARNING: ThreadSanitizer' "$log"; then
		echo "PROBLEM $name (exit $status, see $log)"
		problems=$((problems + 1))
	fi
done
echo "${#projects[@]} projects, $problems with problems"
[ $problems -eq 0 ]
