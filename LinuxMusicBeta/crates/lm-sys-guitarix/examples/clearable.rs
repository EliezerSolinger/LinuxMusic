//! Prints the units that cannot clear their sound (no `clear_state`)
#![allow(clippy::print_stdout)]
fn main() {
    for u in lm_sys_guitarix::units() {
        let mut i = lm_sys_guitarix::Instance::new(u.index).unwrap();
        if !i.clear() {
            println!("{}", u.id);
        }
    }
}
