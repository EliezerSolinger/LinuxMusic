//! The feedback of the OSC server: the value changes wait here by address (the last one wins), and go out to the
//! subscribers at most every [`INTERVAL_NS`], at most [`BURST`] addresses at a time, as bundles that fit a datagram.
//! The same policy as the MIDI feedback's limiter: a fast gesture is sent at 50 Hz, not at the rate it came.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use lm_osc::{Bundle, Message, Packet, TimeTag};

/// The time between two sends
pub(crate) const INTERVAL_NS: u64 = 20_000_000;
/// The addresses sent in one go
pub(crate) const BURST: usize = 128;
/// The addresses that wait (a change of one that waits replaces it; a new one is dropped past this)
pub(crate) const PENDING_MAX: usize = 4096;
/// The longest datagram of feedback
const DATAGRAM_MAX: usize = 1200;
/// The subscribers
pub(crate) const SUBSCRIBERS_MAX: usize = 16;
/// A subscriber that did not say anything for this long is dropped
pub(crate) const SUBSCRIBER_IDLE_NS: u64 = 300_000_000_000;
/// A value that came from a client is not sent back to it for this long
pub(crate) const ECHO_NS: u64 = 100_000_000;

/// A client that asked for the changes
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Subscriber {
    /// Where the feedback goes
    pub to: SocketAddr,
    /// The last time it sent anything
    pub heard: u64,
}

#[derive(Debug, Default)]
pub(crate) struct Outbox {
    pending: BTreeMap<String, Message>,
    last_flush: u64,
    subscribers: Vec<Subscriber>,
    /// (client, address, when): the client set that address, so no echo back for a moment
    echo: Vec<(SocketAddr, String, u64)>,
}

impl Outbox {
    pub(crate) fn has_subscribers(&self) -> bool {
        !self.subscribers.is_empty()
    }

    /// A client subscribes (or renews); `false` when there is no room
    pub(crate) fn subscribe(&mut self, to: SocketAddr, now: u64) -> bool {
        if let Some(s) = self.subscribers.iter_mut().find(|s| s.to == to) {
            s.heard = now;
            return true;
        }
        if self.subscribers.len() >= SUBSCRIBERS_MAX {
            return false;
        }
        self.subscribers.push(Subscriber { to, heard: now });
        true
    }

    pub(crate) fn unsubscribe(&mut self, to: SocketAddr) {
        self.subscribers.retain(|s| s.to != to);
    }

    /// A client sent something: its subscription is alive
    pub(crate) fn heard(&mut self, from: SocketAddr, now: u64) {
        for s in self.subscribers.iter_mut().filter(|s| s.to.ip() == from.ip()) {
            s.heard = now;
        }
    }

    /// The client set this address: its feedback for it is held back for a moment. `to`: where it listens
    pub(crate) fn set_by(&mut self, to: SocketAddr, addr: &str, now: u64) {
        self.echo.retain(|(_, _, t)| now.saturating_sub(*t) < ECHO_NS);
        if self.echo.len() < 256 {
            self.echo.push((to, addr.to_owned(), now));
        }
    }

    /// A value changed
    pub(crate) fn push(&mut self, m: Message) {
        if self.subscribers.is_empty() {
            return;
        }
        if self.pending.len() >= PENDING_MAX && !self.pending.contains_key(&m.addr) {
            return;
        }
        self.pending.insert(m.addr.clone(), m);
    }

    /// What goes out now: for each subscriber, the datagrams. Nothing before the interval has passed
    pub(crate) fn flush(&mut self, now: u64) -> Vec<(SocketAddr, Vec<u8>)> {
        self.subscribers.retain(|s| now.saturating_sub(s.heard) < SUBSCRIBER_IDLE_NS);
        if self.subscribers.is_empty() {
            self.pending.clear();
            return Vec::new();
        }
        if self.pending.is_empty() || now.saturating_sub(self.last_flush) < INTERVAL_NS {
            return Vec::new();
        }
        self.last_flush = now;
        let keys: Vec<String> = self.pending.keys().take(BURST).cloned().collect();
        let batch: Vec<Message> = keys.iter().filter_map(|k| self.pending.remove(k)).collect();
        let mut out = Vec::new();
        for s in &self.subscribers {
            let mine = batch.iter().filter(|m| {
                !self.echo.iter().any(|(c, a, t)| *c == s.to && *a == m.addr && now.saturating_sub(*t) < ECHO_NS)
            });
            let mut items: Vec<Packet> = Vec::new();
            let mut size = 20;
            for m in mine {
                let one = m.encode().len() + 4;
                if !items.is_empty() && size + one > DATAGRAM_MAX {
                    out.push((s.to, bundle(std::mem::take(&mut items))));
                    size = 20;
                }
                size += one;
                items.push(Packet::Message(m.clone()));
            }
            if !items.is_empty() {
                out.push((s.to, bundle(items)));
            }
        }
        out
    }
}

fn bundle(items: Vec<Packet>) -> Vec<u8> {
    match <[Packet; 1]>::try_from(items) {
        Ok([one]) => one.encode(),
        Err(items) => Packet::Bundle(Bundle { time: TimeTag::IMMEDIATELY, items }).encode(),
    }
}

#[cfg(test)]
mod tests {
    use lm_osc::Arg;

    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn m(a: &str, v: f32) -> Message {
        Message::new(a, vec![Arg::Float(v)])
    }

    fn messages(bytes: &[u8]) -> Vec<Message> {
        lm_osc::decode(bytes).unwrap().flatten().into_iter().map(|(_, m)| m).collect()
    }

    #[test]
    fn nothing_is_kept_without_subscribers() {
        let mut o = Outbox::default();
        o.push(m("/a", 1.0));
        assert!(o.flush(1_000_000_000).is_empty());
        assert!(o.pending.is_empty());
    }

    #[test]
    fn the_last_value_of_an_address_goes_out_at_the_rate() {
        let mut o = Outbox::default();
        assert!(o.subscribe(addr(9001), 0));
        o.push(m("/a", 0.1));
        o.push(m("/a", 0.2));
        o.push(m("/b", 0.5));
        let first = o.flush(INTERVAL_NS);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].0, addr(9001));
        assert_eq!(messages(&first[0].1), vec![m("/a", 0.2), m("/b", 0.5)]);
        o.push(m("/a", 0.3));
        assert!(o.flush(INTERVAL_NS + 1).is_empty(), "too soon");
        let second = o.flush(2 * INTERVAL_NS);
        assert_eq!(messages(&second[0].1), vec![m("/a", 0.3)]);
    }

    #[test]
    fn a_burst_is_limited_and_the_rest_waits() {
        let mut o = Outbox::default();
        o.subscribe(addr(9001), 0);
        for i in 0..(BURST + 10) {
            o.push(m(&format!("/p/{i:04}"), 0.5));
        }
        let sent: usize = o.flush(INTERVAL_NS).iter().map(|(_, b)| messages(b).len()).sum();
        assert_eq!(sent, BURST);
        let rest: usize = o.flush(2 * INTERVAL_NS).iter().map(|(_, b)| messages(b).len()).sum();
        assert_eq!(rest, 10);
    }

    #[test]
    fn the_client_that_set_a_value_does_not_get_it_back() {
        let mut o = Outbox::default();
        o.subscribe(addr(9001), 0);
        o.subscribe(addr(9002), 0);
        o.set_by(addr(9001), "/a", INTERVAL_NS - 1);
        o.push(m("/a", 0.4));
        let out = o.flush(INTERVAL_NS);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, addr(9002));
    }

    #[test]
    fn subscribers_are_limited_renewed_and_expire() {
        let mut o = Outbox::default();
        for p in 0..SUBSCRIBERS_MAX as u16 {
            assert!(o.subscribe(addr(9000 + p), 0));
        }
        assert!(!o.subscribe(addr(9999), 0));
        assert!(o.subscribe(addr(9000), 5), "renewing is always possible");
        o.unsubscribe(addr(9001));
        assert!(o.subscribe(addr(9999), 6), "room again");
        o.push(m("/a", 1.0));
        assert!(o.flush(SUBSCRIBER_IDLE_NS + 10).is_empty());
        assert!(!o.has_subscribers(), "idle ones are dropped");
    }

    #[test]
    fn a_long_burst_is_split_in_datagrams_that_fit() {
        let mut o = Outbox::default();
        o.subscribe(addr(9001), 0);
        for i in 0..BURST {
            o.push(m(&format!("/field/t:1/some/long/path/{i:04}"), 0.5));
        }
        let out = o.flush(INTERVAL_NS);
        assert!(out.len() > 1);
        assert!(out.iter().all(|(_, b)| b.len() <= 1300));
        assert_eq!(out.iter().map(|(_, b)| messages(b).len()).sum::<usize>(), BURST);
    }
}
