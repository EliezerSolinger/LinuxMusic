//! The theme as data (spec item 12): a file of `name = #rrggbb` (or `#rrggbbaa`) lines, read at start. No color is
//! written in the code; a screen that needs a new color adds a field here and a line in every theme file.
//! `LM_THEME=<path>` loads another file (a user's theme); a missing or bad line keeps the built-in value.

use std::sync::{LazyLock, OnceLock};

use gpui::{Rgba, rgb};

macro_rules! theme {
    ($($field:ident),* $(,)?) => {
        /// Colors as 0xRRGGBB (or 0xRRGGBBAA for the translucent ones: `scrim`, `selection`)
        #[derive(Debug, Clone, Default)]
        pub struct Theme { $(pub $field: u32),* }
        impl Theme {
            fn set(&mut self, key: &str, v: u32) -> bool {
                match key { $(stringify!($field) => { self.$field = v; true })* _ => false }
            }
        }
    };
}

theme!(
    window,
    bg1,
    bg2,
    bg3,
    bg4,
    line,
    line2,
    text,
    text2,
    text3,
    on_accent,
    green,
    red,
    orange,
    yellow,
    blue,
    purple,
    cyan,
    lcd_bg,
    cursor,
    fader_cap,
    scrim,
    selection,
    loop_range,
    punch_range,
    track1,
    track2,
    track3,
    track4,
    track5,
    track6,
    track7,
    track8,
);

const DARK: &str = include_str!("../../themes/dark.txt");
const LIGHT: &str = include_str!("../../themes/light.txt");

fn parse(text: &str, into: &mut Theme) {
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().trim_start_matches('#');
            if let Ok(n) = u32::from_str_radix(v, 16) {
                into.set(k.trim(), n);
            }
        }
    }
}

static CHOSEN: OnceLock<String> = OnceLock::new();

/// Chooses the built-in theme ("dark" or "light"), once, before the first frame (the preference `theme`); later
/// calls are ignored, so a change applies at the restart. `LM_THEME=<file>` overrides colors on top of it
pub fn set_theme(name: Option<&str>) {
    let _ = CHOSEN.set(name.unwrap_or("dark").to_owned());
}

pub fn theme_name() -> &'static str {
    CHOSEN.get().map_or("dark", String::as_str)
}

pub static THEME: LazyLock<Theme> = LazyLock::new(|| {
    let mut t = Theme::default();
    parse(DARK, &mut t);
    if theme_name() == "light" {
        parse(LIGHT, &mut t);
    }
    if let Ok(path) = std::env::var("LM_THEME")
        && let Ok(text) = std::fs::read_to_string(path)
    {
        parse(&text, &mut t);
    }
    t
});

/// An opaque theme color
pub fn c(hex: u32) -> Rgba {
    rgb(hex)
}

/// The eight track colors (design 02: the number column takes the track's color)
pub fn track_color(i: usize) -> u32 {
    let t = &*THEME;
    [t.track1, t.track2, t.track3, t.track4, t.track5, t.track6, t.track7, t.track8][i % 8]
}

#[cfg(test)]
mod tests {
    #[test]
    fn built_in_theme_sets_every_field() {
        let mut keys = 0;
        for line in super::DARK.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (k, _) = line.split_once('=').unwrap();
            assert!(super::Theme::default().set(k.trim(), 0), "unknown key {k}");
            keys += 1;
        }
        assert_eq!(keys, 33, "every field has a line");
        let light = super::LIGHT.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).count();
        assert_eq!(light, 33, "the light theme sets every field too");
    }
}
