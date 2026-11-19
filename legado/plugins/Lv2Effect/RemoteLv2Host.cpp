/*
 * RemoteLv2Host.cpp - runs one LV2 effect in a process of its own, for the LV2 effect that isolates it
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
#include <condition_variable>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <memory>
#include <mutex>
#include <sstream>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>

#include <lilv/lilv.h>
#include <lv2/atom/atom.h>
#include <lv2/buf-size/buf-size.h>
#include <lv2/options/options.h>
#include <lv2/parameters/parameters.h>
#include <lv2/urid/urid.h>
#include <lv2/worker/worker.h>

#include "Hardware.h"
#include "Lv2Isolation.h"
#include "RemotePluginClient.h"

using namespace lmms;
using namespace lmms::Lv2Isolation;

static_assert(static_cast<int>(IdLv2Load) >= static_cast<int>(IdUserBase), "after RemotePluginBase's messages");

namespace
{

//! URIs to numbers, both ways, for this process (the plugin's view: the program's numbers never reach it)
class UridMap
{
public:
	UridMap()
	{
		m_map.handle = this;
		m_map.map = [](LV2_URID_Map_Handle handle, const char* uri) { return static_cast<UridMap*>(handle)->map(uri); };
		m_unmap.handle = this;
		m_unmap.unmap = [](LV2_URID_Unmap_Handle handle, LV2_URID urid) {
			return static_cast<UridMap*>(handle)->unmap(urid);
		};
	}

	auto map(const char* uri) -> LV2_URID
	{
		if (!uri) { return 0; }
		const auto lock = std::lock_guard{m_mutex};
		if (const auto found = m_ids.find(uri); found != m_ids.end()) { return found->second; }
		m_uris.emplace_back(uri);
		const auto id = static_cast<LV2_URID>(m_uris.size());
		m_ids.emplace(m_uris.back(), id);
		return id;
	}

	auto unmap(LV2_URID urid) -> const char*
	{
		const auto lock = std::lock_guard{m_mutex};
		return urid >= 1 && urid <= m_uris.size() ? m_uris[urid - 1].c_str() : nullptr;
	}

	LV2_URID_Map m_map{};
	LV2_URID_Unmap m_unmap{};

private:
	std::mutex m_mutex;
	std::deque<std::string> m_uris; //!< (a deque: the strings do not move)
	std::unordered_map<std::string, LV2_URID> m_ids;
};

/**
 * The LV2 worker, as the program's (Lv2Worker): with a thread of its own when the program has one, otherwise the work
 * done at once when the plugin asks for it. Responses reach the plugin at the start of its next run.
 */
class Worker
{
public:
	explicit Worker(bool threaded) : m_threaded{threaded}
	{
		m_schedule.handle = this;
		m_schedule.schedule_work = [](LV2_Worker_Schedule_Handle handle, uint32_t size, const void* data) {
			return static_cast<Worker*>(handle)->schedule(size, data);
		};
		if (m_threaded) { m_thread = std::thread{&Worker::loop, this}; }
	}

	~Worker()
	{
		{
			const auto lock = std::lock_guard{m_mutex};
			m_quit = true;
		}
		m_wake.notify_all();
		if (m_thread.joinable()) { m_thread.join(); }
	}

	void setPlugin(LV2_Handle handle, const LV2_Worker_Interface* iface)
	{
		m_handle = handle;
		m_interface = iface;
	}

	//! Before a run: the responses to the plugin
	void deliverResponses()
	{
		if (!m_interface) { return; }
		auto responses = std::deque<std::vector<char>>{};
		{
			const auto lock = std::lock_guard{m_mutex};
			responses.swap(m_responses);
		}
		for (const auto& response : responses)
		{
			m_interface->work_response(m_handle, static_cast<uint32_t>(response.size()), response.data());
		}
	}

	//! After a run
	void runFinished()
	{
		if (m_interface && m_interface->end_run) { m_interface->end_run(m_handle); }
	}

	LV2_Worker_Schedule m_schedule{};

private:
	auto schedule(uint32_t size, const void* data) -> LV2_Worker_Status
	{
		if (!m_interface) { return LV2_WORKER_ERR_UNKNOWN; }
		if (!data) { size = 0; }
		const auto* bytes = static_cast<const char*>(data);
		if (!m_threaded)
		{
			// (as the program when only rendering: the work now, its response straight to the plugin)
			return m_interface->work(m_handle, &Worker::respondNow, this, size, data);
		}
		{
			const auto lock = std::lock_guard{m_mutex};
			m_requests.emplace_back(bytes, bytes + size);
		}
		m_wake.notify_one();
		return LV2_WORKER_SUCCESS;
	}

	static auto respondNow(LV2_Worker_Respond_Handle handle, uint32_t size, const void* data) -> LV2_Worker_Status
	{
		auto* self = static_cast<Worker*>(handle);
		return self->m_interface->work_response(self->m_handle, data ? size : 0, data);
	}

	static auto respondLater(LV2_Worker_Respond_Handle handle, uint32_t size, const void* data) -> LV2_Worker_Status
	{
		auto* self = static_cast<Worker*>(handle);
		const auto* bytes = static_cast<const char*>(data);
		const auto lock = std::lock_guard{self->m_mutex};
		self->m_responses.emplace_back(bytes, bytes + (data ? size : 0));
		return LV2_WORKER_SUCCESS;
	}

	void loop()
	{
		auto lock = std::unique_lock{m_mutex};
		while (true)
		{
			m_wake.wait(lock, [this] { return m_quit || !m_requests.empty(); });
			if (m_quit) { return; }
			auto request = std::move(m_requests.front());
			m_requests.pop_front();
			lock.unlock();
			m_interface->work(m_handle, &Worker::respondLater, this, static_cast<uint32_t>(request.size()),
				request.data());
			lock.lock();
		}
	}

	const bool m_threaded;
	LV2_Handle m_handle = nullptr;
	const LV2_Worker_Interface* m_interface = nullptr;
	std::mutex m_mutex;
	std::condition_variable m_wake;
	std::deque<std::vector<char>> m_requests;
	std::deque<std::vector<char>> m_responses;
	bool m_quit = false;
	std::thread m_thread;
};

//! An atom sequence buffer of a port
struct EventBuffer
{
	std::vector<std::uint64_t> memory; //!< (8-byte aligned, as atoms are)

	void allocate(std::size_t bytes)
	{
		memory.assign((std::max(bytes, sizeof(LV2_Atom_Sequence)) + 7) / 8, 0);
	}

	auto sequence() -> LV2_Atom_Sequence* { return reinterpret_cast<LV2_Atom_Sequence*>(memory.data()); }

	//! An empty sequence for the plugin to read
	void resetForInput(LV2_URID sequenceType)
	{
		sequence()->atom.size = sizeof(LV2_Atom_Sequence_Body);
		sequence()->atom.type = sequenceType;
		sequence()->body.unit = 0;
		sequence()->body.pad = 0;
	}

	//! Room for the plugin to write in
	void resetForOutput(LV2_URID chunkType)
	{
		sequence()->atom.size = static_cast<uint32_t>(memory.size() * 8 - sizeof(LV2_Atom));
		sequence()->atom.type = chunkType;
	}
};

//! One instance of the plugin and how its ports are connected
struct Instance
{
	LilvInstance* lilv = nullptr;
	Plan plan;
	std::vector<std::vector<float>> cvBuffers;  //!< per port, for CV ports
	std::vector<EventBuffer> eventBuffers;      //!< per port, for event ports
	std::vector<const float*> connectedAudio;   //!< per port, what an audio port is connected to now
	//! What it was made with: kept while it lives (a plugin may hold on to these)
	std::vector<std::string> staticFeatureUris;
	std::vector<LV2_Feature> features;
	std::vector<const LV2_Feature*> featurePointers;
};

} // namespace


class RemoteLv2Host : public RemotePluginClient
{
public:
#ifdef SYNC_WITH_SHM_FIFO
	RemoteLv2Host(const std::string& in, const std::string& out) : RemotePluginClient(in, out)
#else
	explicit RemoteLv2Host(const char* socketPath) : RemotePluginClient(socketPath)
#endif
	{
		sendMessage(IdInitDone);
		waitForMessage(IdInitDone);
	}

	~RemoteLv2Host() override { unload(); }

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
		case IdLv2Load:
		{
			const auto why = load(m.getString(0), m.getString(1), static_cast<double>(m.getInt(2)),
				static_cast<std::size_t>(m.getInt(3)), m.getString(4), m.getString(5), m.getInt(6) != 0,
				m.getString(7));
			sendMessage(message{IdLv2Loaded}.addInt(why.empty() ? 1 : 0).addString(why));
			return true;
		}
		case IdLv2Activate:
			activate();
			return true;
		case IdLv2Deactivate:
			deactivate();
			return true;
		case IdLv2MeasureLatency:
			measureLatency();
			sendMessage(IdLv2Measured);
			return true;
		default:
			return RemotePluginClient::processMessage(m);
		}
	}

	void process(const float* const* in, float* const* out) override
	{
		if (m_instances.empty()) { return; }
		// (written by the host before each period; never more than the buffers hold)
		const auto frames = std::min(static_cast<std::size_t>(m_controls[0]), m_maxFrames);
		runInstances(in, out, frames);
	}

private:
	auto load(const std::string& bundle, const std::string& uri, double sampleRate, std::size_t maxFrames,
		const std::string& planText, const std::string& controlKey, bool threadedWorker,
		const std::string& staticFeatures) -> std::string
	{
		unload();
		if (sampleRate <= 0 || maxFrames == 0 || maxFrames > (1u << 20)) { return "impossible settings"; }
		const auto plans = decode(planText);
		if (plans.empty() || plans.size() > 64) { return "an impossible plan of ports"; }

		m_world = lilv_world_new();
		if (!m_world) { return "lilv did not start"; }
		auto* bundleNode = lilv_new_file_uri(m_world, nullptr, bundle.c_str());
		lilv_world_load_bundle(m_world, bundleNode);
		lilv_node_free(bundleNode);
		auto* uriNode = lilv_new_uri(m_world, uri.c_str());
		m_plugin = lilv_plugins_get_by_uri(lilv_world_get_all_plugins(m_world), uriNode);
		lilv_node_free(uriNode);
		if (!m_plugin) { return "no plugin " + uri + " in " + bundle; }
		const auto ports = lilv_plugin_get_num_ports(m_plugin);
		for (const auto& plan : plans)
		{
			if (plan.size() != ports) { return "the plan does not match the plugin's ports"; }
		}

		try { m_controls.attach(controlKey); }
		catch (const std::exception& error) { return std::string{"the control block did not attach: "} + error.what(); }
		for (const auto& plan : plans)
		{
			for (const auto& port : plan)
			{
				const bool inBlock = port.kind == Kind::ControlIn || port.kind == Kind::ControlOut
					|| port.kind == Kind::CvIn;
				if (inBlock && port.index >= m_controls.size()) { return "a control outside the control block"; }
			}
		}
		m_maxFrames = maxFrames;
		m_sequenceType = m_urids.map(LV2_ATOM__Sequence);
		m_chunkType = m_urids.map(LV2_ATOM__Chunk);

		// The features the program offers, the same here
		m_sampleRate = static_cast<float>(sampleRate);
		m_blockLength = static_cast<int32_t>(maxFrames);
		m_sequenceSize = 0;
		for (const auto& plan : plans)
		{
			for (const auto& port : plan) { m_sequenceSize = std::max<int32_t>(m_sequenceSize, static_cast<int32_t>(port.eventCapacity)); }
		}
		const auto floatType = m_urids.map(LV2_ATOM__Float);
		const auto intType = m_urids.map(LV2_ATOM__Int);
		m_options = {
			{LV2_OPTIONS_INSTANCE, 0, m_urids.map(LV2_PARAMETERS__sampleRate), sizeof(float), floatType, &m_sampleRate},
			{LV2_OPTIONS_INSTANCE, 0, m_urids.map(LV2_BUF_SIZE__maxBlockLength), sizeof(int32_t), intType, &m_blockLength},
			{LV2_OPTIONS_INSTANCE, 0, m_urids.map(LV2_BUF_SIZE__minBlockLength), sizeof(int32_t), intType, &m_blockLength},
			{LV2_OPTIONS_INSTANCE, 0, m_urids.map(LV2_BUF_SIZE__nominalBlockLength), sizeof(int32_t), intType,
				&m_blockLength},
			{LV2_OPTIONS_INSTANCE, 0, m_urids.map(LV2_BUF_SIZE__sequenceSize), sizeof(int32_t), intType, &m_sequenceSize},
			{LV2_OPTIONS_INSTANCE, 0, 0, 0, 0, nullptr},
		};

		for (const auto& plan : plans)
		{
			auto owned = std::make_unique<Instance>();
			auto& instance = *owned;
			instance.plan = plan;
			auto worker = std::unique_ptr<Worker>{};
			auto& features = instance.features;
			features = {
				{LV2_URID__map, &m_urids.m_map},
				{LV2_URID__unmap, &m_urids.m_unmap},
				{LV2_OPTIONS__options, m_options.data()},
			};
			auto* workerInterfaceUri = lilv_new_uri(m_world, LV2_WORKER__interface);
			const bool hasWorker = lilv_plugin_has_extension_data(m_plugin, workerInterfaceUri);
			lilv_node_free(workerInterfaceUri);
			if (hasWorker)
			{
				worker = std::make_unique<Worker>(threadedWorker);
				features.push_back({LV2_WORKER__schedule, &worker->m_schedule});
			}
			auto staticUris = std::istringstream{staticFeatures};
			for (std::string feature; staticUris >> feature;) { instance.staticFeatureUris.push_back(feature); }
			for (const auto& feature : instance.staticFeatureUris) { features.push_back({feature.c_str(), nullptr}); }
			for (const auto& feature : features) { instance.featurePointers.push_back(&feature); }
			instance.featurePointers.push_back(nullptr);

			instance.lilv = lilv_plugin_instantiate(m_plugin, sampleRate, instance.featurePointers.data());
			if (!instance.lilv) { unload(); return "the plugin did not make an instance"; }
			if (worker)
			{
				const auto* iface = static_cast<const LV2_Worker_Interface*>(
					lilv_instance_get_extension_data(instance.lilv, LV2_WORKER__interface));
				if (iface) { worker->setPlugin(lilv_instance_get_handle(instance.lilv), iface); }
			}

			instance.cvBuffers.resize(plan.size());
			instance.eventBuffers.resize(plan.size());
			instance.connectedAudio.assign(plan.size(), nullptr);
			for (std::size_t port = 0; port < plan.size(); ++port)
			{
				auto* location = static_cast<void*>(nullptr);
				switch (plan[port].kind)
				{
				case Kind::ControlIn:
				case Kind::ControlOut:
					location = &m_controls[plan[port].index];
					break;
				case Kind::CvIn:
				case Kind::CvOut:
					instance.cvBuffers[port].assign(maxFrames, 0.f);
					location = instance.cvBuffers[port].data();
					break;
				case Kind::EventsIn:
				case Kind::EventsOut:
					instance.eventBuffers[port].allocate(plan[port].eventCapacity);
					location = instance.eventBuffers[port].sequence();
					break;
				case Kind::AudioIn:
				case Kind::AudioOut:
				case Kind::None:
					break; // (the audio before each run, see runInstances())
				}
				lilv_instance_connect_port(instance.lilv, static_cast<uint32_t>(port), location);
			}
			m_instances.push_back(std::move(owned));
			m_workers.push_back(std::move(worker));
		}

		// (the host makes the shared audio buffers for these: its channel count, the same as in the program)
		auto inputs = ch_cnt_t{0};
		auto outputs = ch_cnt_t{0};
		for (const auto& plan : plans)
		{
			for (const auto& port : plan)
			{
				if (port.kind == Kind::AudioIn) { inputs = std::max<ch_cnt_t>(inputs, static_cast<ch_cnt_t>(port.index + 1)); }
				if (port.kind == Kind::AudioOut) { outputs = std::max<ch_cnt_t>(outputs, static_cast<ch_cnt_t>(port.index + 1)); }
			}
		}
		setInputOutputCount(inputs, outputs);
		return {};
	}

	//! One period of every instance, the audio connected to @p in and @p out
	void runInstances(const float* const* in, float* const* out, std::size_t frames)
	{
		for (std::size_t which = 0; which < m_instances.size(); ++which)
		{
			auto& instance = *m_instances[which];
			for (std::size_t port = 0; port < instance.plan.size(); ++port)
			{
				const auto& planned = instance.plan[port];
				switch (planned.kind)
				{
				case Kind::AudioIn:
				case Kind::AudioOut:
				{
					auto* buffer = planned.kind == Kind::AudioIn
						? (in ? const_cast<float*>(in[planned.index]) : nullptr)
						: out[planned.index];
					if (instance.connectedAudio[port] != buffer)
					{
						lilv_instance_connect_port(instance.lilv, static_cast<uint32_t>(port), buffer);
						instance.connectedAudio[port] = buffer;
					}
					break;
				}
				case Kind::CvIn:
					std::fill(instance.cvBuffers[port].begin(), instance.cvBuffers[port].end(),
						m_controls[planned.index]);
					break;
				case Kind::EventsIn:
					instance.eventBuffers[port].resetForInput(m_sequenceType);
					break;
				case Kind::EventsOut:
					instance.eventBuffers[port].resetForOutput(m_chunkType);
					break;
				default:
					break;
				}
			}
			if (m_workers[which]) { m_workers[which]->deliverResponses(); }
			lilv_instance_run(instance.lilv, static_cast<uint32_t>(frames));
			if (m_workers[which]) { m_workers[which]->runFinished(); }
		}
	}

	void activate()
	{
		if (m_active) { return; }
		for (auto& instance : m_instances) { lilv_instance_activate(instance->lilv); }
		m_active = true;
	}

	void deactivate()
	{
		if (!m_active) { return; }
		for (auto& instance : m_instances) { lilv_instance_deactivate(instance->lilv); }
		m_active = false;
	}

	//! As the program does (Lv2Proc::measureLatency()): one run on silence, then activated afresh
	void measureLatency()
	{
		if (!m_active || m_instances.empty()) { return; }
		const auto inputs = static_cast<std::size_t>(inputCount());
		const auto outputs = static_cast<std::size_t>(outputCount());
		auto silence = std::vector<std::vector<float>>(inputs + outputs, std::vector<float>(m_maxFrames));
		auto pointers = std::vector<float*>{};
		for (auto& buffer : silence) { pointers.push_back(buffer.data()); }
		runInstances(pointers.data(), pointers.data() + inputs, m_maxFrames);
		// (the ports were connected to these buffers)
		for (auto& instance : m_instances) { std::fill(instance->connectedAudio.begin(), instance->connectedAudio.end(), nullptr); }
		deactivate();
		activate();
	}

	void unload()
	{
		deactivate();
		m_workers.clear(); // (before the instances: a thread of theirs may be working on one)
		for (auto& instance : m_instances) { lilv_instance_free(instance->lilv); }
		m_instances.clear();
		m_plugin = nullptr;
		if (m_world) { lilv_world_free(m_world); }
		m_world = nullptr;
		m_controls.detach();
	}

	LilvWorld* m_world = nullptr;
	const LilvPlugin* m_plugin = nullptr;
	UridMap m_urids;
	LV2_URID m_sequenceType = 0;
	LV2_URID m_chunkType = 0;
	float m_sampleRate = 0;
	int32_t m_blockLength = 0;
	int32_t m_sequenceSize = 0;
	std::vector<LV2_Options_Option> m_options;
	std::vector<std::unique_ptr<Instance>> m_instances; //!< (each where it was made: the plugin holds its features)
	std::vector<std::unique_ptr<Worker>> m_workers; //!< per instance (null without the worker extension)
	SharedMemory<float[]> m_controls;
	std::size_t m_maxFrames = 0;
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
	auto host = RemoteLv2Host{argv[1], argv[2]};
#else
	auto host = RemoteLv2Host{argv[1]};
#endif
	host.run();
	return EXIT_SUCCESS;
}
