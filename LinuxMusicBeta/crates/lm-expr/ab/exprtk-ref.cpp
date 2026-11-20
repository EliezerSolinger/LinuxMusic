/*
 * exprtk-ref.cpp - evaluates lm-expr's A/B expressions with exprtk itself, as the legacy Xpressive set it up
 * (exprtk<float>, the legacy's -D flags and parser settings: no logic, assignment or control structures), and prints
 * every frame's value as the bits of an f32.
 *
 * Built by build-reference.sh from legado/plugins/Xpressive/exprtk/exprtk.hpp, with -ffp-contract=off (the new
 * program never contracts). The host functions are the legacy ExprSynth.cpp's (copied below), with two changes for
 * a deterministic reference: rand() draws from a fixed LCG, and every per-note constant of the legacy (key, bnote,
 * srate, v, tempo, seed) is a variable, as lm-expr binds them.
 *
 * Input (stdin): one expression per line. Output (stdout): per line, "ok" and FRAMES hex words, or "err" and the
 * byte position of exprtk's first error.
 *
 * This file is part of LinuxMusic - GPL-2.0-or-later.
 */

#include <array>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <iostream>
#include <numbers>
#include <string>

#include <exprtk.hpp>

using symbol_table_t = exprtk::symbol_table<float>;
using expression_t = exprtk::expression<float>;
using parser_t = exprtk::parser<float>;

constexpr int FRAMES = 48;
constexpr unsigned int RATE = 44100;
constexpr unsigned int HISTORY = 64;
constexpr int WAVE = 128;

// ---- from ExprSynth.h / lmms_math.h
inline float absFraction(float x) { return x - std::floor(x); }
inline float fraction(float x) { return x - static_cast<float>(static_cast<int>(x)); }
inline float positiveFraction(float x)
{
	if (std::isnan(x) || std::isinf(x)) return 0;
	return absFraction(x);
}

// ---- from ExprSynth.cpp
template <typename T, typename Functor, bool optimize>
struct freefunc0 : public exprtk::ifunction<T>
{
	using exprtk::ifunction<T>::operator();
	freefunc0() : exprtk::ifunction<T>(0) { if (optimize) { exprtk::disable_has_side_effects(*this); } }
	inline T operator()() override { return Functor::process(); }
};
template <typename T, typename Functor, bool optimize>
struct freefunc1 : public exprtk::ifunction<T>
{
	using exprtk::ifunction<T>::operator();
	freefunc1() : exprtk::ifunction<T>(1) { if (optimize) { exprtk::disable_has_side_effects(*this); } }
	inline T operator()(const T& x) override { return Functor::process(x); }
};

template <typename T>
struct IntegrateFunction : public exprtk::ifunction<T>
{
	using exprtk::ifunction<T>::operator();
	~IntegrateFunction() override { delete[] m_counters; }
	IntegrateFunction(const unsigned int* frame, unsigned int sample_rate, unsigned int max_counters)
		: exprtk::ifunction<T>(1), m_firstValue(0), m_frame(frame), m_sampleRate(sample_rate),
		  m_maxCounters(max_counters), m_nCounters(0), m_nCountersCalls(0), m_cc(0)
	{
		m_counters = new double[max_counters];
		for (unsigned int i = 0; i < max_counters; ++i) { m_counters[i] = 0; }
	}
	inline T operator()(const T& x) override
	{
		if (m_frame)
		{
			if (m_nCountersCalls == 0) { m_firstValue = *m_frame; }
			if (m_firstValue == *m_frame)
			{
				++m_nCountersCalls;
				if (m_nCountersCalls > m_maxCounters) { return 0; }
				m_cc = m_nCounters;
				++m_nCounters;
			}
			else { m_frame = 0; }
		}
		T res = 0;
		if (m_cc < m_nCounters)
		{
			res = m_counters[m_cc];
			m_counters[m_cc] += x;
		}
		m_cc = (m_cc + 1) % m_nCountersCalls;
		return res / m_sampleRate;
	}
	unsigned int m_firstValue;
	const unsigned int* m_frame;
	const unsigned int m_sampleRate;
	const unsigned int m_maxCounters;
	unsigned int m_nCounters;
	unsigned int m_nCountersCalls;
	unsigned int m_cc;
	double* m_counters;
};

template <typename T>
struct LastSampleFunction : public exprtk::ifunction<T>
{
	using exprtk::ifunction<T>::operator();
	~LastSampleFunction() override { delete[] m_samples; }
	LastSampleFunction(unsigned int history_size)
		: exprtk::ifunction<T>(1), m_history_size(history_size), m_pivot_last(history_size - 1)
	{
		m_samples = new T[history_size];
		for (unsigned int i = 0; i < history_size; ++i) { m_samples[i] = 0; }
	}
	inline T operator()(const T& x) override
	{
		if (!std::isnan(x) && x >= 1 && x <= m_history_size)
		{
			return m_samples[(static_cast<std::size_t>(x) + m_pivot_last) % m_history_size];
		}
		return 0;
	}
	void setLastSample(const T& sample)
	{
		if (!std::isnan(sample) && !std::isinf(sample)) { m_samples[m_pivot_last] = sample; }
		if (m_pivot_last == 0) { m_pivot_last = m_history_size - 1; }
		else { --m_pivot_last; }
	}
	unsigned int m_history_size;
	unsigned int m_pivot_last;
	T* m_samples;
};

template <typename T>
struct WaveValueFunction : public exprtk::ifunction<T>
{
	using exprtk::ifunction<T>::operator();
	WaveValueFunction(const T* v, std::size_t s) : exprtk::ifunction<T>(1), m_vec(v), m_size(s) {}
	inline T operator()(const T& index) override { return m_vec[(int)(positiveFraction(index) * m_size)]; }
	const T* m_vec;
	const std::size_t m_size;
};
template <typename T>
struct WaveValueFunctionInterpolate : public exprtk::ifunction<T>
{
	using exprtk::ifunction<T>::operator();
	WaveValueFunctionInterpolate(const T* v, std::size_t s) : exprtk::ifunction<T>(1), m_vec(v), m_size(s) {}
	inline T operator()(const T& index) override
	{
		const T x = positiveFraction(index) * m_size;
		const int ix = (int)x;
		const float xfrc = fraction(x);
		return std::lerp(m_vec[ix], m_vec[(ix + 1) % m_size], xfrc);
	}
	const T* m_vec;
	const std::size_t m_size;
};

#include "random_data.inc"

inline unsigned int rotateLeft(unsigned int x, const int b)
{
	if (b > -32 && b < 32 && b != 0)
	{
		if (b < 0) { x = (x >> (-b)) | (x << (32 + b)); }
		else { x = (x << b) | (x >> (32 - b)); }
	}
	return x;
}

struct RandomVectorSeedFunction : public exprtk::ifunction<float>
{
	using exprtk::ifunction<float>::operator();
	RandomVectorSeedFunction() : exprtk::ifunction<float>(2) { exprtk::disable_has_side_effects(*this); }
	static inline float randv(const float& index, int irseed)
	{
		if (index < 0 || std::isnan(index) || std::isinf(index)) { return 0; }
		const auto xi = (unsigned int)index;
		const unsigned int si = irseed % data_size;
		const unsigned int sa = irseed / data_size;
		unsigned int res = rotateLeft(random_data[(xi + 23 * si + 1) % data_size] ^ random_data[(xi / data_size + sa) % data_size], sa % 31 + 1);
		res ^= rotateLeft(random_data[(3 * xi + si + 13) % data_size], (xi + 2 * si) % 32) ^ rotateLeft(random_data[(xi / data_size + 2 * sa) % data_size], xi % 31 + 1);
		return static_cast<int>(res) / (float)(1 << 31);
	}
	inline float operator()(const float& index, const float& seed) override
	{
		const int irseed = seed < 0 || std::isnan(seed) || std::isinf(seed) ? 0 : static_cast<int>(seed);
		return randv(index, irseed);
	}
	static const int data_size = sizeof(random_data) / sizeof(int);
};
static RandomVectorSeedFunction randsv_func;

struct RandomVectorFunction : public exprtk::ifunction<float>
{
	using exprtk::ifunction<float>::operator();
	RandomVectorFunction(const unsigned int seed) : exprtk::ifunction<float>(1), m_rseed(seed)
	{ exprtk::disable_has_side_effects(*this); }
	inline float operator()(const float& index) override { return RandomVectorSeedFunction::randv(index, m_rseed); }
	const unsigned int m_rseed;
};

// rand(): a fixed LCG instead of the thread's engine (the test's own; lm-expr's tests draw the same)
static std::uint32_t s_lcg = 1;
struct lcg_random
{
	static inline float process()
	{
		s_lcg = s_lcg * 1664525u + 1013904223u;
		return static_cast<float>(s_lcg >> 8) / 8388608.0f - 1.0f;
	}
};
static freefunc0<float, lcg_random, false> simple_rand;

struct sin_wave { static inline float process(float x) { x = positiveFraction(x); return std::sin(x * 2 * std::numbers::pi_v<float>); } };
struct square_wave { static inline float process(float x) { x = positiveFraction(x); if (x >= 0.5f) { return -1.0f; } else { return 1.0f; } } };
struct triangle_wave
{
	static inline float process(float x)
	{
		x = positiveFraction(x);
		if (x < 0.25f) { return x * 4.0f; }
		else { if (x < 0.75f) { return 2.0f - x * 4.0f; } else { return x * 4.0f - 4.0f; } }
	}
};
struct saw_wave { static inline float process(float x) { x = positiveFraction(x); return 2.0f * x - 1.0f; } };
struct moogsaw_wave
{
	static inline float process(float x)
	{
		x = positiveFraction(x);
		if (x < 0.5f) { return -1.0f + x * 4.0f; }
		return 1.0f - 2.0f * x;
	}
};
struct moog_wave
{
	static inline float process(float x)
	{
		x = positiveFraction(x);
		if (x > 0.5f) { x = 1.0f - x; return -1.0f + 4.0f * x * x; }
		return -1.0f + 8.0f * x * x;
	}
};
struct exp_wave
{
	static inline float process(float x)
	{
		x = positiveFraction(x);
		if (x > 0.5f) { x = 1.0f - x; }
		return -1.0f + 8.0f * x * x;
	}
};
struct exp2_wave
{
	static inline float process(float x)
	{
		x = positiveFraction(x);
		if (x > 0.5f) { return -1.0f + 8.0f * (1.0f - x) * x; }
		return -1.0f + 8.0f * x * x;
	}
};
struct harmonic_cent { static inline float process(float x) { return std::exp2(x / 1200); } };
struct harmonic_semitone { static inline float process(float x) { return std::exp2(x / 12); } };
static freefunc1<float, sin_wave, true> sin_wave_func;
static freefunc1<float, square_wave, true> square_wave_func;
static freefunc1<float, triangle_wave, true> triangle_wave_func;
static freefunc1<float, saw_wave, true> saw_wave_func;
static freefunc1<float, moogsaw_wave, true> moogsaw_wave_func;
static freefunc1<float, moog_wave, true> moog_wave_func;
static freefunc1<float, exp_wave, true> exp_wave_func;
static freefunc1<float, exp2_wave, true> exp2_wave_func;
static freefunc1<float, harmonic_cent, true> harmonic_cent_func;
static freefunc1<float, harmonic_semitone, true> harmonic_semitone_func;

std::size_t find_occurances(const std::string& haystack, const char* const needle)
{
	size_t last_pos = 0, count = 0;
	const size_t len = strlen(needle);
	while (last_pos + len <= haystack.length())
	{
		last_pos = haystack.find(needle, last_pos);
		if (last_pos == std::string::npos) break;
		++count;
		last_pos += len;
	}
	return count;
}

// The test's waves (lm-expr's tests make the same): W1 plain, W2 interpolated, W3 plain
static std::array<float, WAVE> s_w1, s_w2, s_w3;

int main()
{
	for (int i = 0; i < WAVE; ++i)
	{
		s_w1[i] = static_cast<float>(i) / WAVE * 2.0f - 1.0f;
		s_w2[i] = (i % 16) < 8 ? 0.75f : -0.5f;
		s_w3[i] = static_cast<float>((i * 37) % WAVE) / WAVE - 0.5f;
	}
	std::string line;
	while (std::getline(std::cin, line))
	{
		s_lcg = 1;
		float t = 0, f = 0, key = 57, rel = 0, trel = 0, a1 = 0, a2 = 0, a3 = -2.5f, v = 0.39f, srate = RATE,
			bnote = 69, tempo = 140, seed = 1234;
		unsigned int frame = 0;
		symbol_table_t table;
		table.add_pi();
		table.add_constant("e", std::numbers::e_v<float>);
		table.add_variable("seed", seed);
		table.add_function("sinew", sin_wave_func);
		table.add_function("squarew", square_wave_func);
		table.add_function("trianglew", triangle_wave_func);
		table.add_function("saww", saw_wave_func);
		table.add_function("moogsaww", moogsaw_wave_func);
		table.add_function("moogw", moog_wave_func);
		table.add_function("expw", exp_wave_func);
		table.add_function("expnw", exp2_wave_func);
		table.add_function("cent", harmonic_cent_func);
		table.add_function("semitone", harmonic_semitone_func);
		table.add_function("rand", simple_rand);
		RandomVectorFunction randv(4321);
		table.add_function("randv", randv);
		table.add_function("randsv", randsv_func);
		LastSampleFunction<float> last(HISTORY);
		table.add_function("last", last);
		WaveValueFunction<float> w1(s_w1.data(), WAVE);
		WaveValueFunctionInterpolate<float> w2(s_w2.data(), WAVE);
		WaveValueFunction<float> w3(s_w3.data(), WAVE);
		table.add_function("W1", w1);
		table.add_function("W2", w2);
		table.add_function("W3", w3);
		table.add_variable("A1", a1);
		table.add_variable("A2", a2);
		table.add_variable("A3", a3);
		table.add_variable("t", t);
		table.add_variable("f", f);
		table.add_variable("rel", rel);
		table.add_variable("trel", trel);
		table.add_variable("key", key);
		table.add_variable("bnote", bnote);
		table.add_variable("srate", srate);
		table.add_variable("v", v);
		table.add_variable("tempo", tempo);
		const unsigned int ointeg = find_occurances(line, "integrate");
		IntegrateFunction<float> integrate(&frame, RATE, ointeg > 0 ? ointeg : 1);
		if (ointeg > 0) { table.add_function("integrate", integrate); }

		expression_t expression;
		expression.register_symbol_table(table);
		parser_t::settings_store sstore;
		sstore.disable_all_logic_ops();
		sstore.disable_all_assignment_ops();
		sstore.disable_all_control_structures();
		parser_t parser(sstore);
		if (!parser.compile(line, expression))
		{
			const auto error = parser.get_error(0);
			std::printf("err %d\n", static_cast<int>(error.token.position));
			continue;
		}
		std::printf("ok");
		for (frame = 0; frame < FRAMES; ++frame)
		{
			// The test's schedule (lm-expr's tests set the same)
			t = frame / 100.0f;
			f = 220.0f + frame * 3.5f;
			rel = frame >= 32 ? 1.0f : 0.0f;
			trel = frame >= 32 ? (frame - 32) / 100.0f : 0.0f;
			a1 = 0.5f - frame * 0.07f;
			a2 = frame * 1.3f;
			const float y = expression.value();
			last.setLastSample(y);
			std::uint32_t bits;
			std::memcpy(&bits, &y, 4);
			std::printf(" %08x", bits);
		}
		std::printf("\n");
	}
	return 0;
}
