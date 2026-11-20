//! A minimal SoundFont 2 made by the tests (of this crate, and with the feature `testing` of the plugins that play
//! SoundFonts), so that no SoundFont has to be installed: each preset is one instrument with one zone, a looped
//! sample of a sine (or noise) at a known pitch, with the SoundFont's default envelopes.

use std::path::Path;

/// One preset of the generated font
#[derive(Clone, Copy, Debug)]
pub struct Voice {
    pub name: &'static str,
    pub program: u16,
    pub bank: u16,
    /// Hz of the sample when played at its root key (A4 = 69)
    pub freq: f64,
    pub noise: bool,
}

const RATE: u32 = 44_100;
/// Each sample: 0.5 s, looped over whole periods
const LEN: u32 = 22_050;

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn list(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut data = kind.to_vec();
    for c in chunks {
        data.extend_from_slice(c);
    }
    chunk(b"LIST", &data)
}

fn name20(name: &str) -> [u8; 20] {
    let mut out = [0_u8; 20];
    out[..name.len().min(19)].copy_from_slice(&name.as_bytes()[..name.len().min(19)]);
    out
}

fn u16le(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn u32le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

/// Writes the font to `path`
pub fn write(path: &Path, voices: &[Voice]) -> std::io::Result<()> {
    // sdta: the samples, each followed by 46 zero points (the SF2 rule)
    let mut smpl: Vec<u8> = Vec::new();
    let mut headers = Vec::new(); // (start, end, loop_start, loop_end)
    let mut seed = 0x1234_5678_u32;
    for v in voices {
        let start = (smpl.len() / 2) as u32;
        let period = f64::from(RATE) / v.freq;
        let periods = (f64::from(LEN) / period).floor();
        let length = (periods * period).round() as u32;
        for n in 0..length {
            let s = if v.noise {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as i32 >> 16) as f64 / 32_768.0 * 0.5
            } else {
                0.5 * (std::f64::consts::TAU * v.freq * f64::from(n) / f64::from(RATE)).sin()
            };
            smpl.extend_from_slice(&((s * 32_767.0) as i16).to_le_bytes());
        }
        let end = start + length;
        smpl.extend(std::iter::repeat_n(0_u8, 46 * 2));
        headers.push((start, end, start, end));
    }

    let (mut phdr, mut pbag, mut pgen, mut inst, mut ibag, mut igen, mut shdr) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, v) in voices.iter().enumerate() {
        let i = i as u16;
        // Preset i -> preset bag i -> generator "instrument" (41) = i
        phdr.extend_from_slice(&name20(v.name));
        phdr.extend_from_slice(&u16le(v.program));
        phdr.extend_from_slice(&u16le(v.bank));
        phdr.extend_from_slice(&u16le(i));
        phdr.extend_from_slice(&[0; 12]);
        pbag.extend_from_slice(&u16le(i));
        pbag.extend_from_slice(&u16le(0));
        pgen.extend_from_slice(&u16le(41));
        pgen.extend_from_slice(&u16le(i));
        // Instrument i -> instrument bag i -> generators sampleModes (54) = 1 (loop), sampleID (53) = i (last)
        inst.extend_from_slice(&name20(v.name));
        inst.extend_from_slice(&u16le(i));
        ibag.extend_from_slice(&u16le(i * 2));
        ibag.extend_from_slice(&u16le(0));
        igen.extend_from_slice(&u16le(54));
        igen.extend_from_slice(&u16le(1));
        igen.extend_from_slice(&u16le(53));
        igen.extend_from_slice(&u16le(i));
        let (start, end, loop_start, loop_end) = headers[i as usize];
        shdr.extend_from_slice(&name20(v.name));
        for value in [start, end, loop_start, loop_end, RATE] {
            shdr.extend_from_slice(&u32le(value));
        }
        // Root key from the frequency (the sample plays at `freq` on key 69 if freq is 440)
        let root = (69.0 + 12.0 * (v.freq / 440.0).log2()).round() as u8;
        let cents = (100.0 * (69.0 + 12.0 * (v.freq / 440.0).log2() - f64::from(root))).round() as i8;
        shdr.push(root);
        shdr.push((-cents) as u8);
        shdr.extend_from_slice(&u16le(0));
        shdr.extend_from_slice(&u16le(1)); // mono
    }
    let n = voices.len() as u16;
    // The terminal records
    phdr.extend_from_slice(&name20("EOP"));
    phdr.extend_from_slice(&[0; 4]);
    phdr.extend_from_slice(&u16le(n));
    phdr.extend_from_slice(&[0; 12]);
    pbag.extend_from_slice(&u16le(n));
    pbag.extend_from_slice(&u16le(0));
    pgen.extend_from_slice(&[0; 4]);
    inst.extend_from_slice(&name20("EOI"));
    inst.extend_from_slice(&u16le(n));
    ibag.extend_from_slice(&u16le(n * 2));
    ibag.extend_from_slice(&u16le(0));
    igen.extend_from_slice(&[0; 4]);
    shdr.extend_from_slice(&name20("EOS"));
    shdr.extend_from_slice(&[0; 26]);

    let info =
        list(b"INFO", &[chunk(b"ifil", &[2, 0, 1, 0]), chunk(b"isng", b"EMU8000\0"), chunk(b"INAM", b"lm test\0")]);
    let sdta = list(b"sdta", &[chunk(b"smpl", &smpl)]);
    let pdta = list(
        b"pdta",
        &[
            chunk(b"phdr", &phdr),
            chunk(b"pbag", &pbag),
            chunk(b"pmod", &[0; 10]),
            chunk(b"pgen", &pgen),
            chunk(b"inst", &inst),
            chunk(b"ibag", &ibag),
            chunk(b"imod", &[0; 10]),
            chunk(b"igen", &igen),
            chunk(b"shdr", &shdr),
        ],
    );
    let mut body = b"sfbk".to_vec();
    body.extend_from_slice(&info);
    body.extend_from_slice(&sdta);
    body.extend_from_slice(&pdta);
    std::fs::write(path, chunk(b"RIFF", &body))
}

/// The font most tests use: a sine at 440 Hz (program 0) and noise (program 1), bank 0
pub fn standard(path: &Path) -> std::io::Result<()> {
    write(
        path,
        &[
            Voice { name: "Sine", program: 0, bank: 0, freq: 440.0, noise: false },
            Voice { name: "Noise", program: 1, bank: 0, freq: 441.0, noise: true },
        ],
    )
}
