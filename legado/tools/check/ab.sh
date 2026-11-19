#!/usr/bin/env bash
# ab.sh <build dir> <reference dir> [<output dir>]
# Renders the A/B projects with the build and compares them with a reference rendered before (by ab-render.sh
# from the commit the work started on). Exits 1 if any project sounds different.
set -u
if [ $# -lt 2 ]; then echo "usage: $0 <build dir> <reference dir> [<output dir>]" >&2; exit 2; fi
here=$(cd "$(dirname "$0")" && pwd)
out=${3:-$(mktemp -d)}
"$here/ab-render.sh" "$1" "$out" || exit 1
python3 "$here/ab-compare.py" "$2" "$out"
