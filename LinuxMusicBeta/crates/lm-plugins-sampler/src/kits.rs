//! The kits the program ships as data (not the project's): the AVL drum kits (SoundFonts) and the DrumMachines'
//! kits (`.ds` files for DrumSynth and `.flac` recordings), with their folders. The tables are the legacy plugins'
//! (`avldrums`, `drummachines`), kept here so the Sampler does not depend on them when they are removed.

use std::path::{Path, PathBuf};

/// A kit
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvlKit {
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
pub const AVL_KITS: [AvlKit; 5] = [
    AvlKit {
        key: "black-pearl",
        name: "Black Pearl",
        file: "Black_Pearl_4_LV2.sf2",
        photo: "black_pearl.png",
        map: "black_pearl.map.png",
        multi_output: true,
    },
    AvlKit {
        key: "red-zeppelin",
        name: "Red Zeppelin",
        file: "Red_Zeppelin_4_LV2.sf2",
        photo: "red_zeppelin.png",
        map: "red_zeppelin.map.png",
        multi_output: true,
    },
    AvlKit {
        key: "blonde-bop",
        name: "Blonde Bop",
        file: "Blonde_Bop_LV2.sf2",
        photo: "blonde_bop.png",
        map: "blonde_bop.map.png",
        multi_output: true,
    },
    AvlKit {
        key: "blonde-bop-hot-rod",
        name: "Blonde Bop Hot Rod",
        file: "Blonde_Bop_HR_LV2.sf2",
        photo: "blonde_bop_hr.png",
        map: "blonde_bop.map.png",
        multi_output: true,
    },
    AvlKit {
        key: "buskmans-holiday",
        name: "Buskman's Holiday (percussion)",
        file: "Buskmans_Holiday_LV2.sf2",
        photo: "buskman.png",
        map: "buskman.map.png",
        multi_output: false,
    },
];

impl AvlKit {
    pub fn from_key(key: &str) -> Option<&'static AvlKit> {
        AVL_KITS.iter().find(|k| k.key == key)
    }
}

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
    kit_folders().into_iter().find(|d| d.join(AVL_KITS[0].file).is_file())
}

/// A piece of a kit: its file in the kit's folder, its General MIDI note and its choke group (0: none)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmPiece {
    pub file: &'static str,
    pub note: u8,
    pub choke: i32,
}

/// A kit
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmKit {
    /// The permanent key in the state
    pub key: &'static str,
    /// Its name (the machines' own names, not translated)
    pub name: &'static str,
    /// Its folder in the samples folder
    pub folder: &'static str,
    pub pieces: &'static [DmPiece],
}

impl DmKit {
    pub fn from_key(key: &str) -> Option<&'static DmKit> {
        DM_KITS.iter().find(|k| k.key == key)
    }
}

/// The kits, in the legacy order (the legacy `DrumMachinesInstrument::kits`)
pub const DM_KITS: [DmKit; 13] = [
    DmKit {
        key: "cr78",
        name: "CR-78",
        folder: "drummachines/cr78",
        pieces: &[
            DmPiece { file: "kick.flac", note: 36, choke: 0 },
            DmPiece { file: "snare.flac", note: 38, choke: 0 },
            DmPiece { file: "rim.flac", note: 37, choke: 0 },
            DmPiece { file: "hihat.flac", note: 42, choke: 1 },
            DmPiece { file: "cymbal.flac", note: 49, choke: 0 },
            DmPiece { file: "bongo_high.flac", note: 60, choke: 0 },
            DmPiece { file: "bongo_low.flac", note: 61, choke: 0 },
            DmPiece { file: "conga.flac", note: 63, choke: 0 },
            DmPiece { file: "clave.flac", note: 75, choke: 0 },
            DmPiece { file: "maracas.flac", note: 70, choke: 0 },
            DmPiece { file: "tambourine.flac", note: 54, choke: 0 },
        ],
    },
    DmKit {
        key: "cr8000",
        name: "CR-8000",
        folder: "drumsynth/cr8000",
        pieces: &[
            DmPiece { file: "Kick.ds", note: 36, choke: 0 },
            DmPiece { file: "Snare.ds", note: 38, choke: 0 },
            DmPiece { file: "Rim.ds", note: 37, choke: 0 },
            DmPiece { file: "Clap.ds", note: 39, choke: 0 },
            DmPiece { file: "Hat_c.ds", note: 42, choke: 1 },
            DmPiece { file: "Hat_o.ds", note: 46, choke: 1 },
            DmPiece { file: "Cymbal.ds", note: 49, choke: 0 },
            DmPiece { file: "Cowbell.ds", note: 56, choke: 0 },
            DmPiece { file: "Clave.ds", note: 75, choke: 0 },
            DmPiece { file: "Conga_m.ds", note: 63, choke: 0 },
            DmPiece { file: "Conga_l.ds", note: 64, choke: 0 },
        ],
    },
    DmKit {
        key: "tr606",
        name: "TR-606",
        folder: "drumsynth/tr606",
        pieces: &[
            DmPiece { file: "TR606_Kick.ds", note: 36, choke: 0 },
            DmPiece { file: "TR606_Snare.ds", note: 38, choke: 0 },
            DmPiece { file: "TR606_Hat_C.ds", note: 42, choke: 1 },
            DmPiece { file: "TR606_Hat_O.ds", note: 46, choke: 1 },
            DmPiece { file: "TR606_Cymbal.ds", note: 49, choke: 0 },
            DmPiece { file: "TR606_Tom_L.ds", note: 45, choke: 0 },
            DmPiece { file: "TR606_Tom_H.ds", note: 50, choke: 0 },
        ],
    },
    DmKit {
        key: "tr77",
        name: "TR-77",
        folder: "drumsynth/tr77",
        pieces: &[
            DmPiece { file: "Tr77_kick.ds", note: 36, choke: 0 },
            DmPiece { file: "Tr77_snare.ds", note: 38, choke: 0 },
            DmPiece { file: "Tr77_hat.ds", note: 42, choke: 1 },
            DmPiece { file: "Tr77_cym.ds", note: 49, choke: 0 },
            DmPiece { file: "Tr77_tom1.ds", note: 50, choke: 0 },
            DmPiece { file: "Tr77_tom2.ds", note: 45, choke: 0 },
            DmPiece { file: "Tr77_cui1.ds", note: 78, choke: 0 },
            DmPiece { file: "Tr77_cui2.ds", note: 79, choke: 0 },
        ],
    },
    DmKit {
        key: "tr808",
        name: "TR-808",
        folder: "drumsynth/tr808",
        pieces: &[
            DmPiece { file: "Kick.ds", note: 36, choke: 0 },
            DmPiece { file: "Kickhard.ds", note: 35, choke: 0 },
            DmPiece { file: "Rimshot.ds", note: 37, choke: 0 },
            DmPiece { file: "Snare.ds", note: 38, choke: 0 },
            DmPiece { file: "Handclap.ds", note: 39, choke: 0 },
            DmPiece { file: "Snr_snap.ds", note: 40, choke: 0 },
            DmPiece { file: "Hat_c.ds", note: 42, choke: 1 },
            DmPiece { file: "Hat_o.ds", note: 46, choke: 1 },
            DmPiece { file: "Tom_lo.ds", note: 45, choke: 0 },
            DmPiece { file: "Tom_mid.ds", note: 47, choke: 0 },
            DmPiece { file: "Tom_hi.ds", note: 50, choke: 0 },
            DmPiece { file: "Ride.ds", note: 51, choke: 0 },
            DmPiece { file: "Cowbell.ds", note: 56, choke: 0 },
            DmPiece { file: "Clave.ds", note: 75, choke: 0 },
            DmPiece { file: "Kicklong.ds", note: 82, choke: 0 },
        ],
    },
    DmKit {
        key: "tr909",
        name: "TR-909",
        folder: "drumsynth/tr909",
        pieces: &[
            DmPiece { file: "Kick.ds", note: 36, choke: 0 },
            DmPiece { file: "Kick_1.ds", note: 35, choke: 0 },
            DmPiece { file: "Snare.ds", note: 38, choke: 0 },
            DmPiece { file: "Snare2.ds", note: 40, choke: 0 },
            DmPiece { file: "Clap.ds", note: 39, choke: 0 },
            DmPiece { file: "Hat-c.ds", note: 42, choke: 1 },
            DmPiece { file: "Hat-c2.ds", note: 44, choke: 1 },
            DmPiece { file: "Hat-o.ds", note: 46, choke: 1 },
            DmPiece { file: "Hat-o2.ds", note: 82, choke: 1 },
            DmPiece { file: "Kick_2.ds", note: 83, choke: 0 },
            DmPiece { file: "Kick_3.ds", note: 84, choke: 0 },
            DmPiece { file: "TR909_Kick.ds", note: 85, choke: 0 },
            DmPiece { file: "TR909_Clap.ds", note: 86, choke: 0 },
        ],
    },
    DmKit {
        key: "linn",
        name: "LinnDrum",
        folder: "drumsynth/linn",
        pieces: &[
            DmPiece { file: "linn_kik.ds", note: 36, choke: 0 },
            DmPiece { file: "linn_stk.ds", note: 37, choke: 0 },
            DmPiece { file: "linn_snr.ds", note: 38, choke: 0 },
            DmPiece { file: "linn_clp.ds", note: 39, choke: 0 },
            DmPiece { file: "linn_tom5.ds", note: 43, choke: 0 },
            DmPiece { file: "linn_tom4.ds", note: 45, choke: 0 },
            DmPiece { file: "linn_tom3.ds", note: 47, choke: 0 },
            DmPiece { file: "linn_tom2.ds", note: 48, choke: 0 },
            DmPiece { file: "linn_tom1.ds", note: 50, choke: 0 },
            DmPiece { file: "linn_tamb.ds", note: 54, choke: 0 },
            DmPiece { file: "linn_cow.ds", note: 56, choke: 0 },
            DmPiece { file: "linn_cgam.ds", note: 62, choke: 0 },
            DmPiece { file: "linn_cga1.ds", note: 63, choke: 0 },
            DmPiece { file: "linn_cga2.ds", note: 64, choke: 0 },
            DmPiece { file: "linn_cab.ds", note: 69, choke: 0 },
            DmPiece { file: "linn_cga3.ds", note: 82, choke: 0 },
        ],
    },
    DmKit {
        key: "electro",
        name: "Electro",
        folder: "drumsynth/electro",
        pieces: &[
            DmPiece { file: "K_Linn.ds", note: 36, choke: 0 },
            DmPiece { file: "K_8bit.ds", note: 35, choke: 0 },
            DmPiece { file: "S_Linn.ds", note: 38, choke: 0 },
            DmPiece { file: "S_8bit.ds", note: 40, choke: 0 },
            DmPiece { file: "Boom.ds", note: 41, choke: 0 },
            DmPiece { file: "H_closed.ds", note: 42, choke: 1 },
            DmPiece { file: "H_open.ds", note: 46, choke: 1 },
            DmPiece { file: "Syntom_2.ds", note: 45, choke: 0 },
            DmPiece { file: "Syntom_1.ds", note: 50, choke: 0 },
            DmPiece { file: "K_reverb.ds", note: 82, choke: 0 },
            DmPiece { file: "S_reverb.ds", note: 83, choke: 0 },
            DmPiece { file: "s_eq.ds", note: 84, choke: 0 },
            DmPiece { file: "Hi-q.ds", note: 85, choke: 0 },
            DmPiece { file: "Hi-q_2.ds", note: 86, choke: 0 },
        ],
    },
    DmKit {
        key: "rnb",
        name: "R&B",
        folder: "drumsynth/r_n_b",
        pieces: &[
            DmPiece { file: "k_hard.ds", note: 36, choke: 0 },
            DmPiece { file: "k_soft.ds", note: 35, choke: 0 },
            DmPiece { file: "popsnare.ds", note: 38, choke: 0 },
            DmPiece { file: "popclap.ds", note: 39, choke: 0 },
            DmPiece { file: "S_hollow.ds", note: 40, choke: 0 },
            DmPiece { file: "hat.ds", note: 42, choke: 1 },
            DmPiece { file: "Hat_O.ds", note: 46, choke: 1 },
            DmPiece { file: "k_muffle.ds", note: 82, choke: 0 },
            DmPiece { file: "old_kik.ds", note: 83, choke: 0 },
            DmPiece { file: "clapfilt.ds", note: 84, choke: 0 },
            DmPiece { file: "s_pitch.ds", note: 85, choke: 0 },
            DmPiece { file: "s_spit.ds", note: 86, choke: 0 },
            DmPiece { file: "snare&hat.ds", note: 87, choke: 0 },
            DmPiece { file: "x_backwd.ds", note: 88, choke: 0 },
            DmPiece { file: "hat_rattle.ds", note: 89, choke: 1 },
        ],
    },
    DmKit {
        key: "sp1200",
        name: "SP-1200 style",
        folder: "drummachines/sp1200",
        pieces: &[
            DmPiece { file: "kick.flac", note: 36, choke: 0 },
            DmPiece { file: "kick2.flac", note: 35, choke: 0 },
            DmPiece { file: "rim.flac", note: 37, choke: 0 },
            DmPiece { file: "snare.flac", note: 38, choke: 0 },
            DmPiece { file: "clap.flac", note: 39, choke: 0 },
            DmPiece { file: "snare2.flac", note: 40, choke: 0 },
            DmPiece { file: "hihat_closed.flac", note: 42, choke: 1 },
            DmPiece { file: "hihat_pedal.flac", note: 44, choke: 1 },
            DmPiece { file: "hihat_open.flac", note: 46, choke: 1 },
            DmPiece { file: "shaker.flac", note: 70, choke: 0 },
            DmPiece { file: "wood.flac", note: 76, choke: 0 },
            DmPiece { file: "kick_low.flac", note: 82, choke: 0 },
            DmPiece { file: "snare_low.flac", note: 83, choke: 0 },
        ],
    },
    DmKit {
        key: "mpc60",
        name: "MPC60 style",
        folder: "drummachines/mpc60",
        pieces: &[
            DmPiece { file: "kick.flac", note: 36, choke: 0 },
            DmPiece { file: "kick2.flac", note: 35, choke: 0 },
            DmPiece { file: "rim.flac", note: 37, choke: 0 },
            DmPiece { file: "snare.flac", note: 38, choke: 0 },
            DmPiece { file: "clap.flac", note: 39, choke: 0 },
            DmPiece { file: "snare2.flac", note: 40, choke: 0 },
            DmPiece { file: "hihat_closed.flac", note: 42, choke: 1 },
            DmPiece { file: "hihat_pedal.flac", note: 44, choke: 1 },
            DmPiece { file: "hihat_open.flac", note: 46, choke: 1 },
            DmPiece { file: "cabasa.flac", note: 69, choke: 0 },
            DmPiece { file: "shaker.flac", note: 70, choke: 0 },
            DmPiece { file: "snare3.flac", note: 82, choke: 0 },
            DmPiece { file: "kick3.flac", note: 83, choke: 0 },
        ],
    },
    DmKit {
        key: "dmx",
        name: "DMX",
        folder: "drumsynth/dmx",
        pieces: &[
            DmPiece { file: "bass1.ds", note: 36, choke: 0 },
            DmPiece { file: "bass2.ds", note: 35, choke: 0 },
            DmPiece { file: "rimshot.ds", note: 37, choke: 0 },
            DmPiece { file: "snare1.ds", note: 38, choke: 0 },
            DmPiece { file: "claps.ds", note: 39, choke: 0 },
            DmPiece { file: "snare2.ds", note: 40, choke: 0 },
            DmPiece { file: "tom6.ds", note: 41, choke: 0 },
            DmPiece { file: "hihat_closed.ds", note: 42, choke: 1 },
            DmPiece { file: "tom5.ds", note: 43, choke: 0 },
            DmPiece { file: "hihat_accent.ds", note: 44, choke: 1 },
            DmPiece { file: "tom4.ds", note: 45, choke: 0 },
            DmPiece { file: "hihat_open.ds", note: 46, choke: 1 },
            DmPiece { file: "tom3.ds", note: 47, choke: 0 },
            DmPiece { file: "tom2.ds", note: 48, choke: 0 },
            DmPiece { file: "crash.ds", note: 49, choke: 0 },
            DmPiece { file: "tom1.ds", note: 50, choke: 0 },
            DmPiece { file: "ride.ds", note: 51, choke: 0 },
            DmPiece { file: "tamb.ds", note: 54, choke: 0 },
            DmPiece { file: "shaker.ds", note: 70, choke: 0 },
            DmPiece { file: "bass3.ds", note: 82, choke: 0 },
            DmPiece { file: "snare3.ds", note: 83, choke: 0 },
        ],
    },
    DmKit {
        key: "tr707",
        name: "TR-707",
        folder: "drumsynth/tr707",
        pieces: &[
            DmPiece { file: "bass1.ds", note: 36, choke: 0 },
            DmPiece { file: "bass2.ds", note: 35, choke: 0 },
            DmPiece { file: "rimshot.ds", note: 37, choke: 0 },
            DmPiece { file: "snare1.ds", note: 38, choke: 0 },
            DmPiece { file: "handclap.ds", note: 39, choke: 0 },
            DmPiece { file: "snare2.ds", note: 40, choke: 0 },
            DmPiece { file: "hihat_closed.ds", note: 42, choke: 1 },
            DmPiece { file: "tom_low.ds", note: 45, choke: 0 },
            DmPiece { file: "hihat_open.ds", note: 46, choke: 1 },
            DmPiece { file: "tom_mid.ds", note: 47, choke: 0 },
            DmPiece { file: "crash.ds", note: 49, choke: 0 },
            DmPiece { file: "tom_high.ds", note: 50, choke: 0 },
            DmPiece { file: "ride.ds", note: 51, choke: 0 },
            DmPiece { file: "tambourine.ds", note: 54, choke: 0 },
            DmPiece { file: "cowbell.ds", note: 56, choke: 0 },
        ],
    },
];

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
    samples_folders().into_iter().find(|d| d.join(DM_KITS[0].folder).is_dir())
}
