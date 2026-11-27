//! An editor plays one item alone (F-NOT-017, F-AUT-011): `item.audition {item, from?, to?, loop?}` compiles the song
//! with every other item silent and the loop of the item's own range (`lm_compile::Audition`, never in the
//! document), and starts the transport when that program is installed, so the song is never heard in between.
//! `item.audition_stop` stops and gives the project's own program back. The transport's position is the item's
//! place in the song, so the editor's cursor follows the telemetry like it does for any playing.

use lm_base::{Error, Result};
use lm_compile::Audition;
use lm_state::{NodeId, Value};

use crate::session::Session;

impl Session {
    /// Starts playing `item` alone, from `from` to `to` (ticks inside the item; the whole item, or its repeating
    /// part, by default), in a loop when `repeat`
    pub(crate) fn audition_start(&mut self, item: NodeId, from: Option<i64>, to: Option<i64>, repeat: bool) -> Result<()> {
        let root = self.document.root();
        let found = root
            .get("tracks")
            .into_iter()
            .flat_map(Value::items)
            .flat_map(|t| t.get("items").into_iter().flat_map(Value::items))
            .find(|i| i.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()) == Some(item))
            .ok_or_else(|| Error::new("action.node.missing").arg("node", item.to_string()))?;
        let int = |k: &str| found.get(k).and_then(Value::as_int);
        let start = int("start").unwrap_or(0).max(0);
        let length = int("length").unwrap_or(0).max(1);
        // a pattern repeats its `loop` part: that is the range to play
        let span = int("loop").filter(|l| *l > 0).map_or(length, |l| l.min(length));
        let a = from.unwrap_or(0).clamp(0, span);
        let b = to.unwrap_or(span).clamp(a + 1, span.max(a + 1));
        let map = crate::session::tempo_map(root)?;
        let begin = map.sample_at(start + a);
        let audition = Audition { item, from: start + a, to: start + b, repeat };
        // `stop` brings the playhead back to where the song was before the audition
        if !self.telemetry.position.playing && self.midi.audition.is_none() {
            self.play_origin = Some(self.telemetry.position.samples);
        }
        self.midi.audition = Some(audition);
        self.midi.audition_start = Some(begin);
        // Playing now would play the song until the program of the audition is installed
        let _ = self.engine.commands().send(lm_render::Command::Stop);
        self.needs_compile = true;
        Ok(())
    }

    /// The song's own transport takes over (it plays or stops): the project's own program is compiled again
    pub(crate) fn audition_clear(&mut self) {
        self.midi.audition_start = None;
        if self.midi.audition.take().is_some() {
            self.needs_compile = true;
        }
    }

    /// Stops the audition and compiles the project's own program again
    pub(crate) fn audition_stop(&mut self) {
        self.stop();
    }

    /// A program was installed: when it is the audition's (nothing else waits to be compiled), the play starts
    pub(crate) fn audition_installed(&mut self) {
        if self.needs_compile || self.compiler.pending() {
            return;
        }
        if let Some(sample) = self.midi.audition_start.take() {
            let _ = self.engine.commands().send(lm_render::Command::Locate { sample });
            let _ = self.engine.commands().send(lm_render::Command::Play);
        }
    }
}
