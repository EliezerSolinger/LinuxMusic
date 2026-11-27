//! Frames on the stream (I-20.2): a 5-byte header — the payload's length (`u32`, little endian) and its kind — then
//! the payload. Kind 0 is a JSON message (the protocol's own reader and writer, `lm-state`); the dense data the
//! screen draws is binary (I-20.4): telemetry (meters as quantized dB in `u16`), peak blocks (`i16` pairs), plugin
//! displays. Every reader is bounded and checked: a malformed frame is an error, never a panic.

use std::io::{Read, Write};
use std::sync::Arc;

use lm_base::{Error, HostTime, Result};
use lm_port::{METER_CHANNELS, Meter, NodeId, PeakBlock, Position, TelemetryView};
use lm_state::Value;

pub const KIND_JSON: u8 = 0;
pub const KIND_TELEMETRY: u8 = 1;
pub const KIND_PEAKS: u8 = 2;
pub const KIND_DISPLAY: u8 = 3;
/// Client to server: a piece of an upload (`u64` upload, `u64` offset, the bytes)
pub const KIND_UPLOAD: u8 = 4;

/// The largest frame a client accepts (a whole project's snapshot is the biggest)
pub const MAX_FRAME_FROM_SERVER: usize = 512 << 20;
/// The largest frame a server accepts from a client (I-20.6: limits per client)
pub const MAX_FRAME_FROM_CLIENT: usize = 32 << 20;

/// A frame read from the stream
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: u8,
    pub payload: Vec<u8>,
}

fn io(e: &std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe => {
            Error::new("remote.disconnected")
        }
        _ => Error::new("remote.io").arg("error", e.to_string()),
    }
}

/// Reads one frame (blocking). `remote.frame_too_big` past `max`
pub fn read_frame(r: &mut impl Read, max: usize) -> Result<Frame> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head).map_err(|e| io(&e))?;
    let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if len > max {
        return Err(Error::new("remote.frame_too_big").arg("bytes", len as i64).arg("max", max as i64));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).map_err(|e| io(&e))?;
    Ok(Frame { kind: head[4], payload })
}

pub fn write_frame(w: &mut impl Write, kind: u8, payload: &[u8]) -> Result<()> {
    let len = u32::try_from(payload.len()).map_err(|_| Error::new("remote.frame_too_big"))?;
    let mut head = [0u8; 5];
    head[..4].copy_from_slice(&len.to_le_bytes());
    head[4] = kind;
    w.write_all(&head).map_err(|e| io(&e))?;
    w.write_all(payload).map_err(|e| io(&e))?;
    w.flush().map_err(|e| io(&e))
}

/// A JSON message's frame payload
pub fn json(v: &Value) -> Result<Vec<u8>> {
    Ok(lm_state::to_json_compact(v)?.into_bytes())
}

pub fn parse_json(payload: &[u8]) -> Result<Value> {
    lm_state::from_json_bytes(payload)
}

// ---------------------------------------------------------------------------------------------------------------
// Binary

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn id(&mut self, id: NodeId) {
        self.u8(id.kind);
        self.u64(id.number);
    }
}

struct In<'a> {
    b: &'a [u8],
    at: usize,
}

fn short() -> Error {
    Error::new("remote.decode").arg("field", "binary")
}

impl<'a> In<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or_else(short)?;
        let s = self.b.get(self.at..end).ok_or_else(short)?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u64(&mut self) -> Result<u64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(self.u64()? as i64)
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn id(&mut self) -> Result<NodeId> {
        let kind = self.u8()?;
        let number = self.u64()?;
        NodeId::new(kind, number).map_err(|_| short())
    }
    /// A count of items of at least `size` bytes each, bounded by what is left
    fn count(&mut self, size: usize) -> Result<usize> {
        let n = self.u32()? as usize;
        if n.saturating_mul(size.max(1)) > self.b.len() - self.at {
            return Err(short());
        }
        Ok(n)
    }
    fn end(&self) -> Result<()> {
        if self.at == self.b.len() { Ok(()) } else { Err(short()) }
    }
}

const DB_FLOOR: f32 = -120.0;
const DB_RANGE: f32 = 144.0;

/// A linear peak as quantized dB (I-20.4): 0 is silence, then −120 dB to +24 dB in 65535 steps
pub fn quantize(peak: f32) -> u16 {
    if peak.is_nan() || peak <= 0.0 || !peak.is_finite() {
        return 0;
    }
    let db = (20.0 * peak.log10()).clamp(DB_FLOOR, DB_FLOOR + DB_RANGE);
    ((db - DB_FLOOR) / DB_RANGE * 65534.0).round() as u16 + 1
}

pub fn dequantize(q: u16) -> f32 {
    if q == 0 {
        return 0.0;
    }
    let db = f32::from(q - 1) / 65534.0 * DB_RANGE + DB_FLOOR;
    10f32.powf(db / 20.0)
}

/// The telemetry frame (only the meters of the nodes asked for; all when `only` is `None`)
pub fn telemetry_to(t: &TelemetryView, only: Option<&[NodeId]>) -> Vec<u8> {
    let mut o = Out::default();
    let p = &t.position;
    o.i64(p.samples);
    o.u64(p.host_time.0);
    o.u32(p.sample_rate);
    o.u8(u8::from(p.playing) | (u8::from(p.recording) << 1) | (u8::from(t.counting_in) << 2));
    o.u32(t.xruns);
    let marks = &t.xrun_marks[t.xrun_marks.len().saturating_sub(64)..];
    o.u32(marks.len() as u32);
    for m in marks {
        o.i64(*m);
    }
    o.u32(t.inputs.len().min(256) as u32);
    for x in t.inputs.iter().take(256) {
        o.f32(*x);
    }
    o.f32(t.load);
    let mut meters: Vec<(&NodeId, &Meter)> = match only {
        Some(ids) => ids.iter().filter_map(|id| t.meters.get_key_value(id)).collect(),
        None => t.meters.iter().collect(),
    };
    meters.sort_by_key(|(id, _)| **id);
    o.u32(meters.len() as u32);
    for (id, m) in meters {
        o.id(*id);
        let ch = (m.channels as usize).min(METER_CHANNELS);
        o.u8(ch as u8);
        o.u8(u8::from(m.dead));
        for p in &m.peak[..ch] {
            o.u16(quantize(*p));
        }
        for p in &m.rms[..ch] {
            o.u16(quantize(*p));
        }
    }
    o.0
}

/// Reads a telemetry frame into `t` (its recording takes are kept: they come as JSON)
pub fn telemetry_from(b: &[u8], t: &mut TelemetryView) -> Result<()> {
    let mut i = In::new(b);
    let samples = i.i64()?;
    let host_time = HostTime(i.u64()?);
    let sample_rate = i.u32()?;
    let flags = i.u8()?;
    let xruns = i.u32()?;
    let n = i.count(8)?;
    let mut marks = Vec::with_capacity(n);
    for _ in 0..n {
        marks.push(i.i64()?);
    }
    let n = i.count(4)?;
    let mut inputs = Vec::with_capacity(n);
    for _ in 0..n {
        inputs.push(i.f32()?);
    }
    let load = i.f32()?;
    let n = i.count(11)?;
    let mut meters = std::collections::HashMap::with_capacity(n);
    for _ in 0..n {
        let id = i.id()?;
        let ch = i.u8()? as usize;
        if ch > METER_CHANNELS {
            return Err(short());
        }
        let dead = i.u8()? != 0;
        let mut m = Meter { channels: ch as u8, dead, ..Meter::default() };
        for p in m.peak.iter_mut().take(ch) {
            *p = dequantize(i.u16()?);
        }
        for p in m.rms.iter_mut().take(ch) {
            *p = dequantize(i.u16()?);
        }
        meters.insert(id, m);
    }
    i.end()?;
    t.position = Position { samples, host_time, sample_rate, playing: flags & 1 != 0, recording: flags & 2 != 0 };
    t.counting_in = flags & 4 != 0;
    t.xruns = xruns;
    t.xrun_marks = marks;
    t.inputs = inputs;
    t.load = load;
    t.meters = meters;
    Ok(())
}

pub fn peaks_to(request: u64, p: &PeakBlock) -> Vec<u8> {
    let mut o = Out::default();
    o.u64(request);
    o.u8(p.level);
    o.u64(p.from);
    o.u64(p.samples_per_pair);
    o.u64(p.length);
    o.u32(p.pairs.len() as u32);
    for (a, b) in p.pairs.iter() {
        o.u16(*a as u16);
        o.u16(*b as u16);
    }
    o.0
}

pub fn peaks_from(b: &[u8]) -> Result<(u64, PeakBlock)> {
    let mut i = In::new(b);
    let request = i.u64()?;
    let level = i.u8()?;
    let from = i.u64()?;
    let samples_per_pair = i.u64()?;
    let length = i.u64()?;
    let n = i.count(4)?;
    let mut pairs = Vec::with_capacity(n);
    for _ in 0..n {
        pairs.push((i.u16()? as i16, i.u16()? as i16));
    }
    i.end()?;
    Ok((request, PeakBlock { level, from, samples_per_pair, pairs: Arc::from(pairs), length }))
}

/// An upload's chunk frame
pub fn upload_to(upload: u64, offset: u64, data: &[u8]) -> Vec<u8> {
    let mut o = Out::default();
    o.u64(upload);
    o.u64(offset);
    o.0.extend_from_slice(data);
    o.0
}

pub fn upload_from(b: &[u8]) -> Result<(u64, u64, &[u8])> {
    let mut i = In::new(b);
    let upload = i.u64()?;
    let offset = i.u64()?;
    Ok((upload, offset, &b[16..]))
}

pub fn display_to(node: NodeId, d: &lm_port::DisplayView) -> Vec<u8> {
    let mut o = Out::default();
    o.id(node);
    let kind = d.kind.as_bytes();
    let k = &kind[..kind.len().min(255)];
    o.u8(k.len() as u8);
    o.0.extend_from_slice(k);
    o.u32(d.frame.len() as u32);
    for x in d.frame.iter() {
        o.f32(*x);
    }
    o.0
}

pub fn display_from(b: &[u8]) -> Result<(NodeId, lm_port::DisplayView)> {
    let mut i = In::new(b);
    let node = i.id()?;
    let n = i.u8()? as usize;
    let kind = std::str::from_utf8(i.take(n)?).map_err(|_| short())?;
    let kind: Arc<str> = Arc::from(kind);
    let n = i.count(4)?;
    let mut frame = Vec::with_capacity(n);
    for _ in 0..n {
        frame.push(i.f32()?);
    }
    i.end()?;
    Ok((node, lm_port::DisplayView { kind, frame: Arc::from(frame) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_meters() {
        let mut buf = Vec::new();
        write_frame(&mut buf, KIND_JSON, b"{}").unwrap();
        let f = read_frame(&mut buf.as_slice(), 10).unwrap();
        assert_eq!(f, Frame { kind: 0, payload: b"{}".to_vec() });
        assert_eq!(read_frame(&mut buf.as_slice(), 1).unwrap_err().key(), "remote.frame_too_big");
        assert_eq!(read_frame(&mut &buf[..3], 10).unwrap_err().key(), "remote.disconnected");
        for p in [0.0, 1e-9, 0.001, 0.5, 1.0, 8.0] {
            let q = dequantize(quantize(p));
            if p < 1e-6 {
                assert!(q <= 1.001e-6, "below the floor: at most −120 dB");
            } else {
                assert!((20.0 * (q / p).log10()).abs() < 0.01, "{p} -> {q}");
            }
        }
    }
}
