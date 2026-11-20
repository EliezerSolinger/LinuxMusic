//! sfxr (legado/plugins/Sfxr/Sfxr.cpp, `SfxrInstrumentView`): the 250×250 artwork, a grid of 20×20 styled knobs
//! colored by their group (the style sheet's `#envKnob`, `#freqKnob`…), the four wave buttons, and the sound
//! generators (pick up, laser, explosion, power up, hit, jump, blip, randomize, mutate), each a push button whose
//! values are the legacy's (`genPickup`…`mutate`), written in one step. As in the legacy, every value a generator
//! writes is kept within 0–1 (the legacy's knobs were all `SfxrZeroToOneFloatModel`) at the knobs' step of 0.001.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use lm_port::Value;

use crate::asset;
use crate::assets::Asset;
use crate::raster::Rgba;
use crate::spec::{Bind, Control, PanelSpec, PressCx, Target, button, knob};
use crate::widgets::{KnobStyle, Styled};

pub const ID: &str = "sfxr";

macro_rules! pair {
    ($on:ident, $off:ident, $name:literal) => {
        static $on: Asset = asset!(concat!("sfxr/", $name, "_active.png"));
        static $off: Asset = asset!(concat!("sfxr/", $name, "_inactive.png"));
    };
}

static ARTWORK: Asset = asset!("sfxr/artwork.png");
pair!(SQR_ON, SQR_OFF, "sfxr_square_wave");
pair!(SAW_ON, SAW_OFF, "sfxr_saw_wave");
pair!(SIN_ON, SIN_OFF, "sfxr_sin_wave");
pair!(NOISE_ON, NOISE_OFF, "sfxr_white_noise_wave");
pair!(PICKUP_ON, PICKUP_OFF, "pickup");
pair!(LASER_ON, LASER_OFF, "laser");
pair!(EXPLOSION_ON, EXPLOSION_OFF, "explosion");
pair!(POWERUP_ON, POWERUP_OFF, "powerup");
pair!(HIT_ON, HIT_OFF, "hit");
pair!(JUMP_ON, JUMP_OFF, "jump");
pair!(BLIP_ON, BLIP_OFF, "blip");
pair!(RANDOM_ON, RANDOM_OFF, "randomize");
pair!(MUTATE_ON, MUTATE_OFF, "mutate");

pub static ASSETS: &[&Asset] = &[
    &ARTWORK,
    &SQR_ON,
    &SQR_OFF,
    &SAW_ON,
    &SAW_OFF,
    &SIN_ON,
    &SIN_OFF,
    &NOISE_ON,
    &NOISE_OFF,
    &PICKUP_ON,
    &PICKUP_OFF,
    &LASER_ON,
    &LASER_OFF,
    &EXPLOSION_ON,
    &EXPLOSION_OFF,
    &POWERUP_ON,
    &POWERUP_OFF,
    &HIT_ON,
    &HIT_OFF,
    &JUMP_ON,
    &JUMP_OFF,
    &BLIP_ON,
    &BLIP_OFF,
    &RANDOM_ON,
    &RANDOM_OFF,
    &MUTATE_ON,
    &MUTATE_OFF,
];

/// `SfxrKnob` (20×20, centered at 10) with the style sheet's colors of its group
fn sfxr_knob(color: u32, outer: u32) -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 20.0,
        h: 20.0,
        color: Rgba::hex(color),
        outer: Some(Rgba::hex(outer)),
        inner_radius: 2.0,
        outer_radius: 9.0,
        center: (10.0, 10.0),
        line_width: 2.0,
    })
}

const ENV: (u32, u32) = (0x263352, 0x4b66a4);
const FREQ: (u32, u32) = (0x1e4a22, 0x3c9544);
const CHANGE: (u32, u32) = (0x591c1c, 0xb23737);
const SQR: (u32, u32) = (0x3b2714, 0x724c27);
const REPEAT: (u32, u32) = (0x292929, 0x515151);
const PHASER: (u32, u32) = (0x144c4d, 0x299899);
const FILTER: (u32, u32) = (0x47224c, 0x8e4397);

/// One knob: (key, column, row, group, the legacy name, the hint when it differs)
type KnobRow = (&'static str, u32, u32, (u32, u32), &'static str, Option<&'static str>);

/// The knobs
const KNOBS: [KnobRow; 22] = [
    ("attackTime", 0, 0, ENV, "Attack Time", None),
    ("sustainTime", 1, 0, ENV, "Sustain Time", None),
    ("sustainPunch", 2, 0, ENV, "Sustain Punch", None),
    ("decayTime", 3, 0, ENV, "Decay Time", None),
    ("startFrequency", 0, 1, FREQ, "Start Frequency", None),
    ("minFrequency", 1, 1, FREQ, "Min Frequency", None),
    ("slide", 2, 1, FREQ, "Slide", None),
    ("deltaSlide", 3, 1, FREQ, "Delta Slide", None),
    ("vibratoDepth", 4, 1, FREQ, "Vibrato Depth", None),
    ("vibratoSpeed", 5, 1, FREQ, "Vibrato Speed", None),
    ("changeAmount", 0, 2, CHANGE, "Change Amount", None),
    ("changeSpeed", 1, 2, CHANGE, "Change Speed", None),
    ("squareDuty", 3, 2, SQR, "Square Duty", Some("Square Duty (Square wave only):")),
    ("dutySweep", 4, 2, SQR, "Duty Sweep", Some("Duty Sweep (Square wave only):")),
    ("repeatSpeed", 0, 3, REPEAT, "Repeat Speed", None),
    ("phaserOffset", 3, 3, PHASER, "Phaser Offset", None),
    ("phaserSweep", 4, 3, PHASER, "Phaser Sweep", None),
    ("lpFilterCutoff", 0, 4, FILTER, "LP Filter Cutoff", None),
    ("lpFilterCutoffSweep", 1, 4, FILTER, "LP Filter Cutoff Sweep", None),
    ("lpFilterResonance", 2, 4, FILTER, "LP Filter Resonance", None),
    ("hpFilterCutoff", 3, 4, FILTER, "HP Filter Cutoff", None),
    ("hpFilterCutoffSweep", 4, 4, FILTER, "HP Filter Cutoff Sweep", None),
];

/// The wave keys in the legacy model's order (`m_waveFormModel`: 0 square, 1 saw, 2 sine, 3 noise)
const WAVES: [&str; 4] = ["square", "saw", "sine", "noise"];

/// A generator's working copy of the knobs, as the legacy's models (clamped to 0–1, rounded to 0.001)
struct Gen<'a, 'b> {
    cx: &'a mut PressCx<'b>,
    knobs: BTreeMap<&'static str, f64>,
    wave: Option<usize>,
}

impl<'a, 'b> Gen<'a, 'b> {
    /// `resetModels()` when `reset`, else the present values
    fn new(cx: &'a mut PressCx<'b>, reset: bool) -> Self {
        let knobs = KNOBS.iter().map(|k| (k.0, if reset { (cx.default)(k.0) } else { (cx.value)(k.0) })).collect();
        Gen { cx, knobs, wave: reset.then_some(0) }
    }
    fn get(&self, k: &'static str) -> f64 {
        self.knobs[k]
    }
    /// `setValue`: within 0–1 at the step of 0.001
    fn set(&mut self, k: &'static str, v: f64) {
        self.knobs.insert(k, ((v.clamp(0.0, 1.0) / 0.001).round() * 0.001).clamp(0.0, 1.0));
    }
    /// `fastRandInc(upper)` for a float
    fn up(&mut self, upper: f64) -> f64 {
        (self.cx.rand)() * upper
    }
    /// `fastRandInc(lo, hi)`
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.cx.rand)() * (hi - lo)
    }
    /// `oneIn(n)`
    fn one_in(&mut self, n: u32) -> bool {
        ((self.cx.rand)() * f64::from(n)) as u32 == 0
    }
    /// `fastRandInc(n)` for an int: 0..=n
    fn int(&mut self, n: u32) -> usize {
        (((self.cx.rand)() * f64::from(n + 1)) as usize).min(n as usize)
    }
    fn done(self) -> Vec<(Target, Value)> {
        let mut out: Vec<(Target, Value)> =
            self.knobs.into_iter().map(|(k, v)| (Target::Param(k), Value::real(v).unwrap_or_default())).collect();
        if let Some(w) = self.wave {
            out.push((Target::Field("wave"), Value::text(WAVES[w.min(3)])));
        }
        out
    }
}

fn pickup(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let mut g = Gen::new(cx, true);
    let v = g.range(0.4, 0.9);
    g.set("startFrequency", v);
    g.set("attackTime", 0.0);
    let v = g.up(0.1);
    g.set("sustainTime", v);
    let v = g.range(0.1, 0.5);
    g.set("decayTime", v);
    let v = g.range(0.3, 0.6);
    g.set("sustainPunch", v);
    if g.one_in(2) {
        let v = g.range(0.5, 0.7);
        g.set("changeSpeed", v);
        let v = g.range(0.2, 0.6);
        g.set("changeAmount", v);
    }
    g.done()
}

fn laser(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let mut g = Gen::new(cx, true);
    let mut w = g.int(2);
    if w == 2 && g.one_in(2) {
        w = g.int(1);
    }
    g.wave = Some(w);
    let v = g.range(0.5, 1.0);
    g.set("startFrequency", v);
    let v = (g.get("startFrequency") - g.range(0.2, 0.8)).max(0.2);
    g.set("minFrequency", v);
    let v = g.range(-0.15, -0.35);
    g.set("slide", v);
    if g.one_in(3) {
        let v = g.range(0.3, 0.9);
        g.set("startFrequency", v);
        let v = g.up(0.1);
        g.set("minFrequency", v);
        let v = g.range(-0.65, -0.35);
        g.set("slide", v);
    }
    if g.one_in(2) {
        let v = g.up(0.5);
        g.set("squareDuty", v);
        g.set("dutySweep", 0.2);
    } else {
        let v = g.range(0.4, 0.9);
        g.set("squareDuty", v);
        let v = g.up(-0.7);
        g.set("dutySweep", v);
    }
    g.set("attackTime", 0.0);
    let v = g.range(0.1, 0.3);
    g.set("sustainTime", v);
    let v = g.up(0.4);
    g.set("decayTime", v);
    if g.one_in(2) {
        let v = g.up(0.3);
        g.set("sustainPunch", v);
    }
    if g.one_in(3) {
        let v = g.up(0.2);
        g.set("phaserOffset", v);
        let v = g.up(-0.2);
        g.set("phaserSweep", v);
    }
    if g.one_in(2) {
        let v = g.up(0.3);
        g.set("hpFilterCutoff", v);
    }
    g.done()
}

fn explosion(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let mut g = Gen::new(cx, true);
    g.wave = Some(3);
    if g.one_in(2) {
        let v = g.range(0.1, 0.5);
        g.set("startFrequency", v);
        let v = g.range(-0.1, 0.3);
        g.set("slide", v);
    } else {
        let v = g.range(0.2, 0.9);
        g.set("startFrequency", v);
        let v = g.range(-0.4, -0.2);
        g.set("slide", v);
    }
    let f = g.get("startFrequency");
    g.set("startFrequency", f * f);
    if g.one_in(5) {
        g.set("slide", 0.0);
    }
    if g.one_in(3) {
        let v = g.range(0.3, 0.8);
        g.set("repeatSpeed", v);
    }
    g.set("attackTime", 0.0);
    let v = g.range(0.1, 0.4);
    g.set("sustainTime", v);
    g.set("decayTime", 0.5);
    if g.one_in(2) {
        let v = g.range(-0.3, 0.6);
        g.set("phaserOffset", v);
        let v = g.up(-0.3);
        g.set("phaserSweep", v);
    }
    let v = g.range(0.2, 0.8);
    g.set("sustainPunch", v);
    if g.one_in(2) {
        let v = g.up(0.7);
        g.set("vibratoDepth", v);
        let v = g.up(0.6);
        g.set("vibratoSpeed", v);
    }
    if g.one_in(3) {
        let v = g.range(0.6, 0.9);
        g.set("changeSpeed", v);
        let v = g.range(-0.8, 0.8);
        g.set("changeAmount", v);
    }
    g.done()
}

fn powerup(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let mut g = Gen::new(cx, true);
    if g.one_in(2) {
        g.wave = Some(1);
    } else {
        let v = g.up(0.6);
        g.set("squareDuty", v);
    }
    if g.one_in(2) {
        let v = g.range(0.2, 0.5);
        g.set("startFrequency", v);
        let v = g.range(0.1, 0.5);
        g.set("slide", v);
        let v = g.range(0.4, 0.8);
        g.set("repeatSpeed", v);
    } else {
        let v = g.range(0.2, 0.5);
        g.set("startFrequency", v);
        let v = g.range(0.05, 0.25);
        g.set("slide", v);
        if g.one_in(2) {
            let v = g.up(0.7);
            g.set("vibratoDepth", v);
            let v = g.up(0.6);
            g.set("vibratoSpeed", v);
        }
    }
    g.set("attackTime", 0.0);
    let v = g.up(0.4);
    g.set("sustainTime", v);
    let v = g.range(0.1, 0.5);
    g.set("decayTime", v);
    g.done()
}

fn hit(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let mut g = Gen::new(cx, true);
    let mut w = g.int(2);
    if w == 2 {
        w = 3;
    }
    g.wave = Some(w);
    if w == 0 {
        let v = g.up(0.6);
        g.set("squareDuty", v);
    }
    let v = g.range(0.2, 0.8);
    g.set("startFrequency", v);
    let v = g.range(-0.7, -0.3);
    g.set("slide", v);
    g.set("attackTime", 0.0);
    let v = g.up(0.1);
    g.set("sustainTime", v);
    let v = g.range(0.1, 0.3);
    g.set("decayTime", v);
    if g.one_in(2) {
        let v = g.up(0.3);
        g.set("hpFilterCutoff", v);
    }
    g.done()
}

fn jump(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let mut g = Gen::new(cx, true);
    g.wave = Some(0);
    let v = g.up(0.6);
    g.set("squareDuty", v);
    let v = g.range(0.3, 0.6);
    g.set("startFrequency", v);
    let v = g.range(0.1, 0.3);
    g.set("slide", v);
    g.set("attackTime", 0.0);
    let v = g.range(0.1, 0.4);
    g.set("sustainTime", v);
    let v = g.range(0.1, 0.3);
    g.set("decayTime", v);
    if g.one_in(2) {
        let v = g.up(0.3);
        g.set("hpFilterCutoff", v);
    }
    if g.one_in(2) {
        let v = g.range(0.4, 1.0);
        g.set("lpFilterCutoff", v);
    }
    g.done()
}

fn blip(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let mut g = Gen::new(cx, true);
    let w = g.int(1);
    g.wave = Some(w);
    if w == 0 {
        let v = g.up(0.6);
        g.set("squareDuty", v);
    }
    let v = g.range(0.2, 0.6);
    g.set("startFrequency", v);
    g.set("attackTime", 0.0);
    let v = g.range(0.1, 0.2);
    g.set("sustainTime", v);
    let v = g.up(0.2);
    g.set("decayTime", v);
    g.set("hpFilterCutoff", 0.1);
    g.done()
}

/// `randomize()`: every knob from a random curve; the wave is kept
fn randomize(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let mut g = Gen::new(cx, false);
    let r = |g: &mut Gen<'_, '_>, n: i32| g.range(-1.0, 1.0).powi(n);
    let v = r(&mut g, 2);
    g.set("startFrequency", v);
    if g.one_in(2) {
        let v = r(&mut g, 3) + 0.5;
        g.set("startFrequency", v);
    }
    g.set("minFrequency", 0.0);
    let v = r(&mut g, 5);
    g.set("slide", v);
    if g.get("startFrequency") > 0.7 && g.get("slide") > 0.2 {
        let v = -g.get("slide");
        g.set("slide", v);
    }
    if g.get("startFrequency") < 0.2 && g.get("slide") < -0.05 {
        let v = -g.get("slide");
        g.set("slide", v);
    }
    for (k, n) in [("deltaSlide", 3), ("squareDuty", 1), ("dutySweep", 3), ("vibratoDepth", 3), ("vibratoSpeed", 1)] {
        let v = r(&mut g, n);
        g.set(k, v);
    }
    let v = r(&mut g, 3);
    g.set("attackTime", v);
    let v = r(&mut g, 2);
    g.set("sustainTime", v);
    let v = r(&mut g, 1);
    g.set("decayTime", v);
    let v = g.up(0.8).powi(2);
    g.set("sustainPunch", v);
    if g.get("attackTime") + g.get("sustainTime") + g.get("decayTime") < 0.2 {
        let v = g.get("sustainTime") + g.range(0.2, 0.5);
        g.set("sustainTime", v);
        let v = g.get("decayTime") + g.range(0.2, 0.5);
        g.set("decayTime", v);
    }
    let v = r(&mut g, 1);
    g.set("lpFilterResonance", v);
    let v = 1.0 - g.up(1.0).powi(3);
    g.set("lpFilterCutoff", v);
    let v = r(&mut g, 3);
    g.set("lpFilterCutoffSweep", v);
    if g.get("lpFilterCutoff") < 0.1 && g.get("lpFilterCutoffSweep") < -0.05 {
        let v = -g.get("lpFilterCutoffSweep");
        g.set("lpFilterCutoffSweep", v);
    }
    let v = g.up(1.0).powi(5);
    g.set("hpFilterCutoff", v);
    for (k, n) in [
        ("hpFilterCutoffSweep", 5),
        ("phaserOffset", 3),
        ("phaserSweep", 3),
        ("repeatSpeed", 1),
        ("changeSpeed", 1),
        ("changeAmount", 1),
    ] {
        let v = r(&mut g, n);
        g.set(k, v);
    }
    g.done()
}

/// `mutate()`: each knob but the minimum frequency, half of the time, moved by up to ±0.05
fn mutate(cx: &mut PressCx<'_>) -> Vec<(Target, Value)> {
    let mut g = Gen::new(cx, false);
    for (k, ..) in KNOBS {
        if k == "minFrequency" {
            continue;
        }
        if g.one_in(2) {
            let v = g.get(k) + g.range(-0.05, 0.05);
            g.set(k, v);
        }
    }
    g.done()
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls: Vec<Control> = KNOBS
        .iter()
        .map(|&(key, col, row, (color, outer), name, hint)| {
            let hint: &'static str = hint.unwrap_or_else(|| Box::leak(format!("{name}:").into_boxed_str()));
            knob(
                key,
                20.0 + 40.0 * col as f32,
                50.0 + 40.0 * row as f32,
                sfxr_knob(color, outer),
                0.001,
                name,
                hint,
                "",
            )
            .tip(name)
        })
        .collect();
    let waves: [(&'static Asset, &'static Asset, &str); 4] = [
        (&SQR_ON, &SQR_OFF, "Square Wave"),
        (&SAW_ON, &SAW_OFF, "Saw Wave"),
        (&SIN_ON, &SIN_OFF, "Sine Wave"),
        (&NOISE_ON, &NOISE_OFF, "Noise"),
    ];
    for (i, (on, off, tip)) in waves.into_iter().enumerate() {
        let key = WAVES[i];
        let id: &'static str = Box::leak(format!("wave.{key}").into_boxed_str());
        controls.push(
            button(Bind::Choice { field: "wave", key }, id, 20.0 + 16.0 * i as f32, 15.0, on, off, "Wave").tip(tip),
        );
    }
    let gens: [(&str, crate::spec::PressFn, &'static Asset, &'static Asset, &str); 7] = [
        ("pickup", pickup, &PICKUP_ON, &PICKUP_OFF, "Generate pick up/coin sfx"),
        ("laser", laser, &LASER_ON, &LASER_OFF, "Generate laser/shoot sfx"),
        ("explosion", explosion, &EXPLOSION_ON, &EXPLOSION_OFF, "Generate explosion sfx"),
        ("powerup", powerup, &POWERUP_ON, &POWERUP_OFF, "Generate power up sfx"),
        ("hit", hit, &HIT_ON, &HIT_OFF, "Generate hit/hurt sfx"),
        ("jump", jump, &JUMP_ON, &JUMP_OFF, "Generate jump sfx"),
        ("blip", blip, &BLIP_ON, &BLIP_OFF, "Generate blip/select sfx"),
    ];
    for (i, (id, f, on, off, tip)) in gens.into_iter().enumerate() {
        controls.push(button(Bind::Press(f), id, 110.0 + 16.0 * i as f32, 25.0, on, off, tip).tip(tip));
    }
    controls.push(
        button(Bind::Press(randomize), "randomize", 160.0, 4.0, &RANDOM_ON, &RANDOM_OFF, "Generate random sfx")
            .tip("Generate random sfx"),
    );
    controls.push(
        button(Bind::Press(mutate), "mutate", 205.0, 4.0, &MUTATE_ON, &MUTATE_OFF, "Mutate sfx").tip("Mutate sfx"),
    );
    PanelSpec {
        plugin: ID,
        size: (250.0, 250.0),
        background: Some(&ARTWORK),
        fill: 0x000000,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});
