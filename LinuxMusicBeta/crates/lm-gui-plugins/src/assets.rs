//! The legacy artwork, embedded. Every file under `assets/` is a copy of a legacy file (GPL like the rest; the
//! third-party ones only with a recorded permission, docs/plugins-visual.md): `assets/lmms/` the theme's widget
//! pixmaps (legado/data/themes/default), `assets/<plugin>/` each plugin's own images (legado/plugins/<Plugin>).
//!
//! A PNG is decoded once, on first use, for the CPU painter (`raster`); the panels' backgrounds go to GPUI as they are
//! (PNG or SVG), which draws them at the screen's scale.

use std::sync::{Arc, OnceLock};

use gpui::{Image, ImageFormat};
use image::RgbaImage;

/// One embedded file
#[derive(Debug)]
pub struct Asset {
    /// Its path under `assets/` (for the messages and the tests)
    pub path: &'static str,
    pub bytes: &'static [u8],
    decoded: OnceLock<RgbaImage>,
    gpui: OnceLock<Arc<Image>>,
}

impl Asset {
    pub const fn new(path: &'static str, bytes: &'static [u8]) -> Asset {
        Asset { path, bytes, decoded: OnceLock::new(), gpui: OnceLock::new() }
    }

    pub fn is_svg(&self) -> bool {
        self.path.ends_with(".svg")
    }

    /// The file as a GPUI image (backgrounds): PNG or SVG, drawn by GPUI at the screen's scale
    pub fn gpui_image(&self) -> Arc<Image> {
        Arc::clone(self.gpui.get_or_init(|| {
            let format = if self.is_svg() { ImageFormat::Svg } else { ImageFormat::Png };
            Arc::new(Image::from_bytes(format, self.bytes.to_vec()))
        }))
    }
}

/// An embedded asset: `asset!("tripleoscillator/artwork.png")`
#[macro_export]
macro_rules! asset {
    ($path:literal) => {
        $crate::assets::Asset::new($path, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/", $path)))
    };
}

/// The decoded pixels of a PNG asset
pub fn image(a: &Asset) -> &RgbaImage {
    a.decoded.get_or_init(|| crate::raster::decode_png(a.bytes))
}

/// The theme's widget pixmaps (legado/data/themes/default)
pub mod lmms {
    use super::Asset;

    pub static KNOB01: Asset = asset!("lmms/knob01.png");
    pub static KNOB02: Asset = asset!("lmms/knob02.png");
    pub static KNOB03: Asset = asset!("lmms/knob03.png");
    pub static KNOB05: Asset = asset!("lmms/knob05.png");
    pub static LED_YELLOW: Asset = asset!("lmms/led_yellow.png");
    pub static LED_GREEN: Asset = asset!("lmms/led_green.png");
    pub static LED_RED: Asset = asset!("lmms/led_red.png");
    pub static LED_BLUE: Asset = asset!("lmms/led_blue.png");
    pub static LED_OFF: Asset = asset!("lmms/led_off.png");
    pub static LCD_19GREEN: Asset = asset!("lmms/lcd_19green.png");
    pub static LCD_19RED: Asset = asset!("lmms/lcd_19red.png");
    pub static LCD_21PINK: Asset = asset!("lmms/lcd_21pink.png");
    pub static LCD_11GREEN: Asset = asset!("lmms/lcd_11green.png");
    pub static LCD_19PURPLE: Asset = asset!("lmms/lcd_19purple.png");
    pub static COMBOBOX_ARROW: Asset = asset!("lmms/combobox_arrow.png");
    pub static COMBOBOX_ARROW_SELECTED: Asset = asset!("lmms/combobox_arrow_selected.png");
    pub static COMBOBOX_BG: Asset = asset!("lmms/combobox_bg.png");

    pub static SAW_WAVE_ACTIVE: Asset = asset!("lmms/saw_wave_active.png");
    pub static SAW_WAVE_INACTIVE: Asset = asset!("lmms/saw_wave_inactive.png");
    pub static TRIANGLE_WAVE_ACTIVE: Asset = asset!("lmms/triangle_wave_active.png");
    pub static TRIANGLE_WAVE_INACTIVE: Asset = asset!("lmms/triangle_wave_inactive.png");
    pub static SQUARE_WAVE_ACTIVE: Asset = asset!("lmms/square_wave_active.png");
    pub static SQUARE_WAVE_INACTIVE: Asset = asset!("lmms/square_wave_inactive.png");
    pub static ROUND_SQUARE_WAVE_ACTIVE: Asset = asset!("lmms/round_square_wave_active.png");
    pub static ROUND_SQUARE_WAVE_INACTIVE: Asset = asset!("lmms/round_square_wave_inactive.png");
    pub static MOOG_SAW_WAVE_ACTIVE: Asset = asset!("lmms/moog_saw_wave_active.png");
    pub static MOOG_SAW_WAVE_INACTIVE: Asset = asset!("lmms/moog_saw_wave_inactive.png");
    pub static SIN_WAVE_ACTIVE: Asset = asset!("lmms/sin_wave_active.png");
    pub static SIN_WAVE_INACTIVE: Asset = asset!("lmms/sin_wave_inactive.png");
    pub static EXP_WAVE_ACTIVE: Asset = asset!("lmms/exp_wave_active.png");
    pub static EXP_WAVE_INACTIVE: Asset = asset!("lmms/exp_wave_inactive.png");
    pub static WHITE_NOISE_WAVE_ACTIVE: Asset = asset!("lmms/white_noise_wave_active.png");
    pub static WHITE_NOISE_WAVE_INACTIVE: Asset = asset!("lmms/white_noise_wave_inactive.png");
    pub static FILTER_LP: Asset = asset!("lmms/filter_lp.png");
    pub static FILTER_HP: Asset = asset!("lmms/filter_hp.png");
    pub static FILTER_BP: Asset = asset!("lmms/filter_bp.png");
    pub static FILTER_NOTCH: Asset = asset!("lmms/filter_notch.png");
    pub static FILTER_AP: Asset = asset!("lmms/filter_ap.png");
    pub static FILTER_2LP: Asset = asset!("lmms/filter_2lp.png");

    pub static ALL: &[&Asset] = &[
        &KNOB01,
        &KNOB02,
        &KNOB03,
        &KNOB05,
        &LED_YELLOW,
        &LED_GREEN,
        &LED_RED,
        &LED_BLUE,
        &LED_OFF,
        &LCD_19GREEN,
        &LCD_19RED,
        &LCD_21PINK,
        &LCD_11GREEN,
        &LCD_19PURPLE,
        &COMBOBOX_ARROW,
        &COMBOBOX_ARROW_SELECTED,
        &COMBOBOX_BG,
        &SAW_WAVE_ACTIVE,
        &SAW_WAVE_INACTIVE,
        &TRIANGLE_WAVE_ACTIVE,
        &TRIANGLE_WAVE_INACTIVE,
        &SQUARE_WAVE_ACTIVE,
        &SQUARE_WAVE_INACTIVE,
        &ROUND_SQUARE_WAVE_ACTIVE,
        &ROUND_SQUARE_WAVE_INACTIVE,
        &MOOG_SAW_WAVE_ACTIVE,
        &MOOG_SAW_WAVE_INACTIVE,
        &SIN_WAVE_ACTIVE,
        &SIN_WAVE_INACTIVE,
        &EXP_WAVE_ACTIVE,
        &EXP_WAVE_INACTIVE,
        &WHITE_NOISE_WAVE_ACTIVE,
        &WHITE_NOISE_WAVE_INACTIVE,
        &FILTER_LP,
        &FILTER_HP,
        &FILTER_BP,
        &FILTER_NOTCH,
        &FILTER_AP,
        &FILTER_2LP,
    ];
}

impl PartialEq for Asset {
    /// The same file
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
