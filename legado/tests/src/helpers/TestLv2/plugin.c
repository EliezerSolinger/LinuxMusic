/*
 * plugin.c - an LV2 plugin for the tests (see Lv2IsolationTest): a gain applied through the worker, scaled by a CV
 * input, a fixed latency, an event input it reads, and a control that crashes it
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

#include <lv2/atom/atom.h>
#include <lv2/atom/util.h>
#include <lv2/core/lv2.h>
#include <lv2/urid/urid.h>
#include <lv2/worker/worker.h>

#define PLUGIN_URI "urn:linuxmusic:test:isolation"

enum { PortIn, PortOut, PortGain, PortCrash, PortLatency, PortMod, PortEvents, PortCount };

typedef struct
{
	const float* in;
	float* out;
	const float* gain;
	const float* crash;
	float* latency;
	const float* mod;
	const LV2_Atom_Sequence* events;
	LV2_Worker_Schedule* schedule;
	float requested; // the gain asked of the worker
	float applied;   // the gain the worker answered with
} Plugin;

static LV2_Handle instantiate(const LV2_Descriptor* descriptor, double rate, const char* bundle,
	const LV2_Feature* const* features)
{
	(void)descriptor; (void)rate; (void)bundle;
	Plugin* self = (Plugin*)calloc(1, sizeof(Plugin));
	if (!self) { return NULL; }
	for (int i = 0; features[i]; ++i)
	{
		if (!strcmp(features[i]->URI, LV2_WORKER__schedule)) { self->schedule = (LV2_Worker_Schedule*)features[i]->data; }
	}
	if (!self->schedule) { free(self); return NULL; }
	self->requested = -1.f;
	self->applied = 1.f;
	return self;
}

static void connect_port(LV2_Handle instance, uint32_t port, void* data)
{
	Plugin* self = (Plugin*)instance;
	switch (port)
	{
	case PortIn: self->in = (const float*)data; break;
	case PortOut: self->out = (float*)data; break;
	case PortGain: self->gain = (const float*)data; break;
	case PortCrash: self->crash = (const float*)data; break;
	case PortLatency: self->latency = (float*)data; break;
	case PortMod: self->mod = (const float*)data; break;
	case PortEvents: self->events = (const LV2_Atom_Sequence*)data; break;
	default: break;
	}
}

static void run(LV2_Handle instance, uint32_t frames)
{
	Plugin* self = (Plugin*)instance;
	if (*self->crash > 0.5f) { raise(SIGSEGV); } // as a plugin with a bug would
	if (*self->gain != self->requested)
	{
		self->requested = *self->gain;
		self->schedule->schedule_work(self->schedule->handle, sizeof(float), &self->requested);
	}
	// (reading the events, as a plugin taking MIDI would: an empty sequence must be valid)
	if (self->events) { LV2_ATOM_SEQUENCE_FOREACH(self->events, event) { (void)event; } }
	for (uint32_t i = 0; i < frames; ++i) { self->out[i] = self->in[i] * self->applied * (1.f + self->mod[i]); }
	*self->latency = 32.f;
}

static LV2_Worker_Status work(LV2_Handle instance, LV2_Worker_Respond_Function respond,
	LV2_Worker_Respond_Handle handle, uint32_t size, const void* data)
{
	(void)instance;
	return respond(handle, size, data);
}

static LV2_Worker_Status work_response(LV2_Handle instance, uint32_t size, const void* data)
{
	Plugin* self = (Plugin*)instance;
	if (size == sizeof(float)) { memcpy(&self->applied, data, sizeof(float)); }
	return LV2_WORKER_SUCCESS;
}

static const void* extension_data(const char* uri)
{
	static const LV2_Worker_Interface worker = {work, work_response, NULL};
	return !strcmp(uri, LV2_WORKER__interface) ? &worker : NULL;
}

static void cleanup(LV2_Handle instance) { free(instance); }

static const LV2_Descriptor s_descriptor = {PLUGIN_URI, instantiate, connect_port, NULL, run, NULL, cleanup, extension_data};

LV2_SYMBOL_EXPORT const LV2_Descriptor* lv2_descriptor(uint32_t index)
{
	return index == 0 ? &s_descriptor : NULL;
}
