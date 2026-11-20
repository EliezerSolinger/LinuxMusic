//! Interned symbols: one `&'static str` per distinct text, for the life of the process.
//!
//! The keys of the schemas, the parameters and the labels are `&'static str` (they are compared and copied
//! everywhere, and most are literals). A key made at run time (`"{prefix}Attack"`, `plugin:<id>`, a key read from a
//! file) becomes one here instead of with `Box::leak` at each call site. The difference is the whole point:
//!
//! - **bounded**: the same text gives the same pointer, so building a registry, a schema or a panel a thousand times
//!   allocates each key once, not a thousand times;
//! - **owned**: the table is a static, so every symbol stays reachable from it. Nothing is lost, and the leak
//!   checkers (LeakSanitizer, miri, `leaks` on macOS) see a cache that lives as long as the process, not a leak
//!   (decisoes-autonomas.md, "Vazamentos e sanitizers").
//!
//! The vocabulary is finite (the keys a program knows), so the table stops growing once every schema was built.
//! Text that comes from outside without a limit (a remote client, a third-party plugin) goes through an interner
//! with a cap of its own before reaching here. Interning locks and may allocate: never on the audio thread.

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

static TABLE: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());

/// The symbol for `text`: the same `&'static str` for every call with equal text
pub fn intern(text: &str) -> &'static str {
    let mut table = TABLE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(&symbol) = table.get(text) {
        return symbol;
    }
    // The only allocation that is never freed: it belongs to the table, which lives as long as the process
    let symbol: &'static str = Box::leak(text.to_owned().into_boxed_str());
    table.insert(symbol);
    symbol
}

/// How many distinct symbols were made so far (for tests that check the table stops growing)
pub fn interned() -> usize {
    TABLE.lock().unwrap_or_else(PoisonError::into_inner).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_text_gives_the_same_symbol_once() {
        let a = intern(&format!("symbol.test.{}", 7));
        let before = interned();
        for _ in 0..100 {
            let b = intern(&format!("symbol.test.{}", 7));
            assert!(std::ptr::eq(a, b));
        }
        assert_eq!(interned(), before, "no new symbol for a text already interned");
        assert_eq!(intern("symbol.test.8"), "symbol.test.8");
    }
}
