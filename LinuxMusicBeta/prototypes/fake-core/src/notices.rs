//! Owner: agent D. The notice queue (I-14) and the activity list (I-2.2) of the fake core.
//!
//! A notice is data, never ready text: an English key the interface translates, its arguments, a severity, the
//! object it is about, and the actions it offers (ids of the action registry, `actions.rs`, with an argument). The
//! core never writes a sentence; the interface shows it and `lm-headless` prints it.
//!
//! The activity list is every long operation running (I-2.2 `Session::activity`), built from the session's
//! operations; this module only has its type.

use std::time::{Duration, Instant};

use crate::Target;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// An action a notice offers: the registry's id and its argument ("try again" says which operation)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeAction {
    pub action: &'static str,
    pub arg: String,
}

impl NoticeAction {
    pub fn new(action: &'static str, arg: impl Into<String>) -> NoticeAction {
        NoticeAction { action, arg: arg.into() }
    }
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub id: u64,
    /// The English key, with `{0}`, `{1}`... for the arguments
    pub key: &'static str,
    pub args: Vec<String>,
    pub severity: Severity,
    pub target: Option<Target>,
    pub actions: Vec<NoticeAction>,
    /// When it came (the last time, if it repeated)
    pub at: Instant,
    /// The same notice again: counted, not stacked (an autosave failing every minute is one line)
    pub repeats: u32,
    pub read: bool,
}

/// The session's queue (I-14)
#[derive(Debug, Default)]
pub struct Queue {
    items: Vec<Notice>,
    next: u64,
    /// Bumped on every change, so a view knows when to rebuild
    pub serial: u64,
}

impl Queue {
    /// Adds a notice, or counts a repeat of the same key and arguments
    pub fn push(
        &mut self,
        key: &'static str,
        args: Vec<String>,
        severity: Severity,
        target: Option<Target>,
        actions: Vec<NoticeAction>,
    ) -> u64 {
        self.serial += 1;
        if let Some(n) = self.items.iter_mut().find(|n| n.key == key && n.args == args) {
            n.repeats += 1;
            n.at = Instant::now();
            n.read = false;
            n.severity = n.severity.max(severity);
            return n.id;
        }
        self.next += 1;
        let id = self.next;
        self.items.push(Notice { id, key, args, severity, target, actions, at: Instant::now(), repeats: 0, read: false });
        id
    }

    /// Newest first
    pub fn iter(&self) -> impl Iterator<Item = &Notice> {
        self.items.iter().rev()
    }

    pub fn get(&self, id: u64) -> Option<&Notice> {
        self.items.iter().find(|n| n.id == id)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn unread(&self) -> usize {
        self.items.iter().filter(|n| !n.read).count()
    }

    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|n| n.id != id);
        self.serial += 1;
    }

    /// Removes the notices `f` picks (an action that resolved them: "allow this folder")
    pub fn dismiss_matching(&mut self, f: impl Fn(&Notice) -> bool) {
        self.items.retain(|n| !f(n));
        self.serial += 1;
    }

    pub fn actions_mut(&mut self, id: u64) -> Option<&mut Vec<NoticeAction>> {
        self.serial += 1;
        self.items.iter_mut().find(|n| n.id == id).map(|n| &mut n.actions)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.serial += 1;
    }

    pub fn mark_read(&mut self) {
        if self.items.iter().any(|n| !n.read) {
            self.items.iter_mut().for_each(|n| n.read = true);
            self.serial += 1;
        }
    }

    /// The notices a fresh session of the prototype starts with: one of each kind the program has
    pub fn with_examples() -> Queue {
        let mut q = Queue::default();
        q.push(
            "Real-time priority is not available: the audio may crackle under load",
            vec![],
            Severity::Warning,
            Some(Target::Device),
            vec![NoticeAction::new("settings.realtime", "")],
        );
        q.push(
            "{0} stopped responding on {1} and was made silent",
            vec!["Vital (VST3)".into(), "Teclado 1".into()],
            Severity::Error,
            None,
            vec![NoticeAction::new("plugin.recreate", "Vital (VST3)")],
        );
        q.push(
            "The file of {0} is not where the project says",
            vec!["Kit Acústico.sf2".into()],
            Severity::Warning,
            None,
            vec![NoticeAction::new("resource.locate", "Kit Acústico.sf2"), NoticeAction::new("resource.locate_folder", "")],
        );
        q.push(
            "{0} is outside the folders this program may read",
            vec!["/media/externo/SoundFonts/Piano.sf2".into()],
            Severity::Warning,
            None,
            vec![NoticeAction::new("folder.allow", "/media/externo/SoundFonts")],
        );
        q.push("Autosaved {0}", vec!["Minha Música".into()], Severity::Info, Some(Target::Project), vec![]);
        q.mark_read();
        q
    }
}

/// One running long operation, as the activity list shows it (I-2.2)
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub op: u64,
    /// The English key of what it is doing ("Exporting")
    pub stage: &'static str,
    pub target: Target,
    pub progress: Option<f32>,
    pub cancellable: bool,
    pub since: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_are_counted_not_stacked() {
        let mut q = Queue::default();
        let a = q.push("Autosaved {0}", vec!["x".into()], Severity::Info, None, vec![]);
        let b = q.push("Autosaved {0}", vec!["x".into()], Severity::Warning, None, vec![]);
        assert_eq!(a, b);
        assert_eq!(q.len(), 1);
        let n = q.get(a).unwrap();
        assert_eq!((n.repeats, n.severity), (1, Severity::Warning));
        q.push("Autosaved {0}", vec!["y".into()], Severity::Info, None, vec![]);
        assert_eq!(q.len(), 2);
        assert_eq!(q.iter().next().unwrap().args[0], "y", "newest first");
        q.dismiss(a);
        assert_eq!(q.len(), 1);
    }
}
