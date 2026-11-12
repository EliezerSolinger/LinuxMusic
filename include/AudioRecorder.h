/*
 * AudioRecorder.h - records the audio input of armed tracks to disk, aligned with the song
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

#ifndef LMMS_AUDIO_RECORDER_H
#define LMMS_AUDIO_RECORDER_H

#include <atomic>
#include <condition_variable>
#include <cstdint>
#include <memory>
#include <mutex>
#include <thread>
#include <vector>

#include <QString>

#include "AudioBufferView.h"
#include "AudioInput.h"
#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

class Track;

/**
 * Records the input of armed tracks (docs/plano-gravacao.md, "Gravação").
 *
 * - The audio thread copies each track's input channels from the period's capture (AudioEngine::capture()) into a
 *   ring of its own, pre-allocated: no allocation, lock or file access there.
 * - A disk thread empties the rings into 32-bit float WAV files and updates their headers every second, so that a
 *   file is valid up to its last second if the program stops.
 * - Alignment: the input of the period being rendered was captured while the device played the period before the
 *   one rendered before it (the engine hands the device each period one period late, and takes the input one period
 *   late), and the sound took the devices' input and output latencies to get out and back in. So the first frame
 *   of the capture belongs to the song frame (period start - 2 periods - latency). A take starts on the first tick
 *   at or after its first frame, its frames before that tick dropped: on the timeline it lies within a sample of
 *   where it was played.
 * - When the disk falls so far behind that a ring is full, the frames that did not fit become silence in the file,
 *   written in their place: the rest of the take stays aligned, and the take counts the frames lost.
 */
class LMMS_EXPORT AudioRecorder
{
public:
	//! What to record for one track
	struct Request
	{
		Track* track = nullptr;
		AudioInput input;
		QString path; //!< the WAV file to write
	};

	//! A recorded take
	struct Take
	{
		Track* track = nullptr;
		QString path;
		bool started = false; //!< false if no period was recorded (the file is empty)
		std::int64_t startTick = 0; //!< where the take begins on the timeline
		std::int64_t frames = 0; //!< frames in the file
		std::int64_t lostFrames = 0; //!< frames that did not reach the disk in time (silence in the file)
		bool diskFailed = false; //!< writing to the file failed: it holds what was written before
	};

	AudioRecorder();
	~AudioRecorder();

	AudioRecorder(const AudioRecorder&) = delete;
	auto operator=(const AudioRecorder&) -> AudioRecorder& = delete;

	/**
	 * Main thread: creates the files and records from the next period on
	 * @param latency the devices' input plus output latency, and any calibration, in frames
	 * @param period the engine's frames per period
	 * @param sampleRate the rate of the capture
	 * @returns an error message, or an empty string if recording started
	 */
	auto start(const std::vector<Request>& requests, f_cnt_t latency, f_cnt_t period, sample_rate_t sampleRate)
		-> QString;

	/**
	 * Main thread, while the audio thread is not in process() (e.g. under AudioEngine::requestChangesGuard()):
	 * ends recording, writes what is left and closes the files
	 */
	auto stop() -> std::vector<Take>;

	auto isRecording() const -> bool { return m_session.load(std::memory_order_acquire) != nullptr; }

	/**
	 * Audio thread, once per period while the song plays
	 * @param capture the input captured for this period (AudioEngine::capture())
	 * @param periodStartFrame the song frame where the period being rendered starts
	 * @param framesPerTick the song's frames per tick
	 */
	void process(PlanarBufferView<const float> capture, double periodStartFrame, double framesPerTick);

private:
	struct TakeState;
	struct Session;

	void diskThread();
	static auto drain(TakeState& take) -> bool;

	std::atomic<Session*> m_session = nullptr;
	std::unique_ptr<Session> m_ownedSession; //!< the session the main thread made (m_session points to it)

	std::thread m_disk;
	std::mutex m_diskMutex;
	std::condition_variable m_diskWake;
	bool m_diskQuit = false;
};

} // namespace lmms

#endif // LMMS_AUDIO_RECORDER_H
