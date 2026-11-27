//! The Sampler's processor: the events go to the engine at their frame, the engine's buses go to the outputs. Nothing
//! here allocates, locks or does I/O.
//!
//! A change of the source is a new build; `adopt` takes the old engine's voices over when the instrument is the same
//! (a parameter-only change never rebuilds), and otherwise keeps the old engine as a *ghost* that plays out what it
//! was playing, mixed with the new one (U32). The ghost gets the note-offs of its notes, so nothing hangs.

use std::any::Any;

use lm_base::Result;
use lm_plugin::{BuildContext, StateView};
use lm_render::{Adopted, ChannelsMut, Event, EventKind, Expression, ProcessContext, Processor};
use lm_sampler::{Engine, EngineConfig, EnvOverride, Params, Window};
use lm_state::Value;

use crate::source::Built;
use crate::{DESCRIPTOR, ENV_KEYS, display, p};

/// Frames the ghost may stay silent before it is left alone
const GHOST_QUIET_SECONDS: f32 = 0.1;

pub struct Sampler {
    current: Option<Engine>,
    ghost: Option<Engine>,
    ghost_live: bool,
    ghost_quiet: u32,
    ghost_quiet_max: u32,
    multi: bool,
    values: [f32; p::COUNT],
    max_block: usize,
    /// The display frame: the hits of the keys, and the zone map of the instrument (made in the build)
    frame: Vec<f32>,
    frame_dirty: bool,
}

impl std::fmt::Debug for Sampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sampler").field("multi", &self.multi).field("current", &self.current).finish_non_exhaustive()
    }
}

/// The engine's parameters from the plugin's values
pub fn engine_params(v: &[f32; p::COUNT]) -> Params {
    let env = {
        let e = &v[p::ENV..p::ENV + 7];
        (e[6] > 0.0).then(|| EnvOverride {
            envelope: lm_sampler::Ahdsr {
                delay: e[0],
                attack: e[1],
                hold: e[2],
                decay: e[3],
                sustain: e[4],
                release: e[5],
            },
            amount: e[6].clamp(0.0, 1.0),
        })
    };
    Params {
        gain_db: v[p::GAIN],
        pan: v[p::PAN],
        tune: v[p::TUNE],
        polyphony: v[p::POLYPHONY].max(1.0) as usize,
        bend_range: v[p::BEND_RANGE],
        env,
        window: Window { start: v[p::START], end: v[p::END], loop_start: v[p::LOOP] },
        reverse: v[p::REVERSE] >= 0.5,
        stutter: v[p::STUTTER] >= 0.5,
        mono: v[p::VOICE_MODE] >= 0.5,
        legato: v[p::VOICE_MODE] >= 1.5,
        glide: v[p::GLIDE].max(0.0) / 1000.0,
    }
}

impl Sampler {
    /// Made in the build, with everything the processor will ever use
    pub fn new(built: Built, state: &Value, cx: &BuildContext<'_>) -> Result<Self> {
        let s = StateView::new(state);
        let mut values = [0.0f32; p::COUNT];
        for (i, spec) in DESCRIPTOR.schema().params.iter().enumerate().take(p::COUNT) {
            values[i] = s.param_f32(spec.key)?;
        }
        debug_assert_eq!(DESCRIPTOR.schema().params.len(), p::COUNT);
        debug_assert!(ENV_KEYS.iter().all(|k| DESCRIPTOR.schema().find_param(k).is_some()));
        let mut config = EngineConfig::new(cx.spec.sample_rate, cx.spec.max_block);
        config.quality = built.source.quality;
        config.multi = built.source.multi;
        let frame = display_frame(&built.instrument);
        let mut engine = Engine::new(built.instrument, config);
        engine.set_params(engine_params(&values));
        if engine.streams() {
            // The long samples' tails are read by a thread of this processor (it ends when the processor is
            // dropped); if the thread cannot start, the engine reads the blocks itself
            let (link, mut feeder) = lm_sampler::stream::link(lm_sampler::stream::QUEUE);
            let spawned = std::thread::Builder::new().name("lm-sampler-stream".to_owned()).spawn(move || {
                while !feeder.abandoned() {
                    if feeder.serve() == 0 {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
            });
            if spawned.is_ok() {
                engine.set_link(link);
            }
        }
        Ok(Self {
            current: Some(engine),
            ghost: None,
            ghost_live: false,
            ghost_quiet: 0,
            ghost_quiet_max: (cx.spec.sample_rate as f32 * GHOST_QUIET_SECONDS) as u32,
            multi: built.source.multi,
            values,
            max_block: cx.spec.max_block,
            frame,
            frame_dirty: true,
        })
    }

    fn event(&mut self, kind: EventKind) {
        match kind {
            EventKind::NoteOn { note, key, velocity, .. } => {
                if let Some(e) = self.current.as_mut() {
                    e.note_on(note.0, key, velocity);
                }
                let k = usize::from(key.min(127));
                self.frame[display::HIT_COUNT + k] = (self.frame[display::HIT_COUNT + k] + 1.0) % 16_384.0;
                self.frame[display::HIT_VELOCITY + k] = (velocity * 127.0 + 0.5).clamp(1.0, 127.0);
                self.frame_dirty = true;
            }
            EventKind::NoteOff { note, .. } => {
                if let Some(e) = self.current.as_mut() {
                    e.note_off(note.0);
                }
                if self.ghost_live
                    && let Some(g) = self.ghost.as_mut()
                {
                    g.note_off(note.0);
                }
            }
            EventKind::Param { index, value } => {
                if let Some(slot) = self.values.get_mut(index as usize)
                    && *slot != value
                {
                    *slot = value;
                    let params = engine_params(&self.values);
                    if let Some(e) = self.current.as_mut() {
                        e.set_params(params);
                    }
                    if self.ghost_live
                        && let Some(g) = self.ghost.as_mut()
                    {
                        g.set_params(params);
                    }
                }
            }
            EventKind::Midi { bytes } => self.midi(bytes),
            EventKind::NoteExpression { note, expression, value, .. } => {
                if let Some(e) = self.current.as_mut() {
                    match expression {
                        Expression::Pan => e.note_expression(note.0, Some(value), None),
                        Expression::Tuning => e.note_expression(note.0, None, Some(value)),
                        Expression::Pressure => e.note_mpe(note.0, Some(value), None),
                        Expression::Timbre => e.note_mpe(note.0, None, Some(value)),
                    }
                }
            }
            EventKind::AllNotesRelease => {
                if let Some(e) = self.current.as_mut() {
                    e.all_notes_release();
                }
                if let Some(g) = self.ghost.as_mut() {
                    g.all_notes_release();
                }
            }
            EventKind::AllSoundOff => {
                if let Some(e) = self.current.as_mut() {
                    e.all_sound_off();
                }
                if let Some(g) = self.ghost.as_mut() {
                    g.all_sound_off();
                }
                self.ghost_quiet = self.ghost_quiet_max;
            }
        }
    }

    fn midi(&mut self, bytes: [u8; 3]) {
        let Some(e) = self.current.as_mut() else { return };
        match bytes[0] & 0xF0 {
            0xB0 => e.cc(bytes[1] & 0x7F, bytes[2] & 0x7F),
            0xE0 => {
                let v = i32::from(bytes[2] & 0x7F) << 7 | i32::from(bytes[1] & 0x7F);
                e.pitch_bend((v - 8192) as f32 / 8192.0);
            }
            _ => {}
        }
    }

    /// Renders `n` frames from `at` into the outputs
    fn render(&mut self, outs: &mut ChannelsMut<'_>, at: usize, n: usize) {
        if n == 0 {
            return;
        }
        let multi = self.multi;
        if let Some(e) = self.current.as_mut() {
            e.render(n);
            mix(e, outs, at, n, multi);
        }
        if self.ghost_live
            && let Some(g) = self.ghost.as_mut()
        {
            g.render(n);
            let peak = mix(g, outs, at, n, multi);
            if peak > 1.0e-7 {
                self.ghost_quiet = 0;
            } else {
                self.ghost_quiet = self.ghost_quiet.saturating_add(n as u32);
            }
            if self.ghost_quiet >= self.ghost_quiet_max && g.active_voices() == 0 {
                // (kept, not dropped: it is freed with the processor, on the main thread)
                self.ghost_live = false;
            }
        }
    }
}

/// The display frame of an instrument: no hits yet, and the zone map (how many zones cover each key and velocity band)
fn display_frame(instrument: &lm_sampler::Instrument) -> Vec<f32> {
    let mut frame = vec![0.0f32; display::WORDS];
    for z in &instrument.zones {
        for key in z.keys.0..=z.keys.1.min(127) {
            for band in 0..display::BANDS {
                let (lo, hi) = (band as u8 * 16, band as u8 * 16 + 15);
                if z.vels.0 <= hi && z.vels.1 >= lo {
                    frame[display::ZONES + usize::from(key) * display::BANDS + band] += 1.0;
                }
            }
        }
    }
    frame
}

/// Adds an engine's buses to the outputs; the largest sample added
fn mix(e: &Engine, outs: &mut ChannelsMut<'_>, at: usize, n: usize, multi: bool) -> f32 {
    let mut peak = 0.0f32;
    let mut channel = 0;
    let groups = if multi { lm_sampler::import::avl::GROUPS.len() } else { 1 };
    for g in 0..groups {
        let stereo = if multi { lm_sampler::import::avl::GROUPS[g].1 } else { true };
        let (l, r) = if g < e.groups() { e.bus(g) } else { (&[][..], &[][..]) };
        if channel >= outs.len() {
            break;
        }
        let out = outs.get_mut(channel);
        for (o, x) in out[at..at + n].iter_mut().zip(l) {
            *o += *x;
            peak = peak.max(x.abs());
        }
        channel += 1;
        if stereo {
            if channel >= outs.len() {
                break;
            }
            let out = outs.get_mut(channel);
            for (o, x) in out[at..at + n].iter_mut().zip(r) {
                *o += *x;
                peak = peak.max(x.abs());
            }
            channel += 1;
        }
    }
    peak
}

impl Processor for Sampler {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames.min(self.max_block);
        let events: &[Event] = cx.events;
        let expected = if self.multi { 9 } else { 2 };
        if cx.buffers.outputs() != expected {
            for e in events {
                self.event(e.kind);
            }
            return;
        }
        let (_, mut outs, _) = cx.buffers.split();
        // Up to each event, so that it starts at its frame
        let mut position = 0;
        for e in events {
            let at = (e.offset as usize).clamp(position, frames);
            self.render(&mut outs, position, at - position);
            position = at;
            self.event(e.kind);
        }
        self.render(&mut outs, position, frames - position);
        if self.frame_dirty
            && let Some(d) = cx.display.as_deref_mut()
        {
            let words = &self.frame;
            d.write(|bytes| {
                for (i, w) in words.iter().enumerate() {
                    if let Some(b) = bytes.get_mut(4 * i..4 * i + 4) {
                        b.copy_from_slice(&w.to_le_bytes());
                    }
                }
            });
            self.frame_dirty = false;
        }
    }

    fn reset(&mut self) {
        if let Some(e) = self.current.as_mut() {
            e.reset();
        }
        if let Some(g) = self.ghost.as_mut() {
            g.reset();
        }
        self.ghost_quiet = self.ghost_quiet_max;
        self.ghost_live = false;
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        if old.multi != self.multi {
            return Adopted::No;
        }
        let same = match (&self.current, &old.current) {
            (Some(a), Some(b)) => a.same_sound(b),
            _ => false,
        };
        if same {
            if let (Some(a), Some(b)) = (self.current.as_mut(), old.current.as_mut()) {
                a.take_over(b);
            }
            std::mem::swap(&mut self.ghost, &mut old.ghost);
            self.frame[..display::ZONES].copy_from_slice(&old.frame[..display::ZONES]);
            self.ghost_live = old.ghost_live;
            self.ghost_quiet = old.ghost_quiet;
        } else {
            // The old engine plays out its notes beside the new one
            self.frame[..display::ZONES].copy_from_slice(&old.frame[..display::ZONES]);
            self.ghost = old.current.take();
            self.ghost_live = self.ghost.is_some();
            self.ghost_quiet = 0;
        }
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
