//! Owner: agent A. The routing (I-3.9, spec item 2, F-MIX-010/012): every track's outputs on the left, each with
//! the desk channel it goes to; a click on the destination lists the channels (several outputs may go to the same
//! one; a new track goes to the master). A track with several outputs (a drum kit's pieces) offers "Create
//! channels for the outputs": a group channel and one channel per output, routed, in one undo step. Not the
//! legacy's spin box of channel numbers: the whole picture in one place, by name.

use fake_core::arrange::TrackKind;
use fake_core::mixer::ChannelKind;
use fake_core::port::EditOp;
use fake_core::{Field, NodeId, Value};
use gpui::{Context, FontWeight, Render, SharedString, Window, div, prelude::*, px};

use super::ScreenCx;
use crate::ui::{THEME, button, c, caption, tr, trf, track_color};

pub struct Routing {
    scx: ScreenCx,
    /// the output whose destination is being chosen: (track id, output id)
    picking: Option<(NodeId, NodeId)>,
    filter_multi: bool,
}

impl Routing {
    pub fn new(scx: ScreenCx, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Routing { scx, picking: None, filter_multi: false }
    }

    fn route(&mut self, track: NodeId, output: NodeId, to: NodeId, cx: &mut Context<Self>) {
        self.scx.core.update(cx, |core, _| {
            let _ = core.port_mut().apply("Route", vec![EditOp::set(track, Field::OutputTo(output), Value::Id(to))]);
        });
        self.picking = None;
        cx.notify();
    }

    fn create_channels(&mut self, track_index: usize, cx: &mut Context<Self>) {
        self.scx.core.update(cx, |core, _| {
            let _ = core.port_mut().run_action("mixer.create_channels_for_outputs", Value::Real(track_index as f64));
        });
        cx.notify();
    }
}

impl Render for Routing {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = self.scx.core.read(cx).port().root();
        let channels: Vec<(NodeId, String, ChannelKind)> = root.channels.iter().map(|c| (c.id, c.name.clone(), c.kind)).collect();
        let name_of = |id: NodeId| channels.iter().find(|c| c.0 == id).map_or("?".to_string(), |c| c.1.clone());
        let picking = self.picking;
        let mut rows = Vec::new();
        for (ti, t) in root.tracks.iter().enumerate() {
            if t.kind == TrackKind::Folder || (self.filter_multi && t.outputs.len() < 2) {
                continue;
            }
            let multi = t.outputs.len() > 1;
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(6.))
                    .child(div().w(px(6.)).h(px(16.)).bg(c(track_color(ti))))
                    .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::BOLD).child(format!("{} · {}", ti + 1, t.name)))
                    .when(multi, |d| {
                        d.child(
                            button(SharedString::from(format!("mk{ti}")), tr("Create channels for the outputs"), false, 0)
                                .on_click(cx.listener(move |r, _, _, cx| r.create_channels(ti, cx))),
                        )
                    })
                    .into_any_element(),
            );
            for o in &t.outputs {
                let (tid, oid) = (t.id, o.id);
                let open = picking == Some((tid, oid));
                rows.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pl(px(16.))
                        .child(div().w(px(90.)).text_size(px(11.)).child(tr(o.port)))
                        .child(div().w(px(60.)).text_size(px(10.)).text_color(c(THEME.text3)).child(if o.channels == 1 { tr("mono") } else { tr("stereo") }))
                        .child(div().text_size(px(11.)).text_color(c(THEME.text3)).child("→"))
                        .child(
                            button(SharedString::from(format!("to{}", oid.number)), name_of(o.to), open, THEME.blue)
                                .min_w(px(180.))
                                .on_click(cx.listener(move |r, _, _, cx| {
                                    r.picking = if r.picking == Some((tid, oid)) { None } else { Some((tid, oid)) };
                                    cx.notify();
                                })),
                        )
                        .into_any_element(),
                );
                if open {
                    // the channels, grouped by kind; a click routes
                    rows.push(
                        div()
                            .ml(px(176.))
                            .p(px(6.))
                            .flex()
                            .flex_wrap()
                            .gap(px(4.))
                            .rounded(px(6.))
                            .bg(c(THEME.bg3))
                            .children(channels.iter().map(|(cid, cname, kind)| {
                                let cid = *cid;
                                let color = match kind {
                                    ChannelKind::Master => THEME.green,
                                    ChannelKind::Return => THEME.cyan,
                                    ChannelKind::Group => THEME.purple,
                                    ChannelKind::Normal => THEME.bg4,
                                };
                                button(SharedString::from(format!("pick{}-{}", oid.number, cid.number)), cname.clone(), cid == o.to, color)
                                    .on_click(cx.listener(move |r, _, _, cx| r.route(tid, oid, cid, cx)))
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
                    .child(button("multi", tr("Only tracks with several outputs"), self.filter_multi, THEME.green).on_click(cx.listener(|r, _, _, cx| {
                        r.filter_multi = !r.filter_multi;
                        cx.notify();
                    })))
                    .child(button("close", tr("Close"), false, 0).on_click(move |_, _, cx| scx.close(cx))),
            )
            .child(caption(trf("Each output of a track goes to a desk channel; several may go to the same one ({0} channels)", &[&channels.len()])))
            .child(div().id("routing-rows").flex().flex_col().gap(px(2.)).overflow_y_scroll().children(rows))
    }
}
