//! Owner: agent A. The routing (I-3.9, spec item 2, F-MIX-010/012): every track's outputs with the desk channel each
//! goes to, chosen by name from the channels grouped by kind (several outputs may go to the same one; a new track goes
//! to the master). A track with several outputs (a drum kit's pieces) offers "Create channels for the outputs": a
//! group and one channel per output, routed, in one undo step. Through the port: the routes are `EditOp::Set` on the
//! outputs' `to`; the channels are inserted with temporary refs in the same list.

use gpui::{Context, FontWeight, Render, SharedString, Window, div, prelude::*, px};
use lm_port::{EditOp, Label, NodeId, Value};

use super::ScreenCx;
use crate::model::{self, ChannelKind};
use crate::ui::{THEME, button, c, tr, tr_str, track_color};

pub struct Routing {
    scx: ScreenCx,
    /// the output whose destination is being chosen
    picking: Option<NodeId>,
    filter_multi: bool,
}

impl Routing {
    pub fn new(scx: ScreenCx, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Routing { scx, picking: None, filter_multi: false }
    }

    fn route(&mut self, output: NodeId, to: NodeId, cx: &mut Context<Self>) {
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| {
                pt.apply(Label::new("Route"), vec![EditOp::set(output, &["to"], Value::text(to.to_string()))])
            });
        });
        self.picking = None;
        cx.notify();
    }

    /// "Create channels for the outputs" (I-3.9): an action of the core, one undo step — a group channel, a channel
    /// per output sending to it, each output routed to its channel
    pub fn create_channels(&mut self, track: NodeId, cx: &mut Context<Self>) {
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| {
                pt.run_action(
                    "mixer.create_channels_for_outputs".into(),
                    Value::object([("track", Value::text(track.to_string()))]),
                )
            });
        });
        cx.notify();
    }
}

impl Render for Routing {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = self.scx.core.read(cx).port().root().clone();
        let channels = model::channels(&root);
        let name_of =
            |id: NodeId| channels.iter().find(|c| c.id == id).map_or_else(|| "?".to_string(), |c| c.name.to_string());
        let picking = self.picking;
        let mut rows = Vec::new();
        for t in model::tracks(&root) {
            if t.folder || (self.filter_multi && t.outputs.len() < 2) {
                continue;
            }
            let multi = t.outputs.len() > 1;
            let tid = t.id;
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(6.))
                    .child(div().w(px(6.)).h(px(16.)).bg(c(track_color(t.index))))
                    .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::BOLD).child(format!(
                        "{} · {}",
                        t.index + 1,
                        t.name
                    )))
                    .when(multi, |d| {
                        d.child(
                            button(
                                SharedString::from(format!("mk{}", tid.number)),
                                tr("Create channels for the outputs"),
                                false,
                                0,
                            )
                            .debug_selector(move || format!("create-channels-{}", tid.number))
                            .on_click(cx.listener(move |r, _, _, cx| r.create_channels(tid, cx))),
                        )
                    })
                    .into_any_element(),
            );
            for o in &t.outputs {
                let (oid, to) = (o.id, o.to);
                let open = picking == Some(oid);
                rows.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pl(px(16.))
                        .child(div().w(px(90.)).text_size(px(11.)).child(tr_str(&o.port)))
                        .child(
                            div().w(px(60.)).text_size(px(10.)).text_color(c(THEME.text3)).child(if o.channels == 1 {
                                tr("mono")
                            } else {
                                tr("stereo")
                            }),
                        )
                        .child(div().text_size(px(11.)).text_color(c(THEME.text3)).child("→"))
                        .child(
                            button(SharedString::from(format!("to{}", oid.number)), name_of(to), open, THEME.blue)
                                .debug_selector(move || format!("route-{}", oid.number))
                                .min_w(px(180.))
                                .on_click(cx.listener(move |r, _, _, cx| {
                                    r.picking = if r.picking == Some(oid) { None } else { Some(oid) };
                                    cx.notify();
                                })),
                        )
                        .into_any_element(),
                );
                if open {
                    rows.push(
                        div()
                            .ml(px(176.))
                            .p(px(6.))
                            .flex()
                            .flex_wrap()
                            .gap(px(4.))
                            .rounded(px(6.))
                            .bg(c(THEME.bg3))
                            .children(channels.iter().map(|ch| {
                                let cid = ch.id;
                                let color = match ch.kind {
                                    ChannelKind::Master => THEME.green,
                                    ChannelKind::Return => THEME.cyan,
                                    ChannelKind::Group => THEME.purple,
                                    ChannelKind::Normal => THEME.bg4,
                                };
                                button(
                                    SharedString::from(format!("pick{}-{}", oid.number, cid.number)),
                                    ch.name.to_string(),
                                    cid == to,
                                    color,
                                )
                                .debug_selector(move || format!("pick-{}-{}", oid.number, cid.number))
                                .on_click(cx.listener(move |r, _, _, cx| r.route(oid, cid, cx)))
                            }))
                            .into_any_element(),
                    );
                }
            }
        }
        let scx = self.scx.clone();
        div()
            .id("routing")
            .w(px(760.))
            .max_h(px(620.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .rounded(px(10.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().text_size(px(16.)).font_weight(FontWeight::BOLD).child(tr("Routing")))
                    .child(
                        button("multi", tr("Only tracks with several outputs"), self.filter_multi, THEME.green)
                            .on_click(cx.listener(|r, _, _, cx| {
                                r.filter_multi = !r.filter_multi;
                                cx.notify();
                            })),
                    )
                    .child(button("close", tr("Close"), false, 0).on_click(move |_, _, cx| scx.close(cx))),
            )
            .child(div().id("routing-rows").flex().flex_col().gap(px(2.)).overflow_y_scroll().children(rows))
    }
}
