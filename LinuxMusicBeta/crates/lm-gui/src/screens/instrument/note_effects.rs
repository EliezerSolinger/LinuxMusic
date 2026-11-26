//! The instrument window's "Note effects" tab: the track's `noteEffects` chain, in the order the notes go through
//! it, before the instrument. Each effect is a row (on/off, name, up, down, remove) and, under it, its controls from
//! the generic plugin editor (the same `Ctl`s as the instrument's). What is added comes from the plugins of category
//! `note`. Every change is an `EditOp` on the track's list (I-15).

use super::*;

/// The category of the note effect plugins (`PluginInfo::category`)
pub(super) const NOTE_CATEGORY: &str = "note";

impl InstrumentWindow {
    pub(super) fn note_effects_tab(&self, plugins: &[PluginInfo], cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(track) = self.track else { return Vec::new() };
        let core = self.scx.core.clone();
        let (effects, root) = {
            let port = core.read(cx).port();
            let root = port.root().clone();
            let node = model::list(&root, "tracks").find(|x| model::id_of(x) == Some(track)).cloned();
            let effects: Vec<Value> =
                node.map(|n| model::list(&n, "noteEffects").cloned().collect()).unwrap_or_default();
            (effects, root)
        };
        let count = effects.len();
        let mut out = Vec::new();
        for (i, v) in effects.iter().enumerate() {
            let Some(id) = model::id_of(v) else { continue };
            let plugin = model::text(v, "plugin");
            let on = !v.get("bypass").and_then(Value::as_bool).unwrap_or(false);
            let row = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .w_full()
                .child(
                    button(SharedString::from(format!("iw-ne-on-{}", id.number)), tr("On"), on, THEME.green).on_click(
                        cx.listener(move |w, _, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Bypass", vec![EditOp::set(id, &["bypass"], Value::bool(on))], cx);
                        }),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.5))
                        .text_color(c(if on { THEME.text } else { THEME.text3 }))
                        .child(plugin_name(plugins, &plugin)),
                )
                .child(
                    button(SharedString::from(format!("iw-ne-up-{}", id.number)), "↑", false, 0)
                        .opacity(if i > 0 { 1.0 } else { 0.4 })
                        .on_click(cx.listener(move |w, _, _, cx| {
                            if i > 0 {
                                w.move_note_effect(id, i - 1, cx);
                            }
                        })),
                )
                .child(
                    button(SharedString::from(format!("iw-ne-down-{}", id.number)), "↓", false, 0)
                        .opacity(if i + 1 < count { 1.0 } else { 0.4 })
                        .on_click(cx.listener(move |w, _, _, cx| {
                            if i + 1 < count {
                                w.move_note_effect(id, i + 1, cx);
                            }
                        })),
                )
                .child(button(SharedString::from(format!("iw-ne-remove-{}", id.number)), "×", false, 0).on_click(
                    cx.listener(move |w, _, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Remove a note effect", vec![EditOp::Remove { id: Ref::Id(id) }], cx);
                    }),
                ))
                .into_any_element();
            let controls = match schema_of(&core, &plugin, cx) {
                Some(schema) => controls_of(id, v, &schema, &root),
                None => Vec::new(),
            };
            let mut card = vec![row];
            if on {
                card.extend(controls.into_iter().map(|k| control(self, k, &*cx)));
            }
            out.push(group_card(plugin_name(plugins, &plugin), card));
        }
        if out.is_empty() {
            out.push(caption(tr("No note effects")).into_any_element());
        }
        let mut available: Vec<&PluginInfo> = plugins.iter().filter(|p| &*p.category == NOTE_CATEGORY).collect();
        available.sort_by_key(|p| p.id.clone());
        let adders: Vec<AnyElement> = available
            .into_iter()
            .map(|p| {
                let plugin = p.id.clone();
                button(SharedString::from(format!("iw-ne-add-{}", p.id)), plugin_name(plugins, &p.id), false, 0)
                    .h(px(24.))
                    .px(px(10.))
                    .on_click(cx.listener(move |w, _, _, cx| w.add_note_effect(&plugin, cx)))
                    .into_any_element()
            })
            .collect();
        out.push(div().flex().flex_wrap().gap(px(6.)).children(adders).into_any_element());
        out
    }

    fn move_note_effect(&mut self, id: NodeId, index: usize, cx: &mut Context<Self>) {
        let Some(track) = self.track else { return };
        let core = self.scx.core.clone();
        let op = EditOp::Move { id: Ref::Id(id), parent: Ref::Id(track), list: "noteEffects".into(), index };
        apply_ops(&core, "Move a note effect", vec![op], cx);
    }

    fn add_note_effect(&mut self, plugin: &str, cx: &mut Context<Self>) {
        let Some(track) = self.track else { return };
        let core = self.scx.core.clone();
        let len = {
            let root = core.read(cx).port().root().clone();
            model::list(&root, "tracks")
                .find(|x| model::id_of(x) == Some(track))
                .map_or(0, |n| model::list(n, "noteEffects").count())
        };
        let node = Value::object([("plugin", Value::text(plugin)), ("state", Value::empty_object())]);
        let op = EditOp::Insert { temp: 1, parent: Ref::Id(track), list: "noteEffects".into(), index: len, node };
        apply_ops(&core, "Add a note effect", vec![op], cx);
    }
}
