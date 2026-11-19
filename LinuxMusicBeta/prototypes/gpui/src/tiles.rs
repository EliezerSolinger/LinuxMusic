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
