/*
 * TempoRenderController.cpp - renders the recordings again when the tempo changes, asking first
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

#include "TempoRenderController.h"

#include <atomic>
#include <cmath>
#include <map>
#include <thread>

#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QFileInfo>
#include <QMessageBox>
#include <QPointer>
#include <QProgressDialog>
#include <QPushButton>

#include "AudioEngine.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "SampleClip.h"
#include "TimeStretcher.h"

namespace lmms::gui
{

namespace
{

//! How long the edits must be quiet before a check (dragging the tempo makes one check, at the end)
constexpr auto SettleMs = 500;
//! Seconds of rendering per second of audio, until a render has measured it on this computer
constexpr auto DefaultSpeed = 0.2;

auto speed() -> double
{
	const auto value = ConfigManager::inst()->value("recording", "stretchspeed").toDouble();
	return value > 0 && std::isfinite(value) ? value : DefaultSpeed;
}

} // namespace


TempoRenderController::TempoRenderController(QWidget* parent) :
	QObject{parent},
	m_parent{parent}
{
	m_settle.setSingleShot(true);
	m_settle.setInterval(SettleMs);
	connect(&m_settle, &QTimer::timeout, this, &TempoRenderController::check);

	auto* song = Engine::getSong();
	connect(song, &Song::tempoChanged, this, [this, song] {
		// With a tempo map, the song's tempo follows the map while it plays: only the map's changes count
		if (song->tempoMap().isEmpty()) { tempoEdited(); }
	});
	connect(&song->tempoMap(), &TempoMap::changed, this, &TempoRenderController::tempoEdited);
	connect(song, &Song::stretchCheckRequested, this, &TempoRenderController::scheduleCheck);
	connect(song, &Song::projectLoaded, this, [this] {
		// A project opens at the tempo it was saved at; what it plays is checked against that
		m_tempoEdited = false;
		confirm();
		scheduleCheck();
	});
	confirm();
}




void TempoRenderController::checkNow()
{
	m_settle.stop();
	check();
}




void TempoRenderController::tempoEdited()
{
	if (m_reverting) { return; }
	m_tempoEdited = true;
	scheduleCheck();
}




void TempoRenderController::scheduleCheck()
{
	m_settle.start();
}




void TempoRenderController::confirm()
{
	auto* song = Engine::getSong();
	m_confirmedTempo = song->getTempo();
	m_confirmedMarkers = song->tempoMap().markers();
}




void TempoRenderController::revert()
{
	auto* song = Engine::getSong();
	m_reverting = true;
	if (song->tempoMap().markers() != m_confirmedMarkers)
	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		song->tempoMap().setMarkers(m_confirmedMarkers);
	}
	if (song->getTempo() != m_confirmedTempo)
	{
		// Going back is not an edit: the journal keeps the change that was cancelled out of it
		song->tempoModel().saveJournallingState(false);
		song->tempoModel().setValue(static_cast<float>(m_confirmedTempo));
		song->tempoModel().restoreJournallingState();
	}
	m_reverting = false;
	m_tempoEdited = false;

	// Back at the confirmed tempo, what was rendered for it plays again
	song->stretchJobs();
}




void TempoRenderController::check()
{
	auto* song = Engine::getSong();
	if (m_busy || song->isExporting() || song->isRecordingAudio() || song->isLoadingProject())
	{
		// Later, when that is over
		m_settle.start();
		return;
	}

	m_busy = true;
	const auto jobs = song->stretchJobs();
	if (jobs.empty())
	{
		m_busy = false;
		m_tempoEdited = false;
		confirm();
		return;
	}

	// One rendering per target (the parts of a split clip share one)
	auto seconds = 0.0;
	auto targets = std::map<QString, double>{};
	for (const auto& job : jobs) { targets.emplace(job.target, job.seconds); }
	for (const auto& [target, length] : targets) { seconds += length; }
	const auto estimate = std::max(1L, std::lround(seconds * speed()));

	auto box = QMessageBox{QMessageBox::Question, tr("Render recordings"),
		m_tempoEdited
			? tr("The new tempo needs %n recorded item(s) rendered again from their originals.", nullptr,
				static_cast<int>(targets.size()))
			: tr("%n recorded item(s) no longer play at the song's tempo and need to be rendered again.", nullptr,
				static_cast<int>(targets.size())),
		QMessageBox::NoButton, m_parent};
	box.setInformativeText(tr("It takes about %n second(s).", nullptr, static_cast<int>(estimate))
		+ (TimeStretcher::canKeepPitch() ? QString{}
			: "\n" + tr("This build has no Rubber Band: the items are resampled, which changes their pitch.")));
	auto* renderButton = box.addButton(tr("Render"), QMessageBox::AcceptRole);
	box.addButton(m_tempoEdited ? tr("Cancel (back to %1 BPM)").arg(m_confirmedTempo) : tr("Cancel"),
		QMessageBox::RejectRole);
	box.setDefaultButton(renderButton);
	box.exec();

	const auto rendered = box.clickedButton() == renderButton && render(jobs);
	if (rendered) { confirm(); }
	else if (m_tempoEdited) { revert(); }
	m_tempoEdited = false;
	m_busy = false;
}




auto TempoRenderController::render(const std::vector<Song::StretchJob>& jobs) -> bool
{
	// The renderings to make, each once
	struct Item
	{
		QString source;
		QString target;
		SampleClip::Timing from;
		SampleClip::Timing to;
		double seconds;
	};
	auto items = std::vector<Item>{};
	for (const auto& job : jobs)
	{
		if (std::none_of(items.begin(), items.end(), [&](const Item& item) { return item.target == job.target; }))
		{
			items.push_back({job.source, job.target, job.from, job.to, job.seconds});
		}
	}

	const auto rate = Engine::audioEngine()->outputSampleRate();
	auto cancel = std::atomic<bool>{false};
	auto current = std::atomic<std::size_t>{0};
	auto progress = std::atomic<double>{0.0};
	auto finished = std::atomic<bool>{false};
	auto results = std::vector<TimeStretcher::Result>(items.size());

	auto elapsed = QElapsedTimer{};
	elapsed.start();
	auto worker = std::thread{[&] {
		for (std::size_t i = 0; i < items.size() && !cancel; ++i)
		{
			current = i;
			progress = 0.0;
			results[i] = TimeStretcher::render(items[i].source, items[i].target, items[i].from, items[i].to, rate,
				cancel, [&](double value) { progress = value; });
			if (results[i].status != TimeStretcher::Status::Done) { break; }
		}
		finished = true;
	}};

	auto dialog = QProgressDialog{m_parent};
	dialog.setWindowTitle(tr("Rendering recordings"));
	dialog.setWindowModality(Qt::ApplicationModal);
	dialog.setRange(0, 1000);
	dialog.setMinimumDuration(0);
	dialog.setAutoClose(false);
	dialog.setAutoReset(false);
	dialog.setCancelButtonText(tr("Cancel"));
	auto loop = QEventLoop{};
	auto poll = QTimer{};
	connect(&poll, &QTimer::timeout, &loop, [&] {
		const auto index = std::min<std::size_t>(current, items.size() - 1);
		dialog.setLabelText(tr("Item %1 of %2: %3").arg(index + 1).arg(items.size())
			.arg(QFileInfo{items[index].source}.fileName()));
		dialog.setValue(static_cast<int>(1000 * (index + progress.load()) / items.size()));
		if (finished) { loop.quit(); }
	});
	connect(&dialog, &QProgressDialog::canceled, &loop, [&] {
		cancel = true;
		dialog.setLabelText(tr("Cancelling…"));
	});
	dialog.show();
	poll.start(50);
	loop.exec();
	worker.join();
	poll.stop();
	dialog.hide();

	const auto failed = std::find_if(results.begin(), results.end(),
		[](const TimeStretcher::Result& result) { return result.status == TimeStretcher::Status::Failed; });
	const auto done = !cancel && failed == results.end()
		&& std::all_of(results.begin(), results.end(),
			[](const TimeStretcher::Result& result) { return result.status == TimeStretcher::Status::Done; });
	if (!done)
	{
		// All or nothing: the renderings already made in this batch go
		for (std::size_t i = 0; i < items.size(); ++i)
		{
			if (results[i].status == TimeStretcher::Status::Done) { QFile::remove(items[i].target); }
		}
		if (failed != results.end())
		{
			QMessageBox::warning(m_parent, tr("Rendering failed"),
				tr("The recordings could not be rendered: %1").arg(failed->error));
		}
		return false;
	}

	// How fast this computer renders, for the next estimate
	auto seconds = 0.0;
	for (const auto& item : items) { seconds += item.seconds; }
	if (seconds > 1.0)
	{
		const auto measured = elapsed.elapsed() / 1000.0 / seconds;
		ConfigManager::inst()->setValue("recording", "stretchspeed", QString::number(0.5 * speed() + 0.5 * measured));
	}

	// Every clip plays its rendering (the clips cannot go away while the modal is up)
	for (const auto& job : jobs) { job.clip->setRendering(job.target, job.to); }
	return true;
}

} // namespace lmms::gui
