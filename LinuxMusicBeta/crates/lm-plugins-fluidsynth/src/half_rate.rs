//! FluidSynth renders at 8 to 96 kHz (decisoes-autonomas.md, "Das ligações de mídia"). Above 96 kHz (176.4 and
//! 192 kHz) the synthesizer runs at half the project's rate and its output is upsampled two times with `lm-dsp`'s
//! half-band polyphase IIR (HIIR). Each frame of the synthesizer makes two of the project's: the result depends only
//! on the project's frame count, never on the block sizes (V3), and an event at a project frame `t` reaches the
//! synthesizer at its frame ⌈t / 2⌉.
//!
//! (The legacy program set FluidSynth's rate to the engine's, which FluidSynth clamps to 96 kHz: at 192 kHz the
//! AvlDrums played an octave high and twice as fast. The Sf2 Player resampled linearly from 96 kHz.)

use lm_dsp::oversampling::VariableUpsampler;

/// The highest rate FluidSynth accepts
pub const MAX_RATE: u32 = 96_000;

/// The rate the synthesizer runs at for a project at `rate`, and whether it is half of it. `None`: above 192 kHz
pub fn synth_rate(rate: u32) -> Option<(u32, bool)> {
    if rate <= MAX_RATE {
        Some((rate, false))
    } else if rate / 2 <= MAX_RATE && rate.is_multiple_of(2) {
        Some((rate / 2, true))
    } else {
        None
    }
}

/// Renders the synthesizer's channels into the project's, at the project's rate
#[derive(Debug)]
pub struct Renderer {
    half: Option<Half>,
}

#[derive(Debug)]
struct Half {
    up: Vec<VariableUpsampler>,
    /// The second sample of the last synthesizer frame, not yet handed out
    carry: Vec<f32>,
    has_carry: bool,
    scratch: Vec<Vec<f32>>,
}

/// The most channels a synthesizer renders (FluidSynth's 32 stereo outputs)
pub const MAX_CHANNELS: usize = 64;

impl Renderer {
    /// For `channels` channels at the project's `rate`, in blocks of at most `max_block` frames. Allocates
    pub fn new(rate: u32, channels: usize, max_block: usize) -> Self {
        let half = match synth_rate(rate) {
            Some((r, true)) => Some(Half {
                up: (0..channels).map(|_| VariableUpsampler::new(1, r as f32)).collect(),
                carry: vec![0.0; channels],
                has_carry: false,
                scratch: vec![vec![0.0; max_block.div_ceil(2) + 1]; channels],
            }),
            _ => None,
        };
        Self { half }
    }

    /// Whether the synthesizer runs at half the rate
    pub fn is_half(&self) -> bool {
        self.half.is_some()
    }

    /// Writes the next frames of every channel of `outs` (all of the same length, at most the `max_block` given to
    /// `new`), asking `synth` for its own frames (it writes every channel of the slices it gets, which all have the
    /// same length). Neither allocates nor locks
    pub fn render(&mut self, outs: &mut [&mut [f32]], mut synth: impl FnMut(&mut [&mut [f32]])) {
        let Some(n) = outs.first().map(|o| o.len()) else { return };
        if n == 0 {
            return;
        }
        let Some(h) = self.half.as_mut() else {
            synth(outs);
            return;
        };
        let channels = outs.len().min(h.up.len());
        let mut at = 0;
        if h.has_carry {
            for (c, out) in outs.iter_mut().enumerate().take(channels) {
                out[0] = h.carry[c];
            }
            h.has_carry = false;
            at = 1;
        }
        let frames = (n - at).div_ceil(2);
        if frames == 0 {
            return;
        }
        {
            let mut it = h.scratch.iter_mut();
            let mut refs: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|i| {
                if i < channels { it.next().map(|v| &mut v[..frames]).unwrap_or_default() } else { Default::default() }
            });
            synth(&mut refs[..channels]);
        }
        let mut two = [0.0f32; 2];
        for (c, out) in outs.iter_mut().enumerate().take(channels) {
            let (up, input) = (&mut h.up[c], &h.scratch[c]);
            for (j, x) in input[..frames].iter().enumerate() {
                up.process(&mut two, *x);
                let k = at + 2 * j;
                out[k] = two[0];
                if k + 1 < n {
                    out[k + 1] = two[1];
                } else {
                    h.carry[c] = two[1];
                }
            }
        }
        h.has_carry = (n - at) % 2 == 1;
    }

    /// Forgets what the filters hold (back to rest)
    pub fn reset(&mut self) {
        if let Some(h) = self.half.as_mut() {
            h.up.iter_mut().for_each(VariableUpsampler::reset);
            h.has_carry = false;
        }
    }
}
