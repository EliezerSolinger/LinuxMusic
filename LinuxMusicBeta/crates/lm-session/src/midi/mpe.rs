//! MPE settings of the MIDI ports (`decisoes-autonomas.md`, 26/11, item 5): a port of the machine's preferences
//! (`midi.ports`) can be marked as an MPE zone with `mpe: "lower" | "upper"` (absent or `"off"` = not MPE) and
//! `mpeMembers: 1..15`, the member channels of the zone. The renderer reads them to turn the member channels'
//! bend, pressure and CC 74 into per-note expressions, and to send a track's notes out as MPE; the recorder reads them
//! to make the per-note curves of the notes.

use lm_render::PortIndex;
use lm_render::mpe::{DEFAULT_BEND_SEMITONES, MAX_MEMBERS, MpePort, Zone};
use lm_state::Value;

use super::devices::port_key;
use crate::session::Session;

/// The members of a zone when the preference does not say
pub(crate) const DEFAULT_MEMBERS: u8 = 15;

/// The zone and member count a port preference sets, `None` when the port is not MPE
pub(crate) fn zone_of(port: &Value) -> Option<(Zone, u8)> {
    let zone = match port.get("mpe").and_then(Value::as_text)? {
        "lower" => Zone::Lower,
        "upper" => Zone::Upper,
        _ => return None,
    };
    let members = port.get("mpeMembers").and_then(Value::as_int).unwrap_or(i64::from(DEFAULT_MEMBERS));
    Some((zone, members.clamp(1, i64::from(MAX_MEMBERS)) as u8))
}

/// What a recording needs of an MPE zone to read the per-note curves
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MpeInfo {
    pub zone: Zone,
    pub members: u8,
    pub bend_semitones: f32,
}

impl MpeInfo {
    pub(crate) fn port(&self) -> MpePort {
        let mut p = MpePort::new(PortIndex(0), self.zone, self.members);
        p.bend_semitones = self.bend_semitones;
        p
    }

    /// The text of the take log's header (`lower:15:48`)
    pub(crate) fn to_text(self) -> String {
        let z = if self.zone == Zone::Lower { "lower" } else { "upper" };
        format!("{z}:{}:{}", self.members, self.bend_semitones)
    }

    pub(crate) fn from_text(t: &str) -> Option<Self> {
        let mut it = t.split(':');
        let zone = match it.next()? {
            "lower" => Zone::Lower,
            "upper" => Zone::Upper,
            _ => return None,
        };
        let members: u8 = it.next()?.parse().ok()?;
        let bend_semitones: f32 = it.next()?.parse().ok()?;
        Some(Self { zone, members: members.clamp(1, MAX_MEMBERS), bend_semitones })
    }
}

impl Session {
    /// The ports that are MPE zones, with their session indices (for the compiler's environment)
    pub(crate) fn midi_mpe_ports(&self) -> Vec<MpePort> {
        let ports = self.preferences.get(&["midi", "ports"]).cloned().unwrap_or_else(Value::empty_list);
        ports
            .items()
            .filter_map(|p| {
                let (zone, members) = zone_of(p)?;
                let key = port_key(p);
                let (index, _) = self.midi.aliases.get(&key)?;
                Some(MpePort::new(*index, zone, members))
            })
            .collect()
    }

    /// The MPE setting a recording on these ports (empty: every instrument port) takes: the first MPE port of them
    pub(crate) fn mpe_for_ports(&self, ports: &[u16]) -> Option<MpeInfo> {
        self.midi_mpe_ports().into_iter().find(|p| ports.is_empty() || ports.contains(&p.port.0)).map(|p| MpeInfo {
            zone: p.zone,
            members: p.members,
            bend_semitones: DEFAULT_BEND_SEMITONES,
        })
    }
}
