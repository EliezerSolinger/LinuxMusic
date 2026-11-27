//! LinuxMusic I/O (interfaces.md, I-10, I-11, I-12; Parte 5 H1–H3, H19; Parte 6 U3–U17; Parte 7 C2, C6–C10).
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod alsa;
mod backend;
mod calibrate;
mod choice;
#[cfg(target_os = "macos")]
mod coreaudio;
mod device;
mod disk;
mod dummy;
mod edge;
#[cfg(feature = "jack")]
mod jack;
#[cfg(feature = "jack")]
mod jackmidi;
mod midi;
mod mididev;
pub mod peaks;
mod reader;
mod record;
mod resources;
pub mod reverse;
mod rt;
pub mod stretch;
mod system;
mod take;

#[cfg(target_os = "linux")]
pub use alsa::{AlsaBackend, card_index, device_present, list_devices};
pub use backend::{Backend, CallbackGuard, CallbackHandle, DeviceCallback, DeviceConfig, DeviceEvent, DeviceInfo};
pub use calibrate::{BURST, Calibration, CalibrationKey, CalibrationOptions, burst, calibrate, find_lag};
pub use choice::{ChoiceCaps, DeviceChoice};
#[cfg(target_os = "macos")]
pub use coreaudio::{
    CoreAudioBackend, device_present as coreaudio_device_present, list_devices as coreaudio_list_devices,
};
pub use device::{
    Device, MAX_STUCK_DEVICES, STOP_TIMEOUT, StartError, StopTimedOut, SwitchFailed, abandoned_devices, stuck_devices,
    switch_device,
};
pub use disk::{
    DiskEvent, DiskFaults, DiskWriter, LIVE_PEAK_CHANNELS, LivePeak, RESERVE_SECONDS, SYNC_SECONDS, ring_frames,
};
pub use dummy::{DummyBackend, DummyControl, DummyOptions, DummySource, Pace, Silence};
pub use edge::{Edge, EdgeStats, MAX_CHANNELS};
#[cfg(feature = "jack")]
pub use jack::{
    JACK_DEFAULT_INPUTS, JACK_MAX_CHANNELS, JackBackend, JackOptions, jack_device_present, jack_list_devices,
};
#[cfg(feature = "jack")]
pub use jackmidi::{JACK_MIDI_PORTS, JackMidiLink, JackMidiShared, SYSEX_SLOT};
#[cfg(target_os = "linux")]
pub use midi::list_midi_ports;
pub use midi::{MidiInput, MidiPortInfo, MidiSink, MidiSource, MidiStats, Parsed, RawParser, midi_ring};
pub use mididev::{
    ACTIVITY_RING, CLIENT_NAME, INPUT_RING, MidiActivity, MidiBackendKind, MidiDeviceEvent, OWN_INPUT, OWN_OUTPUT,
    PortId, PortInfo, PortRole, PortSettings, SYSEX_IN_MAX, SYSEX_OUT_MAX, SysexIn, SysexOut, message_len,
};
#[cfg(target_os = "linux")]
pub use mididev::{MidiDevices, raw_devices};
pub use peaks::{Peaks, PeaksBuilder};
pub use reader::{DiskReader, StreamEvent};
pub use record::{
    DEFAULT_SECONDS, InputIndex, MAX_SPANS, RecordInput, RecordPlan, RecordReceiver, RecordSender, recording_channel,
};
pub use resources::{
    AUDIO_EXTENSIONS, BYTES_MAX, Content, EVICT_AFTER, ImportedFile, ImportedResource, IoFaults, KEY_LEN, KindJob,
    KindLoaded, KindPolicy, LoadJob, Loaded, OnDisk, ResourceState, ResourceStore, ResourceView, STREAM_ABOVE,
    audio_folder, clean_import_leftovers, import_file, import_file_as, is_valid_key, is_valid_kind, kind_policy,
    load_file, render_folder, resolve_external, resource_path, resource_path_in, resource_path_of, resource_path_of_in,
    state_folder, verify_located,
};
pub use rt::{
    DEFAULT_PRIORITY, DEFAULT_STACK, MemoryLock, NoRealtime, RealtimeThreads, RtBuffer, RtMembership, RtMethod,
    RtReport, SystemRealtime, lock_memory, realtime_hook,
};
pub use system::{SleepGuard, device_error};
pub use take::{
    FinalizedTake, LoopCut, REFUSE_SECONDS, RecordHeader, RecordLine, SpaceCheck, StopReason, Take, TakeFormat,
    TakeGap, TakeSpan, TakeStop, WARN_SECONDS, check_record_space, finalize_take, find_interrupted, forget_record,
    free_space, hash_file, new_uuid, peaks_of_file, read_record, record_bytes_per_second, recover_take, space_for,
};
