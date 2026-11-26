//! LinuxMusic: a safe evaluator of the exprtk expressions the legacy Xpressive instrument used (exprtk<float> with
//! its parser settings and compile-time rewrites). In progress: the parser, the compiler and the VM are here, the Xpressive helpers follow

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

pub use compile::Expr;
pub use error::{CompileError, SymbolError, key};
pub use symbols::{FnDef, FnId, Symbols, VarId};
pub use vm::{Host, NoHost};
