use std::fmt;

/// An argument of a translatable text. Plain data, so a label can cross threads and be serialised
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Int(i64),
    Real(f64),
    Text(String),
}

impl From<i64> for Arg {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<f64> for Arg {
    fn from(v: f64) -> Self {
        Self::Real(v)
    }
}

impl From<String> for Arg {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}

impl From<&str> for Arg {
    fn from(v: &str) -> Self {
        Self::Text(v.to_owned())
    }
}

/// A translatable text: a stable key and its arguments (interfaces.md, C13). The core never writes text for the
/// user; the interface translates the key
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub key: &'static str,
    pub args: Vec<(&'static str, Arg)>,
}

impl Label {
    pub fn new(key: &'static str) -> Self {
        Self { key, args: Vec::new() }
    }

    #[must_use]
    pub fn arg(mut self, name: &'static str, value: impl Into<Arg>) -> Self {
        self.args.push((name, value.into()));
        self
    }
}

impl fmt::Display for Label {
    /// For logs and tests only: the key and the arguments, untranslated
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key)?;
        for (name, value) in &self.args {
            match value {
                Arg::Int(v) => write!(f, " {name}={v}")?,
                Arg::Real(v) => write!(f, " {name}={v}")?,
                Arg::Text(v) => write!(f, " {name}={v:?}")?,
            }
        }
        Ok(())
    }
}

/// An error: what went wrong, as a translatable label (interfaces.md, Parte 2, rule 7)
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub label: Label,
}

impl Error {
    pub fn new(key: &'static str) -> Self {
        Self { label: Label::new(key) }
    }

    #[must_use]
    pub fn arg(mut self, name: &'static str, value: impl Into<Arg>) -> Self {
        self.label = self.label.arg(name, value);
        self
    }

    /// The key, for matching in code and tests
    pub fn key(&self) -> &'static str {
        self.label.key
    }

    /// An operation cancelled by its `Cancel`
    pub fn cancelled() -> Self {
        Self::new("error.cancelled")
    }

    pub fn is_cancelled(&self) -> bool {
        self.key() == "error.cancelled"
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.label.fmt(f)
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_carry_typed_arguments() {
        let label = Label::new("disk.full").arg("needed", 30_i64).arg("path", "audio/x");
        assert_eq!(label.to_string(), "disk.full needed=30 path=\"audio/x\"");
    }

    #[test]
    fn cancelled_is_recognised() {
        assert!(Error::cancelled().is_cancelled());
        assert!(!Error::new("other").is_cancelled());
    }
}
