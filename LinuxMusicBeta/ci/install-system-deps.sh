#!/usr/bin/env bash
# Installs the system packages the workspace needs on this runner: ci/system-deps/apt.txt on Linux,
# ci/system-deps/brew.txt on macOS, plus the *-gui.txt list when the workspace has an lm-gui or lm-gui-* crate.
# `--gui` takes the GUI lists even without an lm-gui crate (the prototypes job). Extra packages for one job can be
# passed as arguments.
set -euo pipefail

force_gui=false
if [ "${1:-}" = --gui ]; then
	force_gui=true
	shift
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

packages_from() {
	# One name per line; `#` starts a comment
	sed -e 's/#.*//' -e 's/[[:space:]]//g' "$1" | grep -v '^$' || true
}

has_gui_crate() {
	$force_gui || python3 "$here/workspace.py" members | grep -Eq '^lm-gui(-|$)'
}

case "$(uname -s)" in
Linux)
	mapfile -t packages < <(packages_from "$here/system-deps/apt.txt")
	if has_gui_crate; then
		echo "an lm-gui crate exists: adding the GUI packages"
		mapfile -t -O "${#packages[@]}" packages < <(packages_from "$here/system-deps/apt-gui.txt")
	fi
	packages+=("$@")
	echo "apt packages: ${packages[*]}"
	sudo apt-get update -y -qq
	sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends "${packages[@]}"
	;;
Darwin)
	packages=()
	while IFS= read -r p; do packages+=("$p"); done < <(packages_from "$here/system-deps/brew.txt")
	if has_gui_crate; then
		echo "an lm-gui crate exists: adding the GUI packages"
		while IFS= read -r p; do packages+=("$p"); done < <(packages_from "$here/system-deps/brew-gui.txt")
	fi
	packages+=("$@")
	if [ "${#packages[@]}" -eq 0 ]; then
		echo "no Homebrew packages needed"
		exit 0
	fi
	echo "brew packages: ${packages[*]}"
	HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 brew install "${packages[@]}"
	# What the lm-sys-* build scripts will ask pkg-config (a module whose own dependencies are missing fails
	# `--atleast-version` even though `--modversion` works): printed so a failure shows its reason in the log
	pkg-config --version
	for module in sndfile samplerate fluidsynth rubberband lilv-0; do
		printf '%s %s: ' "$module" "$(pkg-config --modversion "$module" 2>/dev/null || echo missing)"
		if pkg-config --print-errors --exists "$module"; then echo ok; fi
	done
	;;
*)
	echo "unsupported system: $(uname -s)" >&2
	exit 1
	;;
esac
