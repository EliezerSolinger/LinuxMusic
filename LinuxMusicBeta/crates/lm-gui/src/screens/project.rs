//! Owner: agent D. The project dialogs, kept as small as the legacy's (its file dialogs, "save as new version" and
//! the recovery message box), for the PROJECT FOLDER (`projeto.json`, `audio/`, `backup/`):
//! - Open (F-PRJ-003, F-PRJ-015): the recent projects and a folder or a `.zip` by its path; opening is an operation
//!   with the shared loading component;
//! - Save as (F-PRJ-007, F-PRJ-008): the folder, with the legacy's version buttons (− +);
//! - the project's state when it needs an answer: read-only because another program holds the lock or a newer
//!   version saved it (H10, H11), with "Save as"; the takes a crash interrupted, recovered or discarded (U5, as the
//!   legacy's "Recover / Discard"); safe mode with the plugin that stopped the program off (U21); and the three file
//!   commands that have no other place without a menu bar: export as a single file (.zip), save as a template, clean
//!   the unused files (H4, P-13, after a warning).
//!
//! Everything goes through the port: `status()` (lm_port::trees) and the session's actions.

use gpui::{AnyElement, Context, Entity, FontWeight, Render, Subscription, Window, div, prelude::*, px};
use gpui_component::input::{Input, InputEvent, InputState};
use lm_port::{Target, Value};

use super::ScreenCx;
use super::palette::take_screen_arg;
use super::settings::{form_row, modal_size, note, obj, port_action, target_loading};
use crate::ui::{THEME, button, c, caption, tbutton, tr, trf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Open,
    SaveAs,
    State,
    /// Import a MIDI or audio file by its path (a remote core: the server's files, I-15)
    Import,
    /// Export MIDI to a path (the same)
    ExportMidi,
}

pub struct ProjectDialog {
    scx: ScreenCx,
    part: Part,
    chosen: usize,
    path: Entity<InputState>,
    folder: Entity<InputState>,
    confirm_clean: bool,
    _subs: Vec<Subscription>,
}

/// The next or previous version's name (F-PRJ-008): "Música" → "Música-01" → "Música-02"
pub fn versioned(folder: &str, step: i64) -> String {
    let folder = folder.trim_end_matches('/');
    let (base, n) = match folder.rsplit_once('-') {
        Some((b, n)) if n.len() == 2 && n.chars().all(|c| c.is_ascii_digit()) => (b, n.parse::<i64>().unwrap_or(0)),
        _ => (folder, 0),
    };
    let next = (n + step).max(0);
    if next == 0 { base.to_string() } else { format!("{base}-{next:02}") }
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_text).unwrap_or("").to_string()
}

impl ProjectDialog {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let part = match take_screen_arg(cx) {
            Some("open") => Part::Open,
            Some("save_as") => Part::SaveAs,
            Some("import") => Part::Import,
            Some("export_midi") => Part::ExportMidi,
            _ => Part::State,
        };
        let status = scx.core.read(cx).port().status().clone();
        let folder_v = status.get("project").map(|p| text(p, "folder")).unwrap_or_default();
        let path = cx.new(|cx| InputState::new(window, cx).placeholder(tr("/path/to/a/project or a .zip")));
        let folder = cx.new(|cx| InputState::new(window, cx).default_value(folder_v));
        let subs = vec![
            cx.subscribe(&path, |d: &mut ProjectDialog, st, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    let v = st.read(cx).value().to_string();
                    d.open_folder(v, cx);
                }
            }),
            cx.subscribe(&folder, |_: &mut ProjectDialog, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        ProjectDialog { scx, part, chosen: 0, path, folder, confirm_clean: false, _subs: subs }
    }

    fn status(&self, cx: &Context<Self>) -> Value {
        self.scx.core.read(cx).port().status().clone()
    }

    fn open_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        if folder.trim().is_empty() {
            return;
        }
        port_action(&self.scx, "project.open", obj(vec![("folder", Value::text(folder.trim()))]), cx);
        self.part = Part::State;
        cx.notify();
    }

    fn save_as(&mut self, cx: &mut Context<Self>) {
        let folder = self.folder.read(cx).value().trim().to_string();
        if !folder.is_empty() {
            port_action(&self.scx, "project.save_as", obj(vec![("folder", Value::text(folder))]), cx);
            self.part = Part::State;
            cx.notify();
        }
    }

    fn part_open(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let recent: Vec<Value> =
            self.status(cx).get("recent").map(|l| l.items().cloned().collect()).unwrap_or_default();
        let rows = recent.iter().enumerate().map(|(i, r)| {
            let folder = text(r, "folder");
            div()
                .id(("recent", i))
                .debug_selector(move || format!("recent-{i}"))
                .flex()
                .gap(px(10.))
                .px(px(8.))
                .py(px(4.))
                .when(i == self.chosen, |d| d.bg(c(THEME.bg4)))
                .cursor_pointer()
                .child(div().w(px(180.)).text_size(px(12.)).child(text(r, "name")))
                .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(c(THEME.text3)).child(folder.clone()))
                .child(div().w(px(60.)).text_size(px(11.)).text_color(c(THEME.text3)).child(text(r, "opened")))
                .on_click(cx.listener(move |d, ev: &gpui::ClickEvent, _, cx| {
                    d.chosen = i;
                    if ev.click_count() >= 2 {
                        d.open_folder(folder.clone(), cx);
                    }
                    cx.notify();
                }))
        });
        let chosen = recent.get(self.chosen).map(|r| text(r, "folder")).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(caption(tr("Recent projects")))
            .child(div().flex().flex_col().border_1().border_color(c(THEME.line)).children(rows))
            .child(form_row(
                tr("Folder or .zip"),
                div()
                    .flex()
                    .gap(px(4.))
                    .child(div().flex_1().child(crate::ui::text_entry(Input::new(&self.path))))
                    .children(self.choose_button("choose-open", true, cx)),
            ))
            .child(div().flex().justify_end().gap(px(8.)).child(
                tbutton("open-recent", tr("Open"), true, THEME.green).debug_selector(|| "open-recent".into()).on_click(
                    cx.listener(move |d, _, _, cx| {
                        let typed = d.path.read(cx).value().trim().to_string();
                        d.open_folder(if typed.is_empty() { chosen.clone() } else { typed }, cx);
                    }),
                ),
            ))
            .into_any_element()
    }

    /// "Choose…": the system's dialog, when the files are this machine's (a remote core keeps the path field)
    fn choose_button(&self, id: &'static str, open: bool, cx: &mut Context<Self>) -> Option<gpui::Stateful<gpui::Div>> {
        if !super::files::local(&self.scx.core, cx) {
            return None;
        }
        Some(button(id, tr("Choose..."), false, 0).h(px(24.)).flex_none().on_click(cx.listener(move |d, _, _, cx| {
            let me = cx.entity().downgrade();
            if open {
                super::files::choose(cx, "Open project", true, false, move |paths, cx| {
                    if let Some(p) = paths.first() {
                        let _ = me.update(cx, |d, cx| d.open_folder(p.display().to_string(), cx));
                    }
                });
            } else {
                let current = std::path::PathBuf::from(super::palette::expand_home(&d.folder.read(cx).value()));
                let parent = current.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                let name = current.file_name().map_or_else(String::new, |n| n.to_string_lossy().to_string());
                super::files::choose_new(cx, &parent, &name, move |path, cx| {
                    let _ = me.update(cx, |d, cx| {
                        port_action(
                            &d.scx,
                            "project.save_as",
                            obj(vec![("folder", Value::text(path.display().to_string()))]),
                            cx,
                        );
                        d.part = Part::State;
                        cx.notify();
                    });
                });
            }
        })))
    }

    /// Import or Export MIDI by a path typed (the server's files, with a remote core)
    fn part_path(&mut self, import: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(form_row(tr("File"), crate::ui::text_entry(Input::new(&self.path))))
            .child(
                div().flex().justify_end().child(
                    tbutton("path-go", if import { tr("Import") } else { tr("Export") }, true, THEME.green)
                        .debug_selector(|| "path-go".into())
                        .on_click(cx.listener(move |d, _, _, cx| {
                            let path = std::path::PathBuf::from(d.path.read(cx).value().trim().to_string());
                            if path.as_os_str().is_empty() {
                                return;
                            }
                            let core = d.scx.core.clone();
                            if import {
                                let tick = {
                                    let p = core.read(cx).port();
                                    p.tempo_map().tick_at(p.telemetry().position.samples)
                                };
                                super::files::import_paths(&core, &[path], tick, cx);
                            } else {
                                port_action(
                                    &d.scx,
                                    "midi.export",
                                    obj(vec![("path", Value::text(path.display().to_string()))]),
                                    cx,
                                );
                            }
                            d.scx.close(cx);
                        })),
                ),
            )
            .into_any_element()
    }

    fn part_save_as(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(form_row(
                tr("Project folder"),
                div()
                    .flex()
                    .gap(px(4.))
                    .child(div().flex_1().child(crate::ui::text_entry(Input::new(&self.folder))))
                    .children(self.choose_button("choose-save", false, cx))
                    .child(button("version-down", "−", false, 0).h(px(24.)).w(px(24.)).on_click(cx.listener(
                        |d, _, window, cx| {
                            let v = versioned(&d.folder.read(cx).value(), -1);
                            d.folder.update(cx, |st, cx| st.set_value(v, window, cx));
                        },
                    )))
                    .child(
                        button("version-up", "+", false, 0)
                            .h(px(24.))
                            .w(px(24.))
                            .debug_selector(|| "version-up".into())
                            .on_click(cx.listener(|d, _, window, cx| {
                                let v = versioned(&d.folder.read(cx).value(), 1);
                                d.folder.update(cx, |st, cx| st.set_value(v, window, cx));
                            })),
                    ),
            ))
            .child(
                div().flex().justify_end().child(
                    tbutton("save-as-go", tr("Save"), true, THEME.green)
                        .debug_selector(|| "save-as-go".into())
                        .on_click(cx.listener(|d, _, _, cx| d.save_as(cx))),
                ),
            )
            .into_any_element()
    }

    fn part_state(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let status = self.status(cx);
        let project = status.get("project").cloned().unwrap_or(Value::NULL);
        let read_only = project.get("readOnly").and_then(Value::as_bool).unwrap_or(false);
        let mut col = div().flex().flex_col().gap(px(8.));
        col = col
            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(text(&project, "name")))
            .child(caption(text(&project, "folder")));
        // read-only, and why (H10, H11)
        if read_only {
            let locked = project.get("lockedBy").filter(|v| !v.is_null());
            let newer = project.get("newerVersion").filter(|v| !v.is_null());
            let why = match (locked, newer) {
                (Some(l), _) => trf(
                    "Open on {0} (process {1}, since {2}): read-only",
                    &[&text(l, "host"), &l.get("pid").and_then(Value::as_int).unwrap_or(0), &text(l, "since")],
                ),
                (None, Some(n)) => trf("Saved by LinuxMusic {0}: read-only", &[&text(n, "version")]),
                _ => tr("It opened read-only").to_string(),
            };
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(note(why, THEME.orange)))
                    .child(
                        tbutton("ro-save-as", tr("Save as…"), true, THEME.green)
                            .flex_none()
                            .debug_selector(|| "ro-save-as".into())
                            .on_click(cx.listener(|d, _, _, cx| {
                                d.part = Part::SaveAs;
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(v) = project.get("migratedFrom").and_then(Value::as_int) {
            col = col.child(note(trf("Converted from format {0}; the original is kept", &[&v]), THEME.blue));
        }
        // takes a crash interrupted (U5): the legacy's "Recover / Discard", per take
        let takes: Vec<Value> =
            status.get("interruptedTakes").map(|l| l.items().cloned().collect()).unwrap_or_default();
        if !takes.is_empty() {
            col = col
                .child(div().text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr("Takes interrupted by a crash")))
                .children(takes.iter().enumerate().map(|(i, t)| {
                    let secs = t.get("seconds").and_then(Value::as_real).unwrap_or(0.0);
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().min_w_0().text_size(px(12.)).child(trf(
                            "{0} · {1} · from bar {2} · {3} s",
                            &[
                                &text(t, "track"),
                                &text(t, "input"),
                                &t.get("bar").and_then(Value::as_int).unwrap_or(1),
                                &format!("{secs:.1}"),
                            ],
                        )))
                        .child(
                            button(("recover", i), tr("Recover"), false, 0)
                                .flex_none()
                                .debug_selector(move || format!("recover-{i}"))
                                .on_click(cx.listener(move |d, _, _, cx| {
                                    port_action(
                                        &d.scx,
                                        "takes.recover",
                                        obj(vec![("index", Value::int(i as i64))]),
                                        cx,
                                    );
                                    cx.notify();
                                })),
                        )
                        .child(button(("discard", i), tr("Discard"), false, 0).flex_none().on_click(cx.listener(
                            move |d, _, _, cx| {
                                port_action(&d.scx, "takes.discard", obj(vec![("index", Value::int(i as i64))]), cx);
                                cx.notify();
                            },
                        )))
                }));
        }
        // safe mode (U21)
        let safe: Vec<Value> = status.get("safeModePlugins").map(|l| l.items().cloned().collect()).unwrap_or_default();
        if !safe.is_empty() {
            col = col.child(note(tr("Safe mode: these plugins are off"), THEME.yellow)).children(
                safe.iter().enumerate().map(|(i, p)| {
                    let node = text(p, "node");
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .child(trf("{0}: {1}", &[&text(p, "plugin"), &tr_reason(&text(p, "reason"))])),
                        )
                        .child(
                            button(("safe", i), tr("Turn on"), false, 0)
                                .debug_selector(move || format!("safe-{i}"))
                                .on_click(cx.listener(move |d, _, _, cx| {
                                    port_action(
                                        &d.scx,
                                        "safe_mode.enable",
                                        obj(vec![("node", Value::text(node.clone()))]),
                                        cx,
                                    );
                                    cx.notify();
                                })),
                        )
                }),
            );
        }
        // the file commands without another place
        let unused = project.get("unused").cloned().unwrap_or(Value::NULL);
        let files = unused.get("files").and_then(Value::as_int).unwrap_or(0);
        let mb = unused.get("mb").and_then(Value::as_real).unwrap_or(0.0);
        let zip_running = self.scx.core.read(cx).port().activity().iter().any(|a| a.label.key == "Packing the project");
        col = col.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .pt(px(6.))
                .border_t_1()
                .border_color(c(THEME.line))
                .child(
                    button("zip", tr("Export as a single file (.zip)"), zip_running, THEME.green)
                        .debug_selector(|| "zip".into())
                        .when(!zip_running, |b| {
                            b.on_click(
                                cx.listener(|d, _, _, cx| port_action(&d.scx, "project.export_zip", Value::NULL, cx)),
                            )
                        }),
                )
                .child(button("save-template", tr("Save as a template"), false, 0).when(!read_only, |b| {
                    b.on_click(cx.listener(|d, _, _, cx| port_action(&d.scx, "project.save_template", Value::NULL, cx)))
                }))
                .when(files > 0, |d| {
                    d.child(
                        button("clean", trf("Clean the unused files ({0} MB)…", &[&format!("{mb:.0}")]), false, 0)
                            .debug_selector(|| "clean".into())
                            .on_click(cx.listener(|d, _, _, cx| {
                                d.confirm_clean = true;
                                cx.notify();
                            })),
                    )
                }),
        );
        if self.confirm_clean && files > 0 {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(note(
                        trf("{0} files go to the trash; the undo history is cleared", &[&files]),
                        THEME.orange,
                    )))
                    .child(button("clean-cancel", tr("Cancel"), false, 0).flex_none().on_click(cx.listener(
                        |d, _, _, cx| {
                            d.confirm_clean = false;
                            cx.notify();
                        },
                    )))
                    .child(
                        button("clean-yes", tr("Move to the trash"), true, THEME.red)
                            .flex_none()
                            .debug_selector(|| "clean-yes".into())
                            .on_click(cx.listener(|d, _, _, cx| {
                                d.confirm_clean = false;
                                port_action(&d.scx, "project.clean_unused", Value::NULL, cx);
                            })),
                    ),
            );
        }
        col.into_any_element()
    }
}

/// The reason safe mode gives (an English key from the core)
fn tr_reason(r: &str) -> String {
    match r {
        "stopped answering" => tr("stopped answering").to_string(),
        "crashed" => tr("crashed").to_string(),
        other => other.to_string(),
    }
}

impl Render for ProjectDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = modal_size(window, 620.0, 480.0);
        let loading = target_loading(&self.scx, "project-loading", Target::Project, cx);
        let (title, body) = match self.part {
            Part::Open => (tr("Open project"), self.part_open(cx)),
            Part::SaveAs => (tr("Save as"), self.part_save_as(cx)),
            Part::State => (tr("Project"), self.part_state(cx)),
            Part::Import => (tr("Import (MIDI, audio)"), self.part_path(true, cx)),
            Part::ExportMidi => (tr("Export MIDI"), self.part_path(false, cx)),
        };
        div()
            .id("project-dialog")
            .w(px(w))
            .max_h(px(h))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(16.))
                    .py(px(10.))
                    .bg(c(THEME.bg1))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(div().flex_1().text_size(px(15.)).font_weight(FontWeight::BOLD).child(title))
                    .child(
                        button("close-project", tr("Close"), false, 0)
                            .on_click(cx.listener(|d, _, _, cx| d.scx.close(cx))),
                    ),
            )
            .children(loading.map(|l| div().h(px(30.)).px(px(16.)).child(l)))
            .child(div().id("project-body").flex_1().overflow_y_scroll().px(px(16.)).py(px(12.)).child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::versioned;

    #[test]
    fn versions_count_up_and_down() {
        assert_eq!(versioned("~/M/Música", 1), "~/M/Música-01");
        assert_eq!(versioned("~/M/Música-01", 1), "~/M/Música-02");
        assert_eq!(versioned("~/M/Música-01", -1), "~/M/Música");
    }
}
