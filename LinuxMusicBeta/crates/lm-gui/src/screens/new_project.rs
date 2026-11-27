//! Owner: agent D. A new project (F-PRJ-001, F-PRJ-002). The legacy's "New" opened the default template and "New
//! from template" was a menu; the new project is a folder with a sample rate fixed for its life (spec item 8), so a
//! small dialog asks for both, with the template (the user's and the factory's, `status().templates`). When the open
//! project has changes not saved, it offers to save first (F-JAN-009). Creating is the session's `project.new`.

use gpui::{Context, Entity, FontWeight, Render, Subscription, Window, div, prelude::*, px};
use gpui_component::input::{Input, InputEvent, InputState};
use lm_port::Value;

use super::ScreenCx;
use super::settings::{form_row, khz, modal_size, note, obj, port_action, segmented};
use crate::ui::{THEME, button, c, tbutton, tr};

const RATES: [i64; 5] = [44_100, 48_000, 88_200, 96_000, 192_000];

pub struct NewProject {
    scx: ScreenCx,
    name: Entity<InputState>,
    place: Entity<InputState>,
    rate: i64,
    /// 0 = empty; then the templates of `status().templates`, by index + 1
    template: usize,
    _subs: Vec<Subscription>,
}

impl NewProject {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prefs = scx.core.read(cx).port().preferences();
        let dir = prefs.get("projectsFolder").and_then(Value::as_text).unwrap_or("").to_string();
        let rate = prefs.get("general").and_then(|g| g.get("newProjectRate")).and_then(Value::as_int).unwrap_or(48_000);
        let name = cx.new(|cx| InputState::new(window, cx).default_value(tr("Untitled")));
        let place = cx.new(|cx| InputState::new(window, cx).default_value(dir));
        name.update(cx, |n, cx| n.focus(window, cx));
        let subs = vec![
            cx.subscribe(&name, |_: &mut NewProject, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify()
                }
            }),
            cx.subscribe(&place, |_: &mut NewProject, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify()
                }
            }),
        ];
        NewProject { scx, name, place, rate, template: 0, _subs: subs }
    }

    fn folder(&self, cx: &Context<Self>) -> String {
        format!("{}/{}", self.place.read(cx).value().trim().trim_end_matches('/'), self.name.read(cx).value().trim())
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let templates: Vec<Value> = self
            .scx
            .core
            .read(cx)
            .port()
            .status()
            .get("templates")
            .map(|l| l.items().cloned().collect())
            .unwrap_or_default();
        let template = self
            .template
            .checked_sub(1)
            .and_then(|i| templates.get(i))
            .and_then(|t| t.get("folder"))
            .and_then(Value::as_text)
            .map_or(Value::NULL, Value::text);
        let params = obj(vec![
            ("folder", Value::text(self.folder(cx))),
            ("rate", Value::int(self.rate)),
            ("template", template),
        ]);
        self.scx.core.update(cx, |core, _| core.run_guarded("project.new", params));
        self.scx.close(cx);
    }
}

impl Render for NewProject {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, _) = modal_size(window, 520.0, 420.0);
        let port = self.scx.core.read(cx).port();
        let modified = port.modified();
        let templates: Vec<Value> =
            port.status().get("templates").map(|l| l.items().cloned().collect()).unwrap_or_default();
        let name_ok = !self.name.read(cx).value().trim().is_empty();
        let rate = self.rate;
        let mut names = vec![(tr("Empty").into(), self.template == 0, true)];
        names.extend(templates.iter().enumerate().map(|(i, t)| {
            let n = t.get("name").and_then(Value::as_text).unwrap_or("").to_string();
            (n.into(), self.template == i + 1, true)
        }));
        div()
            .id("new-project")
            .w(px(w))
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(16.))
            .rounded(px(8.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(tr("New project")))
            .when(modified, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(note(tr("The open project has changes not saved"), THEME.yellow)),
                        )
                        .child(button("save-first", tr("Save"), false, 0).flex_none().on_click(cx.listener(
                            |n, _, _, cx| {
                                port_action(&n.scx, "project.save", Value::NULL, cx);
                                cx.notify();
                            },
                        ))),
                )
            })
            .child(form_row(tr("Name"), crate::ui::text_entry(Input::new(&self.name))))
            .child(form_row(tr("In"), crate::ui::text_entry(Input::new(&self.place))))
            .child(form_row(
                tr("Sample rate"),
                segmented(
                    "np-rate",
                    RATES.iter().map(|r| (khz(*r).into(), *r == rate, true)).collect(),
                    cx,
                    |n, i, _, cx| {
                        n.rate = RATES[i];
                        cx.notify();
                    },
                ),
            ))
            .child(form_row(
                tr("Template"),
                segmented("template", names, cx, |n, i, _, cx| {
                    n.template = i;
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(
                        button("cancel-new", tr("Cancel"), false, 0)
                            .h(px(24.))
                            .px(px(10.))
                            .on_click(cx.listener(|n, _, _, cx| n.scx.close(cx))),
                    )
                    .child(
                        tbutton("create", tr("Create"), name_ok, THEME.green)
                            .debug_selector(|| "create".into())
                            .when(!name_ok, |b| b.opacity(0.5))
                            .when(name_ok, |b| b.on_click(cx.listener(|n, _, _, cx| n.create(cx)))),
                    ),
            )
    }
}
