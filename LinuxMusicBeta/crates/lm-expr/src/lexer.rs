//! The lexer: text to tokens, with the two token passes exprtk runs before parsing (the operator joiner: `+ -` is
//! `-`, `- -` is `+`; and the commutative inserter: `2x`, `2 pi` and `x y` are products) and exprtk's own number
//! conversion (`string_to_real`, in float, so the same literal gives the same bits)

use crate::builtin;
use crate::error::{CompileError, key};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok {
    Num(f32),
    /// A name, lowercased
    Sym(String),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    /// `&`, the logic and
    Amp,
    /// `|`, the logic or
    Pipe,
    LParen,
    RParen,
    LSquare,
    RSquare,
    LCurly,
    RCurly,
    Comma,
    Semi,
    Question,
    Colon,
    /// An operator of exprtk this crate does not have (assignment, swap, shifts, `!`)
    Unsupported,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Token {
    pub tok: Tok,
    pub pos: usize,
}

/// exprtk's `fract10`: 10^e as the nearest double (parsed, so it is exactly the literal's value)
fn fract10(e: i32) -> f64 {
    format!("1e{e}").parse::<f64>().unwrap_or(f64::INFINITY)
}

/// exprtk `compute_pow10<float>`: scales in double, rounds once to float
fn compute_pow10(d: f32, exponent: i32) -> f32 {
    const MIN_EXPONENT10: i32 = -37; // numeric_limits<float>::min_exponent10
    let e = exponent.abs();
    if exponent >= MIN_EXPONENT10 {
        if e < 309 {
            if exponent > 0 { (f64::from(d) * fract10(e)) as f32 } else { (f64::from(d) / fract10(e)) as f32 }
        } else {
            (f64::from(d) * libm::pow(10.0, 10.0 * f64::from(exponent))) as f32
        }
    } else {
        let d = d / fract10(-MIN_EXPONENT10) as f32;
        (f64::from(d) / fract10(-exponent + MIN_EXPONENT10)) as f32
    }
}

fn valid_exponent(e: i32) -> bool {
    (-38..=38).contains(&e)
}

/// exprtk `string_to_real<float>` for the text of a number token (digits, an optional point and digits, an
/// optional exponent): the digits accumulate in float, the scale applies once. `None` where exprtk fails
pub(crate) fn string_to_real(text: &str) -> Option<f32> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut d = 0.0_f32;
    let mut instate = false;
    // Integer part (leading zeros skipped)
    if b.first() != Some(&b'.') {
        let start = i;
        while b.get(i) == Some(&b'0') {
            i += 1;
        }
        while let Some(&c) = b.get(i) {
            if c.is_ascii_digit() {
                d = d * 10.0 + f32::from(c - b'0');
                i += 1;
            } else {
                break;
            }
        }
        if i != start {
            instate = true;
        }
    }
    let mut exponent = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        let mut tmp = 0.0_f32;
        while let Some(&c) = b.get(i) {
            if c.is_ascii_digit() {
                tmp = tmp * 10.0 + f32::from(c - b'0');
                i += 1;
            } else {
                break;
            }
        }
        if i != start {
            instate = true;
            let frac_exponent = -((i - start) as i32);
            if !valid_exponent(frac_exponent) {
                return None;
            }
            d += compute_pow10(tmp, frac_exponent);
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        let negative = b.get(i) == Some(&b'-');
        if negative || b.get(i) == Some(&b'+') {
            i += 1;
        }
        while b.get(i) == Some(&b'0') {
            i += 1;
        }
        let start = i;
        let mut e = 0_i32;
        while let Some(&c) = b.get(i) {
            if c.is_ascii_digit() {
                e = e.saturating_mul(10).saturating_add(i32::from(c - b'0'));
                i += 1;
            } else {
                break;
            }
        }
        // exprtk's integer reader takes at most four digits
        if i - start > 4 {
            return None;
        }
        exponent += if negative { -e } else { e };
    }
    if i != b.len() || !instate || !valid_exponent(exponent) {
        return None;
    }
    if exponent != 0 {
        d = compute_pow10(d, exponent);
    }
    Some(d)
}

/// The token and its length in bytes for an operator character
fn operator(b: &[u8], i: usize) -> Option<(Tok, usize)> {
    let next = b.get(i + 1).copied();
    let assign = next == Some(b'=');
    Some(match b[i] {
        b'+' => (if assign { Tok::Unsupported } else { Tok::Add }, if assign { 2 } else { 1 }),
        b'-' => (if assign { Tok::Unsupported } else { Tok::Sub }, if assign { 2 } else { 1 }),
        b'*' => (if assign { Tok::Unsupported } else { Tok::Mul }, if assign { 2 } else { 1 }),
        b'/' => (if assign { Tok::Unsupported } else { Tok::Div }, if assign { 2 } else { 1 }),
        b'%' => (if assign { Tok::Unsupported } else { Tok::Mod }, if assign { 2 } else { 1 }),
        b'^' => (Tok::Pow, 1),
        b'<' => match (next, b.get(i + 2).copied()) {
            (Some(b'='), Some(b'>')) => (Tok::Unsupported, 3),
            (Some(b'='), _) => (Tok::Le, 2),
            (Some(b'>'), _) => (Tok::Ne, 2),
            (Some(b'<'), _) => (Tok::Unsupported, 2),
            _ => (Tok::Lt, 1),
        },
        b'>' => match next {
            Some(b'=') => (Tok::Ge, 2),
            Some(b'>') => (Tok::Unsupported, 2),
            _ => (Tok::Gt, 1),
        },
        b'=' => (Tok::Eq, if assign { 2 } else { 1 }),
        b'!' => (if assign { Tok::Ne } else { Tok::Unsupported }, if assign { 2 } else { 1 }),
        b':' => (if assign { Tok::Unsupported } else { Tok::Colon }, if assign { 2 } else { 1 }),
        b'&' => (Tok::Amp, 1),
        b'|' => (Tok::Pipe, 1),
        b'(' => (Tok::LParen, 1),
        b')' => (Tok::RParen, 1),
        b'[' => (Tok::LSquare, 1),
        b']' => (Tok::RSquare, 1),
        b'{' => (Tok::LCurly, 1),
        b'}' => (Tok::RCurly, 1),
        b',' => (Tok::Comma, 1),
        b';' => (Tok::Semi, 1),
        b'?' => (Tok::Question, 1),
        _ => return None,
    })
}

/// The end of the number token that starts at `start`: digits, a point and digits, an exponent that has digits
fn number_end(b: &[u8], start: usize) -> usize {
    let digits = |mut j: usize| {
        while b.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        j
    };
    let mut j = digits(start);
    if b.get(j) == Some(&b'.') {
        j = digits(j + 1);
    }
    if matches!(b.get(j), Some(b'e' | b'E')) {
        let mut k = j + 1;
        if matches!(b.get(k), Some(b'+' | b'-')) {
            k += 1;
        }
        if b.get(k).is_some_and(u8::is_ascii_digit) {
            j = digits(k);
        }
    }
    j
}

fn scan(src: &str) -> Result<Vec<Token>, CompileError> {
    let b = src.as_bytes();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let pos = i;
        if matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c) {
            i += 1;
            continue;
        }
        let (tok, len) = if c.is_ascii_digit() || c == b'.' {
            let end = number_end(b, i);
            // "1.2.3", or a lone point
            if b.get(end) == Some(&b'.') || (end == i + 1 && c == b'.') {
                return Err(CompileError::new(pos, key::BAD_NUMBER));
            }
            let value = string_to_real(&src[i..end]).ok_or_else(|| CompileError::new(pos, key::BAD_NUMBER))?;
            (Tok::Num(value), end - i)
        } else if c.is_ascii_alphabetic() {
            let mut j = i + 1;
            while b.get(j).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                j += 1;
            }
            (Tok::Sym(src[i..j].to_ascii_lowercase()), j - i)
        } else {
            operator(b, i).ok_or_else(|| CompileError::new(pos, key::BAD_CHARACTER))?
        };
        out.push(Token { tok, pos });
        i += len;
    }
    out.push(Token { tok: Tok::Eof, pos: b.len() });
    Ok(out)
}

/// exprtk's operator joiner: `+ -` and `- +` become `-`, `- -` becomes `+`; repeated until nothing changes
fn join(mut tokens: Vec<Token>) -> Vec<Token> {
    loop {
        let mut changed = false;
        let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            let merged = match (tokens.get(i).map(|t| &t.tok), tokens.get(i + 1).map(|t| &t.tok)) {
                (Some(Tok::Add), Some(Tok::Sub)) | (Some(Tok::Sub), Some(Tok::Add)) => Some(Tok::Sub),
                (Some(Tok::Sub), Some(Tok::Sub)) => Some(Tok::Add),
                _ => None,
            };
            match merged {
                Some(tok) => {
                    out.push(Token { tok, pos: tokens[i].pos });
                    changed = true;
                    i += 2;
                }
                None => {
                    out.push(tokens[i].clone());
                    i += 1;
                }
            }
        }
        tokens = out;
        if !changed {
            return tokens;
        }
    }
}

fn ignored_symbol(name: &str) -> bool {
    builtin::logic_word(name).is_some() || matches!(name, "if" | "else" | "in" | "like" | "ilike")
}

fn is_close(k: &Tok) -> bool {
    matches!(k, Tok::RParen | Tok::RSquare | Tok::RCurly)
}

fn is_open(k: &Tok) -> bool {
    matches!(k, Tok::LParen | Tok::LSquare | Tok::LCurly)
}

fn is_num(k: &Tok) -> bool {
    matches!(k, Tok::Num(_))
}

fn is_sym(k: &Tok) -> bool {
    matches!(k, Tok::Sym(_))
}

/// exprtk's commutative inserter: a `*` between number and name, number and open bracket, name and number, close
/// bracket and number or name, name and name
fn insert_products(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len() + 4);
    for t in tokens {
        if let Some(prev) = out.last() {
            let (a, b) = (&prev.tok, &t.tok);
            let skip = matches!(a, Tok::Sym(s) if ignored_symbol(s)) || matches!(b, Tok::Sym(s) if ignored_symbol(s));
            let matched = !skip
                && ((is_num(a) && (is_sym(b) || is_open(b)))
                    || (is_sym(a) && (is_num(b) || is_sym(b)))
                    || (is_close(a) && (is_num(b) || is_sym(b))));
            if matched {
                out.push(Token { tok: Tok::Mul, pos: t.pos });
            }
        }
        out.push(t);
    }
    out
}

/// The tokens of `src`, ending with `Eof`, after the joiner and the inserter
pub(crate) fn tokenize(src: &str) -> Result<Vec<Token>, CompileError> {
    Ok(insert_products(join(scan(src)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        tokenize(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn numbers_convert_like_exprtk() {
        assert_eq!(string_to_real("0"), Some(0.0));
        assert_eq!(string_to_real("12"), Some(12.0));
        assert_eq!(string_to_real(".5"), Some(0.5));
        assert_eq!(string_to_real("1."), Some(1.0));
        assert_eq!(string_to_real("1e3"), Some(1000.0));
        assert_eq!(string_to_real("2.5e-1"), Some(0.25));
        assert_eq!(string_to_real("0.1"), Some(0.1));
        assert_eq!(string_to_real("1e39"), None);
        assert_eq!(string_to_real("1e-39"), None);
        assert_eq!(string_to_real("."), None);
    }

    #[test]
    fn operators_join() {
        assert_eq!(toks("a - -b"), vec![Tok::Sym("a".into()), Tok::Add, Tok::Sym("b".into()), Tok::Eof]);
        assert_eq!(toks("a + -b"), vec![Tok::Sym("a".into()), Tok::Sub, Tok::Sym("b".into()), Tok::Eof]);
    }

    #[test]
    fn products_are_inserted() {
        assert_eq!(toks("2x"), vec![Tok::Num(2.0), Tok::Mul, Tok::Sym("x".into()), Tok::Eof]);
        assert_eq!(
            toks("2(x)"),
            vec![Tok::Num(2.0), Tok::Mul, Tok::LParen, Tok::Sym("x".into()), Tok::RParen, Tok::Eof]
        );
        assert_eq!(toks("x and y"), vec![Tok::Sym("x".into()), Tok::Sym("and".into()), Tok::Sym("y".into()), Tok::Eof]);
    }

    #[test]
    fn bad_input_has_positions() {
        assert_eq!(tokenize("1 + $").unwrap_err().position, 4);
        assert_eq!(tokenize("1.2.3").unwrap_err().position, 0);
        assert_eq!(tokenize("1e40").unwrap_err().position, 0);
    }

    #[test]
    fn two_char_operators() {
        assert_eq!(toks("a<=b")[1], Tok::Le);
        assert_eq!(toks("a<>b")[1], Tok::Ne);
        assert_eq!(toks("a!=b")[1], Tok::Ne);
        assert_eq!(toks("a==b")[1], Tok::Eq);
        assert_eq!(toks("a=b")[1], Tok::Eq);
        assert_eq!(toks("a:=b")[1], Tok::Unsupported);
    }
}
