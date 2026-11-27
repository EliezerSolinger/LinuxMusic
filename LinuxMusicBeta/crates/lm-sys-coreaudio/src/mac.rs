//! The macOS half of the crate (see the crate docs for the safety sheet)

#![allow(non_upper_case_globals)]

use std::ffi::c_void;
use std::mem::{MaybeUninit, size_of};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{null, null_mut};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use core_foundation_sys::array::{CFArrayCreate, kCFTypeArrayCallBacks};
use core_foundation_sys::base::{CFRelease, CFTypeRef, kCFAllocatorDefault};
use core_foundation_sys::dictionary::{
    CFDictionaryCreate, kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks,
};
use core_foundation_sys::number::{CFNumberCreate, kCFNumberSInt32Type};
use core_foundation_sys::string::{
    CFStringCreateWithBytes, CFStringGetCString, CFStringGetLength, CFStringGetMaximumSizeForEncoding, CFStringRef,
    kCFStringEncodingUTF8,
};

use crate::ffi::*;

/// A CoreAudio call that failed: which call and the `OSStatus` it returned
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaError {
    pub call: &'static str,
    pub status: i32,
}

impl CaError {
    /// The device or object is gone (unplugged)
    pub fn is_disconnected(&self) -> bool {
        matches!(
            self.status,
            kAudioHardwareBadDeviceError | kAudioHardwareBadObjectError | kAudioHardwareBadStreamError
        )
    }

    /// The status as the four characters CoreAudio documents (`!dev`), when it is one
    pub fn code(&self) -> String {
        let b = self.status.to_be_bytes();
        if b.iter().all(|c| (0x20..0x7f).contains(c)) {
            String::from_utf8_lossy(&b).into_owned()
        } else {
            self.status.to_string()
        }
    }
}

impl std::fmt::Display for CaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed ({})", self.call, self.code())
    }
}

impl std::error::Error for CaError {}

pub type Result<T> = std::result::Result<T, CaError>;

fn check(call: &'static str, status: OSStatus) -> Result<()> {
    if status == kAudioHardwareNoError { Ok(()) } else { Err(CaError { call, status }) }
}

/// Input or output side of a device
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Input,
    Output,
}

impl Scope {
    fn code(self) -> u32 {
        match self {
            Scope::Input => kAudioObjectPropertyScopeInput,
            Scope::Output => kAudioObjectPropertyScopeOutput,
        }
    }
}

fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain }
}

// ---- Core Foundation guards ----

/// An owned Core Foundation object, released on drop
struct Cf(CFTypeRef);

impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the object was created by a Create function (we own one reference) and is released once.
            unsafe { CFRelease(self.0) };
        }
    }
}

fn cf_string(s: &str) -> Cf {
    // SAFETY: the bytes are valid for `len`; the encoding is UTF-8; the result is owned (Create rule).
    let r =
        unsafe { CFStringCreateWithBytes(kCFAllocatorDefault, s.as_ptr(), s.len() as isize, kCFStringEncodingUTF8, 0) };
    Cf(r.cast())
}

fn cf_number(v: i32) -> Cf {
    // SAFETY: `v` lives through the call and the type says it is an SInt32.
    let r = unsafe { CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, (&raw const v).cast()) };
    Cf(r.cast())
}

fn cf_array(items: &[&Cf]) -> Cf {
    let mut ptrs: Vec<*const c_void> = items.iter().map(|c| c.0).collect();
    // SAFETY: `ptrs` has `len` valid CF objects; the standard callbacks retain them.
    let r =
        unsafe { CFArrayCreate(kCFAllocatorDefault, ptrs.as_mut_ptr(), ptrs.len() as isize, &kCFTypeArrayCallBacks) };
    Cf(r.cast())
}

fn cf_dictionary(entries: &[(&Cf, &Cf)]) -> Cf {
    let mut keys: Vec<*const c_void> = entries.iter().map(|(k, _)| k.0).collect();
    let mut values: Vec<*const c_void> = entries.iter().map(|(_, v)| v.0).collect();
    // SAFETY: both arrays have `len` valid CF objects; the standard callbacks retain keys and values.
    let r = unsafe {
        CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_mut_ptr(),
            values.as_mut_ptr(),
            keys.len() as isize,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
    };
    Cf(r.cast())
}

fn string_of(s: CFStringRef) -> String {
    if s.is_null() {
        return String::new();
    }
    // SAFETY: `s` is a valid CFString for the whole function; the buffer has the size CoreFoundation asks for.
    unsafe {
        let len = CFStringGetLength(s);
        let max = CFStringGetMaximumSizeForEncoding(len, kCFStringEncodingUTF8) + 1;
        let mut buf = vec![0u8; max.max(1) as usize];
        if CFStringGetCString(s, buf.as_mut_ptr().cast(), max, kCFStringEncodingUTF8) == 0 {
            return String::new();
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
}

// ---- property helpers ----

/// Reads a plain-old-data property (`u32`, `f64`, `AudioValueRange`)
fn get_pod<T: Copy>(id: u32, selector: u32, scope: u32, call: &'static str) -> Result<T> {
    let a = address(selector, scope);
    let mut size = size_of::<T>() as u32;
    let mut out = MaybeUninit::<T>::zeroed();
    // SAFETY: `out` has `size` writable bytes; `a` and `size` live through the call.
    let st = unsafe { AudioObjectGetPropertyData(id, &a, 0, null(), &mut size, out.as_mut_ptr().cast()) };
    check(call, st)?;
    // SAFETY: zero-initialised and `T` is plain data (only used with integers, floats and `AudioValueRange`).
    Ok(unsafe { out.assume_init() })
}

fn set_pod<T: Copy>(id: u32, selector: u32, scope: u32, value: T, call: &'static str) -> Result<()> {
    let a = address(selector, scope);
    // SAFETY: `value` has `size_of::<T>()` readable bytes and lives through the call.
    let st = unsafe { AudioObjectSetPropertyData(id, &a, 0, null(), size_of::<T>() as u32, (&raw const value).cast()) };
    check(call, st)
}

/// Reads an array property of plain-old-data elements
fn get_vec<T: Copy>(id: u32, selector: u32, scope: u32, call: &'static str) -> Result<Vec<T>> {
    let a = address(selector, scope);
    let mut size = 0u32;
    // SAFETY: `a` and `size` live through the call.
    check(call, unsafe { AudioObjectGetPropertyDataSize(id, &a, 0, null(), &mut size) })?;
    let n = size as usize / size_of::<T>();
    let mut v: Vec<T> = Vec::with_capacity(n);
    // n * size_of::<T>() <= size, which came as a u32: the cast cannot truncate
    let mut size = (n * size_of::<T>()) as u32;
    // SAFETY: the vector has room for `size` bytes; the length is set to what CoreAudio wrote, in whole elements.
    unsafe {
        check(call, AudioObjectGetPropertyData(id, &a, 0, null(), &mut size, v.as_mut_ptr().cast()))?;
        v.set_len((size as usize / size_of::<T>()).min(n));
    }
    Ok(v)
}

fn get_string(id: u32, selector: u32, scope: u32, call: &'static str) -> Result<String> {
    let a = address(selector, scope);
    let mut size = size_of::<CFStringRef>() as u32;
    let mut s: CFStringRef = null();
    // SAFETY: `s` has room for one pointer; the property returns an owned (retained) CFString released below.
    let st = unsafe { AudioObjectGetPropertyData(id, &a, 0, null(), &mut size, (&raw mut s).cast()) };
    check(call, st)?;
    let out = string_of(s);
    drop(Cf(s.cast()));
    Ok(out)
}

/// A buffer list read by `kAudioDevicePropertyStreamConfiguration`: the channel count of every stream
fn stream_layout(id: u32, scope: Scope) -> Result<Vec<u32>> {
    let call = "AudioObjectGetPropertyData(StreamConfiguration)";
    let a = address(kAudioDevicePropertyStreamConfiguration, scope.code());
    let mut size = 0u32;
    // SAFETY: `a` and `size` live through the call.
    check(call, unsafe { AudioObjectGetPropertyDataSize(id, &a, 0, null(), &mut size) })?;
    if (size as usize) < size_of::<u32>() {
        return Ok(Vec::new());
    }
    // u64 elements: the list holds pointers, so it is 8-aligned
    let mut raw = vec![0u64; (size as usize).div_ceil(8)];
    // SAFETY: `raw` has at least `size` writable bytes.
    check(call, unsafe { AudioObjectGetPropertyData(id, &a, 0, null(), &mut size, raw.as_mut_ptr().cast()) })?;
    let list = raw.as_ptr().cast::<AudioBufferList>();
    // SAFETY: CoreAudio wrote a whole AudioBufferList into `raw`; every buffer read is inside `size` bytes (checked).
    unsafe {
        let n = (*list).mNumberBuffers as usize;
        let first = (*list).mBuffers.as_ptr();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let end = size_of::<u64>() + (i + 1) * size_of::<AudioBuffer>();
            if end > size as usize {
                break;
            }
            out.push((*first.add(i)).mNumberChannels);
        }
        Ok(out)
    }
}

/// A CoreAudio device (or an aggregate), by its `AudioObjectID`. A name, not an owner
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Device(pub u32);

/// What listing a device gives
#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub device: Device,
    /// The device's UID: stable across restarts, what the program stores
    pub uid: String,
    pub name: String,
    pub inputs: u32,
    pub outputs: u32,
    /// Ranges of nominal rates (`min == max` for a discrete rate)
    pub rates: Vec<(f64, f64)>,
    pub nominal_rate: f64,
}

impl Device {
    /// Every device the system knows (physical, aggregates, virtual)
    pub fn all() -> Result<Vec<Device>> {
        let ids = get_vec::<u32>(
            kAudioObjectSystemObject,
            kAudioHardwarePropertyDevices,
            kAudioObjectPropertyScopeGlobal,
            "AudioObjectGetPropertyData(Devices)",
        )?;
        Ok(ids.into_iter().map(Device).collect())
    }

    /// The system's default device for a direction. `None` when there is none
    pub fn default(scope: Scope) -> Result<Option<Device>> {
        let selector = match scope {
            Scope::Input => kAudioHardwarePropertyDefaultInputDevice,
            Scope::Output => kAudioHardwarePropertyDefaultOutputDevice,
        };
        let id = get_pod::<u32>(
            kAudioObjectSystemObject,
            selector,
            kAudioObjectPropertyScopeGlobal,
            "AudioObjectGetPropertyData(DefaultDevice)",
        )?;
        Ok((id != 0).then_some(Device(id)))
    }

    /// The device with this UID
    pub fn from_uid(uid: &str) -> Result<Option<Device>> {
        let s = cf_string(uid);
        let a = address(kAudioHardwarePropertyTranslateUIDToDevice, kAudioObjectPropertyScopeGlobal);
        let mut id: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        let qualifier: CFStringRef = s.0.cast();
        // SAFETY: the qualifier is a pointer to a live CFString (size of one pointer); `id` has room for one u32.
        let st = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &a,
                size_of::<CFStringRef>() as u32,
                (&raw const qualifier).cast(),
                &mut size,
                (&raw mut id).cast(),
            )
        };
        check("AudioObjectGetPropertyData(TranslateUIDToDevice)", st)?;
        Ok((id != 0).then_some(Device(id)))
    }

    /// Finds a device by what the user chose: `"default"` (the system's for `scope`), a UID, or a name
    pub fn find(name: &str, scope: Scope) -> Result<Option<Device>> {
        if name.is_empty() || name == "default" {
            return Self::default(scope);
        }
        if let Some(d) = Self::from_uid(name)? {
            return Ok(Some(d));
        }
        for d in Self::all()? {
            if d.name().is_ok_and(|n| n == name) && d.channels(scope).unwrap_or(0) > 0 {
                return Ok(Some(d));
            }
        }
        Ok(None)
    }

    pub fn uid(self) -> Result<String> {
        get_string(self.0, kAudioDevicePropertyDeviceUID, kAudioObjectPropertyScopeGlobal, "DeviceUID")
    }

    pub fn name(self) -> Result<String> {
        get_string(self.0, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, "ObjectName")
    }

    /// The channel count of every stream of a direction (a stereo pair is one stream of 2, or two of 1)
    pub fn channel_layout(self, scope: Scope) -> Result<Vec<u32>> {
        stream_layout(self.0, scope)
    }

    /// All the channels of a direction
    pub fn channels(self, scope: Scope) -> Result<u32> {
        Ok(self.channel_layout(scope)?.iter().sum())
    }

    pub fn nominal_rate(self) -> Result<f64> {
        get_pod::<f64>(
            self.0,
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
            "NominalSampleRate",
        )
    }

    /// Asks for a rate. The change is asynchronous: see [`Device::wait_rate`]
    pub fn set_nominal_rate(self, rate: f64) -> Result<()> {
        set_pod(
            self.0,
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
            rate,
            "SetNominalSampleRate",
        )
    }

    /// Waits until the device runs at `rate` (a rate change takes a moment), up to `timeout`
    pub fn wait_rate(self, rate: f64, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            if self.nominal_rate().is_ok_and(|r| (r - rate).abs() < 0.5) {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The nominal rates the device offers, as ranges
    pub fn rate_ranges(self) -> Result<Vec<(f64, f64)>> {
        let v = get_vec::<AudioValueRange>(
            self.0,
            kAudioDevicePropertyAvailableNominalSampleRates,
            kAudioObjectPropertyScopeGlobal,
            "AvailableNominalSampleRates",
        )?;
        Ok(v.into_iter().map(|r| (r.mMinimum, r.mMaximum)).collect())
    }

    pub fn supports_rate(self, rate: f64) -> bool {
        self.rate_ranges().map(|r| r.iter().any(|(lo, hi)| rate >= *lo - 0.5 && rate <= *hi + 0.5)).unwrap_or(false)
    }

    /// The buffer sizes the device accepts, in frames: (min, max)
    pub fn buffer_range(self) -> Result<(u32, u32)> {
        let r = get_pod::<AudioValueRange>(
            self.0,
            kAudioDevicePropertyBufferFrameSizeRange,
            kAudioObjectPropertyScopeGlobal,
            "BufferFrameSizeRange",
        )?;
        Ok((r.mMinimum as u32, r.mMaximum as u32))
    }

    pub fn buffer_size(self) -> Result<u32> {
        get_pod::<u32>(self.0, kAudioDevicePropertyBufferFrameSize, kAudioObjectPropertyScopeGlobal, "BufferFrameSize")
    }

    pub fn set_buffer_size(self, frames: u32) -> Result<()> {
        set_pod(
            self.0,
            kAudioDevicePropertyBufferFrameSize,
            kAudioObjectPropertyScopeGlobal,
            frames,
            "SetBufferFrameSize",
        )
    }

    /// The latency of a direction beyond the buffer, in frames: the device's own, its safety offset and the largest
    /// latency of its streams. What the converters and the driver hold
    pub fn latency(self, scope: Scope) -> Result<u32> {
        let device = get_pod::<u32>(self.0, kAudioDevicePropertyLatency, scope.code(), "DeviceLatency").unwrap_or(0);
        let safety =
            get_pod::<u32>(self.0, kAudioDevicePropertySafetyOffset, scope.code(), "SafetyOffset").unwrap_or(0);
        let streams = get_vec::<u32>(self.0, kAudioDevicePropertyStreams, scope.code(), "Streams").unwrap_or_default();
        let stream = streams
            .into_iter()
            .map(|s| {
                get_pod::<u32>(s, kAudioStreamPropertyLatency, kAudioObjectPropertyScopeGlobal, "StreamLatency")
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0);
        Ok(device + safety + stream)
    }

    /// False once the device went away (an error reading it counts as gone)
    pub fn alive(self) -> bool {
        get_pod::<u32>(self.0, kAudioDevicePropertyDeviceIsAlive, kAudioObjectPropertyScopeGlobal, "DeviceIsAlive")
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    pub fn describe(self) -> Result<Description> {
        Ok(Description {
            device: self,
            uid: self.uid()?,
            name: self.name()?,
            inputs: self.channels(Scope::Input).unwrap_or(0),
            outputs: self.channels(Scope::Output).unwrap_or(0),
            rates: self.rate_ranges().unwrap_or_default(),
            nominal_rate: self.nominal_rate().unwrap_or(0.0),
        })
    }

    /// Calls `f` (on a CoreAudio notification thread: set a flag, nothing more) when the event happens, until the
    /// returned [`Watch`] is dropped
    pub fn watch(self, event: Event, f: impl Fn() + Send + Sync + 'static) -> Result<Watch> {
        let (a, f): (AudioObjectPropertyAddress, Box<dyn Fn() + Send + Sync>) = match event {
            Event::Gone => (
                address(kAudioDevicePropertyDeviceIsAlive, kAudioObjectPropertyScopeGlobal),
                // The property changes when the device dies; an unreadable device is dead too
                Box::new(move || {
                    if !self.alive() {
                        f();
                    }
                }),
            ),
            Event::Overload => (address(kAudioDeviceProcessorOverload, kAudioObjectPropertyScopeGlobal), Box::new(f)),
        };
        let client = Box::into_raw(Box::new(f));
        // SAFETY: `client` is a live boxed closure that is freed only by `Watch::drop`, after the listener is removed.
        let st = unsafe { AudioObjectAddPropertyListener(self.0, &a, listener_trampoline, client.cast()) };
        if let Err(e) = check("AudioObjectAddPropertyListener", st) {
            // SAFETY: registration failed: nobody else holds the pointer.
            drop(unsafe { Box::from_raw(client) });
            return Err(e);
        }
        Ok(Watch { device: self, address: a, client })
    }
}

/// What a [`Watch`] listens for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The device went away (unplugged, or its driver died): U11
    Gone,
    /// The IO proc did not finish in time (`kAudioDeviceProcessorOverload`): an xrun
    Overload,
}

type Listener = Box<dyn Fn() + Send + Sync>;

/// Runs a callback body, never unwinding into CoreAudio: `default` is what the callback returns if it panics. Every
/// trampoline here runs its whole body through this
fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

unsafe extern "C" fn listener_trampoline(
    _id: AudioObjectID,
    _count: u32,
    _addresses: *const AudioObjectPropertyAddress,
    client: *mut c_void,
) -> OSStatus {
    // SAFETY: `client` is the boxed closure of a live `Watch` (freed only after the listener is removed).
    let f: &dyn Fn() = unsafe { &**(client as *const Listener) };
    // Unwinding into CoreAudio is undefined behaviour
    guard(0, || {
        f();
        0
    })
}

/// A registered property listener
pub struct Watch {
    device: Device,
    address: AudioObjectPropertyAddress,
    client: *mut Listener,
}

// SAFETY: the closure inside is `Send + Sync`; the raw pointer is only used to free it.
unsafe impl Send for Watch {}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch").field("device", &self.device).finish()
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: the same arguments as the registration; after the removal CoreAudio does not call us again, so
        // the closure can be freed. A device that is already gone returns an error and has no listeners left.
        unsafe {
            let status = AudioObjectRemovePropertyListener(
                self.device.0,
                &self.address,
                listener_trampoline,
                self.client.cast(),
            );
            let gone = check("AudioObjectRemovePropertyListener", status).is_err_and(|e| e.is_disconnected());
            // Any other failure: the listener may still be called, so the closure is leaked rather than freed
            if status == 0 || gone {
                drop(Box::from_raw(self.client));
            }
        }
    }
}

/// Every device with its properties (U17). Can take a while with USB devices: run it in an `Operation`
pub fn list() -> Result<Vec<Description>> {
    Ok(Device::all()?.into_iter().filter_map(|d| d.describe().ok()).filter(|d| d.inputs + d.outputs > 0).collect())
}

// ---- the host clock ----

fn timebase() -> (u32, u32) {
    static TB: OnceLock<(u32, u32)> = OnceLock::new();
    *TB.get_or_init(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: `info` is a valid out-parameter.
        unsafe { mach_timebase_info(&mut info) };
        if info.denom == 0 { (1, 1) } else { (info.numer, info.denom) }
    })
}

/// `mach_absolute_time` ticks (what CoreAudio's host times are in) to nanoseconds. Real-time safe once any [`Io`]
/// was made (the time base is read then)
pub fn host_nanos(ticks: u64) -> u64 {
    let (n, d) = timebase();
    (u128::from(ticks) * u128::from(n) / u128::from(d)) as u64
}

/// The current `mach_absolute_time`, in ticks. Real-time safe
pub fn host_ticks() -> u64 {
    // SAFETY: no arguments, no state.
    unsafe { mach_absolute_time() }
}

// ---- the IO proc ----

/// One cycle of the IO proc: the buffers CoreAudio gave and the times
#[derive(Debug)]
pub struct Cycle<'a> {
    input: *const AudioBufferList,
    output: *mut AudioBufferList,
    now: u64,
    input_time: Option<u64>,
    output_time: Option<u64>,
    _life: std::marker::PhantomData<&'a mut ()>,
}

impl Cycle<'_> {
    /// `mach_absolute_time` ticks when the proc was called
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Ticks when the first input frame was captured (in the past)
    pub fn input_time(&self) -> Option<u64> {
        self.input_time
    }

    /// Ticks when the first output frame will be played (in the future)
    pub fn output_time(&self) -> Option<u64> {
        self.output_time
    }

    pub fn input_streams(&self) -> usize {
        if self.input.is_null() {
            return 0;
        }
        // SAFETY: CoreAudio gives a valid list for the duration of the proc.
        unsafe { (*self.input).mNumberBuffers as usize }
    }

    pub fn output_streams(&self) -> usize {
        if self.output.is_null() {
            return 0;
        }
        // SAFETY: as above.
        unsafe { (*self.output).mNumberBuffers as usize }
    }

    /// Stream `i` of the input: its channel count and its interleaved `f32` samples (`channels` per frame)
    pub fn input(&self, i: usize) -> Option<(u32, &[f32])> {
        if i >= self.input_streams() {
            return None;
        }
        // SAFETY: `i` is below `mNumberBuffers`; the data pointer and size describe the buffer CoreAudio owns until
        // the proc returns, aligned for f32 (CoreAudio's buffers are).
        unsafe {
            let b = *(*self.input).mBuffers.as_ptr().add(i);
            if b.mData.is_null() {
                return Some((b.mNumberChannels, &[]));
            }
            Some((b.mNumberChannels, std::slice::from_raw_parts(b.mData as *const f32, b.mDataByteSize as usize / 4)))
        }
    }

    /// Stream `i` of the output: its channel count and its interleaved `f32` samples to write
    pub fn output(&mut self, i: usize) -> Option<(u32, &mut [f32])> {
        if i >= self.output_streams() {
            return None;
        }
        // SAFETY: as `input`; the borrow of `self` keeps two streams from being held at once.
        unsafe {
            let b = *(*self.output).mBuffers.as_ptr().add(i);
            if b.mData.is_null() {
                return Some((b.mNumberChannels, &mut []));
            }
            Some((b.mNumberChannels, std::slice::from_raw_parts_mut(b.mData as *mut f32, b.mDataByteSize as usize / 4)))
        }
    }

    /// Fills every output with silence
    pub fn silence(&mut self) {
        for i in 0..self.output_streams() {
            if let Some((_, s)) = self.output(i) {
                s.fill(0.0);
            }
        }
    }
}

type Proc = Box<dyn FnMut(&mut Cycle<'_>) + Send>;

struct IoState {
    callback: Proc,
    panicked: AtomicBool,
    cycles: AtomicU32,
}

unsafe extern "C" fn io_trampoline(
    _device: AudioObjectID,
    now: *const AudioTimeStamp,
    input: *const AudioBufferList,
    input_time: *const AudioTimeStamp,
    output: *mut AudioBufferList,
    output_time: *const AudioTimeStamp,
    client: *mut c_void,
) -> OSStatus {
    // SAFETY: `client` is the `IoState` of a live `Io`, which stops the proc before freeing it. Only this proc
    // touches `callback`.
    let state = unsafe { &mut *(client as *mut IoState) };
    guard(0, || {
        let host = |t: *const AudioTimeStamp| {
            // SAFETY: a non-null time stamp is valid for the call.
            (!t.is_null())
                .then(|| unsafe { *t })
                .filter(|t| t.mFlags & kAudioTimeStampHostTimeValid != 0)
                .map(|t| t.mHostTime)
        };
        let mut cycle = Cycle {
            input,
            output,
            now: host(now).unwrap_or_else(host_ticks),
            input_time: host(input_time),
            output_time: host(output_time),
            _life: std::marker::PhantomData,
        };
        if state.panicked.load(Ordering::Relaxed) {
            cycle.silence();
            return 0;
        }
        let r = catch_unwind(AssertUnwindSafe(|| (state.callback)(&mut cycle)));
        if r.is_err() {
            state.panicked.store(true, Ordering::Release);
            cycle.silence();
        }
        state.cycles.fetch_add(1, Ordering::Relaxed);
        0
    })
}

/// An IO proc on a device: the closure runs once per buffer on CoreAudio's real-time thread
pub struct Io {
    device: Device,
    id: AudioDeviceIOProcID,
    state: *mut IoState,
    running: bool,
}

// SAFETY: the raw pointers are owned by this value; the closure inside is `Send`; the calls are thread-safe.
unsafe impl Send for Io {}

impl std::fmt::Debug for Io {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Io").field("device", &self.device).field("running", &self.running).finish()
    }
}

impl Io {
    /// Registers the closure on `device`. `inputs: false` turns the device's input streams off for this proc, so
    /// macOS does not ask for the microphone permission when only playback is wanted
    pub fn new(device: Device, inputs: bool, callback: impl FnMut(&mut Cycle<'_>) + Send + 'static) -> Result<Self> {
        let _ = timebase();
        let state = Box::into_raw(Box::new(IoState {
            callback: Box::new(callback),
            panicked: AtomicBool::new(false),
            cycles: AtomicU32::new(0),
        }));
        let mut id: AudioDeviceIOProcID = null_mut();
        // SAFETY: `state` is live until `Io::drop`; `id` is a valid out-parameter.
        let st = unsafe { AudioDeviceCreateIOProcID(device.0, io_trampoline, state.cast(), &mut id) };
        if let Err(e) = check("AudioDeviceCreateIOProcID", st) {
            // SAFETY: not registered: nobody else holds it.
            drop(unsafe { Box::from_raw(state) });
            return Err(e);
        }
        let io = Self { device, id, state, running: false };
        if !inputs {
            // Not fatal: if the system refuses, the device still works (and may ask for the permission)
            let _ = io.disable_inputs();
        }
        Ok(io)
    }

    /// Turns every input stream off for this proc (`kAudioDevicePropertyIOProcStreamUsage`)
    fn disable_inputs(&self) -> Result<()> {
        let streams = get_vec::<u32>(
            self.device.0,
            kAudioDevicePropertyStreams,
            kAudioObjectPropertyScopeInput,
            "Streams(Input)",
        )?;
        if streams.is_empty() {
            return Ok(());
        }
        // The struct is { proc: pointer, count: u32, on: [u32; count] }: 12 bytes + 4 per stream, 8-aligned
        let bytes = 12 + 4 * streams.len();
        let mut raw = vec![0u64; bytes.div_ceil(8)];
        let base = raw.as_mut_ptr().cast::<u8>();
        // SAFETY: `raw` is zeroed and has `bytes` bytes; the head and the flags are written inside it. Zero flags
        // mean "stream off".
        unsafe {
            let head = base.cast::<StreamUsageHead>();
            (*head).mIOProc = self.id;
            (*head).mNumberStreams = streams.len() as u32;
        }
        let a = address(kAudioDevicePropertyIOProcStreamUsage, kAudioObjectPropertyScopeInput);
        // SAFETY: `raw` is readable for `bytes` bytes and lives through the call.
        let st = unsafe { AudioObjectSetPropertyData(self.device.0, &a, 0, null(), bytes as u32, raw.as_ptr().cast()) };
        check("SetPropertyData(IOProcStreamUsage)", st)
    }

    pub fn start(&mut self) -> Result<()> {
        // SAFETY: the device and the proc id are the ones registered.
        check("AudioDeviceStart", unsafe { AudioDeviceStart(self.device.0, self.id) })?;
        self.running = true;
        Ok(())
    }

    /// Stops the device. Returns after the last call of the closure has returned (CoreAudio waits for it)
    pub fn stop(&mut self) -> Result<()> {
        if !self.running {
            return Ok(());
        }
        self.running = false;
        // SAFETY: as `start`.
        let st = unsafe { AudioDeviceStop(self.device.0, self.id) };
        // A device that is already gone says so: it is stopped all the same
        match check("AudioDeviceStop", st) {
            Err(e) if e.is_disconnected() || e.status == kAudioHardwareNotRunningError => Ok(()),
            other => other,
        }
    }

    /// The closure panicked (a bug): the outputs are silent from then on
    pub fn panicked(&self) -> bool {
        // SAFETY: `state` is live until drop.
        unsafe { (*self.state).panicked.load(Ordering::Acquire) }
    }

    /// Cycles the proc has completed
    pub fn cycles(&self) -> u32 {
        // SAFETY: as above.
        unsafe { (*self.state).cycles.load(Ordering::Relaxed) }
    }
}

impl Drop for Io {
    fn drop(&mut self) {
        let stopped = self.stop().is_ok();
        // SAFETY: the state is freed only when the proc is certainly gone: stopped and destroyed.
        unsafe {
            // Never destroy a proc that may still run
            let destroyed = stopped && AudioDeviceDestroyIOProcID(self.device.0, self.id) == 0;
            if destroyed {
                drop(Box::from_raw(self.state));
            }
            // else: the proc may still be called from the real-time thread, so the state is leaked on purpose
            // (a few bytes) rather than freed under it
        }
    }
}

// ---- the explicit aggregate ----

/// A private aggregate device of two devices, **made because the user chose that combination** (U7: nothing is
/// ever joined behind the user's back). The first device is the clock master; the second follows it with drift
/// compensation. Private: it is not listed to other programs and is destroyed on drop
pub struct Aggregate {
    id: u32,
}

impl std::fmt::Debug for Aggregate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aggregate").field("id", &self.id).finish()
    }
}

impl Aggregate {
    /// `master` (the playback device, in the stream order first) and `follower` (the capture device, drift
    /// compensated). Waits until the aggregate shows its streams
    pub fn create(master: Device, follower: Device) -> Result<Self> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let master_uid = master.uid()?;
        let follower_uid = follower.uid()?;
        let uid =
            format!("com.linuxmusic.aggregate.{}.{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
        let sub = |uid: &str, drift: i32| {
            let (k_uid, k_drift) = (cf_string("uid"), cf_string("drift"));
            let (v_uid, v_drift) = (cf_string(uid), cf_number(drift));
            cf_dictionary(&[(&k_uid, &v_uid), (&k_drift, &v_drift)])
        };
        let list = cf_array(&[&sub(&master_uid, 0), &sub(&follower_uid, 1)]);
        let (k_uid, v_uid) = (cf_string("uid"), cf_string(&uid));
        let (k_name, v_name) = (cf_string("name"), cf_string("LinuxMusic"));
        let (k_private, v_private) = (cf_string("private"), cf_number(1));
        let (k_stacked, v_stacked) = (cf_string("stacked"), cf_number(0));
        let (k_master, v_master) = (cf_string("master"), cf_string(&master_uid));
        let k_subs = cf_string("subdevices");
        let description = cf_dictionary(&[
            (&k_uid, &v_uid),
            (&k_name, &v_name),
            (&k_private, &v_private),
            (&k_stacked, &v_stacked),
            (&k_master, &v_master),
            (&k_subs, &list),
        ]);
        let mut id: u32 = 0;
        // SAFETY: `description` is a valid CFDictionary for the call; `id` is a valid out-parameter.
        let st = unsafe { AudioHardwareCreateAggregateDevice(description.0.cast(), &mut id) };
        check("AudioHardwareCreateAggregateDevice", st)?;
        let aggregate = Self { id };
        // The sub-devices take a moment to show their streams
        let start = Instant::now();
        let device = aggregate.device();
        while device.channels(Scope::Output).unwrap_or(0) + device.channels(Scope::Input).unwrap_or(0) == 0 {
            if start.elapsed() > Duration::from_secs(3) {
                return Err(CaError { call: "aggregate streams", status: kAudioHardwareIllegalOperationError });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(aggregate)
    }

    pub fn device(&self) -> Device {
        Device(self.id)
    }
}

impl Drop for Aggregate {
    fn drop(&mut self) {
        // SAFETY: the id is the one created, destroyed once.
        let _ = unsafe { AudioHardwareDestroyAggregateDevice(self.id) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_char_codes() {
        assert_eq!(fourcc(b"glob"), 0x676c_6f62);
        assert_eq!(CaError { call: "x", status: kAudioHardwareBadDeviceError }.code(), "!dev");
        assert!(CaError { call: "x", status: kAudioHardwareBadDeviceError }.is_disconnected());
    }

    #[test]
    fn host_clock_moves() {
        let _ = timebase();
        let a = host_nanos(host_ticks());
        let b = host_nanos(host_ticks());
        assert!(b >= a);
    }

    #[test]
    fn lists_devices_without_panicking() {
        // On a machine with no audio (CI) the list may be empty; it must still be an Ok or a plain error
        let _ = list();
    }
}
