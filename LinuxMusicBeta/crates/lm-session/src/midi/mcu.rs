//! Controller profiles and the Mackie Control Universal translator (interfaces.md, I-21.3).
//!
//! A profile is a data file (`controllers/*.json`): machine mappings, a strip template repeated for each strip of
//! a bank, and the bank size. The built-in ones are `generic` (editable: a copy in the preferences' folder wins)
//! and `mcu`. Everything a strip does is an ordinary mapping (faders with touch, V-pots with their LED rings,
//! buttons with their LEDs, banks, transport); this module only adds what the protocol needs beyond mappings: the
//! display (SysEx), the meters and the two-digit assignment display. It talks to the core only through the
//! session, like any other client.

use std::path::PathBuf;
use std::sync::Arc;

use lm_base::HostTime;
use lm_midimap::Mapping;
use lm_midimap::mcu::{self, CELL, LCD_WIDTH, MAX_SYSEX, STRIPS};
use lm_render::PortIndex;
use lm_state::{NodeId, Value, from_json};

use crate::prefs::Preferences;
use crate::session::Session;

const BUILTIN_MCU: &str = include_str!("../../controllers/mcu.json");
const BUILTIN_GENERIC: &str = include_str!("../../controllers/generic.json");

/// How often the display and the meters are refreshed (ns)
const REFRESH: u64 = 50_000_000;

/// The text of a profile: the preferences' folder first (`controllers/<name>.json`), then the built-in ones
pub(crate) fn profile_text(name: &str) -> Option<String> {
    let dir: PathBuf = crate::prefs::location().parent().map(|p| p.join("controllers")).unwrap_or_default();
    if let Ok(t) = std::fs::read_to_string(dir.join(format!("{name}.json"))) {
        return Some(t);
    }
    match name {
        "mcu" => Some(BUILTIN_MCU.to_owned()),
        "generic" => Some(BUILTIN_GENERIC.to_owned()),
        _ => None,
    }
}

/// A strip template made into the mapping of strip `i`: `stripNumber` adds `i` to the message numbers (source,
/// touch, feedback), `stripChannel` makes `i` the channel, `stripTouch` adds `i` to the touch note, the target's
/// bank offset is `i`, and `stripParam` puts `i` into that action parameter
fn strip(template: &Value, i: i64) -> Value {
    let flag = |k: &str| template.get(k).and_then(Value::as_bool) == Some(true);
    let add = |v: &Value, key: &str| -> Value {
        match v.get(key).and_then(Value::as_int) {
            Some(n) => v.with(key, Value::int(n + i)),
            None => v.clone(),
        }
    };
    let mut m = template.clone();
    if let Some(s) = m.get("source").cloned() {
        let mut s = if flag("stripNumber") { add(&s, "number") } else { s };
        if flag("stripChannel") {
            s = s.with("channel", Value::int(i));
        }
        m = m.with("source", s);
    }
    if let Some(t) = m.get("touch").cloned().filter(|t| !t.is_null())
        && (flag("stripNumber") || flag("stripTouch"))
    {
        m = m.with("touch", add(&t, "number"));
    }
    if let Some(f) = m.get("feedbackTo").cloned()
        && flag("stripNumber")
    {
        m = m.with("feedbackTo", add(&f, "number"));
    }
    if let Some(t) = m.get("target").cloned() {
        let mut t = add(&t, "offset");
        if let Some(p) = template.get("stripParam").and_then(Value::as_text)
            && let Some(params) = t.get("params").cloned()
        {
            t = t.with("params", params.with(p, Value::int(i)));
        }
        m = m.with("target", t);
    }
    m
}

/// The mappings of one profile for the port `alias` (sources without a port get it)
pub(crate) fn profile(text: &str, alias: &str, bad: &mut usize) -> (Vec<Mapping>, String) {
    let Ok(v) = from_json(text) else {
        *bad += 1;
        return (Vec::new(), String::new());
    };
    let kind = v.get("kind").and_then(Value::as_text).unwrap_or("generic").to_owned();
    let strips = v.get("strips").and_then(Value::as_int).unwrap_or(0);
    let mut values: Vec<Value> = v.get("mappings").map(|l| l.items().cloned().collect()).unwrap_or_default();
    for t in v.get("strip").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        for i in 0..strips {
            values.push(strip(&t, i));
        }
    }
    let mut out = Vec::new();
    for m in values {
        let fix = |s: Value| if s.get("port").is_none() { s.with("port", Value::text(alias)) } else { s };
        let mut m = m;
        if let Some(s) = m.get("source").cloned() {
            m = m.with("source", fix(s));
        }
        if let Some(t) = m.get("touch").cloned().filter(|t| !t.is_null()) {
            m = m.with("touch", fix(t));
        }
        match Mapping::from_value(&m) {
            Ok(x) => out.push(x),
            Err(_) => *bad += 1,
        }
    }
    (out, kind)
}

/// The mappings of every enabled controller profile (`preferences.controllers`)
pub(crate) fn profile_mappings(prefs: &Preferences, bad: &mut usize) -> Vec<Mapping> {
    let mut out = Vec::new();
    for c in prefs.get(&["controllers"]).map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        let (Some(name), Some(alias)) =
            (c.get("profile").and_then(Value::as_text), c.get("port").and_then(Value::as_text))
        else {
            continue;
        };
        match profile_text(name) {
            Some(text) => out.extend(profile(&text, alias, bad).0),
            None => *bad += 1,
        }
    }
    out
}

/// One Mackie Control on a port: what its display and meters show now (sent only when it changes)
#[derive(Debug)]
pub(crate) struct Mcu {
    pub(crate) port: PortIndex,
    alias: Arc<str>,
    lcd: [[u8; LCD_WIDTH]; 2],
    meters: [u8; STRIPS as usize],
    assignment: [u8; 2],
    last: u64,
    fresh: bool,
}

impl Mcu {
    fn new(port: PortIndex, alias: Arc<str>) -> Mcu {
        Mcu {
            port,
            alias,
            lcd: [[0; LCD_WIDTH]; 2],
            meters: [0xFF; STRIPS as usize],
            assignment: [0; 2],
            last: 0,
            fresh: true,
        }
    }

    /// Forget what the unit shows (reconnected, a bank change): everything is sent again
    pub(crate) fn reset(&mut self) {
        self.fresh = true;
    }
}

/// A value in dB for the display (the fader's gain)
fn db_text(gain: f64) -> String {
    if gain <= 0.0 {
        return "-inf".to_owned();
    }
    format!("{:.1}", 20.0 * gain.log10())
}

impl Session {
    /// The units of the enabled `mcu` profiles, kept with their ports
    fn mcu_units(&mut self) {
        let wanted: Vec<Arc<str>> = self
            .preferences
            .get(&["controllers"])
            .map(|l| {
                l.items()
                    .filter(|c| {
                        c.get("profile").and_then(Value::as_text).and_then(profile_text).is_some_and(|t| {
                            from_json(&t).ok().and_then(|v| v.get("kind").and_then(Value::as_text).map(|k| k == "mcu"))
                                == Some(true)
                        })
                    })
                    .filter_map(|c| c.get("port").and_then(Value::as_shared_text).cloned())
                    .collect()
            })
            .unwrap_or_default();
        self.midi.mcu.retain(|m| wanted.contains(&m.alias));
        for alias in wanted {
            if self.midi.mcu.iter().any(|m| m.alias == alias) {
                continue;
            }
            if let Some(port) = self.midi_port_of(&alias) {
                self.midi.mcu.push(Mcu::new(port, alias));
            }
        }
    }

    /// The display (track names and volumes of the bank), the meters and the bank number (I-21.3)
    pub(crate) fn mcu_pump(&mut self, now: HostTime) {
        self.mcu_units();
        if self.midi.mcu.is_empty() {
            return;
        }
        let tracks: Vec<(NodeId, String, f64)> = self
            .document
            .root()
            .get("tracks")
            .map(|l| {
                l.items()
                    .filter_map(|t| {
                        let id = NodeId::parse(t.get("id")?.as_text()?).ok()?;
                        let name = t.get("name").and_then(Value::as_text).unwrap_or("").to_owned();
                        Some((id, name, t.get("volume").and_then(Value::as_real).unwrap_or(1.0)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let bank = self.midi.bank.0 as usize;
        let mut rows = [[b' '; LCD_WIDTH]; 2];
        let mut levels = [0u8; STRIPS as usize];
        for s in 0..STRIPS as usize {
            let Some((id, name, volume)) = tracks.get(bank + s) else { continue };
            let label = if name.is_empty() { format!("{}", bank + s + 1) } else { name.clone() };
            rows[0][s * CELL..(s + 1) * CELL].copy_from_slice(&mcu::cell(&label));
            rows[1][s * CELL..(s + 1) * CELL].copy_from_slice(&mcu::cell(&db_text(*volume)));
            let peak = self.telemetry.peak(*id);
            let db = if peak > 0.0 { 20.0 * peak.log10() } else { -200.0 };
            levels[s] = mcu::meter_level(db);
        }
        let digits = format!("{:02}", (bank / STRIPS as usize + 1) % 100);
        let assignment: Vec<u8> = digits.chars().map(mcu::segment).collect();
        let mut sends: Vec<(PortIndex, Vec<u8>)> = Vec::new();
        let mut shorts: Vec<(PortIndex, Vec<u8>)> = Vec::new();
        for unit in &mut self.midi.mcu {
            if !unit.fresh && now.0.saturating_sub(unit.last) < REFRESH {
                continue;
            }
            unit.last = now.0;
            for (r, row) in rows.iter().enumerate() {
                for s in 0..STRIPS as usize {
                    let cell = &row[s * CELL..(s + 1) * CELL];
                    if unit.fresh || unit.lcd[r][s * CELL..(s + 1) * CELL] != *cell {
                        let mut buf = [0u8; MAX_SYSEX];
                        let text = String::from_utf8_lossy(cell).into_owned();
                        let n = mcu::lcd(r as u8, s * CELL, &text, &mut buf);
                        sends.push((unit.port, buf[..n].to_vec()));
                        unit.lcd[r][s * CELL..(s + 1) * CELL].copy_from_slice(cell);
                    }
                }
            }
            for (s, level) in levels.iter().enumerate() {
                // The unit lets meters fall by itself: a level is sent when it changes or rises
                if unit.fresh || unit.meters[s] != *level {
                    shorts.push((unit.port, mcu::meter(s as u8, *level).to_vec()));
                    unit.meters[s] = *level;
                }
            }
            for (i, c) in assignment.iter().enumerate() {
                if unit.fresh || unit.assignment[i] != *c {
                    let cc = if i == 0 { mcu::ASSIGNMENT_LEFT } else { mcu::ASSIGNMENT_RIGHT };
                    shorts.push((unit.port, vec![0xB0, cc, *c]));
                    unit.assignment[i] = *c;
                }
            }
            unit.fresh = false;
        }
        self.midi.sysex.extend(sends);
        for (port, bytes) in shorts {
            self.midi_send(port, &bytes);
        }
    }

    /// The control surfaces' bank moved (`midi.bank`): the map follows, and every value is sent again
    pub(crate) fn mcu_bank(&mut self, role: &str, delta: i64) {
        let count = |path: &[&str]| -> i64 {
            let mut v = self.document.root();
            for k in path {
                match v.get(k) {
                    Some(x) => v = x,
                    None => return 0,
                }
            }
            v.len() as i64
        };
        let (bank, n) = match role {
            "channel" => (&mut self.midi.bank.1, count(&["mixer", "channels"])),
            _ => (&mut self.midi.bank.0, count(&["tracks"])),
        };
        let max = (n - 1).max(0);
        *bank = (i64::from(*bank) + delta).clamp(0, max) as u32;
        self.midi.mark_dirty();
        for m in &mut self.midi.mcu {
            m.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lm_midimap::{MapTarget, Message, NodeRef, SelRole};

    #[test]
    fn the_mcu_profile_expands_its_strips() {
        let mut bad = 0;
        let (maps, kind) = profile(BUILTIN_MCU, "MCU", &mut bad);
        assert_eq!(bad, 0);
        assert_eq!(kind, "mcu");
        // Fader 3: pitch bend on channel 3, touch note 0x6B, the bank's fourth track's volume
        let fader = maps
            .iter()
            .find(|m| m.source.message == Message::PitchBend && m.source.channel == Some(3))
            .expect("fader 3");
        assert_eq!(fader.touch.as_ref().unwrap().message, Message::Note { key: 0x6B });
        assert!(
            matches!(&fader.target, MapTarget::Param { node: NodeRef::Selected { role: SelRole::TrackBank, offset: 3 }, key } if key == "volume")
        );
        assert_eq!(&*fader.source.port, "MCU");
        // V-pot 5: CC 0x15, sign-magnitude, its ring on CC 0x35
        let vpot = maps.iter().find(|m| m.source.message == Message::Cc { number: 0x15 }).expect("v-pot 5");
        assert_eq!(vpot.feedback_to.map(|f| f.0), Some(Message::Cc { number: 0x35 }));
        // Select 7 is the action with offset 7
        let select = maps.iter().find(|m| m.source.message == Message::Note { key: 0x1F }).expect("select 7");
        assert!(
            matches!(&select.target, MapTarget::Action { params, .. } if params.get("offset").and_then(Value::as_int) == Some(7))
        );
        let (generic, kind) = profile(BUILTIN_GENERIC, "Ctl", &mut bad);
        assert_eq!((bad, kind.as_str()), (0, "generic"));
        assert!(generic.len() > 16);
    }
}
