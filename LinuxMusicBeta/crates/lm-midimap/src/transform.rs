//! From a control's movement to the target's normalized value (interfaces.md, I-21.2): range, inversion and curve,
//! absolute and relative modes, takeover, buttons. Everything works in two spaces:
//! - the **control space** `x` in 0..=1, where the control is (its value over its maximum);
//! - the **target space** `y` in 0..=1, the target's normalized range.
//!
//! `y = min + (max − min) · c(x)`, where `c` is the curve, mirrored when `invert`. The curve law is the automation's
//! (`(e^{kx} − 1)/(e^k − 1)`, `k = 6·curve`; `curve = 0` is linear), so a mapped knob and a drawn curve bend the same
//! way. Nothing here allocates.

/// How a relative encoder writes its steps in a 7-bit value
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// 1..=63 up, 127..=64 down (127 = −1)
    TwosComplement,
    /// Bit 6 is the sign: 1..=63 up, 65..=127 down (65 = −1). The Mackie Control V-pots
    SignMagnitude,
    /// 64 is still: 65 = +1, 63 = −1
    Offset64,
}

impl Encoding {
    /// The signed step count of a 7-bit value
    pub fn delta(self, v: u8) -> i32 {
        let v = i32::from(v & 0x7F);
        match self {
            Encoding::TwosComplement => {
                if v >= 64 {
                    v - 128
                } else {
                    v
                }
            }
            Encoding::SignMagnitude => {
                if v & 0x40 != 0 {
                    -(v & 0x3F)
                } else {
                    v
                }
            }
            Encoding::Offset64 => v - 64,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Encoding::TwosComplement => "twos",
            Encoding::SignMagnitude => "signMagnitude",
            Encoding::Offset64 => "offset64",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Absolute,
    Relative(Encoding),
}

impl Mode {
    pub const NAMES: &'static [&'static str] = &["absolute", "twos", "signMagnitude", "offset64"];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Absolute => "absolute",
            Mode::Relative(e) => e.name(),
        }
    }

    pub fn from_name(name: &str) -> Option<Mode> {
        Some(match name {
            "absolute" => Mode::Absolute,
            "twos" => Mode::Relative(Encoding::TwosComplement),
            "signMagnitude" => Mode::Relative(Encoding::SignMagnitude),
            "offset64" => Mode::Relative(Encoding::Offset64),
            _ => return None,
        })
    }
}

/// What an absolute control does when it is not where the target is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Takeover {
    /// The target jumps to the control
    Jump,
    /// Nothing happens until the control passes the target's value; then it follows
    Pickup,
    /// The target moves toward the control's end in proportion, so both reach the end together
    Scale,
}

impl Takeover {
    pub const NAMES: &'static [&'static str] = &["jump", "pickup", "scale"];

    pub fn name(self) -> &'static str {
        match self {
            Takeover::Jump => "jump",
            Takeover::Pickup => "pickup",
            Takeover::Scale => "scale",
        }
    }

    pub fn from_name(name: &str) -> Option<Takeover> {
        Some(match name {
            "jump" => Takeover::Jump,
            "pickup" => Takeover::Pickup,
            "scale" => Takeover::Scale,
            _ => return None,
        })
    }
}

/// What a key or an on/off controller does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// Pressed = max, released = min
    Momentary,
    /// Each press flips between min and max
    Toggle,
    /// Each press fires (an action), or sets max
    Trigger,
}

impl Button {
    pub const NAMES: &'static [&'static str] = &["momentary", "toggle", "trigger"];

    pub fn name(self) -> &'static str {
        match self {
            Button::Momentary => "momentary",
            Button::Toggle => "toggle",
            Button::Trigger => "trigger",
        }
    }

    pub fn from_name(name: &str) -> Option<Button> {
        Some(match name {
            "momentary" => Button::Momentary,
            "toggle" => Button::Toggle,
            "trigger" => Button::Trigger,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// The target range, in its normalized space (0..=1). `min > max` is allowed and inverts, like `invert`
    pub min: f32,
    pub max: f32,
    pub invert: bool,
    /// −1..=1; 0 is linear
    pub curve: f32,
    pub mode: Mode,
    /// For relative modes: the change of one step, in the target's normalized space
    pub step: f32,
    pub takeover: Takeover,
    pub button: Button,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            min: 0.0,
            max: 1.0,
            invert: false,
            curve: 0.0,
            mode: Mode::Absolute,
            step: 1.0 / 128.0,
            takeover: Takeover::Pickup,
            button: Button::Toggle,
        }
    }
}

/// The automation's curve law on 0..=1 (`lm-render`, `Shape`): `(e^{kt} − 1)/(e^k − 1)`, `k = 6·curve`
pub fn curve(t: f32, shape: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let k = 6.0 * shape.clamp(-1.0, 1.0);
    if k.abs() < 1e-4 {
        return t;
    }
    (libm::expm1f(k * t) / libm::expm1f(k)).clamp(0.0, 1.0)
}

/// The inverse of [`curve`]
pub fn curve_inverse(u: f32, shape: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    let k = 6.0 * shape.clamp(-1.0, 1.0);
    if k.abs() < 1e-4 {
        return u;
    }
    (libm::log1pf(u * libm::expm1f(k)) / k).clamp(0.0, 1.0)
}

impl Transform {
    /// Control space → target space
    pub fn forward(&self, x: f32) -> f32 {
        let mut c = curve(x, self.curve);
        if self.invert {
            c = 1.0 - c;
        }
        (self.min + (self.max - self.min) * c).clamp(0.0, 1.0)
    }

    /// Target space → control space (where the control would be for this target value); clamped to the range
    pub fn inverse(&self, y: f32) -> f32 {
        let span = self.max - self.min;
        let mut c = if span.abs() < 1e-9 { 0.0 } else { ((y - self.min) / span).clamp(0.0, 1.0) };
        if self.invert {
            c = 1.0 - c;
        }
        curve_inverse(c, self.curve)
    }

    fn lo(&self) -> f32 {
        self.min.min(self.max)
    }

    fn hi(&self) -> f32 {
        self.min.max(self.max)
    }
}

/// What one message means for one mapping, before the transform
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// An absolute position: `value` of `max` (127 or 16383)
    Absolute { value: u16, max: u16 },
    /// Steps of a relative encoder, or NRPN increment/decrement
    Relative { steps: i32 },
    /// A key or an on/off controller went down or up. `value` is the raw velocity or controller value
    Press { down: bool, value: u16 },
}

/// The state of one mapping, allocated when the table is compiled
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapState {
    /// The control's last position (control space), if known
    pub last_x: Option<f32>,
    /// What this mapping last wrote (target space), to know whether something else moved the target since
    pub last_out: Option<f32>,
    /// Pickup and Scale: the control is with the target
    pub picked: bool,
    /// A button is down
    pub pressed: bool,
}

/// The result of one input on one mapping
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    /// The new target value, normalized
    pub value: f32,
    /// The raw value of the message (velocity, 7- or 14-bit value, program number; the step count for relative)
    pub raw: i32,
    /// For a button: the press (true) or the release (false)
    pub press: Option<bool>,
}

/// Something else (another control, automation, undo) changed the target by more than this: the pickup is lost
const MOVED: f32 = 1.0e-4;

impl MapState {
    /// Applies one input. `current` is the target's normalized value now (what the processor or the document has).
    /// `None`: nothing to write (a pickup that has not caught the target, a release of a toggle)
    pub fn apply(&mut self, t: &Transform, input: Input, current: f32) -> Option<Output> {
        if self.last_out.is_some_and(|o| (o - current).abs() > MOVED) {
            self.picked = false; // the target moved without us
        }
        let out = match input {
            Input::Absolute { value, max } => {
                let x = f32::from(value) / f32::from(max.max(1));
                let tol = 0.5 / f32::from(max.max(1));
                let y = match t.mode {
                    // An absolute value on a relative mapping: a 7-bit encoder value
                    Mode::Relative(enc) => {
                        let steps = enc.delta((value & 0x7F) as u8);
                        return self.relative(t, steps, current, steps); // raw: the steps
                    }
                    Mode::Absolute => self.absolute(t, x, tol, current),
                };
                self.last_x = Some(x);
                y.map(|value| Output { value, raw: i32::from(value_raw(x, max)), press: None })
            }
            Input::Relative { steps } => return self.relative(t, steps, current, steps),
            Input::Press { down, value } => {
                let was = self.pressed;
                self.pressed = down;
                let raw = i32::from(value);
                match t.button {
                    Button::Momentary if down != was || down => {
                        Some(Output { value: if down { t.max } else { t.min }, raw, press: Some(down) })
                    }
                    Button::Toggle if down && !was => {
                        let mid = 0.5 * (t.min + t.max);
                        let on = if t.max >= t.min { current < mid } else { current > mid };
                        Some(Output { value: if on { t.max } else { t.min }, raw, press: Some(true) })
                    }
                    Button::Trigger if down && !was => Some(Output { value: t.max, raw, press: Some(true) }),
                    _ => None,
                }
            }
        };
        if let Some(o) = out {
            self.last_out = Some(o.value.clamp(0.0, 1.0));
            return Some(Output { value: o.value.clamp(0.0, 1.0), ..o });
        }
        out
    }

    fn absolute(&mut self, t: &Transform, x: f32, tol: f32, current: f32) -> Option<f32> {
        match t.takeover {
            Takeover::Jump => Some(t.forward(x)),
            Takeover::Pickup => {
                if !self.picked {
                    let at = t.inverse(current);
                    let crossed = self.last_x.is_some_and(|l| (l - at) * (x - at) <= 0.0);
                    if crossed || (x - at).abs() <= tol {
                        self.picked = true;
                    }
                }
                if self.picked { Some(t.forward(x)) } else { None }
            }
            Takeover::Scale => {
                if self.picked {
                    return Some(t.forward(x));
                }
                let at = t.inverse(current);
                if (x - at).abs() <= tol {
                    self.picked = true;
                    return Some(t.forward(x));
                }
                let last = self.last_x?;
                let u = if x > last {
                    if last >= 1.0 { at } else { at + (x - last) * (1.0 - at) / (1.0 - last) }
                } else if x < last {
                    if last <= 0.0 { at } else { at - (last - x) * at / last }
                } else {
                    return None;
                };
                let u = u.clamp(0.0, 1.0);
                if (u - x).abs() <= tol {
                    self.picked = true;
                }
                Some(t.forward(u))
            }
        }
    }

    fn relative(&mut self, t: &Transform, steps: i32, current: f32, raw: i32) -> Option<Output> {
        if steps == 0 {
            return None;
        }
        let dir = if (t.max >= t.min) != t.invert { 1.0 } else { -1.0 };
        let y = (current + dir * steps as f32 * t.step).clamp(t.lo(), t.hi()).clamp(0.0, 1.0);
        self.last_out = Some(y);
        self.picked = true;
        Some(Output { value: y, raw, press: None })
    }
}

fn value_raw(x: f32, max: u16) -> u16 {
    (x * f32::from(max)).round().clamp(0.0, f32::from(max)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(v: u16) -> Input {
        Input::Absolute { value: v, max: 127 }
    }

    #[test]
    fn encodings_convert_exactly() {
        use Encoding::*;
        assert_eq!(TwosComplement.delta(1), 1);
        assert_eq!(TwosComplement.delta(63), 63);
        assert_eq!(TwosComplement.delta(127), -1);
        assert_eq!(TwosComplement.delta(64), -64);
        assert_eq!(SignMagnitude.delta(1), 1);
        assert_eq!(SignMagnitude.delta(65), -1);
        assert_eq!(SignMagnitude.delta(0x4F), -15);
        assert_eq!(Offset64.delta(65), 1);
        assert_eq!(Offset64.delta(63), -1);
        assert_eq!(Offset64.delta(64), 0);
        for v in 0..128u8 {
            // Every value decodes inside the 7-bit range of its law
            assert!((-64..=63).contains(&TwosComplement.delta(v)));
            assert!((-63..=63).contains(&SignMagnitude.delta(v)));
            assert!((-64..=63).contains(&Offset64.delta(v)));
        }
    }

    #[test]
    fn curve_and_inverse() {
        for s in [-1.0, -0.3, 0.0, 0.5, 1.0] {
            for i in 0..=100 {
                let t = i as f32 / 100.0;
                let u = curve(t, s);
                assert!((curve_inverse(u, s) - t).abs() < 2e-3, "{s} {t}");
            }
            assert_eq!(curve(0.0, s), 0.0);
            assert!((curve(1.0, s) - 1.0).abs() < 1e-6);
        }
        let t = Transform { min: 0.2, max: 0.6, invert: true, curve: 0.4, ..Transform::default() };
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            assert!((t.inverse(t.forward(x)) - x).abs() < 2e-3);
        }
        assert!((t.forward(0.0) - 0.6).abs() < 1e-6);
        assert!((t.forward(1.0) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn jump_follows_at_once() {
        let t = Transform { takeover: Takeover::Jump, ..Transform::default() };
        let mut s = MapState::default();
        let o = s.apply(&t, abs(127), 0.0).unwrap();
        assert_eq!(o.value, 1.0);
        assert_eq!(o.raw, 127);
    }

    #[test]
    fn pickup_never_jumps() {
        let t = Transform::default();
        let mut s = MapState::default();
        let target = 0.5;
        // The control starts low and moves up: nothing until it passes 0.5
        for v in 0..63u16 {
            assert_eq!(s.apply(&t, abs(v), target), None, "{v}");
        }
        let o = s.apply(&t, abs(64), target).unwrap(); // crossed 63.5
        assert!((o.value - 64.0 / 127.0).abs() < 1e-6);
        // From now on it follows, never further than one step from the previous value
        let mut prev = o.value;
        for v in (0..64u16).rev() {
            let o = s.apply(&t, abs(v), prev).unwrap();
            assert!((o.value - prev).abs() <= 1.0 / 127.0 + 1e-6);
            prev = o.value;
        }
        // Something else moves the target (automation, undo): the pickup is lost again
        assert_eq!(s.apply(&t, abs(1), 0.9), None);
        // A first message exactly at the target picks up at once
        let mut s = MapState::default();
        assert!(s.apply(&t, abs(64), 64.0 / 127.0).is_some());
        // A jump over the target (a fast move) counts as crossing
        let mut s = MapState::default();
        assert_eq!(s.apply(&t, abs(10), 0.5), None);
        assert!(s.apply(&t, abs(120), 0.5).is_some());
    }

    #[test]
    fn scale_converges() {
        let t = Transform { takeover: Takeover::Scale, ..Transform::default() };
        let mut s = MapState::default();
        let mut target = 0.8;
        assert_eq!(s.apply(&t, abs(20), target), None); // only learns where the control is
        let mut prev_x = 20u16;
        // Moving up: the target moves up too, proportionally, and both reach the top together
        for v in 21..=127u16 {
            let o = s.apply(&t, abs(v), target).unwrap();
            assert!(o.value >= target - 1e-6, "moves the same way");
            target = o.value;
            prev_x = v;
        }
        assert_eq!(prev_x, 127);
        assert!((target - 1.0).abs() < 1e-5);
        assert!(s.picked);
    }

    #[test]
    fn relative_steps_and_limits() {
        let t = Transform {
            mode: Mode::Relative(Encoding::SignMagnitude),
            step: 0.1,
            min: 0.0,
            max: 0.5,
            ..Transform::default()
        };
        let mut s = MapState::default();
        let o = s.apply(&t, abs(3), 0.1).unwrap();
        assert!((o.value - 0.4).abs() < 1e-6);
        let o = s.apply(&t, abs(5), 0.4).unwrap();
        assert!((o.value - 0.5).abs() < 1e-6, "limited to max");
        let o = s.apply(&t, abs(0x41), 0.5).unwrap();
        assert!((o.value - 0.4).abs() < 1e-6);
        assert_eq!(s.apply(&t, abs(0), 0.4), None);
        let inv = Transform { invert: true, ..t };
        let o = s.apply(&inv, abs(1), 0.3).unwrap();
        assert!((o.value - 0.2).abs() < 1e-6);
        let o = s.apply(&t, Input::Relative { steps: -2 }, 0.3).unwrap();
        assert!((o.value - 0.1).abs() < 1e-6);
    }

    #[test]
    fn buttons() {
        let press = |d| Input::Press { down: d, value: if d { 127 } else { 0 } };
        // Toggle flips on each press, ignores releases
        let t = Transform { button: Button::Toggle, ..Transform::default() };
        let mut s = MapState::default();
        let o = s.apply(&t, press(true), 0.0).unwrap();
        assert_eq!((o.value, o.press), (1.0, Some(true)));
        assert_eq!(s.apply(&t, press(false), 1.0), None);
        assert_eq!(s.apply(&t, press(true), 1.0).unwrap().value, 0.0);
        assert_eq!(s.apply(&t, press(false), 0.0), None);
        // A repeated press without a release (a CC held at 127) does not flip again
        assert!(s.apply(&t, press(true), 0.0).is_some());
        assert_eq!(s.apply(&t, press(true), 1.0), None);
        // Momentary follows the key
        let t = Transform { button: Button::Momentary, ..Transform::default() };
        let mut s = MapState::default();
        assert_eq!(s.apply(&t, press(true), 0.0).unwrap().value, 1.0);
        let o = s.apply(&t, press(false), 1.0).unwrap();
        assert_eq!((o.value, o.press), (0.0, Some(false)));
        // Trigger fires on the press only
        let t = Transform { button: Button::Trigger, ..Transform::default() };
        let mut s = MapState::default();
        assert_eq!(s.apply(&t, press(true), 0.0).unwrap().press, Some(true));
        assert_eq!(s.apply(&t, press(false), 1.0), None);
        // Range: a toggle between 0.25 and 0.75
        let t = Transform { button: Button::Toggle, min: 0.25, max: 0.75, ..Transform::default() };
        let mut s = MapState::default();
        assert_eq!(s.apply(&t, press(true), 0.25).unwrap().value, 0.75);
    }
}
