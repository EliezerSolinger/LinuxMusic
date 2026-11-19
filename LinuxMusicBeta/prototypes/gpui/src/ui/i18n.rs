//! Translation as data (spec item 15): keys in English in the code, the shown language in `locales/<lang>/*.txt`
//! (`Key = Translation`, one per line, `#` comments). One file per area, so each screen's author edits only its own
//! file; add the file to `FILES` below (one line). A missing key shows the English key (and is listed by the test).

use std::collections::HashMap;
use std::sync::LazyLock;

/// The Portuguese tables, one per area. ONE LINE PER FILE: add yours here.
const FILES: &[&str] = &[
    include_str!("../../locales/pt-BR/common.txt"),
    include_str!("../../locales/pt-BR/composition.txt"),
    include_str!("../../locales/pt-BR/mixing.txt"),
    include_str!("../../locales/pt-BR/screens.txt"),
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
    TABLE.get(key).copied().unwrap_or(key)
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
