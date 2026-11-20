//! MIDI mapping in the renderer (interfaces.md, I-21.2): the compiled table that turns mapped MIDI into `Param`
//! events at the message's sample, and the events that go to the session (fields, actions, the end of a MIDI
//! gesture), and what is recorded (I-21.4).
//!
//! The table comes in an [`crate::Install`] (with a program, or alone: following the selection and changing banks
//! recompile only the map) and goes back in [`crate::Returned::map`]. Its running state (pickup, toggles, 14-bit
//! halves) carries over to the next table.

use lm_base::HostTime;
pub use lm_midimap::{Entry as MapEntry, MapTable, Timing};

use crate::ids::{ParamSlot, PortIndex};
use crate::spec::ProcessSpec;

// The value law (`ParamRange`, `RangeScale`) lives in lm-base, shared with the port and the interface (one law)
pub use lm_base::{ParamRange, RangeScale};

/// What one compiled mapping drives
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MapTarget {
    /// A parameter: the renderer writes it at the message's sample
    Param { slot: ParamSlot, range: ParamRange },
    /// A field or an action: the session applies it (≤ 10 ms)
    Session,
}

/// The compiled map
#[derive(Debug)]
pub struct MidiMap {
    pub(crate) table: MapTable<MapTarget>,
}

impl MidiMap {
    /// The table for `spec` (feedback timing in samples is not used here: the session sends the feedback).
    /// `ports`: how many port indices exist
    pub fn new(entries: Vec<MapEntry<MapTarget>>, ports: u16, spec: ProcessSpec) -> Box<MidiMap> {
        Box::new(MidiMap { table: MapTable::new(entries, ports, Timing::samples(spec.sample_rate)) })
    }

    pub fn empty() -> Box<MidiMap> {
        Box::new(MidiMap { table: MapTable::empty() })
    }

    pub fn entries(&self) -> &[MapEntry<MapTarget>] {
        self.table.entries()
    }
}

/// A mapped message, for the session (lossless ring, explicit overflow; sized for 10 s of 16 ports at 1000
/// messages a second, C4)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MapEvent {
    /// A mapped parameter was written (the value in its unit and normalized). The slot is touched until the
    /// gesture ends ([`MapEvent::Released`])
    Param { mapping: u64, port: PortIndex, slot: ParamSlot, value: f32, normalized: f32, sample: i64, frame: u64 },
    /// A field or an action to apply: the normalized value, the raw value (velocity, program number, 7 or
    /// 14 bits), and for a button whether it was a press or a release
    Session { mapping: u64, port: PortIndex, value: f32, raw: i32, press: Option<bool>, frame: u64 },
    /// A touch source went down or up
    Touch { mapping: u64, port: PortIndex, slot: Option<ParamSlot>, on: bool, frame: u64 },
    /// The MIDI gesture on a slot ended (500 ms after the last message, or the touch released): the session writes
    /// the final value into the document, one undo step
    Released { slot: ParamSlot, value: f32, frame: u64 },
}

/// A MIDI message that reached the instruments while recording (I-21.4), at its sample on the timeline (lossless
/// ring, sized like the map events)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordedMidi {
    pub port: PortIndex,
    pub bytes: [u8; 3],
    pub len: u8,
    /// The timeline sample where it was played (the block delay is already taken out)
    pub sample: i64,
    /// The engine frame (for the loop passes)
    pub frame: u64,
}

/// A MIDI message to send out (P-38, the feedback), with the time it must leave. Consumed by `lm-io`'s output
/// thread, which schedules it on the sequencer queue at `at` minus the port's output latency
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiOutEvent {
    pub port: PortIndex,
    pub at: HostTime,
    pub len: u8,
    pub bytes: [u8; 3],
}

/// The MIDI gesture lasts this long after the last message (I-21.2)
pub const GESTURE_SECONDS_DIV: u64 = 2; // 500 ms = rate / 2
/// Back to the curve in this long after a release (I-21.5): 50 ms = rate / 20
pub const RETURN_DIV: u64 = 20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_round_trip() {
        let r = [
            ParamRange::linear(-15.0, 15.0),
            ParamRange { min: 20.0, max: 20000.0, scale: RangeScale::Log },
            ParamRange { min: 0.0, max: 4.0, scale: RangeScale::Gain },
            ParamRange { min: 0.0, max: 8.0, scale: RangeScale::Stepped },
        ];
        for range in r {
            for i in 1..=100 {
                let y = i as f32 / 100.0;
                let v = range.to_units(y);
                assert!(v >= range.min.min(0.0) && v <= range.max, "{range:?} {y} {v}");
                if range.scale != RangeScale::Stepped {
                    assert!((range.normalize(v) - y).abs() < 1e-3, "{range:?} {y}");
                }
            }
        }
        let gain = r[2];
        assert_eq!(gain.to_units(0.0), 0.0);
        assert!((gain.to_units(1.0) - 4.0).abs() < 1e-4);
        // The legacy fader's numbers for a maximum of 4 (+12.04 dB)
        for (db, travel) in [(0.0f32, 0.751f32), (-6.0, 0.644), (-12.0, 0.547), (-24.0, 0.384), (-60.0, 0.094), (6.0, 0.869)] {
            let t = gain.normalize(libm::powf(10.0, db / 20.0));
            assert!((t - travel).abs() < 1.5e-3, "{db} dB at {t}, the legacy says {travel}");
        }
        assert_eq!(r[1].to_units(0.5).round(), 632.0);
        assert_eq!(r[3].to_units(0.49), 4.0);
    }
}
