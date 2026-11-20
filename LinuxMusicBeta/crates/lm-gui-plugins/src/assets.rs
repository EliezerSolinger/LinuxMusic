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
    pub static COMBOBOX_BG: Asset = asset!("lmms/combobox_bg.png");

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
        &COMBOBOX_BG,
    ];
}

impl PartialEq for Asset {
    /// The same file
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
