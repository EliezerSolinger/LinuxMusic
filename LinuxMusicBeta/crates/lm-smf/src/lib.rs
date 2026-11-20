//! Standard MIDI Files (F-PRJ-019, F-PRJ-021): reading SMF format 0 and 1 (and the RIFF RMID wrapper) and writing
//! format 1, in pure Rust, without dependencies.
//!
//! Reading keeps what the program uses — notes, controllers, program changes, pitch bend, channel and poly pressure,
//! tempo, time signature, track and instrument names — with each event's absolute tick in the file's resolution.
//! SysEx and the other meta events are skipped. Errors are keys (`smf.*`), never text.

#![forbid(unsafe_code)]

use std::fmt;

/// A failure, as a translation key with the byte offset where it happened
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmfError {
    pub key: &'static str,
    pub at: usize,
}

impl fmt::Display for SmfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.key, self.at)
    }
}

impl std::error::Error for SmfError {}

pub type Result<T> = std::result::Result<T, SmfError>;

/// One event, without its time
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    NoteOn {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    Controller {
        channel: u8,
        number: u8,
        value: u8,
    },
    Program {
        channel: u8,
        program: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
    ChannelPressure {
        channel: u8,
        value: u8,
    },
    PolyPressure {
        channel: u8,
        key: u8,
        value: u8,
    },
    /// Microseconds per quarter note
    Tempo(u32),
    /// Numerator and denominator (4/4 is (4, 4))
    TimeSignature(u8, u8),
    TrackName(String),
    InstrumentName(String),
    Marker(String),
}

/// An event at its absolute tick
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timed {
    pub tick: u64,
    pub event: Event,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Track {
    pub events: Vec<Timed>,
}

impl Track {
    pub fn name(&self) -> Option<&str> {
        self.events.iter().find_map(|e| match &e.event {
            Event::TrackName(n) => Some(n.as_str()),
            _ => None,
        })
    }
}

/// A whole file: its format (0, 1 or 2), its resolution in ticks per quarter note, its tracks
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Smf {
    pub format: u16,
    pub ppq: u16,
    pub tracks: Vec<Track>,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn err(&self, key: &'static str) -> SmfError {
        SmfError { key, at: self.at }
    }
    fn byte(&mut self) -> Result<u8> {
        let b = *self.data.get(self.at).ok_or_else(|| self.err("smf.truncated"))?;
        self.at += 1;
        Ok(b)
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.data.len()).ok_or_else(|| self.err("smf.truncated"))?;
        let s = &self.data[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// A variable-length quantity (at most 4 bytes, 28 bits)
    fn vlq(&mut self) -> Result<u32> {
        let mut v: u32 = 0;
        for _ in 0..4 {
            let b = self.byte()?;
            v = (v << 7) | u32::from(b & 0x7f);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(self.err("smf.bad_length"))
    }
}

/// Reads a file's bytes: SMF ("MThd") or RIFF RMID (an SMF inside a RIFF "data" chunk)
pub fn read(data: &[u8]) -> Result<Smf> {
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"RMID") {
        let mut r = Reader { data, at: 12 };
        while r.at + 8 <= data.len() {
            let id = r.bytes(4)?;
            let len = u32::from_le_bytes(r.bytes(4)?.try_into().map_err(|_| r.err("smf.truncated"))?) as usize;
            if id == b"data" {
                return read_smf(r.bytes(len)?);
            }
            r.bytes(len + (len & 1))?;
        }
        return Err(SmfError { key: "smf.rmid_without_data", at: data.len() });
    }
    read_smf(data)
}

fn read_smf(data: &[u8]) -> Result<Smf> {
    let mut r = Reader { data, at: 0 };
    if r.bytes(4)? != b"MThd" {
        return Err(SmfError { key: "smf.not_midi", at: 0 });
    }
    let len = r.u32()? as usize;
    if len < 6 {
        return Err(r.err("smf.bad_header"));
    }
    let format = r.u16()?;
    let ntracks = r.u16()?;
    let division = r.u16()?;
    r.bytes(len - 6)?;
    if format > 2 {
        return Err(r.err("smf.unknown_format"));
    }
    if division & 0x8000 != 0 {
        // SMPTE time: frames per second and ticks per frame; the program works in quarter notes
        return Err(r.err("smf.smpte_time"));
    }
    if division == 0 {
        return Err(r.err("smf.bad_header"));
    }
    let mut tracks = Vec::with_capacity(ntracks as usize);
    while tracks.len() < ntracks as usize && r.at < data.len() {
        let id = r.bytes(4)?;
        let len = r.u32()? as usize;
        let body = r.bytes(len)?;
        if id == b"MTrk" {
            tracks.push(read_track(body, r.at - len)?);
        }
        // unknown chunks are skipped (the standard says so)
    }
    Ok(Smf { format, ppq: division, tracks })
}

fn text(b: &[u8]) -> String {
    // Latin-1 is what most files carry; valid UTF-8 is taken as it is
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|c| char::from(*c)).collect(),
    }
}

fn read_track(data: &[u8], base: usize) -> Result<Track> {
    let mut r = Reader { data, at: 0 };
    let mut tick: u64 = 0;
    let mut status: u8 = 0;
    let mut events = Vec::new();
    let fix = |mut e: SmfError| {
        e.at += base;
        e
    };
    while r.at < data.len() {
        tick += u64::from(r.vlq().map_err(fix)?);
        let mut first = r.byte().map_err(fix)?;
        if first & 0x80 != 0 {
            status = first;
            if status < 0xf0 {
                first = r.byte().map_err(fix)?;
            }
        } else if status == 0 || status >= 0xf0 {
            return Err(fix(r.err("smf.running_status_without_status")));
        }
        let channel = status & 0x0f;
        let event = match status & 0xf0 {
            0x80 => Some(Event::NoteOff { channel, key: first & 0x7f, velocity: r.byte().map_err(fix)? & 0x7f }),
            0x90 => {
                let velocity = r.byte().map_err(fix)? & 0x7f;
                // a note-on with velocity 0 is a note-off
                Some(if velocity == 0 {
                    Event::NoteOff { channel, key: first & 0x7f, velocity: 64 }
                } else {
                    Event::NoteOn { channel, key: first & 0x7f, velocity }
                })
            }
            0xa0 => Some(Event::PolyPressure { channel, key: first & 0x7f, value: r.byte().map_err(fix)? & 0x7f }),
            0xb0 => Some(Event::Controller { channel, number: first & 0x7f, value: r.byte().map_err(fix)? & 0x7f }),
            0xc0 => Some(Event::Program { channel, program: first & 0x7f }),
            0xd0 => Some(Event::ChannelPressure { channel, value: first & 0x7f }),
            0xe0 => {
                let msb = r.byte().map_err(fix)? & 0x7f;
                Some(Event::PitchBend { channel, value: u16::from(first & 0x7f) | (u16::from(msb) << 7) })
            }
            _ => match status {
                0xff => {
                    let kind = r.byte().map_err(fix)?;
                    let len = r.vlq().map_err(fix)? as usize;
                    let body = r.bytes(len).map_err(fix)?;
                    status = 0; // a meta event cancels the running status
                    match kind {
                        0x2f => break,
                        0x51 if len == 3 => Some(Event::Tempo(
                            (u32::from(body[0]) << 16) | (u32::from(body[1]) << 8) | u32::from(body[2]),
                        )),
                        0x58 if len >= 2 => {
                            Some(Event::TimeSignature(body[0], 1u8.checked_shl(u32::from(body[1])).unwrap_or(4)))
                        }
                        0x03 => Some(Event::TrackName(text(body))),
                        0x04 => Some(Event::InstrumentName(text(body))),
                        0x06 => Some(Event::Marker(text(body))),
                        _ => None,
                    }
                }
                0xf0 | 0xf7 => {
                    // SysEx: skipped (it does not record, I-21.1)
                    let len = r.vlq().map_err(fix)? as usize;
                    r.bytes(len).map_err(fix)?;
                    status = 0;
                    None
                }
                _ => return Err(fix(r.err("smf.unknown_event"))),
            },
        };
        if let Some(event) = event {
            events.push(Timed { tick, event });
        }
    }
    Ok(Track { events })
}

fn put_vlq(out: &mut Vec<u8>, mut v: u32) {
    let mut buf = [0u8; 4];
    let mut n = 0;
    loop {
        buf[n] = (v & 0x7f) as u8;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(buf[i] | if i > 0 { 0x80 } else { 0 });
    }
}

fn put_meta(out: &mut Vec<u8>, kind: u8, body: &[u8]) {
    out.push(0xff);
    out.push(kind);
    put_vlq(out, body.len() as u32);
    out.extend_from_slice(body);
}

/// Writes a format-1 file (format 0 when there is one track). Each track's events are sorted by tick (stably);
/// note-offs come before note-ons on the same tick, so a retrigger works (U26)
pub fn write(smf: &Smf) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    let format: u16 = if smf.tracks.len() == 1 { 0 } else { 1 };
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&(smf.tracks.len() as u16).to_be_bytes());
    out.extend_from_slice(&smf.ppq.to_be_bytes());
    for t in &smf.tracks {
        let mut events = t.events.clone();
        events.sort_by_key(|e| (e.tick, !matches!(e.event, Event::NoteOff { .. })));
        let mut body = Vec::new();
        let mut last = 0u64;
        for e in &events {
            put_vlq(&mut body, (e.tick - last).min(0x0fff_ffff) as u32);
            last = e.tick;
            match &e.event {
                Event::NoteOn { channel, key, velocity } => body.extend_from_slice(&[0x90 | channel, *key, *velocity]),
                Event::NoteOff { channel, key, velocity } => body.extend_from_slice(&[0x80 | channel, *key, *velocity]),
                Event::Controller { channel, number, value } => {
                    body.extend_from_slice(&[0xb0 | channel, *number, *value])
                }
                Event::Program { channel, program } => body.extend_from_slice(&[0xc0 | channel, *program]),
                Event::PitchBend { channel, value } => {
                    body.extend_from_slice(&[0xe0 | channel, (value & 0x7f) as u8, ((value >> 7) & 0x7f) as u8])
                }
                Event::ChannelPressure { channel, value } => body.extend_from_slice(&[0xd0 | channel, *value]),
                Event::PolyPressure { channel, key, value } => body.extend_from_slice(&[0xa0 | channel, *key, *value]),
                Event::Tempo(us) => put_meta(&mut body, 0x51, &us.to_be_bytes()[1..]),
                Event::TimeSignature(n, d) => {
                    let pow = d.trailing_zeros() as u8;
                    put_meta(&mut body, 0x58, &[*n, pow, 24, 8]);
                }
                Event::TrackName(n) => put_meta(&mut body, 0x03, n.as_bytes()),
                Event::InstrumentName(n) => put_meta(&mut body, 0x04, n.as_bytes()),
                Event::Marker(n) => put_meta(&mut body, 0x06, n.as_bytes()),
            }
        }
        put_vlq(&mut body, 0);
        put_meta(&mut body, 0x2f, &[]);
        out.extend_from_slice(b"MTrk");
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
    }
    out
}

#[cfg(test)]
mod tests;
