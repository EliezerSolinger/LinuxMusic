//! The pieces of the Synthesis ToolKit (STK 5.0.1, Perry R. Cook and Gary P. Scavone, MIT-style licence, see
//! `rawwaves/LICENSE-STK`) that the legacy Mallets plays, ported to Rust: the struck bars (`ModalBar` on `Modal`), the
//! tubular bells (`TubeBell` on `FM`) and the banded waveguides (`BandedWG`), with the generators and filters they are
//! made of.
//!
//! Every computation is the library's, in the same order and in double precision, so that at the reference rate
//! ([`REFERENCE_RATE`], 44.1 kHz, the legacy program's default) the port gives the legacy samples (the A/B in `ab/`).
//! What differs from the library, on purpose:
//!
//! - **the sample rate is the instance's** ([`Rate`]), never a global: STK keeps it in a static (`Stk::sampleRate`),
//!   which breaks two instances at two rates and needs a lock to change;
//! - **the wave files are data in the program** ([`rawwaves`]), decoded once; STK opened them from a folder when a
//!   voice was built (on the audio thread, in the legacy program, when its spare voices ran out);
//! - **the same sound at any rate** (U36): the library has constants per sample where it means per second (the bars'
//!   pole radii, the strike's playback rate, the stick's low-pass, the bells' feedback differentiator, the bow's
//!   attack and its velocity integrator, the pluck's pulse). Each is scaled by `44100 / rate` so that it means at any
//!   rate what it means at 44.1 kHz; at 44.1 kHz every scale is exactly 1 and the computation is the library's.

pub mod banded_wg;
pub mod filters;
pub mod modal_bar;
pub mod rawwaves;
pub mod tube_bell;

/// STK's `PI` (`Stk.h`: a truncated value, kept for the same coefficients)
#[allow(clippy::approx_constant)]
pub const PI: f64 = 3.14159265358979;
/// STK's `TWO_PI`
pub const TWO_PI: f64 = 2.0 * PI;
/// STK's `ONE_OVER_128`
pub const ONE_OVER_128: f64 = 0.0078125;
/// The rate the library's per-sample constants are kept at (the legacy program's default rate)
pub const REFERENCE_RATE: f64 = 44100.0;

/// The rate an instrument runs at, and what its per-sample constants are scaled by
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    /// Hz (`Stk::sampleRate()`)
    pub hz: f64,
    /// `REFERENCE_RATE / hz`: 1 at 44.1 kHz, where nothing is scaled
    pub k: f64,
}

impl Rate {
    pub fn new(hz: f64) -> Self {
        Self { hz, k: REFERENCE_RATE / hz }
    }

    /// Whether nothing is scaled (44.1 kHz)
    pub fn is_reference(&self) -> bool {
        self.k == 1.0
    }

    /// A per-sample decay factor (a pole radius) that lasts as long at this rate as `r` does at 44.1 kHz
    pub fn radius(&self, r: f64) -> f64 {
        if self.is_reference() || r <= 0.0 { r } else { lm_dsp::math::pow(r, self.k) }
    }

    /// A per-sample increment (a playback rate, an envelope's rate) that covers as much per second as `x` does at
    /// 44.1 kHz
    pub fn per_sample(&self, x: f64) -> f64 {
        if self.is_reference() { x } else { x * self.k }
    }
}
