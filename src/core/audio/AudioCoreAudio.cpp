/*
 * AudioCoreAudio.cpp - native CoreAudio driver for macOS: duplex, multichannel, with latencies
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

#include "AudioCoreAudio.h"

#ifdef LMMS_BUILD_APPLE

#include <algorithm>
#include <atomic>
#include <unistd.h>

#include <CoreFoundation/CoreFoundation.h>
#include <QComboBox>
#include <QFormLayout>
#include <QtDebug>

#include "AudioEngine.h"
#include "ConfigManager.h"

namespace lmms
{

namespace
{

constexpr auto Section = "audiocoreaudio";
constexpr auto OutputDeviceKey = "outputdevice"; //!< the output device's UID; empty: the system's default
constexpr auto InputDeviceKey = "inputdevice"; //!< the input device's UID; empty: the system's default
constexpr auto NoInput = "none"; //!< the input device's value for no input

auto address(AudioObjectPropertySelector selector, AudioObjectPropertyScope scope = kAudioObjectPropertyScopeGlobal,
	AudioObjectPropertyElement element = kAudioObjectPropertyElementMain) -> AudioObjectPropertyAddress
{
	return {selector, scope, element};
}

template<class T>
auto getProperty(AudioObjectID object, const AudioObjectPropertyAddress& where, T& value) -> bool
{
	auto size = static_cast<UInt32>(sizeof(T));
	return AudioObjectGetPropertyData(object, &where, 0, nullptr, &size, &value) == noErr && size == sizeof(T);
}

template<class T>
auto setProperty(AudioObjectID object, const AudioObjectPropertyAddress& where, const T& value) -> bool
{
	return AudioObjectSetPropertyData(object, &where, 0, nullptr, sizeof(T), &value) == noErr;
}

//! @returns a string property, or an empty string
auto getString(AudioObjectID object, const AudioObjectPropertyAddress& where) -> QString
{
	auto value = CFStringRef{nullptr};
	auto size = static_cast<UInt32>(sizeof(value));
	if (AudioObjectGetPropertyData(object, &where, 0, nullptr, &size, &value) != noErr || !value) { return {}; }
	const auto result = QString::fromCFString(value);
	CFRelease(value);
	return result;
}

//! @returns the channels of each buffer the device's IOProc gets in @p scope (input or output)
auto streamConfiguration(AudioObjectID device, AudioObjectPropertyScope scope) -> std::vector<UInt32>
{
	const auto where = address(kAudioDevicePropertyStreamConfiguration, scope);
	auto size = UInt32{0};
	if (AudioObjectGetPropertyDataSize(device, &where, 0, nullptr, &size) != noErr || size == 0) { return {}; }
	auto storage = std::vector<std::byte>(size);
	auto* list = reinterpret_cast<AudioBufferList*>(storage.data());
	if (AudioObjectGetPropertyData(device, &where, 0, nullptr, &size, list) != noErr) { return {}; }
	auto channels = std::vector<UInt32>{};
	for (UInt32 buffer = 0; buffer < list->mNumberBuffers; ++buffer)
	{
		channels.push_back(list->mBuffers[buffer].mNumberChannels);
	}
	return channels;
}

auto channelCount(AudioObjectID device, AudioObjectPropertyScope scope) -> int
{
	auto total = 0;
	for (const auto channels : streamConfiguration(device, scope)) { total += static_cast<int>(channels); }
	return total;
}

//! @returns whether every stream of the device in @p scope hands 32-bit float samples to the IOProc
auto streamsAreFloat(AudioObjectID device, AudioObjectPropertyScope scope) -> bool
{
	const auto where = address(kAudioDevicePropertyStreams, scope);
	auto size = UInt32{0};
	if (AudioObjectGetPropertyDataSize(device, &where, 0, nullptr, &size) != noErr) { return false; }
	auto streams = std::vector<AudioStreamID>(size / sizeof(AudioStreamID));
	if (streams.empty()) { return true; }
	if (AudioObjectGetPropertyData(device, &where, 0, nullptr, &size, streams.data()) != noErr) { return false; }
	for (const auto stream : streams)
	{
		auto format = AudioStreamBasicDescription{};
		if (!getProperty(stream, address(kAudioStreamPropertyVirtualFormat), format)) { return false; }
		if (format.mFormatID != kAudioFormatLinearPCM || !(format.mFormatFlags & kAudioFormatFlagIsFloat)
			|| format.mBitsPerChannel != 32 || (format.mFormatFlags & kAudioFormatFlagIsNonInterleaved))
		{
			return false;
		}
	}
	return true;
}

//! @returns the frames a device adds in @p scope: its own latency, its safety offset and its first stream's
auto deviceLatency(AudioObjectID device, AudioObjectPropertyScope scope) -> f_cnt_t
{
	auto latency = UInt32{0}, safety = UInt32{0}, streamLatency = UInt32{0};
	getProperty(device, address(kAudioDevicePropertyLatency, scope), latency);
	getProperty(device, address(kAudioDevicePropertySafetyOffset, scope), safety);

	const auto where = address(kAudioDevicePropertyStreams, scope);
	auto size = UInt32{0};
	if (AudioObjectGetPropertyDataSize(device, &where, 0, nullptr, &size) == noErr && size >= sizeof(AudioStreamID))
	{
		auto streams = std::vector<AudioStreamID>(size / sizeof(AudioStreamID));
		if (AudioObjectGetPropertyData(device, &where, 0, nullptr, &size, streams.data()) == noErr)
		{
			getProperty(streams.front(), address(kAudioStreamPropertyLatency), streamLatency);
		}
	}
	return static_cast<f_cnt_t>(latency) + safety + streamLatency;
}

auto defaultDevice(bool input) -> AudioObjectID
{
	auto device = AudioObjectID{kAudioObjectUnknown};
	getProperty(kAudioObjectSystemObject, address(input
		? kAudioHardwarePropertyDefaultInputDevice
		: kAudioHardwarePropertyDefaultOutputDevice), device);
	return device;
}

auto deviceForUid(const QString& uid) -> AudioObjectID
{
	for (const auto& device : AudioCoreAudio::devices())
	{
		if (device.uid == uid) { return device.id; }
	}
	return kAudioObjectUnknown;
}

//! Sets the device's sample rate, if it can take it
void setNominalRate(AudioObjectID device, sample_rate_t rate)
{
	auto current = Float64{0};
	if (getProperty(device, address(kAudioDevicePropertyNominalSampleRate), current) && current == rate) { return; }
	const auto wanted = static_cast<Float64>(rate);
	if (!setProperty(device, address(kAudioDevicePropertyNominalSampleRate), wanted))
	{
		qWarning("CoreAudio: the device does not take %u Hz", static_cast<unsigned>(rate));
	}
}

//! Gives the HAL time to finish setting a device up, until @p ready says it is
template<class Ready>
void waitUntil(Ready ready)
{
	for (int attempt = 0; attempt < 200 && !ready(); ++attempt)
	{
		CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, true);
	}
}

} // namespace




auto AudioCoreAudio::devices() -> std::vector<DeviceInfo>
{
	const auto where = address(kAudioHardwarePropertyDevices);
	auto size = UInt32{0};
	if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &where, 0, nullptr, &size) != noErr) { return {}; }
	auto ids = std::vector<AudioObjectID>(size / sizeof(AudioObjectID));
	if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &where, 0, nullptr, &size, ids.data()) != noErr)
	{
		return {};
	}

	auto result = std::vector<DeviceInfo>{};
	for (const auto id : ids)
	{
		auto info = DeviceInfo{};
		info.id = id;
		info.uid = getString(id, address(kAudioDevicePropertyDeviceUID));
		info.name = getString(id, address(kAudioObjectPropertyName));
		info.inputChannels = channelCount(id, kAudioObjectPropertyScopeInput);
		info.outputChannels = channelCount(id, kAudioObjectPropertyScopeOutput);
		// Our own private aggregates are not for the user to choose
		if (info.uid.startsWith("org.linuxmusic.duplex")) { continue; }
		result.push_back(info);
	}
	return result;
}




AudioCoreAudio::AudioCoreAudio(bool& successful, AudioEngine* engine)
	: AudioDevice(DEFAULT_CHANNELS, engine)
{
	successful = false;

	const auto outputUid = ConfigManager::inst()->value(Section, OutputDeviceKey);
	const auto inputUid = ConfigManager::inst()->value(Section, InputDeviceKey);

	const auto output = outputUid.isEmpty() ? defaultDevice(false) : deviceForUid(outputUid);
	if (output == kAudioObjectUnknown)
	{
		qWarning("CoreAudio: no output device");
		return;
	}

	auto input = AudioObjectID{kAudioObjectUnknown};
	if (inputUid != NoInput) { input = inputUid.isEmpty() ? defaultDevice(true) : deviceForUid(inputUid); }
	if (input != kAudioObjectUnknown && channelCount(input, kAudioObjectPropertyScopeInput) == 0)
	{
		input = kAudioObjectUnknown;
	}

	successful = open(output, input);
	if (!successful) { close(); }
}




AudioCoreAudio::~AudioCoreAudio()
{
	stopProcessing();
	close();
}




auto AudioCoreAudio::open(AudioObjectID output, AudioObjectID input) -> bool
{
	// Both devices at the project's rate; a device that cannot take it keeps its own
	setNominalRate(output, sampleRate());
	if (input != kAudioObjectUnknown && input != output) { setNominalRate(input, sampleRate()); }

	m_device = output;
	if (input != kAudioObjectUnknown && input != output)
	{
		m_aggregate = createAggregate(output, input);
		if (m_aggregate == kAudioObjectUnknown)
		{
			qWarning("CoreAudio: could not join the input and output devices; opening without input");
			input = kAudioObjectUnknown;
		}
		else { m_device = m_aggregate; }
	}

	// The IO buffer
	const auto wantedFrames = static_cast<UInt32>(audioEngine()->framesPerAudioBuffer());
	setProperty(m_device, address(kAudioDevicePropertyBufferFrameSize), wantedFrames);
	auto bufferFrames = wantedFrames;
	getProperty(m_device, address(kAudioDevicePropertyBufferFrameSize), bufferFrames);

	// The rate the device runs at
	auto rate = Float64{0};
	if (getProperty(m_device, address(kAudioDevicePropertyNominalSampleRate), rate) && rate > 0
		&& static_cast<sample_rate_t>(rate) != sampleRate())
	{
		setSampleRate(static_cast<sample_rate_t>(rate));
	}

	if (!streamsAreFloat(m_device, kAudioObjectPropertyScopeOutput)
		|| !streamsAreFloat(m_device, kAudioObjectPropertyScopeInput))
	{
		qWarning("CoreAudio: the device does not hand 32-bit float samples");
		return false;
	}

	// Where each channel is in the IOProc's buffers
	const auto channelSlots = [](const std::vector<UInt32>& configuration) {
		auto result = std::vector<ChannelSlot>{};
		for (UInt32 buffer = 0; buffer < configuration.size(); ++buffer)
		{
			for (UInt32 channel = 0; channel < configuration[buffer]; ++channel)
			{
				result.push_back({buffer, channel, configuration[buffer]});
			}
		}
		return result;
	};
	m_outputSlots = channelSlots(streamConfiguration(m_device, kAudioObjectPropertyScopeOutput));
	if (m_outputSlots.empty())
	{
		qWarning("CoreAudio: the output device has no output channels");
		return false;
	}

	m_inputSlots.clear();
	m_inputNames.clear();
	if (input != kAudioObjectUnknown)
	{
		// In an aggregate, the output device's own inputs (e.g. a headset's microphone) come first
		const auto all = channelSlots(streamConfiguration(m_device, kAudioObjectPropertyScopeInput));
		const auto skip = m_aggregate != kAudioObjectUnknown
			? static_cast<std::size_t>(channelCount(output, kAudioObjectPropertyScopeInput))
			: std::size_t{0};
		const auto count = static_cast<std::size_t>(channelCount(input, kAudioObjectPropertyScopeInput));
		for (auto channel = skip; channel < std::min(all.size(), skip + count); ++channel)
		{
			m_inputSlots.push_back(all[channel]);
		}
		for (std::size_t channel = 0; channel < m_inputSlots.size(); ++channel)
		{
			m_inputNames.push_back(getString(input, address(kAudioObjectPropertyElementName,
				kAudioObjectPropertyScopeInput, static_cast<AudioObjectPropertyElement>(channel + 1))));
		}
	}

	// The latencies: the devices', and the IO buffer on each side
	m_outputLatency = deviceLatency(output, kAudioObjectPropertyScopeOutput) + bufferFrames;
	m_inputLatency = input != kAudioObjectUnknown
		? deviceLatency(input, kAudioObjectPropertyScopeInput) + bufferFrames
		: 0;

	// Planar scratch for the engine; a longer callback is taken in parts
	m_capacity = std::max<f_cnt_t>(bufferFrames, audioEngine()->framesPerAudioBuffer());
	const auto engineOutputs = static_cast<ch_cnt_t>(std::min<std::size_t>(m_outputSlots.size(), DEFAULT_CHANNELS));
	setChannels(engineOutputs);
	setInputChannels(static_cast<ch_cnt_t>(m_inputSlots.size()));
	m_inputStorage.assign(m_inputSlots.size() * m_capacity, 0.f);
	m_outputStorage.assign(static_cast<std::size_t>(engineOutputs) * m_capacity, 0.f);
	m_inputPointers.resize(m_inputSlots.size());
	m_outputPointers.resize(engineOutputs);
	for (std::size_t channel = 0; channel < m_inputPointers.size(); ++channel)
	{
		m_inputPointers[channel] = m_inputStorage.data() + channel * m_capacity;
	}
	for (std::size_t channel = 0; channel < m_outputPointers.size(); ++channel)
	{
		m_outputPointers[channel] = m_outputStorage.data() + channel * m_capacity;
	}

	if (AudioDeviceCreateIOProcID(m_device, &AudioCoreAudio::ioProc, this, &m_ioProcId) != noErr)
	{
		qWarning("CoreAudio: could not create the IO procedure");
		m_ioProcId = nullptr;
		return false;
	}

	const auto overload = address(kAudioDeviceProcessorOverload);
	const auto alive = address(kAudioDevicePropertyDeviceIsAlive);
	AudioObjectAddPropertyListener(m_device, &overload, &AudioCoreAudio::propertyListener, this);
	AudioObjectAddPropertyListener(m_device, &alive, &AudioCoreAudio::propertyListener, this);
	return true;
}




auto AudioCoreAudio::createAggregate(AudioObjectID output, AudioObjectID input) -> AudioObjectID
{
	const auto outputUid = getString(output, address(kAudioDevicePropertyDeviceUID));
	const auto inputUid = getString(input, address(kAudioDevicePropertyDeviceUID));
	if (outputUid.isEmpty() || inputUid.isEmpty()) { return kAudioObjectUnknown; }

	static auto s_count = std::atomic<int>{0};
	const auto uid = QString{"org.linuxmusic.duplex.%1.%2"}.arg(getpid()).arg(s_count++);

	auto* description = CFDictionaryCreateMutable(nullptr, 0, &kCFTypeDictionaryKeyCallBacks,
		&kCFTypeDictionaryValueCallBacks);
	const auto one = 1;
	auto* yes = CFNumberCreate(nullptr, kCFNumberIntType, &one);
	auto* uidString = uid.toCFString();
	auto* outputString = outputUid.toCFString();
	auto* inputString = inputUid.toCFString();

	CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceNameKey), CFSTR("LinuxMusic"));
	CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceUIDKey), uidString);
	CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceIsPrivateKey), yes);
	CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceMainSubDeviceKey), outputString);

	// The output is the clock; the input follows it, its drift compensated
	auto* outputSub = CFDictionaryCreateMutable(nullptr, 0, &kCFTypeDictionaryKeyCallBacks,
		&kCFTypeDictionaryValueCallBacks);
	CFDictionarySetValue(outputSub, CFSTR(kAudioSubDeviceUIDKey), outputString);
	auto* inputSub = CFDictionaryCreateMutable(nullptr, 0, &kCFTypeDictionaryKeyCallBacks,
		&kCFTypeDictionaryValueCallBacks);
	CFDictionarySetValue(inputSub, CFSTR(kAudioSubDeviceUIDKey), inputString);
	CFDictionarySetValue(inputSub, CFSTR(kAudioSubDeviceDriftCompensationKey), yes);
	auto* subDevices = CFArrayCreateMutable(nullptr, 2, &kCFTypeArrayCallBacks);
	CFArrayAppendValue(subDevices, outputSub);
	CFArrayAppendValue(subDevices, inputSub);
	CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceSubDeviceListKey), subDevices);

	auto aggregate = AudioObjectID{kAudioObjectUnknown};
	const auto status = AudioHardwareCreateAggregateDevice(description, &aggregate);

	CFRelease(subDevices);
	CFRelease(inputSub);
	CFRelease(outputSub);
	CFRelease(inputString);
	CFRelease(outputString);
	CFRelease(uidString);
	CFRelease(yes);
	CFRelease(description);

	if (status != noErr) { return kAudioObjectUnknown; }

	// The HAL sets the aggregate up asynchronously: wait for its channels
	const auto outputs = channelCount(output, kAudioObjectPropertyScopeOutput);
	const auto inputs = channelCount(output, kAudioObjectPropertyScopeInput)
		+ channelCount(input, kAudioObjectPropertyScopeInput);
	waitUntil([&] {
		return channelCount(aggregate, kAudioObjectPropertyScopeOutput) >= outputs
			&& channelCount(aggregate, kAudioObjectPropertyScopeInput) >= inputs;
	});
	return aggregate;
}




void AudioCoreAudio::close()
{
	if (m_device != kAudioObjectUnknown)
	{
		const auto overload = address(kAudioDeviceProcessorOverload);
		const auto alive = address(kAudioDevicePropertyDeviceIsAlive);
		AudioObjectRemovePropertyListener(m_device, &overload, &AudioCoreAudio::propertyListener, this);
		AudioObjectRemovePropertyListener(m_device, &alive, &AudioCoreAudio::propertyListener, this);
	}
	if (m_ioProcId)
	{
		AudioDeviceStop(m_device, m_ioProcId);
		AudioDeviceDestroyIOProcID(m_device, m_ioProcId);
		m_ioProcId = nullptr;
	}
	if (m_aggregate != kAudioObjectUnknown)
	{
		AudioHardwareDestroyAggregateDevice(m_aggregate);
		m_aggregate = kAudioObjectUnknown;
	}
	m_device = kAudioObjectUnknown;
}




void AudioCoreAudio::startProcessingImpl()
{
	if (m_ioProcId && AudioDeviceStart(m_device, m_ioProcId) != noErr)
	{
		qWarning("CoreAudio: could not start the device");
	}
}




void AudioCoreAudio::stopProcessingImpl()
{
	// AudioDeviceStop() returns after the IOProc's last call
	if (m_ioProcId) { AudioDeviceStop(m_device, m_ioProcId); }
}




auto AudioCoreAudio::inputChannelName(ch_cnt_t channel) const -> QString
{
	if (channel < m_inputNames.size() && !m_inputNames[channel].isEmpty()) { return m_inputNames[channel]; }
	return AudioDevice::inputChannelName(channel);
}




auto AudioCoreAudio::ioProc(AudioObjectID, const AudioTimeStamp*, const AudioBufferList* inputData,
	const AudioTimeStamp*, AudioBufferList* outputData, const AudioTimeStamp*, void* clientData) -> OSStatus
{
	static_cast<AudioCoreAudio*>(clientData)->process(inputData, outputData);
	return noErr;
}




void AudioCoreAudio::process(const AudioBufferList* inputData, AudioBufferList* outputData)
{
	if (!outputData || outputData->mNumberBuffers == 0) { return; }
	const auto& first = outputData->mBuffers[0];
	if (first.mNumberChannels == 0) { return; }
	const auto frames = static_cast<f_cnt_t>(first.mDataByteSize / (sizeof(float) * first.mNumberChannels));

	const auto inputs = static_cast<ch_cnt_t>(m_inputSlots.size());
	const auto outputs = channels();
	const bool render = isRunning() && m_alive.load(std::memory_order_relaxed);

	for (f_cnt_t done = 0; done < frames;)
	{
		const auto count = std::min(frames - done, m_capacity);

		// The input, made planar
		for (ch_cnt_t channel = 0; channel < inputs; ++channel)
		{
			const auto& slot = m_inputSlots[channel];
			auto* planar = m_inputPointers[channel];
			const float* source = inputData && slot.buffer < inputData->mNumberBuffers
				? static_cast<const float*>(inputData->mBuffers[slot.buffer].mData)
				: nullptr;
			for (f_cnt_t frame = 0; frame < count; ++frame)
			{
				planar[frame] = source ? source[(done + frame) * slot.stride + slot.offset] : 0.f;
			}
		}

		if (render)
		{
			audioEngine()->renderNextBuffer(
				PlanarBufferView<const float>{const_cast<const float* const*>(m_inputPointers.data()), inputs, count},
				PlanarBufferView<float>{m_outputPointers.data(), outputs, count});
		}
		else
		{
			for (auto* channel : m_outputPointers) { std::fill_n(channel, count, 0.f); }
		}

		// The output, interleaved into the device's buffers; channels past the engine's are silent
		for (std::size_t channel = 0; channel < m_outputSlots.size(); ++channel)
		{
			const auto& slot = m_outputSlots[channel];
			if (slot.buffer >= outputData->mNumberBuffers) { continue; }
			auto* target = static_cast<float*>(outputData->mBuffers[slot.buffer].mData);
			if (!target) { continue; }
			const auto* planar = channel < m_outputPointers.size() ? m_outputPointers[channel] : nullptr;
			for (f_cnt_t frame = 0; frame < count; ++frame)
			{
				target[(done + frame) * slot.stride + slot.offset] = planar ? planar[frame] : 0.f;
			}
		}
		done += count;
	}
}




auto AudioCoreAudio::propertyListener(AudioObjectID, UInt32 count, const AudioObjectPropertyAddress* addresses,
	void* clientData) -> OSStatus
{
	auto* device = static_cast<AudioCoreAudio*>(clientData);
	for (UInt32 index = 0; index < count; ++index)
	{
		switch (addresses[index].mSelector)
		{
		case kAudioDeviceProcessorOverload:
			device->reportXrun();
			break;
		case kAudioDevicePropertyDeviceIsAlive:
		{
			auto alive = UInt32{1};
			getProperty(device->m_device, address(kAudioDevicePropertyDeviceIsAlive), alive);
			if (!alive)
			{
				device->m_alive = false;
				qWarning("CoreAudio: the audio device went away");
			}
			break;
		}
		default:
			break;
		}
	}
	return noErr;
}




AudioCoreAudio::setupWidget::setupWidget(QWidget* parent)
	: gui::AudioDeviceSetupWidget{AudioCoreAudio::name(), parent}
	, m_outputDevice{new QComboBox{this}}
	, m_inputDevice{new QComboBox{this}}
{
	auto* form = new QFormLayout{this};
	form->addRow(tr("Output device"), m_outputDevice);
	form->addRow(tr("Input device"), m_inputDevice);

	m_outputDevice->addItem(tr("[System Default]"), QString{});
	m_inputDevice->addItem(tr("[None]"), QString{NoInput});
	m_inputDevice->addItem(tr("[System Default]"), QString{});
	for (const auto& device : AudioCoreAudio::devices())
	{
		if (device.outputChannels > 0)
		{
			m_outputDevice->addItem(QString{"%1 (%2)"}.arg(device.name).arg(device.outputChannels), device.uid);
		}
		if (device.inputChannels > 0)
		{
			m_inputDevice->addItem(QString{"%1 (%2)"}.arg(device.name).arg(device.inputChannels), device.uid);
		}
	}

	const auto select = [](QComboBox* box, const QString& uid) {
		const auto index = box->findData(uid);
		box->setCurrentIndex(index >= 0 ? index : box->findData(QString{}));
	};
	select(m_outputDevice, ConfigManager::inst()->value(Section, OutputDeviceKey));
	select(m_inputDevice, ConfigManager::inst()->value(Section, InputDeviceKey));
}




void AudioCoreAudio::setupWidget::saveSettings()
{
	ConfigManager::inst()->setValue(Section, OutputDeviceKey, m_outputDevice->currentData().toString());
	ConfigManager::inst()->setValue(Section, InputDeviceKey, m_inputDevice->currentData().toString());
}

} // namespace lmms

#endif // LMMS_BUILD_APPLE
