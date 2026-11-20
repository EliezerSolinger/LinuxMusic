//! The notices (I-14): a queue the interface shows and `lm-headless` prints. The core never writes finished text: a
//! notice is a key with its arguments. The same notice again counts as a repeat, never a second line.

use lm_base::{Arg, Error, Label};
use lm_port::{ActionId, Notice, Severity, Target, Value};

/// At most this many notices are kept; the oldest read ones go first
const KEEP: usize = 200;

#[derive(Debug, Default)]
pub struct Notices {
    list: Vec<Notice>,
    next: u64,
    /// Notices given out by `take_new` (the headless printer's cursor)
    printed: u64,
}

fn arg_value(a: &Arg) -> Value {
    match a {
        Arg::Int(i) => Value::int(*i),
        Arg::Real(r) => Value::real(*r).unwrap_or(Value::NULL),
        Arg::Text(t) => Value::text(t.as_str()),
    }
}

impl Notices {
    pub fn push(
        &mut self,
        key: &'static str,
        args: Vec<(&'static str, Value)>,
        severity: Severity,
        target: Option<Target>,
        actions: Vec<(ActionId, Value)>,
    ) {
        if let Some(n) = self.list.iter_mut().find(|n| n.key == key && n.args == args && n.target == target) {
            n.repeats += 1;
            n.read = false;
            return;
        }
        self.next += 1;
        self.list.push(Notice { id: self.next, key, args, severity, target, actions, repeats: 0, read: false });
        if self.list.len() > KEEP {
            let i = self.list.iter().position(|n| n.read).unwrap_or(0);
            self.list.remove(i);
        }
    }

    /// A label from the document or a plugin as a notice
    pub fn label(&mut self, label: &Label, severity: Severity) {
        let args = label.args.iter().map(|(k, a)| (*k, arg_value(a))).collect();
        self.push(label.key, args, severity, None, Vec::new());
    }

    /// An error as a notice, on a target, with "try again" when an action is given
    pub fn error(&mut self, error: &Error, target: Option<Target>, retry: Option<(ActionId, Value)>) {
        let args = error.label.args.iter().map(|(k, a)| (*k, arg_value(a))).collect();
        self.push(error.key(), args, Severity::Error, target, retry.into_iter().collect());
    }

    pub fn warn(&mut self, key: &'static str, args: Vec<(&'static str, Value)>) {
        self.push(key, args, Severity::Warning, None, Vec::new());
    }

    pub fn info(&mut self, key: &'static str, args: Vec<(&'static str, Value)>) {
        self.push(key, args, Severity::Info, None, Vec::new());
    }

    /// Every notice kept (the interface marks what it showed as read)
    pub fn all(&mut self) -> Vec<Notice> {
        let out = self.list.clone();
        for n in &mut self.list {
            n.read = true;
        }
        out
    }

    pub fn dismiss(&mut self, id: u64) {
        self.list.retain(|n| n.id != id);
    }

    /// The notices not given out by this call before (the headless printer)
    pub fn take_new(&mut self) -> Vec<Notice> {
        let from = self.printed;
        self.printed = self.next;
        self.list.iter().filter(|n| n.id > from).cloned().collect()
    }

    pub fn keys(&self) -> Vec<&'static str> {
        self.list.iter().map(|n| n.key).collect()
    }
}
