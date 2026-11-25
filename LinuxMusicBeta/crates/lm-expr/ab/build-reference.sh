#!/bin/sh
# Builds exprtk-ref against the legacy tree's exprtk (legado/plugins/Xpressive/exprtk) with the legacy Xpressive's
# flags, without floating-point contraction, and writes reference.txt for exprs.txt.
# Usage: crates/lm-expr/ab/build-reference.sh   (LEGACY_ROOT: the tree with legado/, when this is a worktree)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=${LEGACY_ROOT:-$(cd "$here/../../../.." && pwd)}
work=$(mktemp -d); [ -n "${KEEP:-}" ] || trap 'rm -rf "$work"' EXIT
c++ -std=c++20 -O1 -ffp-contract=off -w -I"$root/legado/plugins/Xpressive/exprtk" -I"$here" \
	-Dexprtk_disable_sc_andor -Dexprtk_disable_return_statement -Dexprtk_disable_break_continue \
	-Dexprtk_disable_comments -Dexprtk_disable_string_capabilities -Dexprtk_disable_rtl_io_file \
	-Dexprtk_disable_rtl_vecops -fexceptions "$here/exprtk-ref.cpp" -o "$work/exprtk-ref"
"$work/exprtk-ref" < "$here/exprs.txt" > "$here/reference.txt"
