/*
 * EnvelopeAndLfoParameters.cpp - class EnvelopeAndLfoParameters
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "EnvelopeAndLfoParameters.h"

#include <QDomElement>
#include <QFileInfo>

#include "AudioEngine.h"
#include "Engine.h"
#include "Oscillator.h"
#include "PathUtil.h"
#include "Song.h"

namespace lmms
{

// how long should be each envelope-segment maximal (e.g. attack)?
extern const float SECS_PER_ENV_SEGMENT = 5.0f;
// how long should be one LFO-oscillation maximal?
extern const float SECS_PER_LFO_OSCILLATION = 20.0f;
// minimum number of frames for ENV/LFO stages that mustn't be '0'
const f_cnt_t minimumFrames = 1;


EnvelopeAndLfoParameters::LfoInstances * EnvelopeAndLfoParameters::s_lfoInstances = nullptr;


void EnvelopeAndLfoParameters::LfoInstances::trigger()
{
	// On the audio thread, once a period, after the workers (AudioEngine::renderStageMix()): each LFO in use moves on
	// and works out its shape for the next period, which the workers then only read (fillLfoLevel())
	for (const auto& lfo : m_lfos)
	{
		lfo->m_lfoFrame += Engine::audioEngine()->framesPerPeriod();
		if (!lfo->m_lfoAmountIsZero) { lfo->updateLfoShapeData(); }
	}
}




void EnvelopeAndLfoParameters::LfoInstances::reset()
{
	// (on the audio thread, before the workers: Song::processNextBuffer())
	for (const auto& lfo : m_lfos)
	{
		lfo->m_lfoFrame = 0;
		if (!lfo->m_lfoAmountIsZero) { lfo->updateLfoShapeData(); }
	}
}




void EnvelopeAndLfoParameters::LfoInstances::add( EnvelopeAndLfoParameters * lfo )
{
	// The audio thread goes through the list (trigger()): under the audio engine's change lock, for the moment
	const auto guard = AudioEngine::changesGuard();
	m_lfos.append( lfo );
}




void EnvelopeAndLfoParameters::LfoInstances::remove( EnvelopeAndLfoParameters * lfo )
{
	const auto guard = AudioEngine::changesGuard();
	m_lfos.removeAll( lfo );
}




EnvelopeAndLfoParameters::EnvelopeAndLfoParameters(
					float _value_for_zero_amount,
							Model * _parent ) :
	Model( _parent ),
	m_used( false ),
	m_predelayModel(0.f, 0.f, 2.f, 0.001f, this, tr("Env pre-delay")),
	m_attackModel(0.f, 0.f, 2.f, 0.001f, this, tr("Env attack")),
	m_holdModel(0.5f, 0.f, 2.f, 0.001f, this, tr("Env hold")),
	m_decayModel(0.5f, 0.f, 2.f, 0.001f, this, tr("Env decay")),
	m_sustainModel(0.5f, 0.f, 1.f, 0.001f, this, tr("Env sustain")),
	m_releaseModel(0.1f, 0.f, 2.f, 0.001f, this, tr("Env release")),
	m_amountModel(0.f, -1.f, 1.f, 0.005f, this, tr("Env mod amount")),
	m_valueForZeroAmount( _value_for_zero_amount ),
	m_pahdFrames( 0 ),
	m_rFrames( 0 ),
	m_lfoPredelayModel(0.f, 0.f, 1.f, 0.001f, this, tr("LFO pre-delay")),
	m_lfoAttackModel(0.f, 0.f, 1.f, 0.001f, this, tr("LFO attack")),
	m_lfoSpeedModel(0.1f, 0.001f, 1.f, 0.0001f,
				SECS_PER_LFO_OSCILLATION * 1000.f, this,
							tr("LFO frequency")),
	m_lfoAmountModel(0.f, -1.f, 1.f, 0.005f, this, tr("LFO mod amount")),
	m_lfoWaveModel( static_cast<int>(LfoShape::SineWave), 0, NumLfoShapes, this, tr( "LFO wave shape" ) ),
	m_x100Model( false, this, tr( "LFO frequency x 100" ) ),
	m_controlEnvAmountModel( false, this, tr( "Modulate env amount" ) ),
	m_lfoFrame( 0 ),
	m_lfoAmountIsZero( false ),
	m_lfoShapeData(Engine::audioEngine()->framesPerPeriod())
{
	m_amountModel.setCenterValue( 0 );
	m_lfoAmountModel.setCenterValue( 0 );

	connect( &m_predelayModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_attackModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_holdModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_decayModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_sustainModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_releaseModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_amountModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );

	connect( &m_lfoPredelayModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_lfoAttackModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_lfoSpeedModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_lfoAmountModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_lfoWaveModel, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );
	connect( &m_x100Model, SIGNAL(dataChanged()),
			this, SLOT(updateSampleVars()), Qt::DirectConnection );

	connect( Engine::audioEngine(), SIGNAL(sampleRateChanged()),
				this, SLOT(updateSampleVars()));


	updateSampleVars();

	// The audio thread steps every LFO each period (LfoInstances::trigger()): this one only once it is whole
	if( s_lfoInstances == nullptr )
	{
		s_lfoInstances = new LfoInstances();
	}
	instances()->add( this );
}




EnvelopeAndLfoParameters::~EnvelopeAndLfoParameters()
{
	// First, while it is whole (see the constructor)
	instances()->remove( this );

	m_predelayModel.disconnect( this );
	m_attackModel.disconnect( this );
	m_holdModel.disconnect( this );
	m_decayModel.disconnect( this );
	m_sustainModel.disconnect( this );
	m_releaseModel.disconnect( this );
	m_amountModel.disconnect( this );
	m_lfoPredelayModel.disconnect( this );
	m_lfoAttackModel.disconnect( this );
	m_lfoSpeedModel.disconnect( this );
	m_lfoAmountModel.disconnect( this );
	m_lfoWaveModel.disconnect( this );
	m_x100Model.disconnect( this );

	if( instances()->isEmpty() )
	{
		delete instances();
		s_lfoInstances = nullptr;
	}
}




inline sample_t EnvelopeAndLfoParameters::lfoShapeSample( f_cnt_t _frame_offset )
{
	f_cnt_t frame = ( m_lfoFrame + _frame_offset ) % m_lfoOscillationFrames;
	const float phase = frame / static_cast<float>(
						m_lfoOscillationFrames );
	sample_t shape_sample;
	switch( static_cast<LfoShape>(m_lfoWaveModel.value())  )
	{
		case LfoShape::TriangleWave:
			shape_sample = Oscillator::triangleSample( phase );
			break;
		case LfoShape::SquareWave:
			shape_sample = Oscillator::squareSample( phase );
			break;
		case LfoShape::SawWave:
			shape_sample = Oscillator::sawSample( phase );
			break;
		case LfoShape::UserDefinedWave:
			shape_sample = Oscillator::userWaveSample(m_userWave.get(), phase);
			break;
		case LfoShape::RandomWave:
			if( frame == 0 )
			{
				m_random = Oscillator::noiseSample( 0.0f );
			}
			shape_sample = m_random;
			break;
		case LfoShape::SineWave:
		default:
			shape_sample = Oscillator::sinSample( phase );
			break;
	}
	return shape_sample * m_lfoAmount;
}




void EnvelopeAndLfoParameters::updateLfoShapeData()
{
	const f_cnt_t frames = Engine::audioEngine()->framesPerPeriod();
	for( f_cnt_t offset = 0; offset < frames; ++offset )
	{
		m_lfoShapeData[offset] = lfoShapeSample( offset );
	}
}




inline void EnvelopeAndLfoParameters::fillLfoLevel( float * _buf,
							f_cnt_t _frame,
							const f_cnt_t _frames )
{
	if( m_lfoAmountIsZero || _frame <= m_lfoPredelayFrames )
	{
		for( f_cnt_t offset = 0; offset < _frames; ++offset )
		{
			*_buf++ = 0.0f;
		}
		return;
	}
	_frame -= m_lfoPredelayFrames;

	// (the shape of this period was worked out before the workers ran, see LfoInstances::trigger())

	f_cnt_t offset = 0;
	const float lafI = 1.0f / std::max(minimumFrames, m_lfoAttackFrames);
	for( ; offset < _frames && _frame < m_lfoAttackFrames; ++offset,
								++_frame )
	{
		*_buf++ = m_lfoShapeData[offset] * _frame * lafI;
	}
	for( ; offset < _frames; ++offset )
	{
		*_buf++ = m_lfoShapeData[offset];
	}
}




void EnvelopeAndLfoParameters::fillLevel( float * _buf, f_cnt_t _frame,
						const f_cnt_t _release_begin,
						const f_cnt_t _frames )
{
	// No lock: the audio thread's workers only read here (see updateSampleVars() and LfoInstances::trigger())
	fillLfoLevel( _buf, _frame, _frames );

	for( f_cnt_t offset = 0; offset < _frames; ++offset, ++_buf, ++_frame )
	{
		float env_level;
		if( _frame < _release_begin )
		{
			if( _frame < m_pahdFrames )
			{
				env_level = m_pahdEnv[_frame];
			}
			else
			{
				env_level = m_sustainLevel;
			}
		}
		else if( ( _frame - _release_begin ) < m_rFrames )
		{
			env_level = m_rEnv[_frame - _release_begin] *
				( ( _release_begin < m_pahdFrames ) ?
				m_pahdEnv[_release_begin] : m_sustainLevel );
		}
		else
		{
			env_level = 0.0f;
		}

		// at this point, *_buf is LFO level
		*_buf = m_controlEnvAmountModel.value() ?
			env_level * ( 0.5f + *_buf ) :
			env_level + *_buf;
	}
}




void EnvelopeAndLfoParameters::saveSettings( QDomDocument & _doc,
							QDomElement & _parent )
{
	m_predelayModel.saveSettings( _doc, _parent, "pdel" );
	m_attackModel.saveSettings( _doc, _parent, "att" );
	m_holdModel.saveSettings( _doc, _parent, "hold" );
	m_decayModel.saveSettings( _doc, _parent, "dec" );
	m_sustainModel.saveSettings( _doc, _parent, "sustain" );
	m_releaseModel.saveSettings( _doc, _parent, "rel" );
	m_amountModel.saveSettings( _doc, _parent, "amt" );
	m_lfoWaveModel.saveSettings( _doc, _parent, "lshp" );
	m_lfoPredelayModel.saveSettings( _doc, _parent, "lpdel" );
	m_lfoAttackModel.saveSettings( _doc, _parent, "latt" );
	m_lfoSpeedModel.saveSettings( _doc, _parent, "lspd" );
	m_lfoAmountModel.saveSettings( _doc, _parent, "lamt" );
	m_x100Model.saveSettings( _doc, _parent, "x100" );
	m_controlEnvAmountModel.saveSettings( _doc, _parent, "ctlenvamt" );
	// (the shortest way to it: a factory or user sample by its place there, not by this computer's path; three
	// factory presets kept their author's /home path and asked for a file that was not there: upstream's #7839)
	_parent.setAttribute("userwavefile", m_userWave->audioFile().isEmpty() ? QString{}
		: PathUtil::toShortestRelative(m_userWave->audioFile()));
}




void EnvelopeAndLfoParameters::loadSettings( const QDomElement & _this )
{
	m_predelayModel.loadSettings( _this, "pdel" );
	m_attackModel.loadSettings( _this, "att" );
	m_holdModel.loadSettings( _this, "hold" );
	m_decayModel.loadSettings( _this, "dec" );
	m_sustainModel.loadSettings( _this, "sustain" );
	m_releaseModel.loadSettings( _this, "rel" );
	m_amountModel.loadSettings( _this, "amt" );
	m_lfoWaveModel.loadSettings( _this, "lshp" );
	m_lfoPredelayModel.loadSettings( _this, "lpdel" );
	m_lfoAttackModel.loadSettings( _this, "latt" );
	m_lfoSpeedModel.loadSettings( _this, "lspd" );
	m_lfoAmountModel.loadSettings( _this, "lamt" );
	m_x100Model.loadSettings( _this, "x100" );
	m_controlEnvAmountModel.loadSettings( _this, "ctlenvamt" );

/*	 ### TODO:
	Old reversed sustain kept for backward compatibility
	with 4.15 file format*/
	if( _this.hasAttribute( "sus" ) )
	{
		m_sustainModel.loadSettings( _this, "sus" );
		m_sustainModel.setValue( 1.0 - m_sustainModel.value() );
	}

	if (const auto userWaveFile = _this.attribute("userwavefile"); !userWaveFile.isEmpty())
	{
		if (QFileInfo(PathUtil::toAbsolute(userWaveFile)).exists())
		{
			setUserWave(SampleBuffer::fromFile(_this.attribute("userwavefile")));
		}
		else { Engine::getSong()->collectError(QString("%1: %2").arg(tr("Sample not found"), userWaveFile)); }  
	}

	updateSampleVars();
}




void EnvelopeAndLfoParameters::setUserWave(std::shared_ptr<const SampleBuffer> wave)
{
	{
		// The audio thread reads it (lfoShapeSample(), from trigger())
		const auto guard = AudioEngine::changesGuard();
		std::swap(m_userWave, wave);
		updateLfoShapeData();
	}
	// (the old one goes here, without the lock)
}




auto EnvelopeAndLfoParameters::shape() const -> Shape
{
	// (as updateSampleVars() works the frames out)
	const auto seconds = [](float knob) { return SECS_PER_ENV_SEGMENT * expKnobVal(knob); };
	auto result = Shape{};
	result.predelay = seconds(m_predelayModel.value());
	result.attack = seconds(m_attackModel.value());
	result.hold = seconds(m_holdModel.value());
	result.decay = seconds(m_decayModel.value() * (1 - m_sustainModel.value()));
	result.sustain = m_sustainModel.value();
	result.release = seconds(m_releaseModel.value());
	result.amount = m_amountModel.value();
	return result;
}




void EnvelopeAndLfoParameters::updateSampleVars()
{
	// Worked out without any lock, then put where the audio thread reads it (fillLevel(), on the workers, which take
	// no lock) in a moment under the audio engine's change lock. Automation changes the knobs on the audio thread
	// itself (Song::processAutomations(), before the workers run): there the tables are written in place, without
	// allocating unless they must grow, and nothing else reads them meanwhile
	const bool rendering = AudioEngine::isRenderingThread();

	const float frames_per_env_seg = SECS_PER_ENV_SEGMENT *
				Engine::audioEngine()->outputSampleRate();

	// TODO: Remove the expKnobVals, time should be linear
	const auto predelay_frames = static_cast<f_cnt_t>(frames_per_env_seg * expKnobVal(m_predelayModel.value()));

	const f_cnt_t attack_frames = std::max(minimumFrames,
					static_cast<f_cnt_t>(frames_per_env_seg *
					expKnobVal(m_attackModel.value())));

	const auto hold_frames = static_cast<f_cnt_t>(frames_per_env_seg * expKnobVal(m_holdModel.value()));

	const f_cnt_t decay_frames = std::max(minimumFrames,
					static_cast<f_cnt_t>(frames_per_env_seg *
					expKnobVal(m_decayModel.value() *
					(1 - m_sustainModel.value()))));

	const float sustain = m_sustainModel.value();
	const float amount = m_amountModel.value();
	const float amountAdd = amount >= 0 ? (1.0f - amount) * m_valueForZeroAmount : m_valueForZeroAmount;

	const f_cnt_t pahdFrames = predelay_frames + attack_frames + hold_frames + decay_frames;
	f_cnt_t rFrames = std::max(minimumFrames,
		static_cast<f_cnt_t>(frames_per_env_seg * expKnobVal(m_releaseModel.value())));
	if (static_cast<int>(floorf(amount * 1000.0f)) == 0) { rFrames = minimumFrames; }

	// The tables: in place on the audio thread (they only grow), new ones here
	auto newPahd = std::vector<sample_t>{};
	auto newR = std::vector<sample_t>{};
	auto& pahdEnv = rendering ? m_pahdEnv : newPahd;
	auto& rEnv = rendering ? m_rEnv : newR;
	if (!rendering)
	{
		pahdEnv.resize(std::max(m_pahdEnv.size(), static_cast<std::size_t>(pahdFrames)));
		rEnv.resize(std::max(m_rEnv.size(), static_cast<std::size_t>(rFrames)));
	}
	if (pahdEnv.size() < static_cast<std::size_t>(pahdFrames)) { pahdEnv.resize(pahdFrames); }
	if (rEnv.size() < static_cast<std::size_t>(rFrames)) { rEnv.resize(rFrames); }

	const float aa = amountAdd;
	for( f_cnt_t i = 0; i < predelay_frames; ++i )
	{
		pahdEnv[i] = aa;
	}

	f_cnt_t add = predelay_frames;

	const float afI = ( 1.0f / attack_frames ) * amount;
	for( f_cnt_t i = 0; i < attack_frames; ++i )
	{
		pahdEnv[add+i] = i * afI + aa;
	}

	add += attack_frames;
	const float amsum = amount + amountAdd;
	for( f_cnt_t i = 0; i < hold_frames; ++i )
	{
		pahdEnv[add + i] = amsum;
	}

	add += hold_frames;
	const float dfI = ( 1.0 / decay_frames ) * ( sustain -1 ) * amount;
	for( f_cnt_t i = 0; i < decay_frames; ++i )
	{
		pahdEnv[add + i] = amsum + i*dfI;
	}

	const float rfI = ( 1.0f / rFrames ) * amount;
	for( f_cnt_t i = 0; i < rFrames; ++i )
	{
		rEnv[i] = (float)( rFrames - i ) * rfI;
	}

	const float frames_per_lfo_oscillation = SECS_PER_LFO_OSCILLATION *
				Engine::audioEngine()->outputSampleRate();
	const auto lfoPredelayFrames = static_cast<f_cnt_t>(frames_per_lfo_oscillation
		* expKnobVal(m_lfoPredelayModel.value()));
	const auto lfoAttackFrames = static_cast<f_cnt_t>(frames_per_lfo_oscillation
		* expKnobVal(m_lfoAttackModel.value()));
	auto lfoOscillationFrames = static_cast<f_cnt_t>(frames_per_lfo_oscillation * m_lfoSpeedModel.value());
	if( m_x100Model.value() )
	{
		lfoOscillationFrames /= 100;
	}
	const float lfoAmount = m_lfoAmountModel.value() * 0.5f;
	const bool lfoAmountIsZero = static_cast<int>(floorf(lfoAmount * 1000.0f)) == 0;
	const bool used = !lfoAmountIsZero || static_cast<int>(floorf(amount * 1000.0f)) != 0;

	{
		const auto guard = AudioEngine::changesGuard();
		if (!rendering)
		{
			m_pahdEnv.swap(newPahd);
			m_rEnv.swap(newR);
		}
		m_amount = amount;
		m_amountAdd = amountAdd;
		m_pahdFrames = pahdFrames;
		m_rFrames = rFrames;
		// (worked out here once, not in the real-time part)
		m_sustainLevel = sustain * amount + amountAdd;
		m_lfoPredelayFrames = lfoPredelayFrames;
		m_lfoAttackFrames = lfoAttackFrames;
		m_lfoOscillationFrames = std::max<f_cnt_t>(1, lfoOscillationFrames);
		m_lfoAmount = lfoAmount;
		m_lfoAmountIsZero = lfoAmountIsZero;
		m_used = used;
		// The LFO's shape for this period, with what changed (then each period, see LfoInstances::trigger())
		updateLfoShapeData();
	}
	// (the old tables go here, without the lock)

	emit dataChanged();

}







} // namespace lmms
