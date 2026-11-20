//! Owner: agent D. The project dialog: the PROJECT FOLDER (`projeto.json`, `interface.json`, `audio/`, `backup/`).
//!
//! - "This project": where it is, its rate, when it was saved, the journal, the folder's parts; read-only when the
//!   project is locked by another program or comes from a newer version (H10, H11), with "Save as" as the way out;
//!   the copy kept when converting from an older version (H11); the takes interrupted by a crash, each recovered to
//!   its place or left (U5); safe mode with the plugin that stopped the program off (U21); save, save as a new
//!   version (F-PRJ-008), save as a template (F-PRJ-002), export as a single file (.zip, with its progress), cleaning
//!   the unused files to the trash (H4, P-13), and the project's notes (F-PRJ-018).
//! - "Open": the recent projects (F-PRJ-015) with their rate, tracks, length and when they changed, and a folder or
//!   a .zip by its path (F-PRJ-003); opening is an operation with the shared loading component and cancel.
//! - "Save as": the name and the place, with what the folder will hold (F-PRJ-007).
//! - "Server": connecting to a remote core (I-15).
//!
//! Everything goes through the port: the state from `port().services().project`, the changes as actions.

use fake_core::Target;
use fake_core::Value;
use fake_core::actions::param_lines;
use fake_core::project::{Access, ReadOnly};
use gpui::{AnyElement, Context, Entity, FontWeight, Render, Subscription, Window, div, prelude::*, px};
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};

use super::ScreenCx;
use super::notices::ago;
use super::palette::{port_action, take_screen_arg};
use super::settings::{badge, connect_panel, khz, modal_size, note, radio_card, target_loading};
use crate::ui::{THEME, button, c, caption, tbutton, tr, trf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Panel {
    Project,
    Open,
    SaveAs,
    Server,
}

pub struct ProjectDialog {
    scx: ScreenCx,
    panel: Panel,
    chosen: usize,
    path: Entity<InputState>,
    name: Entity<InputState>,
    place: Entity<InputState>,
    notes: Entity<TextareaState>,
    server: Entity<InputState>,
    token: Entity<InputState>,
    /// "Clean the unused files" asks once before (it clears the undo history)
    confirm_clean: bool,
    _subs: Vec<Subscription>,
}

/// The next version's name: "Música" → "Música v2", "Música v2" → "Música v3" (F-PRJ-008)
pub fn next_version(name: &str) -> String {
    match name.rsplit_once(" v") {
        Some((base, n)) if n.parse::<u32>().is_ok() => format!("{base} v{}", n.parse::<u32>().unwrap_or(1) + 1),
        _ => format!("{name} v2"),
    }
}

impl ProjectDialog {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let arg = take_screen_arg(cx);
        let sv = scx.core.read(cx).port().services();
        let p = &sv.project;
        let parent = p.folder.rsplit_once('/').map_or(sv.prefs.projects_dir.clone(), |(a, _)| a.to_string());
        let version = arg == Some("version");
        let (name_v, notes_v, server_v) =
            (if version { next_version(&p.name) } else { p.name.clone() }, p.notes.clone(), sv.servers.first().cloned().unwrap_or_default());
        let path = cx.new(|cx| InputState::new(window, cx).placeholder(tr("/path/to/a/project or a .zip")));
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name_v));
        let place = cx.new(|cx| InputState::new(window, cx).default_value(parent));
        let notes = cx.new(|cx| TextareaState::new(window, cx).default_value(notes_v));
        let server = cx.new(|cx| InputState::new(window, cx).default_value(server_v).placeholder(tr("host:port or /path/to/socket")));
        let token = cx.new(|cx| InputState::new(window, cx).placeholder(tr("pairing token")));
        let subs = vec![
            cx.subscribe(&name, |_: &mut ProjectDialog, _, ev: &InputEvent, cx| if matches!(ev, InputEvent::Change) { cx.notify() }),
            cx.subscribe(&place, |_: &mut ProjectDialog, _, ev: &InputEvent, cx| if matches!(ev, InputEvent::Change) { cx.notify() }),
            // the notes are the project's: saved in it as they are written (on leaving the field)
            cx.subscribe(&notes, |d: &mut ProjectDialog, st, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Blur) {
                    let v = st.read(cx).value().to_string();
                    port_action(&d.scx, "project.notes", Value::Text(v), cx);
                }
            }),
            cx.subscribe(&path, |d: &mut ProjectDialog, st, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    let v = st.read(cx).value().to_string();
                    d.open_path(v, cx);
                }
            }),
        ];
        let panel = match arg {
            Some("open") => Panel::Open,
            Some("save_as") | Some("version") => Panel::SaveAs,
            Some("connect") => Panel::Server,
            _ => Panel::Project,
        };
        ProjectDialog { scx, panel, chosen: 0, path, name, place, notes, server, token, confirm_clean: false, _subs: subs }
    }

    fn act(&mut self, id: &str, params: Value, cx: &mut Context<Self>) {
        port_action(&self.scx, id, params, cx);
        cx.notify();
    }

    fn open_path(&mut self, v: String, cx: &mut Context<Self>) {
        if !v.trim().is_empty() {
            self.act("project.open_path", Value::Text(v), cx);
            self.panel = Panel::Project;
        }
    }

    fn save_as(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        let place = self.place.read(cx).value().trim().trim_end_matches('/').to_string();
        if name.is_empty() || place.is_empty() {
            return;
        }
        let folder = format!("{place}/{name}");
        self.act("project.save_to", param_lines(&[("name", name), ("folder", folder)]), cx);
        self.panel = Panel::Project;
    }

    fn panel_project(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let sv = self.scx.core.read(cx).port().services();
        let p = sv.project.clone();
        let saving = sv.exporting.is_none() && false;
        let _ = saving;
        let zip_running = self.scx.core.read(cx).port().activity().iter().any(|a| a.stage == "Packing the project");
        let loading = target_loading(&self.scx, "project-loading", Target::Project, cx);
        let read_only = p.read_only();

        let mut col = div().flex().flex_col().gap(px(10.));
        if let Some(l) = loading {
            col = col.child(div().h(px(30.)).child(l));
        }
        // read-only, and why (H10, H11)
        if let Access::ReadOnly(why) = &p.access {
            let text = match why {
                ReadOnly::Locked { machine, pid, since, this_machine } => {
                    if *this_machine {
                        trf("Open in another LinuxMusic on this machine (process {0}, since {1}): it opened read-only", &[pid, since])
                    } else {
                        trf("Open on {0} (process {1}, since {2}): it opened read-only, so the two never write over each other", &[machine, pid, since])
                    }
                }
                ReadOnly::Newer { schema, version } => trf(
                    "Saved by LinuxMusic {0} (format {1}; this one writes {2}): it opened read-only and is never saved over",
                    &[version, schema, &fake_core::project::SCHEMA],
                ),
            };
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().min_w_0().child(note(format!("{} {}", text, tr("Changes work, but saving, the journal and autosave are off: Save as makes it yours.")), THEME.orange)))
                    .child(tbutton("ro-save-as", tr("Save as…"), true, THEME.green).flex_none().debug_selector(|| "ro-save-as".into()).on_click(cx.listener(|d, _, _, cx| {
                        d.panel = Panel::SaveAs;
                        cx.notify();
                    }))),
            );
        }
        if let Some(v) = p.migrated_from {
            col = col.child(note(trf("Converted from format {0}: the original stays as projeto.json.v{0}.bak, so the older version still opens it", &[&v]), THEME.blue));
        }
        // takes interrupted by a crash (U5)
        if !p.recovery.is_empty() {
            let takes = p.recovery.iter().enumerate().map(|(i, t)| {
                let state: AnyElement = match t.decision {
                    Some(true) => badge(tr("recovered"), THEME.green, true).into_any_element(),
                    Some(false) => badge(tr("left in audio/"), THEME.text3, false).into_any_element(),
                    None => div()
                        .flex()
                        .gap(px(6.))
                        .child(button(("recover", i), tr("Recover"), false, 0).debug_selector(move || format!("recover-{i}")).on_click(cx.listener(move |d, _, _, cx| {
                            d.act("project.decide_take", param_lines(&[("index", i.to_string()), ("recover", "true".into())]), cx)
                        })))
                        .child(button(("leave", i), tr("Leave out"), false, 0).on_click(cx.listener(move |d, _, _, cx| {
                            d.act("project.decide_take", param_lines(&[("index", i.to_string()), ("recover", "false".into())]), cx)
                        })))
                        .into_any_element(),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .bg(c(THEME.bg3))
                    .child(
                        div().flex_1().flex().flex_col().child(div().text_size(px(13.)).child(t.track.clone())).child(caption(trf(
                            "{0} · from bar {1} · {2} s{3}",
                            &[&t.input, &t.start_bar, &format!("{:.1}", t.seconds), &if t.loop_cuts > 0 { format!(" · {}", trf("{0} loop passes, one lane each", &[&(t.loop_cuts + 1)])) } else { String::new() }],
                        ))),
                    )
                    .child(state)
            });
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(c(THEME.orange))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child(tr("Takes interrupted by a crash")))
                    .child(caption(tr("Each take's recording.json says its track, input, start and the latency taken off: a recovered take goes exactly where it was being recorded")))
                    .children(takes),
            );
        }
        // safe mode (U21)
        if !p.safe_mode.is_empty() {
            let rows = p.safe_mode.iter().enumerate().map(|(i, m)| {
                let on = m.enabled;
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().text_size(px(13.)).child(trf("{0} on {1}: {2}", &[&m.plugin, &m.track, &tr(m.why)])))
                    .child(
                        button(("safe", i), if on { tr("Turn off again") } else { tr("Turn on") }, on, THEME.green)
                            .debug_selector(move || format!("safe-{i}"))
                            .on_click(cx.listener(move |d, _, _, cx| d.act("project.safe_plugin", param_lines(&[("index", i.to_string()), ("on", (!on).to_string())]), cx))),
                    )
            });
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(c(THEME.yellow))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child(tr("Safe mode")))
                    .child(caption(tr("The program stopped while this plugin ran: it opened off, the track is silent, and its settings are kept. Turn it on to try again")))
                    .children(rows),
            );
        }
        // where it is, and what the folder holds
        let saved = p.saved.map_or(tr("never").to_string(), |t| ago(t.elapsed()));
        let layout: [(&str, &'static str); 4] = [
            ("projeto.json", "the project's tree"),
            ("interface.json", "windows, zoom, scrolling"),
            ("audio/", ""),
            ("backup/", "autosaves and the journal"),
        ];
        col = col
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().flex().items_center().gap(px(8.)).child(div().text_size(px(19.)).font_weight(FontWeight::BOLD).child(p.name.clone())).when(p.modified, |d| d.child(badge(tr("not saved"), THEME.yellow, false))).when(read_only, |d| d.child(badge(tr("read-only"), THEME.orange, true))))
                    .child(div().text_size(px(12.)).font_family("Menlo").text_color(c(THEME.text2)).child(p.folder.clone()))
                    .child(caption(trf("{0}, fixed for this project · saved {1} · {2} changes in the journal since then", &[&khz(p.rate), &saved, &p.journal]))),
            )
            .child(div().flex().flex_col().gap(px(2.)).children(layout.into_iter().map(|(f, what)| {
                let what = if f == "audio/" { trf("{0} files, {1} MB, each named by its content's hash", &[&p.audio_files, &format!("{:.0}", p.audio_mb)]) } else { tr(what).to_string() };
                div().flex().gap(px(10.)).text_size(px(12.)).child(div().w(px(120.)).font_family("Menlo").child(f)).child(div().text_color(c(THEME.text3)).child(what))
            })))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(tbutton("save", tr("Save"), !read_only, THEME.green).when(read_only, |b| b.opacity(0.4)).when(!read_only, |b| b.on_click(cx.listener(|d, _, _, cx| d.act("file.save", Value::Text(String::new()), cx)))))
                    .child(tbutton("save-version", tr("Save as a new version"), false, 0).on_click(cx.listener(|d, _, window, cx| {
                        let v = next_version(&d.scx.core.read(cx).port().services().project.name);
                        d.name.update(cx, |st, cx| st.set_value(v, window, cx));
                        d.panel = Panel::SaveAs;
                        cx.notify();
                    })))
                    .child(tbutton("save-template", tr("Save as a template"), false, 0).on_click(cx.listener(|d, _, _, cx| d.act("file.save_template", Value::Text(String::new()), cx))))
                    .child(
                        tbutton("zip", if zip_running { tr("Packing…") } else { tr("Export as a single file (.zip)") }, zip_running, THEME.green)
                            .debug_selector(|| "zip".into())
                            .when(!zip_running, |b| b.on_click(cx.listener(|d, _, _, cx| d.act("file.export_zip", Value::Text(String::new()), cx)))),
                    ),
            )
            .child(caption(tr("The .zip holds the whole folder, to send or keep; opening a .zip extracts it into a folder")));
        // unused files (H4, P-13)
        let (n, mb) = p.unused;
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().flex_1().text_size(px(12.)).child(if n == 0 {
                    tr("No unused files in audio/").to_string()
                } else {
                    trf("{0} files in audio/ ({1} MB) are no longer used by the project or its history", &[&n, &format!("{mb:.0}")])
                }))
                .when(n > 0 && !self.confirm_clean, |d| {
                    d.child(button("clean", tr("Clean the unused files…"), false, 0).flex_none().debug_selector(|| "clean".into()).on_click(cx.listener(|d, _, _, cx| {
                        d.confirm_clean = true;
                        cx.notify();
                    })))
                }),
        );
        if self.confirm_clean && n > 0 {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(note(trf("{0} files go to the system's trash, and the undo history is cleared (nothing can be undone past this point)", &[&n]), THEME.orange)))
                    .child(button("clean-cancel", tr("Cancel"), false, 0).flex_none().on_click(cx.listener(|d, _, _, cx| {
                        d.confirm_clean = false;
                        cx.notify();
                    })))
                    .child(tbutton("clean-yes", tr("Move to the trash"), true, THEME.red).flex_none().debug_selector(|| "clean-yes".into()).on_click(cx.listener(|d, _, _, cx| {
                        d.confirm_clean = false;
                        d.act("project.clean_unused", Value::Text(String::new()), cx);
                    }))),
            );
        }
        col.child(div().mt(px(6.)).text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr("Project notes")))
            .child(div().h(px(110.)).child(Textarea::new(&self.notes)))
            .into_any_element()
    }

    fn panel_open(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let sv = self.scx.core.read(cx).port().services();
        let recent = sv.recent.clone();
        let loading = target_loading(&self.scx, "open-loading", Target::Project, cx);
        let cards = recent.iter().enumerate().map(|(i, r)| {
            radio_card(("recent", i), i == self.chosen, r.name.clone(), trf("{0} · {1} · {2} tracks · {3} min · changed {4}", &[&r.folder, &khz(r.rate), &r.tracks, &format!("{:.0}", r.minutes), &ago(r.modified_ago)]))
                .debug_selector(move || format!("recent-{i}"))
                .on_click(cx.listener(move |d, ev: &gpui::ClickEvent, _, cx| {
                    d.chosen = i;
                    if ev.click_count() >= 2 {
                        d.act("project.open_recent", Value::Text(i.to_string()), cx);
                        d.panel = Panel::Project;
                    }
                    cx.notify();
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(loading.map(|l| div().h(px(30.)).child(l)))
            .child(caption(tr("Recent projects (double click opens)")))
            .child(div().flex().flex_col().gap(px(4.)).children(cards))
            .child(
                div().flex().justify_end().child(tbutton("open-recent", tr("Open"), true, THEME.green).debug_selector(|| "open-recent".into()).on_click(cx.listener(|d, _, _, cx| {
                    let i = d.chosen;
                    d.act("project.open_recent", Value::Text(i.to_string()), cx);
                    d.panel = Panel::Project;
                }))),
            )
            .child(caption(tr("A project folder, or a .zip exported as a single file")))
            .child(
                div().flex().gap(px(6.)).child(div().flex_1().child(Input::new(&self.path))).child(button("open-path", tr("Open"), false, 0).on_click(cx.listener(|d, _, _, cx| {
                    let v = d.path.read(cx).value().to_string();
                    d.open_path(v, cx);
                }))),
            )
            .child(note(tr("Projects of the old program (.mmp, .mmpz) do not open here; use the old program, or the converter when it comes"), THEME.blue))
            .into_any_element()
    }

    fn panel_save_as(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let sv = self.scx.core.read(cx).port().services();
        let p = sv.project.clone();
        let name = self.name.read(cx).value().to_string();
        let place = self.place.read(cx).value().to_string();
        let folder = format!("{}/{}", place.trim_end_matches('/'), name);
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().flex().items_center().gap(px(10.)).child(div().w(px(90.)).text_size(px(12.)).child(tr("Name"))).child(div().flex_1().child(Input::new(&self.name))))
            .child(div().flex().items_center().gap(px(10.)).child(div().w(px(90.)).text_size(px(12.)).child(tr("In"))).child(div().flex_1().child(Input::new(&self.place))))
            .child(caption(tr("The project is a folder; this is what it will hold:")))
            .child(
                div()
                    .p(px(10.))
                    .rounded(px(6.))
                    .bg(c(THEME.lcd_bg))
                    .font_family("Menlo")
                    .text_size(px(12.))
                    .flex()
                    .flex_col()
                    .child(format!("{folder}/"))
                    .child(format!("  projeto.json      {}", tr("the project's tree")))
                    .child(format!("  interface.json    {}", tr("windows, zoom, scrolling")))
                    .child(format!("  audio/            {}", trf("{0} files, {1} MB, copied", &[&p.audio_files, &format!("{:.0}", p.audio_mb)])))
                    .child(format!("  backup/           {}", tr("autosaves and the journal"))),
            )
            .child(caption(tr("Saving writes projeto.json.tmp and renames it at the end: a power cut never leaves half a project")))
            .when(p.read_only(), |d| d.child(note(tr("The new folder is yours: it opens writable, with its own lock"), THEME.green)))
            .child(div().flex().justify_end().child(tbutton("save-as-go", tr("Save"), true, THEME.green).debug_selector(|| "save-as-go".into()).on_click(cx.listener(|d, _, _, cx| d.save_as(cx)))))
            .into_any_element()
    }
}

impl Render for ProjectDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = modal_size(window, 900.0, 720.0);
        let link = self.scx.core.read(cx).port().services().link.clone();
        let servers = self.scx.core.read(cx).port().services().servers.clone();
        let body = match self.panel {
            Panel::Project => self.panel_project(cx),
            Panel::Open => self.panel_open(cx),
            Panel::SaveAs => self.panel_save_as(cx),
            Panel::Server => {
                let (server, token) = (self.server.clone(), self.token.clone());
                connect_panel(&link, &servers, &server, &token, cx, |d: &mut ProjectDialog| (d.scx.clone(), d.server.clone())).into_any_element()
            }
        };
        let tabs = [(Panel::Project, "This project"), (Panel::Open, "Open"), (Panel::SaveAs, "Save as"), (Panel::Server, "Server")];
        let nav = div()
            .w(px(190.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(12.))
            .bg(c(THEME.bg1))
            .border_r_1()
            .border_color(c(THEME.line))
            .child(div().mb(px(8.)).text_size(px(15.)).font_weight(FontWeight::BOLD).child(tr("Project")))
            .children(tabs.iter().map(|&(p, label)| {
                let on = p == self.panel;
                div()
                    .id(("project-panel", p as usize))
                    .debug_selector(move || format!("project-panel-{p:?}"))
                    .px(px(12.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .when(on, |d| d.bg(c(THEME.bg3)).font_weight(FontWeight::BOLD))
                    .text_color(c(if on { THEME.text } else { THEME.text2 }))
                    .cursor_pointer()
                    .child(tr(label))
                    .on_click(cx.listener(move |d, _, _, cx| {
                        d.panel = p;
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(button("new-from-dialog", tr("New project…"), false, 0).on_click(cx.listener(|d, _, window, cx| d.scx.open(super::Screen::NewProject, window, cx))))
            .child(div().h(px(6.)))
            .child(button("close-project", tr("Close"), false, 0).on_click(cx.listener(|d, _, _, cx| d.scx.close(cx))));
        div()
            .id("project-dialog")
            .w(px(w))
            .h(px(h))
            .flex()
            .rounded(px(10.))
            .overflow_hidden()
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(nav)
            .child(div().id("project-body").flex_1().min_w_0().overflow_y_scroll().p(px(20.)).child(body))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::Screen;
    use super::super::palette::testing::*;
    use super::*;

    /// Waits (real time: the fake's workers are threads) until the project's operations are done, at most 10 s
    fn wait_operation(cx: &mut gpui::VisualTestContext, core: &gpui::Entity<crate::core::Core>) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            std::thread::sleep(Duration::from_millis(50));
            cx.executor().advance_clock(Duration::from_millis(20));
            cx.run_until_parked();
            let busy = core.read_with(cx, |c, _| c.port().loading(&Target::Project) != fake_core::Loading::Ready);
            if !busy || std::time::Instant::now() > deadline {
                break;
            }
        }
    }

    #[test]
    fn versions_count_up() {
        assert_eq!(next_version("Música"), "Música v2");
        assert_eq!(next_version("Música v2"), "Música v3");
    }

    /// Opening a project another program holds: read-only (H10), and Save as makes it writable again
    #[gpui::test]
    fn locked_project_opens_read_only_and_save_as_frees_it(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        open_screen(&ws, cx, Screen::Project);
        click(cx, "project-panel-Open");
        let i = core.read_with(cx, |c, _| c.port().services().recent.iter().position(|r| r.name == "Podcast 12").unwrap());
        let sel: &'static str = Box::leak(format!("recent-{i}").into_boxed_str());
        click(cx, sel);
        click(cx, "open-recent");
        wait_operation(cx, &core);
        core.read_with(cx, |c, _| assert!(c.port().services().project.read_only(), "locked elsewhere: read-only"));
        click(cx, "ro-save-as");
        click(cx, "save-as-go");
        wait_operation(cx, &core);
        core.read_with(cx, |c, _| {
            let p = &c.port().services().project;
            assert!(!p.read_only());
            assert!(!p.modified);
        });
    }

    /// After a crash while recording, each interrupted take is recovered or left (U5)
    #[gpui::test]
    fn recover_interrupted_takes(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        let i = core.read_with(cx, |c, _| c.port().services().recent.iter().position(|r| r.name == "Ensaio de sábado").unwrap());
        port_action_test(&ws, cx, "project.open_recent", &i.to_string());
        wait_operation(cx, &core);
        open_screen(&ws, cx, Screen::Project);
        click(cx, "recover-0");
        core.read_with(cx, |c, _| assert_eq!(c.port().services().project.recovery[0].decision, Some(true)));
    }

    fn port_action_test(ws: &gpui::Entity<crate::workspace::Workspace>, cx: &mut gpui::VisualTestContext, id: &str, arg: &str) {
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        core.update(cx, |c, _| {
            let _ = c.port_mut().run_action(id, Value::Text(arg.to_string()));
        });
    }
}
