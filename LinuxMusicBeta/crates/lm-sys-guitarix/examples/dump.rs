//! Prints every unit and its parameters (to read what Guitarix registers)
#![allow(clippy::print_stdout)]
fn main() {
    for u in lm_sys_guitarix::units() {
        println!("{} | {} | {} | stereo {} | update {}", u.id, u.name, u.category, u.stereo, u.has_update);
        for p in &u.params {
            println!(
                "    {} | {} | {} | {:?} | {} [{} {}] {} | {:?}",
                p.id,
                p.name,
                p.kind,
                p.var,
                p.default,
                p.low,
                p.up,
                p.step,
                p.values.iter().map(|v| &v.0).collect::<Vec<_>>()
            );
        }
    }
}
