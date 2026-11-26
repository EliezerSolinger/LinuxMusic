/* Two LADSPA test plugins, with the ABI declared here (it is tiny and frozen since 2000):
 *   lm_gain:        mono, control "Gain" (0..4, default 1), toggled "Invert", integer "Steps" (0..8, default 2)
 *   lm_stereo_gain: stereo, control "Gain" (0..4, default 1), output control "latency" (constant 5)
 */
#include <stdlib.h>

typedef float Data;
typedef struct {
	int hint;
	Data lower, upper;
} Hint;
typedef struct Descriptor {
	unsigned long id;
	const char *label;
	int properties;
	const char *name, *maker, *copyright;
	unsigned long port_count;
	const int *port_descriptors;
	const char *const *port_names;
	const Hint *port_hints;
	void *impl;
	void *(*instantiate)(const struct Descriptor *, unsigned long rate);
	void (*connect_port)(void *, unsigned long, Data *);
	void (*activate)(void *);
	void (*run)(void *, unsigned long);
	void (*run_adding)(void *, unsigned long);
	void (*set_run_adding_gain)(void *, Data);
	void (*deactivate)(void *);
	void (*cleanup)(void *);
} Descriptor;

enum { IN = 1, OUT = 2, CTRL = 4, AUDIO = 8 };
enum { BELOW = 1, ABOVE = 2, TOGGLED = 4, INTEGER = 0x20, D_1 = 0x240, D_0 = 0x200 };

typedef struct {
	Data *port[8];
} Inst;

static void *instantiate(const Descriptor *d, unsigned long rate) {
	(void)d;
	(void)rate;
	return calloc(1, sizeof(Inst));
}
static void connect_port(void *h, unsigned long p, Data *x) { ((Inst *)h)->port[p] = x; }
static void cleanup(void *h) { free(h); }

/* lm_gain: ports 0 in, 1 out, 2 gain, 3 invert, 4 steps */
static void run_mono(void *h, unsigned long n) {
	Inst *s = h;
	Data g = *s->port[2] * (*s->port[3] >= 0.5f ? -1.0f : 1.0f);
	for (unsigned long k = 0; k < n; k++) s->port[1][k] = s->port[0][k] * g;
}
static const int mono_desc[] = {IN | AUDIO, OUT | AUDIO, IN | CTRL, IN | CTRL, IN | CTRL};
static const char *const mono_names[] = {"Input", "Output", "Gain", "Invert", "Steps"};
static const Hint mono_hints[] = {{0, 0, 0}, {0, 0, 0}, {BELOW | ABOVE | D_1, 0.0f, 4.0f}, {TOGGLED | D_0, 0, 1},
				  {BELOW | ABOVE | INTEGER | 0x80, 0.0f, 8.0f}};

/* lm_stereo_gain: ports 0 in L, 1 in R, 2 out L, 3 out R, 4 gain, 5 latency (out) */
static void run_stereo(void *h, unsigned long n) {
	Inst *s = h;
	Data g = *s->port[4];
	*s->port[5] = 5.0f;
	for (unsigned long k = 0; k < n; k++) {
		s->port[2][k] = s->port[0][k] * g;
		s->port[3][k] = s->port[1][k] * g;
	}
}
static const int st_desc[] = {IN | AUDIO, IN | AUDIO, OUT | AUDIO, OUT | AUDIO, IN | CTRL, OUT | CTRL};
static const char *const st_names[] = {"In L", "In R", "Out L", "Out R", "Gain", "latency"};
static const Hint st_hints[] = {{0, 0, 0}, {0, 0, 0}, {0, 0, 0}, {0, 0, 0}, {BELOW | ABOVE | D_1, 0.0f, 4.0f}, {0, 0, 0}};

static const Descriptor mono = {1, "lm_gain", 4, "Test gain (mono)", "LinuxMusic", "none", 5, mono_desc, mono_names,
				mono_hints, 0, instantiate, connect_port, 0, run_mono, 0, 0, 0, cleanup};
static const Descriptor stereo = {2, "lm_stereo_gain", 4, "Test gain (stereo)", "LinuxMusic", "none", 6, st_desc,
				  st_names, st_hints, 0, instantiate, connect_port, 0, run_stereo, 0, 0, 0, cleanup};

__attribute__((visibility("default"))) const Descriptor *ladspa_descriptor(unsigned long i) {
	return i == 0 ? &mono : i == 1 ? &stereo : 0;
}
