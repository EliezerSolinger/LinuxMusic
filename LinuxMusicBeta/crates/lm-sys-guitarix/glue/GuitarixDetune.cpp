/****************************************************************************
*
* GuitarixDetune.cpp - Guitarix's Detune (its smbPitchShift), as a unit here
*
* Its code is Guitarix's (src/gx_head/engine/gx_internal_plugins.cpp), from:
*
* NAME: smbPitchShift.cpp
* VERSION: 1.2
* HOME URL: http://www.dspdimension.com
* KNOWN BUGS: none
*
* COPYRIGHT 1999-2009 Stephan M. Bernsee <smb [AT] dspdimension [DOT] com>
*
* Modified for guitarix by Hermann Meyer 2014
*
*                         The Wide Open License (WOL)
*
* Permission to use, copy, modify, distribute and sell this software and its
* documentation for any purpose is hereby granted without fee, provided that
* the above copyright notice and this license appear in all source copies.
* THIS SOFTWARE IS PROVIDED "AS IS" WITHOUT EXPRESS OR IMPLIED WARRANTY OF
* ANY KIND. See http://www.dspguru.com/wol.htm for more information.
*
* Here (LinuxMusic, GPL-2.0 or later as the rest of it): what depended on Guitarix's engine is the unit's own (the
* block it processes, the rate; its latency setting's reallocation done in its update, on the main thread, with no
* period rendered), and its buffers are freed with delete[] (they were new[]'s, freed with delete).
*
*****************************************************************************/

#include <cmath>
#include <cstring>
#include <mutex>

#include <fftw3.h>

#include "GuitarixUnits.h"

#include "gx_faust_support.h"
#include "gx_plugin.h"
#include "gx_resampler.h"

namespace lmms::guitarix
{

namespace
{

constexpr auto Pi = 3.14159265358979323846;
constexpr int MAX_FRAME_LENGTH = 8096;

//! FFTW's planner is not thread safe, and LinuxMusicBeta builds effects on several threads
std::mutex& planMutex()
{
	static auto mutex = std::mutex{};
	return mutex;
}

class smbPitchShift : public PluginDef
{
private:
	gx_resample::SimpleResampler resamp;
	bool mem_allocated = false;
	volatile bool ready = false;
	float gInFIFO[MAX_FRAME_LENGTH];
	float gOutFIFO[MAX_FRAME_LENGTH];
	float* fpb = nullptr;
	float* expect = nullptr;
	float* hanning = nullptr;
	float* hanningd = nullptr;
	float* resampin = nullptr;
	float* resampin2 = nullptr;
	float* resampout = nullptr;
	float* indata2 = nullptr;
	float gLastPhase[MAX_FRAME_LENGTH / 2 + 1];
	float gSumPhase[MAX_FRAME_LENGTH / 2 + 1];
	float gOutputAccum[2 * MAX_FRAME_LENGTH];
	float gAnaFreq[MAX_FRAME_LENGTH];
	float gAnaMagn[MAX_FRAME_LENGTH];
	float gSynFreq[MAX_FRAME_LENGTH];
	float gSynMagn[MAX_FRAME_LENGTH];
	float semitones = 0;
	float a = 1, b = 1, c = 1, d = 1, l = 0;
	float wet = 50;
	float dry = 50;
	float mpi = 0, mpi1 = 0;
	float tone = 0;
	int octave = 0, osamp = 8, numSampsToResamp = 0, numSampsToProcess = 0, fftFrameSize = 0, sampleRate = 0;
	int latency = 0;
	int allocatedLatency = -1; //!< (the latency setting the buffers are for)
	int ai = 0;
	int aio = 0;
	int ii = 0;
	long gRover = 0;
	double magn = 0, phase = 0, tmp = 0, real = 0, imag = 0;
	double freqPerBin = 0, freqPerBin1 = 0, freqPerBin2 = 0, expct = 0;
	double fftFrameSize3 = 0;
	double fftFrameSize4 = 0;
	double osamp1 = 0, osamp2 = 0;
	long i = 0, k = 0, qpd = 0, index = 0, inFifoLatency = 0, stepSize = 0, fftFrameSize2 = 0;

	fftwf_complex fftw_in[MAX_FRAME_LENGTH], fftw_out[MAX_FRAME_LENGTH];
	fftwf_plan ftPlanForward = nullptr, ftPlanInverse = nullptr;

public:
	smbPitchShift() :
		PluginDef()
	{
		std::memset(gInFIFO, 0, MAX_FRAME_LENGTH * sizeof(float));
		std::memset(gOutFIFO, 0, MAX_FRAME_LENGTH * sizeof(float));
		std::memset(gLastPhase, 0, (MAX_FRAME_LENGTH / 2 + 1) * sizeof(float));
		std::memset(gSumPhase, 0, (MAX_FRAME_LENGTH / 2 + 1) * sizeof(float));
		std::memset(gOutputAccum, 0, 2 * MAX_FRAME_LENGTH * sizeof(float));
		std::memset(gAnaFreq, 0, MAX_FRAME_LENGTH * sizeof(float));
		std::memset(gAnaMagn, 0, MAX_FRAME_LENGTH * sizeof(float));
		version = PLUGINDEF_VERSION;
		id = "smbPitchShift";
		name = N_("Detune");
		groups = 0;
		description = N_("detune and pitch shift up"); // description (tooltip)
		category = N_("Misc"); // category
		mono_audio = compute_static;
		stereo_audio = 0;
		set_samplerate = init;
		activate_plugin = activate_static;
		register_params = registerparam;
		delete_instance = del_instance;
		load_ui = load_ui_f_static;
	}

	~smbPitchShift()
	{
		if (mem_allocated) { mem_free(); }
	}

	//! Its latency setting changed: its buffers anew (Guitarix's change_latency) [main thread, no period rendered]
	static void update(PluginDef* p)
	{
		auto& self = *static_cast<smbPitchShift*>(p);
		if (self.mem_allocated && self.latency != self.allocatedLatency)
		{
			self.ready = false;
			self.mem_free();
			self.mem_alloc();
		}
	}

private:
	bool setParameters(int sampleRate_)
	{
		numSampsToProcess = static_cast<int>(blockFrames());
		fftFrameSize = numSampsToProcess / 4;
		sampleRate = sampleRate_;
		osamp = 8;
		osamp1 = 1. / osamp;
		osamp2 = 2. * Pi * osamp1;
		mpi = (1. / (2. * Pi)) * osamp;
		mpi1 = 1. / Pi;
		resamp.setup(sampleRate, 4);
		gRover = inFifoLatency;
		return true;
	}

	static void init(unsigned int samplingFreq, PluginDef* plugin)
	{
		auto& self = *static_cast<smbPitchShift*>(plugin);
		// (a new rate or block: its buffers are made again for it when activated)
		if (self.mem_allocated) { self.mem_free(); }
		self.setParameters(static_cast<int>(samplingFreq));
	}

	void clear_state()
	{
		stepSize = fftFrameSize / osamp;
		freqPerBin = (double)(sampleRate / 4) / (double)fftFrameSize;
		freqPerBin1 = (1 / freqPerBin) * osamp2;
		freqPerBin2 = freqPerBin * mpi;
		expct = 2. * Pi * (double)stepSize / (double)fftFrameSize;
		inFifoLatency = fftFrameSize - stepSize;
		fftFrameSize3 = 2. * (1. / ((double)(fftFrameSize2)*osamp));
		fftFrameSize4 = 1. / (double)fftFrameSize;
		ai = 0;
		aio = 0;
		ii = 0;
		tone = 0;
		std::memset(gInFIFO, 0, MAX_FRAME_LENGTH * sizeof(float));
		std::memset(gOutFIFO, 0, MAX_FRAME_LENGTH * sizeof(float));
		std::memset(gLastPhase, 0, (MAX_FRAME_LENGTH / 2 + 1) * sizeof(float));
		std::memset(gSumPhase, 0, (MAX_FRAME_LENGTH / 2 + 1) * sizeof(float));
		std::memset(gOutputAccum, 0, 2 * MAX_FRAME_LENGTH * sizeof(float));
		std::memset(gAnaFreq, 0, MAX_FRAME_LENGTH * sizeof(float));
		std::memset(gAnaMagn, 0, MAX_FRAME_LENGTH * sizeof(float));
		for (k = 0; k < fftFrameSize2; k++) { fpb[k] = (double)k * freqPerBin; }
		for (k = 0; k < fftFrameSize2; k++) { expect[k] = (double)k * expct; }
		for (k = 0; k < fftFrameSize; k++) { hanning[k] = 0.5 * (1 - cos(2. * Pi * (double)k / ((double)fftFrameSize))); }
		for (k = 0; k < fftFrameSize; k++)
		{
			hanningd[k] = 0.5 * (1 - cos(2. * Pi * (double)k * fftFrameSize4)) * fftFrameSize3;
		}
		for (k = 0; k < fftFrameSize; k++) { resampin[k] = 0.0; }
		for (k = 0; k < fftFrameSize; k++) { resampin2[k] = 0.0; }
		for (k = 0; k < fftFrameSize * 4; k++) { resampout[k] = 0.0; }
		for (k = 0; k < fftFrameSize * 4; k++) { indata2[k] = 0.0; }
		gRover = inFifoLatency;
		mem_allocated = true;
		ready = true;
	}

	void mem_alloc()
	{
		numSampsToProcess = static_cast<int>(blockFrames());
		numSampsToResamp = numSampsToProcess / 4;

		switch (latency)
		{
		case (0):
			if (numSampsToProcess <= 2048) { fftFrameSize = 512; }
			else { fftFrameSize = numSampsToProcess * 0.25; }
			break;
		case (1): fftFrameSize = numSampsToProcess; break;
		case (2): fftFrameSize = numSampsToProcess * 0.25; break;
		default:
			if (numSampsToProcess <= 2048) { fftFrameSize = 512; }
			else { fftFrameSize = numSampsToProcess * 0.25; }
			break;
		}
		fftFrameSize2 = fftFrameSize / 2;

		// create FFTW plan
		const auto lock = std::scoped_lock{planMutex()};
		ftPlanForward = fftwf_plan_dft_1d(fftFrameSize, fftw_in, fftw_out, FFTW_FORWARD, FFTW_ESTIMATE);
		ftPlanInverse = fftwf_plan_dft_1d(fftFrameSize, fftw_in, fftw_out, FFTW_BACKWARD, FFTW_ESTIMATE);
		// alloc buffers
		fpb = new float[fftFrameSize2];
		expect = new float[fftFrameSize2];
		hanning = new float[fftFrameSize];
		hanningd = new float[fftFrameSize];
		resampin = new float[fftFrameSize];
		resampin2 = new float[fftFrameSize];
		resampout = new float[fftFrameSize * 4];
		indata2 = new float[fftFrameSize * 4];
		allocatedLatency = latency;
		clear_state();
	}

	void mem_free()
	{
		ready = false;
		mem_allocated = false;
		delete[] fpb; fpb = nullptr;
		delete[] expect; expect = nullptr;
		delete[] hanning; hanning = nullptr;
		delete[] hanningd; hanningd = nullptr;
		delete[] resampin; resampin = nullptr;
		delete[] resampin2; resampin2 = nullptr;
		delete[] resampout; resampout = nullptr;
		delete[] indata2; indata2 = nullptr;
		const auto lock = std::scoped_lock{planMutex()};
		if (ftPlanForward) { fftwf_destroy_plan(ftPlanForward); ftPlanForward = nullptr; }
		if (ftPlanInverse) { fftwf_destroy_plan(ftPlanInverse); ftPlanInverse = nullptr; }
	}

	int activate(bool start)
	{
		if (start)
		{
			if (!mem_allocated) { mem_alloc(); }
		}
		else if (mem_allocated) { mem_free(); }
		return 0;
	}

	static void compute_static(int count, float* input0, float* output0, PluginDef* p)
	{
		static_cast<smbPitchShift*>(p)->PitchShift(count, input0, output0);
	}

	void PitchShift(int count, float* indata, float* outdata)
	{
		if (!ready || count != numSampsToProcess)
		{
			if (indata != outdata) { std::memcpy(outdata, indata, count * sizeof(float)); }
			return;
		}

		resamp.down(numSampsToResamp, indata, resampin);
		double fSlow0 = (0.01 * wet);
		double fSlow1 = (0.01 * dry);

		// collect data for latency compensation
		for (i = 0; i < count; i++)
		{
			indata2[ii] = indata[i];
			ii++;
		}
		// collect data for fft
		for (i = 0; i < numSampsToResamp; i++)
		{
			resampin2[ai] = resampin[i];
			ai++;
		}
		// now we have enough data
		if (ai >= fftFrameSize)
		{
			ai = 0;
			ii = 0;
			switch (octave)
			{
			case (0): tone = 0; break;
			case (1): tone = 12; break;
			case (2): tone = -12; break;
			default: tone = 0; break;
			}
			float pitchShift = pow(2., (semitones + tone) * 0.0833333333);
			/* main processing loop */
			for (i = 0; i < fftFrameSize; i++)
			{
				/* As long as we have not yet collected enough data just read in */
				float fTemp = resampin2[i];
				gInFIFO[gRover] = fTemp;
				resampin2[i] = gOutFIFO[gRover - inFifoLatency];
				gRover++;

				/* now we have enough data for processing */
				if (gRover >= fftFrameSize)
				{
					gRover = inFifoLatency;

					/* do windowing and re,im interleave */
					for (k = 0; k < fftFrameSize; k++)
					{
						fftw_in[k][0] = gInFIFO[k] * hanning[k];
						fftw_in[k][1] = 0.0;
					}

					/* ***************** ANALYSIS ******************* */
					/* do transform */
					fftwf_execute(ftPlanForward);

					/* this is the analysis step */
					for (k = 0; k < fftFrameSize2; k++)
					{
						/* de-interlace FFT buffer */
						real = fftw_out[k][0];
						imag = fftw_out[k][1];

						/* compute magnitude and phase */
						magn = 2. * sqrt(real * real + imag * imag);
						phase = atan2(imag, real);

						/* compute phase difference */
						tmp = phase - gLastPhase[k];
						gLastPhase[k] = phase;

						/* subtract expected phase difference */
						tmp -= expect[k];

						/* map delta phase into +/- Pi interval */
						qpd = tmp * mpi1;
						if (qpd >= 0) { qpd += qpd & 1; }
						else { qpd -= qpd & 1; }
						tmp -= Pi * (double)qpd;

						/* get deviation from bin frequency from the +/- Pi interval */
						/* compute the k-th partials' true frequency */
						tmp = fpb[k] + tmp * freqPerBin2;

						/* store magnitude and true frequency in analysis arrays */
						gAnaMagn[k] = magn;
						gAnaFreq[k] = tmp;
					}

					/* ***************** PROCESSING ******************* */
					/* this does the actual pitch shifting */
					std::memset(gSynMagn, 0, fftFrameSize * sizeof(float));
					std::memset(gSynFreq, 0, fftFrameSize * sizeof(float));
					for (k = 1; k < fftFrameSize2 - 2; k++)
					{
						index = k * pitchShift;
						if (index < fftFrameSize2)
						{
							if (index < fftFrameSize2 * 0.20) { gSynMagn[index] += gAnaMagn[k] * a; }
							else if (index < fftFrameSize2 * 0.45) { gSynMagn[index] += gAnaMagn[k] * b; }
							else if (index < fftFrameSize2 * 0.667) { gSynMagn[index] += gAnaMagn[k] * c; }
							else { gSynMagn[index] += gAnaMagn[k] * d; }
							gSynFreq[index] = gAnaFreq[k] * pitchShift;
						}
					}

					/* ***************** SYNTHESIS ******************* */
					/* this is the synthesis step */
					for (k = 0; k < fftFrameSize2; k++)
					{
						/* get magnitude and true frequency from synthesis arrays */
						magn = gSynMagn[k];

						/* subtract bin mid frequency */
						/* get bin deviation from freq deviation */
						/* take osamp into account */
						/* add the overlap phase advance back in */
						tmp = ((gSynFreq[k] - fpb[k]) * freqPerBin1) + expect[k];

						/* accumulate delta phase to get bin phase */
						gSumPhase[k] += tmp;
						phase = gSumPhase[k];
						if (magn == 0.0) { continue; }

						/* get real and imag part and re-interleave */
						fftw_in[k][0] = magn * cos(phase);
						fftw_in[k][1] = magn * sin(phase);
					}

					/* do inverse transform */
					fftwf_execute(ftPlanInverse);
					/* do windowing and add to output accumulator */
					for (k = 0; k < fftFrameSize; k++) { gOutputAccum[k] += hanningd[k] * fftw_out[k][0]; }
					for (k = 0; k < stepSize; k++) { gOutFIFO[k] = gOutputAccum[k]; }

					/* shift accumulator */
					std::memmove(gOutputAccum, gOutputAccum + stepSize, fftFrameSize * sizeof(float));

					/* move input FIFO */
					for (k = 0; k < inFifoLatency; k++) { gInFIFO[k] = gInFIFO[k + stepSize]; }
				}
			}
			resamp.up(fftFrameSize, resampin2, resampout);
			aio = 0;
		}
		if (l)
		{
			for (i = 0; i < count; i++)
			{
				outdata[i] = ((fSlow0 * resampout[aio]) + (fSlow1 * indata2[aio]));
				aio++;
			}
		}
		else
		{
			for (i = 0; i < count; i++)
			{
				outdata[i] = ((fSlow0 * resampout[aio]) + (fSlow1 * indata[i]));
				aio++;
			}
		}
	}

	int register_par(const ParamReg& reg)
	{
		reg.registerFloatVar("smbPitchShift.semitone", N_("Detune"), "S", "", &semitones, 0.0, -12., 12., 0.1, 0);
		static const value_pair octave_values[] = {{"unison"}, {"octave up"}, {"octave down"}, {0}};
		reg.registerIntVar("smbPitchShift.octave", N_("add harmonics"), "B", N_("add harmonics"), &octave, 0, 0, 0,
			octave_values);
		static const value_pair latency_values[] = {{"latency "}, {"compensate"}, {0}};
		reg.registerFloatVar("smbPitchShift.l", N_("compensate latency"), "S", N_("compensate latency"), &l, 0.0f,
			0.0f, 1.0f, 1.0f, latency_values);
		static const value_pair latency_set[] = {{"high quality"}, {"low quality"}, {"realtime"}, {0}};
		reg.registerIntVar("smbPitchShift.latency", N_("latency settings"), "B", N_("latency settings"), &latency,
			0, 0, 0, latency_set);
		reg.registerFloatVar("smbPitchShift.wet", N_("Wet"), "S", N_("Wet amount"), &wet, 50.0, 0.0, 100.0, 1, 0);
		reg.registerFloatVar("smbPitchShift.dry", N_("Dry"), "S", N_("Dry amount"), &dry, 50.0, 0.0, 100.0, 1, 0);
		reg.registerFloatVar("smbPitchShift.a", N_("low"), "S", N_("Sub"), &a, 1.0, 0.0, 2.0, 0.01, 0);
		reg.registerFloatVar("smbPitchShift.b", N_("middle low"), "S", N_("Low"), &b, 1.0, 0.0, 2.0, 0.01, 0);
		reg.registerFloatVar("smbPitchShift.c", N_("middle treble"), "S", N_("Mid"), &c, 1.0, 0.0, 2.0, 0.01, 0);
		reg.registerFloatVar("smbPitchShift.d", N_("treble"), "S", N_("Hi"), &d, 1.0, 0.0, 2.0, 0.01, 0);
		return 0;
	}

	static int registerparam(const ParamReg& reg)
	{
		return static_cast<smbPitchShift*>(reg.plugin)->register_par(reg);
	}

	static int load_ui_f_static(const UiBuilder& b, int form)
	{
		if (form & UI_FORM_GLADE)
		{
			b.load_glade_file("gx_detune_ui.glade");
			return 0;
		}
		return -1;
	}

	static int activate_static(bool start, PluginDef* p)
	{
		return static_cast<smbPitchShift*>(p)->activate(start);
	}

	static void del_instance(PluginDef* p)
	{
		delete static_cast<smbPitchShift*>(p);
	}
};

} // namespace


auto makeDetune() -> PluginDef*
{
	return new smbPitchShift;
}

void updateDetune(PluginDef* plugin)
{
	smbPitchShift::update(plugin);
}

} // namespace lmms::guitarix
