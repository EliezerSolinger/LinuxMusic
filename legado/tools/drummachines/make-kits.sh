#!/usr/bin/env bash
# Makes the Drum Machines instrument's sampler style kits (data/samples/drummachines/sp1200, mpc60) from the
# program's own drum samples (data/samples/drums), with character.py: <piece> <source sample> <pitch in semitones>
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
make() {
	local kind=$1; shift
	mkdir -p "$root/data/samples/drummachines/$kind"
	while [ $# -gt 0 ]; do
		python3 "$root/tools/drummachines/character.py" "$kind" "$root/data/samples/drums/$2.ogg" \
			"$root/data/samples/drummachines/$kind/$1.flac" "$3"
		shift 3
	done
}
make sp1200 kick kick_hiphop01 -2  kick2 bassdrum02 -3  snare snare_hiphop01 -2  snare2 snare_muffled02 -1 \
	clap clap03 0  rim rim01 0  hihat_closed hihat_closed02 0  hihat_pedal hihat_closed04 0  hihat_open hihat_opened02 -1 \
	shaker shaker02 0  wood wood01 0  kick_low kick_soft02 -4  snare_low snare05 -3
make mpc60 kick kick04 0  kick2 kick_soft02 0  snare snare05 0  snare2 snare_short01 0  clap clap02 0  rim snare_rim01 0 \
	hihat_closed hihat_closed01 0  hihat_pedal hihat_closed03 0  hihat_open hihat_opened03 0  shaker shaker03 0 \
	cabasa shaker01 0  snare3 snare07 0  kick3 bassdrum04 0
