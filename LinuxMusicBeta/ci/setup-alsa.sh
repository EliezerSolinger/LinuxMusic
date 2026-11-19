#!/usr/bin/env bash
# Prepares ALSA on a Linux runner for the tests (decisoes-autonomas.md: "Linux sem máquina Linux").
#
# The runners have no sound card. Two devices are offered to the tests, by environment variables (in $GITHUB_ENV):
#   LM_TEST_ALSA_NULL=null          always: the ALSA `null` PCM (accepts and produces silence, any format)
#   LM_TEST_ALSA_LOOPBACK=hw:Loopback   only when the snd-aloop kernel module could be loaded: a real kernel
#                                       sound card whose playback comes back on its capture side
# A test that needs the loopback must skip (and say so) when LM_TEST_ALSA_LOOPBACK is not set. This script never
# fails because the loopback is unavailable.
set -uo pipefail

export_var() {
	echo "$1=$2"
	if [ -n "${GITHUB_ENV:-}" ]; then echo "$1=$2" >>"$GITHUB_ENV"; fi
}

sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends alsa-utils >/dev/null || true

# The `null` PCM comes with alsa-lib's configuration
if head -c 19200 /dev/zero | aplay -q -D null -f S16_LE -r 48000 -c 2 -t raw - 2>/dev/null; then
	export_var LM_TEST_ALSA_NULL null
else
	echo "::warning::the ALSA null PCM does not work on this runner"
fi

# The loopback card needs the snd-aloop module, which on the cloud kernels is in linux-modules-extra
if ! sudo modprobe snd-aloop 2>/dev/null; then
	echo "snd-aloop not available; trying linux-modules-extra-$(uname -r)"
	sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
		"linux-modules-extra-$(uname -r)" >/dev/null 2>&1 || true
	sudo modprobe snd-aloop 2>/dev/null || true
fi
if [ -d /proc/asound/Loopback ]; then
	# The runner user is not in the audio group, and a new group needs a new login: open the devices instead
	sudo chmod a+rw /dev/snd/* || true
	export_var LM_TEST_ALSA_LOOPBACK hw:Loopback
	aplay -l || true
else
	echo "::notice::snd-aloop could not be loaded on this runner; ALSA loopback tests will skip"
fi
exit 0
