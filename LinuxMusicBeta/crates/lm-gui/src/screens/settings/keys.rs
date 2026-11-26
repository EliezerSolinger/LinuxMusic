//! The Shortcuts page (design 08's list-and-rows layout): the contexts on the left, each with the preset chosen for
//! it; the actions of the selected context on the right, with their keys (tab Keys) or their mouse gestures (tab
//! Mouse). A key is changed by pressing it, a gesture by doing it on the box that asks for it; a key that is already
//! another action's in the same context (or in Global) asks for a second press before it is taken. "Find key" looks
//! for the actions that have a pressed key. The user's changes live in the preferences on top of the chosen preset
//! (`keymap`), so a preset's update never erases them; "Restore" drops the changes of the context.
//!
//! This file reads the keyboard and the pointer on purpose: it is the capture of the keys and gestures to bind.

use gpui::{MouseButton, MouseDownEvent, ScrollEvent};
use gpui_component::Sizable;

use super::*;
use crate::actions::{ActionDef, find};
use crate::keymap::{self, Btn, CONTEXTS, Gesture, Kind, MOUSE, MouseDef, Preset, presets};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Capture {
    Key(&'static str),
    Mouse(&'static str),
    /// "Find key": the next key pressed filters the list
    Find,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeysTab {
    Keys,
    Mouse,
}

/// A gesture as the page shows it ("Ctrl + Shift + Drag")
pub(super) fn gesture_label(g: &Gesture) -> String {
    let mac = cfg!(target_os = "macos");
    let mut parts: Vec<String> = Vec::new();
    if g.secondary {
        parts.push(if mac { "Cmd".into() } else { tr("Ctrl").to_string() });
    }
    if g.alt {
        parts.push(tr("Alt").to_string());
    }
    if g.shift {
        parts.push(tr("Shift").to_string());
    }
    parts.push(
        tr(match (g.btn, g.kind) {
            (_, Kind::Wheel) => "Wheel",
            (Btn::Left, Kind::Click) => "Click",
            (Btn::Left, Kind::Drag) => "Drag",
            (Btn::Left, Kind::Double) => "Double click",
            (Btn::Right, Kind::Drag) => "Right drag",
            (Btn::Right, _) => "Right click",
            (Btn::Middle, Kind::Drag) => "Middle drag",
            (Btn::Middle, _) => "Middle click",
        })
        .to_string(),
    );
    parts.join(" + ")
}

fn preset_title(name: &str) -> String {
    if name == keymap::DEFAULT { tr("Default").to_string() } else { name.to_string() }
}

impl Settings {
    fn config_dir(&self, cx: &App) -> Option<std::path::PathBuf> {
        self.scx.core.read(cx).port().config_dir()
    }

    /// The context's node of the preferences, written, with the keys and gestures bound again
    fn write_ctx(&mut self, ctx: &str, node: Value, cx: &mut Context<Self>) {
        self.set(cx, &["keymaps", ctx], node);
        self.rebind(cx);
    }

    fn reload_user_presets(&mut self, cx: &App) {
        if let Some(dir) = self.config_dir(cx) {
            keymap::set_user_presets(presets::load_user(&dir));
        }
    }

    fn choose_preset(&mut self, ctx: &str, name: &str, cx: &mut Context<Self>) {
        let node = keymap::with_preset(self.prefs(cx), ctx, name);
        self.write_ctx(ctx, node, cx);
        self.kmenu = None;
        cx.notify();
    }

    // --- the page -----------------------------------------------------------------------------------------------

    pub(super) fn keys(&mut self, cx: &mut Context<Self>) -> Vec<Row> {
        let prefs = self.prefs(cx).clone();
        let user = keymap::user_presets();
        let query = fold(self.ksearch.read(cx).value().trim());
        let list = self.context_list(&prefs, &user, cx);
        let rows = match self.ktab {
            KeysTab::Keys => self.action_rows(&prefs, &user, &query, cx),
            KeysTab::Mouse => self.mouse_rows(&prefs, &user, &query, cx),
        };
        let toolbar = self.toolbar(&prefs, &user, cx);
        let right =
            div().flex_1().min_w_0().flex().flex_col().gap(px(3.)).child(toolbar).child(div().h(px(6.))).children(rows);
        vec![wide("Contexts", Some(Apply::Now), div().flex().gap(px(16.)).child(list).child(right))]
    }

    fn context_list(&mut self, prefs: &Value, user: &[Preset], cx: &mut Context<Self>) -> Div {
        let selected = self.kctx;
        div().w(px(230.)).flex_none().flex().flex_col().gap(px(2.)).children(CONTEXTS.iter().enumerate().map(
            |(i, c)| {
                let id = c.id;
                let on = id == selected;
                let name = keymap::chosen(prefs, id).to_string();
                let open = self.kmenu == Some(id);
                let options: Vec<String> =
                    std::iter::once(keymap::DEFAULT.to_string()).chain(presets::names(id, user)).collect();
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(6.))
                            .when(on, |d| d.bg(c_bg()))
                            .child(
                                div()
                                    .id(("kctx", i))
                                    .debug_selector(move || format!("kctx-{i}"))
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(13.))
                                    .when(on, |d| d.font_weight(FontWeight::BOLD))
                                    .cursor_pointer()
                                    .child(format!(
                                        "{}{}",
                                        tr(c.label),
                                        if keymap::has_changes(prefs, id) { " •" } else { "" }
                                    ))
                                    .on_click(cx.listener(move |s, _, _, cx| {
                                        s.kctx = id;
                                        s.capturing = None;
                                        s.pending_keys = None;
                                        s.kmenu = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                button(("kpreset", i), format!("{} ▾", preset_title(&name)), open, THEME.green)
                                    .debug_selector(move || format!("kpreset-{i}"))
                                    .on_click(cx.listener(move |s, _, _, cx| {
                                        s.kmenu = if s.kmenu == Some(id) { None } else { Some(id) };
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(open, |d| {
                        d.child(div().flex().flex_col().gap(px(2.)).pl(px(14.)).py(px(2.)).children(
                            options.into_iter().enumerate().map(|(j, o)| {
                                let on = o == name;
                                let target = o.clone();
                                button(("kpreset-option", i * 100 + j), preset_title(&o), on, THEME.green)
                                    .debug_selector(move || format!("kpreset-option-{i}-{j}"))
                                    .on_click(cx.listener(move |s, _, _, cx| s.choose_preset(id, &target, cx)))
                            }),
                        ))
                    })
            },
        ))
    }

    fn toolbar(&mut self, prefs: &Value, user: &[Preset], cx: &mut Context<Self>) -> Div {
        let ctx = self.kctx;
        let name = keymap::chosen(prefs, ctx).to_string();
        let is_user = user.iter().any(|p| p.ctx == ctx && p.name == name);
        let finding = self.capturing == Some(Capture::Find);
        let find_label = match (&self.kfind, finding) {
            (_, true) => tr("Press a key…").to_string(),
            (Some(k), _) => format!("{}  ×", keys_label(k)),
            (None, _) => tr("Find key").to_string(),
        };
        let tabs = segmented(
            "ktab",
            vec![
                (tr("Keys").into(), self.ktab == KeysTab::Keys, true),
                (tr("Mouse").into(), self.ktab == KeysTab::Mouse, true),
            ],
            cx,
            |s, i, _, cx| {
                s.ktab = if i == 0 { KeysTab::Keys } else { KeysTab::Mouse };
                s.capturing = None;
                s.pending_keys = None;
                cx.notify();
            },
        );
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(tabs)
                    .child(div().w(px(180.)).child(crate::ui::text_entry(Input::new(&self.ksearch).small())))
                    .child(button("kfind", find_label, finding || self.kfind.is_some(), THEME.green).on_click(
                        cx.listener(|s, _, window, cx| {
                            if s.kfind.take().is_none() {
                                s.capturing = Some(Capture::Find);
                                s.pending_keys = None;
                                window.focus(&s.focus, cx);
                            } else {
                                s.capturing = None;
                            }
                            cx.notify();
                        }),
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        button("krestore", tr("Restore the preset"), false, 0)
                            .when(!keymap::has_changes(prefs, ctx), |b| b.opacity(0.4))
                            .on_click(cx.listener(move |s, _, _, cx| {
                                let node = keymap::restored(s.prefs(cx), ctx);
                                s.write_ctx(ctx, node, cx);
                            })),
                    )
                    .child(div().w(px(130.)).child(crate::ui::text_entry(Input::new(&self.kname).small())))
                    .child(
                        button("ksave", tr("Save as preset"), false, 0)
                            .on_click(cx.listener(move |s, _, _, cx| s.save_preset(ctx, cx))),
                    )
                    .child(
                        button("kimport", tr("Import"), false, 0)
                            .on_click(cx.listener(move |s, _, _, cx| s.import_preset(ctx, cx))),
                    )
                    .child(
                        button("kexport", tr("Export"), false, 0)
                            .on_click(cx.listener(move |s, _, _, cx| s.export_preset(ctx, cx))),
                    )
                    .when(is_user, |d| {
                        d.child(
                            button("kdelete", tr("Delete"), false, 0)
                                .on_click(cx.listener(move |s, _, _, cx| s.delete_preset(ctx, cx))),
                        )
                    }),
            )
            .children(self.kstatus.map(|m| caption(tr(m))))
    }

    fn action_rows(&mut self, prefs: &Value, user: &[Preset], query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let ctx = self.kctx;
        let find = self.kfind.clone();
        let mut out = Vec::new();
        for (i, d) in crate::actions::REGISTRY.iter().enumerate().filter(|(_, d)| keymap::ctx_of(d) == ctx) {
            let id = d.id;
            let keys = keymap::keys_for(d, prefs, user);
            if !query.is_empty() && !fold(tr(d.label)).contains(query) {
                continue;
            }
            if let Some(k) = &find
                && !keys.contains(k)
            {
                continue;
            }
            let capturing = self.capturing == Some(Capture::Key(id));
            let changed = keymap::user_change(prefs, ctx, "keys", id).is_some();
            let shown: AnyElement = if capturing {
                match self.conflict_of(d, prefs, user) {
                    Some(o) => {
                        caption(trf("Used by \"{0}\": press again to take them", &[&tr(o.label)])).into_any_element()
                    }
                    None => caption(tr("Press the new keys…")).into_any_element(),
                }
            } else if keys.is_empty() {
                caption(tr("none")).into_any_element()
            } else {
                let text = keys.iter().map(|k| keys_label(k)).collect::<Vec<_>>().join("  ");
                div().text_size(px(12.)).text_color(c(THEME.text2)).child(text).into_any_element()
            };
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(2.))
                    .when(capturing, |d| d.bg(c_bg()))
                    .child(div().flex_1().text_size(px(12.)).child(tr(d.label)))
                    .child(div().w(px(240.)).flex().justify_end().child(shown))
                    .child(
                        button(("key-change", i), tr("Change"), capturing, THEME.green)
                            .debug_selector(move || format!("key-change-{i}"))
                            .on_click(cx.listener(move |s, _, window, cx| {
                                s.capturing = Some(Capture::Key(id));
                                s.pending_keys = None;
                                window.focus(&s.focus, cx);
                                cx.notify();
                            })),
                    )
                    .child(button(("key-clear", i), "×", false, 0).on_click(cx.listener(move |s, _, _, cx| {
                        let node = keymap::with_change(s.prefs(cx), ctx, "keys", id, Some(&[]));
                        s.write_ctx(ctx, node, cx);
                    })))
                    .child(div().w(px(60.)).when(changed, |d| {
                        d.child(button(("key-default", i), tr("Default"), false, 0).on_click(cx.listener(
                            move |s, _, _, cx| {
                                let node = keymap::with_change(s.prefs(cx), ctx, "keys", id, None);
                                s.write_ctx(ctx, node, cx);
                            },
                        )))
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    fn mouse_rows(&mut self, prefs: &Value, user: &[Preset], query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let ctx = self.kctx;
        let mut out = Vec::new();
        for (i, d) in MOUSE.iter().enumerate().filter(|(_, d)| d.ctx == ctx) {
            let id = d.id;
            let gestures = keymap::gestures_for(d, prefs, user);
            if !query.is_empty() && !fold(tr(d.label)).contains(query) && !fold(tr(d.target)).contains(query) {
                continue;
            }
            let capturing = self.capturing == Some(Capture::Mouse(id));
            let changed = keymap::user_change(prefs, ctx, "mouse", id).is_some();
            let shown: AnyElement = if capturing {
                div()
                    .id("mouse-capture")
                    .debug_selector(|| "mouse-capture".into())
                    .w(px(220.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(c(THEME.green))
                    .child(caption(tr("Do the gesture here")))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |s, ev: &MouseDownEvent, _, cx| s.capture_press(d, Btn::Left, ev, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |s, ev: &MouseDownEvent, _, cx| s.capture_press(d, Btn::Right, ev, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |s, ev: &MouseDownEvent, _, cx| s.capture_press(d, Btn::Middle, ev, cx)),
                    )
                    .on_scroll(cx.listener(move |s, ev: &ScrollEvent, _, cx| s.capture_wheel(d, ev, cx)))
                    .into_any_element()
            } else if gestures.is_empty() {
                caption(tr("none")).into_any_element()
            } else {
                let text = gestures.iter().map(gesture_label).collect::<Vec<_>>().join("  ");
                div().text_size(px(12.)).text_color(c(THEME.text2)).child(text).into_any_element()
            };
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(2.))
                    .when(capturing, |d| d.bg(c_bg()))
                    .child(div().flex_1().text_size(px(12.)).child(tr(d.label)))
                    .child(div().w(px(90.)).child(caption(tr(d.target))))
                    .child(div().w(px(230.)).flex().justify_end().child(shown))
                    .child(
                        button(("mouse-change", i), tr("Change"), capturing, THEME.green)
                            .debug_selector(move || format!("mouse-change-{i}"))
                            .on_click(cx.listener(move |s, _, _, cx| {
                                s.capturing = Some(Capture::Mouse(id));
                                cx.notify();
                            })),
                    )
                    .child(button(("mouse-clear", i), "×", false, 0).on_click(cx.listener(move |s, _, _, cx| {
                        let node = keymap::with_change(s.prefs(cx), ctx, "mouse", id, Some(&[]));
                        s.write_ctx(ctx, node, cx);
                    })))
                    .child(div().w(px(60.)).when(changed, |d| {
                        d.child(button(("mouse-default", i), tr("Default"), false, 0).on_click(cx.listener(
                            move |s, _, _, cx| {
                                let node = keymap::with_change(s.prefs(cx), ctx, "mouse", id, None);
                                s.write_ctx(ctx, node, cx);
                            },
                        )))
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    // --- capturing ----------------------------------------------------------------------------------------------

    /// The other action that has the keys being captured, in the same context or in Global
    fn conflict_of(&self, def: &ActionDef, prefs: &Value, user: &[Preset]) -> Option<&'static ActionDef> {
        let keys = self.pending_keys.as_ref()?;
        let ctx = keymap::ctx_of(def);
        crate::actions::REGISTRY
            .iter()
            .filter(|o| o.id != def.id && (keymap::ctx_of(o) == ctx || keymap::ctx_of(o) == "Global"))
            .find(|o| keymap::keys_for(o, prefs, user).contains(keys))
    }

    pub(super) fn capture_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(cap) = self.capturing else { return };
        cx.stop_propagation();
        if ev.keystroke.key == "escape" {
            self.capturing = None;
            self.pending_keys = None;
            cx.notify();
            return;
        }
        let Some(keys) = keys_of_event(ev) else { return };
        match cap {
            Capture::Mouse(_) => {}
            Capture::Find => {
                self.kfind = Some(keys);
                self.capturing = None;
            }
            Capture::Key(id) => {
                let Some(def) = find(id) else { return };
                let prefs = self.prefs(cx).clone();
                let user = keymap::user_presets();
                self.pending_keys = Some(keys.clone());
                let conflict = self.conflict_of(def, &prefs, &user);
                // a second press of the same keys takes them from the other action
                let confirmed = conflict.is_some() && self.kconfirm.as_deref() == Some(keys.as_str());
                if conflict.is_none() || confirmed {
                    if let Some(other) = conflict {
                        let rest: Vec<String> =
                            keymap::keys_for(other, &prefs, &user).into_iter().filter(|k| *k != keys).collect();
                        let octx = keymap::ctx_of(other);
                        let node = keymap::with_change(&prefs, octx, "keys", other.id, Some(&rest));
                        self.write_ctx(octx, node, cx);
                    }
                    let ctx = keymap::ctx_of(def);
                    let node = keymap::with_change(self.prefs(cx), ctx, "keys", id, Some(&[keys]));
                    self.write_ctx(ctx, node, cx);
                    self.capturing = None;
                    self.pending_keys = None;
                    self.kconfirm = None;
                } else {
                    self.kconfirm = Some(keys);
                }
            }
        }
        cx.notify();
    }

    fn capture_press(&mut self, def: &'static MouseDef, btn: Btn, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let kind = if ev.click_count >= 2 {
            Kind::Double
        } else {
            // a press is a drag when the action's gesture was one: the handler decides at the press
            let was = keymap::gestures_for(def, self.prefs(cx), &keymap::user_presets()).first().map(|g| g.kind);
            if was == Some(Kind::Drag) { Kind::Drag } else { Kind::Click }
        };
        let g = Gesture {
            shift: ev.modifiers.shift,
            secondary: ev.modifiers.control || ev.modifiers.platform,
            alt: ev.modifiers.alt,
            btn,
            kind,
        };
        self.bind_gesture(def, g, cx);
    }

    fn capture_wheel(&mut self, def: &'static MouseDef, ev: &ScrollEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let g = Gesture {
            shift: ev.modifiers.shift,
            secondary: ev.modifiers.control || ev.modifiers.platform,
            alt: ev.modifiers.alt,
            btn: Btn::Left,
            kind: Kind::Wheel,
        };
        self.bind_gesture(def, g, cx);
    }

    fn bind_gesture(&mut self, def: &'static MouseDef, g: Gesture, cx: &mut Context<Self>) {
        let node = keymap::with_change(self.prefs(cx), def.ctx, "mouse", def.id, Some(&[g.text()]));
        self.write_ctx(def.ctx, node, cx);
        self.capturing = None;
        cx.notify();
    }

    // --- the presets as files -----------------------------------------------------------------------------------

    fn save_preset(&mut self, ctx: &'static str, cx: &mut Context<Self>) {
        let Some(dir) = self.config_dir(cx) else {
            self.kstatus = Some("No folder for presets here");
            return cx.notify();
        };
        let user = keymap::user_presets();
        let wanted = self.kname.read(cx).value().trim().to_string();
        let name = presets::free_name(ctx, &wanted, &user);
        let preset = keymap::snapshot(ctx, &name, self.prefs(cx), &user);
        self.kstatus = presets::save_user(&dir, &preset).err().map(|_| "The preset was not saved");
        self.reload_user_presets(cx);
        // the preset has what the context had: its own changes are in it now
        let node = keymap::restored(self.prefs(cx), ctx).with("preset", Value::text(name.as_str()));
        self.write_ctx(ctx, node, cx);
        cx.notify();
    }

    fn delete_preset(&mut self, ctx: &'static str, cx: &mut Context<Self>) {
        let (Some(dir), name) = (self.config_dir(cx), keymap::chosen(self.prefs(cx), ctx).to_string()) else { return };
        let user = keymap::user_presets();
        if let Some(p) = user.iter().find(|p| p.ctx == ctx && p.name == name) {
            self.kstatus = presets::delete_user(&dir, p).err().map(|_| "The preset was not deleted");
        }
        self.reload_user_presets(cx);
        self.choose_preset(ctx, keymap::DEFAULT, cx);
    }

    fn import_preset(&mut self, ctx: &'static str, cx: &mut Context<Self>) {
        let Some(dir) = self.config_dir(cx) else {
            self.kstatus = Some("No folder for presets here");
            return cx.notify();
        };
        let me = cx.entity();
        super::super::files::choose(cx, "Import a preset", false, false, move |paths, cx| {
            let Some(path) = paths.first() else { return };
            me.update(cx, |s, cx| {
                match presets::read_file(path, ctx) {
                    Ok(p) => {
                        let user = keymap::user_presets();
                        let p = Preset { name: presets::free_name(&p.ctx, &p.name, &user), user: true, ..p };
                        let target = p.ctx.clone();
                        s.kstatus = presets::save_user(&dir, &p).err().map(|_| "The preset was not saved");
                        s.reload_user_presets(cx);
                        if let Some(c) = CONTEXTS.iter().find(|c| c.id == target) {
                            s.kctx = c.id;
                            s.choose_preset(c.id, &p.name, cx);
                        }
                    }
                    Err(_) => s.kstatus = Some("This file is not a preset"),
                }
                cx.notify();
            });
        });
    }

    fn export_preset(&mut self, ctx: &'static str, cx: &mut Context<Self>) {
        let name = keymap::chosen(self.prefs(cx), ctx).to_string();
        let preset = keymap::snapshot(ctx, &name, self.prefs(cx), &keymap::user_presets());
        let folder = self.config_dir(cx).unwrap_or_else(|| std::path::PathBuf::from("."));
        let me = cx.entity();
        super::super::files::choose_new(cx, &folder, &format!("{ctx}-{name}.json"), move |path, cx| {
            let failed = presets::write_file(&path, &preset).is_err();
            me.update(cx, |s, cx| {
                s.kstatus = failed.then_some("The preset was not saved");
                cx.notify();
            });
        });
    }
}

fn c_bg() -> gpui::Rgba {
    c(THEME.bg4)
}
