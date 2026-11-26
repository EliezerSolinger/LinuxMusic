/* The C interface over libgig that lm-sys-gig binds (src/shim.cpp). Plain data and opaque handles: no C++ type, no
 * exception crosses it. Every function that can fail returns NULL or a negative number and writes a message into
 * the caller's buffer. */
#ifndef LM_GIG_SHIM_H
#define LM_GIG_SHIM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct lmgig_patch lmgig_patch;

/* The kinds of dimension, as this crate names them (libgig's own numbers are not part of the interface) */
enum {
    LMGIG_DIM_OTHER = 0,
    LMGIG_DIM_LAYER = 1,
    LMGIG_DIM_VELOCITY = 2,
    LMGIG_DIM_RELEASE_TRIGGER = 3,
    LMGIG_DIM_KEYBOARD = 4,
    LMGIG_DIM_ROUND_ROBIN = 5,
    LMGIG_DIM_SUSTAIN_PEDAL = 6,
    LMGIG_DIM_RANDOM = 7
};

/* One dimension region: what a note plays when it is chosen. Times in seconds, amplitudes linear */
typedef struct lmgig_dimregion {
    int32_t sample;               /* index in the patch's samples, or -1: this region has no sample */
    int32_t pitch_track;          /* the sample is pitched by the key */
    int32_t unity_note;           /* the key the sample sounds at (0 to 127) */
    int32_t fine_tune;            /* cents, as the file has it (the legacy used `fine_tune * 0.01` semitones) */
    float sample_attenuation;     /* the region's gain */
    int32_t release_trigger_decay;/* 0 to 8 */
    /* The volume envelope (EG1) */
    float eg1_preattack;          /* permille, 0 to 1000 */
    float eg1_attack;
    float eg1_decay1;
    float eg1_decay2;
    int32_t eg1_infinite_sustain;
    float eg1_sustain;            /* permille, 0 to 1000 */
    float eg1_release;
    /* The first loop (the legacy played at most one) */
    int32_t loop_count;
    int32_t loop_type;            /* 0 forward, 1 bidirectional, 2 backward */
    uint32_t loop_start;          /* frames */
    uint32_t loop_length;         /* frames */
    /* The velocity's attenuation, by MIDI velocity (libgig's GetVelocityAttenuation) */
    float velocity_attenuation[128];
} lmgig_dimregion;

/* The instruments of a file (bank, program, name) */
int32_t lmgig_instrument_count(const char* path, char* error, size_t error_len);
int32_t lmgig_instrument(const char* path, int32_t index, int32_t* bank, int32_t* program, char* name, size_t name_len,
                         char* error, size_t error_len);

/* The instrument of (bank, program) loaded whole, its samples decoded to float in memory; NULL with `error` set */
lmgig_patch* lmgig_load(const char* path, int32_t bank, int32_t program, char* error, size_t error_len);
void lmgig_patch_close(lmgig_patch* patch);

int32_t lmgig_patch_key_dimension(const lmgig_patch* patch, int32_t* low, int32_t* high);

int32_t lmgig_patch_sample_count(const lmgig_patch* patch);
/* Rate, channels (1 or 2) and frames of a sample; the data is `channels * frames` floats, interleaved, at 0.5 of full
 * scale as the legacy player read the file */
int32_t lmgig_patch_sample(const lmgig_patch* patch, int32_t index, uint32_t* rate, uint32_t* channels,
                           uint64_t* frames);
const float* lmgig_patch_sample_data(const lmgig_patch* patch, int32_t index);

int32_t lmgig_patch_region_count(const lmgig_patch* patch);
int32_t lmgig_patch_region(const lmgig_patch* patch, int32_t region, int32_t* key_low, int32_t* key_high,
                           int32_t* dimensions, int32_t* first_dimregion, int32_t* dimregion_count);
int32_t lmgig_patch_dimension(const lmgig_patch* patch, int32_t region, int32_t dimension, int32_t* kind,
                              int32_t* bits, int32_t* zones);

int32_t lmgig_patch_dimregion_count(const lmgig_patch* patch);
const lmgig_dimregion* lmgig_patch_dimregion(const lmgig_patch* patch, int32_t index);
/* The dimension region of `region` for these dimension values (libgig's GetDimensionRegionByValue), as an index
 * of the patch's dimension regions; -1 when there is none. No allocation, no lock, no I/O */
int32_t lmgig_patch_select(const lmgig_patch* patch, int32_t region, const uint32_t values[8]);

/* A small .gig written for the tests: bank 0 program 0 is a sine of 440 Hz at key 69 (looped, on a 44 kHz sample, the
 * whole keyboard), program 1 is the same sine an octave up. 0 on success */
int32_t lmgig_write_test_file(const char* path, char* error, size_t error_len);

#ifdef __cplusplus
}
#endif

#endif
