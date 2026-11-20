//! MIDI recording (interfaces.md, I-21.4).
//!
//! The renderer sends every message that reached the instruments while recording, at its sample on the timeline
//! (`RecordedMidi`, lossless). Here they go to the armed tracks that listen to their port and channel, and every
//! second to an append-only log (`audio/rec-<uuid>.midilog`, `fsync`ed): a crash loses at most the last second, and
//! opening the project again recovers the take. At the stop the take becomes items — notes with their velocity,
//! and control lanes for controllers, pitch bend, aftertouch and program changes; held keys end at the stop — in
//! the same `Origin::Recording` transaction as the audio takes (one undo removes the whole recording).
//!
//! Modes (F-REC-101): overdub (the default) adds an item; replace removes the notes under the take first; layers
//! makes one item per loop pass and mutes all but the last. In a loop a pass shorter than 90 % of the loop is
//! dropped, as with audio. Quantize on record (F-REC-102): grid, strength, starts only.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use lm_base::{HostTime, Label, Result};
use lm_doc::{Edit, Origin};
use lm_render::{RecordedMidi, TransportEvent};
use lm_state::{NodeId, Value};
use lm_time::TempoMap;

use crate::session::{Session, tempo_map};

const SECOND: u64 = 1_000_000_000;

/// One recorded message, routed to a track
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rec {
    pub track: NodeId,
    pub sample: i64,
    pub frame: u64,
    pub bytes: [u8; 3],
}

/// Where the messages of a port go: the armed tracks with an instrument, by their `midiInput`
#[derive(Clone, Debug)]
struct Route {
    track: NodeId,
    /// Port indices (empty: every port)
    ports: Vec<u16>,
    channel: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Quantize {
    pub grid: i64,
    pub strength: f64,
    pub starts_only: bool,
}

/// What one take needs to become items
#[derive(Clone, Debug)]
pub(crate) struct TakeInfo {
    pub start_sample: i64,
    pub start_frame: u64,
    pub stop_sample: i64,
    pub stop_frame: u64,
    pub loop_range: Option<(i64, i64)>,
    /// Engine frames where the loop wrapped
    pub wraps: Vec<u64>,
    pub mode: String,
    pub quantize: Option<Quantize>,
}

#[derive(Debug)]
struct Take {
    info: TakeInfo,
    path: PathBuf,
    file: Option<File>,
    routes: Vec<Route>,
    events: Vec<Rec>,
    unsynced: String,
    last_sync: u64,
}

/// A take ready to enter the tree
#[derive(Debug)]
pub(crate) struct Ready {
    pub info: TakeInfo,
    pub events: Vec<Rec>,
    pub path: PathBuf,
    pub recovered: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Recorder {
    take: Option<Take>,
    pub(crate) ready: Option<Ready>,
}

/// The ticks of a pass: which pass an engine frame falls in
fn pass_of(info: &TakeInfo, frame: u64) -> usize {
    info.wraps.iter().filter(|w| **w <= frame).count()
}

fn quantize(tick: i64, q: &Quantize) -> i64 {
    let g = q.grid.max(1);
    let target = (tick + g / 2).div_euclid(g) * g;
    tick + ((target - tick) as f64 * q.strength.clamp(0.0, 1.0)).round() as i64
}

/// A control lane's key: (kind, number, channel)
pub(crate) type ControlKey = (&'static str, u8, u8);

/// One item of a track's take: notes and control lanes, its start and length in ticks
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Built {
    pub start: i64,
    pub length: i64,
    /// (key, start from the item's start, length, velocity 0..1)
    pub notes: Vec<(i64, i64, i64, f64)>,
    /// (kind, number, channel) → (tick from the item's start, value)
    pub controls: BTreeMap<ControlKey, Vec<(i64, i64)>>,
    pub muted: bool,
}

/// The items one track's messages make (I-21.4), in the take's mode
pub(crate) fn build(info: &TakeInfo, events: &[Rec], map: &TempoMap) -> Vec<Built> {
    let (from, to) = match info.loop_range.filter(|_| !info.wraps.is_empty()) {
        Some((a, b)) => (a, b),
        None => (info.start_sample, info.stop_sample.max(info.start_sample + 1)),
    };
    // The passes kept: in a loop, one shorter than 90 % of the loop is dropped
    let passes = info.wraps.len() + 1;
    let keep: Vec<bool> = (0..passes)
        .map(|i| {
            let Some((a, b)) = info.loop_range.filter(|_| !info.wraps.is_empty()) else { return true };
            let start = if i == 0 { info.start_frame } else { info.wraps[i - 1] };
            let end = if i < info.wraps.len() { info.wraps[i] } else { info.stop_frame };
            end.saturating_sub(start) as f64 >= 0.9 * (b - a) as f64
        })
        .collect();
    let kept: Vec<usize> = (0..passes).filter(|i| keep[*i]).collect();
    if kept.is_empty() {
        return Vec::new();
    }
    let groups: Vec<Vec<usize>> = match info.mode.as_str() {
        "layers" => kept.iter().map(|i| vec![*i]).collect(),
        "replace" => vec![vec![*kept.last().expect("one pass")]],
        _ => vec![kept.clone()],
    };
    let start_tick = map.tick_at(from);
    let end_tick = map.tick_at(to).max(start_tick + 1);
    let n_groups = groups.len();
    let mut out = Vec::new();
    for (g, passes) in groups.into_iter().enumerate() {
        let mut built =
            Built { start: start_tick, length: end_tick - start_tick, muted: g + 1 < n_groups, ..Built::default() };
        let mut open: BTreeMap<(u8, u8), (i64, f64)> = BTreeMap::new();
        let close = |built: &mut Built, key: u8, on: i64, vel: f64, off: i64| {
            let (mut s, mut e) = (on, off.max(on + 1));
            if let Some(q) = &info.quantize {
                let qs = quantize(s, q);
                if q.starts_only {
                    e = e.max(qs + 1);
                } else {
                    e = quantize(e, q).max(qs + 1);
                }
                s = qs;
            }
            let s = s.clamp(0, built.length - 1);
            built.notes.push((i64::from(key), s, (e - s).max(1).min(built.length - s).max(1), vel));
        };
        let mut last_pass = None;
        for ev in events.iter().filter(|e| passes.contains(&pass_of(info, e.frame))) {
            let pass = pass_of(info, ev.frame);
            if last_pass.is_some_and(|p| p != pass) {
                // A wrap ends the keys held from the pass before at the loop's end
                let end = built.length;
                for ((_, key), (on, vel)) in std::mem::take(&mut open) {
                    close(&mut built, key, on, vel, end);
                }
            }
            last_pass = Some(pass);
            let tick = (map.tick_at(ev.sample) - start_tick).clamp(0, built.length - 1);
            let [status, d1, d2] = ev.bytes;
            let ch = status & 0x0F;
            match status & 0xF0 {
                0x90 if d2 > 0 => {
                    if let Some((on, vel)) = open.remove(&(ch, d1)) {
                        close(&mut built, d1, on, vel, tick);
                    }
                    open.insert((ch, d1), (tick, f64::from(d2) / 127.0));
                }
                0x80 | 0x90 => {
                    if let Some((on, vel)) = open.remove(&(ch, d1)) {
                        close(&mut built, d1, on, vel, tick);
                    }
                }
                0xB0 => built.controls.entry(("cc", d1, ch)).or_default().push((tick, i64::from(d2))),
                0xE0 => built
                    .controls
                    .entry(("pitchBend", 0, ch))
                    .or_default()
                    .push((tick, i64::from(d1) | (i64::from(d2) << 7))),
                0xD0 => built.controls.entry(("channelPressure", 0, ch)).or_default().push((tick, i64::from(d1))),
                0xA0 => built.controls.entry(("polyPressure", d1, ch)).or_default().push((tick, i64::from(d2))),
                0xC0 => built.controls.entry(("programChange", 0, ch)).or_default().push((tick, i64::from(d1))),
                _ => {}
            }
        }
        // Keys still held end at the stop (or the loop's end)
        let end = built.length;
        for ((_, key), (on, vel)) in open {
            close(&mut built, key, on, vel, end);
        }
        out.push(built);
    }
    out
}

/// The log's lines: a header, then `<sample> <frame> <track> <b0> <b1> <b2>`, `wrap <frame>`, `stop <sample> <frame>`
fn header(info: &TakeInfo) -> String {
    let (a, b) = info.loop_range.unwrap_or((-1, -1));
    format!(
        "midilog 1 {} {} {} {} {} {} {} {}\n",
        info.start_sample,
        info.start_frame,
        a,
        b,
        info.mode,
        info.quantize.map_or(0, |q| q.grid),
        info.quantize.map_or(0.0, |q| q.strength),
        info.quantize.is_some_and(|q| q.starts_only),
    )
}

/// Reads a log back (a crash recovery): everything written up to the last whole line
pub(crate) fn read_log(text: &str) -> Option<(TakeInfo, Vec<Rec>)> {
    let mut lines = text.lines();
    let h: Vec<&str> = lines.next()?.split(' ').collect();
    if h.len() < 10 || h[0] != "midilog" {
        return None;
    }
    let (a, b): (i64, i64) = (h[4].parse().ok()?, h[5].parse().ok()?);
    let grid: i64 = h[7].parse().ok()?;
    let mut info = TakeInfo {
        start_sample: h[2].parse().ok()?,
        start_frame: h[3].parse().ok()?,
        stop_sample: 0,
        stop_frame: 0,
        loop_range: (a >= 0 && b > a).then_some((a, b)),
        wraps: Vec::new(),
        mode: h[6].to_owned(),
        quantize: (grid > 0).then(|| Quantize {
            grid,
            strength: h[8].parse().unwrap_or(1.0),
            starts_only: h[9] == "true",
        }),
    };
    let mut events = Vec::new();
    let mut stopped = false;
    for l in lines {
        let f: Vec<&str> = l.split(' ').collect();
        match f.as_slice() {
            ["wrap", frame] => info.wraps.push(frame.parse().ok()?),
            ["stop", sample, frame] => {
                info.stop_sample = sample.parse().ok()?;
                info.stop_frame = frame.parse().ok()?;
                stopped = true;
            }
            [sample, frame, track, b0, b1, b2] => {
                let Ok(track) = NodeId::parse(track) else { continue };
                let (Ok(sample), Ok(frame)) = (sample.parse::<i64>(), frame.parse::<u64>()) else { continue };
                let (Ok(b0), Ok(b1), Ok(b2)) = (b0.parse::<u8>(), b1.parse::<u8>(), b2.parse::<u8>()) else { continue };
                events.push(Rec { track, sample, frame, bytes: [b0, b1, b2] });
            }
            _ => {}
        }
    }
    if !stopped {
        // Cut by the crash: the take ends at its last message
        let last = events
            .iter()
            .map(|e| (e.sample, e.frame))
            .max_by_key(|e| e.1)
            .unwrap_or((info.start_sample, info.start_frame));
        info.stop_sample = last.0 + 1;
        info.stop_frame = last.1 + 1;
    }
    Some((info, events))
}

/// Inserts a ready take into the tree, inside the recording's transaction
pub(crate) fn apply(ready: &Ready, map: &TempoMap, e: &mut Edit<'_>) -> Result<()> {
    let mut by_track: BTreeMap<NodeId, Vec<Rec>> = BTreeMap::new();
    for ev in &ready.events {
        by_track.entry(ev.track).or_default().push(*ev);
    }
    for (track, mut events) in by_track {
        if e.find(track).is_none() {
            continue; // the track was removed while recording; its MIDI has nowhere to go
        }
        events.sort_by_key(|e| e.frame);
        let items = build(&ready.info, &events, map);
        if ready.info.mode == "replace" {
            replace_under(e, track, &items)?;
        }
        for b in items {
            if b.notes.is_empty() && b.controls.is_empty() {
                continue;
            }
            let controls: Vec<Value> =
                b.controls
                    .iter()
                    .map(|((kind, number, channel), points)| {
                        Value::object([
                            ("kind", Value::text(*kind)),
                            ("number", Value::int(i64::from(*number))),
                            ("channel", Value::int(i64::from(*channel))),
                            (
                                "points",
                                Value::list(points.iter().map(|(t, v)| {
                                    Value::object([("tick", Value::int(*t)), ("value", Value::int(*v))])
                                })),
                            ),
                        ])
                    })
                    .collect();
            let mut item = Value::object([
                ("kind", Value::text("midi")),
                ("name", Value::text(if ready.recovered { "Take recuperado" } else { "" })),
                ("start", Value::int(b.start)),
                ("length", Value::int(b.length.max(1))),
                ("mute", Value::bool(b.muted)),
            ]);
            if !controls.is_empty() {
                item = item.with("controls", Value::list(controls));
            }
            let n = e.find(track).and_then(|t| t.get("items").map(Value::len)).unwrap_or(0);
            let id = e.insert(track, "items", n, item)?;
            for (key, start, length, velocity) in &b.notes {
                let note = Value::object([
                    ("key", Value::int(*key)),
                    ("start", Value::int(*start)),
                    ("length", Value::int(*length)),
                    ("velocity", Value::real(*velocity)?),
                ]);
                e.add_note(id, note)?;
            }
        }
    }
    Ok(())
}

/// Replace mode: the notes of the track's MIDI items that start under the take go
fn replace_under(e: &mut Edit<'_>, track: NodeId, items: &[Built]) -> Result<()> {
    let Some(b) = items.first() else { return Ok(()) };
    let (from, to) = (b.start, b.start + b.length);
    let t = e.find(track).cloned().unwrap_or_default();
    for item in t.get("items").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        if item.get("kind").and_then(Value::as_text) != Some("midi") {
            continue;
        }
        let Some(id) = item.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) else { continue };
        let start = item.get("start").and_then(Value::as_int).unwrap_or(0);
        let gone: Vec<u64> = item
            .get("notes")
            .map(|notes| {
                notes
                    .entries()
                    .filter_map(|(k, n)| {
                        let s = start + n.get("start").and_then(Value::as_int).unwrap_or(0);
                        (s >= from && s < to).then(|| lm_state::parse_note_key(k)).flatten()
                    })
                    .collect()
            })
            .unwrap_or_default();
        for n in gone {
            e.remove_note(id, n)?;
        }
    }
    Ok(())
}

impl Session {
    /// The armed tracks with an instrument, and what each listens to
    fn record_routes(&self) -> Vec<Route> {
        let mut out = Vec::new();
        let (selected, _, _) = super::read_selection(self.ui_document.root());
        for t in self.document.root().get("tracks").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default()
        {
            let Some(id) = t.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) else { continue };
            if t.get("armed").and_then(Value::as_bool) != Some(true) || t.get("instrument").is_none_or(Value::is_null) {
                continue;
            }
            let input = t.get("midiInput");
            if input.and_then(|i| i.get("followSelection")).and_then(Value::as_bool) == Some(true)
                && selected != Some(id)
            {
                continue;
            }
            let ports = input
                .and_then(|i| i.get("ports"))
                .map(|l| {
                    l.items().filter_map(Value::as_text).filter_map(|a| self.midi_port_of(a)).map(|p| p.0).collect()
                })
                .unwrap_or_default();
            let channel =
                input.and_then(|i| i.get("channel")).and_then(Value::as_int).and_then(|c| u8::try_from(c).ok());
            out.push(Route { track: id, ports, channel });
        }
        out
    }

    /// The renderer's transport events (I-11): a recording starts, wraps, stops
    pub(crate) fn midi_transport(&mut self, e: &TransportEvent) {
        match *e {
            TransportEvent::RecordStarted { sample, frame } => {
                let routes = self.record_routes();
                if routes.is_empty() {
                    return;
                }
                let transport = self.document.root().get("transport").cloned().unwrap_or_default();
                let lp = transport.get("loop");
                let map = tempo_map(self.document.root()).ok();
                let loop_range = lp.filter(|l| l.get("on").and_then(Value::as_bool) == Some(true)).and_then(|l| {
                    let m = map.as_ref()?;
                    let a = m.sample_at(l.get("from").and_then(Value::as_int)?);
                    let b = m.sample_at(l.get("to").and_then(Value::as_int)?);
                    (b > a).then_some((a, b))
                });
                let q = transport.get("recordQuantize").filter(|q| q.get("on").and_then(Value::as_bool) == Some(true));
                let info = TakeInfo {
                    start_sample: sample,
                    start_frame: frame,
                    stop_sample: sample,
                    stop_frame: frame,
                    loop_range,
                    wraps: Vec::new(),
                    mode: transport.get("recordMode").and_then(Value::as_text).unwrap_or("overdub").to_owned(),
                    quantize: q.map(|q| Quantize {
                        grid: q.get("grid").and_then(Value::as_int).unwrap_or(240).max(1),
                        strength: q.get("strength").and_then(Value::as_real).unwrap_or(1.0),
                        starts_only: q.get("startsOnly").and_then(Value::as_bool).unwrap_or(false),
                    }),
                };
                let path = self.project.folder.audio().join(format!("rec-{}.midilog", lm_doc::new_uuid()));
                let file = std::fs::create_dir_all(self.project.folder.audio())
                    .and_then(|_| OpenOptions::new().create(true).append(true).open(&path));
                let mut file = match file {
                    Ok(f) => Some(f),
                    Err(err) => {
                        self.notices.push(
                            "midi.record.log_failed",
                            vec![("error", Value::text(err.to_string()))],
                            lm_port::Severity::Warning,
                            None,
                            Vec::new(),
                        );
                        None
                    }
                };
                if let Some(f) = file.as_mut() {
                    let _ = f.write_all(header(&info).as_bytes()).and_then(|_| f.sync_data());
                }
                self.midi.recorder.take =
                    Some(Take { info, path, file, routes, events: Vec::new(), unsynced: String::new(), last_sync: 0 });
            }
            TransportEvent::LoopWrapped { sample_in_take, .. } => {
                if let Some(t) = self.midi.recorder.take.as_mut() {
                    let frame = t.info.start_frame + sample_in_take;
                    t.info.wraps.push(frame);
                    t.unsynced.push_str(&format!("wrap {frame}\n"));
                }
            }
            TransportEvent::RecordStopped { sample, frame } => {
                if let Some(mut t) = self.midi.recorder.take.take() {
                    t.info.stop_sample = sample;
                    t.info.stop_frame = frame;
                    t.unsynced.push_str(&format!("stop {sample} {frame}\n"));
                    sync(&mut t);
                    self.midi.recorder.ready =
                        Some(Ready { info: t.info, events: t.events, path: t.path, recovered: false });
                }
            }
            _ => {}
        }
    }

    /// The messages that reached the instruments while recording
    pub(crate) fn record_midi(&mut self, recorded: Vec<RecordedMidi>) {
        let Some(t) = self.midi.recorder.take.as_mut() else { return };
        for r in recorded {
            let ch = r.bytes[0] & 0x0F;
            for route in &t.routes {
                if (!route.ports.is_empty() && !route.ports.contains(&r.port.0))
                    || route.channel.is_some_and(|c| c != ch)
                {
                    continue;
                }
                let bytes = if r.len < 3 { [r.bytes[0], r.bytes[1], 0] } else { r.bytes };
                t.events.push(Rec { track: route.track, sample: r.sample, frame: r.frame, bytes });
                t.unsynced.push_str(&format!(
                    "{} {} {} {} {} {}\n",
                    r.sample, r.frame, route.track, bytes[0], bytes[1], bytes[2]
                ));
            }
        }
    }

    /// Each pump: the log reaches the disk every second; a take without audio enters the tree by itself
    pub(crate) fn record_pump(&mut self, now: HostTime) {
        if let Some(t) = self.midi.recorder.take.as_mut()
            && now.0.saturating_sub(t.last_sync) >= SECOND
        {
            t.last_sync = now.0;
            sync(t);
        }
        // With audio, the MIDI take joins the audio takes' transaction (`commit_recording`); without, it goes alone
        if self.midi.recorder.ready.is_some() && self.recordings.is_empty() {
            let ready = self.midi.recorder.ready.take();
            if let Some(r) = ready {
                self.midi_commit_alone(r);
            }
        }
    }

    fn midi_commit_alone(&mut self, ready: Ready) {
        let map = match tempo_map(self.document.root()) {
            Ok(m) => m,
            Err(e) => {
                self.notices.error(&e, None, None);
                return;
            }
        };
        let label = Label::new(if ready.recovered { "midi.take.recovered" } else { "session.take" });
        let r = self.document.transact_as(label, Origin::Recording, |e| apply(&ready, &map, e));
        match r {
            Ok(_) => {
                if let Some(patch) = self.document.take_journal() {
                    self.write_journal(&patch);
                }
                let _ = std::fs::remove_file(&ready.path);
            }
            Err(e) => self.notices.error(&e, None, None),
        }
    }

    /// The MIDI take, for the audio takes' transaction (`commit_recording`): `None` when there is none
    pub(crate) fn midi_take_for_commit(&mut self) -> Option<(Ready, TempoMap)> {
        let ready = self.midi.recorder.ready.take()?;
        match tempo_map(self.document.root()) {
            Ok(m) => Some((ready, m)),
            Err(e) => {
                self.notices.error(&e, None, None);
                None
            }
        }
    }

    /// The MIDI takes a crash interrupted (U5): each log back into the tree, as "Take recuperado"
    pub(crate) fn recover_midi(&mut self) -> Result<usize> {
        let dir = self.project.folder.audio();
        let mut found = 0;
        let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(0) };
        let mut logs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "midilog"))
            .collect();
        logs.sort();
        for path in logs {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Some((info, events)) = read_log(&text) else { continue };
            if events.is_empty() {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            found += 1;
            self.midi_commit_alone(Ready { info, events, path, recovered: true });
        }
        Ok(found)
    }
}

/// Appends what waits to the log and `fsync`s it
fn sync(t: &mut Take) {
    if t.unsynced.is_empty() {
        return;
    }
    if let Some(f) = t.file.as_mut() {
        let _ = f.write_all(t.unsynced.as_bytes()).and_then(|_| f.sync_data());
    }
    t.unsynced.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> TempoMap {
        TempoMap::constant(
            48_000,
            lm_time::Ppq::DEFAULT,
            lm_time::Bpm::from_integer(120).unwrap(),
            lm_time::TimeSignature::COMMON,
        )
        .unwrap()
    }

    fn info() -> TakeInfo {
        TakeInfo {
            start_sample: 0,
            start_frame: 1000,
            stop_sample: 48_000,
            stop_frame: 49_000,
            loop_range: None,
            wraps: Vec::new(),
            mode: "overdub".into(),
            quantize: None,
        }
    }

    fn t() -> NodeId {
        NodeId::parse("t:1").unwrap()
    }

    fn ev(sample: i64, bytes: [u8; 3]) -> Rec {
        Rec { track: t(), sample, frame: 1000 + sample as u64, bytes }
    }

    #[test]
    fn notes_controls_and_held_keys() {
        // 120 BPM at 48 kHz: 960 ticks per 24 000 samples
        let events = vec![
            ev(0, [0x90, 60, 127]),
            ev(12_000, [0x80, 60, 0]),
            ev(6000, [0xB0, 1, 64]),
            ev(6000, [0xE0, 0, 0x40]),
            ev(7000, [0xD0, 9, 0]),
            ev(8000, [0xC0, 4, 0]),
            ev(24_000, [0x91, 62, 64]), // held: ends at the stop
        ];
        let mut sorted = events.clone();
        sorted.sort_by_key(|e| e.frame);
        let b = build(&info(), &sorted, &map());
        assert_eq!(b.len(), 1);
        let b = &b[0];
        assert_eq!((b.start, b.length), (0, 1920));
        assert_eq!(b.notes, vec![(60, 0, 480, 1.0), (62, 960, 960, 64.0 / 127.0)]);
        assert_eq!(b.controls[&("cc", 1, 0)], vec![(240, 64)]);
        assert_eq!(b.controls[&("pitchBend", 0, 0)], vec![(240, 8192)]);
        assert_eq!(b.controls[&("channelPressure", 0, 0)], vec![(280, 9)]);
        assert_eq!(b.controls[&("programChange", 0, 0)], vec![(320, 4)]);
    }

    #[test]
    fn loop_layers_drop_short_passes_and_quantize() {
        // A one-second loop (0..48 000): passes from frame 1000; wraps at 49 000 and 97 000; stop at 120 000
        let mut i = info();
        i.loop_range = Some((0, 48_000));
        i.wraps = vec![49_000, 97_000];
        i.stop_frame = 120_000;
        i.mode = "layers".into();
        i.quantize = Some(Quantize { grid: 240, strength: 1.0, starts_only: true });
        let e = |frame: u64, sample: i64, key: u8| Rec { track: t(), sample, frame, bytes: [0x90, key, 100] };
        let events = vec![e(2000, 1000, 60), e(50_000, 1000, 61), e(98_000, 1000, 62)];
        let b = build(&i, &events, &map());
        // The third pass (23 000 of 48 000 frames) is under 90 %: dropped; two layers, the first muted
        assert_eq!(b.len(), 2);
        assert!(b[0].muted && !b[1].muted);
        assert_eq!(b[0].notes[0].0, 60);
        assert_eq!(b[1].notes[0].0, 61);
        // 1000 samples = 40 ticks → quantized to 0
        assert_eq!(b[1].notes[0].1, 0);
        // Overdub merges the kept passes
        i.mode = "overdub".into();
        let b = build(&i, &events, &map());
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].notes.len(), 2);
    }

    #[test]
    fn the_log_reads_back_after_a_crash() {
        let mut i = info();
        i.quantize = Some(Quantize { grid: 120, strength: 0.5, starts_only: false });
        let mut text = header(&i);
        text.push_str("100 1100 t:1 144 60 100\n");
        text.push_str("wrap 5000\n");
        text.push_str("200 1200 t:1 128 60 0\n");
        text.push_str("300 13"); // cut in the middle of a line
        let (back, events) = read_log(&text).expect("log");
        assert_eq!(events.len(), 2);
        assert_eq!(back.wraps, vec![5000]);
        assert_eq!(back.quantize, i.quantize);
        assert_eq!(back.stop_frame, 1201, "no stop line: the take ends at its last message");
    }
}
