#!/usr/bin/env bash
# Builds LinuxMusicBeta on Linux in one step: installs the system packages and Rust if they are missing, then compiles
# the program (linuxmusicbeta, with the interface) and lm-headless (no interface: render, serve, slice).
#
#   ./build-linux.sh            install what is missing, then build (release)
#   ./build-linux.sh --no-deps  skip the system packages (already installed, or no sudo)
#   ./build-linux.sh --debug    debug build instead of release
#   ./build-linux.sh --run      build, then open the program
#
# Supported: Ubuntu/Debian (apt). On Fedora and Arch it prints the packages to install by hand.
# The package lists are the CI's: ci/system-deps/apt.txt (core) and ci/system-deps/apt-gui.txt (interface).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
deps=true
profile=release
run=false
for arg in "$@"; do
	case "$arg" in
	--no-deps) deps=false ;;
	--debug) profile=debug ;;
	--run) run=true ;;
	-h | --help)
		sed -n '2,11p' "$0"
		exit 0
		;;
	*)
		echo "unknown option: $arg (see --help)" >&2
		exit 1
		;;
	esac
done

if [ "$(uname -s)" != Linux ]; then
	echo "this script is for Linux; on macOS use: cargo build --release -p lm-gui-app" >&2
	exit 1
fi

packages_from() {
	# One name per line; `#` starts a comment
	sed -e 's/#.*//' -e 's/[[:space:]]//g' "$1" | grep -v '^$' || true
}

# ---- system packages -------------------------------------------------------------------------------------------
if $deps; then
	mapfile -t packages < <(packages_from "$here/ci/system-deps/apt.txt"; packages_from "$here/ci/system-deps/apt-gui.txt")
	packages+=(curl ca-certificates build-essential)
	if command -v apt-get >/dev/null; then
		missing=()
		for p in "${packages[@]}"; do
			dpkg -s "$p" >/dev/null 2>&1 || missing+=("$p")
		done
		if [ "${#missing[@]}" -gt 0 ]; then
			echo "==> installing ${#missing[@]} system packages (sudo): ${missing[*]}"
			sudo apt-get update -y -qq
			sudo DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${missing[@]}"
		else
			echo "==> system packages: all installed"
		fi
	else
		echo "==> no apt-get here. Install the equivalents of these Debian packages with your package manager," >&2
		echo "    then run again with --no-deps:" >&2
		printf '    %s\n' "${packages[@]}" >&2
		exit 1
	fi
fi

# ---- Rust --------------------------------------------------------------------------------------------------------
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
toolchain="$(sed -n 's/^rust-version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$here/Cargo.toml" | head -n 1)"
if ! command -v rustup >/dev/null; then
	echo "==> installing Rust (rustup, in ~/.cargo, no sudo)"
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
	# shellcheck disable=SC1091
	. "$HOME/.cargo/env"
fi
# The project needs at least the workspace's rust-version; the newest stable is fine
if ! rustup run stable rustc --version >/dev/null 2>&1; then
	rustup toolchain install stable --profile minimal
fi
echo "==> Rust: $(rustup run stable rustc --version) (the project needs >= $toolchain)"

# ---- build ---------------------------------------------------------------------------------------------------------
cd "$here"
flags=()
[ "$profile" = release ] && flags+=(--release)
echo "==> building ($profile): the program and lm-headless"
rustup run stable cargo build "${flags[@]}" -p lm-gui-app -p lm-headless

bin="$here/target/$profile"
echo
echo "Done:"
echo "  $bin/linuxmusicbeta          the program (with a project folder as argument, it opens it)"
echo "  $bin/lm-headless             no interface: render, info, serve, slice (--help)"
echo
echo "Audio: ALSA, the device chosen in Settings > Audio. Real-time priority comes from rtkit; for the best"
echo "latency, add yourself to the 'audio' group (see the Settings page for the exact commands)."

if $run; then
	exec "$bin/linuxmusicbeta"
fi
