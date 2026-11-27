//! Waveform tiles: blocks of the peak pyramid, asked of the core through the port (`CorePort::peaks`, I-20.4) and
//! kept forever (a resource's peaks never change), rasterized once into small images, one per (resource, level,
//! tile, color), and drawn as images. Scrolling only moves tiles; zooming between two levels only stretches them on
//! the GPU; a new level asks and rasterizes the tiles it shows, once. A block that is not there yet (`Waiting`: the
//! peaks are being made, or, in the client, still on the network) is simply not drawn, and asked again next frame.
//!
//! Why images: GPUI inserts every primitive into the scene's bounds tree (for the draw order). One quad per pixel
//! column was ~13,000 inserts a frame here, the main cost of the frame; one filled path per item moved the cost to
//! the CPU tessellation (lyon). A few dozen images a frame cost almost nothing (docs/prototipo-interface.md, 2.2).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use gpui::{Bounds, Pixels, RenderImage, Window, point, px, size};
use lm_port::{CorePort, Error, PeakBlock, Pending, RequestId, Resource};

/// The pyramid's shape (I-20.4): 64 samples per pair at level 0, each level joins 4
const LEVEL0: u64 = 64;
const FACTOR: u64 = 4;

/// Pairs per tile = pixels per tile at the tile's own level
pub const TILE_PAIRS: usize = 256;
/// Tile height in device pixels (the item's waveform area; 1x display here)
pub const TILE_H: usize = 64;
const MAX_IMAGES: usize = 512; // 64 KB each: 32 MB at most
const MAX_BLOCKS: usize = 16_384; // 1 KB each: 16 MB at most
/// Resources kept by number; the one used the longest ago leaves (with its blocks and images) when a new one comes
const MAX_RESOURCES: usize = 256;
/// Requests in flight remembered (answers that never come are forgotten past this)
const MAX_IN_FLIGHT: usize = 4096;

/// The peaks that arrived and the ones that failed since the last frame (`Core::take_peak_answers`)
pub type PeakAnswers = (Vec<RequestId>, Vec<(RequestId, Error)>);

type BlockKey = (u32, u8, u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    resource: u32,
    level: u8,
    tile: u32,
    color: u32,
}

#[derive(Default)]
pub struct TileCache {
    /// the resources seen, by a small number (the keys below); numbers are never reused. At most `MAX_RESOURCES`:
    /// the least recently used leaves, and so does a resource whose peaks grew
    names: HashMap<Resource, u32>,
    by_slot: HashMap<u32, Resource>,
    used: HashMap<u32, u64>,
    next_slot: u32,
    clock: u64,
    blocks: HashMap<BlockKey, Arc<PeakBlock>>,
    images: HashMap<Key, Arc<RenderImage>>,
    wanted: HashSet<BlockKey>,
    /// Blocks the port answered `Waiting` last frame: only the ones still visible are asked again
    retry: Vec<BlockKey>,
    /// The requests in flight, by block: a block is asked once until its answer comes (or fails, and then never
    /// again until the resource is forgotten)
    asked: HashMap<BlockKey, RequestId>,
    in_flight: HashMap<RequestId, BlockKey>,
    failed: HashSet<BlockKey>,
    /// Images of resources that grew, waiting for a window to free them
    stale: Vec<Arc<RenderImage>>,
    pub rasterized: u64,
}

/// The level whose pairs are at least one pixel wide: tiles are only ever stretched, never shrunk, so no peak is
/// lost to the GPU's filtering
pub fn level_for(samples_per_px: f64) -> u8 {
    let mut level = 0u8;
    while level < 11 && ((LEVEL0 * FACTOR.pow(level as u32)) as f64) < samples_per_px {
        level += 1;
    }
    level
}

fn samples_per_tile(level: u8) -> f64 {
    (LEVEL0 * FACTOR.pow(level as u32)) as f64 * TILE_PAIRS as f64
}

/// The tiles under `a..z` (pixels) for audio whose sample 0 is at `origin_x`
pub fn tile_range(level: u8, samples_per_px: f64, origin_x: f32, a: f32, z: f32) -> Range<usize> {
    let st = samples_per_tile(level);
    let s0 = ((a - origin_x) as f64 * samples_per_px).max(0.0);
    let s1 = (z - origin_x) as f64 * samples_per_px;
    if s1 <= s0 {
        return 0..0;
    }
    (s0 / st) as usize..(s1 / st).ceil() as usize
}

impl TileCache {
    fn slot(&mut self, r: &Resource) -> u32 {
        self.clock += 1;
        if let Some(&slot) = self.names.get(r) {
            self.used.insert(slot, self.clock);
            return slot;
        }
        if self.names.len() >= MAX_RESOURCES
            && let Some((&oldest, _)) = self.used.iter().min_by_key(|(_, at)| **at)
            && let Some(old) = self.by_slot.get(&oldest).cloned()
        {
            self.forget(&old);
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        self.names.insert(r.clone(), slot);
        self.by_slot.insert(slot, r.clone());
        self.used.insert(slot, self.clock);
        slot
    }

    /// The resources kept (the tests check the bound)
    #[cfg(test)]
    pub fn resources(&self) -> usize {
        self.names.len()
    }

    /// A resource whose peaks grow (a take being recorded, U3) or that is gone: what was kept of it is dropped,
    /// number included; its tiles are asked again and drawn again if it is shown again
    pub fn forget(&mut self, resource: &Resource) {
        let Some(slot) = self.names.remove(resource) else { return };
        self.by_slot.remove(&slot);
        self.used.remove(&slot);
        self.blocks.retain(|k, _| k.0 != slot);
        self.wanted.retain(|k| k.0 != slot);
        self.retry.retain(|k| k.0 != slot);
        self.failed.retain(|k| k.0 != slot);
        self.asked.retain(|k, _| k.0 != slot);
        self.in_flight.retain(|_, k| k.0 != slot);
        let gone: Vec<Key> = self.images.keys().filter(|k| k.resource == slot).copied().collect();
        for k in gone {
            if let Some(img) = self.images.remove(&k) {
                self.stale.push(img);
            }
        }
    }

    /// Notes the tiles a frame will draw (from render, which knows the geometry)
    pub fn want(&mut self, resource: &Resource, level: u8, tiles: Range<usize>) {
        let slot = self.slot(resource);
        for t in tiles {
            let k = (slot, level, t as u32);
            if !self.blocks.contains_key(&k) {
                self.wanted.insert(k);
            }
        }
    }

    /// Some wanted block is not there yet (asked again on the next frame)
    pub fn waiting(&self) -> bool {
        !self.wanted.is_empty() || !self.retry.is_empty()
    }

    /// Asks the port for the wanted blocks (once per frame, with the port borrowed mutably); the ones still
    /// `Waiting` are asked again only if the next frame still wants them (scrolled-away ones are dropped), and only
    /// after the answer to the first ask came. `answers` are the peaks that arrived or failed since the last frame;
    /// gives the errors to show
    pub fn fetch(&mut self, port: &mut dyn CorePort, answers: PeakAnswers) -> Vec<Error> {
        let (arrived, failed) = answers;
        for id in arrived {
            if let Some(k) = self.in_flight.remove(&id) {
                self.asked.remove(&k);
            }
        }
        let mut errors = Vec::new();
        for (id, e) in failed {
            if let Some(k) = self.in_flight.remove(&id) {
                self.asked.remove(&k);
                self.failed.insert(k);
                errors.push(e);
            }
        }
        if self.in_flight.len() > MAX_IN_FLIGHT {
            self.in_flight.clear();
            self.asked.clear();
        }
        self.retry.clear();
        for k in std::mem::take(&mut self.wanted) {
            let (res, level, tile) = k;
            if self.failed.contains(&k) {
                continue;
            }
            if self.asked.contains_key(&k) {
                self.retry.push(k);
                continue;
            }
            let from = tile as u64 * TILE_PAIRS as u64;
            let Some(name) = self.by_slot.get(&res).cloned() else { continue };
            match port.peaks(&name, level, from, from + TILE_PAIRS as u64) {
                Pending::Ready(Ok(b)) => {
                    if self.blocks.len() >= MAX_BLOCKS {
                        // simple eviction, like the images: the visible ones are asked again by the next frame
                        self.blocks.clear();
                    }
                    self.blocks.insert(k, Arc::new(b));
                }
                Pending::Waiting(id) => {
                    self.asked.insert(k, id);
                    self.in_flight.insert(id, k);
                    self.retry.push(k);
                }
                Pending::Ready(Err(e)) => {
                    self.failed.insert(k);
                    errors.push(e);
                }
            }
        }
        errors
    }

    fn image(
        &mut self,
        window: &mut Window,
        resource: u32,
        level: u8,
        tile: usize,
        color: u32,
    ) -> Option<Arc<RenderImage>> {
        let key = Key { resource, level, tile: tile as u32, color };
        if let Some(t) = self.images.get(&key) {
            return Some(t.clone());
        }
        let block = self.blocks.get(&(resource, level, tile as u32))?.clone();
        if self.images.len() >= MAX_IMAGES {
            // simple eviction: drop every image and free the atlas space (the blocks stay)
            for (_, t) in self.images.drain() {
                let _ = window.drop_image(t);
            }
        }
        let img = Arc::new(rasterize(&block, color));
        self.rasterized += 1;
        self.images.insert(key, img.clone());
        Some(img)
    }
}

fn rasterize(block: &PeakBlock, color: u32) -> RenderImage {
    let (r, g, b) = ((color >> 16) as u8, (color >> 8) as u8, color as u8);
    let mut buf = image::RgbaImage::new(TILE_PAIRS as u32, TILE_H as u32);
    let half = (TILE_H as f32 - 4.0) / 2.0;
    let mid = TILE_H as f32 / 2.0;
    for (x, &(lo, hi)) in block.pairs.iter().take(TILE_PAIRS).enumerate() {
        let top = (mid - hi as f32 / i16::MAX as f32 * half).floor().max(0.0) as u32;
        let bot = (mid - lo as f32 / i16::MAX as f32 * half).ceil().min(TILE_H as f32 - 1.0) as u32;
        for y in top..=bot.max(top) {
            // RenderImage is BGRA
            buf.put_pixel(x as u32, y, image::Rgba([b, g, r, 255]));
        }
    }
    RenderImage::new(vec![image::Frame::new(buf)])
}

/// Paints a waveform with tiles: `origin_x` is where the audio's sample 0 is on screen; only what falls inside
/// `clip` is drawn (an item, a comp segment, a take row); the tiles stretch to `y`..`y + h`
#[allow(clippy::too_many_arguments)]
pub fn paint_wave(
    window: &mut Window,
    cache: &mut TileCache,
    resource: &Resource,
    color: u32,
    samples_per_px: f64,
    origin_x: Pixels,
    clip: Bounds<Pixels>,
    y: Pixels,
    h: Pixels,
) {
    for img in cache.stale.drain(..) {
        let _ = window.drop_image(img);
    }
    let level = level_for(samples_per_px);
    let tile_w = (samples_per_tile(level) / samples_per_px) as f32;
    let range = tile_range(
        level,
        samples_per_px,
        origin_x.as_f32(),
        clip.origin.x.as_f32(),
        (clip.origin.x + clip.size.width).as_f32(),
    );
    let slot = cache.slot(resource);
    for t in range {
        if let Some(img) = cache.image(window, slot, level, t, color) {
            let tb = Bounds::new(point(origin_x + px(t as f32 * tile_w), y), size(px(tile_w), h));
            let _ = window.paint_image(clip, tb, Default::default(), img, 0, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lm_port_fake::{Config, FakePort};

    #[test]
    fn a_waiting_block_that_scrolled_away_is_not_asked_again() {
        let mut port = FakePort::new(Config { tracks: 1, slow_delay: None, slow_track: usize::MAX }).expect("the demo");
        let res = Resource::new("sample", "0000000000000000").expect("a valid key");
        let mut cache = TileCache::default();
        cache.want(&res, 0, 0..1);
        cache.fetch(&mut port, Default::default());
        // unknown to the port: answered Waiting, so another frame is asked
        assert!(cache.waiting());
        // the next frame no longer wants it: it is dropped, and the animation stops
        cache.fetch(&mut port, Default::default());
        assert!(!cache.waiting());
    }

    #[test]
    fn a_block_in_flight_is_asked_once_and_a_failure_stops_it() {
        let mut port = FakePort::new(Config { tracks: 1, slow_delay: None, slow_track: usize::MAX }).expect("the demo");
        let res = Resource::new("sample", "0000000000000001").expect("a valid key");
        let mut cache = TileCache::default();
        cache.want(&res, 0, 0..1);
        cache.fetch(&mut port, Default::default());
        let id = *cache.in_flight.keys().next().expect("asked");
        // wanted again while the answer has not come: not asked a second time
        cache.want(&res, 0, 0..1);
        cache.fetch(&mut port, Default::default());
        assert_eq!(cache.in_flight.len(), 1);
        assert!(cache.waiting());
        // the answer failed: reported once, and never asked again
        let errors = cache.fetch(&mut port, (Vec::new(), vec![(id, Error::new("test.failed"))]));
        assert_eq!(errors.len(), 1);
        cache.want(&res, 0, 0..1);
        cache.fetch(&mut port, Default::default());
        assert!(!cache.waiting());
        assert!(cache.in_flight.is_empty());
    }

    #[test]
    fn the_resources_kept_are_bounded_and_forget_removes_the_name() {
        let mut cache = TileCache::default();
        for i in 0..(MAX_RESOURCES + 40) {
            let res = Resource::new("sample", &format!("{i:016x}")).expect("a valid key");
            cache.want(&res, 0, 0..1);
        }
        assert_eq!(cache.resources(), MAX_RESOURCES);
        let res = Resource::new("sample", &format!("{:016x}", MAX_RESOURCES + 39)).expect("a valid key");
        assert_eq!(cache.resources(), cache.by_slot.len());
        cache.forget(&res);
        assert_eq!(cache.resources(), MAX_RESOURCES - 1);
        assert_eq!(cache.used.len(), cache.names.len());
    }
}
