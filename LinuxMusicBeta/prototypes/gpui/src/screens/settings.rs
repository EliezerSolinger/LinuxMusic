//! Owner: agent D. Design 08 (approved): settings apply at once and say what needs a restart. This first piece
//! (from the earlier prototype) checks gpui-component's Input, Slider, Switch and Button inside our theme.

use gpui::{Context, Entity, FontWeight, Render, Subscription, Window, div, prelude::*, px};
use gpui_component::button::Button;
use gpui_component::input::{Input, InputState};
use gpui_component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_component::switch::Switch;

use super::ScreenCx;
use crate::ui::{THEME, c, tr, trf};

pub struct Settings {
    scx: ScreenCx,
    project_dir: Entity<InputState>,
    block: Entity<SliderState>,
    block_size: f32,
    follow_cursor: bool,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project_dir = cx.new(|cx| InputState::new(window, cx).default_value("~/Música/LinuxMusic"));
        let block = cx.new(|_| SliderState::new().min(64.0).max(2048.0).step(64.0).default_value(256.0));
        // settings apply at once (design 08): the slider's events reach the view
        let sub = cx.subscribe(&block, |s: &mut Settings, _, ev: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(v)) = ev {
                s.block_size = *v;
                cx.notify();
            }
        });
        Settings { scx, project_dir, block, block_size: 256.0, follow_cursor: false, _subscriptions: vec![sub] }
    }
}

impl Render for Settings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = |label: &'static str| {
            div().flex().flex_col().gap(px(6.)).child(div().text_size(px(12.)).text_color(c(THEME.text2)).child(tr(label)))
        };
        div()
            .w(px(420.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .rounded(px(10.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child(tr("Settings")))
            .child(row("Projects folder").child(Input::new(&self.project_dir)))
            .child(
                row("Audio block size").child(Slider::new(&self.block)).child(
                    div()
                        .text_size(px(11.))
                        .text_color(c(THEME.orange))
                        .child(trf("{0} samples · applies when the device reopens", &[&(self.block_size as u32)])),
                ),
            )
            .child(
                Switch::new("follow")
                    .checked(self.follow_cursor)
                    .label(tr("Follow the cursor while playing"))
                    .on_click(cx.listener(|s, on: &bool, _, cx| {
                        s.follow_cursor = *on;
                        cx.notify();
                    })),
            )
            .child(div().flex().justify_end().child(Button::new("close-settings").label(tr("Close")).on_click(
                cx.listener(|s, _, _, cx| s.scx.close(cx)),
            )))
    }
}
