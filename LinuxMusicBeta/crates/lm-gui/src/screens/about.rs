//! Owner: agent D. A placeholder until its author ports it from prototypes/gpui/src/screens/about.rs to the real tree
//! (model.rs) and the CorePort; docs/prototipo-interface.md says what it covers.

use gpui::{Context, Render, Window, div, prelude::*, px};

use super::ScreenCx;
use crate::ui::{THEME, c, tr, trf};

pub struct About {
    scx: ScreenCx,
}

impl About {
    pub fn new(scx: ScreenCx, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        About { scx }
    }
}

impl Render for About {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(16.))
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text2))
            .child(trf("Not built yet: {0}", &[&tr(self.scx.screen.title())]))
    }
}
