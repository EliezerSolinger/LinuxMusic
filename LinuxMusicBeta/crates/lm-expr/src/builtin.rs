//! The language's own names: the builtin functions of exprtk, its keywords and its reserved words

use crate::ops::{Op1, Op2, OpN};

/// What a builtin function name stands for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Builtin {
    One(Op1),
    Two(Op2),
    /// min, max, avg, sum, mul, mand, mor: one or more arguments
    Many(OpN),
    /// clamp, iclamp, inrange: exactly three
    Three(OpN),
    /// `if(c, a, b)` and `if(c, a)`
    If,
}

pub(crate) fn builtin(lower: &str) -> Option<Builtin> {
    use Builtin::{If, Many, One, Three, Two};
    Some(match lower {
        "abs" => One(Op1::Abs),
        "acos" | "arccos" => One(Op1::Acos),
        "acosh" | "arccosh" => One(Op1::Acosh),
        "asin" | "arcsin" => One(Op1::Asin),
        "asinh" | "arcsinh" => One(Op1::Asinh),
        "atan" | "arctan" => One(Op1::Atan),
        "atanh" | "arctanh" => One(Op1::Atanh),
        "ceil" => One(Op1::Ceil),
        "cos" => One(Op1::Cos),
        "cosh" => One(Op1::Cosh),
        "cot" => One(Op1::Cot),
        "csc" => One(Op1::Csc),
        "d2g" | "deg2grad" => One(Op1::D2g),
        "d2r" | "deg2rad" => One(Op1::D2r),
        "erf" => One(Op1::Erf),
        "erfc" => One(Op1::Erfc),
        "exp" => One(Op1::Exp),
        "expm1" => One(Op1::Expm1),
        "floor" => One(Op1::Floor),
        "frac" => One(Op1::Frac),
        "g2d" | "grad2deg" => One(Op1::G2d),
        "log" => One(Op1::Log),
        "log10" => One(Op1::Log10),
        "log1p" => One(Op1::Log1p),
        "log2" => One(Op1::Log2),
        "ncdf" => One(Op1::Ncdf),
        "not" | "notl" => One(Op1::Notl),
        "r2d" | "rad2deg" => One(Op1::R2d),
        "round" => One(Op1::Round),
        "sec" => One(Op1::Sec),
        "sgn" => One(Op1::Sgn),
        "sin" => One(Op1::Sin),
        "sinc" => One(Op1::Sinc),
        "sinh" => One(Op1::Sinh),
        "sqrt" => One(Op1::Sqrt),
        "tan" => One(Op1::Tan),
        "tanh" => One(Op1::Tanh),
        "trunc" => One(Op1::Trunc),
        "atan2" => Two(Op2::Atan2),
        "hypot" => Two(Op2::Hypot),
        "logn" => Two(Op2::Logn),
        "root" => Two(Op2::Root),
        "roundn" => Two(Op2::Roundn),
        "pow" => Two(Op2::Pow),
        "mod" => Two(Op2::Mod),
        "equal" => Two(Op2::Eq),
        "not_equal" => Two(Op2::Ne),
        "shl" => Two(Op2::Shl),
        "shr" => Two(Op2::Shr),
        "min" => Many(OpN::Min),
        "max" => Many(OpN::Max),
        "avg" => Many(OpN::Avg),
        "sum" => Many(OpN::Sum),
        "mul" => Many(OpN::Mul),
        "mand" => Many(OpN::Mand),
        "mor" => Many(OpN::Mor),
        "clamp" => Three(OpN::Clamp),
        "iclamp" => Three(OpN::Iclamp),
        "inrange" => Three(OpN::Inrange),
        "if" => If,
        _ => return None,
    })
}

/// The logic words that act as binary operators (the legacy disabled them; this crate keeps them)
pub(crate) fn logic_word(lower: &str) -> Option<Op2> {
    Some(match lower {
        "and" => Op2::And,
        "or" => Op2::Or,
        "nand" => Op2::Nand,
        "nor" => Op2::Nor,
        "xor" => Op2::Xor,
        "xnor" => Op2::Xnor,
        _ => return None,
    })
}

/// Words a host cannot use as names: builtins, operators, keywords and the literal words
pub(crate) fn is_reserved(lower: &str) -> bool {
    builtin(lower).is_some()
        || logic_word(lower).is_some()
        || matches!(
            lower,
            "true"
                | "false"
                | "else"
                | "in"
                | "like"
                | "ilike"
                | "var"
                | "const"
                | "for"
                | "while"
                | "repeat"
                | "until"
                | "switch"
                | "case"
                | "default"
                | "return"
                | "break"
                | "continue"
                | "null"
                | "swap"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert_eq!(builtin("sin"), Some(Builtin::One(Op1::Sin)));
        assert_eq!(builtin("clamp"), Some(Builtin::Three(OpN::Clamp)));
        assert_eq!(builtin("sinew"), None);
        assert!(is_reserved("and"));
        assert!(is_reserved("min"));
        assert!(!is_reserved("rand"));
    }
}
