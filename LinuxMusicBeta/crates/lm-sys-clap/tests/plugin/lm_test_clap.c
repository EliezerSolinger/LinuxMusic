// Two small CLAP plugins for lm-sys-clap's tests (and lm-plugin-host's), built with the vendored headers.
//
// lm.test.gain: stereo effect. Params 7 "gain" (0..4, default 1) and 9 "latency" (0..1000, stepped). Output is the
// input times gain, delayed by `latency` frames. A latency change from an event asks the host for a restart; the new
// latency is reported after the reactivation. State: the two values as doubles.
//
// lm.test.synth: instrument with a note port. Param 1 "level" (0..1, default 0.5) and 2 "last key" (0..127, read
// only, stepped). Left: level while a note is held, times the last VOLUME expression. Right: the transport's tempo
// / 1000 when a transport is given. On a note-on the plugin pushes "last key" to the host (its own parameter change)
// and marks its state dirty. State: level and last key.
#include <clap/clap.h>
#include <stdlib.h>
#include <string.h>

#define MAX_DELAY 1024

typedef struct {
   clap_plugin_t plugin;
   const clap_host_t *host;
   const clap_host_state_t *host_state;
   int synth;
   double gain, latency, pending_latency;
   int active_latency;
   float line[2][MAX_DELAY + 1];
   int pos;
   double level, last_key, volume;
   int held;
} test_t;

static test_t *self_of(const clap_plugin_t *p) { return (test_t *)p->plugin_data; }

static const char *gain_features[] = {CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, NULL};
static const char *synth_features[] = {CLAP_PLUGIN_FEATURE_INSTRUMENT, NULL};

static const clap_plugin_descriptor_t gain_desc = {CLAP_VERSION_INIT, "lm.test.gain", "Test gain", "LinuxMusic",
                                                   "", "", "", "1.0", "a gain with latency", gain_features};
static const clap_plugin_descriptor_t synth_desc = {CLAP_VERSION_INIT, "lm.test.synth", "Test synth", "LinuxMusic",
                                                    "", "", "", "1.0", "a level while notes are held", synth_features};

// ---- params ----
static uint32_t params_count(const clap_plugin_t *p) { (void)p; return 2; }

static bool params_info(const clap_plugin_t *p, uint32_t i, clap_param_info_t *info) {
   test_t *t = self_of(p);
   memset(info, 0, sizeof(*info));
   if (i > 1) return false;
   if (!t->synth) {
      if (i == 0) {
         info->id = 7; strcpy(info->name, "Gain"); info->min_value = 0; info->max_value = 4; info->default_value = 1;
         info->flags = CLAP_PARAM_IS_AUTOMATABLE;
      } else {
         info->id = 9; strcpy(info->name, "Latency"); info->min_value = 0; info->max_value = 1000;
         info->default_value = 0; info->flags = CLAP_PARAM_IS_STEPPED;
      }
   } else {
      if (i == 0) {
         info->id = 1; strcpy(info->name, "Level"); info->min_value = 0; info->max_value = 1; info->default_value = 0.5;
         info->flags = CLAP_PARAM_IS_AUTOMATABLE;
      } else {
         info->id = 2; strcpy(info->name, "Last key"); info->min_value = 0; info->max_value = 127;
         info->default_value = 0; info->flags = CLAP_PARAM_IS_STEPPED | CLAP_PARAM_IS_READONLY;
      }
   }
   strcpy(info->module, "Main");
   return true;
}

static bool params_value(const clap_plugin_t *p, clap_id id, double *out) {
   test_t *t = self_of(p);
   switch (id) {
   case 7: *out = t->gain; return !t->synth;
   case 9: *out = t->pending_latency; return !t->synth;
   case 1: *out = t->level; return t->synth;
   case 2: *out = t->last_key; return t->synth;
   }
   return false;
}

static void set_value(test_t *t, clap_id id, double v) {
   switch (id) {
   case 7: t->gain = v; break;
   case 9:
      if (v != t->pending_latency) {
         t->pending_latency = v;
         t->host->request_restart(t->host);
      }
      break;
   case 1: t->level = v; break;
   }
}

static void handle(test_t *t, const clap_event_header_t *h, const clap_output_events_t *out) {
   if (h->space_id != CLAP_CORE_EVENT_SPACE_ID) return;
   switch (h->type) {
   case CLAP_EVENT_PARAM_VALUE: {
      const clap_event_param_value_t *e = (const clap_event_param_value_t *)h;
      set_value(t, e->param_id, e->value);
      break;
   }
   case CLAP_EVENT_NOTE_ON: {
      const clap_event_note_t *e = (const clap_event_note_t *)h;
      t->held++;
      t->last_key = e->key;
      if (out) {
         clap_event_param_value_t ev;
         memset(&ev, 0, sizeof(ev));
         ev.header.size = sizeof(ev); ev.header.time = h->time; ev.header.space_id = CLAP_CORE_EVENT_SPACE_ID;
         ev.header.type = CLAP_EVENT_PARAM_VALUE; ev.param_id = 2; ev.value = e->key;
         ev.note_id = -1; ev.port_index = -1; ev.channel = -1; ev.key = -1;
         out->try_push(out, &ev.header);
      }
      if (t->host_state) t->host_state->mark_dirty(t->host);
      break;
   }
   case CLAP_EVENT_NOTE_OFF:
      if (t->held > 0) t->held--;
      break;
   case CLAP_EVENT_NOTE_CHOKE:
      t->held = 0;
      break;
   case CLAP_EVENT_NOTE_EXPRESSION: {
      const clap_event_note_expression_t *e = (const clap_event_note_expression_t *)h;
      if (e->expression_id == CLAP_NOTE_EXPRESSION_VOLUME) t->volume = e->value;
      break;
   }
   }
}

static void params_flush(const clap_plugin_t *p, const clap_input_events_t *in, const clap_output_events_t *out) {
   test_t *t = self_of(p);
   uint32_t n = in->size(in);
   for (uint32_t i = 0; i < n; i++) handle(t, in->get(in, i), out);
}

static const clap_plugin_params_t params_ext = {params_count, params_info, params_value, NULL, NULL, params_flush};

// ---- latency ----
static uint32_t latency_get(const clap_plugin_t *p) { return (uint32_t)self_of(p)->active_latency; }
static const clap_plugin_latency_t latency_ext = {latency_get};

// ---- state ----
static bool state_save(const clap_plugin_t *p, const clap_ostream_t *s) {
   test_t *t = self_of(p);
   double v[2];
   if (t->synth) { v[0] = t->level; v[1] = t->last_key; } else { v[0] = t->gain; v[1] = t->pending_latency; }
   return s->write(s, v, sizeof(v)) == (int64_t)sizeof(v);
}

static bool state_load(const clap_plugin_t *p, const clap_istream_t *s) {
   test_t *t = self_of(p);
   double v[2];
   int64_t got = 0;
   while (got < (int64_t)sizeof(v)) {
      int64_t r = s->read(s, (char *)v + got, sizeof(v) - got);
      if (r <= 0) return false;
      got += r;
   }
   if (t->synth) { t->level = v[0]; t->last_key = v[1]; }
   else { t->gain = v[0]; set_value(t, 9, v[1]); }
   return true;
}

static const clap_plugin_state_t state_ext = {state_save, state_load};

// ---- ports ----
static uint32_t audio_count(const clap_plugin_t *p, bool is_input) { return (is_input && self_of(p)->synth) ? 0 : 1; }

static bool audio_get(const clap_plugin_t *p, uint32_t i, bool is_input, clap_audio_port_info_t *info) {
   (void)p;
   if (i != 0) return false;
   memset(info, 0, sizeof(*info));
   info->id = is_input ? 0 : 1;
   strcpy(info->name, is_input ? "In" : "Out");
   info->flags = CLAP_AUDIO_PORT_IS_MAIN;
   info->channel_count = 2;
   info->port_type = CLAP_PORT_STEREO;
   info->in_place_pair = CLAP_INVALID_ID;
   return true;
}

static const clap_plugin_audio_ports_t audio_ext = {audio_count, audio_get};

static uint32_t note_count(const clap_plugin_t *p, bool is_input) { return (is_input && self_of(p)->synth) ? 1 : 0; }

static bool note_get(const clap_plugin_t *p, uint32_t i, bool is_input, clap_note_port_info_t *info) {
   (void)p; (void)is_input;
   if (i != 0) return false;
   memset(info, 0, sizeof(*info));
   info->supported_dialects = CLAP_NOTE_DIALECT_CLAP;
   info->preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
   strcpy(info->name, "Notes");
   return true;
}

static const clap_plugin_note_ports_t note_ext = {note_count, note_get};

// ---- the plugin ----
static bool p_init(const clap_plugin_t *p) {
   test_t *t = self_of(p);
   t->host_state = (const clap_host_state_t *)t->host->get_extension(t->host, CLAP_EXT_STATE);
   return true;
}

static void p_destroy(const clap_plugin_t *p) { free(self_of(p)); }

static bool p_activate(const clap_plugin_t *p, double rate, uint32_t min, uint32_t max) {
   (void)rate; (void)min; (void)max;
   test_t *t = self_of(p);
   t->active_latency = (int)t->pending_latency;
   memset(t->line, 0, sizeof(t->line));
   t->pos = 0;
   return true;
}

static void p_deactivate(const clap_plugin_t *p) { (void)p; }
static bool p_start(const clap_plugin_t *p) { (void)p; return true; }
static void p_stop(const clap_plugin_t *p) { (void)p; }
static void p_reset(const clap_plugin_t *p) { self_of(p)->held = 0; }

static clap_process_status p_process(const clap_plugin_t *p, const clap_process_t *pr) {
   test_t *t = self_of(p);
   uint32_t n = pr->in_events->size(pr->in_events);
   uint32_t next = 0;
   float *outl = pr->audio_outputs[0].data32[0], *outr = pr->audio_outputs[0].data32[1];
   for (uint32_t k = 0; k < pr->frames_count; k++) {
      while (next < n) {
         const clap_event_header_t *h = pr->in_events->get(pr->in_events, next);
         if (h->time > k) break;
         handle(t, h, pr->out_events);
         next++;
      }
      if (t->synth) {
         outl[k] = t->held > 0 ? (float)(t->level * t->volume) : 0.0f;
         outr[k] = pr->transport ? (float)(pr->transport->tempo / 1000.0) : 0.0f;
      } else {
         const int len = MAX_DELAY + 1;
         int d = t->active_latency;
         for (int ch = 0; ch < 2; ch++) {
            float x = pr->audio_inputs[0].data32[ch][k];
            t->line[ch][t->pos] = x;
            float y = t->line[ch][(t->pos + len - d) % len];
            pr->audio_outputs[0].data32[ch][k] = (float)(y * t->gain);
         }
         t->pos = (t->pos + 1) % len;
      }
   }
   while (next < n) handle(t, pr->in_events->get(pr->in_events, next++), pr->out_events);
   return CLAP_PROCESS_CONTINUE;
}

static const void *p_ext(const clap_plugin_t *p, const char *id) {
   if (!strcmp(id, CLAP_EXT_PARAMS)) return &params_ext;
   if (!strcmp(id, CLAP_EXT_LATENCY)) return self_of(p)->synth ? NULL : &latency_ext;
   if (!strcmp(id, CLAP_EXT_STATE)) return &state_ext;
   if (!strcmp(id, CLAP_EXT_AUDIO_PORTS)) return &audio_ext;
   if (!strcmp(id, CLAP_EXT_NOTE_PORTS)) return &note_ext;
   return NULL;
}

static void p_main(const clap_plugin_t *p) { (void)p; }

static const clap_plugin_t *create(const clap_host_t *host, int synth) {
   test_t *t = calloc(1, sizeof(test_t));
   t->host = host;
   t->synth = synth;
   t->gain = 1.0;
   t->level = 0.5;
   t->volume = 1.0;
   t->plugin.desc = synth ? &synth_desc : &gain_desc;
   t->plugin.plugin_data = t;
   t->plugin.init = p_init;
   t->plugin.destroy = p_destroy;
   t->plugin.activate = p_activate;
   t->plugin.deactivate = p_deactivate;
   t->plugin.start_processing = p_start;
   t->plugin.stop_processing = p_stop;
   t->plugin.reset = p_reset;
   t->plugin.process = p_process;
   t->plugin.get_extension = p_ext;
   t->plugin.on_main_thread = p_main;
   return &t->plugin;
}

// ---- factory and entry ----
static uint32_t f_count(const clap_plugin_factory_t *f) { (void)f; return 2; }

static const clap_plugin_descriptor_t *f_desc(const clap_plugin_factory_t *f, uint32_t i) {
   (void)f;
   return i == 0 ? &gain_desc : i == 1 ? &synth_desc : NULL;
}

static const clap_plugin_t *f_create(const clap_plugin_factory_t *f, const clap_host_t *host, const char *id) {
   (void)f;
   if (!clap_version_is_compatible(host->clap_version)) return NULL;
   if (!strcmp(id, gain_desc.id)) return create(host, 0);
   if (!strcmp(id, synth_desc.id)) return create(host, 1);
   return NULL;
}

static const clap_plugin_factory_t factory = {f_count, f_desc, f_create};

static int inits = 0;
static bool e_init(const char *path) { (void)path; inits++; return true; }
static void e_deinit(void) { inits--; }
static const void *e_factory(const char *id) { return !strcmp(id, CLAP_PLUGIN_FACTORY_ID) ? &factory : NULL; }

CLAP_EXPORT const clap_plugin_entry_t clap_entry = {CLAP_VERSION_INIT, e_init, e_deinit, e_factory};
