//! Spectrum Analyzer (legado/plugins/SpectrumAnalyzer: SaControlsDialog.cpp, SaSpectrumView.cpp,
//! SaWaterfallView.cpp): the control section on top (pause and reference freeze, the four LEDs, the log/linear
//! buttons with the frequency and amplitude range combos, the FFT block size and window combos, and the button to the
//! advanced knobs, which take the section's place), the spectrum view, and the waterfall.
//!
//! Both views draw the plugin's live frame (`lm-plugins-effects`' `spectrum_analyzer`): the spectrum as the legacy's
//! filled curve (the left side, then the right, in their colors, or one mono color) with the peak or reference curve
//! over it, on the legacy's grid and labels (the same tick series, a black field, `#1e2226` grid, `#c0cad4` labels);
//! the waterfall as the history image scaled to its field, newest line on top, with its time labels.
//!
//! Not ported: the cursor's cross-hair with the frequency and level under it, and the two "resolution" knobs, which
//! are kept as parameters but have nothing to tune (the curves are drawn at one point per pixel). The section is a
//! fixed size, not a splitter, and the waterfall's field is always there (black while the waterfall is off).

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::raster::{Canvas, Cap, Paint, Rgba};
use crate::spec::{
    Bind, ComboItem, PanelSpec, SurfText, SurfaceCx, SurfaceSpec, Text, button, combo, knob, led, surface,
};
use crate::widgets::{KnobStyle, LedColor};

pub const ID: &str = "spectrumanalyzer";

static ART0: Asset = asset!("spectrumanalyzer/artwork0.png");
static ART1: Asset = asset!("spectrumanalyzer/artwork1.png");
static PLAY: Asset = asset!("spectrumanalyzer/play.png");
static PAUSE: Asset = asset!("spectrumanalyzer/pause.png");
static FREEZE: Asset = asset!("spectrumanalyzer/freeze.png");
static FREEZE_OFF: Asset = asset!("spectrumanalyzer/freeze_off.png");
static X_LOG: Asset = asset!("spectrumanalyzer/x_log.png");
static X_LINEAR: Asset = asset!("spectrumanalyzer/x_linear.png");
static Y_LOG: Asset = asset!("spectrumanalyzer/y_log.png");
static Y_LINEAR: Asset = asset!("spectrumanalyzer/y_linear.png");
static ADV_ON: Asset = asset!("spectrumanalyzer/advanced_on.png");
static ADV_OFF: Asset = asset!("spectrumanalyzer/advanced_off.png");
// the two icons beside the combos are in the artwork; these are the sources
static BLOCK_ICON: Asset = asset!("spectrumanalyzer/block_size.png");
static WINDOW_ICON: Asset = asset!("spectrumanalyzer/window.png");

static VIEWS: [&Asset; 2] = [&ART0, &ART1];

pub static ASSETS: &[&Asset] = &[
    &ART0,
    &ART1,
    &PLAY,
    &PAUSE,
    &FREEZE,
    &FREEZE_OFF,
    &X_LOG,
    &X_LINEAR,
    &Y_LOG,
    &Y_LINEAR,
    &ADV_ON,
    &ADV_OFF,
    &BLOCK_ICON,
    &WINDOW_ICON,
];

// the frame of the plugin (`lm_plugins_effects::spectrum_analyzer`)
const SPEC_COLS: usize = 1024;
const WF_COLS: usize = 256;
const WF_ROWS: usize = 96;
const HEADER: usize = 8;
const BLOCKS: [usize; 7] = [256, 512, 1024, 2048, 4096, 8192, 16384];

const W: f32 = 540.0;
const SPEC_Y: f32 = 88.0;
const SPEC_H: f32 = 200.0;
const WF_Y: f32 = 290.0;
const WF_H: f32 = 170.0;

const COLOR_L: Rgba = Rgba::alpha(51, 148, 204, 135);
const COLOR_R: Rgba = Rgba::alpha(204, 107, 51, 135);
const COLOR_MONO: Rgba = Rgba::alpha(51, 148, 204, 204);
const COLOR_BG: Rgba = Rgba::rgb(7, 7, 7);
const COLOR_GRID: Rgba = Rgba::rgb(30, 34, 38);
const LABELS: u32 = 0xc0cad4;
const LABEL_PX: f32 = 10.0;

fn text_of(v: lm_port::Value) -> String {
    v.as_text().unwrap_or("").to_owned()
}

/// `getFreqRangeMin/Max` (the full range starts at 0 on the linear axis, at 5 Hz on the logarithmic one)
fn freq_range(cx: &SurfaceCx<'_>, log: bool) -> (f32, f32) {
    let nyquist = cx.live(1, cx.rate as f32) / 2.0;
    match text_of((cx.field)("freqRange")).as_str() {
        "audible" => (20.0, 20_000.0),
        "bass" => (20.0, 300.0),
        "mids" => (200.0, 5_000.0),
        "high" => (4_000.0, 20_000.0),
        _ => (if log { 5.0 } else { 0.0 }, nyquist.max(100.0)),
    }
}

fn amp_range(cx: &SurfaceCx<'_>) -> (f32, f32) {
    match text_of((cx.field)("ampRange")).as_str() {
        "extended" => (-80.0, 20.0),
        "loud" => (-30.0, 0.0),
        "silent" => (-60.0, -10.0),
        _ => (-50.0, 0.0),
    }
}

/// `freqToXPixel`
fn freq_x(f: f32, (min, max): (f32, f32), log: bool, width: f32) -> f32 {
    if log {
        if f <= 1.0 {
            return 0.0;
        }
        let lo = min.log10();
        (f.log10() - lo) / (max.log10() - lo) * width
    } else {
        (f - min) / (max - min) * width
    }
}

/// `ampToYPixel`: the amplitude's place down a field `height` high
fn amp_y(amp: f32, (min, max): (f32, f32), log: bool, height: f32) -> f32 {
    if log {
        let db = 10.0 * amp.max(f32::MIN_POSITIVE).log10();
        if db < min { height } else { (db - max) / (min - max) * height }
    } else {
        let top = 10f32.powf(max / 10.0);
        let range = 10f32.powf(min / 10.0) - top;
        (amp - top) / range * height
    }
}

fn khz(f: i32) -> String {
    if f < 1000 { f.to_string() } else { format!("{}k", f / 1000) }
}

/// `makeLogFreqTics`
fn log_freq_tics(low: i32, high: i32) -> Vec<i32> {
    let mut out = Vec::new();
    let (a, b) = ([10, 20, 50], [14, 30, 70]);
    let mut i = 1;
    while i <= high {
        for j in 0..3 {
            if i * a[j] >= low && i * a[j] <= high {
                out.push(i * a[j]);
            }
            if ((high as f32).log10() - (low as f32).log10() < 2.0) && i * b[j] >= low && i * b[j] <= high {
                out.push(i * b[j]);
            }
        }
        i *= 10;
    }
    out
}

/// `makeLinearFreqTics`
fn linear_freq_tics(low: i32, high: i32) -> Vec<i32> {
    let step = match high - low {
        d if d < 500 => 50,
        d if d < 1000 => 100,
        d if d < 5000 => 1000,
        _ => 2000,
    };
    (0..=high / step).map(|k| k * step).filter(|f| *f >= low).collect()
}

/// `makeLogAmpTics`: dB lines as (linear amplitude, label)
fn log_amp_tics((low, high): (f32, f32)) -> Vec<(f32, String)> {
    let increment = if high - low < 20.0 {
        10f32.powf(0.3)
    } else if high - low < 45.0 {
        10f32.powf(0.6)
    } else {
        10.0
    };
    let mut out = Vec::new();
    let mut i = 1.0e-9f32;
    while 10.0 * i.log10() <= high + 0.001 {
        if 10.0 * i.log10() >= low - 0.001 {
            out.push((i, format!("{}", (10.0 * i.log10()).round() as i32)));
        }
        i *= increment;
    }
    out
}

/// `makeLinearAmpTics`
fn linear_amp_tics((low, high): (f32, f32)) -> Vec<(f32, String)> {
    let (lo, hi) = (10f32.powf(low / 10.0), 10f32.powf(high / 10.0));
    let split = 5.0;
    let mut out = Vec::new();
    let mut i = 0.0f32;
    while i <= hi + 0.0001 {
        if i >= lo - 0.0001 {
            let label = if i >= 9.99 && i < 99.9 {
                format!("{}", i.round() as i32)
            } else if i >= 0.099 {
                format!("{:.1}", (i * 10.0).round() / 10.0)
            } else if i >= 0.0099 {
                format!("{:.2}", (i * 1000.0).round() / 1000.0)
            } else if i >= 0.00099 {
                format!("{:.3}", (i * 10000.0).round() / 10000.0).trim_start_matches('0').to_owned()
            } else {
                "0".to_owned()
            };
            out.push((i, label));
        }
        i += (hi - lo) / split;
    }
    out
}

struct Geometry {
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

fn spectrum_geometry(cx: &SurfaceCx<'_>) -> Geometry {
    Geometry { left: 26.0, right: cx.w - 26.0, top: 1.0, bottom: cx.h - 20.0 }
}

fn ticks(cx: &SurfaceCx<'_>) -> (Vec<(f32, String)>, Vec<(f32, String)>) {
    let log_x = (cx.param)("logX") >= 0.5;
    let log_y = (cx.param)("logY") >= 0.5;
    let g = spectrum_geometry(cx);
    let width = g.right - g.left;
    let range = freq_range(cx, log_x);
    let tics = if log_x {
        log_freq_tics(range.0 as i32, range.1 as i32)
    } else {
        linear_freq_tics(range.0 as i32, range.1 as i32)
    };
    let freq = tics.into_iter().map(|f| (freq_x(f as f32, range, log_x, width), khz(f))).collect();
    let amps = amp_range(cx);
    let height = g.bottom;
    let rows = if log_y { log_amp_tics(amps) } else { linear_amp_tics(amps) };
    let amp = rows.into_iter().map(|(a, label)| (amp_y(a, amps, log_y, height), label)).collect();
    (freq, amp)
}

fn draw_spectrum(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let g = spectrum_geometry(cx);
    let width = g.right - g.left;
    let height = g.bottom;
    let log_y = (cx.param)("logY") >= 0.5;
    let amps = amp_range(cx);
    c.fill_rect(0.0, 0.0, cx.w, cx.h, Rgba::alpha(0, 0, 0, 0));
    c.fill_rect(g.left, g.top, width, height, COLOR_BG);
    let grid = Paint::Solid(COLOR_GRID);
    let (freq, amp) = ticks(cx);
    for (x, _) in &freq {
        c.line((g.left + x, 2.0), (g.left + x, g.bottom), 1.0, Cap::Flat, &grid);
    }
    for (y, _) in &amp {
        c.line((g.left + 1.0, g.top + y), (g.right - 1.0, g.top + y), 1.0, Cap::Flat, &grid);
    }
    let stereo = (cx.param)("stereo") >= 0.5;
    let peaks = (cx.param)("peakHold") >= 0.5 || (cx.param)("refFreeze") >= 0.5;
    if cx.live(5, 0.0) >= 0.5 {
        let series = |i: usize, col: usize| cx.live(HEADER + i * SPEC_COLS + col, 0.0);
        let px = width.floor() as usize;
        let column = |i: usize, p: usize| {
            let (a, b) =
                (p * SPEC_COLS / px.max(1), ((p + 1) * SPEC_COLS / px.max(1)).max(p * SPEC_COLS / px.max(1) + 1));
            (a..b.min(SPEC_COLS)).map(|k| series(i, k)).fold(0.0f32, f32::max)
        };
        let fills: &[(usize, Rgba)] = if stereo { &[(1, COLOR_R), (0, COLOR_L)] } else { &[(0, COLOR_MONO)] };
        for &(i, color) in fills {
            for p in 0..px {
                let y = amp_y(column(i, p), amps, log_y, height).clamp(0.0, height);
                c.fill_rect(g.left + p as f32, g.top + y, 1.0, height - y, color);
            }
        }
        if peaks {
            let lines: &[(usize, Rgba)] = if stereo { &[(3, COLOR_R), (2, COLOR_L)] } else { &[(2, COLOR_L)] };
            for &(i, color) in lines {
                let paint = Paint::Solid(Rgba { a: 1.0, ..color });
                let mut last: Option<(f32, f32)> = None;
                for p in 0..px {
                    let y = g.top + amp_y(column(i, p), amps, log_y, height).clamp(0.0, height);
                    let x = g.left + p as f32 + 0.5;
                    if let Some(q) = last {
                        c.line(q, (x, y), 1.0, Cap::Round, &paint);
                    }
                    last = Some((x, y));
                }
            }
        }
    }
    outline(c, g.left, 1.0, width, height);
}

/// The display's 2 px rounded outline (drawn as four lines)
fn outline(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    let p = Paint::Solid(COLOR_GRID);
    for (a, b) in
        [((x, y), (x + w, y)), ((x + w, y), (x + w, y + h)), ((x + w, y + h), (x, y + h)), ((x, y + h), (x, y))]
    {
        c.line(a, b, 2.0, Cap::Round, &p);
    }
}

fn spectrum_texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
    let g = spectrum_geometry(cx);
    let (freq, amp) = ticks(cx);
    let mut out = Vec::new();
    for (x, label) in freq {
        out.push(SurfText {
            x: g.left + x,
            y: g.bottom + 5.0 + LABEL_PX,
            px: LABEL_PX,
            text: label,
            color: LABELS,
            centered: true,
        });
    }
    for (y, label) in amp {
        out.push(SurfText {
            x: g.left - 8.0,
            y: g.top + y + 4.0,
            px: LABEL_PX,
            text: label.clone(),
            color: LABELS,
            centered: true,
        });
        out.push(SurfText {
            x: g.right + 14.0,
            y: g.top + y + 4.0,
            px: LABEL_PX,
            text: label,
            color: LABELS,
            centered: true,
        });
    }
    out
}

fn wf_field(cx: &SurfaceCx<'_>) -> (f32, f32, f32, f32) {
    (26.0, 1.0, cx.w - 52.0, cx.h - 3.0)
}

fn draw_waterfall(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let (x, y, w, h) = wf_field(cx);
    c.fill_rect(x, y, w, h, Rgba::rgb(0, 0, 0));
    if (cx.param)("waterfall") >= 0.5 && cx.live(6, 0.0) >= 0.5 {
        let base = HEADER + 4 * SPEC_COLS;
        for j in 0..WF_ROWS {
            let (y0, y1) =
                ((y + h * j as f32 / WF_ROWS as f32).floor(), (y + h * (j + 1) as f32 / WF_ROWS as f32).floor());
            for i in 0..WF_COLS {
                let (x0, x1) =
                    ((x + w * i as f32 / WF_COLS as f32).floor(), (x + w * (i + 1) as f32 / WF_COLS as f32).floor());
                let v = cx.live(base + j * WF_COLS + i, 0.0) as u32;
                if v == 0 {
                    continue;
                }
                c.fill_rect(x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0), Rgba::hex(v));
            }
        }
    }
    outline(c, x, y, w, h);
}

/// `makeTimeTics`: (position down the field, label)
fn waterfall_texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
    let (x, y, w, h) = wf_field(cx);
    let block =
        BLOCKS.iter().copied().find(|b| b.to_string() == text_of((cx.field)("blockSize"))).unwrap_or(2048) as f32;
    let overlap = ((cx.param)("windowOverlap") as f32).max(1.0);
    let rows = ((cx.param)("waterfallHeight") as f32).max(1.0);
    let seconds_per_line = block / overlap / cx.live(1, cx.rate as f32).max(1.0);
    let per_line = h / rows;
    let limit = (h + y) / per_line * seconds_per_line;
    let step = ((10.0 * limit / (h / 30.0)).round() / 10.0).max(0.1);
    let mut out = Vec::new();
    let mut t = 0.0f32;
    while t <= limit {
        let label = if t > 99.0 {
            format!("{}", t.round() as i32)
        } else if t < 10.0 {
            format!("{:.1}", (t * 10.0).round() / 10.0)
        } else {
            format!("{}", t.round() as i32)
        };
        let py = y + per_line * t / seconds_per_line;
        for px in [x - 12.0, x + w + 14.0] {
            out.push(SurfText { x: px, y: py + 4.0, px: LABEL_PX, text: label.clone(), color: LABELS, centered: true });
        }
        t += step;
    }
    out
}

fn item(key: &'static str, text: &'static str) -> ComboItem {
    ComboItem { key, text: Text::new(text), icon: None }
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let mut controls = vec![
        surface(
            SurfaceSpec {
                w: W,
                h: SPEC_H,
                primary: "logX",
                reads: &["logX", "logY", "stereo", "peakHold", "refFreeze"],
                field_reads: &["freqRange", "ampRange"],
                draw: draw_spectrum,
                texts: Some(spectrum_texts),
                handles: None,
                drag: None,
                grab: 0.0,
                set_on_press: false,
                live: true,
                wheel: false,
                wheel_edit: None,
                peaks: None,
                markers: None,
                tag: 0,
            },
            "spectrum",
            0.0,
            SPEC_Y,
            "Spectrum",
        ),
        surface(
            SurfaceSpec {
                w: W,
                h: WF_H,
                primary: "waterfall",
                reads: &["waterfall", "waterfallHeight", "windowOverlap"],
                field_reads: &["blockSize", "freqRange"],
                draw: draw_waterfall,
                texts: Some(waterfall_texts),
                handles: None,
                drag: None,
                grab: 0.0,
                set_on_press: false,
                live: true,
                wheel: false,
                wheel_edit: None,
                peaks: None,
                markers: None,
                tag: 0,
            },
            "waterfallView",
            0.0,
            WF_Y,
            "Waterfall",
        ),
        button(Bind::Switch { param: "pause" }, "pause", 8.0, 6.0, &PLAY, &PAUSE, "Pause")
            .tip("Pause data acquisition"),
        button(Bind::Switch { param: "refFreeze" }, "refFreeze", 8.0, 40.0, &FREEZE, &FREEZE_OFF, "Reference freeze")
            .tip("Freeze current input as a reference / disable falloff in peak-hold mode."),
        led(Bind::Switch { param: "waterfall" }, "waterfallLed", 48.0, 8.0, LedColor::Green, "Waterfall", "Waterfall")
            .tip("Display real-time spectrogram"),
        led(Bind::Switch { param: "smooth" }, "smooth", 48.0, 26.0, LedColor::Green, "Averaging", "Averaging")
            .tip("Enable exponential moving average"),
        led(Bind::Switch { param: "stereo" }, "stereoLed", 48.0, 44.0, LedColor::Green, "Stereo", "Stereo")
            .tip("Display stereo channels separately"),
        led(Bind::Switch { param: "peakHold" }, "peakHold", 48.0, 62.0, LedColor::Green, "Peak hold", "Peak hold")
            .tip("Display envelope of peak values"),
        button(Bind::Switch { param: "logX" }, "logX", 146.0, 6.0, &X_LOG, &X_LINEAR, "Logarithmic frequency")
            .tip("Switch between logarithmic and linear frequency scale"),
        button(Bind::Switch { param: "logY" }, "logY", 146.0, 40.0, &Y_LOG, &Y_LINEAR, "Logarithmic amplitude")
            .tip("Switch between logarithmic and linear amplitude scale"),
        combo(
            "freqRange",
            174.0,
            8.0,
            116.0,
            "Frequency range",
            vec![
                item("full", "Full (auto)"),
                item("audible", "Audible"),
                item("bass", "Bass"),
                item("mids", "Mids"),
                item("high", "High"),
            ],
        )
        .tip("Frequency range"),
        combo(
            "ampRange",
            174.0,
            42.0,
            116.0,
            "Amplitude range",
            vec![
                item("extended", "Extended"),
                item("audible", "Audible"),
                item("loud", "Loud"),
                item("silent", "Silent"),
            ],
        )
        .tip("Amplitude range"),
        combo(
            "blockSize",
            326.0,
            8.0,
            170.0,
            "FFT block size",
            vec![
                item("256", "256 (High time res.)"),
                item("512", "512"),
                item("1024", "1024"),
                item("2048", "2048"),
                item("4096", "4096"),
                item("8192", "8192"),
                item("16384", "16384 (High freq. res.)"),
            ],
        )
        .tip("FFT block size"),
        combo(
            "window",
            326.0,
            42.0,
            170.0,
            "FFT window type",
            vec![
                item("rectangular", "Rectangular (Off)"),
                item("blackmanHarris", "Blackman-Harris (Default)"),
                item("hamming", "Hamming"),
                item("hanning", "Hanning"),
            ],
        )
        .tip("FFT window type"),
        button(Bind::View(1), "advancedOff", 508.0, 6.0, &ADV_ON, &ADV_OFF, "Advanced settings")
            .tip("Access advanced settings"),
        button(Bind::View(0), "advancedOn", 508.0, 6.0, &ADV_ON, &ADV_ON, "Advanced settings")
            .tip("Access advanced settings")
            .in_view_n(1),
    ];
    // the advanced knobs: two rows of four
    let knobs: [(&'static str, &'static str, &'static str, &'static str, f64); 8] = [
        (
            "envelopeRes",
            "Peak envelope resolution",
            "Envelope res.",
            "Maximum number of envelope points drawn per pixel:",
            0.05,
        ),
        (
            "spectrumRes",
            "Spectrum display resolution",
            "Spectrum res.",
            "Maximum number of spectrum points drawn per pixel:",
            0.05,
        ),
        ("peakDecayFactor", "Peak decay multiplier", "Falloff factor", "Multiply buffered value by", 0.001),
        ("averageWeight", "Averaging weight", "Averaging weight", "New sample contributes", 0.01),
        ("waterfallHeight", "Waterfall history size", "Waterfall height", "Number of lines to keep:", 50.0),
        ("waterfallGamma", "Waterfall gamma correction", "Waterfall gamma", "Gamma value:", 0.05),
        ("windowOverlap", "FFT window overlap", "Window overlap", "Number of times each sample is processed:", 1.0),
        ("zeroPadding", "FFT zero padding", "Zero padding", "Processing buffer is", 1.0),
    ];
    for (n, (param, name, label, hint, step)) in knobs.into_iter().enumerate() {
        let (col, row) = (n / 2, n % 2);
        let k =
            knob(param, 70.0 + col as f32 * 110.0, 6.0 + row as f32 * 38.0, KnobStyle::Small17, step, name, hint, "")
                .label(label)
                .in_view_n(1);
        controls.push(k);
    }
    PanelSpec {
        plugin: ID,
        size: (W, WF_Y + WF_H + 2.0),
        background: Some(&ART0),
        view_backgrounds: &VIEWS,
        fill: 0x101116,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});
