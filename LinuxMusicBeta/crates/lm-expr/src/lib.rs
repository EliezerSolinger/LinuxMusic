//! LinuxMusic: a safe evaluator of the exprtk expressions the legacy Xpressive instrument used (exprtk<float> with
//! its parser settings and compile-time rewrites). The A/B harness against the real exprtk is in `ab/`

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
