//! `eval` allocates nothing (the allocator aborts on an allocation inside `assert_no_alloc`)

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_expr::{Expr, FnId, Host, Symbols};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

struct Counting(u32);

impl Host for Counting {
    fn call(&mut self, _f: FnId, _site: u32, args: &[f32]) -> f32 {
        self.0 += 1;
        args.first().copied().unwrap_or(0.5)
    }
}

#[test]
fn eval_does_not_allocate() {
    let mut s = Symbols::new();
    let t = s.add_var("t").unwrap();
    let f = s.add_var("f").unwrap();
    s.add_fn("rand", 0, true).unwrap();
    s.add_fn("integrate", 1, true).unwrap();
    let sources = [
        "t * f + sin(t) ^ 3",
        "min(t, f, 2) + max(1, t) - clamp(0, t, 1)",
        "t > f ? integrate(t) : rand() * 2",
        "rand(); integrate(f * 0.5); (t + f) / 3 % 2",
    ];
    let exprs: Vec<Expr> = sources.iter().map(|s2| Expr::compile(s2, &s).unwrap()).collect();
    let mut stack = vec![0.0_f32; exprs.iter().map(Expr::stack_len).max().unwrap_or(1)];
    let mut vars = vec![0.0_f32; 2];
    let mut host = Counting(0);
    assert_no_alloc(|| {
        for i in 0..1000 {
            vars[t.index()] = i as f32 * 0.01;
            vars[f.index()] = 1.0 - i as f32 * 0.003;
            for e in &exprs {
                let _ = e.eval(&mut stack, &vars, &mut host);
            }
        }
    });
    assert!(host.0 > 0);
}
