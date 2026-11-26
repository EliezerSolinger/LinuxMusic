//! The AVL drum kits (Glen MacArthur's, as avldrums.lv2 ships them): SoundFonts of one drum preset, whose pieces
//! avldrums.lv2 maps to MIDI keys and groups for an output each. The kit is imported from its SF2 with
//! [`crate::import::sf2`]; this adds the piece-to-output map.

use lm_base::Result;

use crate::import::sf2::{self, Sf3Decoder};
use crate::model::{Instrument, OutputGroup};

/// The groups of pieces of the multi-output mode, in output order: its name and whether the output is stereo (the
/// kick, snare, hi-hat and toms are mono)
pub const GROUPS: [(&str, bool); 7] = [
    ("kick", false),
    ("snare", false),
    ("hihat", false),
    ("tom", false),
    ("floortom", false),
    ("cymbals", true),
    ("percussion", true),
];

const PERCUSSION: u8 = 6;

/// Which group a key plays on, as in avldrums.lv2
pub fn group_of(key: u8) -> u8 {
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

/// The drum groups as the instrument's outputs
pub fn output_groups() -> Vec<OutputGroup> {
    GROUPS.iter().map(|(n, stereo)| OutputGroup { name: (*n).to_owned(), stereo: *stereo }).collect()
}

/// Sets each zone's output by its keys' group, splitting a zone whose keys span several groups. `groups`: the
/// kit has groups of pieces (the percussion kit has not)
pub fn assign_outputs(i: &mut Instrument, groups: bool) {
    if !groups {
        return;
    }
    let zones = std::mem::take(&mut i.zones);
    for z in zones {
        let mut from = z.keys.0;
        for k in z.keys.0..=z.keys.1 {
            let last = k == z.keys.1;
            if last || group_of(k + 1) != group_of(from) {
                let mut piece = z.clone();
                piece.keys = (from, k);
                piece.output = group_of(from);
                i.zones.push(piece);
                from = k.saturating_add(1);
            }
        }
    }
    i.outputs = output_groups();
}

/// A kit's SoundFont (the bytes): its first preset, with outputs by group when `groups`. Keys: the SF2 importer's
pub fn import(bytes: &[u8], groups: bool, sf3: Option<&mut dyn Sf3Decoder>) -> Result<Instrument> {
    let font = sf2::SoundFont::parse(bytes)?;
    let mut i = font.import_first(sf3)?;
    // (a kit's pieces are hits: each plays to its end, as FluidSynth plays the drum channel)
    for z in &mut i.zones {
        z.loop_mode = crate::model::LoopMode::OneShot;
    }
    assign_outputs(&mut i, groups);
    i.validate()?;
    Ok(i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Zone;
    use crate::pcm::Pcm;
    use std::sync::Arc;

    #[test]
    fn the_map_is_avldrums_lv2s() {
        assert_eq!(group_of(36), 0);
        assert_eq!(group_of(38), 1);
        assert_eq!(group_of(42), 2);
        assert_eq!(group_of(45), 3);
        assert_eq!(group_of(43), 4);
        assert_eq!(group_of(49), 5);
        assert_eq!(group_of(39), 6);
    }

    #[test]
    fn a_zone_across_groups_is_split() {
        let mut i = Instrument::new("kit");
        let mut z = Zone::new(Arc::new(Pcm::mono(44100, vec![0.0; 10])));
        z.keys = (35, 38);
        i.zones.push(z);
        assign_outputs(&mut i, true);
        // 35 (percussion), 36 (kick), 37-38 (snare)
        assert_eq!(i.zones.len(), 3);
        assert_eq!(i.zones[0].keys, (35, 35));
        assert_eq!(i.zones[1].output, 0);
        assert_eq!(i.zones[2].keys, (37, 38));
        assert_eq!(i.outputs.len(), 7);
    }
}
