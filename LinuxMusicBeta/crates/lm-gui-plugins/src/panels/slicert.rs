//! SlicerT (legado/plugins/SlicerT/SlicerTView.cpp, `SlicerTWaveform.cpp`): the wave of the sample over the whole
//! width with its slice points (a click adds one, a drag moves one between its neighbours, Shift or the right button
//! removes one; the two ends stay), the note threshold and fade-out knobs, the "find slices again" button, the
//! original tempo, the snap, loop and BPM sync controls, the clear button and the sample's name with its open button.
//!
//! The panel is a fixed 516×400 (the legacy view's minimum; it was resizable). Not in the panel: the seeker (the mini
//! map that zoomed the editor), the playing slice's highlight (the core gives no playing position yet) and the "copy
//! the pattern as MIDI" button (there is no clipboard action in the core).
//!
//! Loading a sample finds its slices and tempo in the core (`slicert.analyze`, run when `sample` changes); the "find
//! slices" button runs it again with the present threshold and snap.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::{self, Asset};
use crate::raster::{Canvas, Rgba};
use crate::spec::{
    Bind, ComboItem, MarkersSpec, PanelSpec, PeaksSpec, StaticText, SurfText, SurfaceCx, SurfaceSpec, Target, Text,
    button, combo, knob, lcd, readout, surface,
};
use crate::widgets::{KnobStyle, LcdSheet, Styled};
use lm_port::Value;

pub const ID: &str = "slicert";

static LOGO: Asset = asset!("slicert/full_logo.png");
static SYNC_ON: Asset = asset!("slicert/sync_active.png");
static SYNC_OFF: Asset = asset!("slicert/sync_inactive.png");
static CLEAR_ON: Asset = asset!("slicert/clear_slices_active.png");
static CLEAR_OFF: Asset = asset!("slicert/clear_slices_inactive.png");
static RESET: Asset = asset!("slicert/reset_slices.png");
static FOLDER: Asset = asset!("slicert/folder_icon.png");
static ARROW: Asset = asset!("slicert/slice_indicator_arrow.png");

pub static ASSETS: &[&Asset] = &[&LOGO, &SYNC_ON, &SYNC_OFF, &CLEAR_ON, &CLEAR_OFF, &RESET, &FOLDER, &ARROW];

const W: f32 = 516.0;
const H: f32 = 400.0;
const TOP: f32 = 50.0;
const BOX_TOP: f32 = H - 97.0;
const NAME_BAR: f32 = 14.0;
/// The wave view: from under the top bar to the sample's name bar
const VIEW_H: f32 = BOX_TOP - NAME_BAR - TOP;
const ROW: f32 = H - 65.0;
/// A slice point's line and its shadow, and the wave's colours (`SlicerTWaveform`)
const WAVE: Rgba = Rgba { r: 183.0 / 255.0, g: 124.0 / 255.0, b: 1.0, a: 1.0 };
const SLICE: Rgba = Rgba { r: 218.0 / 255.0, g: 193.0 / 255.0, b: 1.0, a: 1.0 };
const SHADOW: Rgba = Rgba { r: 136.0 / 255.0, g: 120.0 / 255.0, b: 158.0 / 255.0, a: 1.0 };

fn knob_style() -> KnobStyle {
    KnobStyle::Styled(Styled {
        w: 50.0,
        h: 40.0,
        color: Rgba::rgb(162, 128, 226),
        outer: Some(Rgba::rgb(162, 128, 226)),
        inner_radius: 1.0,
        outer_radius: 11.0,
        center: (24.0, 15.0),
        line_width: 3.0,
    })
}

/// The bars, the boxes and the rings behind the two knobs (`SlicerTView::paintEvent`)
fn draw_chrome(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    c.fill_rect(0.0, 0.0, cx.w, cx.h, Rgba::rgb(15, 15, 15));
    c.fill_rect(0.0, 0.0, cx.w, TOP, Rgba::rgb(20, 23, 27));
    let line = Rgba::rgb(56, 58, 60);
    c.fill_rect(0.0, 0.0, cx.w, 1.0, line);
    c.fill_rect(0.0, TOP - 1.0, cx.w, 1.0, line);
    c.fill_rect(0.0, BOX_TOP, cx.w, cx.h - BOX_TOP, Rgba::rgb(23, 26, 31));
    c.fill_rect(0.0, BOX_TOP, cx.w, 1.0, line);
    c.fill_rect(0.0, BOX_TOP - NAME_BAR, cx.w, NAME_BAR, Rgba::rgb(5, 5, 5));
    c.fill_rect(cx.w - 24.0, BOX_TOP - NAME_BAR, 1.0, NAME_BAR, line);
    c.fill_rect(400.0, BOX_TOP, 1.0, cx.h - BOX_TOP, Rgba::rgb(56, 24, 94));
    c.pixmap(assets::image(&LOGO), 10.0, 10.0, false);
    // the rings under the knobs: a wide faint arc and a narrow bright one, from -45 degrees over 270, and the hub
    for x in [20.0, 70.0] {
        c.arc(x, ROW, 30.0, -45.0, 270.0, 4.0, crate::raster::Cap::Round, Rgba::alpha(159, 124, 223, 100));
        c.arc(x, ROW, 30.0, -45.0, 270.0, 2.0, crate::raster::Cap::Round, Rgba::rgb(159, 124, 223));
    }
}

/// The sample's wave over the whole width, the slice points with their arrows
fn draw_wave(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    c.fill_rect(0.0, 0.0, cx.w, cx.h, Rgba::rgb(15, 15, 15));
    let (mid, half) = (5.0 + (cx.h - 5.0) / 2.0, (cx.h - 5.0) / 2.0 - 1.0);
    c.fill_rect(0.0, mid, cx.w, 1.0, Rgba::alpha(255, 255, 255, 70));
    if let Some(b) = cx.peaks
        && b.length > 0
        && !b.pairs.is_empty()
    {
        let len = b.length as f64;
        let cols = cx.w as usize;
        for col in 0..cols {
            let f0 = col as f64 / cols as f64;
            let f1 = (col + 1) as f64 / cols as f64;
            let p0 = ((f0 * len / b.samples_per_pair as f64).floor() as i64 - b.from as i64).max(0) as usize;
            let p1 = (((f1 * len / b.samples_per_pair as f64).ceil() as i64 - b.from as i64).max(p0 as i64 + 1) as usize)
                .min(b.pairs.len());
            if p0 >= b.pairs.len() {
                continue;
            }
            let (mut lo, mut hi) = (i16::MAX, i16::MIN);
            for &(a, z) in &b.pairs[p0..p1.max(p0 + 1).min(b.pairs.len())] {
                lo = lo.min(a);
                hi = hi.max(z);
            }
            let top = (mid - (f32::from(hi) / 32767.0).clamp(-1.0, 1.0) * half).floor();
            let bottom = (mid - (f32::from(lo) / 32767.0).clamp(-1.0, 1.0) * half).ceil();
            c.fill_rect(col as f32, top, 1.0, (bottom - top).max(1.0), WAVE);
        }
    }
    c.fill_rect(0.0, 5.0, cx.w, 1.0, Rgba::rgb(53, 22, 90));
    c.fill_rect(0.0, cx.h - 1.0, cx.w, 1.0, Rgba::rgb(53, 22, 90));
    for p in (cx.field)("slices").items().filter_map(Value::as_real) {
        let x = (p.clamp(0.0, 1.0) as f32 * (cx.w - 1.0)).round();
        c.fill_rect(x - 1.0, 5.0, 1.0, cx.h - 5.0, SHADOW);
        c.fill_rect(x, 5.0, 1.0, cx.h - 5.0, SLICE);
        c.pixmap(assets::image(&ARROW), x - 4.0, 0.0, false);
    }
}

/// "Click to load sample" over an empty view
fn wave_texts(cx: &SurfaceCx<'_>) -> Vec<SurfText> {
    if (cx.field)("sample").as_resource().is_some() {
        return Vec::new();
    }
    vec![SurfText {
        x: cx.w / 2.0,
        y: cx.h / 2.0,
        px: 12.0,
        text: Text::new("Click to load sample").show(),
        color: 0x808080,
        centered: true,
    }]
}

/// The clear button: the two ends only
fn clear(_: &mut crate::spec::PressCx<'_>) -> Vec<(Target, Value)> {
    let ends = [0.0, 1.0].map(|v| Value::real(v).unwrap_or_default());
    vec![(Target::Field("slices"), Value::list(ends))]
}

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let item =
        |(key, text): (&'static str, &'static str)| ComboItem { key, text: crate::spec::Text::new(text), icon: None };
    let chrome = SurfaceSpec {
        w: W,
        h: H,
        primary: "",
        reads: &[],
        field_reads: &[],
        draw: draw_chrome,
        texts: None,
        handles: None,
        drag: None,
        grab: 0.0,
        set_on_press: false,
        live: false,
        wheel: false,
        wheel_edit: None,
        peaks: None,
        tag: 0,
        markers: None,
    };
    let wave = SurfaceSpec {
        w: W,
        h: VIEW_H,
        // a parameter of the plugin only so that the view takes the clicks: its edits are the slice points
        primary: "noteThreshold",
        reads: &[],
        field_reads: &["slices", "sample"],
        draw: draw_wave,
        texts: Some(wave_texts),
        handles: None,
        drag: None,
        grab: 0.0,
        set_on_press: false,
        live: false,
        wheel: false,
        wheel_edit: None,
        peaks: Some(PeaksSpec { field: "sample", start: "", end: "", width: W }),
        tag: 0,
        markers: Some(MarkersSpec { field: "slices", grab: 5.0, x0: 0.0, width: W - 1.0, fixed_ends: true }),
    };
    let mut controls = vec![
        surface(chrome, "chrome", 0.0, 0.0, "SlicerT"),
        surface(wave, "wave", 0.0, TOP, "Slices"),
        knob("noteThreshold", 10.0, ROW, knob_style(), 0.01, "Note threshold", "Note threshold", "")
            .tip("Threshold used for slicing"),
        knob("fadeOut", 60.0, ROW, knob_style(), 0.1, "Fade out", "Fade out", "ms")
            .tip("Fade Out per note in milliseconds"),
        button(Bind::Action { action: "slicert.analyze" }, "reset", 145.0, ROW + 3.0, &RESET, &RESET, "Reset slices")
            .tip("Reset slices"),
        lcd("originalBpm", 262.0, ROW + 4.0, LcdSheet::Purple19, 3, "Original bpm").tip("Original sample BPM"),
        combo(
            "snap",
            317.0,
            ROW + 3.0,
            55.0,
            "Slice snap",
            ["off", "1/1", "1/2", "1/4", "1/8", "1/16", "1/32"]
                .into_iter()
                .map(|k| item((k, if k == "off" { "Off" } else { k })))
                .collect(),
        )
        .tip("Set slice snapping for detection"),
        button(Bind::Switch { param: "sync" }, "sync", W - 100.0, ROW - 7.0, &SYNC_ON, &SYNC_OFF, "BPM sync")
            .tip("Enable BPM sync"),
        button(Bind::Press(clear), "clear", W - 100.0, ROW + 17.0, &CLEAR_ON, &CLEAR_OFF, "Clear slices")
            .tip("Clear all slices"),
        combo(
            "loop",
            W - 100.0,
            ROW + 40.0,
            84.0,
            "Loop",
            vec![item(("off", "Off")), item(("on", "Loop")), item(("pingPong", "Ping-pong"))],
        )
        .tip("How slices play while their note is held: once, looped or back and forth"),
        button(
            Bind::Open { field: "sample", action: None, extensions: &[] },
            "open",
            W - 20.0,
            BOX_TOP - NAME_BAR + 1.0,
            &FOLDER,
            &FOLDER,
            "Open sample",
        )
        .tip("Open sample selector"),
        readout("sample", 5.0, BOX_TOP - NAME_BAR + 1.0, W - 30.0, 0xb4b4b4, "No sample loaded").small(),
    ];
    // (the legacy labels, 7 pt white, under the controls of the bottom row)
    let labels = [("Threshold", 10.0), ("Fade Out", 60.0), ("Reset", 135.0), ("BPM", 259.0), ("Snap", 317.0)]
        .into_iter()
        .map(|(text, x)| StaticText {
            x,
            y: ROW + 27.0,
            px: crate::widgets::theme::SMALL_FONT_PX,
            text: Text::new(text),
            color: 0xffffff,
            view: None,
        })
        .collect();
    controls.shrink_to_fit();
    PanelSpec {
        plugin: ID,
        size: (W, H),
        background: None,
        view_backgrounds: &[],
        fill: 0x0f0f0f,
        controls,
        labels,
        file_drop: None,
    }
});
