//! exprtk's compile-time rewrites, as smart constructors of the tree. The legacy Xpressive compiled with exprtk's
//! defaults (constant folding, strength reduction, the enhanced synthesisers, the cardinal power optimisation), and
//! several of those rewrites change float rounding or drop operands (`x * 0` is 0 even for infinity, `x / 0` is NaN,
//! `(a / b) / c` is `a / (b * c)`, `x ^ 3` is `x * x * x`), so the same expression gives the same bits only if the
//! same rewrites apply, in the same order. Everything here follows `expression_generator::operator()` of the
//! bundled exprtk.hpp; what is not ported is listed in the crate documentation (the four-leaf synthesisers)

use crate::ast::Node;
use crate::ops::{Op1, Op2, OpN, eval_n, eval1, eval2, is_true};

/// `- x`: folded for a constant, a double negation cancels
pub(crate) fn neg(x: Node) -> Node {
    match x {
        Node::Const(v) => Node::Const(-v),
        Node::Un(Op1::Neg, inner) => *inner,
        other => Node::Un(Op1::Neg, Box::new(other)),
    }
}

/// A unary builtin: folded for a constant argument
pub(crate) fn un(op: Op1, x: Node) -> Node {
    if op == Op1::Neg {
        return neg(x);
    }
    match x {
        Node::Const(v) => Node::Const(eval1(op, v)),
        other => Node::Un(op, Box::new(other)),
    }
}

/// A builtin of any arity: folded when every argument is a constant
pub(crate) fn many(op: OpN, args: Vec<Node>) -> Node {
    if !args.is_empty() && args.iter().all(Node::is_const) {
        let values: Vec<f32> = args
            .iter()
            .map(|n| match n {
                Node::Const(v) => *v,
                _ => 0.0,
            })
            .collect();
        return Node::Const(eval_n(op, &values));
    }
    Node::Many(op, args)
}

/// `c ? a : b` and `if (c, a, b)`: a constant condition picks its branch
pub(crate) fn cond(c: Node, then: Node, otherwise: Option<Node>) -> Node {
    if let Node::Const(v) = c {
        return if is_true(v) { then } else { otherwise.unwrap_or(Node::Const(f32::NAN)) };
    }
    Node::If(Box::new(c), Box::new(then), otherwise.map(Box::new))
}

pub(crate) fn seq(mut items: Vec<Node>) -> Node {
    if items.len() == 1 {
        return items.remove(0);
    }
    Node::Seq(items)
}

fn raw(op: Op2, l: Node, r: Node) -> Node {
    Node::Bin(op, Box::new(l), Box::new(r))
}

/// The operations the synthesisers take (exprtk `operation_optimisable`)
fn optimisable(op: Op2) -> bool {
    matches!(
        op,
        Op2::Add
            | Op2::Sub
            | Op2::Mul
            | Op2::Div
            | Op2::Mod
            | Op2::Pow
            | Op2::Lt
            | Op2::Lte
            | Op2::Gt
            | Op2::Gte
            | Op2::Eq
            | Op2::Ne
            | Op2::And
            | Op2::Nand
            | Op2::Or
            | Op2::Nor
            | Op2::Xor
            | Op2::Xnor
    )
}

fn is_arith(op: Op2) -> bool {
    matches!(op, Op2::Add | Op2::Sub | Op2::Mul | Op2::Div)
}

/// exprtk `cardinal_pow_optimisable`: an integer exponent of magnitude 60 or less
fn cardinal(c: f32) -> bool {
    c.abs() <= 60.0 && libm::fmodf(c, 1.0) == 0.0
}

/// `base ^ c` for a cardinal `c`, base not a constant (a variable or a branch)
fn ipow(base: Node, c: f32) -> Node {
    let p = c.abs() as u32;
    if p == 0 {
        return Node::Const(1.0);
    }
    if c >= 0.0 {
        if p == 1 {
            return base;
        }
        if p == 2 && base.is_var() {
            // vov_node<mul>(v, v)
            let twin = base.clone();
            return raw(Op2::Mul, base, twin);
        }
    }
    Node::IPow { base: Box::new(base), n: p, inv: c < 0.0 }
}

/// A leaf of the synthesisers: a variable or a constant
#[derive(Clone, Copy)]
enum Leaf {
    V(u32),
    C(f32),
}

fn leaf(n: &Node) -> Option<Leaf> {
    match n {
        Node::Var(v) => Some(Leaf::V(*v)),
        Node::Const(c) => Some(Leaf::C(*c)),
        _ => None,
    }
}

fn leaf_node(l: Leaf) -> Node {
    match l {
        Leaf::V(v) => Node::Var(v),
        Leaf::C(c) => Node::Const(c),
    }
}

/// A node of two leaves (exprtk's vov, voc and cov nodes)
fn pair(n: &Node) -> Option<(Op2, Leaf, Leaf)> {
    if let Node::Bin(op, l, r) = n {
        if let (Some(a), Some(b)) = (leaf(l), leaf(r)) {
            return Some((*op, a, b));
        }
    }
    None
}

fn cov(op: Op2, c: f32, v: u32) -> Node {
    raw(op, Node::Const(c), Node::Var(v))
}

fn voc(op: Op2, v: u32, c: f32) -> Node {
    raw(op, Node::Var(v), Node::Const(c))
}

fn mul(a: Node, b: Node) -> Node {
    raw(Op2::Mul, a, b)
}

fn div(a: Node, b: Node) -> Node {
    raw(Op2::Div, a, b)
}

/// The rewrites of two leaves (`v o c`, `c o v`) and of three (`(a o b) o c`, `a o (b o c)`); `None` where no
/// rewrite applies and the plain node is built
fn leaf_rules(op: Op2, l: &Node, r: &Node) -> Option<Node> {
    if !optimisable(op) {
        return None;
    }
    match (l, r) {
        (Node::Var(v), Node::Const(c)) => voc_rules(op, *v, *c),
        (Node::Const(c), Node::Var(v)) => {
            let (c, v) = (*c, *v);
            if c == 0.0 && op == Op2::Mul {
                Some(Node::Const(0.0))
            } else if c == 0.0 && op == Op2::Div {
                Some(Node::Const(0.0))
            } else if c == 0.0 && op == Op2::Add {
                Some(Node::Var(v))
            } else if c == 1.0 && op == Op2::Mul {
                Some(Node::Var(v))
            } else {
                None
            }
        }
        _ => {
            if let (Some((o0, a, b)), Some(z)) = (pair(l), leaf(r)) {
                if optimisable(o0) {
                    return three_left(o0, a, b, op, z);
                }
            } else if let (Some(x), Some((o1, a, b))) = (leaf(l), pair(r)) {
                if optimisable(o1) {
                    return three_right(op, x, o1, a, b);
                }
            }
            None
        }
    }
}

fn voc_rules(op: Op2, v: u32, c: f32) -> Option<Node> {
    if op == Op2::Pow && cardinal(c) {
        return Some(if c == 1.0 { Node::Var(v) } else { ipow(Node::Var(v), c) });
    }
    if c == 0.0 && op == Op2::Mul {
        Some(Node::Const(0.0))
    } else if c == 0.0 && op == Op2::Div {
        Some(Node::Const(f32::NAN))
    } else if c == 0.0 && op == Op2::Add {
        Some(Node::Var(v))
    } else if c == 1.0 && op == Op2::Mul {
        Some(Node::Var(v))
    } else if c == 1.0 && op == Op2::Div {
        Some(Node::Var(v))
    } else {
        None
    }
}

/// `(a o0 b) o1 z`: vovov0, vovoc0, vocov0, covov0, covoc0, vococ0
fn three_left(o0: Op2, a: Leaf, b: Leaf, o1: Op2, z: Leaf) -> Option<Node> {
    let div_div = o0 == Op2::Div && o1 == Op2::Div;
    match (a, b, z) {
        // (v0 / v1) / v2 -> v0 / (v1 * v2); (v0 / v1) / c -> v0 / (v1 * c)
        (Leaf::V(v0), Leaf::V(v1), z) if div_div => Some(div(Node::Var(v0), mul(Node::Var(v1), leaf_node(z)))),
        // (v0 / c) / v1 -> v0 / (v1 * c)
        (Leaf::V(v0), Leaf::C(c), Leaf::V(v1)) if div_div => {
            Some(div(Node::Var(v0), mul(Node::Var(v1), Node::Const(c))))
        }
        // (c / v0) / v1 -> c / (v0 * v1)
        (Leaf::C(c), Leaf::V(v0), Leaf::V(v1)) if div_div => {
            Some(div(Node::Const(c), mul(Node::Var(v0), Node::Var(v1))))
        }
        // (c0 o0 v) o1 c1
        (Leaf::C(c0), Leaf::V(v), Leaf::C(c1)) => {
            let (nop, nc) = match (o0, o1) {
                (Op2::Add, Op2::Add) => (Op2::Add, c0 + c1),
                (Op2::Add, Op2::Sub) => (Op2::Add, c0 - c1),
                (Op2::Sub, Op2::Add) => (Op2::Sub, c0 + c1),
                (Op2::Sub, Op2::Sub) => (Op2::Sub, c0 - c1),
                (Op2::Mul, Op2::Mul) => (Op2::Mul, c0 * c1),
                (Op2::Mul, Op2::Div) => (Op2::Mul, c0 / c1),
                (Op2::Div, Op2::Mul) => (Op2::Div, c0 * c1),
                (Op2::Div, Op2::Div) => (Op2::Div, c0 / c1),
                _ => return None,
            };
            Some(cov(nop, nc, v))
        }
        // (v o0 c0) o1 c1
        (Leaf::V(v), Leaf::C(c0), Leaf::C(c1)) => {
            let (nop, nc) = match (o0, o1) {
                (Op2::Add, Op2::Add) => (Op2::Add, c0 + c1),
                (Op2::Add, Op2::Sub) => (Op2::Add, c0 - c1),
                (Op2::Sub, Op2::Add) => (Op2::Add, c1 - c0),
                (Op2::Sub, Op2::Sub) => (Op2::Sub, c0 + c1),
                (Op2::Mul, Op2::Mul) => (Op2::Mul, c0 * c1),
                (Op2::Mul, Op2::Div) => (Op2::Mul, c0 / c1),
                (Op2::Div, Op2::Mul) => (Op2::Mul, c1 / c0),
                (Op2::Div, Op2::Div) => (Op2::Div, c0 * c1),
                (Op2::Pow, Op2::Pow) => (Op2::Pow, c0 * c1),
                _ => return None,
            };
            Some(voc(nop, v, nc))
        }
        _ => None,
    }
}

/// `x o0 (a o1 b)`: vovov1, vovoc1, vocov1, covov1, covoc1, cocov1
fn three_right(o0: Op2, x: Leaf, o1: Op2, a: Leaf, b: Leaf) -> Option<Node> {
    let div_div = o0 == Op2::Div && o1 == Op2::Div;
    match (x, a, b) {
        // v0 / (v1 / v2) -> (v0 * v2) / v1
        (Leaf::V(v0), Leaf::V(v1), Leaf::V(v2)) if div_div => {
            Some(div(mul(Node::Var(v0), Node::Var(v2)), Node::Var(v1)))
        }
        // v0 / (v1 / c) -> (v0 * c) / v1
        (Leaf::V(v0), Leaf::V(v1), Leaf::C(c)) if div_div => {
            Some(div(mul(Node::Var(v0), Node::Const(c)), Node::Var(v1)))
        }
        // v0 / (c / v1) -> (v0 * v1) / c
        (Leaf::V(v0), Leaf::C(c), Leaf::V(v1)) if div_div => {
            Some(div(mul(Node::Var(v0), Node::Var(v1)), Node::Const(c)))
        }
        // c / (v0 / v1) -> (c * v1) / v0
        (Leaf::C(c), Leaf::V(v0), Leaf::V(v1)) if div_div => {
            Some(div(mul(Node::Const(c), Node::Var(v1)), Node::Var(v0)))
        }
        // c0 o0 (v o1 c1)
        (Leaf::C(c0), Leaf::V(v), Leaf::C(c1)) => {
            let (nop, nc) = match (o0, o1) {
                (Op2::Add, Op2::Add) => (Op2::Add, c0 + c1),
                (Op2::Add, Op2::Sub) => (Op2::Add, c0 - c1),
                (Op2::Sub, Op2::Add) => (Op2::Sub, c0 - c1),
                (Op2::Sub, Op2::Sub) => (Op2::Sub, c0 + c1),
                (Op2::Mul, Op2::Mul) => (Op2::Mul, c0 * c1),
                (Op2::Mul, Op2::Div) => (Op2::Mul, c0 / c1),
                (Op2::Div, Op2::Mul) => (Op2::Div, c0 / c1),
                (Op2::Div, Op2::Div) => (Op2::Div, c0 * c1),
                _ => return None,
            };
            Some(cov(nop, nc, v))
        }
        // c0 o0 (c1 o1 v)
        (Leaf::C(c0), Leaf::C(c1), Leaf::V(v)) => {
            let (nop, nc) = match (o0, o1) {
                (Op2::Add, Op2::Add) => (Op2::Add, c0 + c1),
                (Op2::Add, Op2::Sub) => (Op2::Sub, c0 + c1),
                (Op2::Sub, Op2::Add) => (Op2::Sub, c0 - c1),
                (Op2::Sub, Op2::Sub) => (Op2::Add, c0 - c1),
                (Op2::Mul, Op2::Mul) => (Op2::Mul, c0 * c1),
                (Op2::Mul, Op2::Div) => (Op2::Div, c0 * c1),
                (Op2::Div, Op2::Mul) => (Op2::Div, c0 / c1),
                (Op2::Div, Op2::Div) => (Op2::Mul, c0 / c1),
                _ => return None,
            };
            Some(cov(nop, nc, v))
        }
        _ => None,
    }
}

/// A node `c o b` (exprtk's cob): a constant on the left, a branch on the right
fn as_cob(n: &Node) -> Option<(Op2, f32, &Node)> {
    if let Node::Bin(op, l, r) = n {
        if let Node::Const(c) = **l {
            if r.is_branch() {
                return Some((*op, c, r));
            }
        }
    }
    None
}

/// A node `b o c` (boc): a branch on the left, a constant on the right
fn as_boc(n: &Node) -> Option<(Op2, &Node, f32)> {
    if let Node::Bin(op, l, r) = n {
        if let Node::Const(c) = **r {
            if l.is_branch() {
                return Some((*op, l, c));
            }
        }
    }
    None
}

fn cob(op: Op2, c: f32, b: &Node) -> Node {
    raw(op, Node::Const(c), b.clone())
}

fn boc(op: Op2, b: &Node, c: f32) -> Node {
    raw(op, b.clone(), Node::Const(c))
}

/// exprtk's `cocob` and `coboc`: a constant joined to a node that already is `c o b` or `b o c` folds into its
/// constant (`2 * (3 * x)` is `6 * x`, `(x + 1) - 4` is `x + -3`)
fn chain_rules(op: Op2, l: &Node, r: &Node) -> Option<Node> {
    if !is_arith(op) {
        return None;
    }
    if let (Node::Const(c), Some((cop, cc, b))) = (l, as_cob(r)) {
        return c_o_cob(op, *c, cop, cc, b, r);
    }
    if let (Some((cop, cc, b)), Node::Const(c)) = (as_cob(l), r) {
        return cob_o_c(op, cop, cc, b, *c, l);
    }
    if let (Node::Const(c), Some((bop, b, bc))) = (l, as_boc(r)) {
        return c_o_boc(op, *c, bop, b, bc);
    }
    if let (Some((bop, b, bc)), Node::Const(c)) = (as_boc(l), r) {
        return boc_o_c(op, bop, b, bc, *c);
    }
    None
}

/// `c o (cc cop b)`
fn c_o_cob(op: Op2, c: f32, cop: Op2, cc: f32, b: &Node, whole: &Node) -> Option<Node> {
    if c == 0.0 && op == Op2::Mul {
        return Some(Node::Const(0.0));
    }
    if c == 0.0 && op == Op2::Div {
        return Some(Node::Const(0.0));
    }
    if c == 0.0 && op == Op2::Add {
        return Some(whole.clone());
    }
    if c == 1.0 && op == Op2::Mul {
        return Some(whole.clone());
    }
    match (cop, op) {
        (Op2::Add, Op2::Add) => Some(cob(Op2::Add, c + cc, b)),
        (Op2::Add, Op2::Sub) => Some(cob(Op2::Sub, c - cc, b)),
        (Op2::Sub, Op2::Add) => Some(cob(Op2::Sub, c + cc, b)),
        (Op2::Sub, Op2::Sub) => Some(cob(Op2::Add, c - cc, b)),
        (Op2::Mul, Op2::Mul) => Some(cob(Op2::Mul, c * cc, b)),
        (Op2::Mul, Op2::Div) => Some(cob(Op2::Div, c / cc, b)),
        (Op2::Div, Op2::Mul) => Some(cob(Op2::Div, c * cc, b)),
        (Op2::Div, Op2::Div) => Some(cob(Op2::Mul, c / cc, b)),
        _ => None,
    }
}

/// `(cc cop b) o c`
fn cob_o_c(op: Op2, cop: Op2, cc: f32, b: &Node, c: f32, whole: &Node) -> Option<Node> {
    if c == 0.0 && op == Op2::Mul {
        return Some(Node::Const(0.0));
    }
    if c == 0.0 && op == Op2::Div {
        return Some(Node::Const(f32::NAN));
    }
    if c == 0.0 && op == Op2::Add {
        return Some(whole.clone());
    }
    if c == 1.0 && (op == Op2::Mul || op == Op2::Div) {
        return Some(whole.clone());
    }
    match (cop, op) {
        (Op2::Add | Op2::Sub, Op2::Add) => Some(cob(cop, cc + c, b)),
        (Op2::Add | Op2::Sub, Op2::Sub) => Some(cob(cop, cc - c, b)),
        (Op2::Mul, Op2::Mul) => Some(cob(Op2::Mul, cc * c, b)),
        (Op2::Mul, Op2::Div) => Some(cob(Op2::Mul, cc / c, b)),
        (Op2::Div, Op2::Mul) => Some(cob(Op2::Div, cc * c, b)),
        (Op2::Div, Op2::Div) => Some(cob(Op2::Div, cc / c, b)),
        _ => None,
    }
}

/// `(b bop bc) o c`
fn boc_o_c(op: Op2, bop: Op2, b: &Node, bc: f32, c: f32) -> Option<Node> {
    match (bop, op) {
        (Op2::Add, Op2::Add) => Some(boc(Op2::Add, b, bc + c)),
        (Op2::Add, Op2::Sub) => Some(boc(Op2::Add, b, bc - c)),
        (Op2::Mul, Op2::Mul) => Some(boc(Op2::Mul, b, bc * c)),
        (Op2::Mul, Op2::Div) => Some(boc(Op2::Mul, b, bc / c)),
        (Op2::Sub, Op2::Add) => Some(boc(Op2::Add, b, c - bc)),
        (Op2::Sub, Op2::Sub) => Some(boc(Op2::Sub, b, bc + c)),
        (Op2::Div, Op2::Div) => Some(boc(Op2::Div, b, bc * c)),
        (Op2::Div, Op2::Mul) => Some(boc(Op2::Div, b, bc / c)),
        _ => None,
    }
}

/// `c o (b bop bc)`
fn c_o_boc(op: Op2, c: f32, bop: Op2, b: &Node, bc: f32) -> Option<Node> {
    match (bop, op) {
        (Op2::Add, Op2::Add) => Some(boc(Op2::Add, b, c + bc)),
        (Op2::Add, Op2::Sub) => Some(cob(Op2::Sub, c - bc, b)),
        (Op2::Sub, Op2::Add) => Some(boc(Op2::Add, b, c - bc)),
        (Op2::Sub, Op2::Sub) => Some(cob(Op2::Sub, c + bc, b)),
        (Op2::Mul, Op2::Mul) => Some(boc(Op2::Mul, b, c * bc)),
        (Op2::Mul, Op2::Div) => Some(cob(Op2::Div, c / bc, b)),
        (Op2::Div, Op2::Mul) => Some(boc(Op2::Div, b, bc / c)),
        (Op2::Div, Op2::Div) => Some(cob(Op2::Div, c * bc, b)),
        _ => None,
    }
}

/// A binary operator or function of two arguments, through every rewrite in exprtk's order
pub(crate) fn bin(op: Op2, l: Node, r: Node) -> Node {
    if let (Node::Const(a), Node::Const(b)) = (&l, &r) {
        return Node::Const(eval2(op, *a, *b));
    }
    // The cardinal power of a branch
    if op == Op2::Pow && l.is_branch() {
        if let Node::Const(c) = r {
            if cardinal(c) {
                return ipow(l, c);
            }
        }
    }
    if let Some(n) = leaf_rules(op, &l, &r) {
        return n;
    }
    if let Some(n) = chain_rules(op, &l, &r) {
        return n;
    }
    // A constant and a branch (cob, boc): the identities
    match (&l, &r) {
        (Node::Const(c), b) if b.is_branch() && is_arith(op) => {
            if *c == 0.0 && op == Op2::Mul {
                return Node::Const(0.0);
            }
            if *c == 0.0 && op == Op2::Div {
                return Node::Const(0.0);
            }
            if *c == 0.0 && op == Op2::Add {
                return r;
            }
            if *c == 1.0 && op == Op2::Mul {
                return r;
            }
        }
        (b, Node::Const(c)) if b.is_branch() && is_arith(op) => {
            if *c == 0.0 && op == Op2::Mul {
                return Node::Const(0.0);
            }
            if *c == 0.0 && op == Op2::Div {
                return Node::Const(f32::NAN);
            }
            if *c == 0.0 && op == Op2::Add {
                return l;
            }
            if *c == 1.0 && op == Op2::Mul {
                return l;
            }
        }
        _ => {}
    }
    // (b ^ c0) ^ c1 is b ^ (c0 * c1) (a power that is not cardinal)
    if op == Op2::Pow {
        if let (Some((Op2::Pow, b, c0)), Node::Const(c1)) = (as_boc(&l), &r) {
            return boc(Op2::Pow, b, c0 * *c1);
        }
    }
    raw(op, l, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(i: u32) -> Node {
        Node::Var(i)
    }

    fn c(x: f32) -> Node {
        Node::Const(x)
    }

    fn call(f: u32) -> Node {
        Node::Call { f, args: vec![] }
    }

    #[test]
    fn folds_constants() {
        assert_eq!(bin(Op2::Add, c(1.0), c(2.0)), c(3.0));
        assert_eq!(un(Op1::Sqrt, c(9.0)), c(3.0));
        assert_eq!(neg(neg(v(0))), v(0));
        assert_eq!(many(OpN::Max, vec![c(1.0), c(4.0)]), c(4.0));
        assert_eq!(cond(c(0.0), c(1.0), Some(c(2.0))), c(2.0));
    }

    #[test]
    fn identities_with_a_variable() {
        assert_eq!(bin(Op2::Mul, v(0), c(0.0)), c(0.0));
        assert!(matches!(bin(Op2::Div, v(0), c(0.0)), Node::Const(x) if x.is_nan()));
        assert_eq!(bin(Op2::Add, v(0), c(0.0)), v(0));
        assert_eq!(bin(Op2::Mul, c(1.0), v(0)), v(0));
        assert_eq!(bin(Op2::Div, c(0.0), v(0)), c(0.0));
    }

    #[test]
    fn cardinal_powers() {
        assert_eq!(bin(Op2::Pow, v(0), c(2.0)), Node::Bin(Op2::Mul, Box::new(v(0)), Box::new(v(0))));
        assert_eq!(bin(Op2::Pow, v(0), c(3.0)), Node::IPow { base: Box::new(v(0)), n: 3, inv: false });
        assert_eq!(bin(Op2::Pow, v(0), c(-2.0)), Node::IPow { base: Box::new(v(0)), n: 2, inv: true });
        assert_eq!(bin(Op2::Pow, v(0), c(1.0)), v(0));
        assert_eq!(bin(Op2::Pow, v(0), c(0.0)), c(1.0));
        assert!(matches!(bin(Op2::Pow, v(0), c(2.5)), Node::Bin(Op2::Pow, ..)));
        assert!(matches!(bin(Op2::Pow, v(0), c(61.0)), Node::Bin(Op2::Pow, ..)));
        assert_eq!(bin(Op2::Pow, call(0), c(4.0)), Node::IPow { base: Box::new(call(0)), n: 4, inv: false });
    }

    #[test]
    fn strength_reduction_of_three_leaves() {
        // (a / b) / c is a / (b * c)
        let inner = bin(Op2::Div, v(0), v(1));
        let got = bin(Op2::Div, inner, v(2));
        assert_eq!(got, div(v(0), mul(v(1), v(2))));
        // a / (b / c) is (a * c) / b
        let inner = bin(Op2::Div, v(1), v(2));
        assert_eq!(bin(Op2::Div, v(0), inner), div(mul(v(0), v(2)), v(1)));
        // (x + 1) + 2 is x + 3
        let inner = bin(Op2::Add, v(0), c(1.0));
        assert_eq!(bin(Op2::Add, inner, c(2.0)), raw(Op2::Add, v(0), c(3.0)));
        // 2 * (3 * x) is 6 * x
        let inner = bin(Op2::Mul, c(3.0), v(0));
        assert_eq!(bin(Op2::Mul, c(2.0), inner), cov(Op2::Mul, 6.0, 0));
    }

    #[test]
    fn constants_chain_through_a_branch() {
        // (f() + 1) + 2 is f() + 3
        let inner = bin(Op2::Add, call(0), c(1.0));
        assert_eq!(bin(Op2::Add, inner, c(2.0)), raw(Op2::Add, call(0), c(3.0)));
        // 2 * (3 * f()) is 6 * f()
        let inner = bin(Op2::Mul, c(3.0), call(0));
        assert_eq!(bin(Op2::Mul, c(2.0), inner), raw(Op2::Mul, c(6.0), call(0)));
        // f() * 0 drops the call
        assert_eq!(bin(Op2::Mul, call(0), c(0.0)), c(0.0));
    }
}
