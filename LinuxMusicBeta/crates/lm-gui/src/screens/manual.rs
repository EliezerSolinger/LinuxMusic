//! The manual (P-34): Markdown files in `docs/manual/pt` and `docs/manual/en`, compiled into the program, so it
//! opens without the internet. Help > Manual opens it on the About screen's Manual tab.

/// One chapter, in both languages
pub struct Chapter {
    pub pt: &'static str,
    pub en: &'static str,
}

macro_rules! chapter {
    ($pt:literal, $en:literal) => {
        Chapter {
            pt: include_str!(concat!("../../../../docs/manual/pt/", $pt)),
            en: include_str!(concat!("../../../../docs/manual/en/", $en)),
        }
    };
}

pub const CHAPTERS: [Chapter; 9] = [
    chapter!("01-primeiros-passos.md", "01-getting-started.md"),
    chapter!("02-gravacao.md", "02-recording.md"),
    chapter!("03-composicao.md", "03-composing.md"),
    chapter!("04-mesa.md", "04-mixing-desk.md"),
    chapter!("05-instrumentos-efeitos.md", "05-instruments-effects.md"),
    chapter!("06-exportacao.md", "06-export.md"),
    chapter!("07-configuracoes.md", "07-settings.md"),
    chapter!("08-problemas.md", "08-problems.md"),
    chapter!("09-servidor-remoto.md", "09-remote-server.md"),
];

/// A chapter's text in the language shown
pub fn text(i: usize) -> &'static str {
    let c = &CHAPTERS[i.min(CHAPTERS.len() - 1)];
    if crate::ui::i18n::language().starts_with("en") { c.en } else { c.pt }
}

/// A chapter's title: its first line without the `# `
pub fn title(i: usize) -> &'static str {
    text(i).lines().next().unwrap_or("").trim_start_matches('#').trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chapter_has_a_title_in_both_languages() {
        for c in &CHAPTERS {
            for t in [c.pt, c.en] {
                assert!(t.starts_with("# "), "a chapter starts with its title");
                assert!(t.lines().count() > 3, "a chapter has content");
            }
        }
    }

    #[test]
    fn the_two_languages_have_the_same_chapters() {
        assert_eq!(CHAPTERS.len(), 9);
        for (i, c) in CHAPTERS.iter().enumerate() {
            assert_ne!(title(i), "");
            assert_ne!(c.pt, c.en);
        }
    }
}
