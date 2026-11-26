//! A mouse gesture as data: the modifiers, the button and what is done with it (`secondary-shift-drag`,
//! `right-click`, `alt-wheel`, `double-click`). `secondary` is Cmd on macOS and Ctrl elsewhere, but the handlers
//! accept either of the two so a Ctrl-click keeps working on both. A press that starts a drag is both a `click` and
//! a `drag` for the handlers (they decide at the press); the two names are for the user's reading.

use gpui::{Modifiers, MouseButton};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Click,
    Drag,
    Double,
    Wheel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Left,
    Right,
    Middle,
}

impl Btn {
    pub fn of(b: MouseButton) -> Btn {
        match b {
            MouseButton::Right => Btn::Right,
            MouseButton::Middle => Btn::Middle,
            _ => Btn::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gesture {
    pub shift: bool,
    pub secondary: bool,
    pub alt: bool,
    pub btn: Btn,
    pub kind: Kind,
}

impl Gesture {
    /// `secondary-shift-alt-` then `click`, `drag`, `double-click`, `wheel`, `right-click`, `right-drag`,
    /// `middle-click`, `middle-drag`
    pub fn parse(text: &str) -> Option<Gesture> {
        let mut rest = text.trim();
        let (mut shift, mut secondary, mut alt) = (false, false, false);
        loop {
            if let Some(r) = rest.strip_prefix("shift-") {
                shift = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("secondary-") {
                secondary = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("alt-") {
                alt = true;
                rest = r;
            } else {
                break;
            }
        }
        let (btn, kind) = match rest {
            "click" => (Btn::Left, Kind::Click),
            "drag" => (Btn::Left, Kind::Drag),
            "double-click" => (Btn::Left, Kind::Double),
            "wheel" => (Btn::Left, Kind::Wheel),
            "right-click" => (Btn::Right, Kind::Click),
            "right-drag" => (Btn::Right, Kind::Drag),
            "middle-click" => (Btn::Middle, Kind::Click),
            "middle-drag" => (Btn::Middle, Kind::Drag),
            _ => return None,
        };
        Some(Gesture { shift, secondary, alt, btn, kind })
    }

    /// The canonical text, the one `parse` reads
    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.secondary {
            s.push_str("secondary-");
        }
        if self.alt {
            s.push_str("alt-");
        }
        if self.shift {
            s.push_str("shift-");
        }
        s.push_str(match (self.btn, self.kind) {
            (Btn::Left, Kind::Click) => "click",
            (Btn::Left, Kind::Drag) => "drag",
            (Btn::Left, Kind::Double) => "double-click",
            (Btn::Left, Kind::Wheel) | (_, Kind::Wheel) => "wheel",
            (Btn::Right, Kind::Drag) => "right-drag",
            (Btn::Right, _) => "right-click",
            (Btn::Middle, Kind::Drag) => "middle-drag",
            (Btn::Middle, _) => "middle-click",
        });
        s
    }

    /// Whether an event is this gesture: `clicks` is the press' click count (1 for a drag's moves, 0 for a wheel)
    pub fn matches(&self, btn: Btn, clicks: usize, wheel: bool, m: &Modifiers) -> bool {
        let secondary = m.control || m.platform;
        if self.shift != m.shift || self.alt != m.alt || self.secondary != secondary {
            return false;
        }
        match self.kind {
            Kind::Wheel => wheel,
            Kind::Click | Kind::Drag => !wheel && self.btn == btn && clicks >= 1,
            Kind::Double => !wheel && self.btn == btn && clicks >= 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips() {
        for t in ["click", "secondary-drag", "alt-shift-wheel", "right-click", "double-click", "middle-drag"] {
            let g = Gesture::parse(t).unwrap_or_else(|| panic!("{t}"));
            assert_eq!(Gesture::parse(&g.text()), Some(g));
        }
        assert_eq!(Gesture::parse("shift-alt-click").unwrap().text(), "alt-shift-click");
        assert!(Gesture::parse("fling").is_none());
    }

    #[test]
    fn modifiers_must_match_exactly() {
        let g = Gesture::parse("shift-click").unwrap();
        let shift = Modifiers { shift: true, ..Modifiers::default() };
        assert!(g.matches(Btn::Left, 1, false, &shift));
        assert!(!g.matches(Btn::Left, 1, false, &Modifiers::default()));
        assert!(!g.matches(Btn::Left, 1, false, &Modifiers { shift: true, alt: true, ..Modifiers::default() }));
        assert!(!g.matches(Btn::Right, 1, false, &shift));
        assert!(!g.matches(Btn::Left, 0, true, &shift));
    }

    #[test]
    fn ctrl_and_cmd_are_both_secondary() {
        let g = Gesture::parse("secondary-click").unwrap();
        assert!(g.matches(Btn::Left, 1, false, &Modifiers { control: true, ..Modifiers::default() }));
        assert!(g.matches(Btn::Left, 1, false, &Modifiers { platform: true, ..Modifiers::default() }));
    }

    #[test]
    fn a_double_needs_two_presses() {
        let g = Gesture::parse("double-click").unwrap();
        assert!(!g.matches(Btn::Left, 1, false, &Modifiers::default()));
        assert!(g.matches(Btn::Left, 2, false, &Modifiers::default()));
    }
}
