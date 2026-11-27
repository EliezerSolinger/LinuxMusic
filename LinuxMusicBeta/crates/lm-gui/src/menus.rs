//! The menu bar (F-JAN-020..024): File, Edit, View, Tools and Help with the legacy main window's items and shortcuts.
//! The menus are data (`MENUS`: registry ids), so each item is an action of the registry and shows the keys the
//! registry (and the user's preferences) give it. On macOS the system's menu bar is set (`install_native`); elsewhere
//! the workspace draws the bar in the window (`render_bar`). Both dispatch the GPUI action, the same path as the keys.

use gpui::{App, Context, Menu, MenuItem, Window, deferred, div, prelude::*, px};
use lm_port::Value;

use crate::actions::{self, ActionDef};
use crate::ui::{THEME, c, tr};
use crate::workspace::Workspace;

/// Height of the in-window bar
pub const BAR_H: f32 = 24.0;

#[derive(Clone, Copy, Debug)]
pub enum Entry {
    /// An action of the registry, by id
    Action(&'static str),
    /// A submenu of what the project has: the recent projects, the templates (the in-window bar)
    Submenu(Dynamic),
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dynamic {
    Recent,
    Templates,
}

impl Dynamic {
    fn title(self) -> &'static str {
        match self {
            Dynamic::Recent => "Recent projects",
            Dynamic::Templates => "New from template",
        }
    }

    /// The items: (shown name, folder) from the core's status
    fn items(self, status: &Value) -> Vec<(String, String)> {
        let key = if self == Dynamic::Recent { "recent" } else { "templates" };
        status
            .get(key)
            .map(|l| {
                l.items()
                    .map(|e| {
                        let t = |k: &str| e.get(k).and_then(Value::as_text).unwrap_or("").to_string();
                        (t("name"), t("folder"))
                    })
                    .filter(|(_, f)| !f.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}

pub struct MenuDef {
    /// English key of the title
    pub title: &'static str,
    pub entries: &'static [Entry],
}

use Entry::{Action, Separator, Submenu};

pub const MENUS: &[MenuDef] = &[
    MenuDef {
        title: "File",
        entries: &[
            Action("file.new"),
            Submenu(Dynamic::Templates),
            Action("file.open"),
            Submenu(Dynamic::Recent),
            Separator,
            Action("project.save"),
            Action("file.save_as"),
            Action("file.save_version"),
            Action("file.save_template"),
            Separator,
            Action("file.import"),
            Action("file.export"),
            Action("file.export_midi"),
            Separator,
            Action("app.quit"),
        ],
    },
    MenuDef { title: "Edit", entries: &[Action("edit.undo"), Action("edit.redo"), Separator, Action("view.settings")] },
    MenuDef {
        title: "View",
        entries: &[
            Action("view.composition"),
            Action("view.mixing"),
            Action("view.midi_maps"),
            Separator,
            Action("view.piano_roll"),
            Action("view.automation"),
            Action("view.steps"),
            Action("view.browser"),
            Separator,
            Action("view.fullscreen"),
            Action("view.detach"),
            Action("view.detach_all"),
            Action("view.dock_all"),
        ],
    },
    MenuDef { title: "Tools", entries: &[Action("tools.tap_tempo")] },
    MenuDef { title: "Help", entries: &[Action("help.manual"), Action("help.about")] },
];

/// An item's text: the registry's label translated
fn label(def: &ActionDef) -> &'static str {
    tr(def.label)
}

/// The macOS menu bar: the system's own, set once at the start
#[cfg(target_os = "macos")]
pub fn install_native(cx: &mut App) {
    let menus = MENUS
        .iter()
        .map(|m| Menu {
            name: tr(m.title).into(),
            disabled: false,
            items: m
                .entries
                .iter()
                .filter_map(|e| match e {
                    Separator => Some(MenuItem::Separator),
                    // the system's bar is static: the recent projects and the templates are in the Open and New dialogs
                    Submenu(_) => None,
                    Action(id) => {
                        let def = actions::find(id)?;
                        let action = cx.build_action(def.gpui, None).ok()?;
                        Some(MenuItem::Action {
                            name: label(def).into(),
                            action,
                            os_action: None,
                            checked: false,
                            disabled: false,
                        })
                    }
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    cx.set_menus(menus);
}

#[cfg(not(target_os = "macos"))]
pub fn install_native(_cx: &mut App) {}

/// Whether the window draws its own bar
pub const fn in_window() -> bool {
    !cfg!(target_os = "macos")
}

/// The ids every entry of the menus names, which the registry must have (the tests check it)
#[cfg(test)]
pub fn all_ids() -> impl Iterator<Item = &'static str> {
    MENUS.iter().flat_map(|m| m.entries.iter()).filter_map(|e| if let Action(id) = e { Some(*id) } else { None })
}

impl Workspace {
    /// The bar in the window (Linux): the titles, and under the open one its items with their keys
    pub fn render_bar(&self, prefs: &Value, cx: &Context<Self>) -> impl IntoElement {
        let open = self.menu_open;
        div()
            .id("menu-bar")
            .debug_selector(|| "menu-bar".into())
            .h(px(BAR_H))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .px(px(6.))
            .bg(c(THEME.bg1))
            .border_b_1()
            .border_color(c(THEME.line))
            .children(MENUS.iter().enumerate().map(|(i, m)| {
                let on = open == Some(i);
                let title = div()
                    .id(("menu-title", i))
                    .debug_selector(move || format!("menu-{i}"))
                    .px(px(10.))
                    .h_full()
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(on, |d| d.bg(c(THEME.bg3)))
                    .hover(|s| s.bg(c(THEME.bg3)))
                    .child(tr(m.title))
                    .on_click(cx.listener(move |ws, _, _, cx| {
                        ws.menu_open = if ws.menu_open == Some(i) { None } else { Some(i) };
                        cx.notify();
                    }))
                    .on_hover(cx.listener(move |ws, hovered: &bool, _, cx| {
                        if *hovered && ws.menu_open.is_some() && ws.menu_open != Some(i) {
                            ws.menu_open = Some(i);
                            cx.notify();
                        }
                    }));
                div().relative().h_full().child(title).when(on, |d| {
                    d.child(deferred(
                        div()
                            .id(("menu-list", i))
                            .absolute()
                            .top(px(BAR_H))
                            .left(px(0.))
                            .min_w(px(220.))
                            .py(px(4.))
                            .flex()
                            .flex_col()
                            .rounded(px(6.))
                            .bg(c(THEME.bg2))
                            .border_1()
                            .border_color(c(THEME.line2))
                            .shadow_lg()
                            .occlude()
                            .children(m.entries.iter().enumerate().map(|(k, e)| match e {
                                Separator => div().h(px(1.)).my(px(4.)).bg(c(THEME.line)).into_any_element(),
                                Submenu(kind) => self.render_submenu(*kind, i * 100 + k, cx),
                                Action(id) => {
                                    let Some(def) = actions::find(id) else { return div().into_any_element() };
                                    let keys = actions::keys_of(def, prefs)
                                        .first()
                                        .map(|k| crate::screens::palette::keys_label(&actions::keys_for_platform(k)))
                                        .unwrap_or_default();
                                    let id: &'static str = id;
                                    div()
                                        .id(("menu-item", i * 100 + k))
                                        .debug_selector(move || id.to_string())
                                        .px(px(12.))
                                        .py(px(4.))
                                        .flex()
                                        .flex_row()
                                        .gap(px(24.))
                                        .justify_between()
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(c(THEME.bg3)))
                                        .on_hover(cx.listener(|ws, hovered: &bool, _, cx| {
                                            if *hovered && ws.menu_sub.take().is_some() {
                                                cx.notify();
                                            }
                                        }))
                                        .child(label(def))
                                        .child(div().text_color(c(THEME.text3)).child(keys))
                                        .on_click(cx.listener(move |ws, _, window, cx| {
                                            ws.menu_open = None;
                                            ws.run_registry_action(id, window, cx);
                                        }))
                                        .into_any_element()
                                }
                            })),
                    ))
                })
            }))
    }

    /// A submenu row of the bar: its title with an arrow, and beside it, while the pointer is on it, the items
    fn render_submenu(&self, kind: Dynamic, id: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let items = kind.items(self.core.read(cx).port().status());
        let open = self.menu_sub == Some(kind);
        div()
            .id(("menu-sub", id))
            .debug_selector(move || format!("submenu-{kind:?}"))
            .relative()
            .px(px(12.))
            .py(px(4.))
            .flex()
            .flex_row()
            .justify_between()
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|s| s.bg(c(THEME.bg3)))
            .child(tr(kind.title()))
            .child(div().text_color(c(THEME.text3)).child("›"))
            .on_hover(cx.listener(move |ws, hovered: &bool, _, cx| {
                if *hovered && ws.menu_sub != Some(kind) {
                    ws.menu_sub = Some(kind);
                    cx.notify();
                }
            }))
            .when(open, |d| {
                d.child(deferred(
                    div()
                        .id(("menu-sub-list", id))
                        .absolute()
                        .top(px(0.))
                        .left(gpui::relative(1.0))
                        .min_w(px(220.))
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .py(px(4.))
                        .flex()
                        .flex_col()
                        .rounded(px(6.))
                        .bg(c(THEME.bg2))
                        .border_1()
                        .border_color(c(THEME.line2))
                        .shadow_lg()
                        .occlude()
                        .when(items.is_empty(), |d| {
                            d.child(
                                div()
                                    .px(px(12.))
                                    .py(px(4.))
                                    .text_size(px(12.))
                                    .text_color(c(THEME.text3))
                                    .child(tr("None")),
                            )
                        })
                        .children(items.into_iter().enumerate().map(move |(n, (name, folder))| {
                            div()
                                .id(("menu-sub-item", id * 1000 + n))
                                .px(px(12.))
                                .py(px(4.))
                                .text_size(px(12.))
                                .cursor_pointer()
                                .hover(|s| s.bg(c(THEME.bg3)))
                                .child(name)
                                .on_click(cx.listener(move |ws, _, window, cx| {
                                    ws.menu_open = None;
                                    ws.menu_sub = None;
                                    match kind {
                                        Dynamic::Recent => ws.core.update(cx, |core, _| {
                                            core.run_guarded(
                                                "project.open",
                                                Value::object([("folder", Value::text(folder.as_str()))]),
                                            )
                                        }),
                                        Dynamic::Templates => {
                                            cx.set_global(crate::screens::new_project::TemplateArg(Some(
                                                folder.clone(),
                                            )));
                                            ws.open(crate::screens::Screen::NewProject, None, window, cx);
                                        }
                                    }
                                    cx.notify();
                                }))
                        })),
                ))
            })
            .into_any_element()
    }

    /// Runs an action of the registry as if its keys were pressed
    pub fn run_registry_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(def) = actions::find(id)
            && let Ok(action) = cx.build_action(def.gpui, None)
        {
            window.dispatch_action(action, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_is_an_action_of_the_registry() {
        for id in all_ids() {
            assert!(actions::find(id).is_some(), "{id} is not in the registry");
        }
    }

    #[test]
    fn the_legacy_menus_are_all_there() {
        let titles: Vec<_> = MENUS.iter().map(|m| m.title).collect();
        assert_eq!(titles, ["File", "Edit", "View", "Tools", "Help"]);
        let ids: Vec<_> = all_ids().collect();
        for must in [
            "file.new",
            "file.open",
            "project.save",
            "file.save_as",
            "file.save_version",
            "file.export",
            "app.quit",
            "edit.undo",
            "edit.redo",
            "view.settings",
            "tools.tap_tempo",
            "help.manual",
            "help.about",
        ] {
            assert!(ids.contains(&must), "{must} missing");
        }
    }

    #[test]
    fn no_item_repeats() {
        let mut ids: Vec<_> = all_ids().collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
}
