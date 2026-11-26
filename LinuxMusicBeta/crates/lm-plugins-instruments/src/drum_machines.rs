//! DrumMachines (F-INS: legacy `plugins/DrumMachines`): the classic drum machines (CR-78, TR-808, TR-909, LinnDrum...)
//! on the General MIDI drum map, each kit's pieces synthesized from DrumSynth files or recordings brought to the
//! engine's rate, played to their end at the note's velocity (a note-off does not stop a drum), the hi-hats choking
//! one another.
//!
//! State, version 1, no parameters (the legacy instrument had none): field `kit`, a [`Kit`] key (`tr808`).
//!
//! **The kits are the program's data, not the project's** (as AvlDrums'): the state names the kit by its key and the
//! build reads its pieces from the samples folder ([`samples_dir`]): `.ds` files rendered by `lm_dsp::drumsynth` at the
//! engine's rate (its noise from the legacy generator seeded with 1, a piece after another), `.flac` recordings
//! converted to it by libsamplerate's best sinc, as the legacy `loadKit`; a piece that cannot be read stays silent.
//! The build runs off the audio thread. A change of kit is not adopted (the voices of the old kit play out in a tail
//! node, if the install names one); the same kit keeps its voices.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use lm_base::{Cancel, Error, Progress, Result};
use lm_dsp::drum_machines::{DrumKit, DrumMachine, DrumSound};
use lm_dsp::drumsynth;
use lm_dsp::math::LegacyRand;
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Adopted, Event, EventKind, ProcessContext, Processor};
use lm_state::{FieldSpec, NodeSchema, Value};
use lm_sys_samplerate::Quality;
use lm_sys_sndfile::Reader;

use crate::stereo_out;

pub const ID: &str = "drummachines";

/// A piece of a kit: its file in the kit's folder, its General MIDI note and its choke group (0: none)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub file: &'static str,
    pub note: u8,
    pub choke: i32,
}

/// A kit
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kit {
    /// The permanent key in the state
    pub key: &'static str,
    /// Its name (the machines' own names, not translated)
    pub name: &'static str,
    /// Its folder in the samples folder
    pub folder: &'static str,
    pub pieces: &'static [Piece],
}

impl Kit {
    pub fn from_key(key: &str) -> Option<&'static Kit> {
        KITS.iter().find(|k| k.key == key)
    }
}

/// The kits, in the legacy order (the legacy `DrumMachinesInstrument::kits`)
pub const KITS: [Kit; 13] = [
    Kit {
        key: "cr78",
        name: "CR-78",
        folder: "drummachines/cr78",
        pieces: &[
            Piece { file: "kick.flac", note: 36, choke: 0 },
            Piece { file: "snare.flac", note: 38, choke: 0 },
            Piece { file: "rim.flac", note: 37, choke: 0 },
            Piece { file: "hihat.flac", note: 42, choke: 1 },
            Piece { file: "cymbal.flac", note: 49, choke: 0 },
            Piece { file: "bongo_high.flac", note: 60, choke: 0 },
            Piece { file: "bongo_low.flac", note: 61, choke: 0 },
            Piece { file: "conga.flac", note: 63, choke: 0 },
            Piece { file: "clave.flac", note: 75, choke: 0 },
            Piece { file: "maracas.flac", note: 70, choke: 0 },
            Piece { file: "tambourine.flac", note: 54, choke: 0 },
        ],
    },
    Kit {
        key: "cr8000",
        name: "CR-8000",
        folder: "drumsynth/cr8000",
        pieces: &[
            Piece { file: "Kick.ds", note: 36, choke: 0 },
            Piece { file: "Snare.ds", note: 38, choke: 0 },
            Piece { file: "Rim.ds", note: 37, choke: 0 },
            Piece { file: "Clap.ds", note: 39, choke: 0 },
            Piece { file: "Hat_c.ds", note: 42, choke: 1 },
            Piece { file: "Hat_o.ds", note: 46, choke: 1 },
            Piece { file: "Cymbal.ds", note: 49, choke: 0 },
            Piece { file: "Cowbell.ds", note: 56, choke: 0 },
            Piece { file: "Clave.ds", note: 75, choke: 0 },
            Piece { file: "Conga_m.ds", note: 63, choke: 0 },
            Piece { file: "Conga_l.ds", note: 64, choke: 0 },
        ],
    },
    Kit {
        key: "tr606",
        name: "TR-606",
        folder: "drumsynth/tr606",
        pieces: &[
            Piece { file: "TR606_Kick.ds", note: 36, choke: 0 },
            Piece { file: "TR606_Snare.ds", note: 38, choke: 0 },
            Piece { file: "TR606_Hat_C.ds", note: 42, choke: 1 },
            Piece { file: "TR606_Hat_O.ds", note: 46, choke: 1 },
            Piece { file: "TR606_Cymbal.ds", note: 49, choke: 0 },
            Piece { file: "TR606_Tom_L.ds", note: 45, choke: 0 },
            Piece { file: "TR606_Tom_H.ds", note: 50, choke: 0 },
        ],
    },
    Kit {
        key: "tr77",
        name: "TR-77",
        folder: "drumsynth/tr77",
        pieces: &[
            Piece { file: "Tr77_kick.ds", note: 36, choke: 0 },
            Piece { file: "Tr77_snare.ds", note: 38, choke: 0 },
            Piece { file: "Tr77_hat.ds", note: 42, choke: 1 },
            Piece { file: "Tr77_cym.ds", note: 49, choke: 0 },
            Piece { file: "Tr77_tom1.ds", note: 50, choke: 0 },
            Piece { file: "Tr77_tom2.ds", note: 45, choke: 0 },
            Piece { file: "Tr77_cui1.ds", note: 78, choke: 0 },
            Piece { file: "Tr77_cui2.ds", note: 79, choke: 0 },
        ],
    },
    Kit {
        key: "tr808",
        name: "TR-808",
        folder: "drumsynth/tr808",
        pieces: &[
            Piece { file: "Kick.ds", note: 36, choke: 0 },
            Piece { file: "Kickhard.ds", note: 35, choke: 0 },
            Piece { file: "Rimshot.ds", note: 37, choke: 0 },
            Piece { file: "Snare.ds", note: 38, choke: 0 },
            Piece { file: "Handclap.ds", note: 39, choke: 0 },
            Piece { file: "Snr_snap.ds", note: 40, choke: 0 },
            Piece { file: "Hat_c.ds", note: 42, choke: 1 },
            Piece { file: "Hat_o.ds", note: 46, choke: 1 },
            Piece { file: "Tom_lo.ds", note: 45, choke: 0 },
            Piece { file: "Tom_mid.ds", note: 47, choke: 0 },
            Piece { file: "Tom_hi.ds", note: 50, choke: 0 },
            Piece { file: "Ride.ds", note: 51, choke: 0 },
            Piece { file: "Cowbell.ds", note: 56, choke: 0 },
            Piece { file: "Clave.ds", note: 75, choke: 0 },
            Piece { file: "Kicklong.ds", note: 82, choke: 0 },
        ],
    },
    Kit {
        key: "tr909",
        name: "TR-909",
        folder: "drumsynth/tr909",
        pieces: &[
            Piece { file: "Kick.ds", note: 36, choke: 0 },
            Piece { file: "Kick_1.ds", note: 35, choke: 0 },
            Piece { file: "Snare.ds", note: 38, choke: 0 },
            Piece { file: "Snare2.ds", note: 40, choke: 0 },
            Piece { file: "Clap.ds", note: 39, choke: 0 },
            Piece { file: "Hat-c.ds", note: 42, choke: 1 },
            Piece { file: "Hat-c2.ds", note: 44, choke: 1 },
            Piece { file: "Hat-o.ds", note: 46, choke: 1 },
            Piece { file: "Hat-o2.ds", note: 82, choke: 1 },
            Piece { file: "Kick_2.ds", note: 83, choke: 0 },
            Piece { file: "Kick_3.ds", note: 84, choke: 0 },
            Piece { file: "TR909_Kick.ds", note: 85, choke: 0 },
            Piece { file: "TR909_Clap.ds", note: 86, choke: 0 },
        ],
    },
    Kit {
        key: "linn",
        name: "LinnDrum",
        folder: "drumsynth/linn",
        pieces: &[
            Piece { file: "linn_kik.ds", note: 36, choke: 0 },
            Piece { file: "linn_stk.ds", note: 37, choke: 0 },
            Piece { file: "linn_snr.ds", note: 38, choke: 0 },
            Piece { file: "linn_clp.ds", note: 39, choke: 0 },
            Piece { file: "linn_tom5.ds", note: 43, choke: 0 },
            Piece { file: "linn_tom4.ds", note: 45, choke: 0 },
            Piece { file: "linn_tom3.ds", note: 47, choke: 0 },
            Piece { file: "linn_tom2.ds", note: 48, choke: 0 },
            Piece { file: "linn_tom1.ds", note: 50, choke: 0 },
            Piece { file: "linn_tamb.ds", note: 54, choke: 0 },
            Piece { file: "linn_cow.ds", note: 56, choke: 0 },
            Piece { file: "linn_cgam.ds", note: 62, choke: 0 },
            Piece { file: "linn_cga1.ds", note: 63, choke: 0 },
            Piece { file: "linn_cga2.ds", note: 64, choke: 0 },
            Piece { file: "linn_cab.ds", note: 69, choke: 0 },
            Piece { file: "linn_cga3.ds", note: 82, choke: 0 },
        ],
    },
    Kit {
        key: "electro",
        name: "Electro",
        folder: "drumsynth/electro",
        pieces: &[
            Piece { file: "K_Linn.ds", note: 36, choke: 0 },
            Piece { file: "K_8bit.ds", note: 35, choke: 0 },
            Piece { file: "S_Linn.ds", note: 38, choke: 0 },
            Piece { file: "S_8bit.ds", note: 40, choke: 0 },
            Piece { file: "Boom.ds", note: 41, choke: 0 },
            Piece { file: "H_closed.ds", note: 42, choke: 1 },
            Piece { file: "H_open.ds", note: 46, choke: 1 },
            Piece { file: "Syntom_2.ds", note: 45, choke: 0 },
            Piece { file: "Syntom_1.ds", note: 50, choke: 0 },
            Piece { file: "K_reverb.ds", note: 82, choke: 0 },
            Piece { file: "S_reverb.ds", note: 83, choke: 0 },
            Piece { file: "s_eq.ds", note: 84, choke: 0 },
            Piece { file: "Hi-q.ds", note: 85, choke: 0 },
            Piece { file: "Hi-q_2.ds", note: 86, choke: 0 },
        ],
    },
    Kit {
        key: "rnb",
        name: "R&B",
        folder: "drumsynth/r_n_b",
        pieces: &[
            Piece { file: "k_hard.ds", note: 36, choke: 0 },
            Piece { file: "k_soft.ds", note: 35, choke: 0 },
            Piece { file: "popsnare.ds", note: 38, choke: 0 },
            Piece { file: "popclap.ds", note: 39, choke: 0 },
            Piece { file: "S_hollow.ds", note: 40, choke: 0 },
            Piece { file: "hat.ds", note: 42, choke: 1 },
            Piece { file: "Hat_O.ds", note: 46, choke: 1 },
            Piece { file: "k_muffle.ds", note: 82, choke: 0 },
            Piece { file: "old_kik.ds", note: 83, choke: 0 },
            Piece { file: "clapfilt.ds", note: 84, choke: 0 },
            Piece { file: "s_pitch.ds", note: 85, choke: 0 },
            Piece { file: "s_spit.ds", note: 86, choke: 0 },
            Piece { file: "snare&hat.ds", note: 87, choke: 0 },
            Piece { file: "x_backwd.ds", note: 88, choke: 0 },
            Piece { file: "hat_rattle.ds", note: 89, choke: 1 },
        ],
    },
    Kit {
        key: "sp1200",
        name: "SP-1200 style",
        folder: "drummachines/sp1200",
        pieces: &[
            Piece { file: "kick.flac", note: 36, choke: 0 },
            Piece { file: "kick2.flac", note: 35, choke: 0 },
            Piece { file: "rim.flac", note: 37, choke: 0 },
            Piece { file: "snare.flac", note: 38, choke: 0 },
            Piece { file: "clap.flac", note: 39, choke: 0 },
            Piece { file: "snare2.flac", note: 40, choke: 0 },
            Piece { file: "hihat_closed.flac", note: 42, choke: 1 },
            Piece { file: "hihat_pedal.flac", note: 44, choke: 1 },
            Piece { file: "hihat_open.flac", note: 46, choke: 1 },
            Piece { file: "shaker.flac", note: 70, choke: 0 },
            Piece { file: "wood.flac", note: 76, choke: 0 },
            Piece { file: "kick_low.flac", note: 82, choke: 0 },
            Piece { file: "snare_low.flac", note: 83, choke: 0 },
        ],
    },
    Kit {
        key: "mpc60",
        name: "MPC60 style",
        folder: "drummachines/mpc60",
        pieces: &[
            Piece { file: "kick.flac", note: 36, choke: 0 },
            Piece { file: "kick2.flac", note: 35, choke: 0 },
            Piece { file: "rim.flac", note: 37, choke: 0 },
            Piece { file: "snare.flac", note: 38, choke: 0 },
            Piece { file: "clap.flac", note: 39, choke: 0 },
            Piece { file: "snare2.flac", note: 40, choke: 0 },
            Piece { file: "hihat_closed.flac", note: 42, choke: 1 },
            Piece { file: "hihat_pedal.flac", note: 44, choke: 1 },
            Piece { file: "hihat_open.flac", note: 46, choke: 1 },
            Piece { file: "cabasa.flac", note: 69, choke: 0 },
            Piece { file: "shaker.flac", note: 70, choke: 0 },
            Piece { file: "snare3.flac", note: 82, choke: 0 },
            Piece { file: "kick3.flac", note: 83, choke: 0 },
        ],
    },
    Kit {
        key: "dmx",
        name: "DMX",
        folder: "drumsynth/dmx",
        pieces: &[
            Piece { file: "bass1.ds", note: 36, choke: 0 },
            Piece { file: "bass2.ds", note: 35, choke: 0 },
            Piece { file: "rimshot.ds", note: 37, choke: 0 },
            Piece { file: "snare1.ds", note: 38, choke: 0 },
            Piece { file: "claps.ds", note: 39, choke: 0 },
            Piece { file: "snare2.ds", note: 40, choke: 0 },
            Piece { file: "tom6.ds", note: 41, choke: 0 },
            Piece { file: "hihat_closed.ds", note: 42, choke: 1 },
            Piece { file: "tom5.ds", note: 43, choke: 0 },
            Piece { file: "hihat_accent.ds", note: 44, choke: 1 },
            Piece { file: "tom4.ds", note: 45, choke: 0 },
            Piece { file: "hihat_open.ds", note: 46, choke: 1 },
            Piece { file: "tom3.ds", note: 47, choke: 0 },
            Piece { file: "tom2.ds", note: 48, choke: 0 },
            Piece { file: "crash.ds", note: 49, choke: 0 },
            Piece { file: "tom1.ds", note: 50, choke: 0 },
            Piece { file: "ride.ds", note: 51, choke: 0 },
            Piece { file: "tamb.ds", note: 54, choke: 0 },
            Piece { file: "shaker.ds", note: 70, choke: 0 },
            Piece { file: "bass3.ds", note: 82, choke: 0 },
            Piece { file: "snare3.ds", note: 83, choke: 0 },
        ],
    },
    Kit {
        key: "tr707",
        name: "TR-707",
        folder: "drumsynth/tr707",
        pieces: &[
            Piece { file: "bass1.ds", note: 36, choke: 0 },
            Piece { file: "bass2.ds", note: 35, choke: 0 },
            Piece { file: "rimshot.ds", note: 37, choke: 0 },
            Piece { file: "snare1.ds", note: 38, choke: 0 },
            Piece { file: "handclap.ds", note: 39, choke: 0 },
            Piece { file: "snare2.ds", note: 40, choke: 0 },
            Piece { file: "hihat_closed.ds", note: 42, choke: 1 },
            Piece { file: "tom_low.ds", note: 45, choke: 0 },
            Piece { file: "hihat_open.ds", note: 46, choke: 1 },
            Piece { file: "tom_mid.ds", note: 47, choke: 0 },
            Piece { file: "crash.ds", note: 49, choke: 0 },
            Piece { file: "tom_high.ds", note: 50, choke: 0 },
            Piece { file: "ride.ds", note: 51, choke: 0 },
            Piece { file: "tambourine.ds", note: 54, choke: 0 },
            Piece { file: "cowbell.ds", note: 56, choke: 0 },
        ],
    },
];

/// The keys of the kits
pub const KIT_KEYS: [&str; 13] =
    ["cr78", "cr8000", "tr606", "tr77", "tr808", "tr909", "linn", "electro", "rnb", "sp1200", "mpc60", "dmx", "tr707"];
static KEYS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| KIT_KEYS.into());
/// The kit a new instrument has (the legacy: the TR-808 first)
pub const DEFAULT_KIT: &str = "tr808";

/// The folder environment variable that names the samples folder (packaging, tests)
pub const SAMPLES_DIR_VAR: &str = "LM_SAMPLES_DIR";

/// The folders the samples are looked for in, in order: `$LM_SAMPLES_DIR`; `$LM_DATA_DIR/samples`; next to the
/// program, `../share/linuxmusic/samples` (an installation) and `data/samples` (a portable copy); and the legacy
/// tree's `legado/data/samples` of this source tree, or of `$LEGACY_ROOT` (development)
pub fn samples_folders() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os(SAMPLES_DIR_VAR) {
        v.push(PathBuf::from(d));
    }
    if let Some(d) = std::env::var_os("LM_DATA_DIR") {
        v.push(Path::new(&d).join("samples"));
    }
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(exe.join("../share/linuxmusic/samples"));
        v.push(exe.join("data/samples"));
    }
    v.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../legado/data/samples"));
    if let Some(d) = std::env::var_os("LEGACY_ROOT") {
        v.push(Path::new(&d).join("legado/data/samples"));
    }
    v
}

/// The samples folder: the first of [`samples_folders`] that has the first kit's folder
pub fn samples_dir() -> Option<PathBuf> {
    samples_folders().into_iter().find(|d| d.join(KITS[0].folder).is_dir())
}

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.drummachines.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: a drum has no pitch; a note plays its piece to the end, so the tail is the longest piece)
    tests: TestHints { voices: lm_dsp::drum_machines::VOICES, no_adopt: &["kit"], tail: 8.0, ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    vec![NodeSchema::new("plugin:drummachines", 1).field(FieldSpec::choice("kit", &KEYS, DEFAULT_KIT))]
}

/// A DrumSynth piece at @p rate: its mono frames as stereo (the legacy decoder's 16 bits over 32768)
pub fn synthesize(text: &[u8], rate: u32, rand: &mut LegacyRand) -> Vec<[f32; 2]> {
    drumsynth::render(text, rate, rand)
        .into_iter()
        .map(|s| {
            let v = f32::from(s) / 32768.0;
            [v, v]
        })
        .collect()
}

/// A recording at @p rate: the file's frames as stereo (a mono file on both sides), converted to @p rate by the best
/// sinc; the legacy's `src_simple` of the whole piece
pub fn recording(path: &Path, rate: u32) -> Result<Vec<[f32; 2]>> {
    let mut reader = Reader::open(path)?;
    let info = *reader.info();
    let channels = info.channels;
    let mut data = vec![0.0f32; usize::try_from(info.frames).unwrap_or(0) * channels];
    let mut at = 0;
    while at < data.len() {
        let got = reader.read_f32(&mut data[at..])?;
        if got == 0 {
            break;
        }
        at += got * channels;
    }
    data.truncate(at);
    let data = if info.rate == rate {
        data
    } else {
        lm_sys_samplerate::convert(&data, channels, info.rate, rate, Quality::Best, &Cancel::new(), &Progress::new())?
    };
    Ok(data.chunks_exact(channels).map(|f| if channels == 1 { [f[0], f[0]] } else { [f[0], f[1]] }).collect())
}

/// The sounds of a kit from the samples folder @p dir at @p rate; a piece that cannot be read is silent
pub fn load_kit(kit: &Kit, dir: &Path, rate: u32) -> DrumKit {
    let folder = dir.join(kit.folder);
    // (one generator for the kit's pieces, as the legacy decoder's per-thread one)
    let mut rand = LegacyRand::new(1);
    let sounds = kit
        .pieces
        .iter()
        .map(|piece| {
            let path = folder.join(piece.file);
            let frames = if piece.file.ends_with(".ds") {
                std::fs::read(&path).map(|text| synthesize(&text, rate, &mut rand)).unwrap_or_default()
            } else {
                recording(&path, rate).unwrap_or_default()
            };
            DrumSound { frames, note: usize::from(piece.note), choke: piece.choke }
        })
        .collect();
    DrumKit::new(sounds)
}

/// The processor
pub struct DrumMachines {
    machine: DrumMachine,
    kit: Arc<DrumKit>,
    key: &'static str,
    rate: u32,
    scratch: Vec<[f32; 2]>,
}

impl std::fmt::Debug for DrumMachines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrumMachines").field("kit", &self.key).finish_non_exhaustive()
    }
}

impl DrumMachines {
    /// A machine playing @p kit (made off the audio thread)
    pub fn new(key: &'static str, kit: Arc<DrumKit>, rate: u32, max_block: usize) -> Self {
        Self { machine: DrumMachine::new(rate), kit, key, rate, scratch: vec![[0.0; 2]; max_block] }
    }

    fn event(&mut self, kind: EventKind) {
        match kind {
            EventKind::NoteOn { key, velocity, .. } => {
                let _ = self.machine.note_on(&self.kit, i32::from(key), velocity.clamp(0.0, 1.0));
            }
            // (a drum plays to its end: note-offs and the release do not stop it)
            EventKind::AllSoundOff => self.machine.silence(),
            _ => {}
        }
    }

    fn render(&mut self, outs: &mut lm_render::ChannelsMut<'_>, at: usize, n: usize) {
        if n == 0 {
            return;
        }
        let block = &mut self.scratch[..n];
        block.fill([0.0; 2]);
        self.machine.render(&self.kit, block);
        let (l, r) = outs.pair_mut(0, 1);
        for (i, f) in block.iter().enumerate() {
            l[at + i] += f[0];
            r[at + i] += f[1];
        }
    }
}

impl Processor for DrumMachines {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames.min(self.scratch.len());
        let events: &[Event] = cx.events;
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
    }

    fn reset(&mut self) {
        self.machine.silence();
    }

    /// The same kit at the same rate keeps its voices
    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        if old.key != self.key || old.rate != self.rate {
            return Adopted::No;
        }
        std::mem::swap(&mut self.machine, &mut old.machine);
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let kit = s.choice("kit", Kit::from_key)?;
    let dir = samples_dir().ok_or_else(|| {
        Error::new("plugin.drummachines.no_samples").arg("folders", format!("{:?}", samples_folders()))
    })?;
    let rate = cx.spec.sample_rate;
    let loaded = Arc::new(load_kit(kit, &dir, rate));
    Ok(Box::new(DrumMachines::new(kit.key, loaded, rate, cx.spec.max_block as usize)))
}
