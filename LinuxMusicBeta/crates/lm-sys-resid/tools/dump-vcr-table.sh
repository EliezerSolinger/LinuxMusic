#!/bin/sh
# Writes vendor/resid/vcr_n_ids_term.h: the 6581 filter's VCR table as reSID's own filter.cc computes it, from the
# untouched filter.cc of the legacy tree (its exp() and log1p() are the C library's: run it on macOS, as the legacy
# build was, so that the table is the one the A/B references were made with). The vendored filter.cc then copies it
# instead of computing it, the same on every platform.
# Usage: crates/lm-sys-resid/tools/dump-vcr-table.sh <legacy reSID directory> (legado/plugins/Sid/resid/resid)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
legacy=$(cd "${1:?the legacy reSID directory}" && pwd)
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
cp "$legacy"/*.cc "$legacy"/*.h "$work"/
cp "$here/../vendor/resid/siddefs.h" "$here"/../vendor/resid/wave*.h "$work"/
c++ -std=gnu++20 -O2 -ffp-contract=off -w -DVERSION='"1.0"' -I"$work" "$here/dump-vcr-table.cc" \
	"$work/sid.cc" "$work/voice.cc" "$work/wave.cc" "$work/envelope.cc" "$work/filter.cc" "$work/dac.cc" \
	"$work/extfilt.cc" "$work/pot.cc" "$work/version.cc" -o "$work/dump"
"$work/dump" > "$here/../vendor/resid/vcr_n_ids_term.h"
