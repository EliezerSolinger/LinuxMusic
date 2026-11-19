//! Waveform tiles: the peak pyramid rasterized once into small images, one per (resource, level, tile, color), and
//! drawn as images. Scrolling only moves tiles; zooming between two levels only stretches them on the GPU; a new
//! level rasterizes the tiles it shows, once.
//!
//! Why: GPUI inserts every primitive into the scene's bounds tree (for the draw order). One quad per pixel column is
//! ~13,000 inserts a frame here, the main cost of the frame; one filled path per item moves the cost to the CPU
//! tessellation (lyon). A few dozen images a frame cost almost nothing.

use std::collections::HashMap;
use std::sync::Arc;

use fake_core::Peaks;
use gpui::{RenderImage, Window};

/// Pairs per tile = pixels per tile at the tile's own level
pub const TILE_PAIRS: usize = 256;
/// Tile height in device pixels (the item's waveform area; 1x display here)
pub const TILE_H: usize = 64;
const MAX_TILES: usize = 512; // 64 KB each: 32 MB at most

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    resource: u32,
    level: u8,
    tile: u32,
    color: u32,
}

#[derive(Default)]
pub struct TileCache {
    tiles: HashMap<Key, Arc<RenderImage>>,
    pub rasterized: u64,
}

impl TileCache {
    /// The level whose pairs are at least one pixel wide (tiles are only ever stretched, never shrunk, so no peak
    /// is lost to the GPU's filtering)
    pub fn level_for(peaks: &Peaks, samples_per_px: f64) -> usize {
        let mut level = 0;
        while level + 1 < peaks.levels.len() && (Peaks::samples_per_pair(level) as f64) < samples_per_px {
            level += 1;
        }
        level
    }

    pub fn get(&mut self, window: &mut Window, peaks: &Peaks, resource: u32, level: usize, tile: usize, color: u32)
    -> Arc<RenderImage> {
        let key = Key { resource, level: level as u8, tile: tile as u32, color };
        if let Some(t) = self.tiles.get(&key) {
            return t.clone();
        }
        if self.tiles.len() >= MAX_TILES {
            // simple eviction: drop everything and free the atlas space
            for (_, t) in self.tiles.drain() {
                let _ = window.drop_image(t);
            }
        }
        let img = Arc::new(rasterize(peaks, level, tile, color));
        self.rasterized += 1;
        self.tiles.insert(key, img.clone());
        img
    }
}

fn rasterize(peaks: &Peaks, level: usize, tile: usize, color: u32) -> RenderImage {
    let pairs = &peaks.levels[level];
    let (r, g, b) = ((color >> 16) as u8, (color >> 8) as u8, color as u8);
    let mut buf = image::RgbaImage::new(TILE_PAIRS as u32, TILE_H as u32);
    let half = (TILE_H as f32 - 4.0) / 2.0;
    let mid = TILE_H as f32 / 2.0;
    for x in 0..TILE_PAIRS {
        let Some(&(lo, hi)) = pairs.get(tile * TILE_PAIRS + x) else { break };
        let top = (mid - hi as f32 / i16::MAX as f32 * half).floor().max(0.0) as u32;
        let bot = (mid - lo as f32 / i16::MAX as f32 * half).ceil().min(TILE_H as f32 - 1.0) as u32;
        for y in top..=bot.max(top) {
            // RenderImage is BGRA
            buf.put_pixel(x as u32, y, image::Rgba([b, g, r, 255]));
        }
    }
    RenderImage::new(vec![image::Frame::new(buf)])
}

/// Paints the waveform of `peaks` with tiles: `origin_x` is where the audio's sample 0 is on screen; only what falls
/// inside `clip` is drawn (an item, a comp segment, a take row); the tiles stretch to `y`..`y + h`.
#[allow(clippy::too_many_arguments)]
pub fn paint_wave(
    window: &mut Window,
    cache: &mut TileCache,
    peaks: &Peaks,
    resource: u32,
    color: u32,
    samples_per_px: f64,
    origin_x: gpui::Pixels,
    clip: gpui::Bounds<gpui::Pixels>,
    y: gpui::Pixels,
    h: gpui::Pixels,
) {
    use gpui::{Bounds, point, px, size};
    let level = TileCache::level_for(peaks, samples_per_px);
    let tile_samples = Peaks::samples_per_pair(level) as f64 * TILE_PAIRS as f64;
    let tile_w = (tile_samples / samples_per_px) as f32;
    let s0 = ((clip.origin.x - origin_x).as_f32() as f64 * samples_per_px).max(0.0);
    let s1 = ((clip.origin.x + clip.size.width - origin_x).as_f32() as f64 * samples_per_px).min(peaks.length as f64);
    if s1 <= s0 {
        return;
    }
    for t in (s0 / tile_samples) as usize..(s1 / tile_samples).ceil() as usize {
        let img = cache.get(window, peaks, resource, level, t, color);
        let tb = Bounds::new(point(origin_x + px(t as f32 * tile_w), y), size(px(tile_w), h));
        let _ = window.paint_image(clip, tb, Default::default(), img, 0, false);
    }
}
