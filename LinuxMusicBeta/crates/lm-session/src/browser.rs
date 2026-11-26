//! The browser entries the core has given out (I-15), and the actions that name one. An entry is an id the interface
//! (local or remote) holds; the file behind it is the session's: `plugin.load_preset {node, entry}`,
//! `project.open_entry <entry>` and `browser.reveal <entry>` become the same actions with the file, in
//! `Session::run_action`, so a remote client, the headless runner and the `LocalPort` all get it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use lm_base::{Error, Result};
use lm_port::{BrowsePage, EntryId};
use lm_state::Value;

/// One entry of the browser the core has given out, to find its children and its file again
#[derive(Clone, Debug)]
pub(crate) struct Known {
    pub(crate) page: BrowsePage,
    pub(crate) path: Option<PathBuf>,
}

/// Every entry given out, by id
#[derive(Debug, Default)]
pub struct BrowserEntries {
    pub(crate) known: BTreeMap<EntryId, Known>,
}

impl BrowserEntries {
    pub(crate) fn remember(&mut self, id: EntryId, page: BrowsePage, path: Option<PathBuf>) {
        self.known.insert(id, Known { page, path });
    }

    /// The file of an entry, when it has one
    pub fn path(&self, id: EntryId) -> Option<PathBuf> {
        self.known.get(&id).and_then(|k| k.path.clone())
    }

    /// The actions that name a browser entry with the entry's file in its place; the others, and the ones that
    /// already have the file, as they are. `port.browse.unknown_entry` for an id never given out
    pub fn resolve(&self, action: &str, params: Value) -> Result<Value> {
        let entry = match action {
            "plugin.load_preset" => params.get("entry").and_then(Value::as_int),
            "project.open_entry" | "browser.reveal" => {
                params.as_int().or_else(|| params.get("entry").and_then(Value::as_int))
            }
            _ => return Ok(params),
        };
        let Some(entry) = entry else { return Ok(params) };
        let path = self
            .path(EntryId(u64::try_from(entry).unwrap_or(0)))
            .ok_or_else(|| Error::new("port.browse.unknown_entry"))?;
        let text = Value::text(path.display().to_string());
        Ok(match action {
            "plugin.load_preset" => params.with("path", text).without("entry"),
            "project.open_entry" => Value::object([("folder", text)]),
            _ => Value::object([("path", text)]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> BrowserEntries {
        let mut b = BrowserEntries::default();
        b.remember(EntryId(7), BrowsePage::Presets, Some(PathBuf::from("/tmp/a.json")));
        b.remember(EntryId(8), BrowsePage::Presets, None);
        b
    }

    #[test]
    fn an_entry_becomes_its_file() {
        let b = entries();
        let p =
            b.resolve("plugin.load_preset", Value::object([("node", Value::text("n:1")), ("entry", Value::int(7))]));
        let p = p.unwrap();
        assert_eq!(p.get("path").and_then(Value::as_text), Some("/tmp/a.json"));
        assert!(p.get("entry").is_none());
        let o = b.resolve("project.open_entry", Value::int(7)).unwrap();
        assert_eq!(o.get("folder").and_then(Value::as_text), Some("/tmp/a.json"));
        let r = b.resolve("browser.reveal", Value::object([("entry", Value::int(7))])).unwrap();
        assert_eq!(r.get("path").and_then(Value::as_text), Some("/tmp/a.json"));
    }

    #[test]
    fn unknown_entries_and_other_actions() {
        let b = entries();
        assert_eq!(b.resolve("browser.reveal", Value::int(99)).unwrap_err().key(), "port.browse.unknown_entry");
        assert_eq!(b.resolve("browser.reveal", Value::int(8)).unwrap_err().key(), "port.browse.unknown_entry");
        let params = Value::object([("path", Value::text("/x"))]);
        assert_eq!(b.resolve("browser.reveal", params.clone()).unwrap(), params);
        assert_eq!(b.resolve("transport.play", Value::NULL).unwrap(), Value::NULL);
    }
}
