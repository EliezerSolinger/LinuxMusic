//! Gb_Apu 0.4 (game-music-emu, by Shay Green, LGPL-2.1 or later), ported: the Game Boy's sound unit — two square
//! channels (the first with a frequency sweep), a 4-bit wave channel and a noise channel, their length counters and
//! envelopes clocked at 256 Hz by the frame sequencer — synthesized into a [`StereoBuffer`], as the legacy FreeBoy
//! compiled it (`Gb_Apu.cpp`, `Gb_Oscs.cpp`) and drove it (`GbApuWrapper`).
//!
//! The oscillators' output buffers are indices into the [`StereoBuffer`] (`None` silences one), and the APU owns its
//! buffer; nothing here allocates after [`GbApu::new`].

use crate::blip::{BlipEq, BlipSynth, CENTER, LEFT, RIGHT, StereoBuffer};

pub const START_ADDR: u32 = 0xFF10;
pub const END_ADDR: u32 = 0xFF3F;
pub const REGISTER_COUNT: usize = (END_ADDR - START_ADDR + 1) as usize;
const VOL_REG: u32 = 0xFF24;
const STATUS_REG: u32 = 0xFF26;
const OSC_COUNT: usize = 4;
const TRIGGER: u8 = 0x80;
const LEN_ENABLED_MASK: u8 = 0x40;

const POWERUP_REGS: [u8; 0x20] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // square 1
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // square 2
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // wave
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // noise
    0x00, // left/right enables
    0x77, // master volume
    0x80, // power
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// What every channel has (`Gb_Osc`, with `Gb_Env`'s envelope delay)
#[derive(Clone, Copy, Debug, Default)]
struct Osc {
    /// None, right, left, center (indices into the stereo buffer)
    outputs: [Option<usize>; 4],
    output: Option<usize>,
    output_select: usize,
    /// The first of its 5 registers
    regs: usize,
    delay: i32,
    last_amp: i32,
    volume: i32,
    length: i32,
    enabled: i32,
    env_delay: i32,
}

impl Osc {
    fn reset(&mut self) {
        self.delay = 0;
        self.last_amp = 0;
        self.length = 0;
        self.output_select = 3;
        self.output = self.outputs[self.output_select];
    }

    fn reg(&self, regs: &[u8; REGISTER_COUNT], i: usize) -> u8 {
        regs[self.regs + i]
    }

    fn frequency(&self, regs: &[u8; REGISTER_COUNT]) -> i32 {
        i32::from(self.reg(regs, 4) & 7) * 0x100 + i32::from(self.reg(regs, 3))
    }

    fn clock_length(&mut self, regs: &[u8; REGISTER_COUNT]) {
        if self.reg(regs, 4) & LEN_ENABLED_MASK != 0 && self.length != 0 {
            self.length -= 1;
        }
    }

    fn clock_envelope(&mut self, regs: &[u8; REGISTER_COUNT]) {
        if self.env_delay != 0 {
            self.env_delay -= 1;
            if self.env_delay == 0 {
                self.env_delay = i32::from(self.reg(regs, 2) & 7);
                let v = self.volume - 1 + i32::from(self.reg(regs, 2) >> 2 & 2);
                if (v as u32) < 15 {
                    self.volume = v;
                }
            }
        }
    }

    /// `Gb_Env::write_register`: whether it was triggered
    fn env_write(&mut self, regs: &[u8; REGISTER_COUNT], reg: usize, data: u8) -> bool {
        match reg {
            1 => self.length = 64 - i32::from(self.reg(regs, 1) & 0x3F),
            2 => {
                if data >> 4 == 0 {
                    self.enabled = 0;
                }
            }
            4 if data & TRIGGER != 0 => {
                self.env_delay = i32::from(self.reg(regs, 2) & 7);
                self.volume = i32::from(self.reg(regs, 2) >> 4);
                self.enabled = 1;
                if self.length == 0 {
                    self.length = 64;
                }
                return true;
            }
            _ => {}
        }
        false
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Square {
    osc: Osc,
    sweep_delay: i32,
    sweep_freq: i32,
    phase: i32,
}

const PERIOD_MASK: u8 = 0x70;
const SHIFT_MASK: u8 = 0x07;

impl Square {
    fn reset(&mut self) {
        self.phase = 0;
        self.sweep_freq = 0;
        self.sweep_delay = 0;
        self.osc.env_delay = 0;
        self.osc.reset();
    }

    fn clock_sweep(&mut self, regs: &mut [u8; REGISTER_COUNT]) {
        let r = self.osc.regs;
        let sweep_period = i32::from((regs[r] & PERIOD_MASK) >> 4);
        if sweep_period != 0 && self.sweep_delay != 0 {
            self.sweep_delay -= 1;
            if self.sweep_delay == 0 {
                self.sweep_delay = sweep_period;
                regs[r + 3] = (self.sweep_freq & 0xFF) as u8;
                regs[r + 4] = (regs[r + 4] & !0x07) | ((self.sweep_freq >> 8) & 0x07) as u8;
                let mut offset = self.sweep_freq >> (regs[r] & SHIFT_MASK);
                if regs[r] & 0x08 != 0 {
                    offset = -offset;
                }
                self.sweep_freq += offset;
                if self.sweep_freq < 0 {
                    self.sweep_freq = 0;
                } else if self.sweep_freq >= 2048 {
                    self.sweep_delay = 0;
                    self.sweep_freq = 2048;
                }
            }
        }
    }

    fn run(
        &mut self,
        regs: &[u8; REGISTER_COUNT],
        mut time: i32,
        end_time: i32,
        mut playing: i32,
        synth: &BlipSynth<12>,
        bufs: &mut StereoBuffer,
    ) {
        let Some(out) = self.osc.output else { return };
        if self.sweep_freq == 2048 {
            playing = 0;
        }
        const TABLE: [i32; 4] = [1, 2, 4, 6];
        let duty = TABLE[usize::from(self.osc.reg(regs, 1) >> 6)];
        let mut amp = self.osc.volume & playing;
        if self.phase >= duty {
            amp = -amp;
        }
        let frequency = self.osc.frequency(regs);
        if (frequency - 1) as u32 > 2040 {
            amp = self.osc.volume >> 1;
            playing = 0;
        }
        let delta = amp - self.osc.last_amp;
        if delta != 0 {
            self.osc.last_amp = amp;
            synth.offset(time, delta, &mut bufs.bufs[out]);
        }
        time += self.osc.delay;
        if playing == 0 {
            time = end_time;
        }
        if time < end_time {
            let period = (2048 - frequency) * 4;
            let buf = &mut bufs.bufs[out];
            let mut phase = self.phase;
            let mut delta = amp * 2;
            loop {
                phase = (phase + 1) & 7;
                if phase == 0 || phase == duty {
                    delta = -delta;
                    synth.offset(time, delta, buf);
                }
                time += period;
                if time >= end_time {
                    break;
                }
            }
            self.phase = phase;
            self.osc.last_amp = delta >> 1;
        }
        self.osc.delay = time - end_time;
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Noise {
    osc: Osc,
    bits: u32,
}

impl Noise {
    fn run(
        &mut self,
        regs: &[u8; REGISTER_COUNT],
        mut time: i32,
        end_time: i32,
        playing: i32,
        synth: &BlipSynth<8>,
        bufs: &mut StereoBuffer,
    ) {
        let Some(out) = self.osc.output else { return };
        let mut amp = self.osc.volume & playing;
        let tap = 13 - u32::from(self.osc.reg(regs, 3) & 8);
        if self.bits >> tap & 2 != 0 {
            amp = -amp;
        }
        let delta = amp - self.osc.last_amp;
        if delta != 0 {
            self.osc.last_amp = amp;
            synth.offset(time, delta, &mut bufs.bufs[out]);
        }
        time += self.osc.delay;
        if playing == 0 {
            time = end_time;
        }
        if time < end_time {
            const TABLE: [i32; 8] = [8, 16, 32, 48, 64, 80, 96, 112];
            let r3 = self.osc.reg(regs, 3);
            let period = TABLE[usize::from(r3 & 7)] << (r3 >> 4);
            let buf = &mut bufs.bufs[out];
            let resampled_period = buf.resampled_duration(period);
            let mut resampled_time = buf.resampled_time(time);
            let mut bits = self.bits;
            let mut delta = amp * 2;
            loop {
                let changed = (bits >> tap).wrapping_add(1);
                time += period;
                bits <<= 1;
                if changed & 2 != 0 {
                    delta = -delta;
                    bits |= 1;
                    synth.offset_resampled(resampled_time, delta, buf);
                }
                resampled_time = resampled_time.wrapping_add(resampled_period);
                if time >= end_time {
                    break;
                }
            }
            self.bits = bits;
            self.osc.last_amp = delta >> 1;
        }
        self.osc.delay = time - end_time;
    }
}

const WAVE_SIZE: usize = 32;

#[derive(Clone, Copy, Debug, Default)]
struct Wave {
    osc: Osc,
    wave_pos: i32,
    wave: [u8; WAVE_SIZE],
}

impl Wave {
    fn write(&mut self, regs: &[u8; REGISTER_COUNT], reg: usize, data: u8) {
        match reg {
            0 => {
                if data & 0x80 == 0 {
                    self.osc.enabled = 0;
                }
            }
            1 => self.osc.length = 256 - i32::from(self.osc.reg(regs, 1)),
            2 => self.osc.volume = i32::from(data >> 5 & 3),
            4 if data & TRIGGER & self.osc.reg(regs, 0) != 0 => {
                self.wave_pos = 0;
                self.osc.enabled = 1;
                if self.osc.length == 0 {
                    self.osc.length = 256;
                }
            }
            _ => {}
        }
    }

    fn run(
        &mut self,
        regs: &[u8; REGISTER_COUNT],
        mut time: i32,
        end_time: i32,
        mut playing: i32,
        synth: &BlipSynth<8>,
        bufs: &mut StereoBuffer,
    ) {
        let Some(out) = self.osc.output else { return };
        let volume_shift = (self.osc.volume - 1) & 7;
        let frequency;
        {
            let mut amp = ((i32::from(self.wave[self.wave_pos as usize]) >> volume_shift) & playing) * 2;
            frequency = self.osc.frequency(regs);
            if (frequency - 1) as u32 > 2044 {
                amp = (30 >> volume_shift) & playing;
                playing = 0;
            }
            let delta = amp - self.osc.last_amp;
            if delta != 0 {
                self.osc.last_amp = amp;
                synth.offset(time, delta, &mut bufs.bufs[out]);
            }
        }
        time += self.osc.delay;
        if playing == 0 {
            time = end_time;
        }
        if time < end_time {
            let buf = &mut bufs.bufs[out];
            let period = (2048 - frequency) * 2;
            let mut wave_pos = (self.wave_pos + 1) & (WAVE_SIZE as i32 - 1);
            loop {
                let amp = (i32::from(self.wave[wave_pos as usize]) >> volume_shift) * 2;
                wave_pos = (wave_pos + 1) & (WAVE_SIZE as i32 - 1);
                let delta = amp - self.osc.last_amp;
                if delta != 0 {
                    self.osc.last_amp = amp;
                    synth.offset(time, delta, buf);
                }
                time += period;
                if time >= end_time {
                    break;
                }
            }
            self.wave_pos = (wave_pos - 1) & (WAVE_SIZE as i32 - 1);
        }
        self.osc.delay = time - end_time;
    }
}

/// The APU and its stereo buffer (`GbApuWrapper`: a `Gb_Apu` writing into a `Stereo_Buffer`)
#[derive(Clone, Debug)]
pub struct GbApu {
    square1: Square,
    square2: Square,
    wave: Wave,
    noise: Noise,
    next_frame_time: i32,
    last_time: i32,
    frame_period: i32,
    volume_unit: f64,
    frame_count: i32,
    regs: [u8; REGISTER_COUNT],
    square_synth: BlipSynth<12>,
    other_synth: BlipSynth<8>,
    pub buf: StereoBuffer,
}

impl GbApu {
    /// A new APU writing into a new stereo buffer at `sample_rate`, clocked at `clock_rate` (`GbApuWrapper` made and
    /// `setSampleRate`): allocates, build time only
    pub fn new(sample_rate: i64, clock_rate: i64) -> Self {
        let mut apu = Self {
            square1: Square::default(),
            square2: Square::default(),
            wave: Wave::default(),
            noise: Noise::default(),
            next_frame_time: 0,
            last_time: 0,
            frame_period: 0,
            volume_unit: 0.0,
            frame_count: 0,
            // (uninitialized in the original until its reset; the kernels made meanwhile are made again by the first
            // treble_eq, before anything sounds)
            regs: [0; REGISTER_COUNT],
            square_synth: BlipSynth::new(),
            other_synth: BlipSynth::new(),
            buf: StereoBuffer::default(),
        };
        apu.square1.osc.regs = 0;
        apu.square2.osc.regs = 5;
        apu.wave.osc.regs = 10;
        apu.noise.osc.regs = 15;
        apu.frame_period = 4_194_304 / 256;
        apu.set_volume(1.0);
        apu.reset();
        // GbApuWrapper::setSampleRate
        for i in 0..OSC_COUNT {
            apu.osc_output(i, Some(CENTER), Some(LEFT), Some(RIGHT));
        }
        apu.buf.set_clock_rate(clock_rate);
        apu.buf.set_sample_rate(sample_rate, crate::blip::DEFAULT_LENGTH_MS);
        apu
    }

    /// Becomes a copy of `other` (made with the same rates: no allocation)
    pub fn copy_from(&mut self, other: &Self) {
        self.square1 = other.square1;
        self.square2 = other.square2;
        self.wave = other.wave;
        self.noise = other.noise;
        self.next_frame_time = other.next_frame_time;
        self.last_time = other.last_time;
        self.frame_period = other.frame_period;
        self.volume_unit = other.volume_unit;
        self.frame_count = other.frame_count;
        self.regs = other.regs;
        self.square_synth.copy_from(&other.square_synth);
        self.other_synth.copy_from(&other.other_synth);
        self.buf.copy_from(&other.buf);
    }

    fn osc_mut(&mut self, i: usize) -> &mut Osc {
        match i {
            0 => &mut self.square1.osc,
            1 => &mut self.square2.osc,
            2 => &mut self.wave.osc,
            _ => &mut self.noise.osc,
        }
    }

    fn osc_output(&mut self, i: usize, center: Option<usize>, left: Option<usize>, right: Option<usize>) {
        let osc = self.osc_mut(i);
        osc.outputs[1] = right;
        osc.outputs[2] = left;
        osc.outputs[3] = center;
        osc.output = osc.outputs[osc.output_select];
    }

    fn set_volume(&mut self, vol: f64) {
        self.volume_unit = 0.60 / OSC_COUNT as f64 / 15.0 / 2.0 / 8.0 * vol;
        self.update_volume();
    }

    fn update_volume(&mut self) {
        let data = i32::from(self.regs[(VOL_REG - START_ADDR) as usize]);
        let vol = f64::from((data & 7).max(data >> 4 & 7) + 1) * self.volume_unit;
        self.square_synth.volume(vol);
        self.other_synth.volume(vol);
    }

    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.square_synth.treble_eq(eq);
        self.other_synth.treble_eq(eq);
    }

    /// `Gb_Apu::reset`
    pub fn reset(&mut self) {
        self.next_frame_time = 0;
        self.last_time = 0;
        self.frame_count = 0;
        self.square1.reset();
        self.square2.reset();
        self.wave.osc.reset();
        self.noise.osc.env_delay = 0;
        self.noise.osc.reset();
        self.noise.bits = 1;
        self.wave.wave_pos = 0;
        self.regs[(VOL_REG - START_ADDR) as usize] = 0x77;
        self.update_volume();
        self.regs[(STATUS_REG - START_ADDR) as usize] = 0x01;
        self.write_register(0, STATUS_REG, 0x00);
        const INITIAL_WAVE: [u8; 16] =
            [0x84, 0x40, 0x43, 0xAA, 0x2D, 0x78, 0x92, 0x3C, 0x60, 0x59, 0x59, 0xB0, 0x34, 0xB8, 0x2E, 0xDA];
        self.wave.wave[..16].copy_from_slice(&INITIAL_WAVE);
    }

    fn playing(osc: &Osc, regs: &[u8; REGISTER_COUNT]) -> i32 {
        if osc.enabled != 0 && osc.volume != 0 && (osc.reg(regs, 4) & LEN_ENABLED_MASK == 0 || osc.length != 0) {
            -1
        } else {
            0
        }
    }

    fn run_until(&mut self, end_time: i32) {
        if end_time == self.last_time {
            return;
        }
        loop {
            let time = self.next_frame_time.min(end_time);
            let regs = self.regs;
            if let Some(o) = self.square1.osc.output {
                self.buf.bufs[o].set_modified();
                let p = Self::playing(&self.square1.osc, &regs);
                self.square1.run(&regs, self.last_time, time, p, &self.square_synth, &mut self.buf);
            }
            if let Some(o) = self.square2.osc.output {
                self.buf.bufs[o].set_modified();
                let p = Self::playing(&self.square2.osc, &regs);
                self.square2.run(&regs, self.last_time, time, p, &self.square_synth, &mut self.buf);
            }
            if let Some(o) = self.wave.osc.output {
                self.buf.bufs[o].set_modified();
                let p = Self::playing(&self.wave.osc, &regs);
                self.wave.run(&regs, self.last_time, time, p, &self.other_synth, &mut self.buf);
            }
            if let Some(o) = self.noise.osc.output {
                self.buf.bufs[o].set_modified();
                let p = Self::playing(&self.noise.osc, &regs);
                self.noise.run(&regs, self.last_time, time, p, &self.other_synth, &mut self.buf);
            }
            self.last_time = time;
            if time == end_time {
                break;
            }
            self.next_frame_time += self.frame_period;
            // 256 Hz actions
            self.square1.osc.clock_length(&self.regs);
            self.square2.osc.clock_length(&self.regs);
            self.wave.osc.clock_length(&self.regs);
            self.noise.osc.clock_length(&self.regs);
            self.frame_count = (self.frame_count + 1) & 3;
            if self.frame_count == 0 {
                // 64 Hz actions
                self.square1.osc.clock_envelope(&self.regs);
                self.square2.osc.clock_envelope(&self.regs);
                self.noise.osc.clock_envelope(&self.regs);
            }
            if self.frame_count & 1 != 0 {
                self.square1.clock_sweep(&mut self.regs); // 128 Hz action
            }
        }
    }

    /// Runs to `end_time`, ends the frame there and starts the next at 0 (the APU and the buffer:
    /// `GbApuWrapper::endFrame`)
    pub fn end_frame(&mut self, end_time: i32) {
        if end_time > self.last_time {
            self.run_until(end_time);
        }
        self.next_frame_time -= end_time;
        self.last_time -= end_time;
        self.buf.end_frame(end_time);
    }

    pub fn write_register(&mut self, time: i32, addr: u32, data: u8) {
        let reg = addr.wrapping_sub(START_ADDR) as usize;
        if reg >= REGISTER_COUNT {
            return;
        }
        self.run_until(time);
        let old_reg = self.regs[reg];
        self.regs[reg] = data;
        if addr < VOL_REG {
            self.write_osc(reg / 5, reg, data);
        } else if addr == VOL_REG && data != old_reg {
            // return all oscs to 0
            for i in 0..OSC_COUNT {
                let osc = *self.osc_mut(i);
                let amp = osc.last_amp;
                self.osc_mut(i).last_amp = 0;
                if amp != 0
                    && osc.enabled != 0
                    && let Some(o) = osc.output
                {
                    self.other_synth.offset(time, -amp, &mut self.buf.bufs[o]);
                }
            }
            if let Some(o) = self.wave.osc.outputs[3] {
                self.other_synth.offset(time, 30, &mut self.buf.bufs[o]);
            }
            self.update_volume();
            if let Some(o) = self.wave.osc.outputs[3] {
                self.other_synth.offset(time, -30, &mut self.buf.bufs[o]);
            }
        } else if addr == 0xFF25 || addr == STATUS_REG {
            let mask: i32 = if self.regs[(STATUS_REG - START_ADDR) as usize] & 0x80 != 0 { !0 } else { 0 };
            let flags = i32::from(self.regs[(0xFF25 - START_ADDR) as usize]) & mask;
            for i in 0..OSC_COUNT {
                let osc = self.osc_mut(i);
                osc.enabled &= mask;
                let bits = flags >> i;
                let old_output = osc.output;
                osc.output_select = ((bits >> 3 & 2) | (bits & 1)) as usize;
                osc.output = osc.outputs[osc.output_select];
                if osc.output != old_output {
                    let amp = osc.last_amp;
                    osc.last_amp = 0;
                    if amp != 0
                        && let Some(o) = old_output
                    {
                        self.other_synth.offset(time, -amp, &mut self.buf.bufs[o]);
                    }
                }
            }
            if addr == STATUS_REG && data != old_reg && data & 0x80 == 0 {
                for (i, &value) in POWERUP_REGS.iter().enumerate() {
                    if i != (STATUS_REG - START_ADDR) as usize {
                        self.write_register(time, i as u32 + START_ADDR, value);
                    }
                }
            }
        } else if addr >= 0xFF30 {
            let index = ((addr & 0x0F) * 2) as usize;
            self.wave.wave[index] = data >> 4;
            self.wave.wave[index + 1] = data & 0x0F;
        }
    }

    fn write_osc(&mut self, index: usize, reg: usize, data: u8) {
        let reg = reg - index * 5;
        match index {
            0 => {
                if self.square1.osc.env_write(&self.regs, reg, data) {
                    self.square1.sweep_freq = self.square1.osc.frequency(&self.regs);
                    let r0 = self.regs[0];
                    if r0 & PERIOD_MASK != 0 && r0 & SHIFT_MASK != 0 {
                        self.square1.sweep_delay = 1; // cause sweep to recalculate now
                        self.square1.clock_sweep(&mut self.regs);
                    }
                }
            }
            1 => {
                self.square2.osc.env_write(&self.regs, reg, data);
            }
            2 => self.wave.write(&self.regs, reg, data),
            _ => {
                if self.noise.osc.env_write(&self.regs, reg, data) {
                    self.noise.bits = 0x7FFF;
                }
            }
        }
    }
}
