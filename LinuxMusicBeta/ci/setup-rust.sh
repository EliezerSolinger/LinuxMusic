#!/usr/bin/env bash
# Installs a Rust toolchain with rustup and makes it the one every later step of the job uses (RUSTUP_TOOLCHAIN).
#
#   ci/setup-rust.sh msrv    [components...]   the workspace's `rust-version` (Cargo.toml): the toolchain the
#                                              project is built with (decisoes-autonomas.md: "Versão do Rust")
#   ci/setup-rust.sh stable  [components...]   the newest stable (an early warning for new lints)
#   ci/setup-rust.sh nightly [components...]   the newest nightly that has all the components (miri, sanitizers)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
kind="${1:?usage: setup-rust.sh msrv|stable|nightly [components...]}"
shift

case "$kind" in
msrv)
	toolchain="$(sed -n 's/^rust-version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$here/../Cargo.toml" | head -n 1)"
	if [ -z "$toolchain" ]; then
		echo "no rust-version in the workspace Cargo.toml" >&2
		exit 1
	fi
	;;
stable | nightly) toolchain="$kind" ;;
*)
	echo "unknown toolchain kind: $kind" >&2
	exit 1
	;;
esac

args=(toolchain install "$toolchain" --profile minimal --no-self-update)
for component in "$@"; do
	args+=(--component "$component")
done
if [ "$kind" = nightly ]; then
	# Take the newest nightly that has every requested component (miri is sometimes missing for a day or two)
	args+=(--allow-downgrade)
fi
rustup "${args[@]}"

if [ -n "${GITHUB_ENV:-}" ]; then
	echo "RUSTUP_TOOLCHAIN=$toolchain" >>"$GITHUB_ENV"
fi
export RUSTUP_TOOLCHAIN="$toolchain"
rustc --version --verbose
cargo --version
