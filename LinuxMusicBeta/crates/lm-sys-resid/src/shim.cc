// lm-sys-resid: the C interface to reSID that src/lib.rs binds (one SID chip per handle).
//
// Nothing here throws into Rust: the only C++ call that can throw is `new` (std::bad_alloc), caught in lm_resid_new.
// Every function but lm_resid_new and lm_resid_delete runs on the audio thread and neither allocates nor frees.
//
// This file is part of LinuxMusic - GPL-2.0-or-later.

#include <new>

#include "sid.h"

namespace
{

// The SID with access to what a copy must fix: the voices' sync pointers point into the same chip, and the
// resampling buffers (null in SAMPLE_FAST, the only mode set here) belong to one chip only
class LmSid : public reSID::SID
{
public:
	void copyFrom(const LmSid& other)
	{
		// Every plain member: registers, oscillators, envelopes, filter and external filter state, sampling state
		static_cast<reSID::SID&>(*this) = static_cast<const reSID::SID&>(other);
		// The sync ring as the constructor makes it, within this chip
		voice[0].set_sync_source(&voice[2]);
		voice[1].set_sync_source(&voice[0]);
		voice[2].set_sync_source(&voice[1]);
		// (SAMPLE_FAST: both are null in both chips; never shared)
		sample = nullptr;
		fir = nullptr;
	}
};

} // namespace

extern "C"
{

// A new chip, sampled with SAMPLE_FAST at `rate` from `clock` cycles per second, MOS8580, filter on, reset (the
// legacy SidInstrument's first period). Null if it cannot be made
void* lm_resid_new(double clock, double rate)
{
	auto* sid = new (std::nothrow) LmSid;
	if (!sid) { return nullptr; }
	if (!sid->set_sampling_parameters(clock, reSID::SAMPLE_FAST, rate))
	{
		delete sid;
		return nullptr;
	}
	sid->set_chip_model(reSID::MOS8580);
	sid->enable_filter(true);
	sid->reset();
	return sid;
}

void lm_resid_delete(void* sid)
{
	delete static_cast<LmSid*>(sid);
}

void lm_resid_set_chip_model(void* sid, int mos6581)
{
	static_cast<LmSid*>(sid)->set_chip_model(mos6581 ? reSID::MOS6581 : reSID::MOS8580);
}

void lm_resid_write(void* sid, unsigned int offset, unsigned int value)
{
	static_cast<LmSid*>(sid)->write(offset & 0x1f, value & 0xff);
}

// Clocks the chip for at most `*cycles` cycles or until `n` samples are made; `*cycles` keeps what is left
int lm_resid_clock(void* sid, int* cycles, short* buffer, int n)
{
	reSID::cycle_count delta = *cycles;
	const int made = static_cast<LmSid*>(sid)->clock(delta, buffer, n);
	*cycles = delta;
	return made;
}

void lm_resid_reset(void* sid)
{
	static_cast<LmSid*>(sid)->reset();
}

void lm_resid_copy(void* to, const void* from)
{
	static_cast<LmSid*>(to)->copyFrom(*static_cast<const LmSid*>(from));
}

const char* lm_resid_version()
{
	return resid_version_string;
}

} // extern "C"
