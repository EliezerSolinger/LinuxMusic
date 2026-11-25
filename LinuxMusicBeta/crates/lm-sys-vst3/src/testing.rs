//! Two VST3 test plugins, for this crate's tests and the helper's (built as a shared library by
//! `examples/test_vst3.rs`, which calls [`export!`](crate::export_test_plugins)).
//!
//! - **Test gain** (`GAIN`): an effect with a separate controller, connected to the processor through the host's
//!   messages. Parameters 7 "Gain" (×4 at 1, default 0.25) and 9 "Latency" (1000 steps). A latency change on the
//!   controller is sent to the processor as a message and asks the host for a restart; the new latency is reported
//!   after it. State: the two normalized values.
//! - **Test synth** (`SYNTH`): one object that is component, processor and controller, with an event input. Parameter
//!   1 "Level" (default 0.5) and 2 "Last key" (read-only, 127 steps, sent back by the processor on each note-on).
//!   Left: level × volume expression (0.25 is unity) while a note is held; right: tempo / 1000 when the transport
//!   has one. MIDI controller 7 on any channel is "Level".
#![allow(non_snake_case, clippy::missing_safety_doc)]

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_void};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComRef, ComWrapper, Interface, uid};

use crate::host::put_wide;

pub const GAIN: TUID = uid(0x4C4D5631, 0x0A1B2C3D, 0x4E5F6071, 0x8293A4B5);
pub const GAIN_CONTROLLER: TUID = uid(0x4C4D5632, 0x0A1B2C3D, 0x4E5F6071, 0x8293A4B5);
pub const SYNTH: TUID = uid(0x4C4D5633, 0x0A1B2C3D, 0x4E5F6071, 0x8293A4B5);

fn put_narrow(src: &str, dst: &mut [char8]) {
    let b = src.as_bytes();
    let n = b.len().min(dst.len() - 1);
    for (d, s) in dst.iter_mut().zip(&b[..n]) {
        *d = *s as char8;
    }
    dst[n] = 0;
}

fn f64_load(a: &AtomicU64) -> f64 {
    f64::from_bits(a.load(Ordering::Relaxed))
}

fn f64_store(a: &AtomicU64, v: f64) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

unsafe fn last_point(q: ComRef<'_, IParamValueQueue>) -> Option<f64> {
    // SAFETY: the host's queue, valid during process
    let n = unsafe { q.getPointCount() };
    let mut off = 0;
    let mut v = 0.0;
    // SAFETY: as above, with out pointers of ours
    (n > 0 && unsafe { q.getPoint(n - 1, &mut off, &mut v) } == kResultOk).then_some(v)
}

unsafe fn read_f64s(stream: *mut IBStream, out: &mut [f64]) -> bool {
    // SAFETY: the host's stream, valid for the call
    let Some(s) = (unsafe { ComRef::from_raw(stream) }) else { return false };
    for v in out.iter_mut() {
        let mut b = [0u8; 8];
        let mut got = 0;
        // SAFETY: a buffer of ours of 8 bytes
        if unsafe { s.read(b.as_mut_ptr().cast(), 8, &mut got) } != kResultOk || got != 8 {
            return false;
        }
        *v = f64::from_le_bytes(b);
    }
    true
}

unsafe fn write_f64s(stream: *mut IBStream, vals: &[f64]) -> bool {
    // SAFETY: the host's stream, valid for the call
    let Some(s) = (unsafe { ComRef::from_raw(stream) }) else { return false };
    for v in vals {
        let mut b = v.to_le_bytes();
        let mut put = 0;
        // SAFETY: a buffer of ours of 8 bytes
        if unsafe { s.write(b.as_mut_ptr().cast(), 8, &mut put) } != kResultOk {
            return false;
        }
    }
    true
}

fn bus_info(bus: *mut BusInfo, media: MediaType, dir: BusDirection, name: &str, channels: i32) -> tresult {
    // SAFETY: the host's out pointer
    let b = unsafe { &mut *bus };
    b.mediaType = media;
    b.direction = dir;
    b.channelCount = channels;
    put_wide(name, &mut b.name);
    b.busType = BusTypes_::kMain as BusType;
    b.flags = BusInfo_::BusFlags_::kDefaultActive;
    kResultOk
}

fn param_info(info: *mut ParameterInfo, id: u32, title: &str, steps: i32, default: f64, flags: u32) -> tresult {
    // SAFETY: the host's out pointer
    let i = unsafe { &mut *info };
    i.id = id;
    put_wide(title, &mut i.title);
    put_wide(title, &mut i.shortTitle);
    put_wide("", &mut i.units);
    i.stepCount = steps;
    i.defaultNormalizedValue = default;
    i.unitId = 0;
    i.flags = flags as i32;
    kResultOk
}

fn stereo_buses(dir: BusDirection, inputs: bool) -> i32 {
    if dir == BusDirections_::kInput as BusDirection && !inputs { 0 } else { 1 }
}

// ---------------------------------------------------------------------------------------------------------------
// The gain: processor

struct GainProc {
    gain: AtomicU64,
    latency_norm: AtomicU64,
    latency: AtomicU32,
    line: RefCell<Vec<Vec<f32>>>,
    pos: Cell<usize>,
    peer: RefCell<Option<ComPtr<IConnectionPoint>>>,
}

impl Class for GainProc {
    type Interfaces = (IComponent, IAudioProcessor, IConnectionPoint);
}

impl GainProc {
    fn new() -> GainProc {
        GainProc {
            gain: AtomicU64::new(0.25f64.to_bits()),
            latency_norm: AtomicU64::new(0.0f64.to_bits()),
            latency: AtomicU32::new(0),
            line: RefCell::new(vec![vec![0.0; 1025]; 2]),
            pos: Cell::new(0),
            peer: RefCell::new(None),
        }
    }
}

impl IPluginBaseTrait for GainProc {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IComponentTrait for GainProc {
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        // SAFETY: the host's out pointer
        unsafe { *class_id = GAIN_CONTROLLER };
        kResultOk
    }
    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, media: MediaType, dir: BusDirection) -> int32 {
        if media == MediaTypes_::kAudio as MediaType { stereo_buses(dir, true) } else { 0 }
    }
    unsafe fn getBusInfo(&self, media: MediaType, dir: BusDirection, index: int32, bus: *mut BusInfo) -> tresult {
        if media != MediaTypes_::kAudio as MediaType || index != 0 {
            return kInvalidArgument;
        }
        bus_info(bus, media, dir, if dir == BusDirections_::kInput as BusDirection { "In" } else { "Out" }, 2)
    }
    unsafe fn getRoutingInfo(&self, _i: *mut RoutingInfo, _o: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }
    unsafe fn activateBus(&self, _m: MediaType, _d: BusDirection, _i: int32, _s: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setActive(&self, state: TBool) -> tresult {
        if state != 0 {
            for l in self.line.borrow_mut().iter_mut() {
                l.fill(0.0);
            }
            self.pos.set(0);
        }
        kResultOk
    }
    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let mut v = [0.0; 2];
        // SAFETY: the host's stream
        if !unsafe { read_f64s(state, &mut v) } {
            return kResultFalse;
        }
        f64_store(&self.gain, v[0]);
        f64_store(&self.latency_norm, v[1]);
        self.latency.store((v[1] * 1000.0).round() as u32, Ordering::Relaxed);
        kResultOk
    }
    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        // SAFETY: the host's stream
        if unsafe { write_f64s(state, &[f64_load(&self.gain), f64_load(&self.latency_norm)]) } {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

impl IAudioProcessorTrait for GainProc {
    unsafe fn setBusArrangements(
        &self,
        _i: *mut SpeakerArrangement,
        _ni: int32,
        _o: *mut SpeakerArrangement,
        _no: int32,
    ) -> tresult {
        kResultTrue
    }
    unsafe fn getBusArrangement(&self, _d: BusDirection, index: int32, arr: *mut SpeakerArrangement) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        // SAFETY: the host's out pointer
        unsafe { *arr = SpeakerArr::kStereo };
        kResultOk
    }
    unsafe fn canProcessSampleSize(&self, size: int32) -> tresult {
        if size == SymbolicSampleSizes_::kSample32 as i32 { kResultOk } else { kNotImplemented }
    }
    unsafe fn getLatencySamples(&self) -> uint32 {
        self.latency.load(Ordering::Relaxed)
    }
    unsafe fn setupProcessing(&self, _setup: *mut ProcessSetup) -> tresult {
        kResultOk
    }
    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        // SAFETY: the host's process data and everything it points to are valid for the call
        unsafe {
            let d = &*data;
            if let Some(changes) = ComRef::from_raw(d.inputParameterChanges) {
                for i in 0..changes.getParameterCount() {
                    if let Some(q) = ComRef::from_raw(changes.getParameterData(i))
                        && q.getParameterId() == 7
                        && let Some(v) = last_point(q)
                    {
                        f64_store(&self.gain, v);
                    }
                }
            }
            if d.numInputs < 1 || d.numOutputs < 1 {
                return kResultOk;
            }
            let n = d.numSamples as usize;
            let g = (f64_load(&self.gain) * 4.0) as f32;
            let delay = self.latency.load(Ordering::Relaxed) as usize;
            let ins = std::slice::from_raw_parts((*d.inputs).__field0.channelBuffers32, 2);
            let outs = std::slice::from_raw_parts((*d.outputs).__field0.channelBuffers32, 2);
            let mut line = self.line.borrow_mut();
            let len = line[0].len();
            let pos = self.pos.get();
            for ch in 0..2 {
                let x = std::slice::from_raw_parts(ins[ch], n);
                let y = std::slice::from_raw_parts_mut(outs[ch], n);
                for k in 0..n {
                    let w = (pos + k) % len;
                    line[ch][w] = x[k];
                    y[k] = line[ch][(pos + k + len - delay.min(len - 1)) % len] * g;
                }
            }
            self.pos.set((pos + n) % len);
        }
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> uint32 {
        0
    }
}

impl IConnectionPointTrait for GainProc {
    unsafe fn connect(&self, other: *mut IConnectionPoint) -> tresult {
        // SAFETY: the host's connection point, kept with a reference
        *self.peer.borrow_mut() = unsafe { ComRef::from_raw(other) }.map(|r| r.to_com_ptr());
        kResultOk
    }
    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        *self.peer.borrow_mut() = None;
        kResultOk
    }
    unsafe fn notify(&self, message: *mut IMessage) -> tresult {
        // SAFETY: the sender's message, valid for the call
        let Some(m) = (unsafe { ComRef::from_raw(message) }) else { return kInvalidArgument };
        // SAFETY: as above
        let id = unsafe { m.getMessageID() };
        // SAFETY: a NUL-terminated id
        if id.is_null() || unsafe { CStr::from_ptr(id) }.to_bytes() != b"latency" {
            return kResultFalse;
        }
        // SAFETY: as above
        let Some(a) = (unsafe { ComRef::from_raw(m.getAttributes()) }) else { return kResultFalse };
        let mut v = 0.0;
        // SAFETY: an out pointer of ours
        if unsafe { a.getFloat(c"value".as_ptr(), &mut v) } != kResultOk {
            return kResultFalse;
        }
        f64_store(&self.latency_norm, v);
        self.latency.store((v * 1000.0).round() as u32, Ordering::Relaxed);
        kResultOk
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The gain: controller

struct GainCtrl {
    gain: Cell<f64>,
    latency: Cell<f64>,
    host: RefCell<Option<ComPtr<IHostApplication>>>,
    handler: RefCell<Option<ComPtr<IComponentHandler>>>,
    peer: RefCell<Option<ComPtr<IConnectionPoint>>>,
}

impl Class for GainCtrl {
    type Interfaces = (IEditController, IConnectionPoint);
}

impl IPluginBaseTrait for GainCtrl {
    unsafe fn initialize(&self, context: *mut FUnknown) -> tresult {
        // SAFETY: the host's context, kept with a reference
        *self.host.borrow_mut() = unsafe { ComRef::from_raw(context) }.and_then(|c| c.cast::<IHostApplication>());
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        *self.host.borrow_mut() = None;
        kResultOk
    }
}

impl GainCtrl {
    fn send_latency(&self, v: f64) {
        let host = self.host.borrow();
        let peer = self.peer.borrow();
        let (Some(h), Some(p)) = (host.as_ref(), peer.as_ref()) else { return };
        let mut cid = crate::host::tuid(&IMessage::IID);
        let mut iid = crate::host::tuid(&IMessage::IID);
        let mut obj: *mut c_void = std::ptr::null_mut();
        // SAFETY: the host's createInstance; the message comes with a reference, now ours
        unsafe {
            if h.createInstance(&mut cid, &mut iid, &mut obj) != kResultOk {
                return;
            }
            let Some(m) = ComPtr::from_raw(obj as *mut IMessage) else { return };
            m.setMessageID(c"latency".as_ptr());
            if let Some(a) = ComRef::from_raw(m.getAttributes()) {
                a.setFloat(c"value".as_ptr(), v);
            }
            p.notify(m.as_ptr());
        }
    }
}

impl IEditControllerTrait for GainCtrl {
    unsafe fn setComponentState(&self, state: *mut IBStream) -> tresult {
        let mut v = [0.0; 2];
        // SAFETY: the host's stream
        if !unsafe { read_f64s(state, &mut v) } {
            return kResultFalse;
        }
        self.gain.set(v[0]);
        self.latency.set(v[1]);
        kResultOk
    }
    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getParameterCount(&self) -> int32 {
        2
    }
    unsafe fn getParameterInfo(&self, index: int32, info: *mut ParameterInfo) -> tresult {
        let automate = ParameterInfo_::ParameterFlags_::kCanAutomate as u32;
        match index {
            0 => param_info(info, 7, "Gain", 0, 0.25, automate),
            1 => param_info(info, 9, "Latency", 1000, 0.0, automate),
            _ => kInvalidArgument,
        }
    }
    unsafe fn getParamStringByValue(&self, _id: ParamID, v: ParamValue, s: *mut String128) -> tresult {
        // SAFETY: the host's buffer
        put_wide(&format!("{v:.3}"), unsafe { &mut *s });
        kResultOk
    }
    unsafe fn getParamValueByString(&self, _id: ParamID, _s: *mut TChar, _v: *mut ParamValue) -> tresult {
        kNotImplemented
    }
    unsafe fn normalizedParamToPlain(&self, id: ParamID, v: ParamValue) -> ParamValue {
        if id == 7 { v * 4.0 } else { (v * 1000.0).round() }
    }
    unsafe fn plainParamToNormalized(&self, id: ParamID, v: ParamValue) -> ParamValue {
        if id == 7 { v / 4.0 } else { v / 1000.0 }
    }
    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        if id == 7 { self.gain.get() } else { self.latency.get() }
    }
    unsafe fn setParamNormalized(&self, id: ParamID, v: ParamValue) -> tresult {
        match id {
            7 => self.gain.set(v),
            9 => {
                if (self.latency.get() - v).abs() > 1e-9 {
                    self.latency.set(v);
                    self.send_latency(v);
                    if let Some(h) = self.handler.borrow().as_ref() {
                        // SAFETY: the host's handler
                        unsafe { h.restartComponent(RestartFlags_::kLatencyChanged) };
                    }
                }
            }
            _ => return kInvalidArgument,
        }
        kResultOk
    }
    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        // SAFETY: the host's handler, kept with a reference (null: let it go)
        *self.handler.borrow_mut() = unsafe { ComRef::from_raw(handler) }.map(|r| r.to_com_ptr());
        kResultOk
    }
    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        std::ptr::null_mut()
    }
}

impl IConnectionPointTrait for GainCtrl {
    unsafe fn connect(&self, other: *mut IConnectionPoint) -> tresult {
        // SAFETY: the host's connection point, kept with a reference
        *self.peer.borrow_mut() = unsafe { ComRef::from_raw(other) }.map(|r| r.to_com_ptr());
        kResultOk
    }
    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        *self.peer.borrow_mut() = None;
        kResultOk
    }
    unsafe fn notify(&self, _message: *mut IMessage) -> tresult {
        kResultOk
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The synth: one object

struct Synth {
    level: AtomicU64,
    last_key: AtomicU64,
    held: Cell<i32>,
    volume: Cell<f64>,
}

impl Class for Synth {
    type Interfaces = (IComponent, IAudioProcessor, IEditController, IMidiMapping);
}

impl IPluginBaseTrait for Synth {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IComponentTrait for Synth {
    unsafe fn getControllerClassId(&self, _class_id: *mut TUID) -> tresult {
        kNotImplemented
    }
    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, media: MediaType, dir: BusDirection) -> int32 {
        let input = dir == BusDirections_::kInput as BusDirection;
        match media {
            m if m == MediaTypes_::kAudio as MediaType => i32::from(!input),
            m if m == MediaTypes_::kEvent as MediaType => i32::from(input),
            _ => 0,
        }
    }
    unsafe fn getBusInfo(&self, media: MediaType, dir: BusDirection, index: int32, bus: *mut BusInfo) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        if media == MediaTypes_::kEvent as MediaType {
            bus_info(bus, media, dir, "Notes", 16)
        } else {
            bus_info(bus, media, dir, "Out", 2)
        }
    }
    unsafe fn getRoutingInfo(&self, _i: *mut RoutingInfo, _o: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }
    unsafe fn activateBus(&self, _m: MediaType, _d: BusDirection, _i: int32, _s: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setActive(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let mut v = [0.0; 2];
        // SAFETY: the host's stream
        if !unsafe { read_f64s(state, &mut v) } {
            return kResultFalse;
        }
        f64_store(&self.level, v[0]);
        f64_store(&self.last_key, v[1]);
        kResultOk
    }
    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        // SAFETY: the host's stream
        if unsafe { write_f64s(state, &[f64_load(&self.level), f64_load(&self.last_key)]) } {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

impl IAudioProcessorTrait for Synth {
    unsafe fn setBusArrangements(
        &self,
        _i: *mut SpeakerArrangement,
        _ni: int32,
        _o: *mut SpeakerArrangement,
        _no: int32,
    ) -> tresult {
        kResultTrue
    }
    unsafe fn getBusArrangement(&self, _d: BusDirection, index: int32, arr: *mut SpeakerArrangement) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        // SAFETY: the host's out pointer
        unsafe { *arr = SpeakerArr::kStereo };
        kResultOk
    }
    unsafe fn canProcessSampleSize(&self, size: int32) -> tresult {
        if size == SymbolicSampleSizes_::kSample32 as i32 { kResultOk } else { kNotImplemented }
    }
    unsafe fn getLatencySamples(&self) -> uint32 {
        0
    }
    unsafe fn setupProcessing(&self, _setup: *mut ProcessSetup) -> tresult {
        kResultOk
    }
    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        // SAFETY: the host's process data and everything it points to are valid for the call
        unsafe {
            let d = &*data;
            if let Some(changes) = ComRef::from_raw(d.inputParameterChanges) {
                for i in 0..changes.getParameterCount() {
                    if let Some(q) = ComRef::from_raw(changes.getParameterData(i))
                        && q.getParameterId() == 1
                        && let Some(v) = last_point(q)
                    {
                        f64_store(&self.level, v);
                    }
                }
            }
            let n = d.numSamples as usize;
            if d.numOutputs < 1 {
                return kResultOk;
            }
            let outs = std::slice::from_raw_parts((*d.outputs).__field0.channelBuffers32, 2);
            let left = std::slice::from_raw_parts_mut(outs[0], n);
            let right = std::slice::from_raw_parts_mut(outs[1], n);
            // On the stack: the host runs this under an allocation trap
            let mut store: [Event; 64] = std::mem::zeroed();
            let mut count = 0;
            if let Some(list) = ComRef::from_raw(d.inputEvents) {
                for i in 0..list.getEventCount().min(64) {
                    if list.getEvent(i, &mut store[count]) == kResultOk {
                        count += 1;
                    }
                }
            }
            let events = &store[..count];
            let tempo = if d.processContext.is_null()
                || (*d.processContext).state & ProcessContext_::StatesAndFlags_::kTempoValid == 0
            {
                0.0
            } else {
                (*d.processContext).tempo / 1000.0
            };
            let mut next = 0;
            for k in 0..n {
                while next < events.len() && events[next].sampleOffset as usize <= k {
                    let e = &events[next];
                    match e.r#type as u32 {
                        t if t == Event_::EventTypes_::kNoteOnEvent => {
                            self.held.set(self.held.get() + 1);
                            let key = f64::from(e.__field0.noteOn.pitch);
                            f64_store(&self.last_key, key / 127.0);
                            if let Some(out) = ComRef::from_raw(d.outputParameterChanges) {
                                let id: ParamID = 2;
                                let mut index = 0;
                                if let Some(q) = ComRef::from_raw(out.addParameterData(&id, &mut index)) {
                                    q.addPoint(e.sampleOffset, key / 127.0, &mut index);
                                }
                            }
                        }
                        t if t == Event_::EventTypes_::kNoteOffEvent => {
                            self.held.set((self.held.get() - 1).max(0));
                        }
                        t if t == Event_::EventTypes_::kNoteExpressionValueEvent => {
                            let x = e.__field0.noteExpressionValue;
                            if x.typeId == NoteExpressionTypeIDs_::kVolumeTypeID as NoteExpressionTypeID {
                                self.volume.set(x.value / 0.25);
                            }
                        }
                        _ => {}
                    }
                    next += 1;
                }
                left[k] = if self.held.get() > 0 { (f64_load(&self.level) * self.volume.get()) as f32 } else { 0.0 };
                right[k] = tempo as f32;
            }
        }
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> uint32 {
        0
    }
}

impl IEditControllerTrait for Synth {
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getParameterCount(&self) -> int32 {
        2
    }
    unsafe fn getParameterInfo(&self, index: int32, info: *mut ParameterInfo) -> tresult {
        use ParameterInfo_::ParameterFlags_ as F;
        match index {
            0 => param_info(info, 1, "Level", 0, 0.5, F::kCanAutomate as u32),
            1 => param_info(info, 2, "Last key", 127, 0.0, F::kIsReadOnly as u32),
            _ => kInvalidArgument,
        }
    }
    unsafe fn getParamStringByValue(&self, _id: ParamID, v: ParamValue, s: *mut String128) -> tresult {
        // SAFETY: the host's buffer
        put_wide(&format!("{v:.3}"), unsafe { &mut *s });
        kResultOk
    }
    unsafe fn getParamValueByString(&self, _id: ParamID, _s: *mut TChar, _v: *mut ParamValue) -> tresult {
        kNotImplemented
    }
    unsafe fn normalizedParamToPlain(&self, _id: ParamID, v: ParamValue) -> ParamValue {
        v
    }
    unsafe fn plainParamToNormalized(&self, _id: ParamID, v: ParamValue) -> ParamValue {
        v
    }
    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        if id == 1 { f64_load(&self.level) } else { f64_load(&self.last_key) }
    }
    unsafe fn setParamNormalized(&self, id: ParamID, v: ParamValue) -> tresult {
        if id == 1 {
            f64_store(&self.level, v);
        }
        kResultOk
    }
    unsafe fn setComponentHandler(&self, _handler: *mut IComponentHandler) -> tresult {
        kResultOk
    }
    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        std::ptr::null_mut()
    }
}

impl IMidiMappingTrait for Synth {
    unsafe fn getMidiControllerAssignment(&self, _bus: int32, _ch: int16, cc: CtrlNumber, id: *mut ParamID) -> tresult {
        if cc == 7 {
            // SAFETY: the host's out pointer
            unsafe { *id = 1 };
            kResultTrue
        } else {
            kResultFalse
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The factory

struct Factory;

impl Class for Factory {
    type Interfaces = (IPluginFactory, IPluginFactory2);
}

impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        // SAFETY: the host's out pointer
        let i = unsafe { &mut *info };
        put_narrow("LinuxMusic", &mut i.vendor);
        put_narrow("", &mut i.url);
        put_narrow("", &mut i.email);
        i.flags = PFactoryInfo_::FactoryFlags_::kUnicode as int32;
        kResultOk
    }
    unsafe fn countClasses(&self) -> int32 {
        3
    }
    unsafe fn getClassInfo(&self, index: int32, info: *mut PClassInfo) -> tresult {
        let (cid, cat, name) = match index {
            0 => (GAIN, "Audio Module Class", "Test gain"),
            1 => (GAIN_CONTROLLER, "Component Controller Class", "Test gain"),
            2 => (SYNTH, "Audio Module Class", "Test synth"),
            _ => return kInvalidArgument,
        };
        // SAFETY: the host's out pointer
        let i = unsafe { &mut *info };
        i.cid = cid;
        i.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
        put_narrow(cat, &mut i.category);
        put_narrow(name, &mut i.name);
        kResultOk
    }
    unsafe fn createInstance(&self, cid: FIDString, iid: FIDString, obj: *mut *mut c_void) -> tresult {
        // SAFETY: the host's TUID
        let cid = unsafe { *(cid as *const TUID) };
        let made = match cid {
            GAIN => ComWrapper::new(GainProc::new()).to_com_ptr::<FUnknown>(),
            GAIN_CONTROLLER => ComWrapper::new(GainCtrl {
                gain: Cell::new(0.25),
                latency: Cell::new(0.0),
                host: RefCell::new(None),
                handler: RefCell::new(None),
                peer: RefCell::new(None),
            })
            .to_com_ptr::<FUnknown>(),
            SYNTH => ComWrapper::new(Synth {
                level: AtomicU64::new(0.5f64.to_bits()),
                last_key: AtomicU64::new(0),
                held: Cell::new(0),
                volume: Cell::new(1.0),
            })
            .to_com_ptr::<FUnknown>(),
            _ => None,
        };
        match made {
            Some(p) => {
                let raw = p.as_ptr();
                // SAFETY: a live object of ours; queryInterface adds the caller's reference
                unsafe { ((*(*raw).vtbl).queryInterface)(raw, iid as *mut TUID, obj) }
            }
            None => kInvalidArgument,
        }
    }
}

impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: int32, info: *mut PClassInfo2) -> tresult {
        let (cid, cat, name, sub) = match index {
            0 => (GAIN, "Audio Module Class", "Test gain", "Fx"),
            1 => (GAIN_CONTROLLER, "Component Controller Class", "Test gain", ""),
            2 => (SYNTH, "Audio Module Class", "Test synth", "Instrument|Synth"),
            _ => return kInvalidArgument,
        };
        // SAFETY: the host's out pointer
        let i = unsafe { &mut *info };
        i.cid = cid;
        i.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
        put_narrow(cat, &mut i.category);
        put_narrow(name, &mut i.name);
        put_narrow(sub, &mut i.subCategories);
        put_narrow("LinuxMusic", &mut i.vendor);
        put_narrow("1.0", &mut i.version);
        put_narrow("VST 3.7", &mut i.sdkVersion);
        i.classFlags = 0;
        kResultOk
    }
}

/// The factory, for `GetPluginFactory` (one reference, the caller's)
pub fn factory() -> *mut IPluginFactory {
    ComWrapper::new(Factory).to_com_ptr::<IPluginFactory>().map_or(std::ptr::null_mut(), |p| p.into_raw())
}

/// Exports the test plugins' entry points from the shared library that calls it
#[macro_export]
macro_rules! export_test_plugins {
    () => {
        #[unsafe(no_mangle)]
        pub extern "system" fn GetPluginFactory() -> *mut ::std::ffi::c_void {
            $crate::testing::factory().cast()
        }
        #[cfg(target_os = "linux")]
        #[unsafe(no_mangle)]
        pub extern "system" fn ModuleEntry(_handle: *mut ::std::ffi::c_void) -> bool {
            true
        }
        #[cfg(target_os = "linux")]
        #[unsafe(no_mangle)]
        pub extern "system" fn ModuleExit() -> bool {
            true
        }
        #[cfg(target_os = "macos")]
        #[unsafe(no_mangle)]
        pub extern "system" fn bundleEntry(_bundle: *mut ::std::ffi::c_void) -> bool {
            true
        }
        #[cfg(target_os = "macos")]
        #[unsafe(no_mangle)]
        pub extern "system" fn bundleExit() -> bool {
            true
        }
    };
}
