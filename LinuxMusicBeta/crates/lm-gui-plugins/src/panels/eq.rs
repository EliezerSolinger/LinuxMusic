//! Eq (legado/plugins/Eq/EqControlsDialog.cpp, `EqParameterWidget.cpp`, `EqCurve.cpp`): the 500×500 artwork with the
//! 450×200 curve display at (26, 17): eight handles (high-pass, low shelf, four peaks, high shelf, low-pass; their
//! numbered pixmaps, dimmed while the band is off) dragged to set the band's frequency (logarithmic, 20 Hz to 20 kHz
//! across) and gain (±18 dB over the height; the passes stay at 0 dB), the wheel over a handle changing its
//! bandwidth or resonance (0.15 a notch, Ctrl 0.01), and the white response curve: the sum of the active bands' own
//! biquad responses, which the legacy computed from the handles (a 24 dB/oct pass draws its response doubled, a
//! 48 dB/oct pass tripled). Under it the band switches (the numbered labels), the input, output and the six bands'
//! gain faders (the input and output ones with their peak meters from the plugin's live frame), the frequency and
//! resonance (bandwidth for the peaks) knobs, the slope buttons of the two passes and the two analyser LEDs.
//!
//! Under the curve the two spectrum analysers (the legacy `EqSpectrumView`s, input in blue and output in cyan, 150
//! alpha), drawn from the plugin's live frame (`lm_plugins_effects::eq::FRAME_FLOATS`: the plugin runs the FFT on the
//! audio side and publishes 256 log-spaced bands in dBFS, NaN when its analyser is off).
//!
//! The info box of a hovered handle (frequency and resonance or bandwidth) shows over it, and moving a band's knob or
//! fader (or dragging its handle) turns the band on.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::{Canvas, Cap, Paint, Rgba};
use crate::spec::{
    Bind, Control, PanelSpec, SurfText, SurfaceCx, SurfaceHandle, SurfaceSpec, button, fader, knob, led, surface,
};
use crate::widgets::{KnobStyle, LedColor};

pub const ID: &str = "eq";

static ARTWORK: Asset = asset!("eq/artwork.png");
static HOVER: Asset = asset!("eq/handlehover.png");
static HANDLE1: Asset = asset!("eq/handle1.png");
static HANDLE1_OFF: Asset = asset!("eq/handle1inactive.png");
static LABEL1: Asset = asset!("eq/bandLabel1.png");
static LABEL1_OFF: Asset = asset!("eq/bandLabel1off.png");
static HANDLE2: Asset = asset!("eq/handle2.png");
static HANDLE2_OFF: Asset = asset!("eq/handle2inactive.png");
static LABEL2: Asset = asset!("eq/bandLabel2.png");
static LABEL2_OFF: Asset = asset!("eq/bandLabel2off.png");
static HANDLE3: Asset = asset!("eq/handle3.png");
static HANDLE3_OFF: Asset = asset!("eq/handle3inactive.png");
static LABEL3: Asset = asset!("eq/bandLabel3.png");
static LABEL3_OFF: Asset = asset!("eq/bandLabel3off.png");
static HANDLE4: Asset = asset!("eq/handle4.png");
static HANDLE4_OFF: Asset = asset!("eq/handle4inactive.png");
static LABEL4: Asset = asset!("eq/bandLabel4.png");
static LABEL4_OFF: Asset = asset!("eq/bandLabel4off.png");
static HANDLE5: Asset = asset!("eq/handle5.png");
static HANDLE5_OFF: Asset = asset!("eq/handle5inactive.png");
static LABEL5: Asset = asset!("eq/bandLabel5.png");
static LABEL5_OFF: Asset = asset!("eq/bandLabel5off.png");
static HANDLE6: Asset = asset!("eq/handle6.png");
static HANDLE6_OFF: Asset = asset!("eq/handle6inactive.png");
static LABEL6: Asset = asset!("eq/bandLabel6.png");
static LABEL6_OFF: Asset = asset!("eq/bandLabel6off.png");
static HANDLE7: Asset = asset!("eq/handle7.png");
static HANDLE7_OFF: Asset = asset!("eq/handle7inactive.png");
static LABEL7: Asset = asset!("eq/bandLabel7.png");
static LABEL7_OFF: Asset = asset!("eq/bandLabel7off.png");
static HANDLE8: Asset = asset!("eq/handle8.png");
static HANDLE8_OFF: Asset = asset!("eq/handle8inactive.png");
static LABEL8: Asset = asset!("eq/bandLabel8.png");
static LABEL8_OFF: Asset = asset!("eq/bandLabel8off.png");
static DB12: Asset = asset!("eq/12dB.png");
static DB12_OFF: Asset = asset!("eq/12dBoff.png");
static DB24: Asset = asset!("eq/24dB.png");
static DB24_OFF: Asset = asset!("eq/24dBoff.png");
static DB48: Asset = asset!("eq/48dB.png");
static DB48_OFF: Asset = asset!("eq/48dBoff.png");

pub static ASSETS: &[&Asset] = &[
    &ARTWORK,
    &HOVER,
    &HANDLE1,
    &HANDLE1_OFF,
    &LABEL1,
    &LABEL1_OFF,
    &HANDLE2,
    &HANDLE2_OFF,
    &LABEL2,
    &LABEL2_OFF,
    &HANDLE3,
    &HANDLE3_OFF,
    &LABEL3,
    &LABEL3_OFF,
    &HANDLE4,
    &HANDLE4_OFF,
    &LABEL4,
    &LABEL4_OFF,
    &HANDLE5,
    &HANDLE5_OFF,
    &LABEL5,
    &LABEL5_OFF,
    &HANDLE6,
    &HANDLE6_OFF,
    &LABEL6,
    &LABEL6_OFF,
    &HANDLE7,
    &HANDLE7_OFF,
    &LABEL7,
    &LABEL7_OFF,
    &HANDLE8,
    &HANDLE8_OFF,
    &LABEL8,
    &LABEL8_OFF,
    &DB12,
    &DB12_OFF,
    &DB24,
    &DB24_OFF,
    &DB48,
    &DB48_OFF,
];

const HANDLES: [(&Asset, &Asset); 8] = [
    (&HANDLE1, &HANDLE1_OFF),
    (&HANDLE2, &HANDLE2_OFF),
    (&HANDLE3, &HANDLE3_OFF),
    (&HANDLE4, &HANDLE4_OFF),
    (&HANDLE5, &HANDLE5_OFF),
    (&HANDLE6, &HANDLE6_OFF),
    (&HANDLE7, &HANDLE7_OFF),
    (&HANDLE8, &HANDLE8_OFF),
];

/// A band's parameters: switch, frequency, resonance (bandwidth for the peaks), gain (the passes have none)
struct Band {
    active: &'static str,
    freq: &'static str,
    res: &'static str,
    gain: Option<&'static str>,
    kind: Kind,
    /// The slope field of a pass
    slope: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    HighPass,
    LowShelf,
    Peak,
    HighShelf,
    LowPass,
}

const BANDS: [Band; 8] = [
    Band { active: "hpActive", freq: "hpFreq", res: "hpRes", gain: None, kind: Kind::HighPass, slope: Some("hpSlope") },
    Band {
        active: "lowShelfActive",
        freq: "lowShelfFreq",
        res: "lowShelfRes",
        gain: Some("lowShelfGain"),
        kind: Kind::LowShelf,
        slope: None,
    },
    Band {
        active: "peak1Active",
        freq: "peak1Freq",
        res: "peak1Bw",
        gain: Some("peak1Gain"),
        kind: Kind::Peak,
        slope: None,
    },
    Band {
        active: "peak2Active",
        freq: "peak2Freq",
        res: "peak2Bw",
        gain: Some("peak2Gain"),
        kind: Kind::Peak,
        slope: None,
    },
    Band {
        active: "peak3Active",
        freq: "peak3Freq",
        res: "peak3Bw",
        gain: Some("peak3Gain"),
        kind: Kind::Peak,
        slope: None,
    },
    Band {
        active: "peak4Active",
        freq: "peak4Freq",
        res: "peak4Bw",
        gain: Some("peak4Gain"),
        kind: Kind::Peak,
        slope: None,
    },
    Band {
        active: "highShelfActive",
        freq: "highShelfFreq",
        res: "highShelfRes",
        gain: Some("highShelfGain"),
        kind: Kind::HighShelf,
        slope: None,
    },
    Band { active: "lpActive", freq: "lpFreq", res: "lpRes", gain: None, kind: Kind::LowPass, slope: Some("lpSlope") },
];

const W: f32 = 450.0;
const H: f32 = 200.0;
/// The gain range is -18 to +18 dB over the height
const PX_PER_DB: f32 = H / 36.0;

/// `EqHandle::freqToXPixel`
fn freq_to_x(f: f64) -> f32 {
    if f <= 0.0 {
        return 0.0;
    }
    let (lo, hi) = (20f64.log10(), 20_000f64.log10());
    ((f.log10() - lo) / (hi - lo) * f64::from(W)) as f32
}

/// `EqHandle::xPixelToFreq`
fn x_to_freq(x: f32) -> f64 {
    let (lo, hi) = (20f64.log10(), 20_000f64.log10());
    10f64.powf(f64::from(x) * ((hi - lo) / f64::from(W)) + lo)
}

fn gain_to_y(g: f64) -> f32 {
    H * 0.5 - g as f32 * PX_PER_DB
}

fn y_to_gain(y: f32) -> f64 {
    f64::from((H * 0.5 - y) / PX_PER_DB)
}

/// `EqHandle::calculateGain`: the response in dB of a normalized biquad at `freq`
fn response_db(freq: f64, rate: f64, a1: f64, a2: f64, b0: f64, b1: f64, b2: f64) -> f64 {
    let w = (std::f64::consts::PI * freq / rate).sin();
    let phi = w * w * 4.0;
    let bb = b0 + b1 + b2;
    let aa = 1.0 + a1 + a2;
    10.0 * (bb * bb + (b0 * b2 * phi - (b1 * (b0 + b2) + 4.0 * b0 * b2)) * phi).log10()
        - 10.0 * (aa * aa + (a2 * phi - (a1 * (1.0 + a2) + 4.0 * a2)) * phi).log10()
}

/// One band's response in dB at `freq` (the handle's own curve, `getPeakCurve`, `getLowShelfCurve`...)
fn band_db(cx: &SurfaceCx<'_>, band: &Band, freq: f64) -> f64 {
    use std::f64::consts::{LN_2, PI};
    let rate = cx.rate.max(8000.0);
    let f0 = (cx.param)(band.freq);
    let res = (cx.param)(band.res);
    let gain = band.gain.map_or(0.0, |g| (cx.param)(g));
    let w0 = 2.0 * PI * f0 / rate;
    let (c, s) = (w0.cos(), w0.sin());
    let norm = |b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64| {
        response_db(freq, rate, a1 / a0, a2 / a0, b0 / a0, b1 / a0, b2 / a0)
    };
    match band.kind {
        Kind::Peak => {
            let a = 10f64.powf(gain / 40.0);
            let alpha = s * (LN_2 / 2.0 * res * w0 / s).sinh();
            norm(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a)
        }
        Kind::HighShelf => {
            let a = 10f64.powf(gain * 0.025);
            let beta = a.sqrt() / res;
            norm(
                a * ((a + 1.0) + (a - 1.0) * c + beta * s),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - beta * s),
                (a + 1.0) - (a - 1.0) * c + beta * s,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - beta * s,
            )
        }
        Kind::LowShelf => {
            let a = 10f64.powf(gain / 40.0);
            let beta = a.sqrt() / res;
            norm(
                a * ((a + 1.0) - (a - 1.0) * c + beta * s),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - beta * s),
                (a + 1.0) + (a - 1.0) * c + beta * s,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - beta * s,
            )
        }
        Kind::HighPass | Kind::LowPass => {
            let alpha = s / (2.0 * res);
            let g = if band.kind == Kind::HighPass {
                norm((1.0 + c) * 0.5, -(1.0 + c), (1.0 + c) * 0.5, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
            } else {
                norm((1.0 - c) * 0.5, 1.0 - c, (1.0 - c) * 0.5, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
            };
            // the legacy doubles the response of a 24 dB/oct pass and triples a 48 dB/oct one
            match (cx.field)(band.slope.unwrap_or("")).as_text() {
                Some("24db") => g * 2.0,
                Some("48db") => g * 3.0,
                _ => g,
            }
        }
    }
}

fn active(cx: &SurfaceCx<'_>, band: &Band) -> bool {
    (cx.param)(band.active) >= 0.5
}

/// A handle's place: its frequency across, its gain down (the passes on the middle line)
fn handle_at(cx: &SurfaceCx<'_>, band: &Band) -> (f32, f32) {
    let x = freq_to_x((cx.param)(band.freq));
    let y = band.gain.map_or(H / 2.0, |g| gain_to_y((cx.param)(g)));
    (x, y)
}

/// The plugin's frame (`lm_plugins_effects::eq`, whose constants the test below checks these against)
const FRAME_FLOATS: usize = 8 + 2 * 256;
const SPECTRUM_IN: usize = 8;
const SPECTRUM_OUT: usize = 8 + 256;

/// One analyser's bands under the curve: a column per pixel, from the band the pixel's frequency falls in
fn spectrum(c: &mut Canvas, cx: &SurfaceCx<'_>, at: usize, color: Rgba) {
    let (lo, hi, n) = (cx.live(4, 0.0), cx.live(5, 0.0), cx.live(6, 0.0) as usize);
    if cx.frame.len() < FRAME_FLOATS || lo <= 0.0 || hi <= lo || n == 0 {
        return;
    }
    let span = (f64::from(hi) / f64::from(lo)).ln();
    for x in 0..W as usize {
        let f = x_to_freq(x as f32);
        let band = ((f / f64::from(lo)).ln() / span * n as f64).floor();
        if !(0.0..n as f64).contains(&band) {
            continue;
        }
        let db = cx.frame[at + band as usize];
        if db.is_nan() {
            return;
        }
        // -90 dBFS at the bottom, 0 dBFS at the top
        let h = (H * (db + 90.0) / 90.0).clamp(0.0, H);
        if h >= 1.0 {
            c.fill_rect(x as f32, H - h, 1.0, h, color);
        }
    }
}

fn draw(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    spectrum(c, cx, SPECTRUM_IN, Rgba::alpha(77, 101, 242, 150));
    spectrum(c, cx, SPECTRUM_OUT, Rgba::alpha(0, 255, 239, 150));
    // the response: the middle line plus each active band's offset from it
    let on: Vec<&Band> = BANDS.iter().filter(|b| active(cx, b)).collect();
    if !on.is_empty() {
        let paint = Paint::Solid(Rgba::rgb(255, 255, 255));
        let mut last: Option<(f32, f32)> = None;
        for x in 0..W as usize {
            let f = x_to_freq(x as f32);
            let mut y = H / 2.0;
            for b in &on {
                y += gain_to_y(band_db(cx, b, f)) - H / 2.0;
            }
            let p = (x as f32, y.clamp(-50.0, H + 50.0));
            if let Some(l) = last {
                c.line(l, p, 2.0, Cap::Flat, &paint);
            }
            last = Some(p);
        }
    }
    // the handles, over the curve
    for (i, band) in BANDS.iter().enumerate() {
        let (x, y) = handle_at(cx, band);
        let (on_img, off_img) = HANDLES[i];
        let img = if active(cx, band) { on_img } else { off_img };
        let (iw, ih) = {
            let im = crate::assets::image(img);
            (im.width() as f32, im.height() as f32)
        };
        c.pixmap(
            crate::assets::image(img),
            (x - (iw / 2.0).floor() - 1.0).round(),
            (y - (ih / 2.0).floor()).round(),
            false,
        );
        if cx.active == Some(i) {
            c.pixmap(
                crate::assets::image(&HOVER),
                (x - (iw / 2.0).floor() - 1.0).round(),
                (y - (ih / 2.0).floor()).round(),
                false,
            );
            // the info box: 80×30, black outline, `QColor(6, 106, 43, 180)`
            let (bx, by) = info_box(x, y);
            c.fill_rect(bx, by, 80.0, 30.0, Rgba::rgb(0, 0, 0));
            c.fill_rect(bx + 1.0, by + 1.0, 78.0, 28.0, Rgba::alpha(6, 106, 43, 180));
        }
    }
}

/// The info box's corner for a handle at `x`, `y`: 40 up and left of it, kept inside the curve's area
fn info_box(x: f32, y: f32) -> (f32, f32) {
    let (mut rx, mut ry) = (-40.0, -40.0);
    if y < 40.0 {
        ry += 40.0 - y;
    }
    if x < 40.0 {
        rx += 40.0 - x;
    }
    if x > 410.0 {
        rx -= 40.0 - (W - x);
    }
    (x + rx, y + ry)
}

/// The hovered handle's frequency and resonance (bandwidth for a peak), two lines in the box, white over a black shadow
fn texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
    let Some(i) = cx.active else { return Vec::new() };
    let band = &BANDS[i];
    let (x, y) = handle_at(cx, band);
    let (bx, by) = info_box(x, y);
    let res = if band.kind == Kind::Peak { "BW" } else { "Reso" };
    let lines = [
        format!("Freq: {}", crate::widgets::qt_number((cx.param)(band.freq).round())),
        format!("{res}: {}", crate::widgets::qt_number(((cx.param)(band.res) * 100.0).round() / 100.0)),
    ];
    let mut out = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        for (dx, color) in [(1.0, 0x000000), (0.0, 0xffffff)] {
            out.push(SurfText {
                x: bx + 40.0 + dx,
                y: by + 12.0 + n as f32 * 12.0 + dx,
                px: 10.0,
                text: line.clone(),
                color,
                centered: true,
            });
        }
    }
    out
}

fn handles(cx: &SurfaceCx<'_>) -> Vec<SurfaceHandle> {
    BANDS
        .iter()
        .map(|b| {
            let (x, y) = handle_at(cx, b);
            SurfaceHandle { x, y }
        })
        .collect()
}

/// A handle dragged to `x`, `y`: the band's frequency and gain, and the band on (`EqParameterWidget::updateModels`)
fn drag(_cx: &SurfaceCx<'_>, handle: usize, x: f32, y: f32) -> Vec<(&'static str, f64)> {
    let band = &BANDS[handle];
    let x = x.clamp(0.0, W);
    let mut edits = vec![(band.freq, x_to_freq(x)), (band.active, 1.0)];
    if let Some(g) = band.gain {
        edits.push((g, y_to_gain(y.clamp(0.0, H))));
    }
    edits
}

/// `EqHandle::wheelEvent`: the resonance (the bandwidth of a peak) by 0.15 a notch, 0.01 with Ctrl
fn wheel(cx: &SurfaceCx<'_>, handle: usize, notches: f64, fine: bool) -> Vec<(&'static str, f64)> {
    let band = &BANDS[handle];
    let top = if band.kind == Kind::Peak { 4.0 } else { 10.0 };
    let step = if fine { 0.01 } else { 0.15 };
    vec![(band.res, ((cx.param)(band.res) + notches * step).clamp(0.1, top))]
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls: Vec<Control> = vec![surface(
        SurfaceSpec {
            w: W,
            h: H,
            primary: "hpFreq",
            reads: &[
                "hpActive",
                "hpFreq",
                "hpRes",
                "lowShelfActive",
                "lowShelfFreq",
                "lowShelfRes",
                "lowShelfGain",
                "peak1Active",
                "peak1Freq",
                "peak1Bw",
                "peak1Gain",
                "peak2Active",
                "peak2Freq",
                "peak2Bw",
                "peak2Gain",
                "peak3Active",
                "peak3Freq",
                "peak3Bw",
                "peak3Gain",
                "peak4Active",
                "peak4Freq",
                "peak4Bw",
                "peak4Gain",
                "highShelfActive",
                "highShelfFreq",
                "highShelfRes",
                "highShelfGain",
                "lpActive",
                "lpFreq",
                "lpRes",
            ],
            field_reads: &["hpSlope", "lpSlope"],
            draw,
            texts: Some(texts),
            handles: Some(handles),
            drag: Some(drag),
            grab: 10.0,
            set_on_press: false,
            live: true,
            wheel: false,
            wheel_edit: Some(wheel),
            peaks: None,
            markers: None,
            tag: 0,
        },
        "curve",
        26.0,
        17.0,
        "Equalizer curve",
    )];
    let labels = [
        (&LABEL1, &LABEL1_OFF),
        (&LABEL2, &LABEL2_OFF),
        (&LABEL3, &LABEL3_OFF),
        (&LABEL4, &LABEL4_OFF),
        (&LABEL5, &LABEL5_OFF),
        (&LABEL6, &LABEL6_OFF),
        (&LABEL7, &LABEL7_OFF),
        (&LABEL8, &LABEL8_OFF),
    ];
    let names = [
        ("HP", "HP active"),
        ("Low-shelf", "Low-shelf active"),
        ("Peak 1", "Peak 1 active"),
        ("Peak 2", "Peak 2 active"),
        ("Peak 3", "Peak 3 active"),
        ("Peak 4", "Peak 4 active"),
        ("High-shelf", "High-shelf active"),
        ("LP", "LP active"),
    ];
    let freq_names = [
        "HP freq",
        "Low-shelf freq",
        "Peak 1 freq",
        "Peak 2 freq",
        "Peak 3 freq",
        "Peak 4 freq",
        "High-shelf freq",
        "LP freq",
    ];
    let res_names =
        ["HP res", "Low-shelf res", "Peak 1 BW", "Peak 2 BW", "Peak 3 BW", "Peak 4 BW", "High-shelf res", "LP res"];
    let gain_names =
        ["", "Low-shelf gain", "Peak 1 gain", "Peak 2 gain", "Peak 3 gain", "Peak 4 gain", "High-shelf gain", ""];
    for (i, band) in BANDS.iter().enumerate() {
        let x = 81.0 + 44.0 * i as f32;
        let mut res = knob(band.res, x, 440.0, KnobStyle::Bright26, 0.001, res_names[i], "Resonance:", "");
        if let crate::spec::Kind::Knob(k) = &mut res.kind {
            if band.kind == Kind::Peak {
                k.hint = crate::spec::Text::new("Bandwidth:");
                k.unit = " Octave";
            }
        }
        controls.push(res.wakes(band.active));
        controls.push(
            knob(band.freq, x, 396.0, KnobStyle::Bright26, 0.001, freq_names[i], "Frequency:", "Hz")
                .log()
                .wakes(band.active),
        );
        let (on, off) = labels[i];
        controls.push(
            button(Bind::Switch { param: band.active }, band.active, x - 2.0, 276.0, on, off, names[i].1)
                .tip(names[i].0),
        );
        if band.gain.is_some() && i >= 1 && i <= 6 {
            controls.push(
                fader(band.gain.unwrap_or(""), 126.0 + 44.0 * (i as f32 - 1.0), 295.0, 80.0, gain_names[i], "Gain")
                    .fader_unit(" dB")
                    .unity_line()
                    .wakes(band.active),
            );
        }
    }
    controls
        .push(fader("inputGain", 23.0, 295.0, 80.0, "Input gain", "Gain").fader_unit(" dBv").meters(0, 1).unity_line());
    controls.push(
        fader("outputGain", 453.0, 295.0, 80.0, "Output gain", "Gain").fader_unit(" dBv").meters(2, 3).unity_line(),
    );
    controls.push(led(
        Bind::Flag { field: "analyseIn" },
        "analyseIn",
        172.0,
        240.0,
        LedColor::Yellow,
        "",
        "Analyse IN",
    ));
    controls.push(led(
        Bind::Flag { field: "analyseOut" },
        "analyseOut",
        302.0,
        240.0,
        LedColor::Yellow,
        "",
        "Analyse OUT",
    ));
    for (field, x, group) in [("hpSlope", 79.0, "hp"), ("lpSlope", 387.0, "lp")] {
        for (key, y, on, off) in
            [("12db", 298.0, &DB12, &DB12_OFF), ("24db", 328.0, &DB24, &DB24_OFF), ("48db", 358.0, &DB48, &DB48_OFF)]
        {
            let id = lm_port::intern(&format!("{group}{key}"));
            controls.push(button(
                Bind::Choice { field, key },
                id,
                x,
                y,
                on,
                off,
                if group == "hp" { "High-pass type" } else { "Low-pass type" },
            ));
        }
    }
    PanelSpec {
        plugin: ID,
        size: (500.0, 500.0),
        background: Some(&ARTWORK),
        view_backgrounds: &[],
        fill: 0x000000,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});

#[cfg(test)]
mod frame_layout {
    #[test]
    fn the_panel_reads_the_plugins_frame() {
        use lm_plugins_effects::eq as plugin;
        assert_eq!(super::FRAME_FLOATS, plugin::FRAME_FLOATS);
        assert_eq!(super::SPECTRUM_IN, plugin::SPECTRUM_IN);
        assert_eq!(super::SPECTRUM_OUT, plugin::SPECTRUM_OUT);
    }
}
