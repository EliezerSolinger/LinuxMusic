//! Owner: agent D. A new project (F-PRJ-001, F-PRJ-002, spec item 8): its name and folder, its sample rate (fixed for
//! the project from here on; the device follows it or converts at the edge), and the template it starts from, with
//! what each template brings. When the open project has changes not saved, it says so first (F-JAN-009). Creating
//! is an operation in the core ("project.create"), with the loading on the project.

use fake_core::Value;
use fake_core::actions::param_lines;
use fake_core::project::{RATES, TEMPLATES};
use gpui::{Context, Entity, FontWeight, Render, Subscription, Window, div, prelude::*, px};
use gpui_component::input::{Input, InputEvent, InputState};

use super::ScreenCx;
use super::palette::port_action;
use super::settings::{badge, khz, modal_size, note, radio_card, segmented};
use crate::ui::{THEME, button, c, caption, tbutton, tr, trf};

pub struct NewProject {
    scx: ScreenCx,
    name: Entity<InputState>,
    place: Entity<InputState>,
    rate: u32,
    template: usize,
    _subs: Vec<Subscription>,
}

impl NewProject {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let p = &scx.core.read(cx).port().services().prefs;
        let (dir, rate) = (p.projects_dir.clone(), p.new_project_rate);
        let name = cx.new(|cx| InputState::new(window, cx).default_value(tr("Untitled")));
        let place = cx.new(|cx| InputState::new(window, cx).default_value(dir));
        name.update(cx, |n, cx| n.focus(window, cx));
        let subs = vec![
            cx.subscribe(&name, |_: &mut NewProject, _, ev: &InputEvent, cx| if matches!(ev, InputEvent::Change) { cx.notify() }),
            cx.subscribe(&place, |_: &mut NewProject, _, ev: &InputEvent, cx| if matches!(ev, InputEvent::Change) { cx.notify() }),
        ];
        NewProject { scx, name, place, rate, template: 1, _subs: subs }
    }

    fn folder(&self, cx: &Context<Self>) -> String {
        format!("{}/{}", self.place.read(cx).value().trim().trim_end_matches('/'), self.name.read(cx).value().trim())
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let folder = self.folder(cx);
        let t = TEMPLATES[self.template].name;
        port_action(
            &self.scx,
            "project.create",
            param_lines(&[("name", name), ("folder", folder), ("rate", self.rate.to_string()), ("template", t.to_string())]),
            cx,
        );
        self.scx.close(cx);
    }
}

impl Render for NewProject {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = modal_size(window, 820.0, 700.0);
        let sv = self.scx.core.read(cx).port().services();
        let (modified, current, read_only, analog) = (sv.project.modified, sv.project.name.clone(), sv.project.read_only(), sv.prefs.analog_desk);
        let folder = self.folder(cx);
        let exists = sv.recent.iter().any(|r| r.folder == folder);
        let name_ok = !self.name.read(cx).value().trim().is_empty() && !exists;
        let rate = self.rate;

        let templates = TEMPLATES.iter().enumerate().map(|(i, t)| {
            let detail = if t.tracks.is_empty() { tr(t.about).to_string() } else { format!("{} — {}", tr(t.about), t.tracks.join(", ")) };
            radio_card(("template", i), i == self.template, tr(t.name), detail).debug_selector(move || format!("template-{i}")).on_click(cx.listener(move |n, _, _, cx| {
                n.template = i;
                cx.notify();
            }))
        });

        div()
            .id("new-project")
            .w(px(w))
            .h(px(h))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(20.))
            .rounded(px(10.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(div().text_size(px(19.)).font_weight(FontWeight::BOLD).child(tr("New project")))
            .when(modified && !read_only, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().min_w_0().child(note(trf("{0} has changes not saved; the journal keeps them, but save before leaving it", &[&current]), THEME.yellow)))
                        .child(button("save-first", tr("Save"), false, 0).flex_none().on_click(cx.listener(|n, _, _, cx| {
                            port_action(&n.scx, "file.save", Value::Text(String::new()), cx);
                            cx.notify();
                        }))),
                )
            })
            .child(
                div()
                    .id("new-project-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().flex().items_center().gap(px(10.)).child(div().w(px(90.)).text_size(px(12.)).child(tr("Name"))).child(div().flex_1().child(Input::new(&self.name))))
                    .child(div().flex().items_center().gap(px(10.)).child(div().w(px(90.)).text_size(px(12.)).child(tr("In"))).child(div().flex_1().child(Input::new(&self.place))))
                    .child(div().flex().gap(px(8.)).child(caption(trf("The project is the folder {0}/", &[&folder]))).when(exists, |d| d.child(badge(tr("already exists"), THEME.red, false))))
                    .child(div().mt(px(6.)).text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr("Sample rate")))
                    .child(segmented("np-rate", RATES.iter().map(|r| (khz(*r).into(), *r == rate, true)).collect(), cx, |n, i, _, cx| {
                        n.rate = RATES[i];
                        cx.notify();
                    }))
                    .child(caption(tr(
                        "Fixed for this project: the device opens at it, or converts at the edge; imported audio is converted once on the way in. Changing it later is an operation that converts audio/",
                    )))
                    .child(div().mt(px(6.)).text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr("Template")))
                    .child(div().flex().flex_col().gap(px(4.)).children(templates))
                    .child(caption(if analog { tr("The desk starts analog (the settings choose)") } else { tr("The desk starts digital (the settings choose)") })),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(button("cancel-new", tr("Cancel"), false, 0).on_click(cx.listener(|n, _, _, cx| n.scx.close(cx))))
                    .child(
                        tbutton("create", tr("Create"), name_ok, THEME.green)
                            .debug_selector(|| "create".into())
                            .when(!name_ok, |b| b.opacity(0.5))
                            .when(name_ok, |b| b.on_click(cx.listener(|n, _, _, cx| n.create(cx)))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::Screen;
    use super::super::palette::testing::*;

    /// A new project at 96 kHz from the Beat template: the rate is the project's from then on
    #[gpui::test]
    fn create_from_a_template(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        open_screen(&ws, cx, Screen::NewProject);
        click(cx, "np-rate-3");
        click(cx, "template-2");
        click(cx, "create");
        ws.read_with(cx, |ws, _| assert!(!ws.is_open(Screen::NewProject)));
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while core.read_with(cx, |c, _| c.port().services().project.rate) != 96_000 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            cx.executor().advance_clock(Duration::from_millis(20));
            cx.run_until_parked();
        }
        core.read_with(cx, |c, _| {
            let p = &c.port().services().project;
            assert_eq!((p.rate, p.template), (96_000, "Beat"));
        });
    }
}
