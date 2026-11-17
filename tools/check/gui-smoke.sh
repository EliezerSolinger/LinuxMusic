#!/usr/bin/env bash
# gui-smoke.sh <build dir> <output dir> [<projects>...]
# Opens the GUI headless (QT_QPA_PLATFORM=offscreen, no sound device) with each project (the A/B and demo
# projects if none are given), shows the Piano Roll and the Song Editor, saves a screenshot and quits, which
# also clears the project. Reports a crash, a hang (two minutes), or a sanitizer finding with a sanitizer build.
# GUI_SMOKE_ACTIONS chooses what to do (LINUXMUSIC_SCREENSHOT_ACTION values; "pianoroll arrangement" by default;
# "play" plays the song with the mixer and the Piano Roll open, for ThreadSanitizer), and GUI_SMOKE_DELAY how
# long to wait before the screenshot, in milliseconds (GUI_SMOKE_TIMEOUT: how long before a run counts as hung,
# 120 seconds by default).
set -u
if [ $# -lt 2 ]; then echo "usage: $0 <build dir> <output dir> [<projects>...]" >&2; exit 2; fi
build=$(cd "$1" && pwd) || exit 2
out=$2
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
rm -rf "$out"; mkdir -p "$out"
out=$(cd "$out" && pwd)
if [ $# -gt 2 ]; then
	projects=("${@:3}")
else
	projects=("$here"/ab-projects/*.mmp)
	while IFS= read -r -d '' f; do projects+=("$f"); done < <(find "$root/data/projects/demos" -name '*.mmp*' -print0)
fi
export ASAN_OPTIONS=detect_leaks=0:halt_on_error=1 UBSAN_OPTIONS=print_stacktrace=1
export TSAN_OPTIONS="suppressions=$root/tests/tsan.supp"
export QT_QPA_PLATFORM=offscreen SDL_AUDIODRIVER=dummy
problems=0
for project in "${projects[@]}"; do
	name=$(basename "$project")
	config="$out/cfg-$name"
	mkdir -p "$config/workspace"
	# A configured program with no sound device: no first-run dialog, nothing to record from
	cat > "$config/.linuxmusicrc.xml" <<XML
<?xml version="1.0"?>
<!DOCTYPE linuxmusicrc-config-file>
<linuxmusic configversion="3">
  <app configured="1"/>
  <audioengine audiodev="Dummy (no sound output)" framesperaudiobuffer="256" mididev="Dummy (no MIDI support)"/>
  <paths workingdir="$config/workspace/"/>
</linuxmusic>
XML
	for action in ${GUI_SMOKE_ACTIONS:-pianoroll arrangement}; do
		log="$out/$name.$action.log"
		LINUXMUSIC_CONFIG_DIR="$config" LINUXMUSIC_SCREENSHOT="$out/$name.$action.png" \
			LINUXMUSIC_SCREENSHOT_ACTION=$action LINUXMUSIC_SCREENSHOT_DELAY=${GUI_SMOKE_DELAY:-0} \
			"$build/linuxmusic" "$project" > "$log" 2>&1 &
		pid=$!
		( sleep ${GUI_SMOKE_TIMEOUT:-120}; kill -9 $pid 2>/dev/null ) & watchdog=$!
		wait $pid; status=$?
		kill $watchdog 2>/dev/null; wait $watchdog 2>/dev/null
		if [ $status -ne 0 ] || [ ! -s "$out/$name.$action.png" ] \
			|| grep -qE 'runtime error|ERROR: AddressSanitizer|WARNING: ThreadSanitizer' "$log"; then
			echo "PROBLEM $name $action (exit $status, see $log)"
			problems=$((problems + 1))
		fi
	done
done
echo "${#projects[@]} projects, $problems problems"
[ $problems -eq 0 ]
