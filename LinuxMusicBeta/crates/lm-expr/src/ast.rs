//! The tree the parser builds. Its nodes are built through the smart constructors of `opt` (exprtk's compile-time
//! rewrites), so the tree is what the legacy's compiled expression was, and the compiler flattens it as it is

use crate::ops::{Op1, Op2, OpN};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    Const(f32),
    /// A variable of the table, by its index
    Var(u32),
    Un(Op1, Box<Node>),
    Bin(Op2, Box<Node>, Box<Node>),
    Many(OpN, Vec<Node>),
    /// `base ^ n` for an integer constant `n` of 1 to 60 (exprtk's cardinal power optimisation): the multiplications
    /// of `fast_exp`, or `1 / fast_exp` when `inv`
    IPow {
        base: Box<Node>,
        n: u32,
        inv: bool,
    },
    /// A call to a host function, arguments left to right
    Call {
        f: u32,
        args: Vec<Node>,
    },
    /// `cond ? a : b`; no alternative gives NaN when the condition is false (`if (c, a)`)
    If(Box<Node>, Box<Node>, Option<Box<Node>>),
    /// `a; b; c`: all evaluated in order, the last is the value
    Seq(Vec<Node>),
}

impl Node {
    pub(crate) fn is_const(&self) -> bool {
        matches!(self, Node::Const(_))
    }

    pub(crate) fn is_var(&self) -> bool {
        matches!(self, Node::Var(_))
    }

    /// A node that is neither a constant nor a variable: exprtk's "branch"
    pub(crate) fn is_branch(&self) -> bool {
        !self.is_const() && !self.is_var()
    }
}
