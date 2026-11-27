//! The actions of I-21 (MIDI and automation), in the session's action table (I-19): control surfaces (banks,
//! selecting a strip's track), the global automation button, re-arm, and the heavy automation edits (thin, scale,
//! convert between an item's control lanes and a lane, move items with their automation)

use std::sync::Arc;

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, PathStep, ROOT, Value};

use crate::actions::ActionSpec;
use crate::session::Session;

fn params() -> Error {
    Error::new("action.params")
}

fn id(p: &Value, k: &str) -> Result<NodeId> {
    p.get(k).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)
}

fn int_or(p: &Value, k: &str, d: i64) -> i64 {
    p.get(k).and_then(Value::as_int).unwrap_or(d)
}

fn real_or(p: &Value, k: &str, d: f64) -> f64 {
    p.get(k).and_then(|v| v.as_real().or_else(|| v.as_int().map(|i| i as f64))).unwrap_or(d)
}

fn always(_: &Session) -> bool {
    true
}

fn writable(s: &Session) -> bool {
    !s.project().read_only
}

pub static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "item.audition",
        label: "action.item.audition",
        // {item, from?, to?, loop?}: `from` and `to` in ticks inside the item (the item's loop part by default)
        run: |s, p| {
            let item = id(p, "item")?;
            let from = p.get("from").and_then(Value::as_int);
            let to = p.get("to").and_then(Value::as_int);
            let repeat = p.get("loop").and_then(Value::as_bool).unwrap_or(false);
            s.audition_start(item, from, to, repeat)
        },
        enabled: always,
    },
    ActionSpec {
        id: "item.audition_stop",
        label: "action.item.audition_stop",
        run: |s, _| {
            s.audition_stop();
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "midi.bank",
        label: "action.midi.bank",
        // {role: "track"|"channel", delta}
        run: |s, p| {
            let role = p.get("role").and_then(Value::as_text).unwrap_or("track").to_owned();
            s.mcu_bank(&role, int_or(p, "delta", 8));
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "midi.select_track",
        label: "action.midi.select_track",
        // {offset}: the track at the bank's start plus `offset` becomes the selection (in the interface document)
        run: |s, p| {
            let i = s.midi.bank.0 as usize + usize::try_from(int_or(p, "offset", 0)).unwrap_or(0);
            let track = s
                .document
                .root()
                .get("tracks")
                .and_then(|l| l.at(i))
                .and_then(|t| t.get("id").cloned())
                .ok_or_else(|| Error::new("midi.no_track"))?;
            let sel = s.ui_document.root().get("selection").cloned().unwrap_or_else(Value::empty_object);
            let sel = sel.with("track", track);
            s.ui_document.transact(Label::new("midi.select"), |e| e.set(ROOT, &[PathStep::key("selection")], sel))?;
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "midi.assign_alias",
        label: "action.midi.assign_alias",
        // {alias, client, port}: a project opened on another machine asks once for the port of each alias (P-36)
        run: |s, p| {
            let alias = p.get("alias").and_then(Value::as_text).ok_or_else(params)?;
            let client = p.get("client").and_then(Value::as_text).ok_or_else(params)?;
            let port = p.get("port").and_then(Value::as_text).ok_or_else(params)?;
            let ports = s.preferences.get(&["midi", "ports"]).cloned().unwrap_or_else(Value::empty_list);
            let mut found = false;
            let mut list = Vec::new();
            for e in ports.items() {
                let same = e.get("client").and_then(Value::as_text) == Some(client)
                    && e.get("port").and_then(Value::as_text) == Some(port);
                if same {
                    found = true;
                    list.push(e.with("alias", Value::text(alias)));
                } else if e.get("alias").and_then(Value::as_text) == Some(alias) {
                    list.push(e.with("alias", Value::text("")));
                } else {
                    list.push(e.clone());
                }
            }
            if !found {
                list.push(Value::object([
                    ("client", Value::text(client)),
                    ("port", Value::text(port)),
                    ("backend", Value::text("seq")),
                    ("alias", Value::text(alias)),
                    ("role", Value::text("both")),
                    ("input", Value::bool(true)),
                    ("output", Value::bool(true)),
                    ("inputLatencyMs", Value::real(0.0)?),
                    ("outputLatencyMs", Value::real(0.0)?),
                ]));
            }
            s.set_preference(&[PathStep::key("midi"), PathStep::key("ports")], Value::list(list))?;
            s.midi.mark_dirty();
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "notes.step_record",
        label: "action.notes.step_record",
        // {item, at?, step?, velocity?}: step recording at the cursor (F-PAS-001)
        run: |s, p| s.step_start(p),
        enabled: writable,
    },
    ActionSpec {
        id: "notes.step_keys",
        label: "action.notes.step_keys",
        // {keys: [int], velocity?}: one chord at the cursor (the computer keyboard)
        run: |s, p| {
            let v = p.get("velocity").and_then(Value::as_real).unwrap_or(0.8).clamp(0.0, 1.0);
            let keys: Vec<(u8, f64)> = p
                .get("keys")
                .map(|l| {
                    l.items()
                        .filter_map(Value::as_int)
                        .filter_map(|k| u8::try_from(k).ok())
                        .filter(|k| *k < 128)
                        .map(|k| (k, v))
                        .collect()
                })
                .unwrap_or_default();
            s.step_chord(&keys)
        },
        enabled: writable,
    },
    ActionSpec { id: "notes.step_rest", label: "action.notes.step_rest", run: |s, _| s.step_rest(), enabled: always },
    ActionSpec { id: "notes.step_back", label: "action.notes.step_back", run: |s, _| s.step_back(), enabled: writable },
    ActionSpec {
        id: "notes.step_length",
        label: "action.notes.step_length",
        // {delta: ticks}
        run: |s, p| s.step_length(int_or(p, "delta", 0)),
        enabled: writable,
    },
    ActionSpec {
        id: "notes.step_stop",
        label: "action.notes.step_stop",
        run: |s, _| {
            s.step_stop();
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "notes.record_into",
        label: "action.notes.record_into",
        // {item} or {}: the next recording writes into the item open in the Piano Roll (F-NOT-018)
        run: |s, p| s.set_record_into(p),
        enabled: always,
    },
    ActionSpec {
        id: "transport.rewind",
        label: "action.transport.rewind",
        run: |s, _| {
            s.locate(0);
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "automation.global",
        label: "action.automation.global",
        // {mode}: the global automation button (I-21.5); "lanes" gives each lane its own mode back
        run: |s, p| {
            let mode = p.get("mode").and_then(Value::as_text).ok_or_else(params)?.to_owned();
            s.document
                .transact(Label::new("automation.global"), |e| {
                    e.set(ROOT, &[PathStep::key("transport"), PathStep::key("automationMode")], Value::text(mode))
                })
                .map(|_| ())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "automation.rearm",
        label: "action.automation.rearm",
        // {} every lane, {lane} one
        run: |s, p| {
            let lane = p.get("lane").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
            s.automation_rearm(lane);
            Ok(())
        },
        enabled: always,
    },
    ActionSpec {
        id: "automation.thin",
        label: "action.automation.thin",
        // {lane, from?, to?, tolerance? (a multiplier of the parameter's tolerance)}
        run: |s, p| {
            let lane = id(p, "lane")?;
            s.automation_thin(lane, int_or(p, "from", 0), int_or(p, "to", i64::MAX), real_or(p, "tolerance", 1.0))
                .map(|_| ())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "automation.scale",
        label: "action.automation.scale",
        // {lane, scale, offset, from?, to?}, in the parameter's normalized space
        run: |s, p| {
            let lane = id(p, "lane")?;
            s.automation_scale(
                lane,
                int_or(p, "from", 0),
                int_or(p, "to", i64::MAX),
                real_or(p, "scale", 1.0),
                real_or(p, "offset", 0.0),
            )
        },
        enabled: writable,
    },
    ActionSpec {
        id: "automation.to_item",
        label: "action.automation.to_item",
        // {lane, item, cc? (default 1)}
        run: |s, p| {
            let cc = u8::try_from(int_or(p, "cc", 1).clamp(0, 127)).unwrap_or(1);
            s.automation_to_item(id(p, "lane")?, id(p, "item")?, cc)
        },
        enabled: writable,
    },
    ActionSpec {
        id: "automation.from_item",
        label: "action.automation.from_item",
        // {item, cc, owner (the parameter's node), param}
        run: |s, p| {
            let cc = u8::try_from(int_or(p, "cc", 1).clamp(0, 127)).unwrap_or(1);
            let key: Arc<str> = p.get("param").and_then(Value::as_shared_text).cloned().ok_or_else(params)?;
            s.automation_from_item(id(p, "item")?, cc, id(p, "owner")?, &key)
        },
        enabled: writable,
    },
    ActionSpec {
        id: "items.move",
        label: "action.items.move",
        // {items: [ids], delta (ticks), automation? (default true)}
        run: |s, p| {
            let items: Vec<NodeId> = p
                .get("items")
                .map(|l| l.items().filter_map(Value::as_text).filter_map(|t| NodeId::parse(t).ok()).collect())
                .unwrap_or_default();
            let automation = p.get("automation").and_then(Value::as_bool).unwrap_or(true);
            s.items_move(&items, int_or(p, "delta", 0), automation)
        },
        enabled: writable,
    },
];
