//! STK's wave files the instruments play (`rawwaves/`, from STK 5.0.1, redistributed under its licence:
//! `rawwaves/LICENSE-STK`), in the program: decoded once, never read from a folder.
//!
//! A raw wave is headerless 16-bit signed big-endian mono at 22050 Hz (`FileRead::getRawInfo`). The library reads it
//! scaled by `1 / 32768` and then normalizes it to a peak of 1 (`FileWvIn::normalize`, the default when opening);
//! a looped wave (`FileLoop`) has one more frame at the end, a copy of the first, for the interpolation across the
//! loop point, and is normalized with it.

use std::sync::LazyLock;

/// The strike of the struck bars (`ModalBar`): 256 frames
static MARMSTK1: &[u8] = include_bytes!("../../rawwaves/marmstk1.raw");
/// The sine of the bells' operators 0 to 2 (`TubeBell`): 1024 frames
static SINEWAVE: &[u8] = include_bytes!("../../rawwaves/sinewave.raw");
/// The wave of the bells' operator 3: 256 frames
static FWAVBLNK: &[u8] = include_bytes!("../../rawwaves/fwavblnk.raw");

/// The data rate of a raw wave (Hz)
pub const RAW_RATE: f64 = 22050.0;

/// A decoded wave: `frames` frames of the file, then the loop's guard frame for a looped wave
#[derive(Debug)]
pub struct Wave {
    /// The file's length in frames (`fileSize_`)
    pub file_size: usize,
    /// The samples (`data_`): `file_size` frames, plus the guard for a looped wave
    pub data: Box<[f64]>,
}

fn decode(bytes: &[u8], looped: bool) -> Wave {
    let file_size = bytes.len() / 2;
    // FileRead::read: the samples scaled by 1 / 32768
    let gain = 1.0 / 32768.0;
    let mut data: Vec<f64> =
        bytes.as_chunks::<2>().0.iter().map(|b| f64::from(i16::from_be_bytes(*b)) * gain).collect();
    if looped {
        // FileLoop::openFile: the last frame is the first
        data.push(data[0]);
    }
    // FileWvIn::normalize(1.0)
    let mut max = 0.0f64;
    for x in &data {
        if x.abs() > max {
            max = x.abs();
        }
    }
    if max > 0.0 {
        let mut scale = 1.0 / max;
        scale *= 1.0;
        for x in &mut data {
            *x *= scale;
        }
    }
    Wave { file_size, data: data.into_boxed_slice() }
}

/// `marmstk1.raw`, played once (`FileWvIn`)
pub fn marmstk1() -> &'static Wave {
    static W: LazyLock<Wave> = LazyLock::new(|| decode(MARMSTK1, false));
    &W
}

/// `sinewave.raw`, looped (`FileLoop`)
pub fn sinewave() -> &'static Wave {
    static W: LazyLock<Wave> = LazyLock::new(|| decode(SINEWAVE, true));
    &W
}

/// `fwavblnk.raw`, looped (`FileLoop`)
pub fn fwavblnk() -> &'static Wave {
    static W: LazyLock<Wave> = LazyLock::new(|| decode(FWAVBLNK, true));
    &W
}

/// Decodes every wave (the build calls it, so that no note decodes one on the audio thread)
pub fn prepare() {
    let _ = (marmstk1(), sinewave(), fwavblnk());
    let _ = super::filters::sine_table();
    let _ = super::modal_bar::kernel();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_waves_are_decoded_and_normalized() {
        for (w, size, looped) in [(marmstk1(), 256, false), (sinewave(), 1024, true), (fwavblnk(), 256, true)] {
            assert_eq!(w.file_size, size);
            assert_eq!(w.data.len(), size + usize::from(looped));
            let peak = w.data.iter().fold(0.0f64, |m, x| m.max(x.abs()));
            assert!((peak - 1.0).abs() < 1.0e-15, "{peak}");
            if looped {
                assert_eq!(w.data[size], w.data[0]);
            }
        }
    }
}
