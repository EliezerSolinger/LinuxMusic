//! Translation as data (spec item 15): keys in English in the code, the shown language in `locales/<lang>/*.txt`
//! (`Key = Translation`, one per line, `#` comments). One file per area, so each screen's author edits only its own
//! file; add the file to `FILES` below (one line). A missing key shows the English key (and is listed by the test).
//!
//! The language: `LM_LANG` (`pt-BR`, `en`), else the preference `language`, else Portuguese. English shows the keys
//! themselves. Chosen once, when the program starts (`set_language`); the settings say it applies at the restart.

use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};

static LANGUAGE: OnceLock<String> = OnceLock::new();

/// Chooses the language, once (later calls are ignored); `LM_LANG` wins over `preferred`
pub fn set_language(preferred: Option<&str>) {
    let lang = std::env::var("LM_LANG").ok().or_else(|| preferred.map(str::to_owned)).unwrap_or_else(|| "pt-BR".into());
    let _ = LANGUAGE.set(lang);
}

/// The language shown
pub fn language() -> &'static str {
    LANGUAGE.get().map_or("pt-BR", String::as_str)
}

/// The Portuguese tables, one per area. ONE LINE PER FILE: add yours here.
const FILES: &[&str] = &[
    include_str!("../../locales/pt-BR/common.txt"),
    include_str!("../../locales/pt-BR/composition.txt"),
    include_str!("../../locales/pt-BR/mixing.txt"),
    include_str!("../../locales/pt-BR/screens.txt"),
    include_str!("../../locales/pt-BR/editors.txt"),
    include_str!("../../locales/pt-BR/plugins.txt"),
    include_str!("../../locales/pt-BR/system.txt"),
    include_str!("../../locales/pt-BR/midi.txt"),
    include_str!("../../locales/pt-BR/keys.txt"),
];

static TABLE: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut t = HashMap::new();
    for file in FILES {
        for line in file.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once(" = ") {
                t.insert(k.trim(), v.trim());
            }
        }
    }
    t
});

/// The shown text for an English key
pub fn tr(key: &'static str) -> &'static str {
    if language().starts_with("en") {
        return key;
    }
    TABLE.get(key).copied().unwrap_or(key)
}

/// `tr` for a key known only at run time (a plugin's port, a mode read from the document): the key becomes an
/// interned symbol, made once for each distinct key and owned by the symbol table (lm_base::intern), so drawing it
/// every frame allocates nothing new
pub fn tr_str(key: &str) -> &'static str {
    tr(lm_port::intern(key))
}

/// `tr` with `{0}`, `{1}`... replaced
pub fn trf(key: &'static str, args: &[&dyn std::fmt::Display]) -> String {
    let mut s = tr(key).to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), &a.to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn tables_parse_without_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for file in super::FILES {
            for line in file.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                let (k, _) = line.split_once(" = ").unwrap_or_else(|| panic!("not `Key = Value`: {line}"));
                assert!(seen.insert(k.trim()), "duplicate key {k}");
            }
        }
    }
}
