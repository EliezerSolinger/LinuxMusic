//! The OSC server (`decisoes-autonomas.md`, 26/11, item 5): Open Sound Control as the second external control, beside
//! MIDI, with the same targets as the MIDI map (a parameter, a field, an action) and feedback.
//!
//! - It is off by default. The preferences `osc: { on, port, lan }` turn it on and say where: a UDP socket on
//!   `127.0.0.1` (only this machine) unless `lan` is set, which binds `0.0.0.0` (the settings page says so).
//! - It never blocks the main thread: the socket is non-blocking, the pump reads at most [`MAX_PER_PUMP`] datagrams
//!   and sends what the feedback has, and a send that would wait is dropped (the next change sends it again).
//! - The addresses are in `address.rs`; an alias the user wrote is a mapping node whose source is an OSC address
//!   (`lm_midimap::OscMapping`), in the project's `midiMap` or the preferences' `midiMap`.
//! - Time tags are not scheduled: a message in a bundle acts when it arrives.
//! - A value from a client is a 0 to 1 float for a parameter, as a MIDI control; a field takes what its kind takes
//!   (T/F for a switch, a name or an index for a choice, an integer for a whole number, 0 to 1 for a real). An error
//!   goes back to the sender as `/error <address> <key>`.

mod address;
mod outbox;

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;

use lm_base::{Error, Label, Result};
use lm_doc::Origin;
use lm_midimap::{MapTarget, NodeRef, OscMapping, SelRole};
use lm_osc::{Arg, Message, Packet};
use lm_state::{FieldKind, NodeId, PathStep, Value};

use self::address::{Route, field_address, param_address, route};
use self::outbox::Outbox;
use super::midi::{field_norm, read_selection};
use crate::session::Session;

/// The port when the preferences do not say
pub(crate) const DEFAULT_PORT: u16 = 9000;
/// The lowest port the server takes (the ones below need privileges)
const MIN_PORT: i64 = 1024;
/// The datagrams read in one pump
const MAX_PER_PUMP: usize = 256;
/// The changes of one document diff that become feedback
const MAX_CHANGES: usize = 2048;

/// The server's state
#[derive(Debug)]
pub(crate) struct OscCore {
    socket: Option<UdpSocket>,
    /// What the socket was made for (`lan`, port); `None` = off
    bound: Option<(bool, u16)>,
    outbox: Outbox,
    buf: Vec<u8>,
}

impl Default for OscCore {
    fn default() -> Self {
        Self { socket: None, bound: None, outbox: Outbox::default(), buf: vec![0; 65_536] }
    }
}

fn arg_f32(args: &[Arg]) -> Option<f32> {
    args.first().and_then(Arg::as_f32)
}

/// Where the feedback to a client goes: its port, or the one it gave
fn reply_to(from: SocketAddr, args: &[Arg]) -> SocketAddr {
    match args.first() {
        Some(Arg::Int(p)) if (1..=65_535).contains(p) => SocketAddr::new(from.ip(), *p as u16),
        _ => from,
    }
}

/// A field's value as an OSC argument, for the feedback (`None`: not a kind OSC carries)
fn field_arg(kind: &FieldKind, v: &Value) -> Option<(Arg, Option<f32>)> {
    Some(match kind {
        FieldKind::Bool => (if v.as_bool()? { Arg::True } else { Arg::False }, Some(f32::from(u8::from(v.as_bool()?)))),
        FieldKind::Choice(_) => (Arg::Str(v.as_text()?.to_owned()), field_norm(kind, v)),
        FieldKind::Int { .. } => (Arg::Int(i32::try_from(v.as_int()?).ok()?), field_norm(kind, v)),
        FieldKind::Real { .. } => {
            let n = field_norm(kind, v)?;
            (Arg::Float(n), Some(n))
        }
        FieldKind::Text => (Arg::Str(v.as_text()?.to_owned()), None),
        _ => return None,
    })
}

impl Session {
    /// One turn of the server: it follows the preferences, reads what arrived and sends the feedback
    pub(crate) fn osc_pump(&mut self) -> bool {
        let want = if self.preferences.bool(&["osc", "on"]).unwrap_or(false) {
            let port = self
                .preferences
                .int(&["osc", "port"])
                .filter(|p| (MIN_PORT..=65_535).contains(p))
                .map_or(DEFAULT_PORT, |p| p as u16);
            Some((self.preferences.bool(&["osc", "lan"]).unwrap_or(false), port))
        } else {
            None
        };
        if self.osc.bound != want {
            self.osc_rebind(want);
        }
        if self.osc.socket.is_none() {
            return false;
        }
        let now = self.clock.now().0;
        let mut changed = false;
        let mut buf = std::mem::take(&mut self.osc.buf);
        for _ in 0..MAX_PER_PUMP {
            let Some(socket) = self.osc.socket.as_ref() else { break };
            match socket.recv_from(&mut buf) {
                Ok((n, from)) => {
                    self.osc.outbox.heard(from, now);
                    match lm_osc::decode(&buf[..n]) {
                        Ok(p) => changed |= self.osc_packet(from, p, now),
                        Err(e) => self.osc_reply_error(from, "", e.key),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break, // nothing more (WouldBlock) or a socket that failed: next pump
            }
        }
        self.osc.buf = buf;
        let out = self.osc.outbox.flush(now);
        if let Some(socket) = self.osc.socket.as_ref() {
            for (to, bytes) in out {
                // A send that would wait is dropped: the next change sends the value again
                let _ = socket.send_to(&bytes, to);
            }
        }
        changed
    }

    fn osc_rebind(&mut self, want: Option<(bool, u16)>) {
        self.osc.socket = None;
        self.osc.outbox = Outbox::default();
        self.osc.bound = want;
        let Some((lan, port)) = want else { return };
        let ip = if lan { IpAddr::V4(Ipv4Addr::UNSPECIFIED) } else { IpAddr::V4(Ipv4Addr::LOCALHOST) };
        match UdpSocket::bind(SocketAddr::new(ip, port)).and_then(|s| s.set_nonblocking(true).map(|()| s)) {
            Ok(s) => self.osc.socket = Some(s),
            Err(e) => self.notices.warn(
                "osc.bind_failed",
                vec![("port", Value::int(i64::from(port))), ("error", Value::text(e.to_string()))],
            ),
        }
    }

    fn osc_reply_error(&self, to: SocketAddr, addr: &str, key: &str) {
        if let Some(socket) = self.osc.socket.as_ref() {
            let m = Message::new("/error", vec![Arg::Str(addr.to_owned()), Arg::Str(key.to_owned())]);
            let _ = socket.send_to(&m.encode(), to);
        }
    }

    fn osc_packet(&mut self, from: SocketAddr, p: Packet, now: u64) -> bool {
        let mut changed = false;
        for (_, m) in p.flatten() {
            let addr = m.addr.clone();
            match self.osc_message(from, m, now) {
                Ok(()) => changed = true,
                Err(e) => self.osc_reply_error(from, &addr, e.key()),
            }
        }
        changed
    }

    fn osc_message(&mut self, from: SocketAddr, m: Message, now: u64) -> Result<()> {
        let echo_to = reply_to(from, &[]);
        let result = match route(&m.addr) {
            Route::Param { node, key } => {
                let v = arg_f32(&m.args).ok_or_else(|| Error::new("osc.args"))?;
                self.osc_set_param(node, &key, v)
            }
            Route::Field { node, path } => self.osc_set_field(node, &path, &m.args),
            Route::Action { id } => {
                let params = match m.args.first() {
                    Some(Arg::Str(json)) => lm_state::from_json(json).map_err(|_| Error::new("osc.params"))?,
                    _ => Value::NULL,
                };
                self.run_action(&id, &params)
            }
            Route::Transport { name } => {
                // A button sends 1 on the press and 0 on the release: only the press acts
                if arg_f32(&m.args) == Some(0.0) {
                    return Ok(());
                }
                self.run_action(&format!("transport.{name}"), &Value::NULL)
            }
            Route::Subscribe => {
                if self.osc.outbox.subscribe(reply_to(from, &m.args), now) {
                    Ok(())
                } else {
                    Err(Error::new("osc.subscribers_full"))
                }
            }
            Route::Unsubscribe => {
                self.osc.outbox.unsubscribe(reply_to(from, &m.args));
                Ok(())
            }
            Route::Other => match self.osc_aliases().into_iter().find(|a| *a.address == *m.addr) {
                Some(alias) => self.osc_alias(&alias, &m.args),
                None => Err(Error::new("osc.unknown_address")),
            },
        };
        if result.is_ok() {
            self.osc.outbox.set_by(echo_to, &m.addr, now);
        }
        result
    }

    /// A parameter takes a 0 to 1 value of its range, as a MIDI control does
    fn osc_set_param(&mut self, node: NodeId, key: &str, v: f32) -> Result<()> {
        let path = self
            .midi_doc_path(node, key)
            .ok_or_else(|| Error::new("osc.param.unknown").arg("node", node.to_string()).arg("param", key))?;
        let units = self.param_range(node, key).to_units(v.clamp(0.0, 1.0));
        let value = self
            .midi_doc_value(node, key, &path, units)
            .ok_or_else(|| Error::new("osc.param.unknown").arg("node", node.to_string()).arg("param", key))?;
        let label = Label::new("osc.set").arg("target", format!("{node} {key}"));
        self.document.transact_as(label, Origin::Gesture, |e| e.set(node, &path, value)).map(|_| ())
    }

    /// A field takes what its kind takes (see the module's documentation)
    fn osc_set_field(&mut self, node: NodeId, path: &[PathStep], args: &[Arg]) -> Result<()> {
        let kind =
            self.field_kind(node, path).ok_or_else(|| Error::new("osc.field.unknown").arg("node", node.to_string()))?;
        let arg = args.first().ok_or_else(|| Error::new("osc.args"))?;
        let cur = self.document.find(node).and_then(|n| n.at_path(path).cloned()).unwrap_or(Value::NULL);
        let bad = || Error::new("osc.args");
        let new = match &kind {
            FieldKind::Bool => Value::bool(arg.as_f32().ok_or_else(bad)? >= 0.5),
            FieldKind::Choice(options) => match arg {
                Arg::Str(s) => Value::text(*options.iter().find(|o| **o == s.as_str()).ok_or_else(bad)?),
                Arg::Int(i) => Value::text(*usize::try_from(*i).ok().and_then(|i| options.get(i)).ok_or_else(bad)?),
                a => {
                    let n = options.len().max(1);
                    let i = (f64::from(a.as_f32().ok_or_else(bad)?.clamp(0.0, 1.0)) * (n - 1) as f64).round() as usize;
                    Value::text(options[i.min(n - 1)])
                }
            },
            FieldKind::Int { min, max } => match arg {
                Arg::Int(i) => Value::int(i64::from(*i).clamp(*min, *max)),
                a => {
                    let f = f64::from(a.as_f32().ok_or_else(bad)?.clamp(0.0, 1.0));
                    Value::int(min + (f * (max - min) as f64).round() as i64)
                }
            },
            FieldKind::Real { min, max } => {
                let f = f64::from(arg.as_f32().ok_or_else(bad)?.clamp(0.0, 1.0));
                Value::real(min + f * (max - min))?
            }
            FieldKind::Text => Value::text(arg.as_str().ok_or_else(bad)?),
            _ => return Err(Error::new("osc.field.not_settable")),
        };
        if new == cur {
            return Ok(());
        }
        let label = Label::new("osc.set").arg("target", format!("{node} {}", lm_state::path_text(path)));
        self.document.transact(label, |e| e.set(node, path, new)).map(|_| ())
    }

    /// The aliases of the user: mapping nodes with an OSC address as their source, in the project and the preferences
    fn osc_aliases(&self) -> Vec<OscMapping> {
        let mut out = Vec::new();
        for list in [self.document.root().get("midiMap"), self.preferences.get(&["midiMap"])].into_iter().flatten() {
            for v in list.items() {
                if lm_midimap::doc::osc_address(v).is_some()
                    && let Ok(a) = OscMapping::from_value(v)
                {
                    out.push(a);
                }
            }
        }
        out
    }

    /// The node an alias drives: a fixed one, or the selected track, channel or plugin (no bank, no offset)
    fn osc_node(&self, node: &NodeRef) -> Option<NodeId> {
        match node {
            NodeRef::Id(id) => Some(*id),
            NodeRef::Selected { role, offset: 0 } => {
                let (track, channel, plugin) = read_selection(self.ui_document.root());
                match role {
                    SelRole::Track => track,
                    SelRole::Channel => channel,
                    SelRole::Plugin => plugin,
                    _ => None,
                }
            }
            NodeRef::Selected { .. } => None,
        }
    }

    fn osc_alias(&mut self, a: &OscMapping, args: &[Arg]) -> Result<()> {
        let unresolved = || Error::new("osc.alias.unresolved");
        match &a.target {
            MapTarget::Param { node, key } => {
                let n = self.osc_node(node).ok_or_else(unresolved)?;
                let v = arg_f32(args).ok_or_else(|| Error::new("osc.args"))?;
                self.osc_set_param(n, key, a.transform.forward(v.clamp(0.0, 1.0)))
            }
            MapTarget::Field { node, path } => {
                let n = self.osc_node(node).ok_or_else(unresolved)?;
                match args.first() {
                    Some(Arg::Float(f)) => {
                        self.osc_set_field(n, path, &[Arg::Float(a.transform.forward(f.clamp(0.0, 1.0)))])
                    }
                    _ => self.osc_set_field(n, path, args),
                }
            }
            MapTarget::Action { id, params, value_into } => {
                // A button sends 0 on the release: only the press acts
                if arg_f32(args) == Some(0.0) {
                    return Ok(());
                }
                let mut p = params.clone();
                if let Some(path) = value_into {
                    let raw = match args.first() {
                        Some(Arg::Int(i)) => i64::from(*i),
                        Some(a) => (f64::from(a.as_f32().unwrap_or(0.0)) * 127.0).round() as i64,
                        None => 0,
                    };
                    p = p.with_path(path, Value::int(raw)).unwrap_or(p);
                }
                self.run_action(id, &p)
            }
        }
    }

    /// The document changed: the values that changed wait for the subscribers (parameters by their address and
    /// by the aliases that drive them, fields the same)
    pub(crate) fn osc_diff(&mut self, diff: &lm_doc::Diff) {
        if !self.osc.outbox.has_subscribers() {
            return;
        }
        let aliases = self.osc_aliases();
        let alias_of = |s: &Session, node: NodeId, param: Option<&str>, path: Option<&[PathStep]>| -> Vec<Arc<str>> {
            aliases
                .iter()
                .filter(|a| match (&a.target, param, path) {
                    (MapTarget::Param { node: n, key }, Some(p), _) => s.osc_node(n) == Some(node) && key == p,
                    (MapTarget::Field { node: n, path: x }, _, Some(p)) => s.osc_node(n) == Some(node) && x == p,
                    _ => false,
                })
                .map(|a| a.address.clone())
                .collect()
        };
        let mut seen: HashSet<(NodeId, Vec<PathStep>)> = HashSet::new();
        let mut out: Vec<Message> = Vec::new();
        for (node, key, value) in diff.params.iter().take(MAX_CHANGES) {
            let norm = self.param_range(*node, key).normalize(*value as f32);
            out.push(Message::new(param_address(*node, key), vec![Arg::Float(norm)]));
            for addr in alias_of(self, *node, Some(key), None) {
                let t = aliases.iter().find(|a| *a.address == *addr).map_or(1.0, |a| a.transform.inverse(norm));
                out.push(Message::new(addr.to_string(), vec![Arg::Float(t)]));
            }
            if let Some(ty) = self.document.type_of(*node) {
                seen.insert((*node, lm_doc::param_path(ty, key)));
            }
        }
        let mut count = 0;
        'changes: for c in &diff.changed {
            for path in &c.paths {
                count += 1;
                if count > MAX_CHANGES {
                    break 'changes;
                }
                if seen.contains(&(c.node, path.clone())) {
                    continue;
                }
                let Some(v) = self.document.find(c.node).and_then(|n| n.at_path(path).cloned()) else { continue };
                let Some(kind) = self.field_kind(c.node, path) else { continue };
                let Some((arg, norm)) = field_arg(&kind, &v) else { continue };
                if let Some(addr) = field_address(c.node, path) {
                    out.push(Message::new(addr, vec![arg.clone()]));
                }
                for addr in alias_of(self, c.node, None, Some(path)) {
                    let sent = match (&arg, norm) {
                        (Arg::Float(_), Some(n)) => {
                            let inv =
                                aliases.iter().find(|a| *a.address == *addr).map_or(n, |a| a.transform.inverse(n));
                            Arg::Float(inv)
                        }
                        _ => arg.clone(),
                    };
                    out.push(Message::new(addr.to_string(), vec![sent]));
                }
            }
        }
        for m in out {
            self.osc.outbox.push(m);
        }
    }
}
