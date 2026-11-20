//! LinuxMusic I/O (interfaces.md, I-10, I-11, I-12; Parte 5 H1–H3, H19; Parte 6 U3–U17; Parte 7 C2, C6–C10).
#![forbid(unsafe_code)]

mod backend;
mod device;
mod disk;
mod dummy;
mod edge;
pub mod peaks;
mod record;
mod take;

pub use backend::{Backend, CallbackGuard, CallbackHandle, DeviceCallback, DeviceConfig, DeviceEvent, DeviceInfo};
pub use device::{Device, STOP_TIMEOUT, StopTimedOut, SwitchFailed, abandoned_devices, switch_device};
pub use disk::{DiskEvent, DiskWriter, LIVE_PEAK_CHANNELS, LivePeak, RESERVE_SECONDS, SYNC_SECONDS, ring_frames};
pub use dummy::{DummyBackend, DummyControl, DummyOptions, DummySource, Pace, Silence};
pub use edge::{Edge, EdgeStats, MAX_CHANNELS};
pub use peaks::{Peaks, PeaksBuilder};
pub use record::{DEFAULT_SECONDS, InputIndex, MAX_SPANS, RecordInput, RecordPlan, RecordReceiver, RecordSender, recording_channel};
pub use take::{
    FinalizedTake, LoopCut, REFUSE_SECONDS, RecordHeader, RecordLine, SpaceCheck, StopReason, Take, TakeFormat, TakeGap,
    TakeSpan, TakeStop, WARN_SECONDS, check_record_space, finalize_take, find_interrupted, forget_record, free_space,
    hash_file, new_uuid, peaks_of_file, read_record, record_bytes_per_second, recover_take, space_for,
};
