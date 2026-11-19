/*
 * CrashingLadspa.c - a LADSPA plugin for the tests: it passes its input through, and crashes when its "Crash"
 * control goes above one half (see LadspaIsolationTest)
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * General Public License for more details.
 *
 * You should have received a copy of the GNU General Public
 * License along with this program (see COPYING); if not, write to the
 * Free Software Foundation, Inc., 51 Franklin Street, Fifth Floor,
 * Boston, MA 02110-1301 USA.
 *
 */

#include <signal.h>
#include <stdlib.h>
#include <string.h>

#include "ladspa.h"

enum { PortInput, PortOutput, PortCrash, PortCount };

typedef struct
{
	LADSPA_Data* ports[PortCount];
} Instance;

static LADSPA_Handle instantiate(const LADSPA_Descriptor* descriptor, unsigned long sampleRate)
{
	(void)descriptor;
	(void)sampleRate;
	return calloc(1, sizeof(Instance));
}

static void connectPort(LADSPA_Handle handle, unsigned long port, LADSPA_Data* data)
{
	if (port < PortCount) { ((Instance*)handle)->ports[port] = data; }
}

static void run(LADSPA_Handle handle, unsigned long frames)
{
	Instance* instance = (Instance*)handle;
	if (instance->ports[PortCrash] && *instance->ports[PortCrash] > 0.5f)
	{
		// As a plugin with a bug would
		raise(SIGSEGV);
	}
	if (instance->ports[PortInput] && instance->ports[PortOutput])
	{
		memmove(instance->ports[PortOutput], instance->ports[PortInput], frames * sizeof(LADSPA_Data));
	}
}

static void cleanup(LADSPA_Handle handle) { free(handle); }

static const LADSPA_PortDescriptor s_portDescriptors[PortCount] = {
	LADSPA_PORT_INPUT | LADSPA_PORT_AUDIO,
	LADSPA_PORT_OUTPUT | LADSPA_PORT_AUDIO,
	LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,
};
static const char* const s_portNames[PortCount] = {"Input", "Output", "Crash"};
static const LADSPA_PortRangeHint s_portHints[PortCount] = {
	{0, 0, 0},
	{0, 0, 0},
	{LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_MINIMUM, 0, 1},
};

static const LADSPA_Descriptor s_descriptor = {
	9999001, "crash_test", 0, "Crash test", "LinuxMusic tests", "GPL-2.0-or-later", PortCount, s_portDescriptors,
	s_portNames, s_portHints, NULL, instantiate, connectPort, NULL, run, NULL, NULL, NULL, cleanup,
};

#ifdef _WIN32
__declspec(dllexport)
#else
__attribute__((visibility("default")))
#endif
const LADSPA_Descriptor* ladspa_descriptor(unsigned long index)
{
	return index == 0 ? &s_descriptor : NULL;
}
