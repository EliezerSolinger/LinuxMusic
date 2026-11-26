//! Compile errors: where in the text and which translatable label (the core never writes text for the user)

use core::fmt;

use lm_base::Label;

/// A failed compile: the byte position in the source of the token the parser stopped at and a label key with its
/// arguments. The keys are stable (`expr.error.*`); the interface translates them
#[derive(Clone, Debug, PartialEq)]
pub struct CompileError {
    /// Byte offset into the source text (exprtk's `token.position`)
    pub position: usize,
    pub label: Label,
}

impl CompileError {
    pub(crate) fn new(position: usize, key: &'static str) -> Self {
        Self { position, label: Label::new(key) }
    }

    pub(crate) fn with_name(position: usize, key: &'static str, name: &str) -> Self {
        Self { position, label: Label::new(key).arg("name", name) }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.label, self.position)
    }
}

impl std::error::Error for CompileError {}

/// The label keys of [`CompileError`]
pub mod key {
    /// A character the language does not have
    pub const BAD_CHARACTER: &str = "expr.error.bad-character";
    /// A number that does not convert to a float (or its exponent is out of the float's range)
    pub const BAD_NUMBER: &str = "expr.error.bad-number";
    /// Brackets that do not match
    pub const BRACKET: &str = "expr.error.bracket";
    /// Nothing to evaluate
    pub const EMPTY: &str = "expr.error.empty";
    /// The text ends where an operand was expected
    pub const PREMATURE_END: &str = "expr.error.premature-end";
    /// A token where it cannot be (missing operator, stray symbol)
    pub const SYNTAX: &str = "expr.error.syntax";
    /// A name that is no variable, constant or function (argument `name`)
    pub const UNKNOWN_SYMBOL: &str = "expr.error.unknown-symbol";
    /// A function called with the wrong number of arguments (argument `name`)
    pub const ARITY: &str = "expr.error.arity";
    /// A variable used as a function (argument `name`)
    pub const NOT_A_FUNCTION: &str = "expr.error.not-a-function";
    /// An operator or statement of exprtk the legacy Xpressive disabled or this crate does not have
    pub const UNSUPPORTED: &str = "expr.error.unsupported";
    /// The expression nests deeper than the parser accepts
    pub const TOO_DEEP: &str = "expr.error.too-deep";
    /// `?` without its `:`
    pub const TERNARY: &str = "expr.error.ternary";
}

/// Why a name could not be registered in [`crate::Symbols`]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolError {
    /// Not a name: empty, or not letters, digits and underscore starting with a letter
    Invalid,
    /// Already registered (names compare case-insensitively, as exprtk's)
    Duplicate,
    /// A builtin function, a keyword or a constant of the language
    Reserved,
    /// Too many symbols of that kind for the index type
    Full,
}

impl fmt::Display for SymbolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid symbol name",
            Self::Duplicate => "symbol already registered",
            Self::Reserved => "reserved symbol name",
            Self::Full => "too many symbols",
        })
    }
}

impl std::error::Error for SymbolError {}
