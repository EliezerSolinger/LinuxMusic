//! The keys of the Piano Roll that are not about notes: the keyboard's octave, the step input, the scroll, the
//! note length and the quantization by number. All of them are actions of the registry (`notes.*`), so the
//! keymap decides the keys; only the piano keys themselves (a key down and up pair that sounds a note) are read
//! from the keyboard in `key_down`.

use gpui::{Context, Div, Stateful, prelude::*};
use lm_port::Value;

use super::{GRIDS, PianoRoll, Rec};
use crate::actions::{
    PrGrid0, PrGrid1, PrGrid2, PrGrid3, PrGrid4, PrGrid5, PrGrid6, PrGrid7, PrGrid8, PrGrid9, PrHome, PrKbOctaveDown,
    PrKbOctaveUp, PrLen0, PrLen1, PrLen2, PrLen3, PrLen4, PrLen5, PrLen6, PrLen7, PrLen8, PrLen9, PrScrollDown,
    PrScrollUp, PrStepBack, PrStepLonger, PrStepRest, PrStepShorter,
};

/// One listener per numbered action: `PrLen3 => 3`
macro_rules! by_number {
    ($d:expr, $cx:expr, $method:ident, $($t:ident => $n:expr),* $(,)?) => {
        $d $(.on_action($cx.listener(|pr, _: &$t, _, cx| pr.$method($n, cx))))*
    };
}

impl PianoRoll {
    pub(super) fn key_actions(&self, d: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        let d = d
            .on_action(cx.listener(|pr, _: &PrKbOctaveDown, _, cx| {
                pr.octave = (pr.octave - 1).max(0);
                cx.notify();
            }))
            .on_action(cx.listener(|pr, _: &PrKbOctaveUp, _, cx| {
                pr.octave = (pr.octave + 1).min(8);
                cx.notify();
            }))
            // step input: right is a rest, left goes back (the core's `notes.step_rest` and `notes.step_back`)
            .on_action(cx.listener(|pr, _: &PrStepRest, _, cx| {
                if pr.rec == Rec::Step {
                    pr.step_action("notes.step_rest", Vec::new(), cx);
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|pr, _: &PrStepBack, _, cx| {
                if pr.rec == Rec::Step {
                    pr.step_action("notes.step_back", Vec::new(), cx);
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            // the last chord gets shorter or longer, the cursor with it (the core's `notes.step_length`)
            .on_action(cx.listener(|pr, _: &PrStepShorter, _, cx| pr.step_length(-1, cx)))
            .on_action(cx.listener(|pr, _: &PrStepLonger, _, cx| pr.step_length(1, cx)))
            .on_action(cx.listener(|pr, _: &PrScrollUp, _, cx| pr.scroll_keys(1.0, cx)))
            .on_action(cx.listener(|pr, _: &PrScrollDown, _, cx| pr.scroll_keys(-1.0, cx)))
            .on_action(cx.listener(|pr, _: &PrHome, _, cx| {
                pr.play.cursor = 0;
                if pr.rec == Rec::Step {
                    pr.step_action("notes.step_record", vec![("at", Value::int(0))], cx);
                }
                pr.scroll = 0.0;
                cx.notify();
            }));
        // Ctrl+0..9: the note length (0 = the last note); Alt+0..9: the quantization (F-NOT-009)
        let d = by_number!(
            d, cx, set_length_by_number, PrLen0 => 0, PrLen1 => 1, PrLen2 => 2, PrLen3 => 3, PrLen4 => 4,
            PrLen5 => 5, PrLen6 => 6, PrLen7 => 7, PrLen8 => 8, PrLen9 => 9
        );
        by_number!(
            d, cx, set_grid_by_number, PrGrid0 => 0, PrGrid1 => 1, PrGrid2 => 2, PrGrid3 => 3, PrGrid4 => 4,
            PrGrid5 => 5, PrGrid6 => 6, PrGrid7 => 7, PrGrid8 => 8, PrGrid9 => 9
        )
    }

    fn step_length(&mut self, dir: i64, cx: &mut Context<Self>) {
        if self.rec != Rec::Step {
            cx.propagate();
            return;
        }
        let d = dir * self.grid_ticks();
        self.step_action("notes.step_length", vec![("delta", Value::int(d))], cx);
        cx.notify();
    }

    fn scroll_keys(&mut self, dir: f32, cx: &mut Context<Self>) {
        self.scroll_y = (self.scroll_y - dir * self.key_h * 2.0).max(0.0);
        cx.notify();
    }

    fn set_length_by_number(&mut self, d: usize, cx: &mut Context<Self>) {
        self.length = d.checked_sub(1).map(|i| i.min(GRIDS.len() - 1));
        self.remember_grid(cx);
        if self.rec == Rec::Step {
            let step = self.new_len();
            self.step_action("notes.step_record", vec![("step", Value::int(step))], cx);
        }
        cx.notify();
    }

    fn set_grid_by_number(&mut self, d: usize, cx: &mut Context<Self>) {
        self.grid = d.saturating_sub(1).min(GRIDS.len() - 1);
        self.lock = false;
        self.remember_grid(cx);
        cx.notify();
    }
}
