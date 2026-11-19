//! Sample formats of the device and the conversion between them and the `f32` the engine renders in.
//!
//! Pure Rust and platform independent (so it is tested on every platform). Every conversion is real-time safe: no
//! allocation, no lock, no system call.
//!
//! Rules:
//! - float to integer: scaled by 2^(bits-1), rounded to the nearest integer, and clamped (1.0 becomes the largest
//!   positive value, never wraps around);
//! - integer to float: divided by 2^(bits-1), so the result is in [-1, 1);
//! - the integer formats are little endian (the formats ALSA calls `*_LE`), whatever the machine's byte order;
//! - `S24` is 24 bits in the low three bytes of a 32-bit container (`S24_LE`); `S24Packed` is three bytes (`S24_3LE`).

/// A sample format a device can be configured with. The order of [`SampleFormat::PREFERENCE`] is the default order
/// of preference: the widest first, so nothing is lost on the way to and from the device
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SampleFormat {
    /// 32-bit IEEE float
    F32,
    /// 32-bit signed integer
    S32,
    /// 24-bit signed integer in a 32-bit container (the low three bytes)
    S24,
    /// 24-bit signed integer in three bytes
    S24Packed,
    /// 16-bit signed integer
    S16,
}

impl SampleFormat {
    /// The default order of preference when configuring a device
    pub const PREFERENCE: [SampleFormat; 5] =
        [SampleFormat::F32, SampleFormat::S32, SampleFormat::S24, SampleFormat::S24Packed, SampleFormat::S16];

    /// Bytes one sample takes in memory
    pub const fn bytes(self) -> usize {
        match self {
            SampleFormat::F32 | SampleFormat::S32 | SampleFormat::S24 => 4,
            SampleFormat::S24Packed => 3,
            SampleFormat::S16 => 2,
        }
    }

    /// Bits of resolution
    pub const fn bits(self) -> u32 {
        match self {
            SampleFormat::F32 => 24,
            SampleFormat::S32 => 32,
            SampleFormat::S24 | SampleFormat::S24Packed => 24,
            SampleFormat::S16 => 16,
        }
    }

    /// A stable name, for logs, preferences and labels
    pub const fn name(self) -> &'static str {
        match self {
            SampleFormat::F32 => "f32",
            SampleFormat::S32 => "s32",
            SampleFormat::S24 => "s24",
            SampleFormat::S24Packed => "s24_3",
            SampleFormat::S16 => "s16",
        }
    }

    /// Writes one sample. `out` must be at least [`SampleFormat::bytes`] long
    #[inline]
    pub fn encode(self, x: f32, out: &mut [u8]) {
        match self {
            SampleFormat::F32 => out[..4].copy_from_slice(&x.to_le_bytes()),
            SampleFormat::S32 => {
                out[..4].copy_from_slice(&to_int(x, 2_147_483_648.0, i32::MIN, i32::MAX).to_le_bytes())
            }
            SampleFormat::S24 => out[..4].copy_from_slice(&to_int(x, 8_388_608.0, -8_388_608, 8_388_607).to_le_bytes()),
            SampleFormat::S24Packed => {
                let v = to_int(x, 8_388_608.0, -8_388_608, 8_388_607).to_le_bytes();
                out[..3].copy_from_slice(&v[..3]);
            }
            SampleFormat::S16 => {
                let v = to_int(x, 32_768.0, -32_768, 32_767) as i16;
                out[..2].copy_from_slice(&v.to_le_bytes());
            }
        }
    }

    /// Reads one sample. `bytes` must be at least [`SampleFormat::bytes`] long
    #[inline]
    pub fn decode(self, bytes: &[u8]) -> f32 {
        match self {
            SampleFormat::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            SampleFormat::S32 => {
                (f64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])) / 2_147_483_648.0) as f32
            }
            SampleFormat::S24 => {
                // The container's high byte is ignored (drivers do not all sign-extend it)
                sign_extend_24(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0])) as f32 / 8_388_608.0
            }
            SampleFormat::S24Packed => {
                sign_extend_24(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0])) as f32 / 8_388_608.0
            }
            SampleFormat::S16 => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0,
        }
    }
}

#[inline]
fn to_int(x: f32, scale: f64, min: i32, max: i32) -> i32 {
    // f64: the scaled value of a full-scale f32 is exact, and the rounding happens once. NaN becomes 0
    let v = (f64::from(x) * scale).round();
    if v.is_nan() {
        0
    } else if v >= f64::from(max) {
        max
    } else if v <= f64::from(min) {
        min
    } else {
        v as i32
    }
}

#[inline]
fn sign_extend_24(v: u32) -> i32 {
    ((v << 8) as i32) >> 8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(format: SampleFormat, x: f32) -> f32 {
        let mut buf = [0u8; 4];
        format.encode(x, &mut buf);
        format.decode(&buf)
    }

    #[test]
    fn every_format_round_trips_within_one_step() {
        for format in SampleFormat::PREFERENCE {
            let step = 1.0 / (1u64 << (format.bits() - 1)) as f32;
            for i in -100..=100 {
                let x = i as f32 / 101.0;
                let y = round_trip(format, x);
                assert!((x - y).abs() <= step, "{format:?}: {x} became {y}");
            }
        }
    }

    #[test]
    fn full_scale_clamps_and_never_wraps() {
        for format in [SampleFormat::S32, SampleFormat::S24, SampleFormat::S24Packed, SampleFormat::S16] {
            assert!(round_trip(format, 1.0) > 0.99, "{format:?}");
            assert!(round_trip(format, 2.0) > 0.99, "{format:?}");
            assert_eq!(round_trip(format, -1.0), -1.0, "{format:?}");
            assert_eq!(round_trip(format, -5.0), -1.0, "{format:?}");
            assert_eq!(round_trip(format, f32::NAN), 0.0, "{format:?}");
        }
    }

    #[test]
    fn integer_layouts_are_little_endian() {
        let mut buf = [0u8; 4];
        SampleFormat::S16.encode(-1.0, &mut buf);
        assert_eq!(&buf[..2], &[0x00, 0x80]);
        SampleFormat::S24.encode(-1.0, &mut buf);
        assert_eq!(buf, [0x00, 0x00, 0x80, 0xff]);
        let mut packed = [0u8; 3];
        SampleFormat::S24Packed.encode(0.5, &mut packed);
        assert_eq!(packed, [0x00, 0x00, 0x40]);
        SampleFormat::S32.encode(0.5, &mut buf);
        assert_eq!(buf, [0x00, 0x00, 0x00, 0x40]);
    }

    #[test]
    fn s24_ignores_the_container_high_byte() {
        // 0x7f in the high byte, a negative 24-bit value below
        let bytes = [0x00, 0x00, 0x80, 0x7f];
        assert_eq!(SampleFormat::S24.decode(&bytes), -1.0);
    }
}
