//! The YM3812 (OPL2) of adplug's `fmopl.c` 0.37a (Tatsuyuki Satoh, from MAME, LGPL-2.1 or later), ported: the nine
//! 2-operator FM channels, their envelopes, the tremolo and vibrato LFOs and the wave selector, as the legacy OpulenZ
//! compiled it (`OPL_TYPE_YM3812`, 16-bit output, through adplug's `CTemuopl`).
//!
//! The tables are the original's, made the same way in `double` (the shared ones once per process, the chip's own at
//! its rate). Pointers become indices. The arithmetic keeps the original's types: the phase counters and their
//! products unsigned and wrapping, the envelopes signed.
//!
//! Not ported, because OpulenZ never reaches them: the rhythm mode (register 0xBD's bit 5: OpulenZ writes only the
//! depth bits), the timers and their IRQ (no handler was set), the Y8950's ADPCM. One behaviour is fixed: the
//! original kept the tremolo and vibrato depth tables of the last chip it rendered in globals, refreshed only when it
//! rendered another chip, so a change of depth was heard only when some other chip played in between; here each
//! chip reads its own (decisoes-autonomas.md, "Chips e amplificadores").

use std::f64::consts::PI;
use std::sync::OnceLock;

const FREQ_BITS: u32 = 24;
const FREQ_RATE: u32 = 1 << (FREQ_BITS - 20);
const TL_BITS: u32 = FREQ_BITS + 2;
const OPL_OUTSB: u32 = TL_BITS + 3 - 16;
const OPL_MAXOUT: i32 = 0x7fff << OPL_OUTSB;
const OPL_MINOUT: i32 = -(0x8000 << OPL_OUTSB);
const SIN_ENT: usize = 2048;
const ENV_BITS: u32 = 16;
const EG_ENT: usize = 4096;
const EG_OFF: i32 = ((2 * EG_ENT) << ENV_BITS) as i32;
const EG_DED: i32 = EG_OFF;
const EG_DST: i32 = (EG_ENT << ENV_BITS) as i32;
const EG_AED: i32 = EG_DST;
const EG_AST: i32 = 0;
const EG_STEP: f64 = 96.0 / EG_ENT as f64;
const VIB_ENT: usize = 512;
const VIB_SHIFT: u32 = 32 - 9;
const AMS_ENT: usize = 512;
const AMS_SHIFT: u32 = 32 - 9;
const VIB_RATE: u32 = 256;
const TL_MAX: usize = EG_ENT * 2;
const OPL_ARRATE: f64 = 141_280.0;
const OPL_DRRATE: f64 = 1_956_000.0;

const ENV_MOD_RR: u8 = 0;
const ENV_MOD_DR: u8 = 1;
const ENV_MOD_AR: u8 = 2;

pub const CHANNELS: usize = 9;

const SLOT_ARRAY: [i32; 32] = [
    0, 2, 4, 1, 3, 5, -1, -1, 6, 8, 10, 7, 9, 11, -1, -1, 12, 14, 16, 13, 15, 17, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1,
];

/// The key scale levels, dB (3 dB/oct), in TL steps at 6 dB/oct
const KSL_DB: [f64; 8 * 16] = [
    0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000,
    0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.000, 0.750, 1.125, 1.500, 1.875, 2.250, 2.625, 3.000,
    0.000, 0.000, 0.000, 0.000, 0.000, 1.125, 1.875, 2.625, 3.000, 3.750, 4.125, 4.500, 4.875, 5.250, 5.625, 6.000,
    0.000, 0.000, 0.000, 1.875, 3.000, 4.125, 4.875, 5.625, 6.000, 6.750, 7.125, 7.500, 7.875, 8.250, 8.625, 9.000,
    0.000, 0.000, 3.000, 4.875, 6.000, 7.125, 7.875, 8.625, 9.000, 9.750, 10.125, 10.500, 10.875, 11.250, 11.625,
    12.000, 0.000, 3.000, 6.000, 7.875, 9.000, 10.125, 10.875, 11.625, 12.000, 12.750, 13.125, 13.500, 13.875, 14.250,
    14.625, 15.000, 0.000, 6.000, 9.000, 10.875, 12.000, 13.125, 13.875, 14.625, 15.000, 15.750, 16.125, 16.500,
    16.875, 17.250, 17.625, 18.000, 0.000, 9.000, 12.000, 13.875, 15.000, 16.125, 16.875, 17.625, 18.000, 18.750,
    19.125, 19.500, 19.875, 20.250, 20.625, 21.000,
];

const MUL_TABLE: [u32; 16] = [1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 20, 24, 24, 30, 30];

fn sl_table(sl: usize) -> i32 {
    let db = if sl == 15 { 31.0 } else { sl as f64 };
    (db * ((3.0 / EG_STEP) * f64::from(1u32 << ENV_BITS)) + f64::from(EG_DST)) as i32
}

/// The tables every chip shares (`OPLOpenTable`)
#[derive(Debug)]
struct Tables {
    tl: Vec<i32>,
    /// Indices into `tl`, four waves of `SIN_ENT`
    sin: Vec<usize>,
    ams: Vec<i32>,
    vib: Vec<i32>,
    env_curve: Vec<i32>,
    ksl: [u32; 8 * 16],
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut tl = vec![0i32; TL_MAX * 2];
        for t in 0..EG_ENT - 1 {
            let rate = f64::from((1u32 << TL_BITS) - 1) / libm::pow(10.0, EG_STEP * t as f64 / 20.0);
            tl[t] = rate as i32;
            tl[TL_MAX + t] = -tl[t];
        }
        let mut sin = vec![0usize; SIN_ENT * 4];
        sin[0] = EG_ENT - 1;
        sin[SIN_ENT / 2] = EG_ENT - 1;
        for s in 1..=SIN_ENT / 4 {
            let mut pom = libm::sin(2.0 * PI * s as f64 / SIN_ENT as f64);
            pom = 20.0 * libm::log10(1.0 / pom);
            let j = (pom / EG_STEP) as i32 as usize;
            sin[s] = j;
            sin[SIN_ENT / 2 - s] = j;
            sin[SIN_ENT / 2 + s] = TL_MAX + j;
            sin[SIN_ENT - s] = TL_MAX + j;
        }
        for s in 0..SIN_ENT {
            sin[SIN_ENT + s] = if s < SIN_ENT / 2 { sin[s] } else { EG_ENT };
            sin[SIN_ENT * 2 + s] = sin[s % (SIN_ENT / 2)];
            sin[SIN_ENT * 3 + s] = if (s / (SIN_ENT / 4)) & 1 != 0 { EG_ENT } else { sin[SIN_ENT * 2 + s] };
        }
        let mut env_curve = vec![0i32; 2 * EG_ENT + 1];
        for i in 0..EG_ENT {
            let pom = libm::pow((EG_ENT - 1 - i) as f64 / EG_ENT as f64, 8.0) * EG_ENT as f64;
            env_curve[i] = pom as i32;
            env_curve[(EG_DST >> ENV_BITS) as usize + i] = i as i32;
        }
        env_curve[(EG_OFF >> ENV_BITS) as usize] = EG_ENT as i32 - 1;
        let mut ams = vec![0i32; AMS_ENT * 2];
        for i in 0..AMS_ENT {
            let pom = (1.0 + libm::sin(2.0 * PI * i as f64 / AMS_ENT as f64)) / 2.0;
            ams[i] = ((1.0 / EG_STEP) * pom) as i32;
            ams[AMS_ENT + i] = ((4.8 / EG_STEP) * pom) as i32;
        }
        let mut vib = vec![0i32; VIB_ENT * 2];
        for i in 0..VIB_ENT {
            let pom = f64::from(VIB_RATE) * 0.06 * libm::sin(2.0 * PI * i as f64 / VIB_ENT as f64);
            vib[i] = (f64::from(VIB_RATE) + pom * 0.07) as i32;
            vib[VIB_ENT + i] = (f64::from(VIB_RATE) + pom * 0.14) as i32;
        }
        let dv = EG_STEP / 2.0;
        let ksl = std::array::from_fn(|i| (KSL_DB[i] / dv) as u32);
        Tables { tl, sin, ams, vib, env_curve, ksl }
    })
}

/// Where operator 1's output goes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Connect {
    /// Into operator 2's phase (FM)
    #[default]
    Feedback2,
    /// To the output (additive)
    Carrier,
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    tl: i32,
    tll: i32,
    ksr_shift: u8,
    /// Index into the chip's attack table, none for rate 0
    ar: Option<usize>,
    dr: Option<usize>,
    sl: i32,
    rr: usize,
    ksl: u8,
    ksr: u8,
    mul: u32,
    cnt: u32,
    incr: u32,
    eg_typ: u8,
    evm: u8,
    evc: i32,
    eve: i32,
    evs: i32,
    evsa: i32,
    evsd: i32,
    evsr: i32,
    ams: u8,
    vib: u8,
    /// Index of its wave in the sine tables
    wavetable: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    slot: [Slot; 2],
    con: u8,
    fb: u8,
    connect1: Connect,
    op1_out: [i32; 2],
    block_fnum: u32,
    kcode: u8,
    fc: u32,
    ksl_base: u32,
    keyon: u8,
}

/// The rate tables are indexed by a rate (at most 15 << 2 = 60) plus the key scale (a key code of at most 15)
const RATE_TABLE: usize = 60 + 15 + 1;

/// One YM3812
#[derive(Clone, Debug)]
pub struct Opl {
    rate: i32,
    clock: i32,
    freqbase: f64,
    mode: u32,
    ch: [Channel; CHANNELS],
    rythm: u8,
    ar_table: [i32; RATE_TABLE],
    dr_table: [i32; RATE_TABLE],
    fn_table: [u32; 1024],
    ams_table: usize,
    vib_table: usize,
    ams_cnt: u32,
    ams_incr: i32,
    vib_cnt: u32,
    vib_incr: i32,
    wavesel: u8,
}

impl Opl {
    /// `OPLCreate(OPL_TYPE_YM3812, clock, rate)`
    pub fn new(clock: i32, rate: i32) -> Self {
        let _ = tables();
        let mut opl = Self {
            rate,
            clock,
            freqbase: 0.0,
            mode: 0,
            ch: [Channel::default(); CHANNELS],
            rythm: 0,
            ar_table: [0; RATE_TABLE],
            dr_table: [0; RATE_TABLE],
            fn_table: [0; 1024],
            ams_table: 0,
            vib_table: 0,
            ams_cnt: 0,
            ams_incr: 0,
            vib_cnt: 0,
            vib_incr: 0,
            wavesel: 0,
        };
        opl.initialize();
        opl.reset();
        opl
    }

    fn initialize(&mut self) {
        self.freqbase = if self.rate != 0 { (f64::from(self.clock) / f64::from(self.rate)) / 72.0 } else { 0.0 };
        // init_timetables
        for i in 0..4 {
            self.ar_table[i] = 0;
            self.dr_table[i] = 0;
        }
        for i in 4..=60usize {
            let mut rate = self.freqbase;
            if i < 60 {
                rate *= 1.0 + (i & 3) as f64 * 0.25;
            }
            rate *= f64::from(1u32 << ((i >> 2) - 1));
            rate *= f64::from((EG_ENT << ENV_BITS) as u32);
            self.ar_table[i] = (rate / OPL_ARRATE) as i32;
            self.dr_table[i] = (rate / OPL_DRRATE) as i32;
        }
        for i in 60..RATE_TABLE {
            self.ar_table[i] = EG_AED - 1;
            self.dr_table[i] = self.dr_table[60];
        }
        for fn_ in 0..1024u32 {
            self.fn_table[fn_ as usize] =
                (self.freqbase * f64::from(fn_) * f64::from(FREQ_RATE) * f64::from(1u32 << 7) / 2.0) as u32;
        }
        let clock = f64::from(self.clock) / 3_600_000.0;
        self.ams_incr = if self.rate != 0 {
            (AMS_ENT as f64 * f64::from(1u32 << AMS_SHIFT) / f64::from(self.rate) * 3.7 * clock) as i32
        } else {
            0
        };
        self.vib_incr = if self.rate != 0 {
            (VIB_ENT as f64 * f64::from(1u32 << VIB_SHIFT) / f64::from(self.rate) * 6.4 * clock) as i32
        } else {
            0
        };
    }

    /// `OPLResetChip`
    pub fn reset(&mut self) {
        self.mode = 0;
        self.write_reg(0x01, 0);
        self.write_reg(0x02, 0);
        self.write_reg(0x03, 0);
        self.write_reg(0x04, 0);
        for i in (0x20..=0xff).rev() {
            self.write_reg(i, 0);
        }
        // the LFOs (tremolo, vibrato) from their start: a reset chip plays the same each time
        self.ams_cnt = 0;
        self.vib_cnt = 0;
        for ch in &mut self.ch {
            ch.op1_out = [0, 0];
            for s in &mut ch.slot {
                s.wavetable = 0;
                s.evc = EG_OFF;
                s.eve = EG_OFF + 1;
                s.evs = 0;
                s.cnt = 0;
            }
        }
    }

    /// `CTemuopl::write`: the address, then the data
    pub fn write(&mut self, reg: i32, val: i32) {
        self.write_reg(reg & 0xff, val & 0xff);
    }

    fn ar_at(&self, ar: Option<usize>, ksr: u8) -> i32 {
        ar.map_or(0, |a| self.ar_table[a + usize::from(ksr)])
    }

    fn dr_at(&self, dr: Option<usize>, ksr: u8) -> i32 {
        dr.map_or(0, |d| self.dr_table[d + usize::from(ksr)])
    }

    /// `CALC_FCSLOT`
    fn calc_fcslot(&mut self, c: usize, s: usize) {
        let ch = self.ch[c];
        let mut slot = ch.slot[s];
        slot.incr = ch.fc.wrapping_mul(slot.mul);
        let ksr = ch.kcode >> slot.ksr_shift;
        if slot.ksr != ksr {
            slot.ksr = ksr;
            slot.evsa = self.ar_at(slot.ar, ksr);
            slot.evsd = self.dr_at(slot.dr, ksr);
            slot.evsr = self.dr_table[slot.rr + usize::from(ksr)];
        }
        slot.tll = slot.tl.wrapping_add((ch.ksl_base >> slot.ksl) as i32);
        self.ch[c].slot[s] = slot;
    }

    fn write_reg(&mut self, r: i32, v: i32) {
        let t = tables();
        match r & 0xe0 {
            0x00 => {
                if r & 0x1f == 0x01 {
                    // wave selector enable (the YM3812 has it)
                    self.wavesel = (v & 0x20) as u8;
                    if self.wavesel == 0 {
                        for ch in &mut self.ch {
                            ch.slot[0].wavetable = 0;
                            ch.slot[1].wavetable = 0;
                        }
                    }
                }
                // (timers and IRQ: no handler, no effect on the sound)
            }
            0x20 => {
                let Ok(slot) = usize::try_from(SLOT_ARRAY[(r & 0x1f) as usize]) else { return };
                let (c, s) = (slot / 2, slot & 1);
                let sl = &mut self.ch[c].slot[s];
                sl.mul = MUL_TABLE[(v & 0x0f) as usize];
                sl.ksr_shift = if v & 0x10 != 0 { 0 } else { 2 };
                sl.eg_typ = ((v & 0x20) >> 5) as u8;
                sl.vib = (v & 0x40) as u8;
                sl.ams = (v & 0x80) as u8;
                self.calc_fcslot(c, s);
            }
            0x40 => {
                let Ok(slot) = usize::try_from(SLOT_ARRAY[(r & 0x1f) as usize]) else { return };
                let (c, s) = (slot / 2, slot & 1);
                let ksl_base = self.ch[c].ksl_base;
                let sl = &mut self.ch[c].slot[s];
                let ksl = v >> 6;
                sl.ksl = if ksl != 0 { (3 - ksl) as u8 } else { 31 };
                sl.tl = (f64::from(v & 0x3f) * (0.75 / EG_STEP)) as i32;
                if self.mode & 0x80 == 0 {
                    sl.tll = sl.tl.wrapping_add((ksl_base >> sl.ksl) as i32);
                }
            }
            0x60 => {
                let Ok(slot) = usize::try_from(SLOT_ARRAY[(r & 0x1f) as usize]) else { return };
                let (c, s) = (slot / 2, slot & 1);
                let ar = (v >> 4) as usize;
                let dr = (v & 0x0f) as usize;
                let mut sl = self.ch[c].slot[s];
                sl.ar = (ar != 0).then_some(ar << 2);
                sl.evsa = self.ar_at(sl.ar, sl.ksr);
                if sl.evm == ENV_MOD_AR {
                    sl.evs = sl.evsa;
                }
                sl.dr = (dr != 0).then_some(dr << 2);
                sl.evsd = self.dr_at(sl.dr, sl.ksr);
                if sl.evm == ENV_MOD_DR {
                    sl.evs = sl.evsd;
                }
                self.ch[c].slot[s] = sl;
            }
            0x80 => {
                let Ok(slot) = usize::try_from(SLOT_ARRAY[(r & 0x1f) as usize]) else { return };
                let (c, s) = (slot / 2, slot & 1);
                let mut sl = self.ch[c].slot[s];
                sl.sl = sl_table((v >> 4) as usize);
                if sl.evm == ENV_MOD_DR {
                    sl.eve = sl.sl;
                }
                sl.rr = ((v & 0x0f) << 2) as usize;
                sl.evsr = self.dr_table[sl.rr + usize::from(sl.ksr)];
                if sl.evm == ENV_MOD_RR {
                    sl.evs = sl.evsr;
                }
                self.ch[c].slot[s] = sl;
            }
            0xa0 => {
                if r == 0xbd {
                    // depth of the tremolo and the vibrato (the rhythm bits: see the module's documentation)
                    self.ams_table = if v & 0x80 != 0 { AMS_ENT } else { 0 };
                    self.vib_table = if v & 0x40 != 0 { VIB_ENT } else { 0 };
                    self.rythm = (v & 0x3f) as u8;
                    return;
                }
                if r & 0x0f > 8 {
                    return;
                }
                let c = (r & 0x0f) as usize;
                let block_fnum;
                if r & 0x10 == 0 {
                    block_fnum = (self.ch[c].block_fnum & 0x1f00) | v as u32;
                } else {
                    let keyon = ((v >> 5) & 1) as u8;
                    block_fnum = (((v & 0x1f) as u32) << 8) | (self.ch[c].block_fnum & 0xff);
                    if self.ch[c].keyon != keyon {
                        self.ch[c].keyon = keyon;
                        if keyon != 0 {
                            self.ch[c].op1_out = [0, 0];
                            key_on(&mut self.ch[c].slot[0]);
                            key_on(&mut self.ch[c].slot[1]);
                        } else {
                            key_off(&mut self.ch[c].slot[0]);
                            key_off(&mut self.ch[c].slot[1]);
                        }
                    }
                }
                if self.ch[c].block_fnum != block_fnum {
                    let block_rv = 7 - (block_fnum >> 10);
                    let fnum = (block_fnum & 0x3ff) as usize;
                    let ch = &mut self.ch[c];
                    ch.block_fnum = block_fnum;
                    ch.ksl_base = t.ksl[(block_fnum >> 6) as usize];
                    ch.fc = self.fn_table[fnum] >> block_rv;
                    ch.kcode = (ch.block_fnum >> 9) as u8;
                    if self.mode & 0x40 != 0 && ch.block_fnum & 0x100 != 0 {
                        ch.kcode |= 1;
                    }
                    self.calc_fcslot(c, 0);
                    self.calc_fcslot(c, 1);
                }
            }
            0xc0 => {
                if r & 0x0f > 8 {
                    return;
                }
                let ch = &mut self.ch[(r & 0x0f) as usize];
                let feedback = (v >> 1) & 7;
                ch.fb = if feedback != 0 { (8 + 1 - feedback) as u8 } else { 0 };
                ch.con = (v & 1) as u8;
                ch.connect1 = if ch.con != 0 { Connect::Carrier } else { Connect::Feedback2 };
            }
            0xe0 => {
                let Ok(slot) = usize::try_from(SLOT_ARRAY[(r & 0x1f) as usize]) else { return };
                if self.wavesel != 0 {
                    self.ch[slot / 2].slot[slot & 1].wavetable = (v & 0x03) as usize * SIN_ENT;
                }
            }
            _ => {}
        }
    }

    /// `YM3812UpdateOne`: `buf.len()` samples
    pub fn update(&mut self, buf: &mut [i16]) {
        let t = tables();
        let mut ams_cnt = self.ams_cnt;
        let mut vib_cnt = self.vib_cnt;
        for out in buf.iter_mut() {
            ams_cnt = ams_cnt.wrapping_add(self.ams_incr as u32);
            vib_cnt = vib_cnt.wrapping_add(self.vib_incr as u32);
            let ams = t.ams[self.ams_table + (ams_cnt >> AMS_SHIFT) as usize];
            let vib = t.vib[self.vib_table + (vib_cnt >> VIB_SHIFT) as usize];
            let mut outd: i32 = 0;
            for ch in &mut self.ch {
                calc_ch(t, ch, ams, vib, &mut outd);
            }
            let data = outd.clamp(OPL_MINOUT, OPL_MAXOUT);
            *out = (data >> OPL_OUTSB) as i16;
        }
        self.ams_cnt = ams_cnt;
        self.vib_cnt = vib_cnt;
    }
}

fn key_on(slot: &mut Slot) {
    slot.cnt = 0;
    slot.evm = ENV_MOD_AR;
    slot.evs = slot.evsa;
    slot.evc = EG_AST;
    slot.eve = EG_AED;
}

fn key_off(slot: &mut Slot) {
    if slot.evm > ENV_MOD_RR {
        slot.evm = ENV_MOD_RR;
        if slot.evc & EG_DST == 0 {
            slot.evc = EG_DST;
        }
        slot.eve = EG_DED;
        slot.evs = slot.evsr;
    }
}

/// `OPL_CALC_SLOT`: the envelope output
fn calc_slot(t: &Tables, slot: &mut Slot, ams: i32) -> u32 {
    slot.evc = slot.evc.wrapping_add(slot.evs);
    if slot.evc >= slot.eve {
        match slot.evm {
            ENV_MOD_AR => {
                slot.evm = ENV_MOD_DR;
                slot.evc = EG_DST;
                slot.eve = slot.sl;
                slot.evs = slot.evsd;
            }
            ENV_MOD_DR => {
                slot.evc = slot.sl;
                slot.eve = EG_DED;
                if slot.eg_typ != 0 {
                    slot.evs = 0;
                } else {
                    slot.evm = ENV_MOD_RR;
                    slot.evs = slot.evsr;
                }
            }
            ENV_MOD_RR => {
                slot.evc = EG_OFF;
                slot.eve = EG_OFF + 1;
                slot.evs = 0;
            }
            _ => {}
        }
    }
    let am = if slot.ams != 0 { ams } else { 0 };
    slot.tll.wrapping_add(t.env_curve[(slot.evc >> ENV_BITS) as usize]).wrapping_add(am) as u32
}

/// `OP_OUT`
fn op_out(t: &Tables, slot: &Slot, env: u32, con: i32) -> i32 {
    let phase = (slot.cnt.wrapping_add(con as u32) / (0x0100_0000 / SIN_ENT as u32)) as usize & (SIN_ENT - 1);
    t.tl[t.sin[slot.wavetable + phase] + env as usize]
}

/// The phase step of a slot
fn advance(slot: &mut Slot, vib: i32) {
    if slot.vib != 0 {
        slot.cnt = slot.cnt.wrapping_add(slot.incr.wrapping_mul(vib as u32) / VIB_RATE);
    } else {
        slot.cnt = slot.cnt.wrapping_add(slot.incr);
    }
}

/// `OPL_CALC_CH`
fn calc_ch(t: &Tables, ch: &mut Channel, ams: i32, vib: i32, outd: &mut i32) {
    let mut feedback2: i32 = 0;
    let env_out = calc_slot(t, &mut ch.slot[0], ams);
    if env_out < EG_ENT as u32 - 1 {
        advance(&mut ch.slot[0], vib);
        let value = if ch.fb != 0 {
            let feedback1 = ch.op1_out[0].wrapping_add(ch.op1_out[1]) >> ch.fb;
            ch.op1_out[1] = ch.op1_out[0];
            ch.op1_out[0] = op_out(t, &ch.slot[0], env_out, feedback1);
            ch.op1_out[0]
        } else {
            op_out(t, &ch.slot[0], env_out, 0)
        };
        match ch.connect1 {
            Connect::Carrier => *outd = outd.wrapping_add(value),
            Connect::Feedback2 => feedback2 = feedback2.wrapping_add(value),
        }
    } else {
        ch.op1_out[1] = ch.op1_out[0];
        ch.op1_out[0] = 0;
    }
    let env_out = calc_slot(t, &mut ch.slot[1], ams);
    if env_out < EG_ENT as u32 - 1 {
        advance(&mut ch.slot[1], vib);
        *outd = outd.wrapping_add(op_out(t, &ch.slot[1], env_out, feedback2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_highest_rate_with_key_scaling_stays_inside_the_tables() {
        let mut opl = Opl::new(3_579_545, 44_100);
        // Operator 1 of channel 0: key scaling on, the fastest attack, decay and release
        opl.write(0x20, 0x10);
        opl.write(0x60, 0xff);
        opl.write(0x80, 0xff);
        // Block 7, the top fnum bits: key code 15
        opl.write(0xa0, 0xff);
        opl.write(0xb0, 0x3f);
        let mut buf = [0i16; 256];
        opl.update(&mut buf);
        opl.write(0xb0, 0x1f);
        opl.update(&mut buf);
    }
}
