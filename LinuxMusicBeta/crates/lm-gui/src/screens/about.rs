//! Owner: agent D. "About LinuxMusic", as the legacy's dialog (F-GER-009): the name and the version line with the
//! environment (selectable, here also copied with a button for a bug report), and the tabs About, Authors,
//! Involved, Translation and License, with the legacy's own AUTHORS and CONTRIBUTORS. One tab more, Manual (P-34: the
//! manual comes with the program), recorded in decisoes-autonomas.md.

use gpui::{ClipboardItem, Context, FontWeight, Render, Window, div, prelude::*, px};
use lm_port::Value;

use super::ScreenCx;
use super::palette::take_screen_arg;
use super::settings::modal_size;
use crate::ui::{THEME, button, c, caption, tab, tr, trf};

const AUTHORS: &str = include_str!("../../data/AUTHORS");
const CONTRIBUTORS: &str = include_str!("../../data/CONTRIBUTORS");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    About,
    Authors,
    Involved,
    Translation,
    License,
    Manual,
}

const PARTS: [(Part, &str); 6] = [
    (Part::About, "About"),
    (Part::Authors, "Authors"),
    (Part::Involved, "Involved"),
    (Part::Translation, "Translation"),
    (Part::License, "License"),
    (Part::Manual, "Manual"),
];

pub struct About {
    scx: ScreenCx,
    part: Part,
    copied: bool,
    /// The manual's open chapter (none: the list)
    chapter: Option<usize>,
}

impl About {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let part = if take_screen_arg(cx) == Some("manual") { Part::Manual } else { Part::About };
        About { scx, part, copied: false, chapter: None }
    }

    /// The version line and the environment, as text for a bug report
    fn environment(&self, cx: &Context<Self>) -> String {
        let port = self.scx.core.read(cx).port();
        let status = port.status();
        let device = status
            .get("device")
            .filter(|d| !d.is_null())
            .map(|d| {
                let t = |k: &str| d.get(k).and_then(Value::as_text).unwrap_or("").to_string();
                let i = |k: &str| d.get(k).and_then(Value::as_int).unwrap_or(0);
                format!("{} → {}, {} Hz, {}", t("input"), t("output"), i("rate"), i("block"))
            })
            .unwrap_or_else(|| "—".into());
        [
            trf(
                "Version {0} ({1}/{2}, GPUI {3})",
                &[&env!("CARGO_PKG_VERSION"), &std::env::consts::OS, &std::env::consts::ARCH, &"0.3.12"],
            ),
            format!("audio: {device}"),
            format!(
                "{}: {} · {}: {}",
                tr("Language"),
                crate::ui::i18n::language(),
                tr("Theme"),
                crate::ui::theme::theme_name()
            ),
        ]
        .join("\n")
    }
}

impl Render for About {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = modal_size(window, 560.0, 460.0);
        let env = self.environment(cx);
        let mono = |text: &str| {
            div()
                .text_size(px(11.))
                .font_family("Menlo")
                .flex()
                .flex_col()
                .children(text.lines().map(|l| l.to_string()))
        };
        let body = match self.part {
            Part::About => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_size(px(13.)).child(tr("LinuxMusic - easy music production for everyone.")))
                .child(caption(trf("Copyright © {0}", &[&"2004-2026"])))
                .child(div().text_size(px(12.)).text_color(c(THEME.green)).child("https://lmms.io"))
                .into_any_element(),
            Part::Authors => mono(AUTHORS).into_any_element(),
            Part::Involved => div().flex().flex_col().gap(px(6.)).child(mono(CONTRIBUTORS)).into_any_element(),
            Part::Translation => div()
                .text_size(px(12.))
                .child(if crate::ui::i18n::language().starts_with("en") {
                    tr("English").to_string()
                } else {
                    tr("Português, English").to_string()
                })
                .into_any_element(),
            Part::License => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(12.))
                        .child(tr("GNU General Public License, version 2 or later (GPL-2.0-or-later)")),
                )
                .into_any_element(),
            Part::Manual => match self.chapter {
                None => div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .children((0..super::manual::CHAPTERS.len()).map(|i| {
                        div()
                            .id(("chapter", i))
                            .debug_selector(move || format!("chapter-{i}"))
                            .flex()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|s| s.text_color(c(THEME.green)))
                            .child(div().w(px(20.)).text_color(c(THEME.text3)).child(format!("{}.", i + 1)))
                            .child(super::manual::title(i))
                            .on_click(cx.listener(move |a, _, _, cx| {
                                a.chapter = Some(i);
                                cx.notify();
                            }))
                    }))
                    .into_any_element(),
                Some(i) => div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        button("manual-back", tr("Back"), false, 0).debug_selector(|| "manual-back".into()).on_click(
                            cx.listener(|a, _, _, cx| {
                                a.chapter = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .children(super::manual::text(i).lines().map(|l| {
                        if let Some(h) = l.strip_prefix("# ") {
                            div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(h.to_string())
                        } else {
                            div().text_size(px(12.)).min_h(px(6.)).child(l.to_string())
                        }
                    }))
                    .into_any_element(),
            },
        };
        div()
            .id("about")
            .w(px(w))
            .h(px(h))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .rounded(px(8.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).child("LinuxMusic"))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(c(THEME.text2))
                                    .flex()
                                    .flex_col()
                                    .children(env.lines().map(|l| l.to_string())),
                            ),
                    )
                    .child(
                        button(
                            "copy-env",
                            if self.copied { tr("Copied") } else { tr("Copy") },
                            self.copied,
                            THEME.green,
                        )
                        .flex_none()
                        .debug_selector(|| "copy-env".into())
                        .on_click(cx.listener(|a, _, _, cx| {
                            let text = a.environment(cx);
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            a.copied = true;
                            cx.notify();
                        })),
                    )
                    .child(
                        button("close-about", tr("Close"), false, 0)
                            .flex_none()
                            .on_click(cx.listener(|a, _, _, cx| a.scx.close(cx))),
                    ),
            )
            .child(div().flex().border_b_1().border_color(c(THEME.line)).children(PARTS.iter().map(|&(p, l)| {
                tab(l, tr(l), p == self.part).debug_selector(move || format!("about-{p:?}")).on_click(cx.listener(
                    move |a, _, _, cx| {
                        a.part = p;
                        cx.notify();
                    },
                ))
            })))
            .child(div().id("about-body").flex_1().overflow_y_scroll().child(body))
    }
}
