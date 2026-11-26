//! LinuxMusic: a safe evaluator of the exprtk expressions the legacy Xpressive instrument used (exprtk<float> with
//! its parser settings and compile-time rewrites). The A/B harness against the real exprtk is in `ab/`.
//!
//! Flow: build a [`Symbols`] (or [`xpressive::symbols`] for the legacy's), [`Expr::compile`] each text off the audio
//! thread, then per sample write the variables and call [`Expr::eval`] with a stack of [`Expr::stack_len`] values and
//! a [`Host`] for the functions. `eval` never allocates and never panics.
//!
//! Ported from the bundled exprtk: operators and precedence (`^` right-associative, `-x^2` is `-(x^2)`), comparison
//! with exprtk's epsilon, logic words, `?:` and `if`, the builtin functions, implicit products (`2x`), constant
//! folding, the cardinal power rewrite, the identity rewrites (`x * 0`, `x / 0`), the three-leaf strength reductions
//! (`(a / b) / c`) and the constant chains (`2 * (3 * x)`). Not ported: the four-leaf synthesisers (for example
//! `(a / b) * (c / d)`), which exprtk rewrites by the same kind of rule; expressions that hit them may differ from
//! the legacy in the last bit. Loops, assignment, strings, vectors and local variables are left out (the legacy
//! disabled them).

#![forbid(unsafe_code)]

mod ast;
mod builtin;
mod compile;
mod error;
mod lexer;
mod ops;
mod opt;
mod parser;
mod symbols;
mod vm;
pub mod xpressive;

pub use compile::Expr;
pub use error::{CompileError, SymbolError, key};
pub use symbols::{FnDef, FnId, Symbols, VarId};
pub use vm::{Host, NoHost};
