//! The virtual machine: evaluates a compiled [`Expr`] over a stack the caller provides. No allocation, no panic,
//! whatever the values (NaN and infinity flow through as in exprtk); calls to host functions go through [`Host`]

use crate::compile::{Expr, Ins};
use crate::ops::{eval_n, eval1, eval2, fast_exp, is_true};
use crate::symbols::FnId;

/// The functions the host registered in the [`crate::Symbols`], called during evaluation. The calls of one
/// evaluation come in exprtk's left-to-right order, and `site` numbers the calls of each function in that order
/// (the `n`th `integrate` of the text is site `n`), so per-site state (integrate's counters) is stable
pub trait Host {
    /// `f` is the id `Symbols::add_fn` returned; `args` holds as many values as the function's arity
    fn call(&mut self, f: FnId, site: u32, args: &[f32]) -> f32;
}

/// A host with no functions: every call gives 0
#[derive(Clone, Copy, Debug, Default)]
pub struct NoHost;

impl Host for NoHost {
    fn call(&mut self, _f: FnId, _site: u32, _args: &[f32]) -> f32 {
        0.0
    }
}

#[inline]
fn at(stack: &[f32], i: usize) -> f32 {
    stack.get(i).copied().unwrap_or(0.0)
}

#[inline]
fn put(stack: &mut [f32], i: usize, v: f32) {
    if let Some(slot) = stack.get_mut(i) {
        *slot = v;
    }
}

impl Expr {
    /// Evaluates the expression. `stack` must hold [`Expr::stack_len`] values (a shorter slice gives 0: there is
    /// nothing to evaluate on), `vars` is indexed by `VarId` (a variable beyond the slice reads 0). Never panics and
    /// never allocates; the host's calls are the only side effects
    pub fn eval<H: Host + ?Sized>(&self, stack: &mut [f32], vars: &[f32], host: &mut H) -> f32 {
        if stack.len() < self.stack_len() {
            return 0.0;
        }
        let mut sp = 0_usize;
        let mut pc = 0_usize;
        while let Some(ins) = self.code.get(pc) {
            pc += 1;
            match *ins {
                Ins::Const(v) => {
                    put(stack, sp, v);
                    sp += 1;
                }
                Ins::Var(i) => {
                    put(stack, sp, vars.get(i as usize).copied().unwrap_or(0.0));
                    sp += 1;
                }
                Ins::Un(op) => {
                    let i = sp.saturating_sub(1);
                    put(stack, i, eval1(op, at(stack, i)));
                }
                Ins::Bin(op) => {
                    let b = at(stack, sp.saturating_sub(1));
                    let i = sp.saturating_sub(2);
                    put(stack, i, eval2(op, at(stack, i), b));
                    sp = i + 1;
                }
                Ins::Many(op, argc) => {
                    let start = sp.saturating_sub(usize::from(argc));
                    let v = eval_n(op, stack.get(start..sp).unwrap_or(&[]));
                    put(stack, start, v);
                    sp = start + 1;
                }
                Ins::IPow { n, inv } => {
                    let i = sp.saturating_sub(1);
                    let p = fast_exp(at(stack, i), n);
                    put(stack, i, if inv { 1.0 / p } else { p });
                }
                Ins::Call { f, site, argc } => {
                    let start = sp.saturating_sub(usize::from(argc));
                    let r = host.call(FnId(f), site, stack.get(start..sp).unwrap_or(&[]));
                    put(stack, start, r);
                    sp = start + 1;
                }
                Ins::JumpIfFalse(target) => {
                    sp = sp.saturating_sub(1);
                    if !is_true(at(stack, sp)) {
                        pc = target as usize;
                    }
                }
                Ins::Jump(target) => pc = target as usize,
                Ins::Pop => sp = sp.saturating_sub(1),
                Ins::PushNan => {
                    put(stack, sp, f32::NAN);
                    sp += 1;
                }
            }
        }
        at(stack, sp.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Symbols;

    /// Records the calls; `rand` counts up, `twice` doubles, `sum3` adds its three arguments
    #[derive(Default)]
    struct Recorder {
        calls: Vec<(u32, u32, Vec<f32>)>,
        counter: f32,
    }

    impl Host for Recorder {
        fn call(&mut self, f: FnId, site: u32, args: &[f32]) -> f32 {
            self.calls.push((f.0, site, args.to_vec()));
            match f.0 {
                0 => {
                    self.counter += 1.0;
                    self.counter
                }
                1 => args[0] * 2.0,
                _ => args.iter().sum(),
            }
        }
    }

    fn table() -> Symbols {
        let mut s = Symbols::new();
        s.add_var("t").unwrap();
        s.add_var("f").unwrap();
        s.add_fn("rand", 0, true).unwrap();
        s.add_fn("twice", 1, false).unwrap();
        s.add_fn("sum3", 3, false).unwrap();
        s
    }

    fn run_with(src: &str, vars: &[f32], host: &mut Recorder) -> f32 {
        let e = Expr::compile(src, &table()).unwrap_or_else(|e| panic!("{src}: {e}"));
        let mut stack = vec![0.0; e.stack_len()];
        e.eval(&mut stack, vars, host)
    }

    fn run(src: &str, vars: &[f32]) -> f32 {
        run_with(src, vars, &mut Recorder::default())
    }

    #[test]
    fn arithmetic_on_variables() {
        let v = [2.0, 3.0];
        assert_eq!(run("t * f + 1", &v), 7.0);
        assert_eq!(run("t - f", &v), -1.0);
        assert_eq!(run("f % t", &v), 1.0);
        assert_eq!(run("-t", &v), -2.0);
        assert_eq!(run("t ^ f", &v), 8.0);
        assert_eq!(run("2 * (t + f)", &v), 10.0);
        assert_eq!(run("1 / t", &v), 0.5);
        assert_eq!(run("t f", &v), 6.0);
        assert_eq!(run("2t", &v), 4.0);
    }

    #[test]
    fn powers_use_the_cardinal_form() {
        let v = [2.0, 3.0];
        assert_eq!(run("t ^ 3", &v), 8.0);
        assert_eq!(run("t ^ 2", &v), 4.0);
        assert_eq!(run("t ^ -1", &v), 0.5);
        assert_eq!(run("t ^ 0", &v), 1.0);
        assert_eq!(run("t ^ 0.5", &v), 2.0_f32.sqrt());
        assert_eq!(run("(t + f) ^ 2", &v), 25.0);
        assert_eq!(run("(t + f) ^ -2", &v), 0.04);
        assert_eq!(run("t ^ 11", &v), 2048.0);
    }

    #[test]
    fn exprtk_rewrites_show_in_the_values() {
        // x / 0 is NaN, x * 0 is 0 even for an infinity
        assert!(run("t / 0", &[2.0, 0.0]).is_nan());
        assert_eq!(run("t * 0", &[f32::INFINITY, 0.0]), 0.0);
        // (a / b) / 3 is a / (b * 3): not (a / b) / 3 in float
        let (a, b) = (1.0_f32, 7.0_f32);
        assert_eq!(run("t / f / 3", &[a, b]), a / (b * 3.0));
        // 2 * (3 * t) is 6 * t
        assert_eq!(run("2 * (3 * t)", &[1.1, 0.0]), 6.0 * 1.1_f32);
        // t + 0.1 + 0.2 is t + (0.1 + 0.2)
        assert_eq!(run("t + 0.1 + 0.2", &[1.0, 0.0]), 1.0_f32 + (0.1_f32 + 0.2_f32));
    }

    #[test]
    fn comparison_and_logic() {
        let v = [2.0, 3.0];
        assert_eq!(run("t < f", &v), 1.0);
        assert_eq!(run("t > f", &v), 0.0);
        assert_eq!(run("t == 2", &v), 1.0);
        assert_eq!(run("t != 2", &v), 0.0);
        assert_eq!(run("t < f and f < 4", &v), 1.0);
        assert_eq!(run("t > f or f > 4", &v), 0.0);
        assert_eq!(run("not(t)", &v), 0.0);
    }

    #[test]
    fn ternary_evaluates_one_branch() {
        let mut h = Recorder::default();
        assert_eq!(run_with("t > 5 ? rand() : 7", &[2.0, 0.0], &mut h), 7.0);
        assert!(h.calls.is_empty());
        assert_eq!(run_with("t > 1 ? rand() : 7", &[2.0, 0.0], &mut h), 1.0);
        assert_eq!(h.calls.len(), 1);
        assert!(run("if(t, f)", &[0.0, 5.0]).is_nan());
        assert_eq!(run("if(t, f, 9)", &[0.0, 5.0]), 9.0);
    }

    #[test]
    fn calls_are_left_to_right_with_sites() {
        let mut h = Recorder::default();
        let r = run_with("twice(t) + twice(f) * rand() + twice(1)", &[2.0, 3.0], &mut h);
        assert_eq!(r, 4.0 + 6.0 * 1.0 + 2.0);
        let order: Vec<(u32, u32)> = h.calls.iter().map(|(f, s, _)| (*f, *s)).collect();
        assert_eq!(order, vec![(1, 0), (1, 1), (0, 0), (1, 2)]);
        let mut h = Recorder::default();
        assert_eq!(run_with("sum3(t, f, t + f)", &[2.0, 3.0], &mut h), 10.0);
        assert_eq!(h.calls[0].2, vec![2.0, 3.0, 5.0]);
    }

    #[test]
    fn sequences_return_the_last() {
        let mut h = Recorder::default();
        assert_eq!(run_with("rand(); rand(); t", &[9.0, 0.0], &mut h), 9.0);
        assert_eq!(h.calls.len(), 2);
    }

    #[test]
    fn degenerate_values_flow_through() {
        let nan = f32::NAN;
        assert!(run("t + f", &[nan, 1.0]).is_nan());
        assert_eq!(run("t * f", &[f32::INFINITY, 2.0]), f32::INFINITY);
        assert!(run("sqrt(t)", &[-1.0, 0.0]).is_nan());
        assert_eq!(run("t < f", &[nan, 1.0]), 0.0);
        // NaN is true
        assert_eq!(run("t ? 1 : 2", &[nan, 0.0]), 1.0);
        assert!(run("log(t)", &[0.0, 0.0]).is_infinite());
    }

    #[test]
    fn short_inputs_do_not_panic() {
        let e = Expr::compile("t + f * 2", &table()).unwrap();
        // A stack that is too short gives 0, missing variables read 0
        assert_eq!(e.eval(&mut [], &[1.0, 2.0], &mut NoHost), 0.0);
        let mut stack = vec![0.0; e.stack_len()];
        assert_eq!(e.eval(&mut stack, &[], &mut NoHost), 0.0);
        assert_eq!(e.eval(&mut stack, &[1.0], &mut NoHost), 1.0);
        // A longer stack is fine
        let mut big = vec![7.0; 64];
        assert_eq!(e.eval(&mut big, &[1.0, 2.0], &mut NoHost), 5.0);
    }

    #[test]
    fn no_host_calls_give_zero() {
        let e = Expr::compile("rand() + 1", &table()).unwrap();
        let mut stack = vec![0.0; e.stack_len()];
        assert_eq!(e.eval(&mut stack, &[], &mut NoHost), 1.0);
    }
}
