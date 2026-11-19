//! The processor and its buffers (interfaces.md, I-9.2, S4, D12; U23).
//!
//! A processor is built on the main thread (or in an `Operation`), where it allocates everything it will ever need,
//! and then moves to the audio side inside an [`crate::Install`]. There it only processes: no allocation, no lock, no
//! I/O. Its buffers and events are lent for one call; the lifetimes stop it from keeping them

use std::any::Any;
use std::fmt;

use lm_time::TimeSignature;

use crate::event::Event;
use crate::rng::Rng;

/// A sound processor: an instrument, an insert, the desk rack, or one of the host's own (`crate::builtin`).
///
/// `Send` but not `Sync`: it lives on one render thread at a time
pub trait Processor: Send {
    /// Renders `cx.frames` samples. Outputs come zeroed (an in-place channel holds its input) (U23)
    fn process(&mut self, cx: &mut ProcessContext<'_>);

    /// Clears the sound state (voices, tails, delay memories). Called by the panic command
    fn reset(&mut self) {}

    /// Latency in samples, read only when the processor is built; a change later goes through
    /// [`PluginRequest::LatencyChanged`] and a recompilation (U23)
    fn latency(&self) -> u32 {
        0
    }

    /// Takes over the live state of the processor it replaces (voices, tails), when it is the same plugin and the
    /// same major version (R18). `No` sends the old one to a tail node, if the compiler made one (U32)
    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let _ = old;
        Adopted::No
    }

    /// For [`Processor::adopt`]: the concrete type behind the trait object
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl fmt::Debug for dyn Processor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("dyn Processor")
    }
}

/// The answer of [`Processor::adopt`] (U32)
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adopted {
    /// The new processor carries on the old one's sound; the old one is returned
    Yes,
    /// The old processor's notes are released and it keeps sounding in a tail node, if the install names one
    No,
}

/// Where the music is, for the block being processed
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransportInfo {
    pub sample_rate: u32,
    pub playing: bool,
    pub recording: bool,
    /// The timeline sample of offset 0 (the playhead; it does not move while stopped)
    pub position: i64,
    /// The tick reached at `position` (lm-time's `tick_at`); LFOs synced to the tempo derive their phase from it (U37)
    pub tick: i64,
    pub ppq: u32,
    /// The tempo at `position`, for display and synced effects (never for a conversion)
    pub bpm: f64,
    pub time_signature: TimeSignature,
    /// Samples rendered since the engine started; never jumps, also while stopped
    pub frame: u64,
}

/// One node's audio for one call: a single contiguous block allocated with the program (S4), channel after channel,
/// each `stride` samples apart. No array of slices is built per block
pub struct Buffers<'b> {
    data: &'b mut [f32],
    stride: usize,
    frames: usize,
    inputs: usize,
    outputs: usize,
    sidechain: usize,
    in_place: bool,
}

impl fmt::Debug for Buffers<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Buffers")
            .field("frames", &self.frames)
            .field("inputs", &self.inputs)
            .field("outputs", &self.outputs)
            .field("sidechain", &self.sidechain)
            .field("in_place", &self.in_place)
            .finish()
    }
}

impl<'b> Buffers<'b> {
    /// Layout of `data`: inputs, then outputs, then sidechain channels; an in-place node has no separate inputs (its
    /// outputs hold the input when it is called)
    pub(crate) fn new(data: &'b mut [f32], layout: BufferLayout, frames: usize) -> Self {
        debug_assert!(frames <= layout.stride);
        debug_assert!(data.len() >= layout.len());
        Self {
            data,
            stride: layout.stride,
            frames,
            inputs: if layout.in_place { 0 } else { layout.inputs },
            outputs: layout.outputs,
            sidechain: layout.sidechain,
            in_place: layout.in_place,
        }
    }

    /// A block for a host other than the renderer (a plugin's test bench, an offline render of one plugin): `data`
    /// holds the input channels (none when `in_place`), then the output channels, then the sidechain channels, each
    /// `stride` samples apart, as the renderer lays a node's block out. On an in-place block the outputs must already
    /// hold the input; every output must be zeroed by the caller otherwise (U23)
    pub fn from_block(data: &'b mut [f32], stride: usize, frames: usize, shape: BlockShape) -> Self {
        let layout = BufferLayout {
            inputs: shape.inputs,
            outputs: shape.outputs,
            sidechain: shape.sidechain,
            in_place: shape.in_place,
            stride,
        };
        assert!(frames <= stride, "{frames} frames in a stride of {stride}");
        assert!(data.len() >= layout.len(), "a block of {} samples for {} needed", data.len(), layout.len());
        assert!(!shape.in_place || shape.inputs == shape.outputs, "an in-place block has as many inputs as outputs");
        Self::new(data, layout, frames)
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Input channels (0 on an in-place node: use [`Buffers::in_out`])
    pub fn inputs(&self) -> usize {
        self.inputs
    }

    pub fn outputs(&self) -> usize {
        self.outputs
    }

    pub fn sidechains(&self) -> usize {
        self.sidechain
    }

    pub fn is_in_place(&self) -> bool {
        self.in_place
    }

    fn range(&self, first_channel: usize) -> std::ops::Range<usize> {
        let start = first_channel * self.stride;
        start..start + self.frames
    }

    /// Input channel `ch`
    pub fn input(&self, ch: usize) -> &[f32] {
        assert!(ch < self.inputs, "input {ch} of {}", self.inputs);
        &self.data[self.range(ch)]
    }

    /// Output channel `ch`, zeroed before the call (U23)
    pub fn output(&mut self, ch: usize) -> &mut [f32] {
        assert!(ch < self.outputs, "output {ch} of {}", self.outputs);
        let r = self.range(self.inputs + ch);
        &mut self.data[r]
    }

    /// Channel `ch` in place: it holds the input and becomes the output. Only on an in-place node; on another node
    /// it is the (zeroed) output
    pub fn in_out(&mut self, ch: usize) -> &mut [f32] {
        self.output(ch)
    }

    /// Sidechain channel `ch`, if the node has it
    pub fn sidechain(&self, ch: usize) -> Option<&[f32]> {
        (ch < self.sidechain).then(|| &self.data[self.range(self.inputs + self.outputs + ch)])
    }

    /// Everything at once: the inputs (read), the outputs (written) and the sidechain (read)
    pub fn split(&mut self) -> (Channels<'_>, ChannelsMut<'_>, Channels<'_>) {
        let (ins, rest) = self.data.split_at_mut(self.inputs * self.stride);
        let (outs, side) = rest.split_at_mut(self.outputs * self.stride);
        (
            Channels { data: ins, stride: self.stride, frames: self.frames, count: self.inputs },
            ChannelsMut { data: outs, stride: self.stride, frames: self.frames, count: self.outputs },
            Channels { data: side, stride: self.stride, frames: self.frames, count: self.sidechain },
        )
    }
}

/// Read-only channels of a [`Buffers`]
#[derive(Debug)]
pub struct Channels<'a> {
    data: &'a [f32],
    stride: usize,
    frames: usize,
    count: usize,
}

impl Channels<'_> {
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get(&self, ch: usize) -> &[f32] {
        assert!(ch < self.count, "channel {ch} of {}", self.count);
        &self.data[ch * self.stride..ch * self.stride + self.frames]
    }
}

/// Writable channels of a [`Buffers`]
#[derive(Debug)]
pub struct ChannelsMut<'a> {
    data: &'a mut [f32],
    stride: usize,
    frames: usize,
    count: usize,
}

impl ChannelsMut<'_> {
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get_mut(&mut self, ch: usize) -> &mut [f32] {
        assert!(ch < self.count, "channel {ch} of {}", self.count);
        &mut self.data[ch * self.stride..ch * self.stride + self.frames]
    }

    /// Two different channels at once (`a != b`)
    pub fn pair_mut(&mut self, a: usize, b: usize) -> (&mut [f32], &mut [f32]) {
        assert!(a != b && a < self.count && b < self.count, "channels {a} and {b} of {}", self.count);
        let frames = self.frames;
        let (lo, hi, swap) = if a < b { (a, b, false) } else { (b, a, true) };
        let (first, second) = self.data.split_at_mut(hi * self.stride);
        let x = &mut first[lo * self.stride..lo * self.stride + frames];
        let y = &mut second[..frames];
        if swap { (y, x) } else { (x, y) }
    }

    /// Every channel, in order
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut [f32]> {
        let frames = self.frames;
        self.data.chunks_mut(self.stride).take(self.count).map(move |c| &mut c[..frames])
    }
}

/// The channels of a block made with [`Buffers::from_block`]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockShape {
    pub inputs: usize,
    pub outputs: usize,
    pub sidechain: usize,
    /// Inputs and outputs are the same channels (an insert): `inputs == outputs`
    pub in_place: bool,
}

impl BlockShape {
    fn separate_inputs(&self) -> usize {
        if self.in_place { 0 } else { self.inputs }
    }

    /// The samples a block of this shape needs with `stride` samples per channel
    pub fn len(&self, stride: usize) -> usize {
        (self.separate_inputs() + self.outputs + self.sidechain) * stride
    }

    /// Where input channel `ch` starts (on an in-place block, the output channel that holds it)
    pub fn input_offset(&self, ch: usize, stride: usize) -> usize {
        ch * stride
    }

    /// Where output channel `ch` starts
    pub fn output_offset(&self, ch: usize, stride: usize) -> usize {
        (self.separate_inputs() + ch) * stride
    }

    /// Where sidechain channel `ch` starts
    pub fn sidechain_offset(&self, ch: usize, stride: usize) -> usize {
        (self.separate_inputs() + self.outputs + ch) * stride
    }
}

/// The shape of a node's block
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BufferLayout {
    pub(crate) inputs: usize,
    pub(crate) outputs: usize,
    pub(crate) sidechain: usize,
    pub(crate) in_place: bool,
    pub(crate) stride: usize,
}

impl BufferLayout {
    pub(crate) fn input_channels(&self) -> usize {
        if self.in_place { 0 } else { self.inputs }
    }

    pub(crate) fn len(&self) -> usize {
        (self.input_channels() + self.outputs + self.sidechain) * self.stride
    }

    /// Where input channel `ch` is written by the host (on an in-place node, the output channel)
    pub(crate) fn input_offset(&self, ch: usize) -> usize {
        ch * self.stride
    }

    pub(crate) fn output_offset(&self, ch: usize) -> usize {
        (self.input_channels() + ch) * self.stride
    }

    pub(crate) fn sidechain_offset(&self, ch: usize) -> usize {
        (self.input_channels() + self.outputs + ch) * self.stride
    }
}

/// What a processor gets for one call
pub struct ProcessContext<'b> {
    pub frames: usize,
    pub buffers: Buffers<'b>,
    pub events: &'b [Event],
    pub transport: &'b TransportInfo,
    pub rng: &'b mut Rng,
    /// Requests to the host (F2, I-9.4)
    pub host: &'b mut HostOutbox,
    /// Live data for the plugin's window (F2, I-9.5)
    pub display: Option<&'b mut DisplayWriter>,
}

impl fmt::Debug for ProcessContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessContext")
            .field("frames", &self.frames)
            .field("buffers", &self.buffers)
            .field("events", &self.events.len())
            .field("transport", self.transport)
            .finish()
    }
}

/// A translatable error key of a plugin (rule 7: never ready text)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginErrorKey(pub &'static str);

/// A plugin asking the host for something (F2, I-9.4)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PluginRequest {
    LatencyChanged,
    ParamChanged {
        index: u32,
        value: f32,
    },
    Error(PluginErrorKey),
    /// Third-party plugins only (S18)
    StateChanged,
}

/// A processor's requests in one call; at most [`HostOutbox::CAPACITY`], the rest are counted as dropped
#[derive(Debug, Default)]
pub struct HostOutbox {
    requests: [Option<PluginRequest>; HostOutbox::CAPACITY],
    len: usize,
    dropped: u32,
}

impl HostOutbox {
    pub const CAPACITY: usize = 8;

    /// Queues a request; `false` if this call already made [`HostOutbox::CAPACITY`] of them
    pub fn request(&mut self, request: PluginRequest) -> bool {
        if self.len == Self::CAPACITY {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        self.requests[self.len] = Some(request);
        self.len += 1;
        true
    }

    pub(crate) fn drain(&mut self) -> impl Iterator<Item = PluginRequest> + '_ {
        let len = std::mem::take(&mut self.len);
        self.requests[..len].iter_mut().filter_map(Option::take)
    }

    pub(crate) fn take_dropped(&mut self) -> u32 {
        std::mem::take(&mut self.dropped)
    }
}

/// The audio side of a plugin's live display (F2, I-9.5): a fixed-size byte frame in a triple buffer, allocated with
/// the processor. The format of the bytes is the plugin's
pub struct DisplayWriter {
    input: triple_buffer::Input<Box<[u8]>>,
}

/// The window side of a plugin's live display: the last complete frame
pub struct DisplayReader {
    output: triple_buffer::Output<Box<[u8]>>,
}

impl fmt::Debug for DisplayWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DisplayWriter")
    }
}

impl fmt::Debug for DisplayReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DisplayReader")
    }
}

/// A display channel of `bytes` bytes per frame
pub fn display_channel(bytes: usize) -> (DisplayWriter, DisplayReader) {
    let (input, output) = triple_buffer::triple_buffer(&vec![0u8; bytes].into_boxed_slice());
    (DisplayWriter { input }, DisplayReader { output })
}

impl DisplayWriter {
    /// Fills a frame in place and publishes it. The buffer holds an older frame: write all of it
    pub fn write(&mut self, fill: impl FnOnce(&mut [u8])) {
        fill(self.input.input_buffer_mut());
        self.input.publish();
    }
}

impl DisplayReader {
    /// The newest complete frame
    pub fn read(&mut self) -> &[u8] {
        self.output.read()
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn split_and_pairs() {
        let layout = BufferLayout { inputs: 2, outputs: 2, sidechain: 1, in_place: false, stride: 4 };
        let mut data = vec![0.0f32; layout.len()];
        data[layout.input_offset(1)] = 1.0;
        data[layout.sidechain_offset(0)] = 7.0;
        let mut b = Buffers::new(&mut data, layout, 3);
        assert_eq!(b.input(1)[0], 1.0);
        assert_eq!(b.sidechain(0).map(|s| s[0]), Some(7.0));
        assert_eq!(b.sidechain(1), None);
        {
            let (ins, mut outs, _) = b.split();
            let (l, r) = outs.pair_mut(1, 0);
            l[0] = ins.get(1)[0] * 2.0;
            r[2] = 3.0;
            assert_eq!(l.len(), 3);
        }
        assert_eq!(b.output(1)[0], 2.0);
        assert_eq!(b.output(0)[2], 3.0);
    }

    #[test]
    fn a_block_from_outside_the_renderer() {
        let shape = BlockShape { inputs: 1, outputs: 2, sidechain: 1, in_place: false };
        let mut data = vec![0.0f32; shape.len(8)];
        data[shape.input_offset(0, 8)] = 1.0;
        data[shape.sidechain_offset(0, 8) + 2] = 5.0;
        let mut b = Buffers::from_block(&mut data, 8, 4, shape);
        assert_eq!(b.input(0)[0], 1.0);
        assert_eq!(b.sidechain(0).map(|s| s[2]), Some(5.0));
        b.output(1)[3] = 2.0;
        assert_eq!(b.output(1).len(), 4);
        assert_eq!(data[shape.output_offset(1, 8) + 3], 2.0);
        let insert = BlockShape { inputs: 2, outputs: 2, sidechain: 0, in_place: true };
        let mut data = vec![0.5f32; insert.len(4)];
        let mut b = Buffers::from_block(&mut data, 4, 4, insert);
        assert_eq!(b.inputs(), 0);
        assert_eq!(b.in_out(1)[0], 0.5);
    }

    #[test]
    fn outbox_is_bounded() {
        let mut o = HostOutbox::default();
        for _ in 0..10 {
            o.request(PluginRequest::LatencyChanged);
        }
        assert_eq!(o.drain().count(), HostOutbox::CAPACITY);
        assert_eq!(o.take_dropped(), 2);
        assert_eq!(o.drain().count(), 0);
    }

    #[test]
    fn display_frames() {
        let (mut w, mut r) = display_channel(4);
        w.write(|b| b.copy_from_slice(&[1, 2, 3, 4]));
        assert_eq!(r.read(), &[1, 2, 3, 4]);
    }
}
