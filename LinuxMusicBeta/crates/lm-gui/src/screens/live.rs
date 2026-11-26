//! Live notes from the interface to an instrument (P2, I-15 `note_on`/`note_off`/`all_notes_off`): the instrument
//! window's test keyboard, the computer keyboard as a piano, the piano roll's keys and the audition of a note being
//! drawn or moved, the kit pictures. One [`Live`] per window keeps what it started, so that no note sticks: the owner
//! releases it on mouse-up, on focus loss, on deactivation and when the window closes

use std::collections::BTreeSet;

use gpui::{AppContext, Entity};
use lm_port::NodeId;

use crate::core::Core;

/// The velocity of a key with no position to give one (the computer keyboard)
pub const DEFAULT_VELOCITY: u8 = 100;

/// The legacy's computer-keyboard piano (PianoView `getKeyOffsetFromKeyEvent`, the Linux layout): the semitones above
/// the lowest key (Z = C), two rows: `z s x d c v g b h n j m , l . ; /` and `q 2 w 3 e r 5 t 6 y 7 u i 9 o 0 p [ = ]`
/// (the second row starts an octave above). The key number is `36 + offset` (the legacy's octave 3 as MIDI)
pub fn piano_offset(k: &str) -> Option<i32> {
    const LOW: [&str; 17] = ["z", "s", "x", "d", "c", "v", "g", "b", "h", "n", "j", "m", ",", "l", ".", ";", "/"];
    const HIGH: [&str; 20] =
        ["q", "2", "w", "3", "e", "r", "5", "t", "6", "y", "7", "u", "i", "9", "o", "0", "p", "[", "=", "]"];
    LOW.iter()
        .position(|x| *x == k)
        .map(|i| i as i32)
        .or_else(|| HIGH.iter().position(|x| *x == k).map(|i| i as i32 + 12))
}

/// The key number the legacy gives a computer key in an instrument window
pub fn window_key(k: &str) -> Option<u8> {
    piano_offset(k).map(|o| (36 + o).clamp(0, 127) as u8)
}

/// The notes one window has sounding, by `(track, key)`
#[derive(Default)]
pub struct Live {
    held: BTreeSet<(NodeId, u8)>,
}

impl Live {
    /// Starts `key` on `track` (a key already sounding is restarted)
    pub fn on(&mut self, core: &Entity<Core>, track: NodeId, key: u8, velocity: u8, cx: &mut impl AppContext) {
        let key = key.min(127);
        core.update(cx, |c, _| {
            if self.held.contains(&(track, key)) {
                c.port_mut().note_off(track, key);
            }
            c.port_mut().note_on(track, key, velocity.clamp(1, 127));
        });
        self.held.insert((track, key));
    }

    /// Ends `key` on `track` if this window started it
    pub fn off(&mut self, core: &Entity<Core>, track: NodeId, key: u8, cx: &mut impl AppContext) {
        if self.held.remove(&(track, key)) {
            core.update(cx, |c, _| c.port_mut().note_off(track, key));
        }
    }

    /// Ends everything this window started: one note-off each, then all notes off on each track it touched (the
    /// instrument ends what a lost note-off left)
    pub fn release_all(&mut self, core: &Entity<Core>, cx: &mut impl AppContext) {
        if self.held.is_empty() {
            return;
        }
        let held = std::mem::take(&mut self.held);
        core.update(cx, |c, _| {
            let mut tracks = BTreeSet::new();
            for (t, k) in held {
                c.port_mut().note_off(t, k);
                tracks.insert(t);
            }
            for t in tracks {
                c.port_mut().all_notes_off(t);
            }
        });
    }
}

/// The General MIDI key a kit picture's piece plays, by the name of its output group in AvlDrums (avldrums.lv2's
/// groups, one representative key each)
pub fn group_key(name: &str) -> Option<u8> {
    Some(match name {
        "kick" => 36,
        "snare" => 38,
        "hihat" => 42,
        "tom" => 45,
        "floortom" => 41,
        "cymbals" => 49,
        "percussion" => 39,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legacy_layout() {
        assert_eq!(window_key("z"), Some(36));
        assert_eq!(window_key("s"), Some(37));
        assert_eq!(window_key("/"), Some(52));
        assert_eq!(window_key("q"), Some(48));
        assert_eq!(window_key("2"), Some(49));
        assert_eq!(window_key("p"), Some(64));
        assert_eq!(window_key("]"), Some(67));
        assert_eq!(window_key("1"), None);
        assert_eq!(window_key("a"), None);
    }

    #[test]
    fn groups_have_keys() {
        assert_eq!(group_key("kick"), Some(36));
        assert_eq!(group_key("nope"), None);
    }
}
