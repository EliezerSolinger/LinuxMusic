/*
 * A small LV2 bundle for the tests of lm-sys-lilv. Compiled by the tests themselves (tests/common/bundle.rs) with the
 * system's C compiler and the LV2 headers; never part of the library.
 *
 * Four plugins:
 *   urn:linuxmusic:test:gain         a gain with a worker, a state, MIDI in and out, the options reported back,
 *                                    a latency port, and a log of work/cleanup for the join test
 *   urn:linuxmusic:test:unobtainium  requires a feature no host has
 *   urn:linuxmusic:test:refuses      its instantiate returns NULL
 *   urn:linuxmusic:test:stateless    no ports, no worker, no state
 */

#include <lv2/atom/atom.h>
#include <lv2/atom/util.h>
#include <lv2/buf-size/buf-size.h>
#include <lv2/core/lv2.h>
#include <lv2/midi/midi.h>
#include <lv2/options/options.h>
#include <lv2/parameters/parameters.h>
#include <lv2/state/state.h>
#include <lv2/urid/urid.h>
#include <lv2/worker/worker.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define BLOB_SIZE 1000
#define KEY_SEED "urn:linuxmusic:test#seed"
#define KEY_BLOB "urn:linuxmusic:test#blob"

enum {
  P_IN,
  P_OUT,
  P_GAIN,
  P_LATENCY,
  P_RATE,
  P_MAX_BLOCK,
  P_TRIGGER,
  P_WORKED,
  P_REPORT,
  P_WORK_MS,
  P_SEED,
  P_MIDI_IN,
  P_NOTES,
  P_EVENTS_OUT,
  P_RESTORED,
  P_END_RUNS,
  N_PORTS
};

typedef struct {
  void* ports[N_PORTS];
  LV2_URID_Map* map;
  LV2_Worker_Schedule* schedule;
  LV2_URID atom_Int, atom_Float, atom_Chunk, atom_Sequence, midi_Event, k_seed, k_blob;
  float rate;
  int max_block;
  float last_trigger;
  int triggers;
  int worked;
  int notes;
  int end_runs;
  int report_id;
  int restored_via_worker;
  int32_t seed;
  uint8_t blob[BLOB_SIZE];
  char* bundle;
} Gain;

typedef struct {
  int32_t seed;
  int32_t report;
  int32_t ms;
  int32_t from_restore;
} Request;

typedef struct {
  int32_t seed;
  int32_t from_restore;
  uint8_t blob[BLOB_SIZE];
} Response;

static void fill_blob(int32_t seed, uint8_t* out) {
  for (int i = 0; i < BLOB_SIZE; ++i) {
    out[i] = (uint8_t)(seed * 31 + i * 7 + (i * i) % 13);
  }
}

static void report(const Gain* self, int id, const char* what) {
  if (id <= 0 || !self->bundle) {
    return;
  }
  char path[4096];
  snprintf(path, sizeof(path), "%s/report-%d.txt", self->bundle, id);
  FILE* f = fopen(path, "a");
  if (f) {
    fprintf(f, "%s\n", what);
    fclose(f);
  }
}

static LV2_Handle instantiate_gain(const LV2_Descriptor* d, double rate, const char* bundle,
                                   const LV2_Feature* const* features) {
  (void)d;
  (void)rate;
  Gain* self = (Gain*)calloc(1, sizeof(Gain));
  if (!self) {
    return NULL;
  }
  const LV2_Options_Option* options = NULL;
  for (int i = 0; features && features[i]; ++i) {
    if (!strcmp(features[i]->URI, LV2_URID__map)) {
      self->map = (LV2_URID_Map*)features[i]->data;
    } else if (!strcmp(features[i]->URI, LV2_WORKER__schedule)) {
      self->schedule = (LV2_Worker_Schedule*)features[i]->data;
    } else if (!strcmp(features[i]->URI, LV2_OPTIONS__options)) {
      options = (const LV2_Options_Option*)features[i]->data;
    }
  }
  if (!self->map || !options || !self->schedule) {
    free(self);
    return NULL;
  }
  LV2_URID_Map* m = self->map;
  self->atom_Int = m->map(m->handle, LV2_ATOM__Int);
  self->atom_Float = m->map(m->handle, LV2_ATOM__Float);
  self->atom_Chunk = m->map(m->handle, LV2_ATOM__Chunk);
  self->atom_Sequence = m->map(m->handle, LV2_ATOM__Sequence);
  self->midi_Event = m->map(m->handle, LV2_MIDI__MidiEvent);
  self->k_seed = m->map(m->handle, KEY_SEED);
  self->k_blob = m->map(m->handle, KEY_BLOB);
  LV2_URID k_rate = m->map(m->handle, LV2_PARAMETERS__sampleRate);
  LV2_URID k_max = m->map(m->handle, LV2_BUF_SIZE__maxBlockLength);
  self->rate = -1.0f;
  self->max_block = -1;
  for (const LV2_Options_Option* o = options; o->key; ++o) {
    if (o->key == k_rate && o->type == self->atom_Float) {
      self->rate = *(const float*)o->value;
    } else if (o->key == k_max && o->type == self->atom_Int) {
      self->max_block = *(const int32_t*)o->value;
    }
  }
  if (self->rate <= 0.0f || self->max_block <= 0) {
    free(self);
    return NULL;
  }
  self->bundle = strdup(bundle);
  fill_blob(0, self->blob);
  return self;
}

static void connect_gain(LV2_Handle h, uint32_t port, void* data) {
  Gain* self = (Gain*)h;
  if (port < N_PORTS) {
    self->ports[port] = data;
  }
}

static void activate_gain(LV2_Handle h) {
  Gain* self = (Gain*)h;
  self->last_trigger = 0.0f;
}

static void run_gain(LV2_Handle h, uint32_t n) {
  Gain* self = (Gain*)h;
  const float* in = (const float*)self->ports[P_IN];
  float* out = (float*)self->ports[P_OUT];
  const float gain = *(const float*)self->ports[P_GAIN];
  for (uint32_t i = 0; i < n; ++i) {
    out[i] = in[i] * gain;
  }

  self->report_id = (int)*(const float*)self->ports[P_REPORT];
  const float trigger = *(const float*)self->ports[P_TRIGGER];
  if (trigger > 0.5f && self->last_trigger <= 0.5f) {
    Request r = {++self->triggers * 17 + 3, self->report_id, (int32_t)*(const float*)self->ports[P_WORK_MS], 0};
    self->schedule->schedule_work(self->schedule->handle, sizeof(r), &r);
  }
  self->last_trigger = trigger;

  /* MIDI: count note-ons, echo every MIDI event to the output */
  const LV2_Atom_Sequence* seq = (const LV2_Atom_Sequence*)self->ports[P_MIDI_IN];
  LV2_Atom_Sequence* echo = (LV2_Atom_Sequence*)self->ports[P_EVENTS_OUT];
  const uint32_t space = echo->atom.size;
  echo->atom.type = self->atom_Sequence;
  echo->atom.size = sizeof(LV2_Atom_Sequence_Body);
  echo->body.unit = 0;
  echo->body.pad = 0;
  LV2_ATOM_SEQUENCE_FOREACH (seq, ev) {
    if (ev->body.type != self->midi_Event) {
      continue;
    }
    const uint8_t* msg = (const uint8_t*)(ev + 1);
    if (ev->body.size >= 3 && (msg[0] & 0xF0) == 0x90 && msg[2] > 0) {
      ++self->notes;
    }
    const uint32_t need = (uint32_t)sizeof(LV2_Atom_Event) + lv2_atom_pad_size(ev->body.size);
    if (echo->atom.size + need <= space) {
      LV2_Atom_Event* copy = (LV2_Atom_Event*)((uint8_t*)LV2_ATOM_BODY(&echo->atom) + echo->atom.size);
      memcpy(copy, ev, sizeof(LV2_Atom_Event) + ev->body.size);
      echo->atom.size += need;
    }
  }

  *(float*)self->ports[P_LATENCY] = 7.0f;
  *(float*)self->ports[P_RATE] = self->rate;
  *(float*)self->ports[P_MAX_BLOCK] = (float)self->max_block;
  *(float*)self->ports[P_WORKED] = (float)self->worked;
  *(float*)self->ports[P_SEED] = (float)self->seed;
  *(float*)self->ports[P_NOTES] = (float)self->notes;
  *(float*)self->ports[P_RESTORED] = (float)self->restored_via_worker;
  *(float*)self->ports[P_END_RUNS] = (float)self->end_runs;
}

static LV2_Worker_Status work(LV2_Handle h, LV2_Worker_Respond_Function respond, LV2_Worker_Respond_Handle handle,
                              uint32_t size, const void* data) {
  Gain* self = (Gain*)h;
  if (size != sizeof(Request)) {
    return LV2_WORKER_ERR_UNKNOWN;
  }
  const Request* r = (const Request*)data;
  report(self, r->report, "work-begin");
  if (r->ms > 0) {
    struct timespec t = {r->ms / 1000, (long)(r->ms % 1000) * 1000000L};
    nanosleep(&t, NULL);
  }
  Response resp;
  resp.seed = r->seed;
  resp.from_restore = r->from_restore;
  fill_blob(r->seed, resp.blob);
  report(self, r->report, "work-end");
  return respond(handle, sizeof(resp), &resp);
}

static LV2_Worker_Status work_response(LV2_Handle h, uint32_t size, const void* data) {
  Gain* self = (Gain*)h;
  if (size != sizeof(Response)) {
    return LV2_WORKER_ERR_UNKNOWN;
  }
  const Response* resp = (const Response*)data;
  self->seed = resp->seed;
  memcpy(self->blob, resp->blob, BLOB_SIZE);
  if (resp->from_restore) {
    self->restored_via_worker = 1;
  } else {
    ++self->worked;
  }
  return LV2_WORKER_SUCCESS;
}

static LV2_Worker_Status end_run(LV2_Handle h) {
  ++((Gain*)h)->end_runs;
  return LV2_WORKER_SUCCESS;
}

static LV2_State_Status save(LV2_Handle h, LV2_State_Store_Function store, LV2_State_Handle handle, uint32_t flags,
                             const LV2_Feature* const* features) {
  (void)flags;
  (void)features;
  Gain* self = (Gain*)h;
  const uint32_t pod = LV2_STATE_IS_POD | LV2_STATE_IS_PORTABLE;
  LV2_State_Status s = store(handle, self->k_seed, &self->seed, sizeof(int32_t), self->atom_Int, pod);
  if (s) {
    return s;
  }
  return store(handle, self->k_blob, self->blob, BLOB_SIZE, self->atom_Chunk, pod);
}

static LV2_State_Status restore(LV2_Handle h, LV2_State_Retrieve_Function retrieve, LV2_State_Handle handle,
                                uint32_t flags, const LV2_Feature* const* features) {
  (void)flags;
  Gain* self = (Gain*)h;
  size_t size = 0;
  uint32_t type = 0, vflags = 0;
  const void* seed = retrieve(handle, self->k_seed, &size, &type, &vflags);
  if (seed && size == sizeof(int32_t) && type == self->atom_Int) {
    self->seed = *(const int32_t*)seed;
  } else {
    self->seed = 0;
  }
  const void* blob = retrieve(handle, self->k_blob, &size, &type, &vflags);
  if (blob && size == BLOB_SIZE && type == self->atom_Chunk) {
    memcpy(self->blob, blob, BLOB_SIZE);
  } else {
    fill_blob(self->seed, self->blob);
  }
  /* A restore that needs slow work (loading a file) schedules it with the worker given to restore */
  for (int i = 0; features && features[i]; ++i) {
    if (!strcmp(features[i]->URI, LV2_WORKER__schedule)) {
      LV2_Worker_Schedule* s = (LV2_Worker_Schedule*)features[i]->data;
      Request r = {self->seed, 0, 0, 1};
      s->schedule_work(s->handle, sizeof(r), &r);
    }
  }
  return LV2_STATE_SUCCESS;
}

static const void* extension_data_gain(const char* uri) {
  static const LV2_Worker_Interface worker = {work, work_response, end_run};
  static const LV2_State_Interface state = {save, restore};
  if (!strcmp(uri, LV2_WORKER__interface)) {
    return &worker;
  }
  if (!strcmp(uri, LV2_STATE__interface)) {
    return &state;
  }
  return NULL;
}

static void cleanup_gain(LV2_Handle h) {
  Gain* self = (Gain*)h;
  report(self, self->report_id, "cleanup");
  free(self->bundle);
  free(self);
}

/* The others: no ports, nothing to do */
typedef struct {
  int unused;
} Empty;

static LV2_Handle instantiate_empty(const LV2_Descriptor* d, double rate, const char* bundle,
                                    const LV2_Feature* const* features) {
  (void)d;
  (void)rate;
  (void)bundle;
  (void)features;
  return calloc(1, sizeof(Empty));
}

static LV2_Handle instantiate_refuses(const LV2_Descriptor* d, double rate, const char* bundle,
                                      const LV2_Feature* const* features) {
  (void)d;
  (void)rate;
  (void)bundle;
  (void)features;
  return NULL;
}

static void connect_empty(LV2_Handle h, uint32_t port, void* data) {
  (void)h;
  (void)port;
  (void)data;
}

static void run_empty(LV2_Handle h, uint32_t n) {
  (void)h;
  (void)n;
}

static void cleanup_empty(LV2_Handle h) { free(h); }

static const void* extension_data_none(const char* uri) {
  (void)uri;
  return NULL;
}

static const LV2_Descriptor descriptors[] = {
    {"urn:linuxmusic:test:gain", instantiate_gain, connect_gain, activate_gain, run_gain, NULL, cleanup_gain,
     extension_data_gain},
    {"urn:linuxmusic:test:unobtainium", instantiate_empty, connect_empty, NULL, run_empty, NULL, cleanup_empty,
     extension_data_none},
    {"urn:linuxmusic:test:refuses", instantiate_refuses, connect_empty, NULL, run_empty, NULL, cleanup_empty,
     extension_data_none},
    {"urn:linuxmusic:test:stateless", instantiate_empty, connect_empty, NULL, run_empty, NULL, cleanup_empty,
     extension_data_none},
};

LV2_SYMBOL_EXPORT const LV2_Descriptor* lv2_descriptor(uint32_t index) {
  return index < sizeof(descriptors) / sizeof(descriptors[0]) ? &descriptors[index] : NULL;
}
