//! Typed readings of the project tree (the port's `root()`, a `Value`), by the schemas' keys (lm-doc's project
//! schemas). The views are cheap copies (a `Value` clone is O(1)) made when a page renders; nothing here keeps a
//! reference into the core. Writing is never here: the pages send `EditOp`s through the port.

use std::sync::Arc;

use lm_port::{NodeId, Resource, Value};
use lm_time::TempoMap;

pub fn id_of(v: &Value) -> Option<NodeId> {
    v.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok())
}

pub fn ref_of(v: &Value, key: &str) -> Option<NodeId> {
    v.get(key).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok())
}

pub fn text(v: &Value, key: &str) -> Arc<str> {
    v.get(key).and_then(Value::as_shared_text).cloned().unwrap_or_else(|| Arc::from(""))
}

pub fn real(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_real).unwrap_or(default)
}

pub fn int(v: &Value, key: &str, default: i64) -> i64 {
    v.get(key).and_then(Value::as_int).unwrap_or(default)
}

pub fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn list<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> + 'a {
    v.get(key).into_iter().flat_map(Value::items)
}

pub fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for k in path {
        cur = cur.get(k)?;
    }
    Some(cur)
}

/// The project's sample rate and ppq
pub fn sample_rate(root: &Value) -> u32 {
    at(root, &["project", "sampleRate"]).and_then(Value::as_int).unwrap_or(48_000) as u32
}

pub fn ppq(root: &Value) -> u32 {
    at(root, &["project", "ppq"]).and_then(Value::as_int).unwrap_or(960) as u32
}

/// The tempo map of a tree the caller already holds (an export's snapshot, a function that got only the root): the
/// core's own reading of it (`lm_port::tempo_map_or_default`), never one of the interface's. Where the port is at hand
/// the screens ask `CorePort::tempo_map`, which is kept by revision
pub fn tempo_map(root: &Value) -> TempoMap {
    lm_port::tempo_map_or_default(root)
}

#[derive(Clone, Debug)]
pub struct OutputView {
    pub id: NodeId,
    pub port: Arc<str>,
    pub channels: i64,
    pub to: NodeId,
}

#[derive(Clone, Debug)]
pub struct LaneView {
    pub id: NodeId,
    pub name: Arc<str>,
    pub source: Option<Resource>,
    pub offset: i64,
    /// The silences a capture xrun left (source frame, frames), marked red on the take (F-REC-084)
    pub gaps: Vec<(i64, i64)>,
}

#[derive(Clone, Debug)]
pub struct CompView {
    pub from: i64,
    pub to: i64,
    pub lane: Option<NodeId>,
    pub fade: i64,
}

#[derive(Clone, Debug)]
pub struct ItemView {
    pub id: NodeId,
    /// 0xRRGGBB, or none: the track's (F-CLP-001)
    pub color: Option<u32>,
    pub audio: bool,
    pub name: Arc<str>,
    pub start: i64,
    pub length: i64,
    pub mute: bool,
    pub fade_in: i64,
    pub fade_out: i64,
    /// The fades' tension, -1..1 (0: straight)
    pub fade_in_curve: f64,
    pub fade_out_curve: f64,
    pub gain: f64,
    pub lanes: Vec<LaneView>,
    pub comp: Vec<CompView>,
    /// (key, start, length, velocity) of the MIDI notes
    pub notes: Vec<(i64, i64, i64, f64)>,
}

impl ItemView {
    pub fn lane(&self, id: NodeId) -> Option<&LaneView> {
        self.lanes.iter().find(|l| l.id == id)
    }
}

#[derive(Clone, Debug)]
pub struct AutomationView {
    pub id: NodeId,
    /// The parameter's node (the track, a channel or a plugin) and key
    pub node: Option<NodeId>,
    pub param: Arc<str>,
    pub on: bool,
    /// Off, Read, Touch, Latch or Write (I-21.5); absent is Read
    pub mode: Arc<str>,
    /// (tick, value, curve)
    pub points: Vec<(i64, f64, f64)>,
}

#[derive(Clone, Debug)]
pub struct TrackView {
    pub index: usize,
    pub id: NodeId,
    /// 0xRRGGBB, or none: the palette's for its place (F-TRK-001)
    pub color: Option<u32>,
    pub name: Arc<str>,
    pub folder: bool,
    pub collapsed: bool,
    pub parent: Option<NodeId>,
    pub volume: f64,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub monitor: Arc<str>,
    /// the input's channels (1-based): none, one, or a stereo pair; for the input meter and the input menu
    pub input: Vec<i64>,
    pub instrument: Option<(NodeId, Arc<str>)>,
    /// The instrument plays a drum kit (a row per piece): the Sampler on a kit, or a kit instrument of the fake core
    pub kit: bool,
    pub outputs: Vec<OutputView>,
    pub items: Vec<ItemView>,
    pub automation: Vec<AutomationView>,
}

/// The `color` field (0xRRGGBB); 0 or absent is the default one
fn int_color_of(v: &Value) -> Option<u32> {
    v.get("color").and_then(Value::as_int).filter(|c| *c != 0).map(|c| c as u32)
}

fn item_view(v: &Value) -> Option<ItemView> {
    let lanes = list(v, "lanes")
        .filter_map(|l| {
            Some(LaneView {
                id: id_of(l)?,
                name: text(l, "name"),
                source: l.get("source").and_then(Value::as_resource).cloned(),
                offset: int(l, "sourceOffset", 0),
                gaps: list(l, "gaps").map(|g| (int(g, "frame", 0), int(g, "frames", 1))).collect(),
            })
        })
        .collect();
    let comp = list(v, "comp")
        .map(|c| CompView {
            from: int(c, "from", 0),
            to: int(c, "to", 0),
            lane: ref_of(c, "lane"),
            fade: int(c, "fade", 240),
        })
        .collect();
    let notes = v
        .get("notes")
        .map(|m| {
            m.entries()
                .map(|(_, n)| (int(n, "key", 60), int(n, "start", 0), int(n, "length", 240), real(n, "velocity", 0.8)))
                .collect()
        })
        .unwrap_or_default();
    Some(ItemView {
        id: id_of(v)?,
        color: int_color_of(v),
        audio: v.get("kind").and_then(Value::as_text) == Some("audio"),
        name: text(v, "name"),
        start: int(v, "start", 0),
        length: int(v, "length", 3840),
        mute: flag(v, "mute"),
        fade_in: int(v, "fadeIn", 0),
        fade_out: int(v, "fadeOut", 0),
        fade_in_curve: real(v, "fadeInCurve", 0.0).clamp(-1.0, 1.0),
        fade_out_curve: real(v, "fadeOutCurve", 0.0).clamp(-1.0, 1.0),
        gain: real(v, "gain", 1.0),
        lanes,
        comp,
        notes,
    })
}

pub fn tracks(root: &Value) -> Vec<TrackView> {
    list(root, "tracks")
        .enumerate()
        .filter_map(|(index, t)| {
            Some(TrackView {
                index,
                id: id_of(t)?,
                color: int_color_of(t),
                name: text(t, "name"),
                folder: flag(t, "folder"),
                collapsed: flag(t, "collapsed"),
                parent: ref_of(t, "parent"),
                volume: real(t, "volume", 1.0),
                mute: flag(t, "mute"),
                solo: flag(t, "solo"),
                armed: flag(t, "armed"),
                monitor: t.get("monitor").and_then(Value::as_shared_text).cloned().unwrap_or_else(|| Arc::from("auto")),
                input: t
                    .get("input")
                    .filter(|i| !i.is_null())
                    .and_then(|i| i.get("channels"))
                    .map(|c| c.items().filter_map(Value::as_int).collect())
                    .unwrap_or_default(),
                instrument: t
                    .get("instrument")
                    .filter(|i| !i.is_null())
                    .and_then(|i| Some((id_of(i)?, text(i, "plugin")))),
                kit: t.get("instrument").filter(|i| !i.is_null()).is_some_and(|i| match text(i, "plugin").as_ref() {
                    "avldrums" | "drummachines" => true,
                    "sampler" => i.get("state").and_then(|s| s.get("format")).and_then(Value::as_text) == Some("kit"),
                    _ => false,
                }),
                outputs: list(t, "outputs")
                    .filter_map(|o| {
                        Some(OutputView {
                            id: id_of(o)?,
                            port: text(o, "port"),
                            channels: int(o, "channels", 2),
                            to: ref_of(o, "to")?,
                        })
                    })
                    .collect(),
                items: list(t, "items").filter_map(item_view).collect(),
                automation: list(t, "automation")
                    .filter_map(|a| {
                        Some(AutomationView {
                            id: id_of(a)?,
                            node: a.get("target").and_then(|tg| ref_of(tg, "node")),
                            param: a.get("target").map(|tg| text(tg, "param")).unwrap_or_else(|| Arc::from("")),
                            on: a.get("on").and_then(Value::as_bool).unwrap_or(true),
                            mode: a
                                .get("mode")
                                .and_then(Value::as_shared_text)
                                .cloned()
                                .unwrap_or_else(|| Arc::from("read")),
                            points: list(a, "points")
                                .map(|p| (int(p, "tick", 0), real(p, "value", 0.0), real(p, "curve", 0.0)))
                                .collect(),
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    /// A channel only other channels output to: a group (P-18)
    Group,
    /// A channel others send to: a return (P-19)
    Return,
    Normal,
    Master,
}

#[derive(Clone, Debug)]
pub struct PluginView {
    pub id: NodeId,
    pub plugin: Arc<str>,
    pub bypass: bool,
    pub point: Arc<str>,
    #[expect(dead_code, reason = "read by the screens B, C and D port onto lm-gui next")]
    pub state: Value,
}

#[derive(Clone, Debug)]
pub struct SendView {
    pub id: NodeId,
    pub to: NodeId,
    pub amount: f64,
    pub pre: bool,
    /// the input of the plugin it feeds (`"sidechain"`), when it goes to an insert rather than a channel
    pub port: Option<Arc<str>>,
}

#[derive(Clone, Debug)]
pub struct ChannelView {
    pub id: NodeId,
    pub name: Arc<str>,
    pub kind: ChannelKind,
    pub volume: f64,
    pub mute: bool,
    pub solo: bool,
    pub output: Option<NodeId>,
    pub inserts: Vec<PluginView>,
    pub sends: Vec<SendView>,
    /// the rack: the node itself, read by key paths (`tube.drive`)
    pub rack: Value,
    /// The legacy strip's input controls (optional in the schema: absent is off)
    pub mono: bool,
    pub phase_invert: bool,
    pub swap: bool,
    /// "#rrggbb", or none
    pub color: Option<u32>,
    /// how many audio channels the strip carries (2 is a stereo pair)
    pub channels: i64,
    /// The master's PRE faders and their LINK (1, 1, on for the other channels)
    pub pre: (f64, f64, bool),
}

fn color_of(v: &Value) -> Option<u32> {
    let t = v.get("color").and_then(Value::as_text)?;
    u32::from_str_radix(t.trim_start_matches('#'), 16).ok().filter(|_| t.len() == 7)
}

fn plugin_view(p: &Value) -> Option<PluginView> {
    Some(PluginView {
        id: id_of(p)?,
        plugin: text(p, "plugin"),
        bypass: flag(p, "bypass"),
        point: p.get("point").and_then(Value::as_shared_text).cloned().unwrap_or_else(|| Arc::from("post-eq")),
        state: p.get("state").cloned().unwrap_or_default(),
    })
}

/// The desk: the channels in order, then the master. Groups and returns are read from the routing: a channel others
/// output to is a group, one others send to is a return
pub fn channels(root: &Value) -> Vec<ChannelView> {
    let list_v: Vec<Value> = at(root, &["mixer", "channels"]).map(|l| l.items().cloned().collect()).unwrap_or_default();
    let outputs_to: Vec<NodeId> = list_v.iter().filter_map(|c| ref_of(c, "output")).collect();
    let sends_to: Vec<NodeId> =
        list_v.iter().flat_map(|c| list(c, "sends").filter_map(|s| ref_of(s, "to")).collect::<Vec<_>>()).collect();
    let mut out: Vec<ChannelView> = list_v
        .iter()
        .filter_map(|c| {
            let id = id_of(c)?;
            let kind = if sends_to.contains(&id) || flag(c, "return") {
                ChannelKind::Return
            } else if outputs_to.contains(&id) {
                ChannelKind::Group
            } else {
                ChannelKind::Normal
            };
            Some(ChannelView {
                id,
                name: text(c, "name"),
                kind,
                volume: real(c, "volume", 1.0),
                mute: flag(c, "mute"),
                solo: flag(c, "solo"),
                output: ref_of(c, "output"),
                inserts: list(c, "inserts").filter_map(plugin_view).collect(),
                sends: list(c, "sends")
                    .filter_map(|s| {
                        Some(SendView {
                            id: id_of(s)?,
                            to: ref_of(s, "to")?,
                            amount: real(s, "amount", 1.0),
                            pre: flag(s, "pre"),
                            port: s.get("port").and_then(Value::as_shared_text).cloned(),
                        })
                    })
                    .collect(),
                rack: c.get("rack").cloned().unwrap_or_default(),
                mono: flag(c, "mono"),
                phase_invert: flag(c, "phaseInvert"),
                swap: flag(c, "swap"),
                color: color_of(c),
                channels: int(c, "channels", 2),
                pre: (1.0, 1.0, true),
            })
        })
        .collect();
    if let Some(m) = at(root, &["mixer", "master"]) {
        out.push(ChannelView {
            id: id_of(m).unwrap_or(NodeId { kind: b'm', number: 0 }),
            name: text(m, "name"),
            kind: ChannelKind::Master,
            volume: real(m, "volume", 1.0),
            mute: flag(m, "mute"),
            solo: false,
            output: None,
            inserts: list(m, "inserts").filter_map(plugin_view).collect(),
            sends: Vec::new(),
            rack: m.get("rack").cloned().unwrap_or_default(),
            mono: false,
            phase_invert: false,
            swap: false,
            color: color_of(m),
            channels: int(m, "channels", 2),
            pre: (
                real(m, "preLeft", 1.0),
                real(m, "preRight", 1.0),
                m.get("preLink").and_then(Value::as_bool).unwrap_or(true),
            ),
        });
    }
    out
}

/// The song's switches and ranges, by key path from the root
#[derive(Clone, Debug)]
pub struct Song {
    pub loop_on: bool,
    pub loop_range: (i64, i64),
    pub punch_on: bool,
    pub punch_range: (i64, i64),
    pub pre_roll: i64,
    pub metronome: bool,
    pub count_in: i64,
    pub transpose: i64,
    pub analog: bool,
}

pub fn song(root: &Value) -> Song {
    let tr = |p: &[&str]| at(root, &[&["transport"], p].concat()).cloned().unwrap_or_default();
    Song {
        loop_on: tr(&["loop", "on"]).as_bool().unwrap_or(false),
        loop_range: (tr(&["loop", "from"]).as_int().unwrap_or(0), tr(&["loop", "to"]).as_int().unwrap_or(0)),
        punch_on: tr(&["punch", "on"]).as_bool().unwrap_or(false),
        punch_range: (tr(&["punch", "from"]).as_int().unwrap_or(0), tr(&["punch", "to"]).as_int().unwrap_or(0)),
        pre_roll: tr(&["preRoll", "bars"]).as_int().unwrap_or(1),
        metronome: tr(&["metronome", "on"]).as_bool().unwrap_or(false),
        count_in: tr(&["metronome", "countIn"]).as_int().unwrap_or(0),
        transpose: tr(&["transpose"]).as_int().unwrap_or(0),
        analog: at(root, &["mixer", "mode"]).and_then(Value::as_text) != Some("digital"),
    }
}

/// A marker of the ruler (F-REG-011)
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerView {
    pub id: NodeId,
    pub name: Arc<str>,
    pub tick: i64,
    pub color: Option<u32>,
}

/// A region of the ruler (F-REG-012)
#[derive(Clone, Debug, PartialEq)]
pub struct RegionView {
    pub id: NodeId,
    pub name: Arc<str>,
    pub from: i64,
    pub to: i64,
    pub color: Option<u32>,
}

/// The markers, in time order
pub fn markers(root: &Value) -> Vec<MarkerView> {
    let mut v: Vec<MarkerView> = list(root, "markers")
        .filter_map(|m| {
            Some(MarkerView {
                id: id_of(m)?,
                name: text(m, "name"),
                tick: int(m, "tick", 0),
                color: m.get("color").and_then(Value::as_int).filter(|c| *c != 0).map(|c| c as u32),
            })
        })
        .collect();
    v.sort_by_key(|m| m.tick);
    v
}

/// The regions, in time order of their starts
pub fn regions(root: &Value) -> Vec<RegionView> {
    let mut v: Vec<RegionView> = list(root, "regions")
        .filter_map(|r| {
            Some(RegionView {
                id: id_of(r)?,
                name: text(r, "name"),
                from: int(r, "from", 0),
                to: int(r, "to", 0),
                color: r.get("color").and_then(Value::as_int).filter(|c| *c != 0).map(|c| c as u32),
            })
        })
        .collect();
    v.sort_by_key(|r| r.from);
    v
}
