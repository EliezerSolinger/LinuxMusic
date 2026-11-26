//! The numeric semantics of exprtk<float>: one function per operator and builtin function, as `exprtk.hpp`'s
//! `numeric::details` define them (the legacy Xpressive built exprtk with `float`). Pure and total: every function
//! returns a value (NaN or infinity included) and never panics

use core::f32::consts::{E, LN_2, PI, SQRT_2};

/// exprtk's float epsilon for `==` and `!=` (`epsilon_type<float>`)
const EPSILON: f32 = 0.000_001;

/// A unary operator or builtin function
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op1 {
    Neg,
    Pos,
    Abs,
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Ceil,
    Cos,
    Cosh,
    Cot,
    Csc,
    D2g,
    D2r,
    Erf,
    Erfc,
    Exp,
    Expm1,
    Floor,
    Frac,
    G2d,
    Log,
    Log10,
    Log1p,
    Log2,
    Ncdf,
    Notl,
    R2d,
    Round,
    Sec,
    Sgn,
    Sin,
    Sinc,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
    Trunc,
}

/// A binary operator or builtin function of two arguments
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op2 {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Lt,
    Lte,
    Gt,
    Gte,
    Eq,
    Ne,
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
    Atan2,
    Hypot,
    Logn,
    Root,
    Roundn,
    Shl,
    Shr,
}

/// A builtin function of any number of arguments (`clamp`, `iclamp` and `inrange` take exactly three)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpN {
    Min,
    Max,
    Sum,
    Avg,
    Mul,
    Clamp,
    Iclamp,
    Inrange,
    Mand,
    Mor,
}

#[inline]
fn bool_f(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

/// exprtk's truth: anything but zero (NaN is true)
#[inline]
pub fn is_true(v: f32) -> bool {
    v != 0.0
}

/// exprtk `numeric::equal` for float: equal within epsilon scaled by the larger magnitude, at least 1
fn equal(a: f32, b: f32) -> f32 {
    let scale = 1.0_f32.max(a.abs().max(b.abs())) * EPSILON;
    bool_f((a - b).abs() <= scale)
}

fn nequal(a: f32, b: f32) -> f32 {
    let scale = 1.0_f32.max(a.abs().max(b.abs())) * EPSILON;
    bool_f((a - b).abs() > scale)
}

/// The legacy's `pow10[]` of roundn: 1e0 to 1e16
fn pow10_table(index: usize) -> f32 {
    const TABLE: [f64; 17] =
        [1.0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16];
    TABLE.get(index).copied().unwrap_or(1e16) as f32
}

/// C++ `static_cast<long long>(v)` converted back to float (saturating, NaN to 0: the cast is undefined in C++ there)
#[inline]
fn via_i64(v: f32) -> f32 {
    (v as i64) as f32
}

pub fn eval1(op: Op1, v: f32) -> f32 {
    match op {
        Op1::Neg => -v,
        Op1::Pos => v,
        Op1::Abs => libm::fabsf(v),
        Op1::Acos => libm::acosf(v),
        Op1::Acosh => libm::acoshf(v),
        Op1::Asin => libm::asinf(v),
        Op1::Asinh => libm::asinhf(v),
        Op1::Atan => libm::atanf(v),
        Op1::Atanh => libm::atanhf(v),
        Op1::Ceil => libm::ceilf(v),
        Op1::Cos => libm::cosf(v),
        Op1::Cosh => libm::coshf(v),
        Op1::Cot => 1.0 / libm::tanf(v),
        Op1::Csc => 1.0 / libm::sinf(v),
        Op1::D2g => v * (10.0_f64 / 9.0) as f32,
        Op1::D2r => v * (core::f64::consts::PI / 180.0) as f32,
        Op1::Erf => libm::erff(v),
        Op1::Erfc => libm::erfcf(v),
        Op1::Exp => libm::expf(v),
        Op1::Expm1 => {
            if libm::fabsf(v) < 0.00001 {
                v + (0.5 * v * v)
            } else {
                libm::expf(v) - 1.0
            }
        }
        Op1::Floor => libm::floorf(v),
        Op1::Frac => v - via_i64(v),
        Op1::G2d => v * (9.0_f64 / 10.0) as f32,
        Op1::Log => libm::logf(v),
        Op1::Log10 => libm::log10f(v),
        Op1::Log1p => {
            if v > -1.0 {
                if libm::fabsf(v) > 0.0001 { libm::logf(1.0 + v) } else { (-0.5 * v + 1.0) * v }
            } else {
                f32::NAN
            }
        }
        Op1::Log2 => libm::logf(v) / LN_2,
        Op1::Ncdf => 0.5 * libm::erfcf(-(v / SQRT_2)),
        Op1::Notl => bool_f(!is_true(v)),
        Op1::R2d => v * (180.0_f64 / core::f64::consts::PI) as f32,
        Op1::Round => {
            if v < 0.0 {
                libm::ceilf(v - 0.5)
            } else {
                libm::floorf(v + 0.5)
            }
        }
        Op1::Sec => 1.0 / libm::cosf(v),
        Op1::Sgn => {
            if v > 0.0 {
                1.0
            } else if v < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        Op1::Sin => libm::sinf(v),
        Op1::Sinc => {
            if libm::fabsf(v) >= f32::EPSILON {
                libm::sinf(v) / v
            } else {
                1.0
            }
        }
        Op1::Sinh => libm::sinhf(v),
        Op1::Sqrt => libm::sqrtf(v),
        Op1::Tan => libm::tanf(v),
        Op1::Tanh => libm::tanhf(v),
        Op1::Trunc => via_i64(v),
    }
}

pub fn eval2(op: Op2, a: f32, b: f32) -> f32 {
    match op {
        Op2::Add => a + b,
        Op2::Sub => a - b,
        Op2::Mul => a * b,
        Op2::Div => a / b,
        Op2::Mod => libm::fmodf(a, b),
        Op2::Pow => libm::powf(a, b),
        Op2::Lt => bool_f(a < b),
        Op2::Lte => bool_f(a <= b),
        Op2::Gt => bool_f(a > b),
        Op2::Gte => bool_f(a >= b),
        Op2::Eq => equal(a, b),
        Op2::Ne => nequal(a, b),
        Op2::And => bool_f(is_true(a) && is_true(b)),
        Op2::Or => bool_f(is_true(a) || is_true(b)),
        Op2::Nand => bool_f(!(is_true(a) && is_true(b))),
        Op2::Nor => bool_f(!(is_true(a) || is_true(b))),
        Op2::Xor => bool_f(is_true(a) != is_true(b)),
        Op2::Xnor => bool_f(is_true(a) == is_true(b)),
        Op2::Atan2 => libm::atan2f(a, b),
        Op2::Hypot => libm::sqrtf((a * a) + (b * b)),
        Op2::Logn => libm::logf(a) / libm::logf(b),
        Op2::Root => {
            if b < 0.0 {
                return f32::NAN;
            }
            let n = b as usize;
            if a < 0.0 && n % 2 == 0 {
                return f32::NAN;
            }
            libm::powf(a, 1.0 / n as f32)
        }
        Op2::Roundn => {
            let index = (libm::floorf(b) as i32).clamp(0, 16) as usize;
            let p10 = pow10_table(index);
            if a < 0.0 { libm::ceilf((a * p10) - 0.5) / p10 } else { libm::floorf((a * p10) + 0.5) / p10 }
        }
        Op2::Shl => a * libm::powf(2.0, (b as i32) as f32),
        Op2::Shr => a * (1.0 / libm::powf(2.0, (b as i32) as f32)),
    }
}

/// The builtin of any arity over the evaluated arguments. An empty list gives NaN (the parser never builds one)
pub fn eval_n(op: OpN, args: &[f32]) -> f32 {
    let Some((&first, rest)) = args.split_first() else {
        return f32::NAN;
    };
    match op {
        OpN::Min => rest.iter().fold(first, |m, &v| if v < m { v } else { m }),
        OpN::Max => rest.iter().fold(first, |m, &v| if v > m { v } else { m }),
        OpN::Sum => rest.iter().fold(first, |s, &v| s + v),
        OpN::Mul => rest.iter().fold(first, |s, &v| s * v),
        OpN::Avg => rest.iter().fold(first, |s, &v| s + v) / args.len() as f32,
        OpN::Clamp | OpN::Iclamp | OpN::Inrange => {
            // (r0, v, r1)
            let (r0, v, r1) =
                (first, args.get(1).copied().unwrap_or(f32::NAN), args.get(2).copied().unwrap_or(f32::NAN));
            match op {
                OpN::Clamp => {
                    if v < r0 {
                        r0
                    } else if v > r1 {
                        r1
                    } else {
                        v
                    }
                }
                OpN::Iclamp => {
                    if v <= r0 || r1 <= v {
                        v
                    } else if libm::fabsf(v - r0) < libm::fabsf(v - r1) {
                        r0
                    } else {
                        r1
                    }
                }
                _ => bool_f(r0 <= v && v <= r1),
            }
        }
        OpN::Mand => bool_f(args.iter().all(|&v| is_true(v))),
        OpN::Mor => bool_f(args.iter().any(|&v| is_true(v))),
    }
}

/// exprtk's `fast_exp<float, N>`: the multiplications of an integer power, in the order of its specialisations
/// (1 to 10) and of the generic square-and-multiply loop (the others)
pub fn fast_exp(v: f32, n: u32) -> f32 {
    match n {
        0 => 1.0,
        1 => v,
        2 => v * v,
        3 => v * v * v,
        4 => {
            let v2 = v * v;
            v2 * v2
        }
        5 => fast_exp(v, 4) * v,
        6 => {
            let v3 = fast_exp(v, 3);
            v3 * v3
        }
        7 => fast_exp(v, 6) * v,
        8 => {
            let v4 = fast_exp(v, 4);
            v4 * v4
        }
        9 => fast_exp(v, 8) * v,
        10 => {
            let v5 = fast_exp(v, 5);
            v5 * v5
        }
        _ => {
            let (mut k, mut l, mut v) = (n, 1.0_f32, v);
            while k != 0 {
                if k % 2 == 1 {
                    l *= v;
                    k -= 1;
                }
                v *= v;
                k /= 2;
            }
            l
        }
    }
}

/// exprtk's constants as float
pub const CONST_PI: f32 = PI;
pub const CONST_E: f32 = E;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_uses_epsilon() {
        assert_eq!(eval2(Op2::Eq, 1.0, 1.0 + 1e-7), 1.0);
        assert_eq!(eval2(Op2::Eq, 1.0, 1.1), 0.0);
        assert_eq!(eval2(Op2::Ne, 1.0, 1.1), 1.0);
        assert_eq!(eval2(Op2::Eq, f32::NAN, f32::NAN), 0.0);
    }

    #[test]
    fn rounding_family() {
        assert_eq!(eval1(Op1::Round, 2.5), 3.0);
        assert_eq!(eval1(Op1::Round, -2.5), -3.0);
        assert_eq!(eval1(Op1::Trunc, -2.7), -2.0);
        assert_eq!(eval1(Op1::Frac, 2.75), 0.75);
        assert_eq!(eval2(Op2::Roundn, 3.14159, 2.0), 3.14);
    }

    #[test]
    fn logic_follows_exprtk_truth() {
        assert_eq!(eval2(Op2::And, 2.0, f32::NAN), 1.0);
        assert_eq!(eval2(Op2::Xor, 1.0, 0.0), 1.0);
        assert_eq!(eval1(Op1::Notl, 0.0), 1.0);
        assert_eq!(eval_n(OpN::Mand, &[1.0, 0.0]), 0.0);
        assert_eq!(eval_n(OpN::Mor, &[0.0, 0.0, 3.0]), 1.0);
    }

    #[test]
    fn variadic() {
        assert_eq!(eval_n(OpN::Min, &[3.0, 1.0, 2.0]), 1.0);
        assert_eq!(eval_n(OpN::Max, &[3.0, 1.0, 2.0]), 3.0);
        assert_eq!(eval_n(OpN::Sum, &[1.0, 2.0, 3.0]), 6.0);
        assert_eq!(eval_n(OpN::Avg, &[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(eval_n(OpN::Clamp, &[0.0, 5.0, 2.0]), 2.0);
        assert_eq!(eval_n(OpN::Inrange, &[0.0, 1.0, 2.0]), 1.0);
        assert!(eval_n(OpN::Min, &[]).is_nan());
    }

    #[test]
    fn fast_exp_matches_pow_for_exact_cases() {
        for n in 0..=20 {
            assert_eq!(fast_exp(2.0, n), 2.0_f32.powi(n as i32), "n = {n}");
        }
    }

    #[test]
    fn degenerate_inputs_do_not_panic() {
        for op in [Op2::Root, Op2::Roundn, Op2::Shl, Op2::Shr, Op2::Mod, Op2::Logn] {
            for a in [f32::NAN, f32::INFINITY, -f32::INFINITY, 0.0, -1.0] {
                for b in [f32::NAN, f32::INFINITY, -f32::INFINITY, 0.0, -1.0, 1e30] {
                    let _ = eval2(op, a, b);
                }
            }
        }
    }
}
