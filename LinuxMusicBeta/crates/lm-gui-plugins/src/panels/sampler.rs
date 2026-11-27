//! The Sampler (`lm-plugins-sampler`): the one panel of the Sf2 Player, the GigPlayer, PatMan, the AudioFileProcessor,
//! AvlDrums and the DrumMachines. It is a source picker and the global knobs, and a view that depends on what the
//! source is:
//!
//! - **audio file** (the AudioFileProcessor's view): the wave with the start, end and loop markers (dragged with the
//!   mouse, the wheel zooms), the loop choice, reverse and stutter, and the three points as knobs;
//! - **an AVL drum kit**: the photo of the kit (the images of avldrums.lv2) and the pieces that light up when they are
//!   hit, from the plugin's display frame (the hits of each key; a light fades by 0.85 every 33 ms, as the legacy's);
//! - **a SoundFont or a GIG**: the list of its presets, read by the core (`sampler.list_presets`), and the zone map;
//! - **anything else** (SFZ, PAT, a DrumMachines kit, no source): the zone map: for each key and each of 8 velocity
//!   bands, how many zones cover it, with the hits lit over it.
//!
//! Short labels, no explanatory text. What the display frame holds is `lm_plugins_sampler::display`'s (the numbers
//! are repeated here: this crate does not depend on the plugins).

use std::sync::{LazyLock, OnceLock};

use image::RgbaImage;

use super::sampler_wave as afp;
use crate::asset;
use crate::assets::{self, Asset};
use crate::raster::{Canvas, Rgba};
use crate::spec::{
    Bind, ComboItem, ListSpec, PanelSpec, PeaksSpec, SURFACE_FADE, SurfaceCx, SurfaceSpec, button, combo, knob, led,
    list, readout, surface,
};
use crate::widgets::{KnobStyle, LedColor};

pub const ID: &str = "sampler";

static SELECT_FILE: Asset = asset!("sampler/select_file.png");
static SELECT_FILE_ON: Asset = asset!("sampler/select_file_on.png");

static BLACK_PEARL: Asset = asset!("sampler/black_pearl.png");
static BLACK_PEARL_MAP: Asset = asset!("sampler/black_pearl.map.png");
static RED_ZEPPELIN: Asset = asset!("sampler/red_zeppelin.png");
static RED_ZEPPELIN_MAP: Asset = asset!("sampler/red_zeppelin.map.png");
static BLONDE_BOP: Asset = asset!("sampler/blonde_bop.png");
static BLONDE_BOP_HR: Asset = asset!("sampler/blonde_bop_hr.png");
static BLONDE_BOP_MAP: Asset = asset!("sampler/blonde_bop.map.png");
static BUSKMAN: Asset = asset!("sampler/buskman.png");
static BUSKMAN_MAP: Asset = asset!("sampler/buskman.map.png");

pub static ASSETS: &[&Asset] = &[
    &SELECT_FILE,
    &SELECT_FILE_ON,
    &BLACK_PEARL,
    &BLACK_PEARL_MAP,
    &RED_ZEPPELIN,
    &RED_ZEPPELIN_MAP,
    &BLONDE_BOP,
    &BLONDE_BOP_HR,
    &BLONDE_BOP_MAP,
    &BUSKMAN,
    &BUSKMAN_MAP,
];

/// The core's action that reads a bank, an SFZ, a patch or a wave into the source's field
pub const LOAD_ACTION: &str = "plugin.load_into_field";
/// The core's action that lists the presets of the bank
pub const LIST_ACTION: &str = "sampler.list_presets";

// The plugin's display frame (`lm_plugins_sampler::display`)
const KEYS: usize = 128;
const BANDS: usize = 8;
const HIT_COUNT: usize = 0;
const HIT_VELOCITY: usize = KEYS;
const ZONES: usize = 2 * KEYS;

pub const SIZE: (f32, f32) = (560.0, 330.0);
const VIEW: (f32, f32) = (8.0, 40.0);

// ---- the kit photo ----

/// Where the photo sits and its size (the avldrums.lv2 images are 1024 x 512)
const PHOTO: (u32, u32) = (400, 200);

/// avldrums.lv2's map: a piece's color in the map's red channel is `10 + 9 * index`, and the index is this table's
/// position; the table's value is the piece, whose key is `35 + piece`
const MAP_PIECES: [u8; 26] =
    [0, 2, 4, 1, 3, 12, 10, 6, 7, 5, 8, 11, 9, 13, 14, 16, 23, 15, 17, 18, 22, 21, 20, 24, 19, 25];
const FIRST_KEY: usize = 35;
const PIECES: usize = 27;

/// A kit's photo at the view's size, and the pixels of each piece
struct Kit {
    photo: RgbaImage,
    pieces: Vec<Vec<(u16, u16)>>,
}

fn build_kit(photo: &Asset, map: &Asset) -> Kit {
    let (w, h) = PHOTO;
    let shrunk = image::imageops::resize(assets::image(photo), w, h, image::imageops::FilterType::Triangle);
    let map = assets::image(map);
    let mut pieces = vec![Vec::new(); PIECES];
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (x * map.width() / w, y * map.height() / h);
            let r = map.get_pixel(sx.min(map.width() - 1), sy.min(map.height() - 1))[0];
            if r >= 10 {
                let i = usize::from((r - 10) / 9);
                if let Some(p) = MAP_PIECES.get(i)
                    && let Some(list) = pieces.get_mut(usize::from(*p))
                {
                    list.push((x as u16, y as u16));
                }
            }
        }
    }
    Kit { photo: shrunk, pieces }
}

/// The AVL kit of a state's `kit` key, made on first use
fn avl_kit(key: &str) -> Option<&'static Kit> {
    static KITS: [OnceLock<Kit>; 5] = [const { OnceLock::new() }; 5];
    let (i, photo, map) = match key {
        "black-pearl" => (0, &BLACK_PEARL, &BLACK_PEARL_MAP),
        "red-zeppelin" => (1, &RED_ZEPPELIN, &RED_ZEPPELIN_MAP),
        "blonde-bop" => (2, &BLONDE_BOP, &BLONDE_BOP_MAP),
        "blonde-bop-hot-rod" => (3, &BLONDE_BOP_HR, &BLONDE_BOP_MAP),
        "buskmans-holiday" => (4, &BUSKMAN, &BUSKMAN_MAP),
        _ => return None,
    };
    Some(KITS[i].get_or_init(|| build_kit(photo, map)))
}

/// How lit a key is now: the velocity of its last hit, falling by 0.85 every 33 ms (the legacy's brightness), from the
/// frames of the last 700 ms that changed
fn brightness(cx: &SurfaceCx<'_>, key: usize) -> f32 {
    let mut lit = 0.0f32;
    for pair in cx.history.windows(2) {
        let (before, after) = (&pair[0].1, &pair[1].1);
        let (a, b) = (before.get(HIT_COUNT + key), after.get(HIT_COUNT + key));
        if a != b {
            let velocity = after.get(HIT_VELOCITY + key).copied().unwrap_or(127.0) / 127.0;
            let ticks = pair[1].0 / 33.0;
            lit = velocity * 0.85f32.powf(ticks);
        }
    }
    if lit < 0.02 { 0.0 } else { lit }
}

fn is_kit(cx: &SurfaceCx<'_>) -> bool {
    (cx.field)("format").as_text() == Some("kit")
}

/// The photo with the pieces hit, for an AVL kit
fn draw_photo(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    if !is_kit(cx) {
        return;
    }
    let key = (cx.field)("kit");
    let Some(kit) = key.as_text().and_then(avl_kit) else { return };
    c.pixmap(&kit.photo, 0.0, 0.0, false);
    for (piece, pixels) in kit.pieces.iter().enumerate() {
        let b = brightness(cx, FIRST_KEY + piece);
        if b > 0.0 {
            let color = Rgba::alpha(255, 214, 120, (170.0 * b) as u8);
            for (x, y) in pixels {
                c.fill_rect(f32::from(*x), f32::from(*y), 1.0, 1.0, color);
            }
        }
    }
}

// ---- the zone map ----

pub const MAP_W: f32 = 336.0;
pub const MAP_H: f32 = 168.0;

/// How many zones cover each key and velocity band (the display frame), the hits lit over it
fn draw_zones(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    let format = (cx.field)("format");
    let format = format.as_text().unwrap_or("none");
    let kit = (cx.field)("kit");
    if format == "audio" || (format == "kit" && kit.as_text().and_then(avl_kit).is_some()) {
        return;
    }
    c.fill_rect(0.0, 0.0, MAP_W, MAP_H, Rgba::rgb(0x1b, 0x1e, 0x23));
    let (kw, bh) = (MAP_W / KEYS as f32, (MAP_H - 6.0) / BANDS as f32);
    for key in 0..KEYS {
        for band in 0..BANDS {
            let n = cx.frame.get(ZONES + key * BANDS + band).copied().unwrap_or(0.0);
            if n > 0.0 {
                // (a velocity band's top is the loudest: drawn from the bottom up)
                let level = (0.45 + 0.15 * n.min(4.0)).min(1.0);
                let color = Rgba::alpha(0x5f, 0xaf, 0xff, (255.0 * level) as u8);
                c.fill_rect(key as f32 * kw, MAP_H - 6.0 - (band as f32 + 1.0) * bh, kw.max(1.0), bh - 1.0, color);
            }
        }
        let b = brightness(cx, key);
        if b > 0.0 {
            c.fill_rect(key as f32 * kw, 0.0, kw.max(1.0), MAP_H - 6.0, Rgba::alpha(255, 214, 120, (150.0 * b) as u8));
        }
        if key % 12 == 0 {
            c.fill_rect(key as f32 * kw, MAP_H - 5.0, 1.0, 5.0, Rgba::rgb(0x80, 0x88, 0x94));
        }
    }
}

// ---- the wave of an audio file ----

fn draw_wave(c: &mut Canvas, cx: &SurfaceCx<'_>) {
    if (cx.field)("format").as_text() != Some("audio") {
        return;
    }
    // the knob is in dB: the waveform is drawn as loud as it will be
    afp::draw_wave(c, cx, 10f32.powf(((cx.param)("gain") / 20.0) as f32));
}

// ---- the panel ----

const FORMATS: [(&str, &str); 7] = [
    ("none", "None"),
    ("sf2", "SF2"),
    ("sfz", "SFZ"),
    ("pat", "PAT"),
    ("audio", "Audio"),
    ("kit", "Kit"),
    ("gig", "GIG"),
];

const KITS: [(&str, &str); 18] = [
    ("black-pearl", "Black Pearl"),
    ("red-zeppelin", "Red Zeppelin"),
    ("blonde-bop", "Blonde Bop"),
    ("blonde-bop-hot-rod", "Blonde Bop Hot Rod"),
    ("buskmans-holiday", "Buskman's Holiday"),
    ("cr78", "CR-78"),
    ("cr8000", "CR-8000"),
    ("tr606", "TR-606"),
    ("tr77", "TR-77"),
    ("tr808", "TR-808"),
    ("tr909", "TR-909"),
    ("linn", "LinnDrum"),
    ("electro", "Electro"),
    ("rnb", "R&B"),
    ("sp1200", "SP-1200"),
    ("mpc60", "MPC60"),
    ("dmx", "DMX"),
    ("tr707", "TR-707"),
];

/// A source's open button and its file name: the field and what the file is called for the dialog
const SOURCES: [(&str, &str, &[&str], Option<&str>); 5] = [
    ("sf2", "soundfont", &["sf2", "sf3"], Some(LOAD_ACTION)),
    ("gig", "gig", &["gig"], Some(LOAD_ACTION)),
    ("sfz", "sfz", &["sfz"], Some(LOAD_ACTION)),
    ("pat", "patch", &["pat"], Some(LOAD_ACTION)),
    ("audio", "sample", &[], None),
];

static LISTING: ListSpec = ListSpec {
    w: 200.0,
    h: 168.0,
    row: 14.0,
    bank: "bank",
    program: "program",
    action: LIST_ACTION,
    format_field: "format",
    files: &[("sf2", "soundfont"), ("gig", "gig")],
};

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| {
    let item =
        |(key, text): (&'static str, &'static str)| ComboItem { key, text: crate::spec::Text::new(text), icon: None };
    let mut controls = vec![combo("format", 8.0, 8.0, 100.0, "Source", FORMATS.iter().copied().map(item).collect())];
    for (format, field, extensions, action) in SOURCES {
        let id: &'static str = match format {
            "sf2" => "openSf2",
            "gig" => "openGig",
            "sfz" => "openSfz",
            "pat" => "openPat",
            _ => "openAudio",
        };
        controls.push(
            button(Bind::Open { field, action, extensions }, id, 116.0, 8.0, &SELECT_FILE_ON, &SELECT_FILE, "Open")
                .tip("Open")
                .only_while("format", format),
        );
        controls.push(readout(field, 150.0, 12.0, 400.0, 0xd1d8e4, "").small().only_while("format", format));
    }
    controls.push(
        combo("kit", 116.0, 8.0, 200.0, "Kit", KITS.iter().copied().map(item).collect()).only_while("format", "kit"),
    );
    // what the source shows
    controls.push(list(LISTING.clone(), "presetsSf2", VIEW.0, VIEW.1, "Preset").only_while("format", "sf2"));
    controls.push(list(LISTING.clone(), "presetsGig", VIEW.0, VIEW.1, "Preset").only_while("format", "gig"));
    controls.push(surface(
        SurfaceSpec {
            w: MAP_W,
            h: MAP_H,
            primary: "",
            reads: &[],
            field_reads: &["format", "kit"],
            draw: draw_zones,
            texts: None,
            handles: None,
            drag: None,
            grab: 0.0,
            set_on_press: false,
            live: true,
            wheel: false,
            wheel_edit: None,
            peaks: None,
            markers: None,
            tag: SURFACE_FADE,
        },
        "zones",
        VIEW.0 + 208.0,
        VIEW.1,
        "Zones",
    ));
    controls.push(surface(
        SurfaceSpec {
            w: PHOTO.0 as f32,
            h: PHOTO.1 as f32,
            primary: "",
            reads: &[],
            field_reads: &["format", "kit"],
            draw: draw_photo,
            texts: None,
            handles: None,
            drag: None,
            grab: 0.0,
            set_on_press: false,
            live: true,
            wheel: false,
            wheel_edit: None,
            peaks: None,
            markers: None,
            tag: SURFACE_FADE,
        },
        "photo",
        VIEW.0,
        VIEW.1,
        "Kit",
    ));
    // an audio file
    controls.push(
        surface(
            SurfaceSpec {
                w: afp::W,
                h: afp::H,
                primary: "startPoint",
                reads: &["startPoint", "endPoint", "loopPoint", "gain", "reverse"],
                field_reads: &["format", "sample"],
                draw: draw_wave,
                texts: Some(afp::texts),
                handles: Some(afp::handles),
                drag: Some(afp::drag),
                grab: 4.0,
                set_on_press: false,
                live: false,
                wheel: false,
                wheel_edit: None,
                peaks: Some(PeaksSpec { field: "sample", start: "startPoint", end: "endPoint", width: afp::GW }),
                markers: None,
                tag: 0,
            },
            "wave",
            VIEW.0,
            VIEW.1 + 4.0,
            "Wave",
        )
        .only_while("format", "audio"),
    );
    let audio = |c: crate::spec::Control| c.only_while("format", "audio");
    controls.push(audio(combo(
        "loop",
        VIEW.0,
        VIEW.1 + 90.0,
        90.0,
        "Loop",
        vec![item(("off", "No loop")), item(("on", "Loop")), item(("pingPong", "Ping-pong"))],
    )));
    controls.push(audio(led(
        Bind::Switch { param: "reverse" },
        "reverse",
        VIEW.0 + 100.0,
        VIEW.1 + 94.0,
        LedColor::Green,
        "Reverse",
        "Reverse",
    )));
    controls.push(audio(led(
        Bind::Switch { param: "stutter" },
        "stutter",
        VIEW.0 + 176.0,
        VIEW.1 + 94.0,
        LedColor::Green,
        "Stutter",
        "Stutter",
    )));
    for (i, (param, label, name)) in
        [("startPoint", "Start", "Start"), ("loopPoint", "Loop", "Loop point"), ("endPoint", "End", "End")]
            .into_iter()
            .enumerate()
    {
        let k = knob(param, 280.0 + 50.0 * i as f32, VIEW.1 + 4.0, KnobStyle::Small17, 0.0000001, name, name, "")
            .label(label);
        controls.push(audio(if param == "loopPoint" { k.hidden_while("loop", "off") } else { k }));
    }
    // the global knobs
    let global = [
        ("gain", "Gain", "dB", 0.1),
        ("pan", "Pan", "", 0.01),
        ("tune", "Tune", "cents", 1.0),
        ("bendRange", "Bend", "semitones", 1.0),
        ("polyphony", "Voices", "", 1.0),
        ("glide", "Glide", "ms", 1.0),
    ];
    for (i, (param, label, unit, step)) in global.into_iter().enumerate() {
        controls.push(
            knob(param, 14.0 + 50.0 * i as f32, 254.0, KnobStyle::Small17, step, label, label, unit).label(label),
        );
    }
    let envelope =
        [("volAttack", "Atk"), ("volDecay", "Dec"), ("volSustain", "Sus"), ("volRelease", "Rel"), ("volAmount", "Env")];
    for (i, (param, label)) in envelope.into_iter().enumerate() {
        controls
            .push(knob(param, 14.0 + 50.0 * i as f32, 292.0, KnobStyle::Small17, 0.001, label, label, "").label(label));
    }
    for (i, (value, text)) in [(0.0, "Poly"), (1.0, "Mono"), (2.0, "Legato")].into_iter().enumerate() {
        let id = ["poly", "mono", "legato"][i];
        controls.push(led(
            Bind::Step { param: "voiceMode", value },
            id,
            340.0,
            252.0 + 18.0 * i as f32,
            LedColor::Green,
            text,
            "Voice mode",
        ));
    }
    for (i, (key, text)) in [("main", "Main"), ("groups", "Groups")].into_iter().enumerate() {
        controls.push(
            led(
                Bind::Choice { field: "outputs", key },
                if i == 0 { "outMain" } else { "outGroups" },
                430.0,
                252.0 + 18.0 * i as f32,
                LedColor::Green,
                text,
                "Outputs",
            )
            .only_while("format", "kit"),
        );
    }
    controls.push(combo(
        "interpolation",
        430.0,
        296.0,
        100.0,
        "Interpolation",
        vec![item(("linear", "Linear")), item(("cubic", "Cubic")), item(("sinc", "Sinc"))],
    ));
    PanelSpec {
        plugin: ID,
        size: SIZE,
        background: None,
        view_backgrounds: &[],
        fill: 0x16181b,
        controls,
        labels: Vec::new(),
        file_drop: None,
    }
});
