//! Owner: agent D. About (F-GER-009) and the manual (P-34): the version and the environment (selectable and copied
//! with one click, for a bug report: the program, GPUI, the platform, where the core runs, the audio device and the
//! real-time permission), the authors (the fork and the LMMS contributors it comes from), the translations, the
//! license (GPL-2.0-or-later) and the libraries; and the manual that comes with the program, in Portuguese and English.

use fake_core::project::Link;
use gpui::{ClipboardItem, Context, FontWeight, Render, Window, div, prelude::*, px};

use super::ScreenCx;
use super::palette::take_screen_arg;
use super::settings::{khz, modal_size, note};
use crate::ui::{THEME, button, c, caption, tab, tr, trf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    About,
    Authors,
    License,
    Manual,
}

pub struct About {
    scx: ScreenCx,
    part: Part,
    copied: bool,
}

/// The libraries in the build, and their licenses
const LIBRARIES: &[(&str, &str)] = &[
    ("GPUI (gpui-pre =0.3.12, Zed)", "Apache-2.0"),
    ("gpui-component =0.10.0", "Apache-2.0"),
    ("ALSA (alsa-lib)", "LGPL-2.1"),
    ("libsndfile", "LGPL-2.1"),
    ("libsamplerate", "BSD-2-Clause"),
    ("FluidSynth", "LGPL-2.1"),
    ("Rubber Band", "GPL-2.0-or-later"),
    ("imbl", "MPL-2.0"),
    ("zbus", "MIT"),
];

/// The manual's chapters (English keys)
const CHAPTERS: &[&str] = &[
    "Starting: the project folder, the device, the first recording",
    "Recording: inputs, arming, monitoring, takes and comping",
    "Composition: tracks, items, the Piano Roll, automation, steps",
    "Mixing: the desk, the rack of each channel, inserts and sends",
    "Instruments and effects, built in and third-party",
    "Exporting: formats, ranges, stems",
    "Settings, shortcuts and the command palette",
    "When something goes wrong: notices, recovery, safe mode",
    "Working with a server",
];

impl About {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let part = if take_screen_arg(cx) == Some("manual") { Part::Manual } else { Part::About };
        About { scx, part, copied: false }
    }

    /// The environment as text (for a bug report)
    fn environment(&self, cx: &Context<Self>) -> String {
        let sv = self.scx.core.read(cx).port().services();
        let device = sv.devices.opened.as_ref().map_or("—".to_string(), |o| format!("{} → {}, {}, {}", o.input, o.output, khz(o.rate), o.block));
        let core = match &sv.link {
            Link::Connected { address, server, .. } => format!("{server} ({address})"),
            _ => "local".to_string(),
        };
        let rt = &sv.devices.realtime;
        [
            format!("LinuxMusic {} (prototype)", env!("CARGO_PKG_VERSION")),
            "GPUI gpui-pre 0.3.12 · gpui-component 0.10.0".to_string(),
            format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            format!("core: {core}"),
            format!("audio: {device}"),
            format!("real time: rtkit {} · RLIMIT_RTPRIO {} · audio group {}", rt.rtkit, rt.rtprio, rt.audio_group),
            format!("language: {} · theme: {}", sv.prefs.language, sv.prefs.started_theme),
        ]
        .join("\n")
    }
}

impl Render for About {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = modal_size(window, 720.0, 600.0);
        let env = self.environment(cx);
        let parts = [(Part::About, "About"), (Part::Authors, "Authors"), (Part::License, "License"), (Part::Manual, "Manual")];
        let body = match self.part {
            Part::About => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(div().text_size(px(13.)).child(tr("Recording, composing and mixing music, on Linux first.")))
                .child(caption(tr("Version and environment (select, or copy for a bug report)")))
                .child(div().p(px(10.)).rounded(px(6.)).bg(c(THEME.lcd_bg)).font_family("Menlo").text_size(px(12.)).flex().flex_col().children(env.lines().map(|l| l.to_string())))
                .child(div().flex().gap(px(8.)).items_center().child(button("copy-env", tr("Copy"), self.copied, THEME.green).debug_selector(|| "copy-env".into()).on_click(cx.listener(|a, _, _, cx| {
                    let text = a.environment(cx);
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    a.copied = true;
                    cx.notify();
                }))).when(self.copied, |d| d.child(caption(tr("Copied")))))
                .into_any_element(),
            Part::Authors => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_size(px(13.)).child(tr("LinuxMusic is a new program built from what the LinuxMusic fork of LMMS does.")))
                .child(caption(tr("The fork's work: recording, the analog desk, the drum kits, the amplifiers")))
                .child(div().text_size(px(13.)).child("Eliezer Solinger"))
                .child(caption(tr("LMMS, which the fork comes from: its authors and contributors, in the order of their commits")))
                .child(div().text_size(px(12.)).text_color(c(THEME.text2)).child("Tobias Doerffel, Paul Giblock, Vesa Kivimäki, Lukas W, Hyunjin Song, Dominic Clark, Kevin Zander, Johannes Lorenz, Oskar Wallgren, Spekular, Colin Wallace, Javier Serrano Polo, Tres Finocchiaro, Michael Gregorius, and many more."))
                .child(caption(tr("Translations")))
                .child(div().text_size(px(12.)).text_color(c(THEME.text2)).child(tr("Portuguese and English with the program; the legacy's other languages where the text still matches")))
                .into_any_element(),
            Part::License => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_size(px(13.)).child(tr("GNU General Public License, version 2 or later (GPL-2.0-or-later)")))
                .child(caption(tr("You may use, study, share and change it; what you share, you share under the same license.")))
                .child(caption(tr("Libraries")))
                .children(LIBRARIES.iter().map(|(n, l)| div().flex().text_size(px(12.)).child(div().w(px(320.)).child(*n)).child(div().text_color(c(THEME.text3)).child(*l))))
                .into_any_element(),
            Part::Manual => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(note(tr("The manual comes with the program, in Portuguese and English, and opens without the internet"), THEME.blue))
                .children(CHAPTERS.iter().enumerate().map(|(i, k)| div().flex().gap(px(10.)).text_size(px(13.)).child(div().w(px(24.)).text_color(c(THEME.text3)).child(format!("{}.", i + 1))).child(tr(k))))
                .child(caption(trf("F1 opens it from anywhere; {0}", &[&tr("each screen's help opens its own chapter")])))
                .into_any_element(),
        };
        div()
            .id("about")
            .w(px(w))
            .max_h(px(h))
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(20.))
            .rounded(px(10.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().flex_1().flex().flex_col().child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).child("LinuxMusic")).child(caption(trf("Version {0}", &[&env!("CARGO_PKG_VERSION")]))))
                    .child(button("close-about", tr("Close"), false, 0).on_click(cx.listener(|a, _, _, cx| a.scx.close(cx)))),
            )
            .child(div().flex().border_b_1().border_color(c(THEME.line)).children(parts.iter().map(|&(p, l)| {
                tab(l, tr(l), p == self.part).debug_selector(move || format!("about-{p:?}")).on_click(cx.listener(move |a, _, _, cx| {
                    a.part = p;
                    cx.notify();
                }))
            })))
            .child(div().id("about-body").flex_1().overflow_y_scroll().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::super::Screen;
    use super::super::palette::{ScreenArg, testing::*};

    #[gpui::test]
    fn copies_the_environment_and_opens_the_manual(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        open_screen(&ws, cx, Screen::About);
        click(cx, "copy-env");
        let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
        assert!(text.contains("GPUI") && text.contains("core: local"));
        ws.update(cx, |ws, cx| ws.close(Screen::About, cx));
        cx.update(|_, cx| cx.set_global(ScreenArg(Some("manual"))));
        open_screen(&ws, cx, Screen::About);
        assert!(cx.debug_bounds("about-Manual").is_some());
    }
}
