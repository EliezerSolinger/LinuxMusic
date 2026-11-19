//! Owner: agent B. A placeholder until its author builds it; see docs/prototipo-interface.md for what it covers.

use gpui::{Context, Render, Window, div, prelude::*, px};

use super::ScreenCx;
use crate::ui::{THEME, c, tr, trf};

pub struct AutomationEditor {
    scx: ScreenCx,
}

impl AutomationEditor {
    pub fn new(scx: ScreenCx, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        AutomationEditor { scx }
    }
}

impl Render for AutomationEditor {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(16.))
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text2))
            .child(trf("Not built yet: {0}", &[&tr(self.scx.screen.title())]))
    }
}
