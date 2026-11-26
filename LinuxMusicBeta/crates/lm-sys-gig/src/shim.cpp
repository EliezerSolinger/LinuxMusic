// The C interface over libgig (shim.h). Everything libgig throws is caught here. After lmgig_load, the audio thread
// only reads: the samples are decoded into memory here, and lmgig_patch_select computes with structures libgig
// already built (no allocation, no lock, no I/O).

#include "shim.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <exception>
#include <map>
#include <memory>
#include <string>
#include <vector>

#include <gig.h>

namespace {

void set_error(char* error, size_t len, const char* what)
{
    if (error == nullptr || len == 0) { return; }
    std::snprintf(error, len, "%s", what);
}

int32_t kind_of(gig::dimension_t d)
{
    switch (d) {
        case gig::dimension_layer: return LMGIG_DIM_LAYER;
        case gig::dimension_velocity: return LMGIG_DIM_VELOCITY;
        case gig::dimension_releasetrigger: return LMGIG_DIM_RELEASE_TRIGGER;
        case gig::dimension_keyboard: return LMGIG_DIM_KEYBOARD;
        case gig::dimension_roundrobin:
        case gig::dimension_roundrobinkeyboard: return LMGIG_DIM_ROUND_ROBIN;
        case gig::dimension_sustainpedal: return LMGIG_DIM_SUSTAIN_PEDAL;
        case gig::dimension_random: return LMGIG_DIM_RANDOM;
        default: return LMGIG_DIM_OTHER;
    }
}

struct SampleData {
    uint32_t rate = 0;
    uint32_t channels = 1;
    uint64_t frames = 0;
    std::vector<float> data;
};

struct RegionInfo {
    gig::Region* region = nullptr;
    int32_t key_low = 0;
    int32_t key_high = 0;
    int32_t first = 0;
    int32_t count = 0;
};

} // namespace

struct lmgig_patch {
    // (the order matters: the gig file points into the RIFF file)
    std::unique_ptr<RIFF::File> riff;
    std::unique_ptr<gig::File> gig;
    gig::Instrument* instrument = nullptr;
    std::vector<SampleData> samples;
    std::vector<RegionInfo> regions;
    std::vector<lmgig_dimregion> dimregions;
    std::vector<gig::DimensionRegion*> dimregion_sources;
    int32_t key_low = 0;
    int32_t key_high = 127;
};

namespace {

struct Opened {
    std::unique_ptr<RIFF::File> riff;
    std::unique_ptr<gig::File> gig;
};

Opened open_file(const char* path)
{
    Opened o;
    o.riff = std::make_unique<RIFF::File>(std::string(path));
    o.gig = std::make_unique<gig::File>(o.riff.get());
    return o;
}

void decode_sample(gig::Sample* s, SampleData& out)
{
    out.rate = s->SamplesPerSecond;
    out.channels = s->Channels >= 2 ? 2 : 1;
    const auto stride = std::max<uint32_t>(s->Channels, 1);
    const auto frame_size = static_cast<uint64_t>(s->FrameSize);
    if (frame_size == 0 || s->SamplesTotal == 0) { return; }
    gig::buffer_t cache = s->LoadSampleData();
    const auto total = std::min<uint64_t>(s->SamplesTotal, cache.Size / frame_size);
    out.frames = total;
    out.data.assign(total * out.channels, 0.0f);
    const auto* bytes = static_cast<const uint8_t*>(cache.pStart);
    // Half of full scale: the legacy player divided by 0x10000 (16 bit) and by 2^32 after a shift of 8 (24 bit)
    for (uint64_t i = 0; i < total; ++i) {
        for (uint32_t c = 0; c < out.channels; ++c) {
            float v = 0.0f;
            if (s->BitDepth == 24) {
                const uint8_t* p = bytes + (i * stride + c) * 3;
                const int32_t raw = static_cast<int32_t>((uint32_t(p[0]) << 8) | (uint32_t(p[1]) << 16) | (uint32_t(p[2]) << 24));
                v = static_cast<float>(raw) / 4294967296.0f;
            } else {
                const uint8_t* p = bytes + (i * stride + c) * 2;
                const auto raw = static_cast<int16_t>(uint16_t(p[0]) | (uint16_t(p[1]) << 8));
                v = static_cast<float>(raw) / 65536.0f;
            }
            out.data[i * out.channels + c] = v;
        }
    }
    s->ReleaseSampleData();
}

} // namespace

extern "C" {

int32_t lmgig_instrument_count(const char* path, char* error, size_t error_len)
{
    try {
        auto o = open_file(path);
        int32_t n = 0;
        while (o.gig->GetInstrument(static_cast<size_t>(n)) != nullptr) { ++n; }
        return n;
    } catch (const RIFF::Exception& e) {
        set_error(error, error_len, e.Message.c_str());
    } catch (const std::exception& e) {
        set_error(error, error_len, e.what());
    } catch (...) {
        set_error(error, error_len, "libgig failed");
    }
    return -1;
}

int32_t lmgig_instrument(const char* path, int32_t index, int32_t* bank, int32_t* program, char* name, size_t name_len,
                         char* error, size_t error_len)
{
    try {
        auto o = open_file(path);
        gig::Instrument* i = index >= 0 ? o.gig->GetInstrument(static_cast<size_t>(index)) : nullptr;
        if (i == nullptr) {
            set_error(error, error_len, "no such instrument");
            return -1;
        }
        *bank = static_cast<int32_t>(i->MIDIBank);
        *program = static_cast<int32_t>(i->MIDIProgram);
        const std::string n = i->pInfo != nullptr ? i->pInfo->Name : std::string();
        set_error(name, name_len, n.c_str());
        return 0;
    } catch (const RIFF::Exception& e) {
        set_error(error, error_len, e.Message.c_str());
    } catch (const std::exception& e) {
        set_error(error, error_len, e.what());
    } catch (...) {
        set_error(error, error_len, "libgig failed");
    }
    return -1;
}

lmgig_patch* lmgig_load(const char* path, int32_t bank, int32_t program, char* error, size_t error_len)
{
    try {
        auto patch = std::make_unique<lmgig_patch>();
        auto o = open_file(path);
        patch->riff = std::move(o.riff);
        patch->gig = std::move(o.gig);
        for (size_t i = 0;; ++i) {
            gig::Instrument* candidate = patch->gig->GetInstrument(i);
            if (candidate == nullptr) { break; }
            if (static_cast<int32_t>(candidate->MIDIBank) == bank && static_cast<int32_t>(candidate->MIDIProgram) == program) {
                patch->instrument = candidate;
                break;
            }
        }
        if (patch->instrument == nullptr) {
            set_error(error, error_len, "the file has no instrument at this bank and program");
            return nullptr;
        }
        patch->key_low = patch->instrument->DimensionKeyRange.low;
        patch->key_high = patch->instrument->DimensionKeyRange.high;
        std::map<gig::Sample*, int32_t> sample_index;
        for (size_t r = 0;; ++r) {
            gig::Region* region = patch->instrument->GetRegionAt(r);
            if (region == nullptr) { break; }
            RegionInfo info;
            info.region = region;
            info.key_low = region->KeyRange.low;
            info.key_high = region->KeyRange.high;
            info.first = static_cast<int32_t>(patch->dimregions.size());
            // The array of dimension regions, with holes where a combination is not in use
            uint32_t total_bits = 0;
            for (unsigned i = 0; i < region->Dimensions; ++i) { total_bits += region->pDimensionDefinitions[i].bits; }
            const uint32_t slots = total_bits >= 8 ? 256u : (1u << total_bits);
            for (uint32_t d = 0; d < slots; ++d) {
                gig::DimensionRegion* dr = region->pDimensionRegions[d];
                lmgig_dimregion out;
                std::memset(&out, 0, sizeof out);
                out.sample = -1;
                if (dr != nullptr) {
                    if (dr->pSample != nullptr && dr->pSample->SamplesTotal != 0) {
                        auto found = sample_index.find(dr->pSample);
                        if (found == sample_index.end()) {
                            SampleData data;
                            decode_sample(dr->pSample, data);
                            found = sample_index.emplace(dr->pSample, static_cast<int32_t>(patch->samples.size())).first;
                            patch->samples.push_back(std::move(data));
                        }
                        out.sample = found->second;
                    }
                    out.pitch_track = dr->PitchTrack ? 1 : 0;
                    out.unity_note = dr->UnityNote;
                    out.fine_tune = dr->FineTune;
                    out.sample_attenuation = static_cast<float>(dr->SampleAttenuation);
                    out.release_trigger_decay = dr->ReleaseTriggerDecay;
                    out.eg1_preattack = dr->EG1PreAttack;
                    out.eg1_attack = static_cast<float>(dr->EG1Attack);
                    out.eg1_decay1 = static_cast<float>(dr->EG1Decay1);
                    out.eg1_decay2 = static_cast<float>(dr->EG1Decay2);
                    out.eg1_infinite_sustain = dr->EG1InfiniteSustain ? 1 : 0;
                    out.eg1_sustain = dr->EG1Sustain;
                    out.eg1_release = static_cast<float>(dr->EG1Release);
                    if (dr->pSampleLoops != nullptr && dr->SampleLoops > 0) {
                        out.loop_count = static_cast<int32_t>(dr->SampleLoops);
                        out.loop_type = static_cast<int32_t>(dr->pSampleLoops[0].LoopType);
                        out.loop_start = dr->pSampleLoops[0].LoopStart;
                        out.loop_length = dr->pSampleLoops[0].LoopLength;
                    }
                    for (int v = 0; v < 128; ++v) {
                        out.velocity_attenuation[v] = static_cast<float>(dr->GetVelocityAttenuation(static_cast<uint8_t>(v)));
                    }
                }
                patch->dimregions.push_back(out);
                patch->dimregion_sources.push_back(dr);
            }
            info.count = static_cast<int32_t>(patch->dimregions.size()) - info.first;
            patch->regions.push_back(info);
        }
        return patch.release();
    } catch (const RIFF::Exception& e) {
        set_error(error, error_len, e.Message.c_str());
    } catch (const std::exception& e) {
        set_error(error, error_len, e.what());
    } catch (...) {
        set_error(error, error_len, "libgig failed");
    }
    return nullptr;
}

void lmgig_patch_close(lmgig_patch* patch)
{
    try {
        delete patch;
    } catch (...) {
    }
}

int32_t lmgig_patch_key_dimension(const lmgig_patch* patch, int32_t* low, int32_t* high)
{
    if (patch == nullptr) { return -1; }
    *low = patch->key_low;
    *high = patch->key_high;
    return 0;
}

int32_t lmgig_patch_sample_count(const lmgig_patch* patch)
{
    return patch == nullptr ? 0 : static_cast<int32_t>(patch->samples.size());
}

int32_t lmgig_patch_sample(const lmgig_patch* patch, int32_t index, uint32_t* rate, uint32_t* channels, uint64_t* frames)
{
    if (patch == nullptr || index < 0 || static_cast<size_t>(index) >= patch->samples.size()) { return -1; }
    const SampleData& s = patch->samples[static_cast<size_t>(index)];
    *rate = s.rate;
    *channels = s.channels;
    *frames = s.frames;
    return 0;
}

const float* lmgig_patch_sample_data(const lmgig_patch* patch, int32_t index)
{
    if (patch == nullptr || index < 0 || static_cast<size_t>(index) >= patch->samples.size()) { return nullptr; }
    return patch->samples[static_cast<size_t>(index)].data.data();
}

int32_t lmgig_patch_region_count(const lmgig_patch* patch)
{
    return patch == nullptr ? 0 : static_cast<int32_t>(patch->regions.size());
}

int32_t lmgig_patch_region(const lmgig_patch* patch, int32_t region, int32_t* key_low, int32_t* key_high,
                           int32_t* dimensions, int32_t* first_dimregion, int32_t* dimregion_count)
{
    if (patch == nullptr || region < 0 || static_cast<size_t>(region) >= patch->regions.size()) { return -1; }
    const RegionInfo& r = patch->regions[static_cast<size_t>(region)];
    *key_low = r.key_low;
    *key_high = r.key_high;
    *dimensions = static_cast<int32_t>(r.region->Dimensions);
    *first_dimregion = r.first;
    *dimregion_count = r.count;
    return 0;
}

int32_t lmgig_patch_dimension(const lmgig_patch* patch, int32_t region, int32_t dimension, int32_t* kind, int32_t* bits,
                              int32_t* zones)
{
    if (patch == nullptr || region < 0 || static_cast<size_t>(region) >= patch->regions.size()) { return -1; }
    const gig::Region* r = patch->regions[static_cast<size_t>(region)].region;
    if (dimension < 0 || static_cast<unsigned>(dimension) >= r->Dimensions) { return -1; }
    const gig::dimension_def_t& d = r->pDimensionDefinitions[dimension];
    *kind = kind_of(d.dimension);
    *bits = d.bits;
    *zones = d.zones;
    return 0;
}

int32_t lmgig_patch_dimregion_count(const lmgig_patch* patch)
{
    return patch == nullptr ? 0 : static_cast<int32_t>(patch->dimregions.size());
}

const lmgig_dimregion* lmgig_patch_dimregion(const lmgig_patch* patch, int32_t index)
{
    if (patch == nullptr || index < 0 || static_cast<size_t>(index) >= patch->dimregions.size()) { return nullptr; }
    return &patch->dimregions[static_cast<size_t>(index)];
}

int32_t lmgig_patch_select(const lmgig_patch* patch, int32_t region, const uint32_t values[8])
{
    if (patch == nullptr || region < 0 || static_cast<size_t>(region) >= patch->regions.size()) { return -1; }
    const RegionInfo& info = patch->regions[static_cast<size_t>(region)];
    // (libgig's own selection: the bit positions, the zones, the velocity splits; it reads the region and allocates
    // nothing. The pointer is found among the region's own slots, no map)
    uint32_t v[8];
    for (int i = 0; i < 8; ++i) { v[i] = values[i]; }
    gig::DimensionRegion* chosen = info.region->GetDimensionRegionByValue(v);
    if (chosen == nullptr) { return -1; }
    for (int32_t i = 0; i < info.count; ++i) {
        if (patch->dimregion_sources[static_cast<size_t>(info.first + i)] == chosen) { return info.first + i; }
    }
    return -1;
}

int32_t lmgig_write_test_file(const char* path, char* error, size_t error_len)
{
    try {
        constexpr uint32_t rate = 44000;
        constexpr uint32_t frames = 2000; // 20 periods of 440 Hz (100 frames each) at 44 kHz: a loop of whole periods
        gig::File file;
        file.pInfo->Name = "lm-sys-gig test";
        std::vector<gig::Sample*> samples;
        const double freqs[2] = {440.0, 880.0};
        for (int s = 0; s < 2; ++s) {
            gig::Sample* sample = file.AddSample();
            sample->pInfo->Name = s == 0 ? "sine" : "sine up";
            sample->Channels = 1;
            sample->BitDepth = 16;
            sample->FrameSize = 2;
            sample->SamplesPerSecond = rate;
            sample->MIDIUnityNote = 69;
            sample->Resize(frames);
            samples.push_back(sample);
        }
        for (int p = 0; p < 2; ++p) {
            gig::Instrument* instrument = file.AddInstrument();
            instrument->pInfo->Name = p == 0 ? "Sine" : "Sine up";
            instrument->MIDIBankCoarse = 0;
            instrument->MIDIBankFine = 0;
            instrument->MIDIProgram = static_cast<uint32_t>(p);
            gig::Region* region = instrument->AddRegion();
            region->SetKeyRange(0, 127);
            gig::DimensionRegion* dr = region->pDimensionRegions[0];
            dr->pSample = samples[static_cast<size_t>(p)];
            dr->UnityNote = 69;
            dr->FineTune = 0;
            dr->PitchTrack = true;
            dr->EG1PreAttack = 0;
            dr->EG1Attack = 0.0;
            dr->EG1Decay1 = 0.0;
            dr->EG1Decay2 = 0.0;
            dr->EG1InfiniteSustain = true;
            dr->EG1Sustain = 1000;
            dr->EG1Release = 0.05;
            DLS::sample_loop_t loop;
            loop.Size = sizeof(DLS::sample_loop_t);
            loop.LoopType = 0;
            loop.LoopStart = 0;
            loop.LoopLength = frames;
            dr->AddSampleLoop(&loop);
        }
        // The wave chunks come with the first save; the data goes in after it, then the file is saved again
        file.Save(std::string(path));
        for (int s = 0; s < 2; ++s) {
            std::vector<int16_t> pcm(frames);
            for (uint32_t i = 0; i < frames; ++i) {
                pcm[i] = static_cast<int16_t>(std::lround(16384.0 * std::sin(6.283185307179586 * freqs[s] * i / rate)));
            }
            samples[static_cast<size_t>(s)]->SetPos(0);
            samples[static_cast<size_t>(s)]->Write(pcm.data(), frames);
        }
        file.Save();
        return 0;
    } catch (const RIFF::Exception& e) {
        set_error(error, error_len, e.Message.c_str());
    } catch (const std::exception& e) {
        set_error(error, error_len, e.what());
    } catch (...) {
        set_error(error, error_len, "libgig failed");
    }
    return -1;
}

} // extern "C"
