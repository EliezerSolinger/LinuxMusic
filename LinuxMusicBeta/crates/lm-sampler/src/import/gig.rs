//! GIG (GigaStudio): a pure-Rust reader of the `.gig` files the GigPlayer played with libgig. A `.gig` is a RIFF/DLS
//! file with Gigasampler chunks; this reads exactly what the GigPlayer used, and nothing else:
//!
//! - the instruments (`lins`/`ins `: bank, program, name) and their regions (`rgn `: key range);
//! - the dimensions of a region (`3lnk`: velocity, release trigger, sustain pedal, random, round robin, layer, sample
//!   channel, keyboard, and the controller dimensions) and the dimension regions (`3prg`/`3ewl`) with their `3ewa`
//!   (the volume envelope EG1, the velocity response curve, the pitch tracking, the release trigger decay, the zone
//!   limits) and `wsmp` (unity note, fine tune, gain, the first loop);
//! - the wave pool (`ptbl`, `wvpl`) with the 16 and 24-bit PCM samples (mono or stereo).
//!
//! Each dimension region becomes a [`Zone`]. The dimensions map to what the model has: the velocity splits to the
//! velocity ranges (the file's custom zone limits, or even zones), the release trigger to [`Trigger::Release`], the
//! sustain pedal to [`Zone::pedal`], the random dimension to [`Zone::random`], round robin to [`Zone::sequence`],
//! layers all play, the sample channel dimension plays its two mono samples left and right. The keyboard dimension
//! (key switches) and the controller dimensions take their first zone, as the GigPlayer did.
//!
//! The samples are decoded at half of full scale (16 bit: `raw / 65536`), as the GigPlayer read them. Compressed gig
//! samples (the `EWAV` chunk) are **not supported**: an instrument that uses one fails with `sampler.gig.compressed`.
//! Only the samples of the chosen instrument are decoded.

use std::collections::HashMap;
use std::sync::Arc;

use lm_base::{Error, Result};

use crate::import::pat::MONO_CENTER_DB;
use crate::model::{Ahdsr, Instrument, LoopMode, PresetInfo, Sequence, Trigger, Zone};
use crate::pcm::Pcm;
use crate::stream::{FileTail, Raw, StreamPlan};

fn fail(reason: &'static str) -> Error {
    Error::new("sampler.gig.invalid").arg("reason", reason)
}

// ---- RIFF ----

fn u8_at(b: &[u8], o: usize) -> u8 {
    b.get(o).copied().unwrap_or(0)
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from(u8_at(b, o)) | u16::from(u8_at(b, o + 1)) << 8
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from(u16_at(b, o)) | u32::from(u16_at(b, o + 2)) << 16
}

fn i16_at(b: &[u8], o: usize) -> i16 {
    u16_at(b, o) as i16
}

fn i32_at(b: &[u8], o: usize) -> i32 {
    u32_at(b, o) as i32
}

/// A chunk: a list has its type and the body after it
#[derive(Clone, Copy)]
struct Ck<'a> {
    id: [u8; 4],
    kind: Option<[u8; 4]>,
    /// Where its header starts in the file
    pos: usize,
    data: &'a [u8],
}

impl<'a> Ck<'a> {
    fn is(&self, id: &[u8; 4]) -> bool {
        &self.id == id
    }

    fn is_list(&self, kind: &[u8; 4]) -> bool {
        self.is(b"LIST") && self.kind.as_ref() == Some(kind)
    }

    /// The chunks inside a list
    fn kids(&self) -> Vec<Ck<'a>> {
        children(self.data, self.pos + 12)
    }

    fn find(&self, id: &[u8; 4]) -> Option<Ck<'a>> {
        self.kids().into_iter().find(|c| c.is(id))
    }

    fn find_list(&self, kind: &[u8; 4]) -> Option<Ck<'a>> {
        self.kids().into_iter().find(|c| c.is_list(kind))
    }
}

/// The chunks of a body that starts at `base` in the file
fn children(body: &[u8], base: usize) -> Vec<Ck<'_>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= body.len() {
        let id = [body[at], body[at + 1], body[at + 2], body[at + 3]];
        let size = u32_at(body, at + 4) as usize;
        let end = (at + 8).saturating_add(size).min(body.len());
        let mut data = &body[at + 8..end];
        let mut kind = None;
        if &id == b"LIST" && data.len() >= 4 {
            kind = Some([data[0], data[1], data[2], data[3]]);
            data = &data[4..];
        }
        out.push(Ck { id, kind, pos: base + at, data });
        at = (at + 8).saturating_add(size).saturating_add(size & 1);
    }
    out
}

// ---- the dimension regions ----

const DIM_SAMPLE_CHANNEL: u8 = 0x80;
const DIM_LAYER: u8 = 0x81;
const DIM_VELOCITY: u8 = 0x82;
const DIM_RELEASE_TRIGGER: u8 = 0x84;
const DIM_ROUND_ROBIN: u8 = 0x86;
const DIM_RANDOM: u8 = 0x87;
const DIM_ROUND_ROBIN_KEYBOARD: u8 = 0x89;
const DIM_SUSTAIN_PEDAL: u8 = 0x40;

fn exp_decode(x: i32) -> f64 {
    libm::pow(1.000_000_008_813_822, f64::from(x))
}

#[derive(Clone, Copy, Debug)]
struct Dim {
    kind: u8,
    bits: u32,
    zones: u32,
}

impl Dim {
    fn zone_size(&self) -> u32 {
        (128 / self.zones.max(1)).max(1)
    }
}

#[derive(Clone, Copy, Debug)]
struct Loop {
    kind: u32,
    start: u32,
    length: u32,
}

#[derive(Clone, Debug)]
struct DimRegion {
    /// Index in the file's waves
    wave: Option<usize>,
    unity: u8,
    fine_tune: i16,
    gain: i32,
    first_loop: Option<Loop>,
    eg1_attack: f64,
    eg1_decay: f64,
    eg1_sustain: u16,
    eg1_release: f64,
    pitch_track: bool,
    pan: i8,
    release_decay: u8,
    vel_curve: (u32, u8, u8),
    vel_upper: u8,
    upper: [u8; 8],
}

impl DimRegion {
    fn read(list: &Ck<'_>) -> Self {
        let mut d = DimRegion {
            wave: None,
            unity: 60,
            fine_tune: 0,
            gain: 0,
            first_loop: None,
            eg1_attack: 0.0,
            eg1_decay: 0.005,
            eg1_sustain: 1000,
            eg1_release: 0.3,
            pitch_track: true,
            pan: 0,
            release_decay: 0,
            vel_curve: (0, 3, 32),
            vel_upper: 0,
            upper: [127; 8],
        };
        if let Some(w) = list.find(b"wsmp") {
            let b = w.data;
            let header = u32_at(b, 0) as usize;
            d.unity = u16_at(b, 4).min(127) as u8;
            d.fine_tune = i16_at(b, 6);
            d.gain = i32_at(b, 8);
            let loops = u32_at(b, 16) as usize;
            if loops > 0 {
                let size = (u32_at(b, header) as usize).max(16);
                let at = header;
                if size >= 16 && at + 16 <= b.len() {
                    d.first_loop =
                        Some(Loop { kind: u32_at(b, at + 4), start: u32_at(b, at + 8), length: u32_at(b, at + 12) });
                }
            }
        }
        if let Some(e) = list.find(b"3ewa")
            && e.data.len() >= 140
        {
            let b = e.data;
            d.eg1_attack = exp_decode(i32_at(b, 28));
            d.eg1_decay = exp_decode(i32_at(b, 32));
            d.eg1_sustain = u16_at(b, 38);
            d.eg1_release = exp_decode(i32_at(b, 40));
            let response = u8_at(b, 96);
            d.vel_curve = match response {
                0..=4 => (0, response, u8_at(b, 98)),
                5..=9 => (1, response - 5, u8_at(b, 98)),
                10..=14 => (2, response - 10, u8_at(b, 98)),
                _ => (u32::MAX, 0, 0),
            };
            d.pitch_track = u8_at(b, 108) & 0x01 == 0;
            let pan = u8_at(b, 109);
            d.pan = if pan < 64 { pan as i8 } else { (-(i32::from(pan) - 63)) as i8 };
            d.vel_upper = u8_at(b, 124);
            d.release_decay = u8_at(b, 128);
            d.upper = if b.len() >= 148 {
                let mut u = [0u8; 8];
                u.copy_from_slice(&b[140..148]);
                u
            } else {
                [0; 8]
            };
        }
        d
    }
}

#[derive(Clone, Debug)]
struct Region {
    keys: (u8, u8),
    dims: Vec<Dim>,
    /// The dimension regions by their slot (the position in the file's list)
    slots: Vec<DimRegion>,
}

// libgig's line-segment approximations of the velocity curves (nonlinear 0-4, linear 5-9, special 10-15)
const CURVES: [&[i32]; 16] = [
    &[1, 4, 24, 5, 57, 17, 92, 57, 122, 127, 127, 127],
    &[1, 4, 46, 9, 93, 56, 118, 106, 123, 127, 127, 127],
    &[1, 4, 46, 9, 57, 20, 102, 107, 107, 127, 127, 127],
    &[1, 15, 10, 19, 67, 73, 80, 80, 90, 98, 98, 127, 127, 127],
    &[1, 25, 33, 57, 82, 81, 92, 127, 127, 127],
    &[1, 1, 127, 127],
    &[1, 21, 127, 127],
    &[1, 45, 127, 127],
    &[1, 74, 127, 127],
    &[1, 127, 127, 127],
    &[1, 2, 76, 10, 90, 15, 95, 20, 99, 28, 103, 44, 113, 127, 127, 127],
    &[1, 2, 27, 5, 67, 18, 89, 29, 95, 35, 107, 67, 118, 127, 127, 127],
    &[1, 1, 33, 1, 53, 5, 61, 13, 69, 32, 79, 74, 85, 90, 91, 127, 127, 127],
    &[1, 32, 28, 35, 66, 48, 89, 59, 95, 65, 99, 73, 117, 127, 127, 127],
    &[1, 4, 23, 5, 49, 13, 57, 17, 92, 57, 122, 127, 127, 127],
    &[1, 2, 30, 5, 60, 19, 77, 70, 83, 85, 88, 106, 91, 127, 127, 127],
];

/// The level of each MIDI velocity for a velocity response curve (libgig's `CreateVelocityTable`)
fn velocity_table(kind: u32, depth: u8, scaling: u8) -> [f32; 128] {
    let (mut kind, mut depth, mut scaling) = (kind, depth, scaling);
    let bad = match kind {
        0 | 1 => depth > 4,
        2 => depth > 5,
        _ => true,
    };
    if bad {
        // (an unknown curve or depth: linear, depth 0, unscaled, as libgig falls back)
        if kind > 2 {
            kind = 1;
        }
        depth = 0;
        scaling = 0;
    }
    let curve = CURVES[(kind as usize * 5 + usize::from(depth)).min(15)];
    let s = if scaling == 0 { 20.0 } else { f64::from(scaling) };
    let mut table = [0.0f32; 128];
    let mut at = 0usize;
    for (x, slot) in table.iter_mut().enumerate().skip(1) {
        let x = x as i32;
        if x > curve[at + 2] && at + 4 <= curve.len() {
            at += 2;
        }
        let (x0, y0, x1, y1) = (curve[at], curve[at + 1], curve[at + 2], curve[at + 3]);
        let mut y = f64::from(y0) + f64::from(x - x0) * (f64::from(y1 - y0) / f64::from((x1 - x0).max(1)));
        y /= 127.0;
        if s < 20.0 && y >= 0.5 {
            y /= (2.0 - 40.0 / s) * y + 40.0 / s - 1.0;
        } else {
            y *= s / 20.0;
        }
        *slot = y.min(1.0) as f32;
    }
    table
}

// ---- the file ----

struct Wave<'a> {
    offset: u64,
    format: u16,
    channels: u16,
    rate: u32,
    bits: u16,
    compressed: bool,
    data: &'a [u8],
    /// Where `data` is in the file
    data_at: Option<usize>,
}

#[derive(Clone, Debug)]
struct Ins {
    name: String,
    bank: u16,
    program: u16,
    regions: Vec<Region>,
}

/// A parsed `.gig`: the instruments' structure, with the samples still in the file's bytes
pub struct GigFile<'a> {
    instruments: Vec<Ins>,
    waves: Vec<Wave<'a>>,
    stream: Option<StreamPlan>,
}

impl std::fmt::Debug for GigFile<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GigFile")
            .field("instruments", &self.instruments.len())
            .field("waves", &self.waves.len())
            .finish()
    }
}

fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

impl<'a> GigFile<'a> {
    /// The long uncompressed waves stream from `plan`'s file (which must be the file `parse` read) instead of being
    /// decoded whole
    #[must_use]
    pub fn streaming(mut self, plan: StreamPlan) -> Self {
        self.stream = Some(plan);
        self
    }

    /// Reads the structure of a file's bytes. Keys: `sampler.gig.invalid`
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"DLS " {
            return Err(fail("not a RIFF DLS file"));
        }
        let top = children(&bytes[12..], 12);
        let major = top.iter().find(|c| c.is(b"vers")).map_or(0, |c| u16_at(c.data, 2));
        let v3 = major > 2;
        // The wave pool
        let mut pool: Vec<u64> = Vec::new();
        if let Some(p) = top.iter().find(|c| c.is(b"ptbl")) {
            let header = u32_at(p.data, 0) as usize;
            let count = (u32_at(p.data, 4) as usize).min(p.data.len() / 4);
            let wide = p.data.len().saturating_sub(header) == count * 8;
            for i in 0..count {
                pool.push(if wide {
                    let hi = u64::from(u32_at(p.data, header + i * 8));
                    let lo = u64::from(u32_at(p.data, header + i * 8 + 4));
                    hi << 32 | lo
                } else {
                    u64::from(u32_at(p.data, header + i * 4))
                });
            }
        }
        let mut waves = Vec::new();
        if let Some(wvpl) = top.iter().find(|c| c.is_list(b"wvpl")) {
            for w in wvpl.kids().into_iter().filter(|c| c.is_list(b"wave")) {
                let fmt = w.find(b"fmt ");
                let data = w.find(b"data").map_or(&[][..], |d| d.data);
                let (format, channels, rate, bits) = match fmt {
                    Some(f) => (u16_at(f.data, 0), u16_at(f.data, 2), u32_at(f.data, 4), u16_at(f.data, 14)),
                    None => (1, 1, 44_100, 16),
                };
                waves.push(Wave {
                    offset: (w.pos - wvpl.pos - 12) as u64,
                    format,
                    channels,
                    rate,
                    bits,
                    compressed: w.find(b"EWAV").is_some(),
                    data,
                    data_at: crate::stream::offset_in(bytes, data),
                });
            }
        }
        let wave_of = |index: u32| -> Option<usize> {
            if index == u32::MAX {
                return None;
            }
            let offset = *pool.get(index as usize)?;
            waves.iter().position(|w| w.offset == offset)
        };
        let mut instruments = Vec::new();
        if let Some(lins) = top.iter().find(|c| c.is_list(b"lins")) {
            for ins in lins.kids().into_iter().filter(|c| c.is_list(b"ins ")) {
                let insh = ins.find(b"insh").map_or(&[][..], |c| c.data);
                let bank_raw = u32_at(insh, 4);
                let bank = ((((bank_raw & 0x7F00) >> 8) << 7) | (bank_raw & 0x7F)) as u16;
                let program = u32_at(insh, 8) as u16;
                let name =
                    ins.find_list(b"INFO").and_then(|i| i.find(b"INAM").map(|n| text(n.data))).unwrap_or_default();
                let mut regions = Vec::new();
                if let Some(lrgn) = ins.find_list(b"lrgn") {
                    for rgn in lrgn.kids().into_iter().filter(|c| c.is_list(b"rgn ")) {
                        if let Some(r) = read_region(&rgn, v3, &wave_of) {
                            regions.push(r);
                        }
                    }
                }
                instruments.push(Ins { name, bank, program, regions });
            }
        }
        Ok(Self { instruments, waves, stream: None })
    }

    /// The instruments, in the file's order
    pub fn presets(&self) -> Vec<PresetInfo> {
        self.instruments.iter().map(|i| PresetInfo { bank: i.bank, program: i.program, name: i.name.clone() }).collect()
    }

    /// The zones of the instrument at `bank`/`program`, with the samples it uses decoded. Keys:
    /// `sampler.gig.no_preset`, `sampler.gig.compressed`, `sampler.gig.format`
    pub fn import(&self, bank: u16, program: u16) -> Result<Instrument> {
        let ins = self.instruments.iter().find(|i| i.bank == bank && i.program == program).ok_or_else(|| {
            Error::new("sampler.gig.no_preset").arg("bank", i64::from(bank)).arg("program", i64::from(program))
        })?;
        let mut out = Instrument::new(if ins.name.is_empty() { "gig" } else { ins.name.as_str() });
        let mut pcms: HashMap<usize, Arc<Pcm>> = HashMap::new();
        let mut round_groups = 0u16;
        for region in &ins.regions {
            self.region_zones(region, &mut pcms, &mut round_groups, &mut out.zones)?;
        }
        out.validate()?;
        Ok(out)
    }

    fn pcm(&self, index: usize, cache: &mut HashMap<usize, Arc<Pcm>>) -> Result<Option<Arc<Pcm>>> {
        if let Some(p) = cache.get(&index) {
            return Ok(Some(Arc::clone(p)));
        }
        let Some(w) = self.waves.get(index) else { return Ok(None) };
        if w.compressed {
            return Err(Error::new("sampler.gig.compressed"));
        }
        if w.format != 1 || !(w.bits == 16 || w.bits == 24) || w.channels == 0 {
            return Err(Error::new("sampler.gig.format")
                .arg("bits", i64::from(w.bits))
                .arg("format", i64::from(w.format)));
        }
        let stride = usize::from(w.channels);
        let width = usize::from(w.bits / 8);
        let frames = w.data.len() / (stride * width);
        if frames == 0 {
            return Ok(None);
        }
        let sample = |frame: usize, c: usize| -> f32 {
            let o = (frame * stride + c) * width;
            if width == 3 {
                let raw = i32::from(w.data[o]) << 8 | i32::from(w.data[o + 1]) << 16 | i32::from(w.data[o + 2]) << 24;
                raw as f32 / 4_294_967_296.0
            } else {
                f32::from(i16::from_le_bytes([w.data[o], w.data[o + 1]])) / 65_536.0
            }
        };
        if let (Some(plan), Some(at)) = (&self.stream, w.data_at)
            && frames > plan.min_frames
            && stride <= 2
        {
            // The long wave keeps its head; the rest is read from the file as the voices reach it (half amplitude, as
            // above)
            let head = frames.min(plan.head);
            let left: Vec<f32> = (0..head).map(|i| sample(i, 0)).collect();
            let head_pcm = if stride == 2 {
                Pcm::stereo(w.rate, left, (0..head).map(|i| sample(i, 1)).collect())
            } else {
                Pcm::mono(w.rate, left)
            };
            let raw = if width == 3 { Raw::I24 } else { Raw::I16 };
            let tail = FileTail::new(Arc::clone(&plan.file), at as u64, frames, stride, raw).scaled(0.5);
            let pcm = Arc::new(Pcm::streamed(head_pcm, Arc::new(tail)));
            cache.insert(index, Arc::clone(&pcm));
            return Ok(Some(pcm));
        }
        let left: Vec<f32> = (0..frames).map(|i| sample(i, 0)).collect();
        let pcm = if stride >= 2 {
            Pcm::stereo(w.rate, left, (0..frames).map(|i| sample(i, 1)).collect())
        } else {
            Pcm::mono(w.rate, left)
        };
        let pcm = Arc::new(pcm);
        cache.insert(index, Arc::clone(&pcm));
        Ok(Some(pcm))
    }

    fn region_zones(
        &self,
        region: &Region,
        pcms: &mut HashMap<usize, Arc<Pcm>>,
        round_groups: &mut u16,
        zones: &mut Vec<Zone>,
    ) -> Result<()> {
        let veldim = region.dims.iter().position(|d| d.kind == DIM_VELOCITY);
        let mut round_group = None;
        // Every combination of the other dimensions' zones
        let mut combos: Vec<Vec<(usize, Pick)>> = vec![Vec::new()];
        for (i, d) in region.dims.iter().enumerate() {
            if Some(i) == veldim {
                continue;
            }
            let picks = picks_of(d);
            let mut next = Vec::with_capacity(combos.len() * picks.len());
            for c in &combos {
                for p in &picks {
                    let mut c = c.clone();
                    c.push((i, *p));
                    next.push(c);
                }
            }
            combos = next;
        }
        for combo in combos {
            // The slot with the velocity zone 0
            let mut index = 0usize;
            let mut bit = 0u32;
            let mut vel_bit = 0u32;
            for (i, d) in region.dims.iter().enumerate() {
                if Some(i) == veldim {
                    vel_bit = bit;
                } else if let Some((_, p)) = combo.iter().find(|(j, _)| *j == i) {
                    index |= (p.zone as usize) << bit;
                }
                bit += d.bits;
            }
            // The velocity ranges that pick the same slot
            let mut ranges: Vec<(u8, u8, usize)> = Vec::new();
            for v in 1u8..=127 {
                let slot = match veldim {
                    Some(vd) => {
                        let zone = velocity_zone(region, vd, index, vel_bit, v);
                        let mask = (1usize << region.dims[vd].bits) - 1;
                        (index | (zone & mask) << vel_bit) & 255
                    }
                    None => index & 255,
                };
                match ranges.last_mut() {
                    Some(last) if last.2 == slot => last.1 = v,
                    _ => ranges.push((v, v, slot)),
                }
            }
            for (lo, hi, slot) in ranges {
                let Some(dr) = region.slots.get(slot) else { continue };
                let Some(wave) = dr.wave else { continue };
                let Some(pcm) = self.pcm(wave, pcms)? else { continue };
                let mut z = zone_of(dr, pcm);
                z.keys = region.keys;
                z.vels = (lo, hi);
                for (_, p) in &combo {
                    match p.role {
                        Role::None => {}
                        Role::Channel(c) => z.pan = if c == 0 { -1.0 } else { 1.0 },
                        Role::Release => {
                            z.trigger = Trigger::Release;
                            z.loop_mode = LoopMode::Off;
                            z.release_decay = 0.0915 * (256u32 >> u32::from(dr.release_decay.min(8))) as f32;
                            z.gain -= 20.0 * libm::log10f(sample_attenuation(dr.gain)).max(-120.0);
                        }
                        Role::Pedal(down) => z.pedal = Some(down),
                        Role::Round(position, length) => {
                            let group = *round_group.get_or_insert_with(|| {
                                *round_groups += 1;
                                *round_groups - 1
                            });
                            z.sequence = Some(Sequence { group, length: length as u16, position: position as u16 + 1 });
                        }
                        Role::Random(n, of) => z.random = Some((n as f32 / of as f32, (n + 1) as f32 / of as f32)),
                    }
                }
                zones.push(z);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
enum Role {
    None,
    Channel(u32),
    Release,
    Pedal(bool),
    /// The zone's position and how many take turns
    Round(u32, u32),
    Random(u32, u32),
}

#[derive(Clone, Copy, Debug)]
struct Pick {
    zone: u32,
    role: Role,
}

/// The zones of a dimension that play (the first for a controller or key switch one)
fn picks_of(d: &Dim) -> Vec<Pick> {
    let n = d.zones.max(1);
    let all = |f: &dyn Fn(u32) -> Role| (0..n).map(|z| Pick { zone: z, role: f(z) }).collect();
    match d.kind {
        DIM_SAMPLE_CHANNEL => all(&|z| Role::Channel(z)),
        DIM_LAYER => all(&|_| Role::None),
        DIM_RELEASE_TRIGGER => {
            (0..n.min(2)).map(|z| Pick { zone: z, role: if z == 1 { Role::Release } else { Role::None } }).collect()
        }
        DIM_ROUND_ROBIN | DIM_ROUND_ROBIN_KEYBOARD => all(&|z| Role::Round(z, n)),
        DIM_RANDOM => all(&|z| Role::Random(z, n)),
        DIM_SUSTAIN_PEDAL => {
            let mut v = vec![Pick { zone: 0, role: Role::Pedal(false) }];
            if n >= 2 {
                v.push(Pick { zone: n - 1, role: Role::Pedal(true) });
            }
            v
        }
        _ => vec![Pick { zone: 0, role: Role::None }],
    }
}

/// libgig's velocity zone of `v` for the slot with velocity zone 0 at `base`
fn velocity_zone(region: &Region, veldim: usize, base: usize, vel_bit: u32, v: u8) -> usize {
    let d = region.dims[veldim];
    let zones = d.zones.max(1) as usize;
    let first = region.slots.get(base & 255);
    let custom = first.is_some_and(|f| f.upper[veldim.min(7)] != 0 || f.vel_upper != 0);
    if custom {
        for k in 0..zones {
            let Some(s) = region.slots.get((base + (k << vel_bit)) & 255) else { break };
            let limit =
                if first.is_some_and(|f| f.upper[veldim.min(7)] != 0) { s.upper[veldim.min(7)] } else { s.vel_upper };
            if v <= limit {
                return k;
            }
        }
        return zones - 1;
    }
    (usize::from(v) / d.zone_size() as usize).min(zones - 1)
}

fn sample_attenuation(gain: i32) -> f32 {
    libm::pow(10.0, -f64::from(gain) / (20.0 * 655_360.0)) as f32
}

/// A zone from a dimension region and its sample (the keys, velocities and dimension roles are set by the caller)
fn zone_of(dr: &DimRegion, pcm: Arc<Pcm>) -> Zone {
    let frames = pcm.frames() as u32;
    let mono = !pcm.is_stereo();
    let mut z = Zone::new(pcm);
    z.root_key = dr.unity;
    z.key_track = if dr.pitch_track { 100.0 } else { 0.0 };
    z.tune = if dr.pitch_track { f32::from(dr.fine_tune) } else { 0.0 };
    z.gain = 20.0 * libm::log10f(sample_attenuation(dr.gain)).max(-120.0) + if mono { MONO_CENTER_DB } else { 0.0 };
    z.pan = (f32::from(dr.pan) / 63.0).clamp(-1.0, 1.0);
    if let Some(l) = dr.first_loop
        && l.length > 0
        && l.start < frames
    {
        z.loop_mode = if l.kind == 1 { LoopMode::PingPong } else { LoopMode::Forward };
        z.loop_start = l.start;
        z.loop_end = l.start.saturating_add(l.length).min(frames);
    }
    let table = velocity_table(dr.vel_curve.0, dr.vel_curve.1, dr.vel_curve.2);
    z.vel_table = Some(Arc::new(table));
    // The volume envelope: the file's linear decay to the sustain over `decay1` seconds, and a release that falls as the
    // GigPlayer's curve (exp(-5 t / release)) does
    let sustain = (f32::from(dr.eg1_sustain) / 1000.0).clamp(0.0, 1.0);
    let floor = sustain.max(1.0e-5);
    let decay = if sustain < 1.0 { (dr.eg1_decay as f32) * 100.0 / (-20.0 * libm::log10f(floor)) } else { 0.0 };
    z.amp = Ahdsr {
        delay: 0.0,
        attack: dr.eg1_attack as f32,
        hold: 0.0,
        decay,
        sustain: floor,
        release: ((dr.eg1_release as f32) * 100.0 / 43.43).max(0.001),
    };
    z
}

fn read_region(rgn: &Ck<'_>, v3: bool, wave_of: &dyn Fn(u32) -> Option<usize>) -> Option<Region> {
    let rgnh = rgn.find(b"rgnh").map_or(&[][..], |c| c.data);
    let keys =
        if rgnh.len() >= 4 { (u16_at(rgnh, 0).min(127) as u8, u16_at(rgnh, 2).min(127) as u8) } else { (0, 127) };
    let lnk = rgn.find(b"3lnk")?;
    let max_dims = if v3 { 8 } else { 5 };
    let mut dims = Vec::new();
    for i in 0..max_dims {
        let at = 4 + i * 8;
        let kind = u8_at(lnk.data, at);
        if kind == 0 {
            continue;
        }
        let bits = u32::from(u8_at(lnk.data, at + 1)).min(8);
        let zones = u32::from(u8_at(lnk.data, at + 4));
        // A dimension cannot have more zones than its bits address: the combinations of the dimensions stay within the
        // 256 slots, whatever the file says
        dims.push(Dim { kind, bits, zones: if zones == 0 { 1 << bits } else { zones.min(1 << bits) } });
    }
    let total_bits: u32 = dims.iter().map(|d| d.bits).sum();
    if total_bits > 8 {
        return None;
    }
    let prg = rgn.find_list(b"3prg")?;
    let lists: Vec<Ck<'_>> = prg.kids().into_iter().filter(|c| c.is_list(b"3ewl")).collect();
    if lists.is_empty() {
        return None;
    }
    let table = if v3 { 68 } else { 44 };
    let mut slots: Vec<DimRegion> = lists.iter().map(DimRegion::read).collect();
    for (i, s) in slots.iter_mut().enumerate() {
        s.wave = wave_of(u32_at(lnk.data, table + i * 4));
    }
    Some(Region { keys, dims, slots })
}

/// The instruments of a `.gig` file's bytes
pub fn list_presets(bytes: &[u8]) -> Result<Vec<PresetInfo>> {
    Ok(GigFile::parse(bytes)?.presets())
}

/// One instrument of a `.gig` file's bytes, its long waves streaming from `plan`'s file
pub fn import_streaming(bytes: &[u8], bank: u16, program: u16, plan: StreamPlan) -> Result<Instrument> {
    GigFile::parse(bytes)?.streaming(plan).import(bank, program)
}

/// One instrument of a `.gig` file's bytes
pub fn import(bytes: &[u8], bank: u16, program: u16) -> Result<Instrument> {
    GigFile::parse(bytes)?.import(bank, program)
}

#[cfg(test)]
mod tests;
