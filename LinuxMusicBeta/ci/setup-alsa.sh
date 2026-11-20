#!/usr/bin/env bash
# Prepares ALSA on a Linux runner for the tests (decisoes-autonomas.md: "Linux sem máquina Linux").
#
# The runners have no sound card. What works is announced to the tests by environment variables (in $GITHUB_ENV),
# each set to 1 only when that part works on this runner:
#   LM_TEST_ALSA_NULL=1       the ALSA `null` PCM (accepts and produces silence, any format; device name "null")
#   LM_TEST_ALSA_SEQ=1        the ALSA sequencer (/dev/snd/seq, module snd-seq), open to the runner's user
#   LM_TEST_ALSA_LOOPBACK=1   the snd-aloop card (device "hw:Loopback"): its playback comes back on its capture side
# A test that needs one of them must skip (and say so) when its variable is not set. This script never fails because
# a device is unavailable. It also installs `dbus` (dbus-daemon, for the ReserveDevice1 test of lm-sys-linux).
set -uo pipefail

export_var() {
	echo "$1=$2"
	if [ -n "${GITHUB_ENV:-}" ]; then echo "$1=$2" >>"$GITHUB_ENV"; fi
}

sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends alsa-utils dbus >/dev/null || true

# The `null` PCM comes with alsa-lib's configuration
if head -c 19200 /dev/zero | aplay -q -D null -f S16_LE -r 48000 -c 2 -t raw - 2>/dev/null; then
	export_var LM_TEST_ALSA_NULL 1
else
	echo "::warning::the ALSA null PCM does not work on this runner"
fi

# The sequencer: the module may not be loaded on a machine without a sound card
sudo modprobe snd-seq 2>/dev/null || true
if [ -e /dev/snd/seq ]; then
	sudo chmod a+rw /dev/snd/seq || true
	export_var LM_TEST_ALSA_SEQ 1
else
	echo "::notice::the ALSA sequencer (snd-seq) is not available on this runner; sequencer tests will skip"
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
	export_var LM_TEST_ALSA_LOOPBACK 1
	aplay -l || true
else
	echo "::notice::snd-aloop could not be loaded on this runner; ALSA loopback tests will skip"
fi
exit 0
