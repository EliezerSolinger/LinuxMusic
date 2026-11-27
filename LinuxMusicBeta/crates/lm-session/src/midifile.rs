//! MIDI files in and out of the project (owner: agent D; F-PRJ-019, F-PRJ-021, P-39), on `lm-smf`.
//!
//! Importing is two steps, as every long operation (I-2.2): the worker reads the file into a *plan* (a `Value`, in
//! the project's ticks), and the pump applies the plan in one transaction (one undo step):
//! - one track per (file track × channel), each with an instrument and **a desk channel of its own** (P-39);
//! - the notes as MIDI items, a new item where more than a bar of silence separates them (F-PRJ-019);
//! - CC7 becomes the track's volume automation, CC10 the pan automation of its channel (P-39);
//! - pitch bend, aftertouch and every other CC are control lanes of the MIDI items (played back with the notes), the
//!   bank select (CC0/CC32) and the first program change choose the preset of the instrument (channel 10: bank 128),
//!   the later program changes are `programChange` lanes;
//! - with a default SoundFont the instrument is a Sampler on it, else the placeholder synth (nothing to play it on);
//! - the tempo changes and the time signatures go to the tempo map.
//!
//! Exporting writes SMF type 1 at the project's resolution: a first track with the tempo map, then one track per
//! instrument track with the notes and the control lanes (bend, CC, pressure, program changes) of its MIDI items, the
//! channels in order skipping 10; a Sampler on a SoundFont or a GIG writes its bank select and program at the start,
//! and one on the drum bank (128) takes channel 10 (F-PRJ-021).

use std::path::Path;

use lm_base::{Error, Result};
use lm_smf::{Event, Smf, Timed, Track};
use lm_state::{NodeId, PathStep, ROOT, Value};

/// The instrument an imported track gets when there is no default SoundFont to play it on
pub const IMPORT_INSTRUMENT: &str = "tripleoscillator";

/// The most points one imported control lane keeps (a file may carry millions of bend events)
const LANE_POINTS_MAX: usize = 50_000;

/// A lane's key: the control kind and its number (0 unless a CC or a poly pressure)
type LaneKey = (&'static str, u8);

fn int(v: i64) -> Value {
    Value::int(v)
}

fn real(v: f64) -> Value {
    Value::real(v).unwrap_or(Value::NULL)
}

fn smf_error(e: lm_smf::SmfError) -> Error {
    Error::new("session.midi.read").arg("problem", e.key).arg("at", e.at as i64)
}

/// A tempo in µs per quarter as the tempo map's exact decimal text (up to 6 decimals, I-18)
pub fn bpm_text(us_per_quarter: u32) -> String {
    let bpm = 60_000_000.0 / f64::from(us_per_quarter.max(1));
    let s = format!("{:.6}", bpm.clamp(1.0, 1000.0));
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A tempo or time-signature change at a tick
type Change = (i64, Option<u32>, Option<(u8, u8)>);
/// A note: key, start, length, velocity 0..1, in project ticks
type Note = (i64, i64, i64, f64);

/// One imported track as the plan carries it
#[derive(Default)]
struct PlanTrack {
    name: String,
    channel: u8,
    program: Option<u8>,
    /// (key, start, length, velocity 0..1) in project ticks
    notes: Vec<Note>,
    volume: Vec<(i64, f64)>,
    pan: Vec<(i64, f64)>,
    /// bank select MSB and LSB seen so far
    bank: (Option<u8>, Option<u8>),
    /// the bank in effect when the first program change came
    program_bank: Option<u16>,
    /// (tick, value) of every other control, by lane
    controls: std::collections::BTreeMap<LaneKey, Vec<(i64, i64)>>,
}

impl PlanTrack {
    fn control(&mut self, key: LaneKey, tick: i64, value: i64) {
        self.controls.entry(key).or_default().push((tick, value));
    }

    /// The preset's bank: channel 10 is the drum bank (128), else CC0 × 128 + CC32
    fn bank_number(&self) -> i64 {
        if self.channel == 9 {
            return 128;
        }
        let (msb, lsb) = self.bank;
        match (msb, lsb) {
            (None, None) => i64::from(self.program_bank.unwrap_or(0)),
            (m, l) => i64::from(m.unwrap_or(0)) * 128 + i64::from(l.unwrap_or(0)),
        }
    }
}

/// Reads a MIDI file into the plan (the worker's part): `{ tempo: [point], tracks: [{ name, channel, program, items:
/// [{ start, length, notes: [[key, start, length, velocity]], controls: [{ kind, number, channel, points }] }], bank,
/// program, volume: [[tick, gain]], pan: [[tick, pan]] }], soundfont }`; `soundfont` is the file the Sampler of every
/// track opens (none: the placeholder synth)
pub fn read_plan(path: &Path, ppq: i64, soundfont: Option<&str>) -> Result<Value> {
    let bytes = std::fs::read(path).map_err(|e| Error::new("session.midi.read").arg("problem", e.to_string()))?;
    let smf = lm_smf::read(&bytes).map_err(smf_error)?;
    Ok(plan(&smf, ppq).with("soundfont", soundfont.map_or(Value::NULL, Value::text)))
}

/// The plan of a file already read
pub fn plan(smf: &Smf, ppq: i64) -> Value {
    let scale = |t: u64| ((t as i128 * i128::from(ppq) + i128::from(smf.ppq) / 2) / i128::from(smf.ppq.max(1))) as i64;
    // tempo and time signatures, from every track (format 1 keeps them in the first)
    let mut changes: Vec<Change> = Vec::new();
    for t in &smf.tracks {
        for e in &t.events {
            match e.event {
                Event::Tempo(us) => changes.push((scale(e.tick), Some(us), None)),
                Event::TimeSignature(n, d) => changes.push((scale(e.tick), None, Some((n, d)))),
                _ => {}
            }
        }
    }
    changes.sort_by_key(|c| c.0);
    let mut points: Vec<(i64, u32, (u8, u8))> = Vec::new();
    let (mut tempo, mut sig) = (500_000u32, (4u8, 4u8));
    for (tick, us, ts) in changes {
        if let Some(us) = us {
            tempo = us;
        }
        if let Some(ts) = ts {
            sig = ts;
        }
        match points.last_mut() {
            Some(last) if last.0 == tick => *last = (tick, tempo, sig),
            _ => points.push((tick, tempo, sig)),
        }
    }
    if points.first().is_none_or(|p| p.0 != 0) {
        points.insert(0, (0, 500_000, (4, 4)));
    }
    let tempo_list: Vec<Value> = if points.len() == 1 && points[0].1 == 500_000 && points[0].2 == (4, 4) {
        Vec::new() // the file says nothing: the project's map stays
    } else {
        points
            .iter()
            .map(|(tick, us, (n, d))| {
                Value::object([
                    ("tick", int(*tick)),
                    ("bpm", Value::text(bpm_text(*us))),
                    ("timeSig", Value::list([int(i64::from(*n)), int(i64::from(*d))])),
                    ("ramp", Value::bool(false)),
                ])
            })
            .collect()
    };

    // one track per (file track × channel)
    let mut out: Vec<PlanTrack> = Vec::new();
    for (ti, t) in smf.tracks.iter().enumerate() {
        let name = t.name().map(str::to_string);
        let mut by_channel: Vec<PlanTrack> = Vec::new();
        let mut open: Vec<(u8, u8, i64, u8)> = Vec::new(); // channel, key, start, velocity
        let get = |v: &mut Vec<PlanTrack>, ch: u8| -> usize {
            match v.iter().position(|p| p.channel == ch) {
                Some(i) => i,
                None => {
                    v.push(PlanTrack { channel: ch, ..PlanTrack::default() });
                    v.len() - 1
                }
            }
        };
        for Timed { tick, event } in &t.events {
            let tick = scale(*tick);
            match *event {
                Event::NoteOn { channel, key, velocity } => open.push((channel, key, tick, velocity)),
                Event::NoteOff { channel, key, .. } => {
                    // the oldest open note of that key ends (FIFO, so overlapping notes pair in order)
                    if let Some(i) = open.iter().position(|o| o.0 == channel && o.1 == key) {
                        let (ch, key, start, vel) = open.remove(i);
                        let p = get(&mut by_channel, ch);
                        by_channel[p].notes.push((
                            i64::from(key),
                            start,
                            (tick - start).max(1),
                            f64::from(vel) / 127.0,
                        ));
                    }
                }
                Event::Controller { channel, number: 7, value } => {
                    let p = get(&mut by_channel, channel);
                    let g = f64::from(value) / 127.0;
                    by_channel[p].volume.push((tick, g * g));
                }
                Event::Controller { channel, number: 10, value } => {
                    let p = get(&mut by_channel, channel);
                    by_channel[p].pan.push((tick, ((f64::from(value) - 64.0) / 63.0).clamp(-1.0, 1.0)));
                }
                Event::Controller { channel, number: 0, value } => {
                    let p = get(&mut by_channel, channel);
                    by_channel[p].bank.0 = Some(value);
                }
                Event::Controller { channel, number: 32, value } => {
                    let p = get(&mut by_channel, channel);
                    by_channel[p].bank.1 = Some(value);
                }
                Event::Controller { channel, number, value } => {
                    let p = get(&mut by_channel, channel);
                    by_channel[p].control(("cc", number), tick, i64::from(value));
                }
                Event::PitchBend { channel, value } => {
                    let p = get(&mut by_channel, channel);
                    by_channel[p].control(("pitchBend", 0), tick, i64::from(value.min(16383)));
                }
                Event::ChannelPressure { channel, value } => {
                    let p = get(&mut by_channel, channel);
                    by_channel[p].control(("channelPressure", 0), tick, i64::from(value));
                }
                Event::PolyPressure { channel, key, value } => {
                    let p = get(&mut by_channel, channel);
                    by_channel[p].control(("polyPressure", key), tick, i64::from(value));
                }
                Event::Program { channel, program } => {
                    let p = get(&mut by_channel, channel);
                    if by_channel[p].program.is_none() {
                        by_channel[p].program = Some(program);
                        let (m, l) = by_channel[p].bank;
                        by_channel[p].program_bank = Some(u16::from(m.unwrap_or(0)) * 128 + u16::from(l.unwrap_or(0)));
                    } else {
                        by_channel[p].control(("programChange", 0), tick, i64::from(program));
                    }
                }
                _ => {}
            }
        }
        // notes still held at the end of the track end there
        let end = t.events.last().map_or(0, |e| scale(e.tick));
        for (ch, key, start, vel) in open {
            let p = get(&mut by_channel, ch);
            by_channel[p].notes.push((i64::from(key), start, (end - start).max(1), f64::from(vel) / 127.0));
        }
        let several = by_channel.iter().filter(|p| !p.notes.is_empty()).count() > 1;
        for mut p in by_channel.into_iter().filter(|p| !p.notes.is_empty()) {
            let base = name.clone().unwrap_or_else(|| format!("MIDI {}", ti + 1));
            p.name = if p.channel == 9 {
                format!("{base} (bateria)")
            } else if several {
                format!("{base} ({})", p.channel + 1)
            } else {
                base
            };
            out.push(p);
        }
    }

    let bar = ppq * 4;
    let tracks: Vec<Value> = out
        .into_iter()
        .map(|mut p| {
            p.notes.sort_by_key(|n| (n.1, n.0));
            // items: a new one after more than a bar of silence
            let mut items: Vec<(i64, i64, Vec<Note>)> = Vec::new();
            for n in &p.notes {
                let end = n.1 + n.2;
                match items.last_mut() {
                    Some((_, last_end, notes)) if n.1 - *last_end <= bar => {
                        *last_end = (*last_end).max(end);
                        notes.push(*n);
                    }
                    _ => items.push((n.1, end, vec![*n])),
                }
            }
            let mut spans: Vec<(i64, i64, Vec<Note>)> = items
                .into_iter()
                .map(|(first, last_end, notes)| {
                    let start = first / bar * bar;
                    let length = ((last_end - start + ppq - 1) / ppq * ppq).max(ppq);
                    (start, length, notes)
                })
                .collect();
            // The controls: each point goes to the last item that starts at or before it (the first item for the
            // earlier ones); an item grows to hold the points after its end, never into the next item
            let mut lanes: Vec<std::collections::BTreeMap<LaneKey, Vec<(i64, i64)>>> =
                vec![std::collections::BTreeMap::new(); spans.len()];
            if !spans.is_empty() {
                for (key, points) in &p.controls {
                    let mut last = None;
                    let mut kept = 0;
                    for &(tick, value) in points {
                        // a repeated value says nothing new
                        if last == Some(value) {
                            continue;
                        }
                        if kept >= LANE_POINTS_MAX {
                            break;
                        }
                        last = Some(value);
                        kept += 1;
                        let i = spans.iter().rposition(|s| s.0 <= tick).unwrap_or(0);
                        let start = spans[i].0;
                        let next = spans.get(i + 1).map_or(i64::MAX, |n| n.0);
                        let wanted = ((tick.max(start) + 1 - start + ppq - 1) / ppq * ppq).min(next - start);
                        if wanted > spans[i].1 {
                            spans[i].1 = wanted;
                        }
                        lanes[i].entry(*key).or_default().push(((tick - start).max(0), value));
                    }
                }
            }
            let items: Vec<Value> = spans
                .into_iter()
                .zip(lanes)
                .map(|((start, length, notes), lanes)| {
                    let notes =
                        notes.into_iter().map(|(k, s, l, v)| Value::list([int(k), int(s - start), int(l), real(v)]));
                    let controls = lanes.into_iter().map(|((kind, number), points)| {
                        Value::object([
                            ("kind", Value::text(kind)),
                            ("number", int(i64::from(number))),
                            ("channel", int(0)),
                            (
                                "points",
                                Value::list(points.into_iter().map(|(t, v)| {
                                    Value::object([("tick", int(t)), ("value", int(v))])
                                })),
                            ),
                        ])
                    });
                    Value::object([
                        ("start", int(start)),
                        ("length", int(length)),
                        ("notes", Value::list(notes)),
                        ("controls", Value::list(controls)),
                    ])
                })
                .collect();
            let points = |v: &[(i64, f64)]| Value::list(v.iter().map(|(t, x)| Value::list([int(*t), real(*x)])));
            let bank = if p.program.is_some() || p.channel == 9 { int(p.bank_number()) } else { Value::NULL };
            Value::object([
                ("name", Value::text(p.name)),
                ("channel", int(i64::from(p.channel))),
                ("program", p.program.map_or(Value::NULL, |x| int(i64::from(x)))),
                ("bank", bank),
                ("items", Value::list(items)),
                ("volume", points(&p.volume)),
                ("pan", points(&p.pan)),
            ])
        })
        .collect();
    Value::object([("tempo", Value::list(tempo_list)), ("tracks", Value::list(tracks))])
}

/// Applies a plan in a transaction (the pump's part): returns the tracks made. `at`: the tick the file starts at
pub fn apply_plan(plan: &Value, at: i64, e: &mut lm_doc::Edit<'_>) -> Result<Vec<NodeId>> {
    let tempo: Vec<Value> = plan.get("tempo").map(|l| l.items().cloned().collect()).unwrap_or_default();
    if !tempo.is_empty() && at == 0 {
        e.set(ROOT, &[PathStep::key("tempoMap")], Value::list(tempo))?;
    }
    let mut made = Vec::new();
    for t in plan.get("tracks").into_iter().flat_map(Value::items) {
        let name = t.get("name").and_then(Value::as_text).unwrap_or("MIDI").to_string();
        let channels = e.root().get("mixer").and_then(|m| m.get("channels")).map_or(0, Value::len);
        let ch = e.insert(ROOT, "mixer.channels", channels, Value::object([("name", Value::text(name.clone()))]))?;
        // A Sampler on the default SoundFont (the file is loaded after the transaction), at the preset the file chose
        let instrument = if plan.get("soundfont").and_then(Value::as_text).is_some() {
            let g = |k: &str| t.get(k).and_then(Value::as_int).unwrap_or(0).clamp(0, 16383);
            Value::object([
                ("plugin", Value::text(lm_plugins_sampler::ID)),
                ("version", int(1)),
                (
                    "state",
                    Value::object([
                        ("format", Value::text("sf2")),
                        ("bank", int(g("bank"))),
                        ("program", int(g("program").min(127))),
                    ]),
                ),
            ])
        } else {
            Value::object([
                ("plugin", Value::text(IMPORT_INSTRUMENT)),
                ("version", int(1)),
                ("state", Value::empty_object()),
            ])
        };
        let output =
            Value::object([("port", Value::text("main")), ("channels", int(2)), ("to", Value::text(ch.to_string()))]);
        let tracks = e.root().get("tracks").map_or(0, Value::len);
        let track = e.insert(
            ROOT,
            "tracks",
            tracks,
            Value::object([
                ("name", Value::text(name)),
                ("instrument", instrument),
                ("outputs", Value::list([output])),
            ]),
        )?;
        for (i, item) in t.get("items").into_iter().flat_map(Value::items).enumerate() {
            let start = item.get("start").and_then(Value::as_int).unwrap_or(0) + at;
            let length = item.get("length").and_then(Value::as_int).unwrap_or(3840);
            let mut node =
                Value::object([("kind", Value::text("midi")), ("start", int(start)), ("length", int(length))]);
            let controls = item.get("controls").filter(|c| !c.is_empty());
            if let Some(c) = controls {
                node = node.with("controls", c.clone());
            }
            let it = e.insert(track, "items", i, node)?;
            for n in item.get("notes").into_iter().flat_map(Value::items) {
                let g = |k: usize| n.at(k).and_then(Value::as_int).unwrap_or(0);
                let vel = n.at(3).and_then(Value::as_real).unwrap_or(0.8).clamp(0.0, 1.0);
                e.add_note(
                    it,
                    Value::object([
                        ("key", int(g(0))),
                        ("start", int(g(1))),
                        ("length", int(g(2).max(1))),
                        ("velocity", real(vel)),
                    ]),
                )?;
            }
        }
        let lane = |owner: NodeId, param: &str, pts: &Value| {
            let points: Vec<Value> = pts
                .items()
                .map(|p| {
                    Value::object([
                        ("tick", int(p.at(0).and_then(Value::as_int).unwrap_or(0) + at)),
                        ("value", p.at(1).cloned().unwrap_or(Value::NULL)),
                        ("shape", Value::text("step")),
                    ])
                })
                .collect();
            Value::object([
                ("target", Value::object([("node", Value::text(owner.to_string())), ("param", Value::text(param))])),
                ("points", Value::list(points)),
            ])
        };
        if let Some(v) = t.get("volume").filter(|v| !v.is_empty()) {
            e.insert(track, "automation", 0, lane(track, "volume", v))?;
        }
        if let Some(p) = t.get("pan").filter(|v| !v.is_empty()) {
            e.insert(ch, "automation", 0, lane(ch, "pan", p))?;
        }
        made.push(track);
    }
    Ok(made)
}

/// The project's notes as a type-1 MIDI file (F-PRJ-021): the tempo map in the first track, then one track per
/// instrument track (or only `only`, "export the track as MIDI"), channels in order skipping 10
pub fn write_smf(root: &Value, only: Option<NodeId>) -> Vec<u8> {
    write_smf_of(root, only, None)
}

/// [`write_smf`] with only one item's notes ("export the clip as MIDI", F-CLP-008): the item's track alone, the item
/// at the file's start
pub fn write_smf_of(root: &Value, only: Option<NodeId>, only_item: Option<NodeId>) -> Vec<u8> {
    let ppq = root.get("project").and_then(|p| p.get("ppq")).and_then(Value::as_int).unwrap_or(960).clamp(1, 32767);
    let mut tempo = Track::default();
    for p in root.get("tempoMap").into_iter().flat_map(Value::items) {
        let tick = p.get("tick").and_then(Value::as_int).unwrap_or(0).max(0) as u64;
        let bpm: f64 = p.get("bpm").and_then(Value::as_text).and_then(|t| t.parse().ok()).unwrap_or(120.0);
        tempo.events.push(Timed { tick, event: Event::Tempo((60_000_000.0 / bpm.max(1.0)).round() as u32) });
        if let Some(sig) = p.get("timeSig") {
            let n = sig.at(0).and_then(Value::as_int).unwrap_or(4) as u8;
            let d = sig.at(1).and_then(Value::as_int).unwrap_or(4) as u8;
            tempo.events.push(Timed { tick, event: Event::TimeSignature(n, d) });
        }
    }
    let mut tracks = vec![tempo];
    let mut channel = 0u8;
    for t in root.get("tracks").into_iter().flat_map(Value::items) {
        let id = t.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok());
        if t.get("instrument").is_none_or(Value::is_null) || only.is_some_and(|o| Some(o) != id) {
            continue;
        }
        // A Sampler's preset: bank and program (bank 128, the drums, plays on channel 10)
        let preset = sampler_preset(t.get("instrument"));
        let drums = preset.is_some_and(|(bank, _)| bank >= 128);
        let ch = if drums {
            9
        } else {
            let ch = channel;
            channel = if channel == 8 { 10 } else { (channel + 1) % 16 };
            ch
        };
        // the velocity follows the track's volume, as the legacy's export
        let volume = t.get("volume").and_then(Value::as_real).unwrap_or(1.0).clamp(0.0, 1.0);
        let mut out = Track::default();
        let name = t.get("name").and_then(Value::as_text).unwrap_or("").to_string();
        out.events.push(Timed { tick: 0, event: Event::TrackName(name) });
        if let Some((bank, program)) = preset {
            if !drums && bank > 0 {
                out.events.push(Timed {
                    tick: 0,
                    event: Event::Controller { channel: ch, number: 0, value: ((bank >> 7) & 0x7f) as u8 },
                });
                out.events.push(Timed {
                    tick: 0,
                    event: Event::Controller { channel: ch, number: 32, value: (bank & 0x7f) as u8 },
                });
            }
            out.events.push(Timed { tick: 0, event: Event::Program { channel: ch, program } });
        }
        for item in t.get("items").into_iter().flat_map(Value::items) {
            let item_id = item.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok());
            if item.get("kind").and_then(Value::as_text) != Some("midi")
                || (only_item.is_none() && item.get("mute").and_then(Value::as_bool) == Some(true))
                || only_item.is_some_and(|o| Some(o) != item_id)
            {
                continue;
            }
            let start = if only_item.is_some() { 0 } else { item.get("start").and_then(Value::as_int).unwrap_or(0) };
            let length = item.get("length").and_then(Value::as_int).unwrap_or(0);
            for lane in item.get("controls").into_iter().flat_map(Value::items) {
                control_events(lane, ch, start, length, &mut out.events);
            }
            let Some(notes) = item.get("notes") else { continue };
            for (_, n) in notes.entries() {
                let at = n.get("start").and_then(Value::as_int).unwrap_or(0);
                if at >= length {
                    continue;
                }
                let key = n.get("key").and_then(Value::as_int).unwrap_or(60).clamp(0, 127) as u8;
                let len = n.get("length").and_then(Value::as_int).unwrap_or(1).min(length - at).max(1);
                let vel = n.get("velocity").and_then(Value::as_real).unwrap_or(0.8) * volume;
                let velocity = ((vel * 127.0).round() as u8).clamp(1, 127);
                let on = (start + at).max(0) as u64;
                out.events.push(Timed { tick: on, event: Event::NoteOn { channel: ch, key, velocity } });
                out.events
                    .push(Timed { tick: on + len as u64, event: Event::NoteOff { channel: ch, key, velocity: 64 } });
            }
        }
        tracks.push(out);
    }
    lm_smf::write(&Smf { format: 1, ppq: ppq as u16, tracks })
}

/// The bank and program of a Sampler playing a SoundFont or a GIG; `None` for any other instrument
fn sampler_preset(instrument: Option<&Value>) -> Option<(u16, u8)> {
    let i = instrument?;
    if i.get("plugin").and_then(Value::as_text) != Some(lm_plugins_sampler::ID) {
        return None;
    }
    let state = i.get("state")?;
    if !matches!(state.get("format").and_then(Value::as_text), Some("sf2" | "gig")) {
        return None;
    }
    let bank = state.get("bank").and_then(Value::as_int).unwrap_or(0).clamp(0, 16383) as u16;
    let program = state.get("program").and_then(Value::as_int).unwrap_or(0).clamp(0, 127) as u8;
    Some((bank, program))
}

/// One control lane of an item as SMF events on `channel`, from `start`; the points at or after the item's end
/// are not played, so they are not written
fn control_events(lane: &Value, channel: u8, start: i64, length: i64, out: &mut Vec<Timed>) {
    let kind = lane.get("kind").and_then(Value::as_text).unwrap_or("cc");
    let number = lane.get("number").and_then(Value::as_int).unwrap_or(0).clamp(0, 127) as u8;
    for p in lane.get("points").into_iter().flat_map(Value::items) {
        let at = p.get("tick").and_then(Value::as_int).unwrap_or(0);
        if at < 0 || at >= length {
            continue;
        }
        let v = p.get("value").and_then(Value::as_int).unwrap_or(0).clamp(0, 16383);
        let v7 = v.min(127) as u8;
        let event = match kind {
            "pitchBend" => Event::PitchBend { channel, value: v as u16 },
            "channelPressure" => Event::ChannelPressure { channel, value: v7 },
            "polyPressure" => Event::PolyPressure { channel, key: number, value: v7 },
            "programChange" => Event::Program { channel, program: v7 },
            _ => Event::Controller { channel, number, value: v7 },
        };
        out.push(Timed { tick: (start + at).max(0) as u64, event });
    }
}

impl crate::session::Session {
    /// "midi.import": reads `path` in an operation; the pump applies it at `at` (F-PRJ-019)
    pub fn midi_import(&mut self, path: std::path::PathBuf, at: i64) -> lm_base::OperationId {
        let ppq =
            self.document().root().get("project").and_then(|p| p.get("ppq")).and_then(Value::as_int).unwrap_or(960);
        let soundfont = self.default_soundfont().map(|p| p.display().to_string());
        self.start(
            lm_base::Label::new("session.midi.importing"),
            lm_port::Target::Project,
            true,
            "midi_import",
            move |cancel, _| {
                cancel.check()?;
                let p = read_plan(&path, ppq, soundfont.as_deref())?;
                Ok(Value::object([("plan", p), ("at", int(at))]))
            },
        )
    }

    /// "midi.export": writes the project's (or one track's) notes to `path` in an operation (F-PRJ-021)
    pub fn midi_export(&mut self, path: std::path::PathBuf, only: Option<NodeId>) -> lm_base::OperationId {
        self.midi_export_of(path, only, None)
    }

    /// [`Session::midi_export`] of one item only (F-CLP-008)
    pub fn midi_export_of(
        &mut self,
        path: std::path::PathBuf,
        only: Option<NodeId>,
        item: Option<NodeId>,
    ) -> lm_base::OperationId {
        let root = self.document().root().clone();
        self.start(
            lm_base::Label::new("session.midi.exporting"),
            lm_port::Target::Project,
            false,
            "midi_export",
            move |_, _| {
                let bytes = write_smf_of(&root, only, item);
                // A name of its own (two exports at once do not collide), removed if anything fails
                let tmp = path.with_extension(format!("{}.mid.tmp", lm_doc::new_uuid()));
                std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    Error::new("session.midi.write")
                        .arg("file", path.display().to_string())
                        .arg("problem", e.to_string())
                })?;
                Ok(Value::text(path.display().to_string()))
            },
        )
    }

    /// The import's plan, applied in one transaction (one undo step)
    pub(crate) fn midi_import_finished(&mut self, result: Value) {
        let at = result.get("at").and_then(Value::as_int).unwrap_or(0);
        let Some(plan) = result.get("plan").cloned() else { return };
        let mut made = Vec::new();
        let r = self.document_mut().transact(lm_base::Label::new("session.midi.import"), |e| {
            made = apply_plan(&plan, at, e)?;
            Ok(())
        });
        match r {
            Ok(_) => {
                // every Sampler opens the default SoundFont (the file is imported once: it is hashed)
                if let Some(file) = plan.get("soundfont").and_then(Value::as_text) {
                    for track in made {
                        let params = Value::object([
                            ("node", Value::text(track.to_string())),
                            ("field", Value::text("soundfont")),
                            ("path", Value::text(file)),
                        ]);
                        if let Err(e) = self.load_into_field(&params) {
                            self.notices().error(&e, Some(lm_port::Target::Project), None);
                        }
                    }
                }
                let n = plan.get("tracks").map_or(0, Value::len) as i64;
                self.notices().info("session.midi.imported", vec![("tracks", int(n))]);
            }
            Err(e) => self.notices().error(&e, Some(lm_port::Target::Project), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_text_is_exact_enough() {
        assert_eq!(bpm_text(500_000), "120");
        assert_eq!(bpm_text(400_000), "150");
        assert_eq!(bpm_text(600_000), "100");
    }

    #[test]
    fn a_plan_splits_channels_items_and_controllers() {
        let t = |tick, event| Timed { tick, event };
        let smf = Smf {
            format: 1,
            ppq: 96,
            tracks: vec![Track {
                events: vec![
                    t(0, Event::TrackName("Piano".into())),
                    t(0, Event::Controller { channel: 0, number: 7, value: 127 }),
                    t(0, Event::Controller { channel: 0, number: 10, value: 0 }),
                    t(0, Event::NoteOn { channel: 0, key: 60, velocity: 127 }),
                    t(96, Event::NoteOff { channel: 0, key: 60, velocity: 0 }),
                    // more than a bar (4 × 96) later: a second item
                    t(96 * 10, Event::NoteOn { channel: 0, key: 62, velocity: 64 }),
                    t(96 * 11, Event::NoteOff { channel: 0, key: 62, velocity: 0 }),
                    t(0, Event::NoteOn { channel: 9, key: 36, velocity: 100 }),
                    t(48, Event::NoteOff { channel: 9, key: 36, velocity: 0 }),
                ],
            }],
        };
        let p = plan(&smf, 960);
        let tracks: Vec<&Value> = p.get("tracks").unwrap().items().collect();
        assert_eq!(tracks.len(), 2, "one per channel");
        assert_eq!(tracks[0].get("items").unwrap().len(), 2, "split after a bar of silence");
        assert_eq!(tracks[1].get("name").and_then(Value::as_text), Some("Piano (bateria)"));
        let vol = tracks[0].get("volume").unwrap().at(0).unwrap().at(1).and_then(Value::as_real).unwrap();
        assert!((vol - 1.0).abs() < 1e-9);
        let pan = tracks[0].get("pan").unwrap().at(0).unwrap().at(1).and_then(Value::as_real).unwrap();
        assert_eq!(pan, -1.0);
        // the file chose no program: no bank either (the drum track still has bank 128)
        assert_eq!(tracks[0].get("bank"), Some(&Value::NULL));
        assert_eq!(tracks[1].get("bank").and_then(Value::as_int), Some(128));
        let second = tracks[0].get("items").unwrap().at(1).unwrap();
        assert_eq!(second.get("start").and_then(Value::as_int), Some(9600 / 3840 * 3840));
    }

    #[test]
    fn a_plan_keeps_bend_controls_bank_and_program() {
        let t = |tick, event| Timed { tick, event };
        let smf = Smf {
            format: 1,
            ppq: 96,
            tracks: vec![Track {
                events: vec![
                    t(0, Event::Controller { channel: 2, number: 0, value: 1 }),
                    t(0, Event::Controller { channel: 2, number: 32, value: 5 }),
                    t(0, Event::Program { channel: 2, program: 40 }),
                    t(0, Event::NoteOn { channel: 2, key: 60, velocity: 100 }),
                    t(0, Event::PitchBend { channel: 2, value: 8192 }),
                    t(48, Event::PitchBend { channel: 2, value: 12000 }),
                    t(60, Event::PitchBend { channel: 2, value: 12000 }),
                    t(96, Event::NoteOff { channel: 2, key: 60, velocity: 0 }),
                    t(96, Event::Controller { channel: 2, number: 1, value: 90 }),
                    t(100, Event::Program { channel: 2, program: 41 }),
                ],
            }],
        };
        let p = plan(&smf, 960);
        let track = p.get("tracks").unwrap().at(0).unwrap();
        assert_eq!(track.get("bank").and_then(Value::as_int), Some(128 + 5));
        assert_eq!(track.get("program").and_then(Value::as_int), Some(40));
        let item = track.get("items").unwrap().at(0).unwrap();
        let lanes: Vec<&Value> = item.get("controls").unwrap().items().collect();
        let bend = lanes.iter().find(|l| l.get("kind").and_then(Value::as_text) == Some("pitchBend")).unwrap();
        assert_eq!(bend.get("points").unwrap().len(), 2, "the repeated value is dropped");
        assert!(lanes.iter().any(|l| l.get("kind").and_then(Value::as_text) == Some("programChange")));
        // the points after the notes grow the item, which still starts on the bar
        assert!(item.get("length").and_then(Value::as_int).unwrap() >= 960);
    }

    #[test]
    fn export_writes_bank_program_and_controls() {
        let root = Value::object([
            ("project", Value::object([("ppq", int(960))])),
            (
                "tracks",
                Value::list([Value::object([
                    ("name", Value::text("Strings")),
                    (
                        "instrument",
                        Value::object([
                            ("plugin", Value::text("sampler")),
                            (
                                "state",
                                Value::object([("format", Value::text("sf2")), ("bank", int(133)), ("program", int(40))]),
                            ),
                        ]),
                    ),
                    (
                        "items",
                        Value::list([Value::object([
                            ("kind", Value::text("midi")),
                            ("start", int(0)),
                            ("length", int(3840)),
                            (
                                "controls",
                                Value::list([Value::object([
                                    ("kind", Value::text("pitchBend")),
                                    ("number", int(0)),
                                    ("channel", int(0)),
                                    ("points", Value::list([Value::object([("tick", int(10)), ("value", int(9000))])])),
                                ])]),
                            ),
                        ])]),
                    ),
                ])]),
            ),
        ]);
        let smf = lm_smf::read(&write_smf(&root, None)).unwrap();
        let events: Vec<&Event> = smf.tracks[1].events.iter().map(|e| &e.event).collect();
        assert!(events.contains(&&Event::Controller { channel: 0, number: 0, value: 1 }));
        assert!(events.contains(&&Event::Controller { channel: 0, number: 32, value: 5 }));
        assert!(events.contains(&&Event::Program { channel: 0, program: 40 }));
        assert!(events.contains(&&Event::PitchBend { channel: 0, value: 9000 }));
    }
}
