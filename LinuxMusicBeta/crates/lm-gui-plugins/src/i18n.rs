//! The panels' texts: the legacy English source strings are the keys (so the legacy `.ts` translations carry over),
//! `locales/pt-BR/panels.txt` has the Portuguese (`Key = Translation`, one per line, as lm-gui's tables). The
//! interface tells the language once (`set_language`, the same value lm-gui uses); English shows the keys.

use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};

static LANGUAGE: OnceLock<String> = OnceLock::new();

/// The shown language, set once by the interface (`pt-BR`, `en`); later calls are ignored
pub fn set_language(lang: &str) {
    let _ = LANGUAGE.set(lang.to_owned());
}

fn language() -> &'static str {
    LANGUAGE.get().map_or("pt-BR", String::as_str)
}

pub(crate) const FILE: &str = include_str!("../locales/pt-BR/panels.txt");

static TABLE: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    FILE.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once(" = "))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect()
});

/// The shown text of a key
pub fn tr(key: &'static str) -> &'static str {
    if language().starts_with("en") {
        return key;
    }
    TABLE.get(key).copied().unwrap_or(key)
}
