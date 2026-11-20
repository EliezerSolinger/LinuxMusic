#!/usr/bin/env bash
# Runs the tests of the whole workspace under a sanitizer, on nightly (interfaces.md, Parte 4 and I-17):
#
#   ci/run-sanitizer.sh address   ASan (with LeakSanitizer on, as ASan does on Linux)
#   ci/run-sanitizer.sh thread    TSan
#   ci/run-sanitizer.sh leak      LSan alone
#
# The standard library is rebuilt with the same sanitizer (-Zbuild-std; needed by TSan, and it keeps every crate's
# ABI the same for the others). An explicit --target keeps proc-macros and build scripts out of the sanitizer.
# Doctests are left out (they run in the normal jobs). LM_TEST_SANITIZER tells a test which sanitizer is on, so it can
# shorten a very long loop.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here/.."

kind="${1:?usage: run-sanitizer.sh address|thread|leak}"
case "$kind" in
address) export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=1:detect_stack_use_after_return=1:check_initialization_order=1:strict_init_order=1}" ;;
thread) export TSAN_OPTIONS="${TSAN_OPTIONS:-halt_on_error=1:second_deadlock_stack=1}" ;;
leak) export LSAN_OPTIONS="${LSAN_OPTIONS:-report_objects=1}" ;;
*)
	echo "unknown sanitizer: $kind" >&2
	exit 1
	;;
esac

target="$(rustc -vV | sed -n 's/^host: //p')"
export RUSTFLAGS="-Zsanitizer=$kind -Cforce-frame-pointers=yes ${RUSTFLAGS:-}"
export RUSTDOCFLAGS="-Zsanitizer=$kind ${RUSTDOCFLAGS:-}"
export LM_TEST_SANITIZER="$kind"
# lm-session: create and destroy the session 1000 times while playing, under ASan and LSan (interfaces.md, Parte 4,
# step 9: "criar e destruir a sessão mil vezes tocando, sob ASan e LSan")
export LM_SESSION_CYCLES="${LM_SESSION_CYCLES:-1000}"

echo "sanitizer: $kind on $target"
cargo test -Zbuild-std --target "$target" --workspace --lib --bins --tests
