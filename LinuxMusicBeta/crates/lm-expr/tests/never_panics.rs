//! Whatever the text and the values, compile returns a result and eval returns a number: no panic

use lm_expr::{Expr, FnId, Host, Symbols};

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }
}

struct Wild(Lcg);

impl Host for Wild {
    fn call(&mut self, _f: FnId, _site: u32, args: &[f32]) -> f32 {
        const VALUES: [f32; 6] = [f32::NAN, f32::INFINITY, -f32::INFINITY, 0.0, -1.0, 1e30];
        let r = self.0.below(8);
        VALUES.get(r).copied().unwrap_or(args.first().copied().unwrap_or(0.25))
    }
}

fn table() -> Symbols {
    let mut s = Symbols::new();
    for n in ["t", "f", "key", "rel", "a1"] {
        s.add_var(n).unwrap();
    }
    s.add_fn("rand", 0, true).unwrap();
    s.add_fn("integrate", 1, true).unwrap();
    s.add_fn("randsv", 2, false).unwrap();
    s
}

const ATOMS: [&str; 12] = ["t", "f", "key", "a1", "0", "1", "2", "0.5", "pi", "1e3", "rand()", "integrate(t)"];
const BINARY: [&str; 14] = ["+", "-", "*", "/", "%", "^", "<", "<=", "==", "!=", " and ", " or ", ">", ">="];
const FUNCS: [&str; 16] = [
    "sin", "cos", "abs", "sqrt", "log", "exp", "floor", "ceil", "round", "sgn", "frac", "tan", "acos", "erf", "ncdf",
    "trunc",
];

fn generate(rng: &mut Lcg, depth: usize) -> String {
    if depth == 0 || rng.below(4) == 0 {
        return ATOMS[rng.below(ATOMS.len())].to_owned();
    }
    match rng.below(8) {
        0 | 1 | 2 => {
            format!("({}{}{})", generate(rng, depth - 1), BINARY[rng.below(BINARY.len())], generate(rng, depth - 1))
        }
        3 => format!("{}({})", FUNCS[rng.below(FUNCS.len())], generate(rng, depth - 1)),
        4 => format!("({} ? {} : {})", generate(rng, depth - 1), generate(rng, depth - 1), generate(rng, depth - 1)),
        5 => format!("min({}, {}, {})", generate(rng, depth - 1), generate(rng, depth - 1), generate(rng, depth - 1)),
        6 => format!("-{}", generate(rng, depth - 1)),
        _ => format!("randsv({}, {})", generate(rng, depth - 1), generate(rng, depth - 1)),
    }
}

#[test]
fn random_expressions_compile_and_evaluate() {
    let s = table();
    let mut rng = Lcg(7);
    let weird = [f32::NAN, f32::INFINITY, -f32::INFINITY, 0.0, -0.0, 1.0, -1.0, 1e30, -1e30, 1e-30, 0.5];
    let mut compiled = 0;
    for _ in 0..3000 {
        let text = generate(&mut rng, 5);
        let Ok(expr) = Expr::compile(&text, &s) else { continue };
        compiled += 1;
        let mut stack = vec![0.0_f32; expr.stack_len()];
        let mut host = Wild(Lcg(rng.next()));
        for _ in 0..8 {
            let vars: Vec<f32> = (0..5).map(|_| weird[rng.below(weird.len())]).collect();
            let _ = expr.eval(&mut stack, &vars, &mut host);
        }
    }
    assert!(compiled > 2000, "the generator makes valid text: {compiled}");
}

#[test]
fn random_bytes_are_an_error_or_an_expression() {
    let s = table();
    let mut rng = Lcg(99);
    const ALPHABET: &[u8] = b"tf1 2.e+-*/%^()[]{}<>=!&|?:;,abcxyz_ \t$#\"'\\0";
    for _ in 0..5000 {
        let len = rng.below(24);
        let bytes: Vec<u8> = (0..len).map(|_| ALPHABET[rng.below(ALPHABET.len())]).collect();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if let Ok(expr) = Expr::compile(&text, &s) {
            let mut stack = vec![0.0_f32; expr.stack_len()];
            let _ = expr.eval(&mut stack, &[1.0, 2.0, 3.0, 4.0, 5.0], &mut Wild(Lcg(1)));
        }
    }
}

#[test]
fn pathological_text() {
    let s = table();
    for text in [
        "".to_owned(),
        " ".to_owned(),
        "(".repeat(100_000),
        ")".repeat(100_000),
        "-".repeat(100_000),
        "1+".repeat(100_000),
        "t".repeat(100_000),
        "é€😀".to_owned(),
        "1e999999999999".to_owned(),
        "0.".repeat(10_000),
        format!("{}t{}", "sin(".repeat(500), ")".repeat(500)),
        format!("{}1", "t?".repeat(300)),
    ] {
        let _ = Expr::compile(&text, &s);
    }
}
