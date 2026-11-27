//! Owner: agent D. Notices and activities, docked on the right (I-14, I-2.2).
//!
//! Above, the activities: every long operation past 200 ms (`CorePort::activity`), each with the shared loading
//! component and its cancel. Below, the notices: the core never writes a sentence, so each notice is a key the
//! interface translates with its arguments (by position, `{0}`, or by name, `{name}`), its severity, how many times
//! it came (counted, not stacked), and the actions it offers ("try again", "recreate the plugin"...), run through the
//! port with their parameters.

use gpui::{Context, FontWeight, Render, Window, div, prelude::*, px};
use lm_port::{Loading, Notice, Severity, Target, Value};

use super::ScreenCx;
use super::settings::port_action;
use crate::ui::{THEME, button, c, caption, loading_component, tr, trf};

pub struct Notices {
    scx: ScreenCx,
    notices: Vec<Notice>,
}

/// A notice's text: its key translated, with the arguments in place, by position or by name
pub fn notice_text(key: &'static str, args: &[(&'static str, Value)]) -> String {
    let mut s = tr(key).to_string();
    for (i, (name, v)) in args.iter().enumerate() {
        let shown = value_text(v);
        s = s.replace(&format!("{{{i}}}"), &shown).replace(&format!("{{{name}}}"), &shown);
    }
    s
}

/// A value as a notice shows it: text as it is, numbers plainly
pub fn value_text(v: &Value) -> String {
    if let Some(t) = v.as_text() {
        t.to_string()
    } else if let Some(i) = v.as_int() {
        i.to_string()
    } else if let Some(r) = v.as_real() {
        format!("{r:.1}")
    } else if let Some(b) = v.as_bool() {
        b.to_string()
    } else {
        String::new()
    }
}

/// An error of a call to the port, as one short line: the key translated, its arguments in place
pub fn error_text(e: &lm_port::Error) -> String {
    use lm_port::Label;
    let Label { key, args } = &e.label;
    let mut s = tr(key).to_string();
    for (i, (name, v)) in args.iter().enumerate() {
        let shown = match v {
            lm_port::Arg::Int(n) => n.to_string(),
            lm_port::Arg::Real(r) => format!("{r:.1}"),
            lm_port::Arg::Text(t) => t.clone(),
        };
        s = s.replace(&format!("{{{i}}}"), &shown).replace(&format!("{{{name}}}"), &shown);
    }
    s
}

/// The label of an action a notice offers
pub fn action_label(id: &str) -> &'static str {
    match id {
        "project.open" | "project.export" | "device.reopen" | "device.calibrate" => tr("Try again"),
        "plugin.recreate" => tr("Recreate the plugin"),
        "resource.locate" => tr("Locate the file"),
        "resource.locate_folder" => tr("Locate the folder"),
        "folder.allow" => tr("Allow this folder"),
        "takes.recover" => tr("Recover"),
        "midi.assign_alias" => tr("Use this port"),
        "project.save_as" => tr("Save as"),
        _ => tr("Do it"),
    }
}

/// What a loading target is, in words
pub fn target_label(t: &Target) -> String {
    match t {
        Target::Project => tr("the project").to_string(),
        Target::Device => tr("the audio device").to_string(),
        Target::Export(n) => trf("export {0}", &[n]),
        Target::Scan(_) => tr("the device list").to_string(),
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

impl Notices {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let notices = scx.core.update(cx, |c, _| c.port_mut().notices());
        Notices { scx, notices }
    }
}

impl Render for Notices {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // the queue as the port has it now
        self.notices = self.scx.core.update(cx, |c, _| c.port_mut().notices());
        let activity = self.scx.core.read(cx).port().activity();
        let core = self.scx.core.clone();
        let activities: Vec<_> = activity
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let loading = Loading::Visible {
                    progress: a.progress,
                    stage: Some(a.label.clone()),
                    cancellable: a.cancellable,
                    since: a.since,
                };
                let (core, op) = (core.clone(), a.id);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .py(px(4.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(caption(target_label(&a.target)))
                    .child(div().h(px(24.)).children(loading_component(("activity", i), &loading, move |_, cx| {
                        core.update(cx, |c, cx| {
                            c.port_mut().cancel(op);
                            cx.notify();
                        });
                    })))
            })
            .collect();
        let cards: Vec<_> = self
            .notices
            .iter()
            .rev()
            .map(|n| {
                let id = n.id;
                let actions = n.actions.iter().enumerate().map(|(k, (action, params))| {
                    let (action, params) = (action.clone(), params.clone());
                    button(("notice-action", (id * 8 + k as u64) as usize), action_label(action.as_str()), false, 0)
                        .debug_selector(move || format!("notice-{id}-action-{k}"))
                        .on_click(cx.listener(move |n: &mut Notices, _, _, cx| {
                            port_action(&n.scx, action.as_str(), params.clone(), cx);
                            n.scx.core.update(cx, |c, _| c.port_mut().dismiss_notice(id));
                            cx.notify();
                        }))
                });
                div()
                    .id(("notice", id as usize))
                    .debug_selector(move || format!("notice-{id}"))
                    .flex()
                    .gap(px(6.))
                    .py(px(5.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(div().flex_none().w(px(3.)).rounded(px(2.)).bg(c(severity_color(n.severity))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().text_size(px(12.)).child(notice_text(n.key, &n.args)))
                            .when(n.repeats > 0, |d| d.child(caption(trf("{0} times", &[&(n.repeats + 1)]))))
                            .child(div().flex().flex_wrap().gap(px(4.)).children(actions)),
                    )
                    .child(
                        button(("dismiss", id as usize), "×", false, 0)
                            .flex_none()
                            .debug_selector(move || format!("dismiss-{id}"))
                            .on_click(cx.listener(move |n: &mut Notices, _, _, cx| {
                                n.scx.core.update(cx, |c, _| c.port_mut().dismiss_notice(id));
                                cx.notify();
                            })),
                    )
            })
            .collect();
        let none = self.notices.is_empty();
        let running = activity.len();
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
                    .px(px(10.))
                    .py(px(6.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr("Notices")))
                    .when(!none, |d| {
                        d.child(button("clear-notices", tr("Clear"), false, 0).on_click(cx.listener(
                            |n: &mut Notices, _, _, cx| {
                                let ids: Vec<u64> = n.notices.iter().map(|x| x.id).collect();
                                n.scx
                                    .core
                                    .update(cx, |c, _| ids.into_iter().for_each(|id| c.port_mut().dismiss_notice(id)));
                                cx.notify();
                            },
                        )))
                    })
                    .child(
                        button("close-notices", tr("Close"), false, 0)
                            .on_click(cx.listener(|n: &mut Notices, _, _, cx| n.scx.close(cx))),
                    ),
            )
            .child(
                div()
                    .id("notices-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .px(px(10.))
                    .py(px(6.))
                    .when(running > 0, |d| d.child(caption(trf("Running now ({0})", &[&running]))))
                    .children(activities)
                    .when(none, |d| {
                        d.child(div().py(px(8.)).text_size(px(12.)).text_color(c(THEME.text3)).child(tr("No notices")))
                    })
                    .children(cards),
            )
    }
}
