//! The parser: tokens to the tree, by the same precedence climbing as exprtk's `parse_expression` (so `-x ^ 2` is
//! `-(x ^ 2)`, `a ^ b ^ c` groups to the right, `c ? a : b` is the loosest), building every node through `opt`

use crate::ast::Node;
use crate::builtin::{Builtin, builtin, logic_word};
use crate::error::{CompileError, key};
use crate::lexer::{Tok, Token, tokenize};
use crate::ops::Op2;
use crate::opt;
use crate::symbols::{Lookup, Symbols};

/// How deep `parse_expression` may nest (exprtk's own stack limit is 400 calls of parse_expression and
/// parse_branch together)
const MAX_DEPTH: usize = 200;

/// The longest expression: a left-deep chain of this many terms is as deep as the tree gets, and the compiler and
/// the drop of the tree recurse on it
const MAX_TOKENS: usize = 8000;

/// Exprtk's precedence of a binary operator token: (left, right)
type Prec = (Op2, u8, u8);

struct Parser<'a> {
    toks: Vec<Token>,
    at: usize,
    symbols: &'a Symbols,
    depth: usize,
}

/// Parses `src` against `symbols`. An empty text is an error (nothing to evaluate)
pub(crate) fn parse(src: &str, symbols: &Symbols) -> Result<Node, CompileError> {
    let toks = tokenize(src)?;
    if toks.len() > MAX_TOKENS {
        return Err(CompileError::new(toks[MAX_TOKENS].pos, key::TOO_DEEP));
    }
    let mut p = Parser { toks, at: 0, symbols, depth: 0 };
    p.corpus()
}

impl Parser<'_> {
    fn tok(&self) -> &Tok {
        &self.toks[self.at.min(self.toks.len() - 1)].tok
    }

    fn pos(&self) -> usize {
        self.toks[self.at.min(self.toks.len() - 1)].pos
    }

    fn advance(&mut self) {
        if self.at < self.toks.len() - 1 {
            self.at += 1;
        }
    }

    /// An error at the current token: unsupported operators and the end of the text get their own keys
    fn unexpected(&self) -> CompileError {
        match self.tok() {
            Tok::Eof => CompileError::new(self.pos(), key::PREMATURE_END),
            Tok::Unsupported => CompileError::new(self.pos(), key::UNSUPPORTED),
            _ => CompileError::new(self.pos(), key::SYNTAX),
        }
    }

    /// `expr ; expr ; ...` (a trailing `;` is fine): every one is evaluated in order, the last is the value
    fn corpus(&mut self) -> Result<Node, CompileError> {
        if matches!(self.tok(), Tok::Eof) {
            return Err(CompileError::new(self.pos(), key::EMPTY));
        }
        let mut items = Vec::new();
        loop {
            while matches!(self.tok(), Tok::Semi) {
                self.advance();
            }
            if matches!(self.tok(), Tok::Eof) {
                break;
            }
            items.push(self.expression(0)?);
            match self.tok() {
                Tok::Semi => {}
                Tok::Eof => break,
                _ => return Err(self.unexpected()),
            }
        }
        if items.is_empty() {
            return Err(CompileError::new(self.pos(), key::EMPTY));
        }
        Ok(opt::seq(items))
    }

    fn binary(&self) -> Option<Prec> {
        Some(match self.tok() {
            Tok::Lt => (Op2::Lt, 5, 6),
            Tok::Le => (Op2::Lte, 5, 6),
            Tok::Gt => (Op2::Gt, 5, 6),
            Tok::Ge => (Op2::Gte, 5, 6),
            Tok::Eq => (Op2::Eq, 5, 6),
            Tok::Ne => (Op2::Ne, 5, 6),
            Tok::Add => (Op2::Add, 7, 8),
            Tok::Sub => (Op2::Sub, 7, 8),
            Tok::Mul => (Op2::Mul, 10, 11),
            Tok::Div => (Op2::Div, 10, 11),
            Tok::Mod => (Op2::Mod, 10, 11),
            Tok::Pow => (Op2::Pow, 12, 12),
            Tok::Amp => (Op2::And, 3, 4),
            Tok::Pipe => (Op2::Or, 1, 2),
            Tok::Sym(word) => {
                let op = logic_word(word)?;
                if matches!(op, Op2::And | Op2::Nand) { (op, 3, 4) } else { (op, 1, 2) }
            }
            _ => return None,
        })
    }

    fn expression(&mut self, precedence: u8) -> Result<Node, CompileError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(CompileError::new(self.pos(), key::TOO_DEEP));
        }
        let result = self.expression_inner(precedence);
        self.depth -= 1;
        result
    }

    fn expression_inner(&mut self, precedence: u8) -> Result<Node, CompileError> {
        let mut expr = self.branch(precedence)?;
        while let Some((op, left, right)) = self.binary() {
            if left < precedence {
                break;
            }
            self.advance();
            let rhs = self.expression(right)?;
            expr = opt::bin(op, expr, rhs);
            if precedence == 0 && matches!(self.tok(), Tok::Question) {
                expr = self.ternary(expr)?;
            }
        }
        Ok(expr)
    }

    fn branch(&mut self, precedence: u8) -> Result<Node, CompileError> {
        let mut node = match self.tok().clone() {
            Tok::Num(v) => {
                self.advance();
                Node::Const(v)
            }
            Tok::Sym(name) => self.symbol(&name)?,
            Tok::LParen => self.bracketed(&Tok::RParen)?,
            Tok::LSquare => self.bracketed(&Tok::RSquare)?,
            Tok::LCurly => self.bracketed(&Tok::RCurly)?,
            Tok::Sub => {
                self.advance();
                let operand = self.expression(11)?;
                opt::neg(operand)
            }
            Tok::Add => {
                self.advance();
                self.expression(13)?
            }
            _ => return Err(self.unexpected()),
        };
        if precedence == 0 && matches!(self.tok(), Tok::Question) {
            node = self.ternary(node)?;
        }
        Ok(node)
    }

    fn bracketed(&mut self, close: &Tok) -> Result<Node, CompileError> {
        self.advance();
        let inner = self.expression(0)?;
        if self.tok() != close {
            return Err(CompileError::new(self.pos(), key::BRACKET));
        }
        self.advance();
        Ok(inner)
    }

    fn ternary(&mut self, cond: Node) -> Result<Node, CompileError> {
        self.advance(); // the '?'
        let then = self.expression(0)?;
        if !matches!(self.tok(), Tok::Colon) {
            return Err(CompileError::new(self.pos(), key::TERNARY));
        }
        self.advance();
        let otherwise = self.expression(0)?;
        Ok(opt::cond(cond, then, Some(otherwise)))
    }

    fn symbol(&mut self, name: &str) -> Result<Node, CompileError> {
        let name_pos = self.pos();
        match name {
            "true" => {
                self.advance();
                return Ok(Node::Const(1.0));
            }
            "false" => {
                self.advance();
                return Ok(Node::Const(0.0));
            }
            _ => {}
        }
        match self.symbols.lookup(name) {
            Lookup::Var(id) => {
                self.advance();
                if matches!(self.tok(), Tok::LParen) {
                    return Err(CompileError::with_name(self.pos(), key::NOT_A_FUNCTION, name));
                }
                Ok(Node::Var(id.0))
            }
            Lookup::Const(v) => {
                self.advance();
                Ok(Node::Const(v))
            }
            Lookup::Fn(id) => {
                let arity = self.symbols.fn_def(id).map_or(0, |d| usize::from(d.arity));
                self.advance();
                let args = self.arguments(arity == 0)?;
                if args.len() != arity {
                    return Err(CompileError::with_name(name_pos, key::ARITY, name));
                }
                Ok(Node::Call { f: id.0, args })
            }
            Lookup::None => match builtin(name) {
                Some(b) => {
                    self.advance();
                    let args = self.arguments(false)?;
                    self.builtin_call(b, name, name_pos, args)
                }
                None => Err(CompileError::with_name(name_pos, key::UNKNOWN_SYMBOL, name)),
            },
        }
    }

    /// `( a, b, ... )`; the parentheses may be left out only when `optional` (a function of no arguments)
    fn arguments(&mut self, optional: bool) -> Result<Vec<Node>, CompileError> {
        if !matches!(self.tok(), Tok::LParen) {
            return if optional { Ok(Vec::new()) } else { Err(self.unexpected()) };
        }
        self.advance();
        let mut args = Vec::new();
        if matches!(self.tok(), Tok::RParen) {
            self.advance();
            return Ok(args);
        }
        loop {
            args.push(self.expression(0)?);
            match self.tok() {
                Tok::Comma => self.advance(),
                Tok::RParen => {
                    self.advance();
                    return Ok(args);
                }
                _ => return Err(CompileError::new(self.pos(), key::BRACKET)),
            }
        }
    }

    fn builtin_call(&mut self, b: Builtin, name: &str, pos: usize, mut args: Vec<Node>) -> Result<Node, CompileError> {
        let bad = || CompileError::with_name(pos, key::ARITY, name);
        match b {
            Builtin::One(op) => {
                if args.len() != 1 {
                    return Err(bad());
                }
                Ok(opt::un(op, args.remove(0)))
            }
            Builtin::Two(op) => {
                if args.len() != 2 {
                    return Err(bad());
                }
                let r = args.remove(1);
                let l = args.remove(0);
                Ok(opt::bin(op, l, r))
            }
            Builtin::Many(op) => {
                if args.is_empty() {
                    return Err(bad());
                }
                Ok(opt::many(op, args))
            }
            Builtin::Three(op) => {
                if args.len() != 3 {
                    return Err(bad());
                }
                Ok(opt::many(op, args))
            }
            Builtin::If => match args.len() {
                2 => {
                    let then = args.remove(1);
                    let c = args.remove(0);
                    Ok(opt::cond(c, then, None))
                }
                3 => {
                    let otherwise = args.remove(2);
                    let then = args.remove(1);
                    let c = args.remove(0);
                    Ok(opt::cond(c, then, Some(otherwise)))
                }
                _ => Err(bad()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Symbols {
        let mut s = Symbols::new();
        for name in ["t", "f", "a1"] {
            s.add_var(name).unwrap();
        }
        s.add_fn("rand", 0, true).unwrap();
        s.add_fn("integrate", 1, true).unwrap();
        s.add_fn("randsv", 2, false).unwrap();
        s
    }

    fn value(src: &str) -> f32 {
        match parse(src, &table()).unwrap() {
            Node::Const(v) => v,
            other => panic!("{src}: not a constant: {other:?}"),
        }
    }

    fn err(src: &str) -> CompileError {
        parse(src, &table()).unwrap_err()
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(value("1 + 2 * 3"), 7.0);
        assert_eq!(value("(1 + 2) * 3"), 9.0);
        assert_eq!(value("2 ^ 3 ^ 2"), 512.0);
        assert_eq!(value("-2 ^ 2"), -4.0);
        assert_eq!(value("2 ^ -1"), 0.5);
        assert_eq!(value("10 - 4 - 3"), 3.0);
        assert_eq!(value("100 / 10 / 5"), 2.0);
        assert_eq!(value("7 % 4"), 3.0);
        assert_eq!(value("2 * 3 + 4 * 5"), 26.0);
        assert_eq!(value("+3"), 3.0);
        assert_eq!(value("--3"), 3.0);
        assert_eq!(value("1 - -1"), 2.0);
    }

    #[test]
    fn comparison_and_logic() {
        assert_eq!(value("1 < 2"), 1.0);
        assert_eq!(value("2 <= 1"), 0.0);
        assert_eq!(value("1 + 1 == 2"), 1.0);
        assert_eq!(value("1 < 2 and 2 < 3"), 1.0);
        assert_eq!(value("0 or 0"), 0.0);
        assert_eq!(value("1 xor 1"), 0.0);
        assert_eq!(value("1 & 0 | 1"), 1.0);
    }

    #[test]
    fn ternary_and_if() {
        assert_eq!(value("1 ? 2 : 3"), 2.0);
        assert_eq!(value("0 ? 2 : 3"), 3.0);
        assert_eq!(value("0 ? 1 : 0 ? 2 : 3"), 3.0);
        assert_eq!(value("1 + 1 > 1 ? 10 : 20"), 10.0);
        assert_eq!(value("if(1, 5, 6)"), 5.0);
        assert!(value("if(0, 5)").is_nan());
    }

    #[test]
    fn constants_and_functions() {
        assert_eq!(value("pi"), core::f32::consts::PI);
        assert_eq!(value("E"), core::f32::consts::E);
        assert_eq!(value("abs(-3)"), 3.0);
        assert_eq!(value("min(3, 1, 2)"), 1.0);
        assert_eq!(value("max(3, 1, 2)"), 3.0);
        assert_eq!(value("clamp(0, 5, 2)"), 2.0);
        assert_eq!(value("floor(2.7) + ceil(2.2)"), 5.0);
        assert_eq!(value("pow(2, 10)"), 1024.0);
        assert_eq!(value("sqrt(16)"), 4.0);
        assert_eq!(value("SIN(0)"), 0.0);
        assert_eq!(value("true + true"), 2.0);
        assert_eq!(value("2pi"), 2.0 * core::f32::consts::PI);
    }

    #[test]
    fn sequences() {
        assert_eq!(value("1; 2; 3"), 3.0);
        assert_eq!(value("4;"), 4.0);
    }

    #[test]
    fn host_calls_keep_their_arguments() {
        let node = parse("integrate(f) + rand()", &table()).unwrap();
        assert!(matches!(node, Node::Bin(Op2::Add, ..)));
        let node = parse("rand", &table()).unwrap();
        assert_eq!(node, Node::Call { f: 0, args: vec![] });
        let node = parse("randsv(t, 3)", &table()).unwrap();
        assert!(matches!(node, Node::Call { f: 2, ref args } if args.len() == 2));
    }

    #[test]
    fn variables_are_case_insensitive() {
        assert_eq!(parse("A1", &table()).unwrap(), Node::Var(2));
        assert_eq!(parse("T", &table()).unwrap(), Node::Var(0));
    }

    #[test]
    fn errors_carry_positions_and_keys() {
        let e = err("1 +");
        assert_eq!((e.position, e.label.key), (3, key::PREMATURE_END));
        let e = err("1 + foo");
        assert_eq!((e.position, e.label.key), (4, key::UNKNOWN_SYMBOL));
        let e = err("(1 + 2");
        assert_eq!((e.position, e.label.key), (6, key::BRACKET));
        let e = err("sin(1, 2)");
        assert_eq!((e.position, e.label.key), (0, key::ARITY));
        let e = err("1 2");
        assert_eq!((e.position, e.label.key), (2, key::SYNTAX));
        let e = err("");
        assert_eq!(e.label.key, key::EMPTY);
        let e = err("1 ? 2");
        assert_eq!(e.label.key, key::TERNARY);
        let e = err("t := 3");
        assert_eq!((e.position, e.label.key), (2, key::UNSUPPORTED));
        let e = err("t(1)");
        assert_eq!(e.label.key, key::NOT_A_FUNCTION);
        let e = err("integrate()");
        assert_eq!(e.label.key, key::ARITY);
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let src = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
        assert_eq!(err(&src).label.key, key::TOO_DEEP);
        let src = "-".repeat(1) + &"+".repeat(1);
        let _ = parse(&src, &table());
    }
}
