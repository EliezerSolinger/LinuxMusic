//! The addresses of the OSC server (`decisoes-autonomas.md`, 26/11, item 5): the same targets as the MIDI map.
//!
//! - `/param/<node>/<key> f`: a parameter, the float 0 to 1 of its range (as a MIDI control)
//! - `/field/<node>/<path…> v`: a field (a switch, a choice, a number), the path one segment per key (a number
//!   segment is a list index); `<node>` is a node id (`t:1`) or `root`
//! - `/action/<id> s`: an action of the core by its id; the optional string is its parameters in JSON
//! - `/transport/<name>`: the action `transport.<name>` (`/transport/play`, `/transport/stop`, `/transport/record`)
//! - `/subscribe [i]` and `/unsubscribe [i]`: the sender gets the value changes (at its port, or at the port given)
//!
//! Anything else may be an alias the user wrote (a mapping whose source is an OSC address).

use lm_state::{NodeId, PathStep, ROOT};

/// What an address asks for
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Route {
    Param {
        node: NodeId,
        key: String,
    },
    Field {
        node: NodeId,
        path: Vec<PathStep>,
    },
    Action {
        id: String,
    },
    Transport {
        name: String,
    },
    Subscribe,
    Unsubscribe,
    /// Not one of ours: an alias of the user's, or nothing
    Other,
}

fn node_of(text: &str) -> Option<NodeId> {
    if text == "root" { Some(ROOT) } else { NodeId::parse(text).ok() }
}

fn node_text(node: NodeId) -> String {
    if node == ROOT { "root".to_owned() } else { node.to_string() }
}

/// What an address is
pub(crate) fn route(addr: &str) -> Route {
    let Some(rest) = addr.strip_prefix('/') else { return Route::Other };
    let seg: Vec<&str> = rest.split('/').collect();
    if seg.iter().any(|s| s.is_empty()) {
        return Route::Other;
    }
    match (seg[0], seg.len()) {
        ("param", 3) => match node_of(seg[1]) {
            Some(node) => Route::Param { node, key: seg[2].to_owned() },
            None => Route::Other,
        },
        ("field", n) if n >= 3 => match node_of(seg[1]) {
            Some(node) => Route::Field {
                node,
                path: seg[2..]
                    .iter()
                    .map(|s| s.parse::<usize>().map_or_else(|_| PathStep::key(*s), PathStep::Index))
                    .collect(),
            },
            None => Route::Other,
        },
        ("action", 2) => Route::Action { id: seg[1].to_owned() },
        ("transport", 2) => Route::Transport { name: seg[1].to_owned() },
        ("subscribe", 1) => Route::Subscribe,
        ("unsubscribe", 1) => Route::Unsubscribe,
        _ => Route::Other,
    }
}

/// The address of a parameter (what the feedback sends)
pub(crate) fn param_address(node: NodeId, key: &str) -> String {
    format!("/param/{}/{key}", node_text(node))
}

/// The address of a field; `None` for a path that cannot be one (a segment with a slash, or no segment)
pub(crate) fn field_address(node: NodeId, path: &[PathStep]) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let mut out = format!("/field/{}", node_text(node));
    for s in path {
        out.push('/');
        match s {
            PathStep::Key(k) if !k.is_empty() && !k.contains('/') => out.push_str(k),
            PathStep::Index(i) => out.push_str(&i.to_string()),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(t: &str) -> NodeId {
        NodeId::parse(t).unwrap()
    }

    #[test]
    fn the_addresses_of_the_targets() {
        assert_eq!(route("/param/t:1/volume"), Route::Param { node: id("t:1"), key: "volume".into() });
        assert_eq!(route("/field/t:2/mute"), Route::Field { node: id("t:2"), path: vec![PathStep::key("mute")] });
        assert_eq!(
            route("/field/root/transport/loop/on"),
            Route::Field {
                node: ROOT,
                path: vec![PathStep::key("transport"), PathStep::key("loop"), PathStep::key("on")]
            }
        );
        assert_eq!(
            route("/field/t:2/outputs/0/gain"),
            Route::Field {
                node: id("t:2"),
                path: vec![PathStep::key("outputs"), PathStep::Index(0), PathStep::key("gain")]
            }
        );
        assert_eq!(route("/action/transport.play"), Route::Action { id: "transport.play".into() });
        assert_eq!(route("/transport/stop"), Route::Transport { name: "stop".into() });
        assert_eq!(route("/subscribe"), Route::Subscribe);
        assert_eq!(route("/unsubscribe"), Route::Unsubscribe);
    }

    #[test]
    fn what_is_not_ours_is_other() {
        for a in [
            "/param/t:1",
            "/param/zzz/volume",
            "/param//volume",
            "/field/t:1",
            "/mine/fader1",
            "x",
            "/",
            "/subscribe/x",
        ] {
            assert_eq!(route(a), Route::Other, "{a}");
        }
    }

    #[test]
    fn the_feedback_addresses_route_back() {
        let a = param_address(id("t:1"), "rack.tube.drive");
        assert_eq!(route(&a), Route::Param { node: id("t:1"), key: "rack.tube.drive".into() });
        let path = vec![PathStep::key("outputs"), PathStep::Index(1), PathStep::key("on")];
        let f = field_address(ROOT, &path).unwrap();
        assert_eq!(route(&f), Route::Field { node: ROOT, path });
        assert_eq!(field_address(ROOT, &[]), None);
        assert_eq!(field_address(ROOT, &[PathStep::key("a/b")]), None);
    }
}
