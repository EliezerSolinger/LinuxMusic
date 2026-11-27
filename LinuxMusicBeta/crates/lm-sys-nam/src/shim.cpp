// lm-sys-nam: a C interface to Neural Amp Modeler's core (vendor/nam), for the Rust binding in lib.rs.
//
// No C++ exception crosses this interface: every function that can throw catches everything and reports it as an
// error code and a message copied into the caller's buffer. The model is owned by one `LmNamModel`, deleted by
// lm_nam_free alone.
//
// This file is part of LinuxMusic - GPL-2.0-or-later.

#include <cstddef>
#include <cstring>
#include <exception>
#include <memory>
#include <string>

#include "NAM/dsp.h"
#include "NAM/get_dsp.h"
#include "json.hpp"

struct LmNamModel
{
	std::unique_ptr<nam::DSP> dsp;
};

namespace
{

void copyMessage(const char* message, char* error, std::size_t room)
{
	if (!error || room == 0) { return; }
	std::strncpy(error, message ? message : "", room - 1);
	error[room - 1] = '\0';
}

} // namespace

extern "C" {

// Parses @p len bytes of a .nam file (its JSON) and makes its model. NULL on failure, with the reason in @p error
LmNamModel* lm_nam_load(const char* json, std::size_t len, char* error, std::size_t room)
{
	try
	{
		// (a null pointer is an empty document: adding to it would be undefined, and the parse refuses an empty one)
		const auto config = json ? nlohmann::json::parse(json, json + len) : nlohmann::json::parse("");
		auto model = std::make_unique<LmNamModel>();
		model->dsp = nam::get_dsp(config);
		if (!model->dsp)
		{
			copyMessage("the model could not be made", error, room);
			return nullptr;
		}
		return model.release();
	}
	catch (const std::exception& e)
	{
		copyMessage(e.what(), error, room);
	}
	catch (...)
	{
		copyMessage("unknown error", error, room);
	}
	return nullptr;
}

void lm_nam_free(LmNamModel* model)
{
	delete model;
}

int lm_nam_input_channels(const LmNamModel* model) { return model->dsp->NumInputChannels(); }
int lm_nam_output_channels(const LmNamModel* model) { return model->dsp->NumOutputChannels(); }
double lm_nam_expected_rate(const LmNamModel* model) { return model->dsp->GetExpectedSampleRate(); }
int lm_nam_arbitrary_rate(const LmNamModel* model) { return model->dsp->SupportsArbitrarySampleRate() ? 1 : 0; }
int lm_nam_has_loudness(const LmNamModel* model) { return model->dsp->HasLoudness() ? 1 : 0; }

// The loudness, or 0 with @p ok 0 when the model does not know it
double lm_nam_loudness(const LmNamModel* model, int* ok)
{
	try
	{
		*ok = 1;
		return model->dsp->GetLoudness();
	}
	catch (...)
	{
		*ok = 0;
		return 0.0;
	}
}

// Sets the rate and the most frames a call processes, and prewarms (allocates). 0 on success
int lm_nam_reset(LmNamModel* model, double rate, int max_frames, char* error, std::size_t room)
{
	try
	{
		model->dsp->Reset(rate, max_frames);
		return 0;
	}
	catch (const std::exception& e)
	{
		copyMessage(e.what(), error, room);
	}
	catch (...)
	{
		copyMessage("unknown error", error, room);
	}
	return -1;
}

// @p frames (at most the reset's) of the mono model: @p input to @p output (they may be the same). 0 on success; on
// an exception (none is thrown by the models' process) the output is silence and -1 is returned
int lm_nam_process(LmNamModel* model, float* input, float* output, int frames) noexcept
{
	try
	{
		float* inputs[] = {input};
		float* outputs[] = {output};
		model->dsp->process(inputs, outputs, frames);
		return 0;
	}
	catch (...)
	{
		for (int i = 0; i < frames; ++i) { output[i] = 0.f; }
		return -1;
	}
}

} // extern "C"
