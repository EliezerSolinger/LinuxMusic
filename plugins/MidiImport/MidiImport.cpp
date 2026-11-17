/*
 * MidiImport.cpp - support for importing MIDI files
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 *
 * This file is part of LMMS - https://lmms.io
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


#include <QDir>
#include <QCoreApplication>
#include <QFile>

#include <algorithm>
#include <cmath>
#include <sstream>
#include <unordered_map>

#include "MidiImport.h"
#include "TrackContainer.h"
#include "InstrumentTrack.h"
#include "AutomationTrack.h"
#include "AutomationClip.h"
#include "ConfigManager.h"
#include "MidiClip.h"
#include "MidiPatch.h"
#include "Instrument.h"
#include "TimePos.h"
#include "Song.h"
#include "TempoMap.h"
#include "UserNotice.h"

#include "plugin_export.h"

#include "portsmf/include/allegro.h"

namespace
{
	constexpr std::int32_t makeID(const char c[4]) { return c[0] | (c[1] << 8) | (c[2] << 16) | (c[3] << 24); }
}

namespace lmms
{


extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT midiimport_plugin_descriptor =
{
	LMMS_STRINGIFY(PLUGIN_NAME),
	"MIDI Import",
	QT_TRANSLATE_NOOP("PluginBrowser", "Filter for importing MIDI files into LinuxMusic"),
	"Tobias Doerffel <tobydox/at/users/dot/sf/dot/net>",
	0x0100,
	Plugin::Type::ImportFilter,
	nullptr,
	nullptr,
	nullptr,
};

PLUGIN_EXPORT Plugin* lmms_plugin_main(Model*, void* data)
{
	return new MidiImport(QString::fromUtf8(static_cast<const char*>(data)));
}

}


MidiImport::MidiImport(const QString& file) :
	ImportFilter(file, &midiimport_plugin_descriptor)
{}


bool MidiImport::tryImport(TrackContainer* tc)
{
	if (!openFile()) { return false; }

#ifdef LMMS_HAVE_FLUIDSYNTH
	if (UserNotice::hasUser() && ConfigManager::inst()->sf2File().isEmpty())
	{
		UserNotice::information(tr("Setup incomplete"),
			tr("You have not set up a default soundfont in "
				"the settings dialog (Edit->Settings). "
				"Therefore no sound will be played back after "
				"importing this MIDI file. You should download "
				"a General MIDI soundfont, specify it in "
				"settings dialog and try again."));
	}
#else
	if (UserNotice::hasUser())
	{
		UserNotice::information(tr("Setup incomplete"),
			tr("You did not compile LinuxMusic with support for "
				"SoundFont2 player, which is used to add default "
				"sound to imported MIDI files. "
				"Therefore no sound will be played back after "
				"importing this MIDI file."));
	}
#endif

	switch (read32LE()) // Read ID
	{
		case makeID("MThd"):
			// printf("MidiImport::tryImport(): found MThd\n");
			return readSMF(tc);

		case makeID("RIFF"):
			// printf("MidiImport::tryImport(): found RIFF\n");
			return readRIFF(tc);

		default:
			printf("MidiImport::tryImport(): not a Standard MIDI file\n");
			return false;
	}
}



class smfMidiCC
{

public:
	AutomationTrack* at = nullptr;
	AutomationClip* ap = nullptr;
	TimePos lastPos = 0;

	smfMidiCC& create(TrackContainer* tc, QString tn)
	{
		if (!at)
		{
			// Keep LMMS responsive, for now the import runs
			// in the main thread. This should probably be
			// removed if that ever changes.
			QCoreApplication::processEvents();
			at = dynamic_cast<AutomationTrack*>(Track::create(Track::Type::Automation, tc));
		}
		if (tn != "") { at->setName(tn); }
		return *this;
	}


	void clear()
	{
		at = nullptr;
		ap = nullptr;
		lastPos = 0;
	}


	smfMidiCC& putValue(TimePos time, AutomatableModel* objModel, float value)
	{
		if (!ap || time > lastPos + DefaultTicksPerBar)
		{
			TimePos pPos = TimePos(time.getBar(), 0);
			ap = dynamic_cast<AutomationClip*>(at->createClip(pPos));
			ap->addObject(objModel);
		}

		lastPos = time;
		time = time - ap->startPosition();
		ap->putValue(time, value, false);
		ap->changeLength(TimePos(time.getBar() + 1, 0));
		return *this;
	}
};



class smfMidiChannel
{

public:
	InstrumentTrack* it = nullptr;
	MidiClip* p = nullptr;
	Instrument* it_inst = nullptr;
	bool isSF2 = false;
	bool hasNotes = false;
	QString trackName;
	MidiPatch currentPatch;

	smfMidiChannel* create(TrackContainer* tc, QString tn)
	{
		if (!it) {
			// Keep LMMS responsive
			QCoreApplication::processEvents();
			it = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, tc));

#ifdef LMMS_HAVE_FLUIDSYNTH
			it_inst = it->loadInstrument("sf2player");

			if (it_inst)
			{
				isSF2 = true;
				it_inst->loadFile(ConfigManager::inst()->sf2File());

				// (models it has; checked, as an assert is gone in a release build)
				if (auto bank = it_inst->childModel("bank")) { bank->setValue(0); }
				currentPatch.bank = 0;

				if (auto patch = it_inst->childModel("patch")) { patch->setValue(0); }
				currentPatch.program = 0;
			}
			else { it_inst = it->loadInstrument("patman"); }
#else
			it_inst = it->loadInstrument("patman");
#endif
			trackName = tn;
			if (trackName != "") { it->setName(tn); }
			// General MIDI default
			it->pitchRangeModel()->setInitValue(2);
			// Create a default pattern
			p = dynamic_cast<MidiClip*>(it->createClip(0));
		}
		return this;
	}


	void addNote(Note& n)
	{
		if (!p) { p = dynamic_cast<MidiClip*>(it->createClip(0)); }
		p->addNote(n, false);
		hasNotes = true;
	}


	void splitMidiClips()
	{
		MidiClip* newMidiClip = nullptr;
		TimePos lastEnd(0);

		p->rearrangeAllNotes();
		for (auto n : p->notes())
		{
			if (!newMidiClip || n->pos() > lastEnd + DefaultTicksPerBar)
			{
				TimePos pPos = TimePos(n->pos().getBar(), 0);
				newMidiClip = dynamic_cast<MidiClip*>(it->createClip(pPos));
			}
			lastEnd = n->pos() + n->length();

			Note newNote(*n);
			newNote.setPos(n->pos(newMidiClip->startPosition()));
			newMidiClip->addNote(newNote, false);
		}

		delete p;
		p = nullptr;
	}
};


bool MidiImport::readSMF(TrackContainer* tc)
{
	constexpr int MIDI_CC_COUNT = 128 + 1; // 0-127 (128) + pitch bend
	constexpr int preTrackSteps = 2;
	auto pd = UserNotice::Progress{TrackContainer::tr("Please wait..."), TrackContainer::tr("Importing MIDI-file..."),
		preTrackSteps};

	std::istringstream stream(readAllData().toStdString());
	auto seq = new Alg_seq(stream, true);
	seq->convert_to_beats();

	pd.setMaximum(seq->tracks() + preTrackSteps);
	pd.setValue(1);

	// 128 CC + Pitch Bend
	auto ccs = std::array<smfMidiCC, MIDI_CC_COUNT>{};

	// channel to CC object for program changes
	std::unordered_map<long, smfMidiCC> pcs;

	// channels can be set out of 256 range
	// using unordered_map should fix most invalid loads and crashes while loading
	std::unordered_map<long, smfMidiChannel> chs;
	// NOTE: unordered_map::operator[] creates a new element if none exists

	// The song's ticks are 48 to a quarter note (Allegro's beat)
	double ticksPerBeat = DefaultTicksPerBar / 4.0;
	const auto tickOf = [&](double beat) {
		return static_cast<tick_t>(std::clamp(std::llround(beat * ticksPerBeat), 0ll, static_cast<long long>(TimePos::MaxTicks)));
	};

	// The tempo along the song: the tempo map's markers (each holds its tempo up to the next one)
	auto* song = Engine::getSong();
	auto markers = std::vector<TempoMap::Marker>{};
	{
		Alg_time_map* timeMap = seq->get_time_map();
		Alg_beats& beats = timeMap->beats;
		const auto addTempo = [&](double beat, double bpm) {
			if (!std::isfinite(bpm) || bpm <= 0) { return; }
			bpm = std::clamp(bpm, static_cast<double>(MinTempo), static_cast<double>(MaxTempo));
			const auto tick = tickOf(beat);
			// The same tempo again says nothing new
			if (!markers.empty() && std::abs(markers.back().bpm - bpm) < 1e-6) { return; }
			if (!markers.empty() && markers.back().tick == tick) { markers.back().bpm = bpm; }
			else { markers.push_back({tick, bpm, false}); }
		};
		for (int i = 0; i < beats.len - 1; ++i)
		{
			const auto seconds = beats[i + 1].time - beats[i].time;
			if (seconds <= 0) { continue; }
			addTempo(beats[i].beat, (beats[i + 1].beat - beats[i].beat) / seconds * 60.0);
		}
		if (timeMap->last_tempo_flag && beats.len > 0) { addTempo(beats[beats.len - 1].beat, timeMap->last_tempo * 60.0); }
	}
	if (!markers.empty() && markers.front().tick > 0) { markers.insert(markers.begin(), {0, static_cast<double>(song->getTempo()), false}); }

	pd.setValue(2);

	// The time signatures: the first one the song's, the others changes in the tempo map (each starts a bar)
	auto& meterModel = song->getTimeSigModel();
	Alg_time_sigs* timeSigs = &seq->time_sig;
	for (int s = 0; s < timeSigs->length(); ++s)
	{
		const Alg_time_sig timeSig = (*timeSigs)[s];
		const auto meter = TempoMap::Meter{static_cast<int>(std::lround(timeSig.num)), static_cast<int>(std::lround(timeSig.den))};
		const auto tick = tickOf(timeSig.beat);
		if (tick == 0)
		{
			meterModel.setNumerator(std::clamp(meter.numerator, 1, TempoMap::MaxNumerator));
			meterModel.setDenominator(std::clamp(meter.denominator, 1, 32));
			continue;
		}
		if (!TempoMap::isValid(meter)) { continue; }
		auto at = std::find_if(markers.begin(), markers.end(), [tick](const TempoMap::Marker& m) { return m.tick == tick; });
		if (at == markers.end())
		{
			// The tempo there: the last marker's before it
			auto bpm = static_cast<double>(song->getTempo());
			for (const auto& marker : markers) { if (marker.tick < tick) { bpm = marker.bpm; } }
			if (markers.empty()) { markers.push_back({0, bpm, false}); }
			markers.push_back({tick, bpm, false, meter});
			std::sort(markers.begin(), markers.end(), [](const auto& a, const auto& b) { return a.tick < b.tick; });
		}
		else { at->meter = meter; }
	}

	// Only a tempo at the start: the song's tempo, without a map
	if (!markers.empty())
	{
		song->tempoModel().setValue(static_cast<float>(std::lround(markers.front().bpm)));
		const bool onlyStart = markers.size() == 1;
		if (onlyStart) { song->tempoMap().clear(); }
		else { song->tempoMap().setMarkers(std::move(markers)); }
	}

	// Update the tempo to avoid crash when playing a project imported
	// via the command line
	Engine::updateFramesPerTick();

	// Song events
	for (int e = 0; e < seq->length(); ++e)
	{
		Alg_event* evt = (*seq)[e];

		if (evt->is_update())
		{
			printf("Unhandled SONG update: %d %f %s\n",
				evt->get_type_code(), evt->time, evt->get_attribute());
		}
	}

	// Tracks
	for (int t = 0; t < seq->tracks(); ++t)
	{
		QString trackName = QString(tr("Track") + " %1").arg(t);
		Alg_track* trk = seq->track(t);
		pd.setValue(t + preTrackSteps);

		for (auto& cc : ccs) { cc.clear(); }

		// Now look at events
		for (int e = 0; e < trk->length(); ++e)
		{
			Alg_event* evt = (*trk)[e];

			if (evt->chan == -1)
			{
				bool handled = false;
				if (evt->is_update())
				{
					QString attr = evt->get_attribute();
					// seqnames is a track0 identifier (see allegro code)
					if (attr == (t == 0 ? "seqnames" : "tracknames")
						&& evt->get_update_type() == 's')
					{
						trackName = evt->get_string_value();
						handled = true;
					}
				}
				if (!handled) {
					// Write debug output
					printf("MISSING GLOBAL HANDLER\n");
					printf("\tChn: %ld, Type Code: %d, Time: %f", evt->chan, evt->get_type_code(), evt->time);
					if (evt->is_update())
					{
						printf(", Update Type: %s", evt->get_attribute());
						if (evt->get_update_type() == 'a') {
							printf(", Atom: %s", evt->get_atom_value());
						}
					}
					printf("\n");
				}
			}
			else if (evt->is_note())
			{
				// LMMS does not currently support specifying the channel of a single note
				// To be safe, put the notes from different channels on separate tracks so that no information is lost
				smfMidiChannel* ch = chs[evt->chan + 16 * t].create(tc, trackName);
				auto noteEvt = static_cast<Alg_note*>(evt);
				tick_t ticks = noteEvt->get_duration() * ticksPerBeat;
				Note n(
					ticks < 1 ? 1 : ticks,
					noteEvt->get_start_time() * ticksPerBeat,
					noteEvt->get_identifier(),
					// Map from MIDI velocity to LMMS volume
					noteEvt->get_loud() * (200.f / 127.f)
				);
				ch->addNote(n);
			}
			else if (evt->is_update())
			{
				smfMidiChannel* ch = chs[evt->chan + 16 * t].create(tc, trackName);

				double time = evt->time*ticksPerBeat;
				QString update(evt->get_attribute());

				if (update == "programi")
				{
					const auto prog = evt->get_integer_value();
					if (ch->isSF2)
					{
						auto& pc = pcs[evt->chan];
						AutomatableModel* objModel = ch->it_inst ? ch->it_inst->childModel("patch") : nullptr;
						if (objModel == nullptr) { continue; } // (checked: an assert is gone in a release build)
						if (time == 0)
						{
							objModel->setInitValue(prog);
						}
						else
						{
							if (!pc.at)
							{
								pc.create(tc, trackName + " > " + objModel->displayName());
							}
							pc.putValue(time, objModel, prog);
						}
					}
					else
					{
						const QString num = QString::number(prog);
						const QString filter = QString().fill('0', 3 - num.length()) + num + "*.pat";
						const QString dir = "/usr/share/midi/"
								"freepats/Tone_000/";
						const QStringList files = QDir(dir).
						entryList(QStringList(filter));
						if (ch->it_inst && !files.empty())
						{
							ch->it_inst->loadFile(dir + files.front());
						}
					}
				}
				else if (update.startsWith("control") || update == "bendr")
				{
					int ccid = update.mid(7, update.length() - 8).toInt();
					if (update == "bendr") { ccid = 128; }
					if (ccid <= 128)
					{
						double cc = evt->get_real_value();
						AutomatableModel* modelObject = nullptr;
						// Some CC knobs correspond to specific things like pitch and volume, so in that case, set the lmms pitch/volume/etc knobs.
						// Otherwise, set the knob in the instrument track CC rack
						switch (ccid)
						{
							case 0:
								if (ch->isSF2 && ch->it_inst)
								{
									modelObject = ch->it_inst->childModel("bank");
									assert(modelObject != nullptr);
									printf("BANK SELECT (MSB) %f %d\n", cc, static_cast<int>(cc * 127));
									ch->currentPatch.setBankMSB(static_cast<std::uint8_t>(cc * 127));
									cc = static_cast<double>(ch->currentPatch.bank);
								}
								break;

							case 7:
								modelObject = ch->it->volumeModel();
								cc *= 100.0f;
								break;

							case 10:
								modelObject = ch->it->panningModel();
								cc = cc * 200.f - 100.0f;
								break;

							case 32:
								if (ch->isSF2 && ch->it_inst)
								{
									modelObject = ch->it_inst->childModel("bank");
									assert(modelObject != nullptr);
									printf("BANK SELECT (LSB) %f %d\n", cc, static_cast<int>(cc * 127));
									ch->currentPatch.setBankLSB(static_cast<std::uint8_t>(cc * 127));
									cc = static_cast<double>(ch->currentPatch.bank);
								}
								break;

							case 128:
								modelObject = ch->it->pitchModel();
								cc = cc * 100.0f;
								break;

							default:
								if (ccid >= 0 && ccid < 128)
								{
									modelObject = ch->it->midiCCModel(ccid);
									cc = cc * 127.0f;
								}
								break;
						}

						if (modelObject)
						{
							if (time == 0)
							{
								modelObject->setInitValue(cc);
							}
							else
							{
								if (ccs[ccid].at == nullptr)
								{
									ccs[ccid].create(tc, trackName + " > " + modelObject->displayName());
								}
								ccs[ccid].putValue(time, modelObject, cc);
							}
						}
					}
				}
				else {
					printf("Unhandled update: %ld %d %f %s\n",
						evt->chan, evt->get_type_code(), evt->time, evt->get_attribute());
				}
			}
		}
	}

	delete seq;

	for (auto& c: chs)
	{
		if (c.second.hasNotes)
		{
			c.second.splitMidiClips();
		}
		else if (c.second.it)
		{
			printf(" Should remove empty track\n");
			// must delete trackView first - but where is it?
			//tc->removeTrack(chs[c].it);
			//it->deleteLater();
		}
		// Set channel 10 to drums as per General MIDI's orders
		if (c.first % 16l == 9 /* channel 10 */
			&& c.second.hasNotes && c.second.it_inst && c.second.isSF2)
		{
			auto bank = c.second.it_inst->childModel("bank");
			assert(bank != nullptr);
			bank->setValue(128);
			c.second.currentPatch.bank = 128;

			auto patch = c.second.it_inst->childModel("patch");
			assert(patch != nullptr);
			patch->setValue(0);
			c.second.currentPatch.program = 0;
		}
	}

	return true;
}


bool MidiImport::readRIFF(TrackContainer* tc)
{
	// skip file length
	skip(4);

	// check file type ("RMID" = RIFF MIDI)
	if (read32LE() != makeID("RMID"))
	{
invalid_format:
		qWarning("MidiImport::readRIFF(): invalid file format");
		return false;
	}

	// search for "data" chunk
	while (true)
	{
		const std::int32_t id = read32LE();
		const std::int32_t len = read32LE();
		if (file().atEnd())
		{
data_not_found:
			qWarning("MidiImport::readRIFF(): data chunk not found");
			return false;
		}
		if (id == makeID("data")) { break; }
		if (len < 0) { goto data_not_found; }
		skip((len + 1) & ~1);
	}

	// the "data" chunk must contain data in SMF format
	if (read32LE() != makeID("MThd")) { goto invalid_format; }
	return readSMF(tc);
}


void MidiImport::error()
{
	printf("MidiImport::readTrack(): invalid MIDI data (offset %#x)\n",
		static_cast<unsigned int>(file().pos()));
}


} // namespace lmms
