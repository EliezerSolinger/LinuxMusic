#!/usr/bin/env bash
# determinism.sh <build dir> [<output dir>]
# Renders the A/B projects twice with the same build and requires the two renders to be the same, bit for bit:
# a render must not depend on the threads' timing. Projects that use random numbers (ab-random.txt) are only
# reported; ladspa-mbeq, whose FFTW plans are measured at run time, may differ below -120 dBFS.
set -u
if [ $# -lt 1 ]; then echo "usage: $0 <build dir> [<output dir>]" >&2; exit 2; fi
here=$(cd "$(dirname "$0")" && pwd)
out=${2:-$(mktemp -d)}
"$here/ab-render.sh" "$1" "$out/first" || exit 1
"$here/ab-render.sh" "$1" "$out/second" || exit 1
noise=$(mktemp)
{ cat "$here/ab-random.txt"; echo ladspa-mbeq; } > "$noise"
python3 "$here/ab-compare.py" "$out/first" "$out/second" --noise "$noise" --limit -998 || exit 1
python3 "$here/ab-compare.py" "$out/first" "$out/second" --noise "$here/ab-random.txt" --limit -120 > /dev/null
