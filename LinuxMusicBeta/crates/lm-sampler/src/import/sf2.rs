//! SF2: a pure-Rust SoundFont 2 reader. It covers the presets, their instruments, the zones with the generators
//! that shape a sample's sound (ranges, tuning, loops, the volume and modulation envelopes, the filter, pan,
//! attenuation, exclusive classes), the preset-level generators added to the instrument's, and the modulators that
//! change what a zone sounds like on its own: the velocity to attenuation and to filter cutoff curves (the
//! defaults, and the ones a file overrides). The controller modulators (modulation wheel, pressure, reverb and
//! chorus sends) and the LFOs are not part of the model and are skipped.
//!
//! `.sf3` (the Ogg-compressed variant) needs a Vorbis decoder: the plugin gives one through [`Sf3Decoder`]; without
//! it a compressed sample fails the import with `sampler.sf2.compressed`.

use std::collections::HashMap;
use std::sync::Arc;

use lm_base::{Error, Result};

use crate::model::{Ahdsr, Filter, FilterKind, Instrument, LoopMode, PresetInfo, VelCurve, Zone};
use crate::pcm::Pcm;
use crate::stream::{FileTail, Raw, StreamPlan};

fn fail(reason: &'static str) -> Error {
    Error::new("sampler.sf2.invalid").arg("reason", reason)
}

/// Decodes a compressed (`.sf3`) sample: the bytes of one Ogg Vorbis stream to its mono frames
pub trait Sf3Decoder {
    fn decode(&mut self, ogg: &[u8], rate: u32) -> Result<Vec<f32>>;
}

// Generator numbers (SoundFont 2.04, 8.1.2)
mod g {
    pub const START_ADDRS_OFFSET: u16 = 0;
    pub const END_ADDRS_OFFSET: u16 = 1;
    pub const STARTLOOP_ADDRS_OFFSET: u16 = 2;
    pub const ENDLOOP_ADDRS_OFFSET: u16 = 3;
    pub const START_ADDRS_COARSE: u16 = 4;
    pub const INITIAL_FILTER_FC: u16 = 8;
    pub const INITIAL_FILTER_Q: u16 = 9;
    pub const MOD_ENV_TO_FILTER_FC: u16 = 11;
    pub const END_ADDRS_COARSE: u16 = 12;
    pub const PAN: u16 = 17;
    pub const DELAY_MOD_ENV: u16 = 25;
    pub const ATTACK_MOD_ENV: u16 = 26;
    pub const HOLD_MOD_ENV: u16 = 27;
    pub const DECAY_MOD_ENV: u16 = 28;
    pub const SUSTAIN_MOD_ENV: u16 = 29;
    pub const RELEASE_MOD_ENV: u16 = 30;
    pub const DELAY_VOL_ENV: u16 = 33;
    pub const ATTACK_VOL_ENV: u16 = 34;
    pub const HOLD_VOL_ENV: u16 = 35;
    pub const DECAY_VOL_ENV: u16 = 36;
    pub const SUSTAIN_VOL_ENV: u16 = 37;
    pub const RELEASE_VOL_ENV: u16 = 38;
    pub const INSTRUMENT: u16 = 41;
    pub const KEY_RANGE: u16 = 43;
    pub const VEL_RANGE: u16 = 44;
    pub const STARTLOOP_ADDRS_COARSE: u16 = 45;
    pub const INITIAL_ATTENUATION: u16 = 48;
    pub const ENDLOOP_ADDRS_COARSE: u16 = 50;
    pub const COARSE_TUNE: u16 = 51;
    pub const FINE_TUNE: u16 = 52;
    pub const SAMPLE_ID: u16 = 53;
    pub const SAMPLE_MODES: u16 = 54;
    pub const SCALE_TUNING: u16 = 56;
    pub const EXCLUSIVE_CLASS: u16 = 57;
    pub const OVERRIDING_ROOT_KEY: u16 = 58;
    pub const COUNT: usize = 60;
}

/// A generator's value: the amount's two bytes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Amount(u16);

impl Amount {
    fn i16(self) -> i32 {
        i32::from(self.0 as i16)
    }

    fn range(self) -> (u8, u8) {
        ((self.0 & 0xFF) as u8, (self.0 >> 8) as u8)
    }
}

#[derive(Clone, Copy, Debug)]
struct Gen {
    oper: u16,
    amount: Amount,
}

#[derive(Clone, Copy, Debug)]
struct Modulator {
    src: u16,
    dest: u16,
    amount: i32,
    amt_src: u16,
    transform: u16,
}

#[derive(Clone, Copy, Debug)]
struct Bag {
    first_gen: usize,
    modulator: usize,
}

#[derive(Clone, Debug)]
struct PresetHeader {
    name: String,
    program: u16,
    bank: u16,
    bag: usize,
}

#[derive(Clone, Debug)]
struct InstHeader {
    bag: usize,
}

#[derive(Clone, Debug)]
struct SampleHeader {
    start: u32,
    end: u32,
    start_loop: u32,
    end_loop: u32,
    rate: u32,
    pitch: u8,
    correction: i8,
    kind: u16,
}

/// A parsed SoundFont: its tables and where its sample data is. Nothing is decoded until [`SoundFont::import`]
pub struct SoundFont<'a> {
    presets: Vec<PresetHeader>,
    pbag: Vec<Bag>,
    pmod: Vec<Modulator>,
    pgen: Vec<Gen>,
    insts: Vec<InstHeader>,
    ibag: Vec<Bag>,
    imod: Vec<Modulator>,
    igen: Vec<Gen>,
    samples: Vec<SampleHeader>,
    smpl: &'a [u8],
    sm24: &'a [u8],
    /// Where `smpl` is in the file, and what to stream (a sample of many frames keeps only its head in memory)
    smpl_at: Option<usize>,
    stream: Option<StreamPlan>,
}

impl std::fmt::Debug for SoundFont<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundFont")
            .field("presets", &self.presets.len())
            .field("instruments", &self.insts.len())
            .field("samples", &self.samples.len())
            .finish()
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
}

impl Cursor<'_> {
    fn u16(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]])
    }

    fn u32(&self, at: usize) -> u32 {
        u32::from_le_bytes([self.bytes[at], self.bytes[at + 1], self.bytes[at + 2], self.bytes[at + 3]])
    }

    fn name(&self, at: usize) -> String {
        let raw = &self.bytes[at..at + 20];
        let end = raw.iter().position(|b| *b == 0).unwrap_or(20);
        String::from_utf8_lossy(&raw[..end]).trim().to_owned()
    }
}

/// The chunks of a RIFF list: (id, body)
fn chunks(mut bytes: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    while bytes.len() >= 8 {
        let id = [bytes[0], bytes[1], bytes[2], bytes[3]];
        let size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        let end = (8 + size).min(bytes.len());
        out.push((id, &bytes[8..end]));
        let next = (8 + size + (size & 1)).min(bytes.len());
        bytes = &bytes[next..];
    }
    out
}

fn records<'a>(body: &'a [u8], size: usize) -> impl Iterator<Item = Cursor<'a>> + 'a {
    body.chunks_exact(size).map(|bytes| Cursor { bytes })
}

impl<'a> SoundFont<'a> {
    /// The long 16-bit samples of this font stream from `plan`'s file (which must be the file `parse` read) instead of
    /// being decoded whole; a compressed or 24-bit sample is decoded whole as before
    #[must_use]
    pub fn streaming(mut self, plan: StreamPlan) -> Self {
        self.stream = Some(plan);
        self
    }

    /// Parses the file's bytes. Keys: `sampler.sf2.invalid`
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"sfbk" {
            return Err(fail("not a SoundFont"));
        }
        let mut smpl: &[u8] = &[];
        let mut sm24: &[u8] = &[];
        let mut pdta: Option<Vec<([u8; 4], &[u8])>> = None;
        for (id, body) in chunks(&bytes[12..]) {
            if &id != b"LIST" || body.len() < 4 {
                continue;
            }
            match &body[..4] {
                b"sdta" => {
                    for (cid, cbody) in chunks(&body[4..]) {
                        match &cid {
                            b"smpl" => smpl = cbody,
                            b"sm24" => sm24 = cbody,
                            _ => {}
                        }
                    }
                }
                b"pdta" => pdta = Some(chunks(&body[4..])),
                _ => {}
            }
        }
        let pdta = pdta.ok_or_else(|| fail("no pdta list"))?;
        let get = |id: &[u8; 4]| -> Result<&[u8]> {
            pdta.iter().find(|(i, _)| i == id).map(|(_, b)| *b).ok_or_else(|| fail("a pdta chunk is missing"))
        };
        let presets: Vec<PresetHeader> = records(get(b"phdr")?, 38)
            .map(|c| PresetHeader { name: c.name(0), program: c.u16(20), bank: c.u16(22), bag: usize::from(c.u16(24)) })
            .collect();
        let bags = |id: &[u8; 4]| -> Result<Vec<Bag>> {
            Ok(records(get(id)?, 4)
                .map(|c| Bag { first_gen: usize::from(c.u16(0)), modulator: usize::from(c.u16(2)) })
                .collect())
        };
        let mods = |id: &[u8; 4]| -> Result<Vec<Modulator>> {
            Ok(records(get(id)?, 10)
                .map(|c| Modulator {
                    src: c.u16(0),
                    dest: c.u16(2),
                    amount: i32::from(c.u16(4) as i16),
                    amt_src: c.u16(6),
                    transform: c.u16(8),
                })
                .collect())
        };
        let gens = |id: &[u8; 4]| -> Result<Vec<Gen>> {
            Ok(records(get(id)?, 4).map(|c| Gen { oper: c.u16(0), amount: Amount(c.u16(2)) }).collect())
        };
        let insts: Vec<InstHeader> =
            records(get(b"inst")?, 22).map(|c| InstHeader { bag: usize::from(c.u16(20)) }).collect();
        let samples: Vec<SampleHeader> = records(get(b"shdr")?, 46)
            .map(|c| SampleHeader {
                start: c.u32(20),
                end: c.u32(24),
                start_loop: c.u32(28),
                end_loop: c.u32(32),
                rate: c.u32(36),
                pitch: c.bytes[40],
                correction: c.bytes[41] as i8,
                kind: c.u16(44),
            })
            .collect();
        let font = Self {
            presets,
            pbag: bags(b"pbag")?,
            pmod: mods(b"pmod")?,
            pgen: gens(b"pgen")?,
            insts,
            ibag: bags(b"ibag")?,
            imod: mods(b"imod")?,
            igen: gens(b"igen")?,
            samples,
            smpl,
            sm24,
            smpl_at: crate::stream::offset_in(bytes, smpl),
            stream: None,
        };
        if font.presets.len() < 2 || font.insts.len() < 2 {
            return Err(fail("no presets"));
        }
        font.validate_indices()?;
        Ok(font)
    }

    /// The presets (the terminal record left out), in file order
    pub fn presets(&self) -> Vec<PresetInfo> {
        self.presets[..self.presets.len() - 1]
            .iter()
            .map(|p| PresetInfo { bank: p.bank, program: p.program, name: p.name.clone() })
            .collect()
    }

    /// The bag, generator and modulator indices must never decrease, or the slices built from them would be
    /// reversed. Checked once here so the import can trust them.
    fn validate_indices(&self) -> Result<()> {
        fn rising(it: impl Iterator<Item = usize>) -> bool {
            let mut last = 0;
            it.into_iter().all(|v| {
                let ok = v >= last;
                last = v;
                ok
            })
        }
        let ok = rising(self.presets.iter().map(|p| p.bag))
            && rising(self.insts.iter().map(|i| i.bag))
            && rising(self.pbag.iter().map(|b| b.first_gen))
            && rising(self.pbag.iter().map(|b| b.modulator))
            && rising(self.ibag.iter().map(|b| b.first_gen))
            && rising(self.ibag.iter().map(|b| b.modulator));
        if ok { Ok(()) } else { Err(fail("bag indices that go backwards")) }
    }

    fn bag_range(&self, bags: &[Bag], from: usize, to: usize) -> Vec<(usize, usize, usize, usize)> {
        // (gen start, gen end, mod start, mod end) of each bag in from..to
        (from..to.min(bags.len().saturating_sub(1)))
            .map(|b| (bags[b].first_gen, bags[b + 1].first_gen, bags[b].modulator, bags[b + 1].modulator))
            .collect()
    }

    /// The zones of preset `bank`/`program`, with every sample it uses decoded. Keys: `sampler.sf2.no_preset`
    pub fn import(&self, bank: u16, program: u16, sf3: Option<&mut dyn Sf3Decoder>) -> Result<Instrument> {
        let index = self
            .presets
            .iter()
            .take(self.presets.len() - 1)
            .position(|p| p.bank == bank && p.program == program)
            .ok_or_else(|| {
                Error::new("sampler.sf2.no_preset").arg("bank", i64::from(bank)).arg("program", i64::from(program))
            })?;
        self.import_index(index, sf3)
    }

    /// The first preset of the file
    pub fn import_first(&self, sf3: Option<&mut dyn Sf3Decoder>) -> Result<Instrument> {
        if self.presets.len() < 2 {
            return Err(fail("no presets"));
        }
        self.import_index(0, sf3)
    }

    fn import_index(&self, index: usize, mut sf3: Option<&mut dyn Sf3Decoder>) -> Result<Instrument> {
        let preset = &self.presets[index];
        let next_bag = self.presets[index + 1].bag;
        let mut out = Instrument::new(preset.name.clone());
        let mut cache: HashMap<usize, Arc<Pcm>> = HashMap::new();
        let pzones = self.bag_range(&self.pbag, preset.bag, next_bag);
        let mut p_global: Option<Vec<Gen>> = None;
        for (pi, &(g0, g1, m0, m1)) in pzones.iter().enumerate() {
            let pgens = &self.pgen[g0.min(self.pgen.len())..g1.min(self.pgen.len())];
            let is_global = !pgens.iter().any(|g| g.oper == g::INSTRUMENT);
            if is_global {
                if pi == 0 {
                    p_global = Some(pgens.to_vec());
                }
                continue;
            }
            let Some(inst_idx) = pgens.iter().find(|g| g.oper == g::INSTRUMENT).map(|g| usize::from(g.amount.0)) else {
                continue;
            };
            if inst_idx + 1 >= self.insts.len() {
                continue;
            }
            // The preset-level generators: the global zone's, overridden by the zone's own
            let mut preset_level: [Option<Amount>; g::COUNT] = [None; g::COUNT];
            for g in p_global.iter().flatten().chain(pgens.iter()) {
                if usize::from(g.oper) < g::COUNT {
                    preset_level[usize::from(g.oper)] = Some(g.amount);
                }
            }
            let preset_mods: Vec<Modulator> = self.pmod[m0.min(self.pmod.len())..m1.min(self.pmod.len())].to_vec();
            self.instrument_zones(inst_idx, &preset_level, &preset_mods, &mut cache, &mut sf3, &mut out)?;
        }
        if out.zones.is_empty() {
            return Err(Error::new("sampler.sf2.empty").arg("preset", preset.name.clone()));
        }
        out.validate()?;
        Ok(out)
    }

    fn instrument_zones(
        &self,
        inst_idx: usize,
        preset_level: &[Option<Amount>; g::COUNT],
        preset_mods: &[Modulator],
        cache: &mut HashMap<usize, Arc<Pcm>>,
        sf3: &mut Option<&mut dyn Sf3Decoder>,
        out: &mut Instrument,
    ) -> Result<()> {
        let zones = self.bag_range(&self.ibag, self.insts[inst_idx].bag, self.insts[inst_idx + 1].bag);
        let mut global: Option<(Vec<Gen>, Vec<Modulator>)> = None;
        for (zi, &(g0, g1, m0, m1)) in zones.iter().enumerate() {
            let gens = &self.igen[g0.min(self.igen.len())..g1.min(self.igen.len())];
            let mods = &self.imod[m0.min(self.imod.len())..m1.min(self.imod.len())];
            let Some(sample_id) = gens.iter().find(|g| g.oper == g::SAMPLE_ID).map(|g| usize::from(g.amount.0)) else {
                if zi == 0 {
                    global = Some((gens.to_vec(), mods.to_vec()));
                }
                continue;
            };
            if sample_id >= self.samples.len().saturating_sub(1) {
                continue;
            }
            let header = &self.samples[sample_id];
            // Monaural, left and right (right and left samples are played as they are: the zones pair them by pan)
            if header.kind & 0x8000 != 0 {
                return Err(fail("a ROM sample"));
            }
            let mut values = Values::defaults();
            if let Some((g, _)) = &global {
                values.apply(g);
            }
            values.apply(gens);
            // Instrument ranges, then the preset's: the intersection
            let key_i = values.range(g::KEY_RANGE, (0, 127));
            let vel_i = values.range(g::VEL_RANGE, (0, 127));
            let key_p = preset_level[usize::from(g::KEY_RANGE)].map_or((0, 127), Amount::range);
            let vel_p = preset_level[usize::from(g::VEL_RANGE)].map_or((0, 127), Amount::range);
            let keys = (key_i.0.max(key_p.0), key_i.1.min(key_p.1));
            let vels = (vel_i.0.max(vel_p.0), vel_i.1.min(vel_p.1));
            if keys.0 > keys.1 || vels.0 > vels.1 {
                continue;
            }
            // The preset's generators add to the instrument's (not the ranges, the sample, the instrument, the loop
            // mode, the exclusive class, the root key nor the sample addresses)
            for (oper, v) in preset_level.iter().enumerate() {
                let Some(v) = v else { continue };
                let oper = oper as u16;
                let additive = !matches!(
                    oper,
                    g::KEY_RANGE
                        | g::VEL_RANGE
                        | g::INSTRUMENT
                        | g::SAMPLE_ID
                        | g::SAMPLE_MODES
                        | g::EXCLUSIVE_CLASS
                        | g::OVERRIDING_ROOT_KEY
                        | g::START_ADDRS_OFFSET
                        | g::END_ADDRS_OFFSET
                        | g::STARTLOOP_ADDRS_OFFSET
                        | g::ENDLOOP_ADDRS_OFFSET
                        | g::START_ADDRS_COARSE
                        | g::END_ADDRS_COARSE
                        | g::STARTLOOP_ADDRS_COARSE
                        | g::ENDLOOP_ADDRS_COARSE
                );
                if additive {
                    values.v[usize::from(oper)] += v.i16();
                }
            }
            let pcm = match cache.get(&sample_id) {
                Some(p) => Arc::clone(p),
                None => {
                    let p = Arc::new(self.decode(header, sf3)?);
                    cache.insert(sample_id, Arc::clone(&p));
                    p
                }
            };
            out.zones.push(build_zone(
                &values,
                header,
                pcm,
                keys,
                vels,
                mods,
                global.as_ref().map(|g| g.1.as_slice()),
                preset_mods,
            ));
        }
        Ok(())
    }

    /// A sample's frames from the data chunks
    fn decode(&self, h: &SampleHeader, sf3: &mut Option<&mut dyn Sf3Decoder>) -> Result<Pcm> {
        if h.end <= h.start || h.rate == 0 {
            return Err(fail("a sample header"));
        }
        let rate = h.rate;
        if h.kind & 0x10 != 0 {
            // Ogg Vorbis (SF3): start and end are byte offsets
            let bytes =
                self.smpl.get(h.start as usize..h.end as usize).ok_or_else(|| fail("a sample past the data"))?;
            let decoder = sf3.as_deref_mut().ok_or_else(|| Error::new("sampler.sf2.compressed"))?;
            let frames = decoder.decode(bytes, rate)?;
            if frames.is_empty() {
                return Err(fail("an empty sample"));
            }
            return Ok(Pcm::mono(rate, frames));
        }
        let (a, b) = (h.start as usize, h.end as usize);
        let data = self.smpl.get(a * 2..b * 2).ok_or_else(|| fail("a sample past the data"))?;
        let low = self.sm24.get(a..b);
        if let (Some(plan), Some(at), None) = (&self.stream, self.smpl_at, low)
            && b - a > plan.min_frames
        {
            let head: Vec<f32> =
                data.as_chunks::<2>().0.iter().take(plan.head).map(|p| f32::from(i16::from_le_bytes(*p)) / 32768.0).collect();
            let tail = FileTail::new(Arc::clone(&plan.file), (at + a * 2) as u64, b - a, 1, Raw::I16);
            return Ok(Pcm::streamed(Pcm::mono(rate, head), Arc::new(tail)));
        }
        let frames: Vec<f32> = data
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let v = i32::from(i16::from_le_bytes(*p));
                match low {
                    Some(l) if l.len() > i => (v * 256 + i32::from(l[i])) as f32 / 8_388_608.0,
                    _ => v as f32 / 32768.0,
                }
            })
            .collect();
        Ok(Pcm::mono(rate, frames))
    }
}

/// The generator values of a zone being built (the default of each, then the changes)
struct Values {
    v: [i32; g::COUNT],
    /// The key and velocity ranges, if set
    ranges: [Option<(u8, u8)>; 2],
}

impl Values {
    fn defaults() -> Self {
        let mut v = [0i32; g::COUNT];
        v[usize::from(g::INITIAL_FILTER_FC)] = 13_500;
        for g in [
            g::DELAY_MOD_ENV,
            g::ATTACK_MOD_ENV,
            g::HOLD_MOD_ENV,
            g::DECAY_MOD_ENV,
            g::RELEASE_MOD_ENV,
            g::DELAY_VOL_ENV,
            g::ATTACK_VOL_ENV,
            g::HOLD_VOL_ENV,
            g::DECAY_VOL_ENV,
            g::RELEASE_VOL_ENV,
        ] {
            v[usize::from(g)] = -12_000;
        }
        v[usize::from(g::SCALE_TUNING)] = 100;
        v[usize::from(g::OVERRIDING_ROOT_KEY)] = -1;
        Self { v, ranges: [None; 2] }
    }

    fn apply(&mut self, gens: &[Gen]) {
        for g in gens {
            let i = usize::from(g.oper);
            if i < g::COUNT && !matches!(g.oper, g::KEY_RANGE | g::VEL_RANGE) {
                self.v[i] = g.amount.i16();
            }
        }
        for g in gens {
            if matches!(g.oper, g::KEY_RANGE | g::VEL_RANGE) {
                self.ranges[usize::from(g.oper == g::VEL_RANGE)] = Some(g.amount.range());
            }
        }
    }

    fn range(&self, which: u16, default: (u8, u8)) -> (u8, u8) {
        self.ranges[usize::from(which == g::VEL_RANGE)].unwrap_or(default)
    }

    fn get(&self, g: u16) -> i32 {
        self.v[usize::from(g)]
    }
}

fn timecents(tc: i32) -> f32 {
    if tc <= -12_000 { 0.0 } else { libm::exp2f(tc as f32 / 1200.0) }
}

/// Centibels of attenuation to a linear level
fn from_centibels(cb: i32) -> f32 {
    libm::powf(10.0, -(cb as f32) / 200.0)
}

#[allow(clippy::too_many_arguments)]
fn build_zone(
    v: &Values,
    h: &SampleHeader,
    pcm: Arc<Pcm>,
    keys: (u8, u8),
    vels: (u8, u8),
    inst_mods: &[Modulator],
    global_mods: Option<&[Modulator]>,
    preset_mods: &[Modulator],
) -> Zone {
    let mut z = Zone::new(pcm);
    z.keys = keys;
    z.vels = (vels.0.max(1), vels.1.max(vels.0.max(1)));
    let frames = z.sample.frames() as i64;
    let offset = |fine: u16, coarse: u16| i64::from(v.get(fine)) + 32_768 * i64::from(v.get(coarse));
    let start = offset(g::START_ADDRS_OFFSET, g::START_ADDRS_COARSE).clamp(0, frames - 1);
    let end = (frames + offset(g::END_ADDRS_OFFSET, g::END_ADDRS_COARSE)).clamp(start + 1, frames);
    z.start = start as u32;
    z.end = end as u32;
    // The loop points are relative to the sample's start in the header
    let loop_start =
        i64::from(h.start_loop) - i64::from(h.start) + offset(g::STARTLOOP_ADDRS_OFFSET, g::STARTLOOP_ADDRS_COARSE);
    let loop_end =
        i64::from(h.end_loop) - i64::from(h.start) + offset(g::ENDLOOP_ADDRS_OFFSET, g::ENDLOOP_ADDRS_COARSE);
    z.loop_start = loop_start.clamp(0, frames) as u32;
    z.loop_end = loop_end.clamp(0, frames) as u32;
    z.loop_mode = match v.get(g::SAMPLE_MODES) & 3 {
        1 => LoopMode::Forward,
        3 => LoopMode::UntilRelease,
        _ => LoopMode::Off,
    };
    if z.loop_mode.is_looping() && z.loop_end <= z.loop_start + 1 {
        z.loop_mode = LoopMode::Off;
    }
    let root = v.get(g::OVERRIDING_ROOT_KEY);
    z.root_key = if (0..=127).contains(&root) {
        root as u8
    } else if h.pitch <= 127 {
        h.pitch
    } else {
        60
    };
    z.tune = (100 * v.get(g::COARSE_TUNE) + v.get(g::FINE_TUNE) + i32::from(h.correction)) as f32;
    z.key_track = v.get(g::SCALE_TUNING) as f32;
    z.gain = -(v.get(g::INITIAL_ATTENUATION).max(0) as f32) / 10.0;
    z.pan = (v.get(g::PAN) as f32 / 500.0).clamp(-1.0, 1.0);
    z.amp = Ahdsr {
        delay: timecents(v.get(g::DELAY_VOL_ENV)),
        attack: timecents(v.get(g::ATTACK_VOL_ENV)),
        hold: timecents(v.get(g::HOLD_VOL_ENV)),
        decay: timecents(v.get(g::DECAY_VOL_ENV)),
        sustain: from_centibels(v.get(g::SUSTAIN_VOL_ENV).clamp(0, 1440)),
        release: timecents(v.get(g::RELEASE_VOL_ENV)).max(0.002),
    };
    let fc = v.get(g::INITIAL_FILTER_FC);
    let mod_to_fc = v.get(g::MOD_ENV_TO_FILTER_FC);
    if fc < 13_500 || mod_to_fc != 0 {
        let q_cb = v.get(g::INITIAL_FILTER_Q).clamp(0, 960);
        z.filter = Some(Filter {
            kind: FilterKind::LowPass,
            cutoff: 8.175_798 * libm::exp2f(fc.min(13_500) as f32 / 1200.0),
            resonance: q_cb as f32 / 10.0,
            key_track: 0.0,
            vel_cents: 0.0,
            env: Ahdsr {
                delay: timecents(v.get(g::DELAY_MOD_ENV)),
                attack: timecents(v.get(g::ATTACK_MOD_ENV)),
                hold: timecents(v.get(g::HOLD_MOD_ENV)),
                decay: timecents(v.get(g::DECAY_MOD_ENV)),
                sustain: (1.0 - v.get(g::SUSTAIN_MOD_ENV).clamp(0, 1000) as f32 / 1000.0).clamp(0.0, 1.0),
                release: timecents(v.get(g::RELEASE_MOD_ENV)),
            },
            env_depth: mod_to_fc as f32,
        });
    }
    z.group = v.get(g::EXCLUSIVE_CLASS).max(0) as u32;
    z.off_by = z.group;
    z.off_fast = false;
    z.vel_curve = VelCurve::Concave;
    apply_velocity_modulators(&mut z, inst_mods, global_mods, preset_mods);
    z
}

/// The velocity modulators. The defaults of the specification (velocity to attenuation: 960 cB, concave, negative
/// unipolar; velocity to filter cutoff: −2400 cents, linear-in-velocity concave) hold unless the file's own modulator
/// of the same source and destination replaces them: its amount sets how much of each applies
fn apply_velocity_modulators(z: &mut Zone, inst: &[Modulator], global: Option<&[Modulator]>, preset: &[Modulator]) {
    // Source 0x0502: note-on velocity (index 2), concave (type 1), negative (bit 9), unipolar: direction bits 0x0502
    const VEL_CONCAVE_NEG: u16 = 0x0502;
    const VEL_LINEAR_NEG: u16 = 0x0102;
    let mut attenuation = 960.0f32;
    let mut cutoff = -2400.0f32;
    for m in global.into_iter().flatten().chain(inst.iter()).chain(preset.iter()) {
        if m.amt_src != 0 || m.transform != 0 {
            continue;
        }
        if matches!(m.src, VEL_CONCAVE_NEG | VEL_LINEAR_NEG) {
            match m.dest {
                g::INITIAL_ATTENUATION => attenuation = m.amount as f32,
                g::INITIAL_FILTER_FC => cutoff = m.amount as f32,
                _ => {}
            }
        }
    }
    z.vel_track = (attenuation / 960.0).clamp(0.0, 1.0);
    if let Some(f) = &mut z.filter {
        f.vel_cents = cutoff;
    }
}

/// Reads the presets of a SoundFont's bytes
pub fn list_presets(bytes: &[u8]) -> Result<Vec<PresetInfo>> {
    Ok(SoundFont::parse(bytes)?.presets())
}

/// Reads one preset of a SoundFont's bytes, its long 16-bit samples streaming from `plan`'s file
pub fn import_streaming(
    bytes: &[u8],
    bank: u16,
    program: u16,
    sf3: Option<&mut dyn Sf3Decoder>,
    plan: StreamPlan,
) -> Result<Instrument> {
    SoundFont::parse(bytes)?.streaming(plan).import(bank, program, sf3)
}

/// Reads one preset of a SoundFont's bytes
pub fn import(bytes: &[u8], bank: u16, program: u16, sf3: Option<&mut dyn Sf3Decoder>) -> Result<Instrument> {
    SoundFont::parse(bytes)?.import(bank, program, sf3)
}

#[cfg(test)]
mod tests;
