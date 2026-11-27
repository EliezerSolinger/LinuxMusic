//! The browser's previews of what is not audio (F-BRW-005): a preset, a SoundFont and a MIDI file are played by the
//! plugin that opens them, rendered in a worker (an Operation of the port) into a sample the preview engine plays
//! like any other; its waveform comes from the same data. Nothing here touches the audio thread or the project.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lm_base::{Cancel, Error, Result};
use lm_plugin::{BuildContext, Loaded, Registry, ResourceView};
use lm_render::{
    BlockShape, Buffers, Event, EventKind, HostOutbox, NoteId, ProcessContext, ProcessSpec, Rng, SampleData,
    TransportInfo,
};
use lm_state::{Resource, Value, from_json};

/// The plugin that plays SoundFonts and MIDI files: the Sampler on a SoundFont
pub const SOUNDFONT_PLAYER: &str = "sampler";
/// The block the preview is rendered in
const BLOCK: usize = 256;
/// A single note of a preset or a SoundFont: its key, how long it is held, and the tail after it
const KEY: u8 = 60;
const HOLD_SECONDS: f64 = 1.5;
const TAIL_SECONDS: f64 = 1.5;
/// The longest part of a MIDI file that is played
const MIDI_SECONDS: f64 = 20.0;
const PRESET_MAX: u64 = 4 << 20;

/// What a preview plays
#[derive(Clone, Debug)]
pub enum Source {
    /// A preset file of `Presets/<plugin>/<name>.json`: one note on its plugin
    Preset(PathBuf),
    /// A SoundFont: one note
    SoundFont(PathBuf),
    /// A MIDI file, on this SoundFont (none: the first the browser finds, else the preset's default)
    Midi(PathBuf, Option<PathBuf>),
}

/// One thing to send to the processor, at a frame
#[derive(Clone, Copy, Debug, PartialEq)]
struct At {
    frame: u64,
    kind: EventKind,
}

/// The tempo map of a file: where each tick is in frames
struct Clock {
    ppq: f64,
    rate: f64,
    tick: u64,
    frame: f64,
    micros: f64,
}

impl Clock {
    fn new(ppq: u16, rate: u32) -> Clock {
        Clock { ppq: f64::from(ppq.max(1)), rate: f64::from(rate), tick: 0, frame: 0.0, micros: 500_000.0 }
    }

    /// The frame of `tick` (ticks only go forward)
    fn advance(&mut self, tick: u64) -> u64 {
        self.frame += (tick.saturating_sub(self.tick)) as f64 * self.micros / self.ppq / 1e6 * self.rate;
        self.tick = tick.max(self.tick);
        self.frame as u64
    }
}

/// The events of a MIDI file as they are played: notes, controllers, programs and bends, at their frames, up to
/// `seconds`. A note-on with velocity 0 is a note-off
fn midi_events(smf: &lm_smf::Smf, rate: u32, seconds: f64) -> Vec<At> {
    let mut all: Vec<&lm_smf::Timed> = smf.tracks.iter().flat_map(|t| t.events.iter()).collect();
    all.sort_by_key(|e| e.tick);
    let limit = (seconds * f64::from(rate)) as u64;
    let mut clock = Clock::new(smf.ppq, rate);
    let mut out = Vec::new();
    let mut next = 0u32;
    let mut open: HashMap<(u8, u8), VecDeque<NoteId>> = HashMap::new();
    for e in all {
        if let lm_smf::Event::Tempo(us) = e.event {
            clock.advance(e.tick);
            clock.micros = f64::from(us.max(1));
            continue;
        }
        let frame = clock.advance(e.tick);
        if frame > limit {
            break;
        }
        use lm_smf::Event as M;
        let kind = match e.event {
            M::NoteOn { channel, key, velocity } if velocity > 0 => {
                let id = NoteId(next);
                next += 1;
                open.entry((channel, key)).or_default().push_back(id);
                EventKind::NoteOn { note: id, channel, key, velocity: f32::from(velocity) / 127.0 }
            }
            M::NoteOn { channel, key, .. } | M::NoteOff { channel, key, .. } => {
                let Some(id) = open.get_mut(&(channel, key)).and_then(VecDeque::pop_front) else { continue };
                EventKind::NoteOff { note: id, channel, key, velocity: 0.0 }
            }
            M::Controller { channel, number, value } => {
                EventKind::Midi { bytes: [0xB0 | (channel & 15), number, value] }
            }
            M::Program { channel, program } => EventKind::Midi { bytes: [0xC0 | (channel & 15), program, 0] },
            M::PitchBend { channel, value } => {
                EventKind::Midi { bytes: [0xE0 | (channel & 15), (value & 0x7f) as u8, ((value >> 7) & 0x7f) as u8] }
            }
            _ => continue,
        };
        out.push(At { frame, kind });
    }
    // What is still sounding ends with the part played
    for ((channel, key), ids) in open {
        for note in ids {
            out.push(At {
                frame: limit.min(clock.frame as u64),
                kind: EventKind::NoteOff { note, channel, key, velocity: 0.0 },
            });
        }
    }
    out
}

/// The single note of a preset or a SoundFont
fn one_note(rate: u32) -> Vec<At> {
    let off = (HOLD_SECONDS * f64::from(rate)) as u64;
    vec![
        At { frame: 0, kind: EventKind::NoteOn { note: NoteId(0), channel: 0, key: KEY, velocity: 0.8 } },
        At { frame: off, kind: EventKind::NoteOff { note: NoteId(0), channel: 0, key: KEY, velocity: 0.0 } },
    ]
}

/// A key for a file that is not hashed: stable for its path
pub fn path_key(path: &Path) -> String {
    blake3::hash(path.display().to_string().as_bytes()).to_hex().to_string()
}

fn soundfont_state(registry: &Registry, path: &Path) -> Result<(Value, ResourceView)> {
    let resource = Resource::new("soundfont", &path_key(path))?;
    let state = registry.complete(
        SOUNDFONT_PLAYER,
        &Value::empty_object().with("format", Value::text("sf2")).with("soundfont", Value::resource(resource.clone())),
    )?;
    let view = ResourceView::new().with(resource, Loaded::File(Arc::from(path)));
    Ok((state, view))
}

/// The plugin, state and resources of a source, and what it plays
fn prepare(
    registry: &Registry,
    source: &Source,
    rate: u32,
    first_soundfont: Option<&Path>,
) -> Result<(String, Value, ResourceView, Vec<At>)> {
    match source {
        Source::Preset(file) => {
            let read = |e: std::io::Error| {
                Error::new("plugin.preset.read").arg("file", file.display().to_string()).arg("problem", e.to_string())
            };
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(
                &mut std::io::Read::take(std::fs::File::open(file).map_err(read)?, PRESET_MAX),
                &mut bytes,
            )
            .map_err(read)?;
            let saved = from_json(std::str::from_utf8(&bytes).map_err(|_| Error::new("plugin.preset.invalid"))?)?;
            let plugin = saved
                .get("plugin")
                .and_then(Value::as_text)
                .ok_or_else(|| Error::new("plugin.preset.invalid"))?
                .to_owned();
            let state = saved.get("state").cloned().ok_or_else(|| Error::new("plugin.preset.invalid"))?;
            if registry.kind(&plugin) != Some(lm_plugin::PluginKind::Instrument) {
                return Err(Error::new("port.preview.unsupported"));
            }
            Ok((plugin, state, ResourceView::new(), one_note(rate)))
        }
        Source::SoundFont(path) => {
            let (state, view) = soundfont_state(registry, path)?;
            Ok((SOUNDFONT_PLAYER.to_owned(), state, view, one_note(rate)))
        }
        Source::Midi(path, font) => {
            let bytes = std::fs::read(path).map_err(|e| {
                Error::new("session.midi.read").arg("file", path.display().to_string()).arg("problem", e.to_string())
            })?;
            let smf = lm_smf::read(&bytes).map_err(|e| Error::new(e.key))?;
            let events = midi_events(&smf, rate, MIDI_SECONDS);
            match font.as_deref().or(first_soundfont) {
                Some(f) => {
                    let (state, view) = soundfont_state(registry, f)?;
                    Ok((SOUNDFONT_PLAYER.to_owned(), state, view, events))
                }
                // No SoundFont to play it on: the plugin's own sound
                None => {
                    Ok((SOUNDFONT_PLAYER.to_owned(), registry.defaults(SOUNDFONT_PLAYER)?, ResourceView::new(), events))
                }
            }
        }
    }
}

/// Renders the source into stereo audio (in a worker; cancellable between blocks)
pub fn render(
    registry: &Registry,
    source: &Source,
    rate: u32,
    first_soundfont: Option<&Path>,
    cancel: &Cancel,
) -> Result<SampleData> {
    let (plugin, state, view, mut events) = prepare(registry, source, rate, first_soundfont)?;
    let spec = ProcessSpec { sample_rate: rate, max_block: BLOCK };
    let mut processor = registry.build(&plugin, &state, &BuildContext { spec, resources: &view, channels: 0 })?;
    let outputs: usize = registry
        .ports(&plugin, &state)
        .map_or(2, |p| p.outputs.iter().map(|o| o.channels as usize).sum::<usize>())
        .max(1);
    events.sort_by_key(|e| (e.frame, e.kind.rank()));
    let last = events.iter().map(|e| e.frame).max().unwrap_or(0);
    let total = (last as f64 + TAIL_SECONDS * f64::from(rate)) as u64;
    let mut left = Vec::with_capacity(total as usize);
    let mut right = Vec::with_capacity(total as usize);
    let mut data = vec![0.0f32; outputs * BLOCK];
    let mut rng = Rng::new(1);
    let mut host = HostOutbox::default();
    let mut at = 0u64;
    let mut next = 0usize;
    while at < total {
        cancel.check()?;
        let frames = BLOCK.min((total - at) as usize);
        let mut block = Vec::new();
        while next < events.len() && events[next].frame < at + frames as u64 {
            block.push(Event { offset: (events[next].frame.max(at) - at) as u32, kind: events[next].kind });
            next += 1;
        }
        data.iter_mut().for_each(|x| *x = 0.0);
        let transport = TransportInfo {
            sample_rate: rate,
            playing: true,
            recording: false,
            position: at as i64,
            tick: 0,
            ppq: 960,
            bpm: 120.0,
            time_signature: lm_time::TimeSignature::COMMON,
            frame: at,
        };
        {
            let buffers = Buffers::from_block(
                &mut data,
                BLOCK,
                frames,
                BlockShape { inputs: 0, outputs, sidechain: 0, in_place: false },
            );
            let mut cx = ProcessContext {
                frames,
                buffers,
                events: &block,
                transport: &transport,
                rng: &mut rng,
                host: &mut host,
                display: None,
            };
            processor.process(&mut cx);
        }
        let _ = host.drain().count();
        left.extend_from_slice(&data[..frames]);
        // A mono instrument is spread to both sides
        let second = if outputs > 1 { BLOCK } else { 0 };
        right.extend_from_slice(&data[second..second + frames]);
        at += frames as u64;
    }
    // The final silence is cut, keeping a short rest
    let loud = |x: &f32| x.abs() > 1e-4;
    let end = left.iter().rposition(loud).max(right.iter().rposition(loud)).map_or(0, |i| i + rate as usize / 10);
    let end = end.clamp(rate as usize / 10, left.len());
    left.truncate(end);
    right.truncate(end);
    SampleData::from_channels(rate, vec![left, right])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smf() -> lm_smf::Smf {
        use lm_smf::{Event as M, Smf, Timed, Track};
        let t = |tick, event| Timed { tick, event };
        Smf {
            format: 0,
            ppq: 480,
            tracks: vec![Track {
                events: vec![
                    t(0, M::Tempo(500_000)),
                    t(0, M::NoteOn { channel: 0, key: 60, velocity: 100 }),
                    t(480, M::NoteOn { channel: 0, key: 60, velocity: 0 }),
                    t(480, M::NoteOn { channel: 1, key: 64, velocity: 90 }),
                    t(960, M::NoteOff { channel: 1, key: 64, velocity: 0 }),
                ],
            }],
        }
    }

    #[test]
    fn a_midi_file_is_laid_out_in_frames() {
        let ev = midi_events(&smf(), 48_000, 20.0);
        assert_eq!(ev.len(), 4);
        // 480 ticks at 120 bpm: half a second
        assert_eq!(ev[1].frame, 24_000);
        assert!(matches!(ev[1].kind, EventKind::NoteOff { key: 60, .. }));
        assert_eq!(ev[3].frame, 48_000);
    }

    #[test]
    fn a_tempo_change_moves_the_frames() {
        let mut s = smf();
        s.tracks[0].events.insert(1, lm_smf::Timed { tick: 0, event: lm_smf::Event::Tempo(1_000_000) });
        let ev = midi_events(&s, 48_000, 20.0);
        assert_eq!(ev[1].frame, 48_000);
    }

    #[test]
    fn the_part_played_is_limited() {
        assert!(midi_events(&smf(), 48_000, 0.1).iter().all(|e| e.frame <= 4_800));
    }

    #[test]
    fn a_single_note_is_held_then_released() {
        let n = one_note(48_000);
        assert_eq!(n.len(), 2);
        assert_eq!(n[1].frame, 72_000);
    }

    #[test]
    fn the_key_of_a_path_is_a_resource_key() {
        assert!(Resource::new("soundfont", &path_key(Path::new("/a/b.sf2"))).is_ok());
    }
}
