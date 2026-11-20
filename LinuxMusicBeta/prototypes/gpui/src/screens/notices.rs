//! Owner: agent D. Notices and activities, docked on the right (I-14, I-2.2).
//!
//! Above, the activity list: every long operation running past 200 ms (`Session::activity`), each with the shared
//! loading component (progress, and cancel after 2 s when it can be cancelled). Below, the notice queue: each notice
//! is a translatable key with arguments (the core never writes a sentence), its severity, when it came (and how many
//! times, counted instead of stacked), and its actions from the registry ("try again", "recreate the plugin",
//! "locate the file", "allow this folder"...). Opening the dock marks the notices read.

use std::time::Duration;

use fake_core::{Loading, Value};
use fake_core::Target;
use fake_core::actions;
use fake_core::notices::Severity;
use gpui::{Context, FontWeight, Render, Window, div, prelude::*, px};

use super::ScreenCx;
use super::palette::{port_action, run_action};
use crate::ui::{THEME, button, c, caption, loading_component, tab, tr, trf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Problems,
    Info,
}

pub struct Notices {
    scx: ScreenCx,
    filter: Filter,
}

impl Notices {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        port_action(&scx, "notices.read", Value::Text(String::new()), cx);
        Notices { scx, filter: Filter::All }
    }
}

/// "3 min ago", from a duration
pub fn ago(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        tr("just now").to_string()
    } else if s < 3600 {
        trf("{0} min ago", &[&(s / 60)])
    } else if s < 86_400 {
        trf("{0} h ago", &[&(s / 3600)])
    } else {
        trf("{0} days ago", &[&(s / 86_400)])
    }
}

/// What a loading target is, in words
pub fn target_label(t: &Target) -> String {
    match t {
        Target::Project => tr("the project").to_string(),
        Target::Device => tr("the audio device").to_string(),
        Target::Export(n) => trf("export {0}", &[n]),
        Target::Scan => tr("the device list").to_string(),
        Target::Node(_) => tr("an item").to_string(),
        Target::Resource(_) => tr("a recording").to_string(),
    }
}

pub fn severity_color(s: Severity) -> u32 {
    match s {
        Severity::Info => THEME.blue,
        Severity::Warning => THEME.orange,
        Severity::Error => THEME.red,
    }
}

impl Render for Notices {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let port = self.scx.core.read(cx).port();
        let activity = port.activity();
        let queue = &port.services().notices;
        let filter = self.filter;
        let shown: Vec<_> = queue
            .iter()
            .filter(|n| match filter {
                Filter::All => true,
                Filter::Problems => n.severity != Severity::Info,
                Filter::Info => n.severity == Severity::Info,
            })
            .cloned()
            .collect();
        let total = queue.len();

        let activities = activity.iter().enumerate().map(|(i, a)| {
            let loading = Loading::Visible { progress: a.progress, stage: a.stage, cancellable: a.cancellable, since: a.since };
            let core = self.scx.core.clone();
            let op = a.op;
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(8.))
                .rounded(px(6.))
                .bg(c(THEME.bg3))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .child(div().text_size(px(12.)).font_weight(FontWeight::BOLD).child(tr(a.stage)))
                        .child(caption(trf("{0} s", &[&a.since.as_secs()]))),
                )
                .child(caption(target_label(&a.target)))
                .child(div().h(px(28.)).children(loading_component(("activity", i), &loading, move |_, cx| {
                    core.update(cx, |core, cx| {
                        core.port_mut().cancel(op);
                        cx.notify();
                    });
                })))
        });

        let cards = shown.into_iter().map(|n| {
            let args: Vec<&dyn std::fmt::Display> = n.args.iter().map(|a| a as &dyn std::fmt::Display).collect();
            let text = trf(n.key, &args);
            let id = n.id;
            let has_actions = !n.actions.is_empty();
            let acts = n.actions.iter().enumerate().filter_map(|(k, na)| {
                let spec = actions::find(na.action)?;
                let scx = self.scx.clone();
                let (action, arg) = (na.action, na.arg.clone());
                Some(
                    button(("notice-action", (id * 8 + k as u64) as usize), tr(spec.label), false, 0)
                        .debug_selector(move || format!("notice-{id}-action-{k}"))
                        .on_click(move |_, window, cx| run_action(&scx, action, &arg, window, cx)),
                )
            });
            div()
                .id(("notice", id as usize))
                .debug_selector(move || format!("notice-{id}"))
                .flex()
                .rounded(px(6.))
                .bg(c(THEME.bg3))
                .overflow_hidden()
                .child(div().w(px(4.)).bg(c(severity_color(n.severity))))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .p(px(8.))
                        .child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(div().flex_1().min_w_0().text_size(px(12.)).child(text))
                                .child(
                                    button(("dismiss", id as usize), "×", false, 0).flex_none()
                                        .debug_selector(move || format!("dismiss-{id}"))
                                        .on_click(cx.listener(move |s, _, _, cx| {
                                            port_action(&s.scx, "notices.dismiss", Value::Text(id.to_string()), cx);
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(caption(ago(n.at.elapsed())))
                                .when(n.repeats > 0, |d| d.child(caption(trf("{0} times", &[&(n.repeats + 1)])))),
                        )
                        .when(has_actions, |d| d.child(div().flex().flex_wrap().gap(px(6.)).children(acts))),
                )
        });

        div()
            .id("notices")
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(div().flex_1().text_size(px(14.)).font_weight(FontWeight::BOLD).child(tr("Notices")))
                    .when(total > 0, |d| {
                        d.child(button("clear-notices", tr("Clear"), false, 0).on_click(cx.listener(|s, _, _, cx| {
                            port_action(&s.scx, "notices.clear", Value::Text(String::new()), cx);
                        })))
                    })
                    .child(button("close-notices", tr("Close"), false, 0).on_click(cx.listener(|s, _, _, cx| s.scx.close(cx)))),
            )
            .child(
                div()
                    .id("notices-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(10.))
                    .child(caption(trf("Running now ({0})", &[&activity.len()])))
                    .when(activity.is_empty(), |d| {
                        d.child(div().text_size(px(12.)).text_color(c(THEME.text3)).child(tr("Nothing running")))
                    })
                    .children(activities)
                    .child(
                        div()
                            .flex()
                            .mt(px(6.))
                            .border_b_1()
                            .border_color(c(THEME.line))
                            .children([(Filter::All, "All"), (Filter::Problems, "Problems"), (Filter::Info, "Information")].map(
                                |(f, label)| {
                                    tab(label, tr(label), filter == f).on_click(cx.listener(move |s, _, _, cx| {
                                        s.filter = f;
                                        cx.notify();
                                    }))
                                },
                            )),
                    )
                    .when(total == 0, |d| d.child(div().text_size(px(12.)).text_color(c(THEME.text3)).child(tr("No notices"))))
                    .children(cards),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::super::Screen;
    use super::super::palette::testing::*;

    /// "Allow this folder" on a notice: the folder joins the allowed ones (C9) and the notice goes away
    #[gpui::test]
    fn a_notice_action_runs_and_resolves_it(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        open_screen(&ws, cx, Screen::Notices);
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        let (id, k) = core.read_with(cx, |core, _| {
            let n = core.port().services().notices.iter().find(|n| n.actions.iter().any(|a| a.action == "folder.allow")).unwrap();
            (n.id, n.actions.iter().position(|a| a.action == "folder.allow").unwrap())
        });
        let sel: &'static str = Box::leak(format!("notice-{id}-action-{k}").into_boxed_str());
        click(cx, sel);
        core.read_with(cx, |core, _| {
            let s = &core.port().services();
            assert!(s.prefs.allowed.iter().any(|f| f == "/media/externo/SoundFonts"));
            assert!(s.notices.get(id).is_none());
            assert_eq!(s.notices.unread(), 0, "opening the dock read them");
        });
        let before = core.read_with(cx, |c, _| c.port().services().notices.len());
        let first = core.read_with(cx, |c, _| c.port().services().notices.iter().next().unwrap().id);
        let sel: &'static str = Box::leak(format!("dismiss-{first}").into_boxed_str());
        click(cx, sel);
        core.read_with(cx, |c, _| assert_eq!(c.port().services().notices.len(), before - 1));
    }
}
