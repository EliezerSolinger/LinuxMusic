/*
 * RemoteLadspaHost.cpp - runs one LADSPA effect in a process of its own, for the LADSPA effect that isolates it
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

#include <algorithm>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#ifdef _WIN32
#include <windows.h>
#else
#include <dlfcn.h>
#endif

#include "Hardware.h"
#include "LadspaPorts.h"
#include "RemotePluginClient.h"

using namespace lmms;
using namespace lmms::LadspaPorts;

static_assert(static_cast<int>(IdLadspaLoad) >= static_cast<int>(IdUserBase), "the LADSPA messages come after RemotePluginBase's own");

namespace
{

//! The plugin's library, loaded in this process
class Library
{
public:
	Library() = default;
	Library(const Library&) = delete;
	Library& operator=(const Library&) = delete;
	~Library() { close(); }

	auto open(const std::string& path) -> bool
	{
#ifdef _WIN32
		const auto size = MultiByteToWideChar(CP_UTF8, 0, path.c_str(), -1, nullptr, 0);
		auto wide = std::wstring(static_cast<std::size_t>(std::max(size, 1)), L'\0');
		MultiByteToWideChar(CP_UTF8, 0, path.c_str(), -1, wide.data(), size);
		m_handle = LoadLibraryW(wide.c_str());
#else
		m_handle = dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
#endif
		return m_handle != nullptr;
	}

	auto descriptorFunction() const -> LADSPA_Descriptor_Function
	{
		if (!m_handle) { return nullptr; }
#ifdef _WIN32
		return reinterpret_cast<LADSPA_Descriptor_Function>(
			GetProcAddress(static_cast<HMODULE>(m_handle), "ladspa_descriptor"));
#else
		return reinterpret_cast<LADSPA_Descriptor_Function>(dlsym(m_handle, "ladspa_descriptor"));
#endif
	}

	void close()
	{
		if (!m_handle) { return; }
#ifdef _WIN32
		FreeLibrary(static_cast<HMODULE>(m_handle));
#else
		dlclose(m_handle);
#endif
		m_handle = nullptr;
	}

private:
	void* m_handle = nullptr;
};

} // namespace


class RemoteLadspaHost : public RemotePluginClient
{
public:
#ifdef SYNC_WITH_SHM_FIFO
	RemoteLadspaHost(const std::string& in, const std::string& out) : RemotePluginClient(in, out)
#else
	explicit RemoteLadspaHost(const char* socketPath) : RemotePluginClient(socketPath)
#endif
	{
		sendMessage(IdInitDone);
		waitForMessage(IdInitDone);
	}

	~RemoteLadspaHost() override { unload(); }

	void run()
	{
		// (as the program's audio threads: denormals are slow, and the sound must be the same as there)
		disableDenormals();
		message m;
		while (!isInvalid() && (m = receiveMessage()).id != IdQuit) { processMessage(m); }
	}

	auto processMessage(const message& m) -> bool override
	{
		switch (m.id)
		{
		case IdLadspaLoad:
		{
			const auto why = load(m.getString(0), m.getString(1), static_cast<unsigned long>(m.getInt(2)),
				static_cast<std::size_t>(m.getInt(3)), static_cast<std::size_t>(m.getInt(4)), m.getString(5));
			sendMessage(message{IdLadspaLoaded}.addInt(why.empty() ? 1 : 0).addString(why));
			return true;
		}
		case IdLadspaActivate:
			activate();
			return true;
		case IdLadspaDeactivate:
			deactivate();
			return true;
		case IdLadspaMeasureLatency:
			measureLatency();
			sendMessage(IdLadspaMeasured);
			return true;
		default:
			return RemotePluginClient::processMessage(m);
		}
	}

	//! One period: the channels in the shared audio buffers, the controls in the control block
	void process(const float* const* in, float* const* out) override
	{
		if (!m_descriptor || !m_descriptor->run || m_handles.empty()) { return; }
		// (written by the host before each period; never more than the buffers hold)
		const auto frames = std::min(static_cast<std::size_t>(m_controls[0]), m_maxFrames);
		std::size_t input = 0;
		std::size_t output = 0;
		std::size_t channel = 0;
		for (std::size_t processor = 0; processor < m_handles.size(); ++processor)
		{
			for (unsigned long port = 0; port < m_descriptor->PortCount; ++port)
			{
				const auto kind = kindOf(*m_descriptor, port);
				if (kind != Kind::ChannelIn && kind != Kind::ChannelOut) { continue; }
				// LADSPA has no const input ports, but plugins must not write to their inputs
				auto* buffer = kind == Kind::ChannelIn
					? (in ? const_cast<LADSPA_Data*>(in[input++]) : nullptr)
					: out[output++];
				if (m_connected[channel] != buffer)
				{
					m_descriptor->connect_port(m_handles[processor], port, buffer);
					m_connected[channel] = buffer;
				}
				++channel;
			}
		}
		for (auto* handle : m_handles) { m_descriptor->run(handle, static_cast<unsigned long>(frames)); }
	}

private:
	//! Loads the plugin and makes its instances; what went wrong, or nothing
	auto load(const std::string& path, const std::string& label, unsigned long sampleRate, std::size_t maxFrames,
		std::size_t processors, const std::string& controlKey) -> std::string
	{
		unload();
		if (processors == 0 || processors > 64 || maxFrames == 0 || maxFrames > 1 << 20 || sampleRate == 0)
		{
			return "impossible settings";
		}
		if (!m_library.open(path)) { return "the library did not load: " + path; }
		const auto descriptorFunction = m_library.descriptorFunction();
		if (!descriptorFunction) { return "the library has no ladspa_descriptor"; }
		for (unsigned long index = 0; const auto* descriptor = descriptorFunction(index); ++index)
		{
			if (descriptor->Label && label == descriptor->Label) { m_descriptor = descriptor; break; }
		}
		if (!m_descriptor) { return "no plugin " + label + " in the library"; }
		if (!m_descriptor->instantiate || !m_descriptor->connect_port) { return "the plugin cannot be made"; }

		const auto layout = layoutOf(*m_descriptor, processors, maxFrames);
		try { m_controls.attach(controlKey); }
		catch (const std::exception& error) { return std::string{"the control block did not attach: "} + error.what(); }
		if (m_controls.size() < layout.size) { return "the control block is too small"; }
		m_maxFrames = maxFrames;

		auto inputs = ch_cnt_t{0};
		auto outputs = ch_cnt_t{0};
		for (std::size_t processor = 0; processor < processors; ++processor)
		{
			auto* handle = m_descriptor->instantiate(m_descriptor, sampleRate);
			if (!handle) { unload(); return "the plugin did not make an instance"; }
			m_handles.push_back(handle);
			for (unsigned long port = 0; port < m_descriptor->PortCount; ++port)
			{
				const auto place = layout.places[processor * m_descriptor->PortCount + port];
				if (place != NoPlace) { m_descriptor->connect_port(handle, port, &m_controls[place]); }
				else if (kindOf(*m_descriptor, port) == Kind::ChannelIn) { ++inputs; }
				else { ++outputs; }
			}
		}
		m_connected.assign(static_cast<std::size_t>(inputs + outputs), nullptr);
		// (the host makes the shared audio buffers for these)
		setInputOutputCount(inputs, outputs);
		return {};
	}

	void activate()
	{
		if (m_active || !m_descriptor) { return; }
		if (m_descriptor->activate) { for (auto* handle : m_handles) { m_descriptor->activate(handle); } }
		m_active = true;
	}

	void deactivate()
	{
		if (!m_active || !m_descriptor) { return; }
		if (m_descriptor->deactivate) { for (auto* handle : m_handles) { m_descriptor->deactivate(handle); } }
		m_active = false;
	}

	//! As the effect does in the program (LadspaEffect::measureLatency()): run once on silence, then afresh
	void measureLatency()
	{
		if (!m_active || !m_descriptor || !m_descriptor->run) { return; }
		auto silence = std::vector<std::vector<float>>(m_connected.size(), std::vector<float>(m_maxFrames));
		auto pointers = std::vector<float*>{};
		for (auto& buffer : silence) { pointers.push_back(buffer.data()); }
		const auto frames = m_controls[0];
		m_controls[0] = static_cast<float>(m_maxFrames);
		const auto inputCount = static_cast<std::size_t>(this->inputCount());
		process(pointers.data(), pointers.data() + inputCount);
		m_controls[0] = frames;
		// (the ports were connected to these buffers)
		std::fill(m_connected.begin(), m_connected.end(), nullptr);
		deactivate();
		activate();
	}

	void unload()
	{
		deactivate();
		if (m_descriptor && m_descriptor->cleanup) { for (auto* handle : m_handles) { m_descriptor->cleanup(handle); } }
		m_handles.clear();
		m_connected.clear();
		m_descriptor = nullptr;
		m_controls.detach();
		m_library.close();
	}

	Library m_library;
	const LADSPA_Descriptor* m_descriptor = nullptr;
	std::vector<LADSPA_Handle> m_handles;
	SharedMemory<float[]> m_controls;
	std::size_t m_maxFrames = 0;
	std::vector<LADSPA_Data*> m_connected; //!< what each channel port is connected to now
	bool m_active = false;
};


int main(int argc, char** argv)
{
#ifdef SYNC_WITH_SHM_FIFO
	if (argc < 3) { return EXIT_FAILURE; }
#else
	if (argc < 2) { return EXIT_FAILURE; }
#endif

#ifndef LMMS_BUILD_WIN32
	// (it ends with the program)
	const auto pollParentThread = PollParentThread{};
#endif

#ifdef SYNC_WITH_SHM_FIFO
	auto host = RemoteLadspaHost{argv[1], argv[2]};
#else
	auto host = RemoteLadspaHost{argv[1]};
#endif
	host.run();
	return EXIT_SUCCESS;
}
