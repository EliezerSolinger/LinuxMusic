//! AvlDrums (F-INS-002; the fork's `plugins/AvlDrums`): the AVL drum kits by Glen MacArthur, as avldrums.lv2 by
//! Robin Gareus ships them, played by FluidSynth with avldrums.lv2's note map, and an output per group of pieces.
//!
//! Its state, version 1 (no parameters: the legacy instrument had none):
//! - `kit`: the kit, a [`Kit`] key (`black-pearl`);
//! - `multiOutput`: an output per group of pieces (`false`). The groups: kick, snare, hi-hat, tom and floor tom
//!   (mono), cymbals and percussion (stereo); off, or with the percussion kit (which has no groups), one stereo
//!   output.
//!
//! **The kits are the program's data, not the project's** (decisoes-autonomas.md, "Do AvlDrums e do Sf2 Player"):
//! 151 MB that every installation has, chosen from a closed list, so the state names the kit by its key and the build
//! reads its SoundFont from the kits folder ([`kits_dir`]). The build runs in an `Operation`, off the main thread,
//! and a kit change keeps the old kit playing until the new one is ready (I-2.3).
//!
//! As in the legacy instrument: a single stream that takes notes at their frame (FluidSynth starts them at its next
//! block of 64 of its own frames); no pitch bend; velocity 1 to 127; only control changes pass, except 10, 42, 64,
//! 66 and 68 (pan, its fine part and the pedals: a kit's layout and its hits are its own); dry (FluidSynth's reverb
//! and chorus off). Every MIDI channel plays the kit's first preset; the channel of each group is a drum channel
//! except the percussion's.

use std::any::Any;
use std::path::{Path, PathBuf};

use lm_base::{Error, Result};
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, Port, Ports, StateView, TestHints, Velocity};
use lm_render::{Adopted, Event, EventKind, ProcessContext, Processor};
use lm_state::{FieldKind, FieldSpec, NodeSchema, Value};
use lm_sys_fluidsynth::{ChannelType, Synth, SynthBuilder, SynthConfig};

use crate::half_rate::{Renderer, synth_rate};

pub const ID: &str = "avldrums";

/// A kit
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kit {
    /// The permanent key in the state
    pub key: &'static str,
    /// Its name (the kits' own names, not translated)
    pub name: &'static str,
    /// Its SoundFont in the kits folder
    pub file: &'static str,
    /// Its photo and the map of where each piece is on it (avldrums.lv2's), in the kits folder's `images/`; for the
    /// plugin's panel (I-22)
    pub photo: &'static str,
    pub map: &'static str,
    /// Whether it has the groups of pieces for an output each (the percussion kit has not)
    pub multi_output: bool,
}

/// The kits, in the legacy order
pub const KITS: [Kit; 5] = [
    Kit {
        key: "black-pearl",
        name: "Black Pearl",
        file: "Black_Pearl_4_LV2.sf2",
        photo: "black_pearl.png",
        map: "black_pearl.map.png",
        multi_output: true,
    },
    Kit {
        key: "red-zeppelin",
        name: "Red Zeppelin",
        file: "Red_Zeppelin_4_LV2.sf2",
        photo: "red_zeppelin.png",
        map: "red_zeppelin.map.png",
        multi_output: true,
    },
    Kit {
        key: "blonde-bop",
        name: "Blonde Bop",
        file: "Blonde_Bop_LV2.sf2",
        photo: "blonde_bop.png",
        map: "blonde_bop.map.png",
        multi_output: true,
    },
    Kit {
        key: "blonde-bop-hot-rod",
        name: "Blonde Bop Hot Rod",
        file: "Blonde_Bop_HR_LV2.sf2",
        photo: "blonde_bop_hr.png",
        map: "blonde_bop.map.png",
        multi_output: true,
    },
    Kit {
        key: "buskmans-holiday",
        name: "Buskman's Holiday (percussion)",
        file: "Buskmans_Holiday_LV2.sf2",
        photo: "buskman.png",
        map: "buskman.map.png",
        multi_output: false,
    },
];

static KIT_KEYS: [&str; 5] = [KITS[0].key, KITS[1].key, KITS[2].key, KITS[3].key, KITS[4].key];

impl Kit {
    pub fn from_key(key: &str) -> Option<&'static Kit> {
        KITS.iter().find(|k| k.key == key)
    }
}

/// The groups of pieces of the multi-output mode, in output order: FluidSynth renders MIDI channel `g` into its
/// output `g`
pub const GROUPS: [(&str, u32); 7] =
    [("kick", 1), ("snare", 1), ("hihat", 1), ("tom", 1), ("floortom", 1), ("cymbals", 2), ("percussion", 2)];
const MONO_GROUPS: usize = 5;
const PERCUSSION: usize = 6;

/// Which group (and MIDI channel) a key plays on, as in avldrums.lv2
pub fn group_of(key: u8) -> usize {
    match key {
        36 => 0,
        37 | 38 | 40 => 1,
        42 | 44 | 46 | 48 => 2,
        45 | 47 => 3,
        41 | 43 => 4,
        35 | 39 | 54 | 56 | 61 => PERCUSSION,
        _ => 5,
    }
}

/// The pieces the panel shows: one per key from [`FIRST_KEY`]
pub const FIRST_KEY: u8 = 35;
pub const PIECES: usize = 27;
/// Voices per MIDI channel (the legacy's 26 × 16)
const POLYPHONY: usize = 26 * 16;

/// The folder environment variable that names the kits folder (packaging, tests)
pub const KITS_DIR_VAR: &str = "LM_AVLDRUMS_DIR";

/// The folders the kits are looked for in, in order: `$LM_AVLDRUMS_DIR`; `$LM_DATA_DIR/avldrums`; next to the
/// program, `../share/linuxmusic/avldrums` (an installation) and `data/avldrums` (a portable copy); and the
/// legacy tree's `legado/data/avldrums` of this source tree, or of `$LEGACY_ROOT` (development: the kits are kept
/// once, in the repository's legacy data)
pub fn kit_folders() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os(KITS_DIR_VAR) {
        v.push(PathBuf::from(d));
    }
    if let Some(d) = std::env::var_os("LM_DATA_DIR") {
        v.push(Path::new(&d).join("avldrums"));
    }
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(exe.join("../share/linuxmusic/avldrums"));
        v.push(exe.join("data/avldrums"));
    }
    v.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../legado/data/avldrums"));
    if let Some(d) = std::env::var_os("LEGACY_ROOT") {
        v.push(Path::new(&d).join("legado/data/avldrums"));
    }
    v
}

/// The kits folder: the first of [`kit_folders`] that has the kits
pub fn kits_dir() -> Option<PathBuf> {
    kit_folders().into_iter().find(|d| d.join(KITS[0].file).is_file())
}

/// The descriptor
pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.avldrums.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    // FluidSynth's velocity layers and curve
    velocity: Velocity::Timbre,
    // The last hit of each piece, for the panel's photo (two `f32` words per piece, the plugin display convention: a count of hits and the velocity)
    display: Some(DisplaySpec { bytes: 8 * PIECES }),
    build,
    tests: TestHints {
        tuning: None,
        // Another output layout is another set of ports: the processor is replaced, not adopted
        no_adopt: &["multiOutput"],
        voices: 16,
        max_level: 1.0e3,
        tail: lm_plugin::DEFAULT_TAIL,
    },
};

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:avldrums", 1)
            .field(FieldSpec::choice("kit", &KIT_KEYS, KITS[0].key))
            .field(FieldSpec::new("multiOutput", FieldKind::Bool).default(Value::bool(false))),
    ]
}

/// Whether a state plays with an output per group
fn multi(state: &Value) -> bool {
    let kit = state.get("kit").and_then(Value::as_text).and_then(Kit::from_key).unwrap_or(&KITS[0]);
    let on = state.get("multiOutput").and_then(Value::as_bool).unwrap_or(false);
    on && kit.multi_output
}

fn ports(state: &Value) -> Ports {
    let outputs =
        if multi(state) { GROUPS.iter().map(|(k, c)| Port::new(k, *c)).collect() } else { vec![Port::new("main", 2)] };
    Ports { outputs, sidechains: Vec::new() }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let kit = s.choice("kit", Kit::from_key)?;
    let multi = s.boolean("multiOutput")? && kit.multi_output;
    let rate = cx.spec.sample_rate;
    let (synth_rate, _) =
        synth_rate(rate).ok_or_else(|| Error::new("plugin.avldrums.rate").arg("rate", i64::from(rate)))?;
    let dir = kits_dir()
        .ok_or_else(|| Error::new("plugin.avldrums.no_kits").arg("folders", format!("{:?}", kit_folders())))?;
    let path = dir.join(kit.file);
    let groups = if multi { GROUPS.len() } else { 1 };
    let config = SynthConfig {
        sample_rate: f64::from(synth_rate),
        outputs: groups,
        midi_channels: 16,
        polyphony: POLYPHONY,
        gain: 1.0,
    };
    let mut b = SynthBuilder::new(config)?;
    let font = b.load_soundfont(&path).map_err(|e| e.arg("kit", kit.name))?;
    // Every MIDI channel plays the kit's preset (its first)
    let preset = b.presets(font)?.into_iter().next().ok_or_else(|| {
        Error::new("plugin.avldrums.empty_kit").arg("kit", kit.name).arg("path", path.display().to_string())
    })?;
    for channel in 0..16 {
        b.program_select(channel, font, preset.bank, preset.program)?;
    }
    if multi {
        // The mono pieces panned hard left: the left channel of their group has all of them
        for channel in 0..MONO_GROUPS {
            b.cc(channel, 10, 0)?;
            b.cc(channel, 42, 0)?;
        }
    }
    for channel in 0..PERCUSSION {
        b.set_channel_type(channel, ChannelType::Drum)?;
    }
    let synth = b.build();
    // The outputs: the stereo pair of each group, the right of a mono group rendered and dropped
    let channels = 2 * groups;
    Ok(Box::new(AvlDrums {
        current: Some(Kitset { synth, renderer: Renderer::new(rate, channels, cx.spec.max_block), frames_rendered: 0 }),
        ghost: None,
        ghost_scratch: vec![vec![0.0; cx.spec.max_block]; channels].into_boxed_slice(),
        ghost_quiet: 0,
        ghost_quiet_max: rate / 10,
        multi,
        drop: vec![vec![0.0; cx.spec.max_block]; MONO_GROUPS].into_boxed_slice(),
        hits: [(0, 0); PIECES],
        hits_changed: false,
        kit: kit.key,
    }))
}

/// A kit loaded: its synthesizer and how it reaches the project's rate
struct Kitset {
    synth: Synth,
    renderer: Renderer,
    /// The synthesizer's frames since it was made or reset (FluidSynth renders in blocks: see `reset`)
    frames_rendered: u64,
}

impl Kitset {
    /// Renders into `outs` (the stereo pair of each output, in order)
    fn render(&mut self, outs: &mut [&mut [f32]]) {
        let Self { synth, renderer, frames_rendered } = self;
        renderer.render(outs, |bufs| {
            *frames_rendered += bufs.first().map_or(0, |b| b.len()) as u64;
            if synth.outputs() == 1 {
                let (l, r) = bufs.split_at_mut(1);
                let _ = synth.render_stereo(l[0], r[0]);
            } else {
                let _ = synth.render(bufs);
            }
        });
    }

    /// Every voice off at once, channel by channel, and what FluidSynth had rendered ahead of the last block dropped,
    /// so that the next render starts on a block of its own (the legacy `reset`)
    fn reset(&mut self) {
        for channel in 0..16 {
            let _ = self.synth.all_sounds_off(Some(channel));
        }
        let block = self.synth.internal_block() as u64;
        let mut rest = ((block - self.frames_rendered % block) % block) as usize;
        let mut scratch = [[0.0f32; 64]; 2 * GROUPS.len()];
        let outputs = self.synth.outputs();
        while rest > 0 {
            let n = rest.min(64);
            let mut it = scratch.iter_mut();
            let mut refs: [&mut [f32]; 2 * GROUPS.len()] =
                std::array::from_fn(|_| it.next().map(|b| &mut b[..n]).unwrap_or_default());
            if outputs == 1 {
                let (l, r) = refs.split_at_mut(1);
                let _ = self.synth.render_stereo(l[0], r[0]);
            } else {
                let _ = self.synth.render(&mut refs[..2 * outputs]);
            }
            rest -= n;
        }
        self.frames_rendered = 0;
        self.renderer.reset();
    }
}

/// The processor
pub struct AvlDrums {
    current: Option<Kitset>,
    /// The kit before a change, playing out what it was playing (U32)
    ghost: Option<Kitset>,
    ghost_scratch: Box<[Vec<f32>]>,
    /// Frames the ghost has been silent
    ghost_quiet: u32,
    ghost_quiet_max: u32,
    multi: bool,
    /// Where the right sides of the mono groups go
    drop: Box<[Vec<f32>]>,
    /// Per piece, a count of hits (wrapping) and the velocity of the last
    hits: [(u8, u8); PIECES],
    hits_changed: bool,
    kit: &'static str,
}

impl std::fmt::Debug for AvlDrums {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvlDrums").field("kit", &self.kit).field("multi", &self.multi).finish_non_exhaustive()
    }
}

/// The MIDI velocity of a note's (0 to 1, linear), as the legacy instrument rounded it
pub fn midi_velocity(v: f32) -> u8 {
    (f64::from(v) * 127.0 + 0.5).clamp(1.0, 127.0) as u8
}

impl AvlDrums {
    fn event(&mut self, kind: EventKind) {
        let Some(s) = self.current.as_mut().map(|k| &mut k.synth) else { return };
        match kind {
            EventKind::NoteOn { key, velocity, .. } => {
                let velocity = midi_velocity(velocity);
                let _ = s.note_on(group_of(key), key, velocity);
                if let Some(piece) = key.checked_sub(FIRST_KEY).map(usize::from).filter(|p| *p < PIECES) {
                    let h = &mut self.hits[piece];
                    *h = (h.0.wrapping_add(1), velocity);
                    self.hits_changed = true;
                }
            }
            EventKind::NoteOff { key, .. } => {
                let _ = s.note_off(group_of(key), key);
            }
            EventKind::Midi { bytes } => {
                // Only control changes, and not the ones that would move the kit's panning or hold notes
                if bytes[0] & 0xF0 == 0xB0 && !matches!(bytes[1], 10 | 42 | 64 | 66 | 68) {
                    for channel in 0..GROUPS.len() {
                        let _ = s.cc(channel, bytes[1] & 0x7F, bytes[2] & 0x7F);
                    }
                }
            }
            EventKind::AllNotesRelease => {
                for channel in 0..GROUPS.len() {
                    let _ = s.all_notes_off(Some(channel));
                }
            }
            EventKind::AllSoundOff => {
                for channel in 0..GROUPS.len() {
                    let _ = s.all_sounds_off(Some(channel));
                }
            }
            EventKind::Param { .. } | EventKind::NoteExpression { .. } => {}
        }
    }

    /// Renders `n` frames from `at` into the outputs
    fn render(&mut self, outs: &mut lm_render::ChannelsMut<'_>, at: usize, n: usize) {
        if n == 0 {
            return;
        }
        let Self { current, ghost, ghost_scratch, ghost_quiet, ghost_quiet_max, multi, drop, .. } = self;
        let mut refs: [&mut [f32]; 2 * GROUPS.len()] = Default::default();
        let count = if *multi {
            let mut out = outs.iter_mut();
            let mut drops = drop.iter_mut();
            for (g, (_, channels)) in GROUPS.iter().enumerate() {
                refs[2 * g] = out.next().map(|o| &mut o[at..at + n]).unwrap_or_default();
                refs[2 * g + 1] = if *channels == 2 {
                    out.next().map(|o| &mut o[at..at + n]).unwrap_or_default()
                } else {
                    drops.next().map(|d| &mut d[..n]).unwrap_or_default()
                };
            }
            2 * GROUPS.len()
        } else {
            let (l, r) = outs.pair_mut(0, 1);
            refs[0] = &mut l[at..at + n];
            refs[1] = &mut r[at..at + n];
            2
        };
        match current.as_mut() {
            Some(k) => k.render(&mut refs[..count]),
            None => refs[..count].iter_mut().for_each(|r| r.fill(0.0)),
        }
        if let Some(g) = ghost.as_mut()
            && *ghost_quiet < *ghost_quiet_max
        {
            let mut it = ghost_scratch.iter_mut();
            let mut grefs: [&mut [f32]; 2 * GROUPS.len()] =
                std::array::from_fn(|_| it.next().map(|b| &mut b[..n]).unwrap_or_default());
            g.render(&mut grefs[..count]);
            let mut peak = 0.0f32;
            for (o, x) in refs[..count].iter_mut().zip(grefs.iter()) {
                for (a, b) in o.iter_mut().zip(x.iter()) {
                    *a += *b;
                    peak = peak.max(b.abs());
                }
            }
            if peak > 1.0e-7 {
                *ghost_quiet = 0;
            } else {
                *ghost_quiet += n as u32;
            }
        }
    }
}

impl Processor for AvlDrums {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
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
        if self.hits_changed
            && let Some(d) = cx.display.as_deref_mut()
        {
            let hits = &self.hits;
            d.write(|frame| {
                for (i, (count, velocity)) in hits.iter().enumerate() {
                    if let Some(b) = frame.get_mut(8 * i..8 * i + 8) {
                        b[..4].copy_from_slice(&f32::from(*count).to_le_bytes());
                        b[4..].copy_from_slice(&f32::from(*velocity).to_le_bytes());
                    }
                }
            });
            self.hits_changed = false;
        }
    }

    /// Back to rest: nothing sounds, the old kit's tail included, and nothing of what played is kept
    fn reset(&mut self) {
        if let Some(k) = self.current.as_mut() {
            k.reset();
        }
        if let Some(g) = self.ghost.as_mut() {
            g.reset();
        }
        self.ghost_quiet = self.ghost_quiet_max;
    }

    /// The same kit takes the old synthesizer over (its voices go on, bit for bit); another kit cannot take a
    /// FluidSynth's voices, so the old kit becomes the ghost that plays out what it was playing, mixed with the new
    /// one (U32). Another output layout is not adopted (other ports)
    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        if old.multi != self.multi {
            return Adopted::No;
        }
        if old.kit == self.kit {
            std::mem::swap(&mut self.current, &mut old.current);
            std::mem::swap(&mut self.ghost, &mut old.ghost);
            self.ghost_quiet = old.ghost_quiet;
        } else {
            self.ghost = old.current.take();
            self.ghost_quiet = 0;
        }
        self.hits = old.hits;
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
