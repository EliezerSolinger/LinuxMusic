//! Owner: agent C. A placeholder until its author builds it; see docs/prototipo-interface.md for what it covers.

use gpui::{Context, Render, Window, div, prelude::*, px};

use super::ScreenCx;
use crate::ui::{THEME, c, tr, trf};

pub struct Browser {
    scx: ScreenCx,
}

impl Browser {
    pub fn new(scx: ScreenCx, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Browser { scx }
    }
}

impl Render for Browser {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(16.))
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text2))
            .child(trf("Not built yet: {0}", &[&tr(self.scx.screen.title())]))
    }
}
