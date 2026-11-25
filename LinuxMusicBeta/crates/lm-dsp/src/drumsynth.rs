//! DrumSynth (Paul Kellett's, as the legacy program renders its .ds files): a percussion sound made of a tone
//! (with its pitch's droop), a sloped noise, two overtones (added, FM, RM or the 808 cymbal's six square waves), two
//! noise bands, a resonant filter with its envelope, and a distortion of bits and rate; each part with its envelope of
//! points. Rendered once (a few milliseconds), away from the audio thread.
//!
//! A .ds file is an INI file; its values are read as the legacy program read them (its quirks kept: a line of 199
//! characters or more ends the search, a number is read from its first 15 characters, a value starts right after the
//! character that ends its key), so that every file sounds as it did.

use crate::math::{LegacyRand, cosf, expf, f32c::LN_10, f32c::PI, powf, sinf};

/// A file's settings: its text, searched as the legacy program searched it
struct Profile<'a> {
    text: &'a [u8],
}

fn eq_ignore_case(a: &[u8], b: &str) -> bool {
    a.eq_ignore_ascii_case(b.as_bytes())
}

impl Profile<'_> {
    /// The value of @p key in section @p sec, at most @p size - 1 characters, or @p def
    fn string(&self, sec: &str, key: &str, def: &str, size: usize) -> Vec<u8> {
        let maxlen = size.max(1) - 1;
        let text = self.text;
        let mut at = 0usize;
        let mut in_section = false;
        let mut found: Vec<u8> = Vec::new();
        // getline into 200 characters: up to 199 stored, failing (and ending the search) when it fills first
        let getline = |at: &mut usize, delim: u8| -> Option<Vec<u8>> {
            let mut line = Vec::new();
            loop {
                if *at >= text.len() {
                    return Some(line);
                }
                if line.len() == 199 {
                    return None;
                }
                let c = text[*at];
                *at += 1;
                if c == delim {
                    return Some(line);
                }
                line.push(c);
            }
        };
        while at < text.len() {
            if !in_section {
                match text[at..].iter().position(|&c| c == b'[') {
                    Some(p) => at += p + 1,
                    None => break,
                }
                if at >= text.len() {
                    break;
                }
                let Some(line) = getline(&mut at, b']') else { break };
                if eq_ignore_case(&line, sec) {
                    in_section = true;
                }
            } else {
                let Some(line) = getline(&mut at, b'\n') else { break };
                if line.first() == Some(&b'[') {
                    break;
                }
                // strtok(line, " \t="): the key; then the rest, from after the character that ended it
                let is_delim = |c: u8| c == b' ' || c == b'\t' || c == b'=';
                let Some(start) = line.iter().position(|&c| !is_delim(c)) else { continue };
                let end = line[start..].iter().position(|&c| is_delim(c)).map_or(line.len(), |p| start + p);
                let k = &line[start..end];
                let rest: &[u8] = if end < line.len() { &line[end + 1..] } else { &[] };
                let skip = rest.iter().position(|&c| c != b'\n' && c != b'\r').unwrap_or(rest.len());
                let rest = &rest[skip..];
                let stop = rest.iter().position(|&c| c == b'\n' || c == b'\r' || c == 0).unwrap_or(rest.len());
                let b = &rest[..stop];
                if eq_ignore_case(k, key) {
                    if !b.is_empty() {
                        let mut v = b.to_vec();
                        while matches!(v.last(), Some(b' ' | b'\t' | b'\r')) {
                            v.pop();
                        }
                        v.truncate(maxlen);
                        found = v;
                    }
                    break;
                }
            }
        }
        if found.is_empty() {
            let mut d = def.as_bytes().to_vec();
            d.truncate(maxlen);
            d
        } else {
            found
        }
    }

    fn int(&self, sec: &str, key: &str, def: i32) -> i32 {
        let v = self.string(sec, key, "", 16);
        if v.is_empty() {
            return def;
        }
        strtol(&v).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }

    fn float(&self, sec: &str, key: &str, def: f32) -> f32 {
        let v = self.string(sec, key, "", 16);
        if v.is_empty() {
            return def;
        }
        strtof(&v).unwrap_or(0.0)
    }
}

/// C's strtol, base 10: the leading whole number (0 if none)
fn strtol(s: &[u8]) -> i64 {
    let mut i = s.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(s.len());
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v.saturating_mul(10).saturating_add(i64::from(s[i] - b'0'));
        i += 1;
    }
    if neg { -v } else { v }
}

/// C's strtof (and scanf's %f): the leading decimal number, if there is one
fn strtof(s: &[u8]) -> Option<f32> {
    let mut i = s.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(s.len());
    let start = i;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    let digits_start = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - digits_start;
    if i < s.len() && s[i] == b'.' {
        i += 1;
        let frac = i;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        digits += i - frac;
    }
    if digits == 0 {
        return None;
    }
    if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        if j < s.len() && s[j].is_ascii_digit() {
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let text = core::str::from_utf8(&s[start..i]).ok()?;
    // (a trailing point: Rust's parser wants a digit after it)
    let text = text.strip_suffix('.').unwrap_or(text);
    text.parse::<f32>().ok()
}

/// A value from a file as a whole number: one out of range, or not a number, is 0
fn to_int(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    value.clamp(-2147483520.0, 2147483520.0) as i32
}

#[inline]
fn fast_pow10f(x: f32) -> f32 {
    expf(LN_10 * x)
}

const MAX: usize = 0;
const ENV: usize = 1;
const PNT: usize = 2;
const DENV: usize = 3;
const NEXTT: usize = 4;

/// The envelopes: their points (time, level) and their running state
struct Envelopes {
    pts: [[[f32; 32]; 2]; 8],
    data: [[f32; 6]; 8],
    timestretch: f32,
}

impl Envelopes {
    fn read(&mut self, env: usize, p: &Profile<'_>, sec: &str, key: &str) {
        let mut en = p.string(sec, key, "0,0 100,0", 256);
        en.truncate(255);
        const MAX_POINTS: usize = 31;
        let mut ep = 0;
        for pair in en.split(|&c| c == b' ') {
            if pair.is_empty() {
                continue;
            }
            if ep == MAX_POINTS {
                break;
            }
            let comma = pair.iter().position(|&c| c == b',');
            let time = comma.map_or(pair, |c| &pair[..c]);
            self.pts[env][0][ep] = strtof(time).unwrap_or(0.0);
            self.pts[env][1][ep] = comma.map_or(0.0, |c| strtof(&pair[c + 1..]).unwrap_or(0.0));
            ep += 1;
        }
        if ep == 0 {
            self.pts[env][0][0] = 0.0;
            self.pts[env][1][0] = 0.0;
            ep = 1;
        }
        self.pts[env][0][ep] = -1.0;
        self.data[env][MAX] = self.pts[env][0][ep - 1];
    }

    fn update(&mut self, e: usize, t: i64) {
        let point = self.data[e][PNT] as usize;
        self.data[e][ENV] = self.pts[e][1][point] * 0.01;
        if self.pts[e][0][point + 1] < 0.0 {
            // The end point holds its level
            self.data[e][NEXTT] = 442000.0 * self.timestretch;
            self.data[e][DENV] = 0.0;
            return;
        }
        self.data[e][NEXTT] = self.pts[e][0][point + 1] * self.timestretch;
        let end_env = self.pts[e][1][point + 1] * 0.01;
        let mut dt = self.data[e][NEXTT] - t as f32;
        if dt < 1.0 {
            dt = 1.0;
        }
        self.data[e][DENV] = (end_env - self.data[e][ENV]) / dt;
        self.data[e][PNT] += 1.0;
    }

    /// One step of envelope @p e at frame @p t
    #[inline]
    fn step(&mut self, e: usize, t: i64) {
        if (t as f32) < self.data[e][NEXTT] {
            self.data[e][ENV] += self.data[e][DENV];
        } else {
            self.update(e, t);
        }
    }
}

fn waveform(ph: f32, form: i32) -> f32 {
    match form {
        0 => sinf(ph),
        1 => (2.0 * sinf(0.5 * ph)).abs() - 1.0,
        _ => {
            let x = ph / (2.0 * PI);
            let saw01 = x - x.floor();
            match form {
                2 => 1.0 - 4.0 * (saw01 - 0.5).abs(),
                3 => 2.0 * saw01 - 1.0,
                _ => {
                    if saw01 < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
            }
        }
    }
}

/// Renders the .ds file @p text at @p sample_rate: its 16-bit mono frames (none if it is not a DrumSynth file); its
/// noise from @p rand (the legacy program's generator, drawn in its order)
pub fn render(text: &[u8], sample_rate: u32, rand: &mut LegacyRand) -> Vec<i16> {
    let p = Profile { text };
    let fs = sample_rate as f32;
    let mut sec = "General";
    let ver = p.string(sec, "Version", "", 32);
    let mut v = [0u8; 32];
    v[..ver.len()].copy_from_slice(&ver);
    v[9] = 0;
    let name_end = v.iter().position(|&c| c == 0).unwrap_or(32);
    if !eq_ignore_case(&v[..name_end], "DrumSynth") || (v[11] != b'1' && v[11] != b'2') {
        return Vec::new();
    }

    let mut env = Envelopes { pts: [[[0.0; 32]; 2]; 8], data: [[0.0; 6]; 8], timestretch: 0.0 };
    let mut chk_on = [0i32; 8];
    let mut sli_lev = [0i32; 8];

    env.timestretch = (0.01 * p.float(sec, "Stretch", 100.0)).clamp(0.2, 10.0);
    // (the envelopes' unit is a sample at 44.1 kHz)
    env.timestretch *= fs / 44100.0;
    let mut d_gain = fast_pow10f(0.05 * p.float(sec, "Level", 0.0));
    let master_tune = powf(1.0594631, p.float(sec, "Tuning", 0.0) + 0.0);
    let mut main_filter = 2 * p.int(sec, "Filter", 0);
    let mf_res = (0.0101 * p.float(sec, "Resonance", 0.0)).sqrt();
    let high_pass = p.int(sec, "HighPass", 0);
    env.read(7, &p, sec, "FilterEnv");

    // Noise
    sec = "Noise";
    chk_on[1] = p.int(sec, "On", 0);
    sli_lev[1] = p.int(sec, "Level", 0);
    let nt = p.int(sec, "Slope", 0);
    env.read(2, &p, sec, "Envelope");
    let mut non = chk_on[1];
    let nl = (sli_lev[1].wrapping_mul(sli_lev[1])) as f32;
    let (a, b, c, d, g);
    if nt < 0 {
        a = 1.0 + (nt as f32 / 105.0);
        b = 0.0;
        c = 0.0;
        d = -nt as f32 / 105.0;
        g = (1.0 + 0.0005 * nt as f32 * nt as f32) * nl;
    } else {
        a = 1.0;
        b = -nt as f32 / 50.0;
        c = (nt as f32).abs() / 100.0;
        d = 0.0;
        g = nl;
    }

    // Tone
    sec = "Tone";
    chk_on[0] = p.int(sec, "On", 0);
    let mut ton = chk_on[0];
    sli_lev[0] = p.int(sec, "Level", 128);
    let tl = (sli_lev[0].wrapping_mul(sli_lev[0])) as f32;
    env.read(1, &p, sec, "Envelope");
    let mut f1 = master_tune * 2.0 * PI * p.float(sec, "F1", 200.0) / fs;
    if f1.abs() < 0.001 {
        f1 = 0.001;
    }
    let mut f2 = master_tune * 2.0 * PI * p.float(sec, "F2", 120.0) / fs;
    let mut droop_rate = p.float(sec, "Droop", 0.0);
    let droop;
    let ddf;
    if droop_rate > 0.0 {
        droop_rate = fast_pow10f((droop_rate - 20.0) / 30.0);
        droop_rate = droop_rate * -4.0 / env.data[1][MAX];
        droop = true;
        f2 = f1 + ((f2 - f1) / (1.0 - expf(droop_rate * env.data[1][MAX])));
        ddf = f1 - f2;
    } else {
        droop = false;
        ddf = f2 - f1;
    }
    let mut tphi = p.float(sec, "Phase", 90.0) / 57.29578;

    // Overtones
    sec = "Overtones";
    chk_on[2] = p.int(sec, "On", 0);
    let oon = chk_on[2];
    sli_lev[2] = p.int(sec, "Level", 128);
    let ol = (sli_lev[2].wrapping_mul(sli_lev[2])) as f32;
    env.read(3, &p, sec, "Envelope1");
    env.read(4, &p, sec, "Envelope2");
    let omode = p.int(sec, "Method", 2);
    let mut of1 = master_tune * 2.0 * PI * p.float(sec, "F1", 200.0) / fs;
    let mut of2 = master_tune * 2.0 * PI * p.float(sec, "F2", 120.0) / fs;
    let ow1 = p.int(sec, "Wave1", 0);
    let ow2 = p.int(sec, "Wave2", 0);
    let mut obal2 = p.int(sec, "Param", 50) as f32;
    let odrive = (obal2 * obal2 * obal2) / 125000.0;
    obal2 *= 0.01;
    let obal1 = 1.0 - obal2;
    let mut ophi1 = tphi;
    let mut ophi2 = tphi;
    if main_filter == 0 {
        main_filter = p.int(sec, "Filter", 0);
    }
    let mut of1_sync = false;
    let mut of2_sync = false;
    if p.int(sec, "Track1", 0) == 1 && ton == 1 {
        of1_sync = true;
        of1 /= f1;
    }
    if p.int(sec, "Track2", 0) == 1 && ton == 1 {
        of2_sync = true;
        of2 /= f1;
    }
    let mut oca = 0.28 + obal1 * obal1;
    let ocq = oca * oca;
    let ocf = (1.8 - 0.7 * ocq) * 0.92;
    oca *= 1.0 + 4.0 * obal1;
    let _ = oca;
    let ocf1_base = 2.0 * PI / of1;
    let ocf2 = 2.0 * PI / of2;
    let mut oc = [[0.0f32; 2]; 6];
    for (i, o) in oc.iter_mut().enumerate() {
        let v = ocf1_base + (ocf2 - ocf1_base) * 0.2 * i as f32;
        *o = [v, v];
    }
    let (mut oc0, mut oc1, mut oc2) = (0.0f32, 0.0f32, 0.0f32);

    // Noise band 1
    sec = "NoiseBand";
    chk_on[3] = p.int(sec, "On", 0);
    let mut bon = chk_on[3];
    sli_lev[3] = p.int(sec, "Level", 128);
    let bl = (sli_lev[3].wrapping_mul(sli_lev[3])) as f32;
    let bf = master_tune * 2.0 * PI * p.float(sec, "F", 1000.0) / fs;
    let mut bphi = PI / 4.0;
    env.read(5, &p, sec, "Envelope");
    let df = p.int(sec, "dF", 50);
    let mut bq = df as f32;
    bq = bq * bq / (10000.0 - 6600.0 * (bf.sqrt() - 0.19));
    let bf_step = 1 + to_int(f64::from((40.0 - (df as f32 / 2.5)) / (bq + 1.0 + (1.0 * bf))));

    // Noise band 2
    sec = "NoiseBand2";
    chk_on[4] = p.int(sec, "On", 0);
    let mut bon2 = chk_on[4];
    sli_lev[4] = p.int(sec, "Level", 128);
    let bl2 = (sli_lev[4].wrapping_mul(sli_lev[4])) as f32;
    let bf2 = master_tune * 2.0 * PI * p.float(sec, "F", 1000.0) / fs;
    let mut bphi2 = PI / 4.0;
    env.read(6, &p, sec, "Envelope");
    let df2 = p.int(sec, "dF", 50);
    let mut bq2 = df2 as f32;
    bq2 = bq2 * bq2 / (10000.0 - 6600.0 * (bf2.sqrt() - 0.19));
    let bf_step2 = 1 + to_int((40.0 - (f64::from(df2) / 2.5)) / f64::from(bq2 + 1.0 + (1.0 * bf2)));

    // Distortion
    sec = "Distortion";
    chk_on[5] = p.int(sec, "On", 0);
    let dion = chk_on[5];
    let mut dstep = 1 + i64::from(p.int(sec, "Rate", 0));
    dstep = match dstep {
        7 => 20,
        6 => 10,
        5 => 8,
        s => s,
    }
    .clamp(1, 1200);
    let mut clippoint: i16 = 32700;
    let mut d_atten = 1.0f32;
    if dion == 1 {
        let mut loudest = 0.0f32;
        for i in 0..5 {
            if chk_on[i] == 1 && sli_lev[i] as f32 > loudest {
                loudest = sli_lev[i] as f32;
            }
        }
        d_atten = d_gain * (loudest * loudest);
        clippoint = d_atten.clamp(0.0, 32700.0) as i16;
        d_atten = crate::math::exp2(2.0 * f64::from(p.int(sec, "Bits", 0))) as f32;
        d_gain = d_atten * d_gain * fast_pow10f(0.05 * p.int(sec, "Clipping", 0) as f32);
    }

    // The envelopes, from their start; the length, of the longest part that is on
    for i in 1..8 {
        env.data[i][NEXTT] = 0.0;
        env.data[i][PNT] = 0.0;
    }
    let length = {
        let mut l = 0.0f32;
        for e in 1..7 {
            let eon = if e - 1 > 2 { e - 2 } else { e - 1 };
            let mut pt = 0;
            while env.pts[e][0][pt + 1] >= 0.0 {
                pt += 1;
            }
            env.data[e][MAX] = env.pts[e][0][pt] * env.timestretch;
            if chk_on[eon] == 1 && env.data[e][MAX] > l {
                l = env.data[e][MAX];
            }
        }
        const MAX_BLOCKS: i32 = (1 << 24) / 1200;
        2400 + 1200 * i64::from(to_int(f64::from(l / 1200.0)).clamp(0, MAX_BLOCKS))
    };

    let mut wave = Vec::with_capacity(length as usize);
    let mut buf = [0.0f32; 1200];
    let mut phi = [0.0f32; 1200];
    let mut x = [0.0f32; 3];
    let mut tt = 0.0f32;
    let mut tphi_start = 0.0f32;
    let (mut bdf, mut bdf2) = (0.0f32, 0.0f32);
    let (mut mf_in, mut mf_out) = (0.0f32, 0.0f32);
    let mut ot = 0.0f32;
    let mut tpos: i64 = 0;
    while tpos < length {
        let tplus = tpos + 1199;
        if non == 1 {
            for t in tpos..=tplus {
                env.step(2, t);
                x[2] = x[1];
                x[1] = x[0];
                x[0] = -1.0 + rand.inclusive(2.0);
                tt = a * x[0] + b * x[1] + c * x[2] + d * tt;
                buf[(t - tpos) as usize] = tt * g * env.data[2][ENV];
            }
            if (tplus + 1) as f32 >= env.data[2][MAX] {
                non = 0;
            }
        } else {
            buf.fill(0.0);
        }

        if ton == 1 {
            tphi_start = tphi;
            for t in tpos..=tplus {
                phi[(t - tpos) as usize] = if droop {
                    f2 + (ddf * expf(t as f32 * droop_rate))
                } else {
                    f1 + (t as f32 / env.data[1][MAX]) * ddf
                };
            }
            for t in tpos..=tplus {
                let i = (t - tpos) as usize;
                env.step(1, t);
                tphi += phi[i];
                buf[i] += tl * env.data[1][ENV] * sinf(tphi);
            }
            if (tplus + 1) as f32 >= env.data[1][MAX] {
                ton = 0;
            }
        } else {
            // (for the overtones' tracking)
            phi.fill(f2);
        }

        if bon == 1 {
            for t in tpos..=tplus {
                env.step(5, t);
                if t.checked_rem(i64::from(bf_step)) == Some(0) {
                    bdf = -0.5 + rand.inclusive(1.0);
                }
                bphi = bphi + bf + bq * bdf;
                let i = (t - tpos) as usize;
                buf[i] += cosf(bphi) * env.data[5][ENV] * bl;
            }
            if (tplus + 1) as f32 >= env.data[5][MAX] {
                bon = 0;
            }
        }
        if bon2 == 1 {
            for t in tpos..=tplus {
                env.step(6, t);
                if t.checked_rem(i64::from(bf_step2)) == Some(0) {
                    bdf2 = -0.5 + rand.inclusive(1.0);
                }
                bphi2 = bphi2 + bf2 + bq2 * bdf2;
                let i = (t - tpos) as usize;
                buf[i] += cosf(bphi2) * env.data[6][ENV] * bl2;
            }
            if (tplus + 1) as f32 >= env.data[6][MAX] {
                bon2 = 0;
            }
        }

        for t in tpos..=tplus {
            let i = (t - tpos) as usize;
            if oon == 1 {
                for e in [3, 4] {
                    if (t as f32) < env.data[e][NEXTT] {
                        env.data[e][ENV] += env.data[e][DENV];
                    } else if t as f32 >= env.data[e][MAX] {
                        // (it waits for the other one)
                        env.data[e][ENV] = 0.0;
                        env.data[e][DENV] = 0.0;
                        env.data[e][NEXTT] = 999999.0;
                    } else {
                        env.update(e, t);
                    }
                }
                tphi_start += phi[i];
                ophi1 = if of1_sync { tphi_start * of1 } else { ophi1 + of1 };
                ophi2 = if of2_sync { tphi_start * of2 } else { ophi2 + of2 };
                ot = 0.0;
                let (e3, e4) = (env.data[3][ENV], env.data[4][ENV]);
                match omode {
                    0 => {
                        ot = obal1 * e3 * waveform(ophi1, ow1);
                        ot = ol * (ot + obal2 * e4 * waveform(ophi2, ow2));
                    }
                    1 => {
                        ot = odrive * e4 * waveform(ophi2, ow2);
                        ot = ol * e3 * waveform(ophi1 + ot, ow1);
                    }
                    2 => {
                        ot = (1.0 - odrive / 8.0) + (((odrive / 8.0) * e4) * waveform(ophi2, ow2));
                        ot *= ol * e3 * waveform(ophi1, ow1);
                    }
                    3 => {
                        for o in &mut oc {
                            o[0] += 1.0;
                            if o[0] > o[1] {
                                o[0] -= o[1];
                                ot = ol * e3;
                            }
                        }
                        let f = e4 * ocf;
                        oc0 += f * oc1;
                        oc1 += f * (ot + oc2 - ocq * oc1 - oc0);
                        oc2 = ot;
                        ot = oc1;
                    }
                    _ => {}
                }
            }
            if main_filter == 1 || main_filter == 2 {
                env.step(7, t);
                let m = env.data[7][ENV];
                let fb = if m > 0.2 { 1.001 - fast_pow10f(m - 1.0) } else { 0.999 - 0.7824 * m };
                let input = if main_filter == 1 { ot } else { buf[i] + ot };
                let tmp = input + mf_res * (1.0 + (1.0 / fb)) * (mf_in - mf_out);
                mf_in = fb * (mf_in - tmp) + tmp;
                mf_out = fb * (mf_out - mf_in) + mf_in;
                buf[i] = if main_filter == 1 {
                    buf[i] + (mf_out - (high_pass as f32 * ot))
                } else {
                    mf_out - (high_pass as f32 * (buf[i] + ot))
                };
            } else {
                buf[i] += ot;
            }
        }

        if dion == 1 {
            for v in &mut buf {
                *v = d_gain * to_int(f64::from(*v / d_atten)) as f32;
            }
            let mut j = 0i64;
            while j < 1200 {
                let end = (j + dstep - 1).min(1199);
                let mut ave = 0.0f32;
                for jj in j..=end {
                    ave += buf[jj as usize];
                }
                ave /= (end - j + 1) as f32;
                for jj in j..=end {
                    buf[jj as usize] = ave;
                }
                j += dstep;
            }
        } else {
            for v in &mut buf {
                *v *= d_gain;
            }
        }

        let clip = f32::from(clippoint);
        for &v in &buf {
            wave.push(if v > clip {
                clippoint
            } else if v < -clip {
                -clippoint
            } else {
                to_int(f64::from(v)) as i16
            });
        }
        tpos += 1200;
    }
    wave
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_read_as_c_reads_them() {
        assert_eq!(strtol(b"  -42abc"), -42);
        assert_eq!(strtol(b"x"), 0);
        assert_eq!(strtof(b"1.5e2z"), Some(150.0));
        assert_eq!(strtof(b"12."), Some(12.0));
        assert_eq!(strtof(b".5"), Some(0.5));
        assert_eq!(strtof(b"abc"), None);
    }

    #[test]
    fn values_are_found_in_their_section_whatever_the_case() {
        let p = Profile { text: b"[General]\r\nVersion=DrumSynth v2.0\r\n[Tone]\nf1=55.5\nOn=1\n" };
        assert_eq!(p.string("general", "version", "", 32), b"DrumSynth v2.0");
        assert_eq!(p.float("Tone", "F1", 0.0), 55.5);
        assert_eq!(p.int("Tone", "On", 0), 1);
        assert_eq!(p.int("Tone", "Level", 128), 128);
        assert_eq!(p.int("Noise", "On", 7), 7);
    }

    #[test]
    fn not_a_drumsynth_file_renders_nothing() {
        let mut rand = LegacyRand::new(1);
        assert!(render(b"[General]\nVersion=Other\n", 44100, &mut rand).is_empty());
    }
}
