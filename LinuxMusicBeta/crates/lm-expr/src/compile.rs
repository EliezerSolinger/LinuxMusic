//! The compiler: the tree to a flat program for a stack machine, with the stack depth worked out so evaluation
//! needs a caller-provided slice and never allocates. Compiling allocates and parses; it belongs off the audio thread

use crate::ast::Node;
use crate::error::CompileError;
use crate::ops::{Op1, Op2, OpN};
use crate::parser::parse;
use crate::symbols::{FnId, Symbols};

/// One instruction. Every expression node leaves exactly one value on the stack
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Ins {
    Const(f32),
    /// Pushes `vars[index]` (0 when the slice is shorter)
    Var(u32),
    /// Replaces the top by `op(top)`
    Un(Op1),
    /// Pops two, pushes `op(a, b)`
    Bin(Op2),
    /// Pops `n`, pushes the builtin of them
    Many(OpN, u8),
    /// Replaces the top by `fast_exp(top, n)`, or `1 / fast_exp(top, n)`
    IPow {
        n: u32,
        inv: bool,
    },
    /// Pops `argc` arguments, calls the host, pushes the result. `site` counts the calls of `f` in evaluation order
    Call {
        f: u32,
        site: u32,
        argc: u8,
    },
    /// Pops the condition; jumps to the instruction when it is false
    JumpIfFalse(u32),
    Jump(u32),
    /// Drops the top (the value of a statement that is not the last)
    Pop,
    PushNan,
}

/// A compiled expression: evaluates without allocating, over a stack the caller provides. Compile once (off the
/// audio thread) against a [`Symbols`]; the same `Symbols` ids are valid in every expression compiled with it
#[derive(Clone, Debug)]
pub struct Expr {
    pub(crate) code: Vec<Ins>,
    stack_len: usize,
    side_effects: bool,
    /// Calls per host function, by `FnId` index
    call_counts: Vec<u32>,
    constant: Option<f32>,
}

struct Emitter<'a> {
    code: Vec<Ins>,
    depth: usize,
    max: usize,
    symbols: &'a Symbols,
    call_counts: Vec<u32>,
    side_effects: bool,
}

impl Emitter<'_> {
    fn push(&mut self, ins: Ins, delta: isize) {
        self.code.push(ins);
        self.depth = self.depth.saturating_add_signed(delta);
        self.max = self.max.max(self.depth);
    }

    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    fn node(&mut self, n: &Node) {
        match n {
            Node::Const(v) => self.push(Ins::Const(*v), 1),
            Node::Var(i) => self.push(Ins::Var(*i), 1),
            Node::Un(op, x) => {
                self.node(x);
                self.push(Ins::Un(*op), 0);
            }
            Node::Bin(op, l, r) => {
                self.node(l);
                self.node(r);
                self.push(Ins::Bin(*op), -1);
            }
            Node::Many(op, args) => {
                for a in args {
                    self.node(a);
                }
                let argc = args.len().min(255) as u8;
                self.push(Ins::Many(*op, argc), 1 - isize::from(argc));
            }
            Node::IPow { base, n, inv } => {
                self.node(base);
                self.push(Ins::IPow { n: *n, inv: *inv }, 0);
            }
            Node::Call { f, args } => {
                for a in args {
                    self.node(a);
                }
                let argc = args.len().min(255) as u8;
                let slot = self.call_counts.get_mut(*f as usize);
                let site = match slot {
                    Some(count) => {
                        let site = *count;
                        *count += 1;
                        site
                    }
                    None => 0,
                };
                if self.symbols.fn_def(FnId(*f)).is_some_and(|d| d.side_effects) {
                    self.side_effects = true;
                }
                self.push(Ins::Call { f: *f, site, argc }, 1 - isize::from(argc));
            }
            Node::If(c, t, f) => {
                self.node(c);
                let jump_else = self.here();
                self.push(Ins::JumpIfFalse(0), -1);
                let base = self.depth;
                self.node(t);
                let jump_end = self.here();
                self.push(Ins::Jump(0), 0);
                let else_at = self.here();
                self.depth = base;
                match f {
                    Some(f) => self.node(f),
                    None => self.push(Ins::PushNan, 1),
                }
                let end = self.here();
                if let Some(Ins::JumpIfFalse(target)) = self.code.get_mut(jump_else as usize) {
                    *target = else_at;
                }
                if let Some(Ins::Jump(target)) = self.code.get_mut(jump_end as usize) {
                    *target = end;
                }
            }
            Node::Seq(items) => {
                for (i, item) in items.iter().enumerate() {
                    self.node(item);
                    if i + 1 < items.len() {
                        self.push(Ins::Pop, -1);
                    }
                }
            }
        }
    }
}

impl Expr {
    /// Parses and compiles `source` against `symbols`. Allocates: not for the audio thread
    pub fn compile(source: &str, symbols: &Symbols) -> Result<Self, CompileError> {
        let tree = parse(source, symbols)?;
        let mut e = Emitter {
            code: Vec::new(),
            depth: 0,
            max: 0,
            symbols,
            call_counts: vec![0; symbols.fn_count()],
            side_effects: false,
        };
        e.node(&tree);
        let constant = match e.code.as_slice() {
            [Ins::Const(v)] => Some(*v),
            _ => None,
        };
        Ok(Self {
            code: e.code,
            stack_len: e.max.max(1),
            side_effects: e.side_effects,
            call_counts: e.call_counts,
            constant,
        })
    }

    /// The length of the stack slice `eval` needs
    pub fn stack_len(&self) -> usize {
        self.stack_len
    }

    /// Whether the expression calls a host function declared with side effects (`rand`, `integrate`): the legacy
    /// compiled such expressions without its optimisations, and they must not be evaluated twice per sample
    pub fn has_side_effects(&self) -> bool {
        self.side_effects
    }

    /// How many calls of `f` the compiled expression holds (the `site` of the calls is below this)
    pub fn call_count(&self, f: FnId) -> u32 {
        self.call_counts.get(f.index()).copied().unwrap_or(0)
    }

    /// The value when the expression folded to a constant at compile time
    pub fn constant(&self) -> Option<f32> {
        self.constant
    }

    /// Number of instructions (for tests and diagnostics)
    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Symbols {
        let mut s = Symbols::new();
        s.add_var("t").unwrap();
        s.add_var("f").unwrap();
        s.add_fn("rand", 0, true).unwrap();
        s.add_fn("integrate", 1, true).unwrap();
        s.add_fn("sinew", 1, false).unwrap();
        s
    }

    /// The deepest tree the parser admits (a left-deep chain of the most tokens) compiles, runs and drops on the
    /// stack of a plain spawned thread (2 MiB), like the one a compile `Operation` runs on
    #[test]
    fn the_deepest_admitted_chain_fits_a_worker_stack() {
        let source = vec!["t"; 3999].join("+");
        let result = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || Expr::compile(&source, &table()).map(|e| e.len()))
            .expect("thread")
            .join()
            .expect("no stack overflow");
        assert!(result.is_ok());
        let too_long = vec!["t"; 4100].join("+");
        assert_eq!(Expr::compile(&too_long, &table()).unwrap_err().label.key, crate::error::key::TOO_DEEP);
    }

    #[test]
    fn constants_fold_to_one_instruction() {
        let e = Expr::compile("1 + 2 * 3", &table()).unwrap();
        assert_eq!(e.constant(), Some(7.0));
        assert_eq!(e.len(), 1);
        assert_eq!(e.stack_len(), 1);
    }

    #[test]
    fn stack_depth_is_exact() {
        let t = table();
        // t + (f * (t - f)): t, f, t, f on the stack at the deepest
        let e = Expr::compile("t + (f * (t - f))", &t).unwrap();
        assert_eq!(e.stack_len(), 4);
        let e = Expr::compile("t + f", &t).unwrap();
        assert_eq!(e.stack_len(), 2);
        let e = Expr::compile("min(t, f, 3, 4)", &t).unwrap();
        assert_eq!(e.stack_len(), 4);
    }

    #[test]
    fn side_effects_and_sites() {
        let t = table();
        let e = Expr::compile("sinew(t) * 2", &t).unwrap();
        assert!(!e.has_side_effects());
        let e = Expr::compile("integrate(t) + integrate(f) + rand()", &t).unwrap();
        assert!(e.has_side_effects());
        assert_eq!(e.call_count(t.function("integrate").unwrap()), 2);
        assert_eq!(e.call_count(t.function("rand").unwrap()), 1);
        // The sites are numbered in evaluation order
        let sites: Vec<(u32, u32)> = e
            .code
            .iter()
            .filter_map(|i| match i {
                Ins::Call { f, site, .. } => Some((*f, *site)),
                _ => None,
            })
            .collect();
        assert_eq!(sites, vec![(1, 0), (1, 1), (0, 0)]);
    }

    #[test]
    fn an_eliminated_call_is_not_counted() {
        let t = table();
        // The legacy's rewrite: x * 0 drops its operand
        let e = Expr::compile("rand() * 0", &t).unwrap();
        assert_eq!(e.constant(), Some(0.0));
        assert!(!e.has_side_effects());
    }

    #[test]
    fn branches_patch_their_jumps() {
        let t = table();
        let e = Expr::compile("t > 0 ? f : 2", &t).unwrap();
        assert!(e.code.iter().any(|i| matches!(i, Ins::JumpIfFalse(_))));
        assert_eq!(e.stack_len(), 2);
    }
}
