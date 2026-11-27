//! The objects the host gives a VST3 plugin, as COM classes: the host application (and the messages and attribute
//! lists it makes for the plugin's two halves), the component handler, the parameter change lists and queues, the
//! event list, and a byte stream for the state.
//!
//! The change lists, queues and event list are made once, with fixed capacity, and reused at every block: nothing
//! is allocated while processing.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ffi::{CStr, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComRef, ComWrapper, Interface};

/// Runs the body of a COM method, never unwinding into the plugin: a panic across the plugin's C frames is not
/// survivable, so it becomes `default` (for a `tresult`, `kInternalError`; for a pointer, null). Every method of
/// every class below runs its whole body through this
pub(crate) fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(default)
}

/// An interface id as a TUID (the same 16 bytes)
pub(crate) fn tuid(g: &[u8; 16]) -> TUID {
    g.map(|b| b as std::ffi::c_char)
}

/// Writes a UTF-16 string into a fixed buffer, NUL-terminated
pub(crate) fn put_wide(src: &str, dst: &mut [TChar]) {
    let mut n = 0;
    for (s, d) in src.encode_utf16().zip(dst.iter_mut()) {
        *d = s as TChar;
        n += 1;
    }
    if n < dst.len() {
        dst[n] = 0;
    } else if let Some(last) = dst.last_mut() {
        *last = 0;
    }
}

/// A fixed UTF-16 buffer as a string, up to its NUL
pub(crate) fn wide(src: &[TChar]) -> String {
    let n = src.iter().position(|c| *c == 0).unwrap_or(src.len());
    String::from_utf16_lossy(&src[..n])
}

/// A fixed 8-bit buffer as a string, up to its NUL
pub(crate) fn narrow(src: &[char8]) -> String {
    let bytes: Vec<u8> = src.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---------------------------------------------------------------------------------------------------------------
// The host application, its messages and attribute lists

pub(crate) struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        guard(kInternalError, || {
            if name.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: the plugin gives a String128 to fill
            put_wide("LinuxMusic", unsafe { &mut *name });
            kResultOk
        })
    }

    unsafe fn createInstance(&self, cid: *mut TUID, iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        guard(kInternalError, || {
            if cid.is_null() || iid.is_null() || obj.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: the plugin gives two TUIDs and an out pointer
            let (cid, iid) = unsafe { (*cid, *iid) };
            let made: Option<ComPtr<FUnknown>> = if cid == tuid(&IMessage::IID) && iid == tuid(&IMessage::IID) {
                ComWrapper::new(Message::default()).to_com_ptr::<FUnknown>()
            } else if cid == tuid(&IAttributeList::IID) && iid == tuid(&IAttributeList::IID) {
                ComWrapper::new(Attributes::default()).to_com_ptr::<FUnknown>()
            } else {
                None
            };
            match made {
                Some(p) => {
                    let raw = p.as_ptr();
                    let mut iid = iid;
                    // SAFETY: a live object of ours; queryInterface adds the reference the plugin owns
                    unsafe { ((*(*raw).vtbl).queryInterface)(raw, &mut iid, obj) }
                }
                None => {
                    // SAFETY: the out pointer the plugin gave
                    unsafe { *obj = std::ptr::null_mut() };
                    kNoInterface
                }
            }
        })
    }
}

#[derive(Default)]
pub(crate) struct Message {
    id: RefCell<Option<std::ffi::CString>>,
    attributes: RefCell<Option<ComPtr<IAttributeList>>>,
}

impl Class for Message {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        guard(std::ptr::null(), || {
            self.id.borrow().as_ref().map_or(std::ptr::null(), |c| c.as_ptr())
        })
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        guard((), || {
            let v = if id.is_null() {
                None
            } else {
                // SAFETY: a NUL-terminated id from the plugin, copied now
                Some(unsafe { CStr::from_ptr(id) }.to_owned())
            };
            *self.id.borrow_mut() = v;
        })
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        guard(std::ptr::null_mut(), || {
            let mut a = self.attributes.borrow_mut();
            if a.is_none() {
                *a = ComWrapper::new(Attributes::default()).to_com_ptr::<IAttributeList>();
            }
            // Borrowed by the plugin (no reference added, as the SDK's own host does)
            a.as_ref().map_or(std::ptr::null_mut(), |p| p.as_ptr())
        })
    }
}

#[derive(Clone)]
enum Attr {
    Int(i64),
    Float(f64),
    Text(Vec<TChar>),
    Binary(Vec<u8>),
}

#[derive(Default)]
pub(crate) struct Attributes {
    map: RefCell<BTreeMap<Vec<u8>, Attr>>,
}

impl Class for Attributes {
    type Interfaces = (IAttributeList,);
}

fn key(id: IAttributeList_::AttrID) -> Option<Vec<u8>> {
    // SAFETY: a NUL-terminated id from the plugin, copied now
    (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_bytes().to_vec())
}

impl IAttributeListTrait for Attributes {
    unsafe fn setInt(&self, id: IAttributeList_::AttrID, value: int64) -> tresult {
        guard(kInternalError, || {
            let Some(k) = key(id) else { return kInvalidArgument };
            self.map.borrow_mut().insert(k, Attr::Int(value));
            kResultOk
        })
    }

    unsafe fn getInt(&self, id: IAttributeList_::AttrID, value: *mut int64) -> tresult {
        guard(kInternalError, || {
            match key(id).and_then(|k| self.map.borrow().get(&k).cloned()) {
                Some(Attr::Int(v)) if !value.is_null() => {
                    // SAFETY: the plugin's out pointer
                    unsafe { *value = v };
                    kResultOk
                }
                _ => kResultFalse,
            }
        })
    }

    unsafe fn setFloat(&self, id: IAttributeList_::AttrID, value: f64) -> tresult {
        guard(kInternalError, || {
            let Some(k) = key(id) else { return kInvalidArgument };
            self.map.borrow_mut().insert(k, Attr::Float(value));
            kResultOk
        })
    }

    unsafe fn getFloat(&self, id: IAttributeList_::AttrID, value: *mut f64) -> tresult {
        guard(kInternalError, || {
            match key(id).and_then(|k| self.map.borrow().get(&k).cloned()) {
                Some(Attr::Float(v)) if !value.is_null() => {
                    // SAFETY: the plugin's out pointer
                    unsafe { *value = v };
                    kResultOk
                }
                _ => kResultFalse,
            }
        })
    }

    unsafe fn setString(&self, id: IAttributeList_::AttrID, string: *const TChar) -> tresult {
        guard(kInternalError, || {
            let Some(k) = key(id) else { return kInvalidArgument };
            if string.is_null() {
                return kInvalidArgument;
            }
            let mut v = Vec::new();
            for i in 0..65_536 {
                // SAFETY: a NUL-terminated UTF-16 string from the plugin; bounded
                let c = unsafe { *string.add(i) };
                if c == 0 {
                    break;
                }
                v.push(c);
            }
            self.map.borrow_mut().insert(k, Attr::Text(v));
            kResultOk
        })
    }

    unsafe fn getString(&self, id: IAttributeList_::AttrID, string: *mut TChar, size_in_bytes: uint32) -> tresult {
        guard(kInternalError, || {
            match key(id).and_then(|k| self.map.borrow().get(&k).cloned()) {
                Some(Attr::Text(v)) if !string.is_null() && size_in_bytes >= 2 => {
                    let room = size_in_bytes as usize / 2;
                    let n = v.len().min(room - 1);
                    // SAFETY: the plugin's buffer of `size_in_bytes` bytes; n + 1 ≤ room characters written
                    unsafe {
                        std::ptr::copy_nonoverlapping(v.as_ptr(), string, n);
                        *string.add(n) = 0;
                    }
                    kResultOk
                }
                _ => kResultFalse,
            }
        })
    }

    unsafe fn setBinary(&self, id: IAttributeList_::AttrID, data: *const c_void, size_in_bytes: uint32) -> tresult {
        guard(kInternalError, || {
            let Some(k) = key(id) else { return kInvalidArgument };
            let v = if data.is_null() || size_in_bytes == 0 {
                Vec::new()
            } else {
                // SAFETY: the plugin's `size_in_bytes` readable bytes, copied now
                unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size_in_bytes as usize) }.to_vec()
            };
            self.map.borrow_mut().insert(k, Attr::Binary(v));
            kResultOk
        })
    }

    unsafe fn getBinary(
        &self,
        id: IAttributeList_::AttrID,
        data: *mut *const c_void,
        size_in_bytes: *mut uint32,
    ) -> tresult {
        guard(kInternalError, || {
            let Some(k) = key(id) else { return kInvalidArgument };
            let map = self.map.borrow();
            match map.get(&k) {
                Some(Attr::Binary(v)) if !data.is_null() && !size_in_bytes.is_null() => {
                    // SAFETY: the plugin's out pointers; the bytes live as long as this list (the SDK's contract)
                    unsafe {
                        *data = v.as_ptr().cast();
                        *size_in_bytes = v.len() as uint32;
                    }
                    kResultOk
                }
                _ => kResultFalse,
            }
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The component handler

/// What the plugin's controller asked of the host
#[derive(Debug, Default)]
pub(crate) struct Requests {
    pub restart: AtomicBool,
    pub latency: AtomicBool,
    pub params: AtomicBool,
    pub dirty: AtomicBool,
}

/// One step of the controller's own editing (its window): a gesture is `Begin`, any number of `Set`, `End`
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Edit {
    Begin(ParamID),
    Set(ParamID, f64),
    End(ParamID),
}

pub(crate) struct Handler {
    pub requests: std::sync::Arc<Requests>,
    /// The controller's own edits (its window): to the processor at the next block, and to the core
    pub edits: Mutex<rtrb::Producer<Edit>>,
}

impl Class for Handler {
    type Interfaces = (IComponentHandler, IComponentHandler2);
}

impl IComponentHandlerTrait for Handler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        guard(kInternalError, || {
            let mut p = self.edits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = p.push(Edit::Begin(id));
            kResultOk
        })
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        guard(kInternalError, || {
            let mut p = self.edits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = p.push(Edit::Set(id, value));
            kResultOk
        })
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        guard(kInternalError, || {
            let mut p = self.edits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = p.push(Edit::End(id));
            kResultOk
        })
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        guard(kInternalError, || {
            let f = flags as u32;
            if f & (RestartFlags_::kLatencyChanged as u32) != 0 {
                self.requests.latency.store(true, Ordering::Release);
                self.requests.restart.store(true, Ordering::Release);
            }
            if f & (RestartFlags_::kIoChanged as u32 | RestartFlags_::kReloadComponent as u32) != 0 {
                self.requests.restart.store(true, Ordering::Release);
            }
            if f & (RestartFlags_::kParamValuesChanged as u32) != 0 {
                self.requests.params.store(true, Ordering::Release);
            }
            kResultOk
        })
    }
}

impl IComponentHandler2Trait for Handler {
    unsafe fn setDirty(&self, state: TBool) -> tresult {
        guard(kInternalError, || {
            if state != 0 {
                self.requests.dirty.store(true, Ordering::Release);
            }
            kResultOk
        })
    }

    unsafe fn requestOpenEditor(&self, _name: FIDString) -> tresult {
        guard(kInternalError, || {
            kNotImplemented
        })
    }

    unsafe fn startGroupEdit(&self) -> tresult {
        guard(kInternalError, || {
            kResultOk
        })
    }

    unsafe fn finishGroupEdit(&self) -> tresult {
        guard(kInternalError, || {
            kResultOk
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Parameter changes

/// The most points one parameter gets in one block
pub(crate) const MAX_POINTS: usize = 64;

pub(crate) struct Queue {
    id: Cell<ParamID>,
    points: RefCell<Vec<(i32, f64)>>,
}

impl Class for Queue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for Queue {
    unsafe fn getParameterId(&self) -> ParamID {
        guard(0, || {
            self.id.get()
        })
    }

    unsafe fn getPointCount(&self) -> int32 {
        guard(0, || {
            self.points.borrow().len() as int32
        })
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        guard(kInternalError, || {
            match self.points.borrow().get(index as usize) {
                Some((o, v)) if !offset.is_null() && !value.is_null() => {
                    // SAFETY: the plugin's out pointers
                    unsafe {
                        *offset = *o;
                        *value = *v;
                    }
                    kResultOk
                }
                _ => kInvalidArgument,
            }
        })
    }

    unsafe fn addPoint(&self, offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        guard(kInternalError, || {
            let mut p = self.points.borrow_mut();
            if p.len() == p.capacity() {
                return kResultFalse;
            }
            // In order of offset; the same offset replaces
            let at = p.iter().position(|(o, _)| *o >= offset).unwrap_or(p.len());
            if p.get(at).is_some_and(|(o, _)| *o == offset) {
                p[at].1 = value;
            } else {
                p.insert(at, (offset, value));
            }
            if !index.is_null() {
                // SAFETY: the plugin's out pointer
                unsafe { *index = at as int32 };
            }
            kResultOk
        })
    }
}

/// A list of parameter changes for one block, with fixed capacity
pub(crate) struct Changes {
    queues: Vec<ComWrapper<Queue>>,
    used: Cell<usize>,
}

impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}

impl Changes {
    pub fn new(capacity: usize) -> Changes {
        let queues = (0..capacity.max(1))
            .map(|_| ComWrapper::new(Queue { id: Cell::new(0), points: RefCell::new(Vec::with_capacity(MAX_POINTS)) }))
            .collect();
        Changes { queues, used: Cell::new(0) }
    }

    pub fn clear(&self) {
        for q in &self.queues[..self.used.get()] {
            q.points.borrow_mut().clear();
        }
        self.used.set(0);
    }

    fn queue_of(&self, id: ParamID) -> Option<usize> {
        let n = self.used.get();
        if let Some(i) = self.queues[..n].iter().position(|q| q.id.get() == id) {
            return Some(i);
        }
        if n == self.queues.len() {
            return None;
        }
        self.queues[n].id.set(id);
        self.used.set(n + 1);
        Some(n)
    }

    /// Adds a point (the host's side). No allocation
    pub fn add(&self, id: ParamID, offset: i32, value: f64) {
        if let Some(i) = self.queue_of(id) {
            let mut idx = 0;
            // SAFETY: our own queue, no pointer escapes
            unsafe { self.queues[i].addPoint(offset, value, &mut idx) };
        }
    }

    /// Each parameter's last point (what the plugin said)
    pub fn last_points(&self, mut f: impl FnMut(ParamID, f64)) {
        for q in &self.queues[..self.used.get()] {
            if let Some((_, v)) = q.points.borrow().last() {
                f(q.id.get(), *v);
            }
        }
    }
}

impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> int32 {
        guard(0, || {
            self.used.get() as int32
        })
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        guard(std::ptr::null_mut(), || {
            if index < 0 || index as usize >= self.used.get() {
                return std::ptr::null_mut();
            }
            self.queues[index as usize].as_com_ref::<IParamValueQueue>().map_or(std::ptr::null_mut(), |r| r.as_ptr())
        })
    }

    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
        guard(std::ptr::null_mut(), || {
            if id.is_null() {
                return std::ptr::null_mut();
            }
            // SAFETY: the plugin's id
            let Some(i) = self.queue_of(unsafe { *id }) else { return std::ptr::null_mut() };
            if !index.is_null() {
                // SAFETY: the plugin's out pointer
                unsafe { *index = i as int32 };
            }
            self.queues[i].as_com_ref::<IParamValueQueue>().map_or(std::ptr::null_mut(), |r| r.as_ptr())
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Events

pub(crate) struct Events {
    pub list: RefCell<Vec<Event>>,
}

impl Class for Events {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for Events {
    unsafe fn getEventCount(&self) -> int32 {
        guard(0, || {
            self.list.borrow().len() as int32
        })
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        guard(kInternalError, || {
            match self.list.borrow().get(index as usize) {
                Some(ev) if !e.is_null() => {
                    // SAFETY: the plugin's out pointer; Event is plain data
                    unsafe { std::ptr::copy_nonoverlapping(ev, e, 1) };
                    kResultOk
                }
                _ => kInvalidArgument,
            }
        })
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        guard(kInternalError, || {
            let mut l = self.list.borrow_mut();
            if e.is_null() || l.len() == l.capacity() {
                return kResultFalse;
            }
            // SAFETY: the plugin's event, copied
            l.push(unsafe { std::ptr::read(e) });
            kResultOk
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// A byte stream

pub(crate) struct Stream {
    pub data: RefCell<Vec<u8>>,
    pub at: Cell<usize>,
}

impl Class for Stream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for Stream {
    unsafe fn read(&self, buffer: *mut c_void, n: int32, read: *mut int32) -> tresult {
        guard(kInternalError, || {
            if buffer.is_null() || n < 0 {
                return kInvalidArgument;
            }
            let d = self.data.borrow();
            let at = self.at.get().min(d.len());
            let k = (n as usize).min(d.len() - at);
            // SAFETY: the plugin's buffer of n bytes; k ≤ n
            unsafe { std::ptr::copy_nonoverlapping(d[at..].as_ptr(), buffer.cast::<u8>(), k) };
            self.at.set(at + k);
            if !read.is_null() {
                // SAFETY: the plugin's out pointer
                unsafe { *read = k as int32 };
            }
            kResultOk
        })
    }

    unsafe fn write(&self, buffer: *mut c_void, n: int32, written: *mut int32) -> tresult {
        guard(kInternalError, || {
            if buffer.is_null() || n < 0 {
                return kInvalidArgument;
            }
            // SAFETY: the plugin's n readable bytes
            let bytes = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n as usize) };
            let mut d = self.data.borrow_mut();
            let at = self.at.get();
            // The position and the size come from the plugin: bounded, so a wild seek cannot ask for a huge allocation
            let Some(end) = at.checked_add(bytes.len()).filter(|e| *e <= MAX_STREAM) else {
                return kResultFalse;
            };
            if d.len() < end {
                d.resize(end, 0);
            }
            d[at..end].copy_from_slice(bytes);
            self.at.set(end);
            if !written.is_null() {
                // SAFETY: the plugin's out pointer
                unsafe { *written = n };
            }
            kResultOk
        })
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        guard(kInternalError, || {
            let len = self.data.borrow().len() as i64;
            let base = match mode as u32 {
                m if m == IBStream_::IStreamSeekMode_::kIBSeekSet => 0,
                m if m == IBStream_::IStreamSeekMode_::kIBSeekCur => self.at.get() as i64,
                m if m == IBStream_::IStreamSeekMode_::kIBSeekEnd => len,
                _ => return kInvalidArgument,
            };
            let to = match base.checked_add(pos) {
                Some(to) => usize::try_from(to.max(0)).ok().filter(|t| *t <= MAX_STREAM),
                None => None,
            };
            let Some(to) = to else { return kInvalidArgument };
            self.at.set(to);
            if !result.is_null() {
                // SAFETY: the plugin's out pointer
                unsafe { *result = to as int64 };
            }
            kResultOk
        })
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        guard(kInternalError, || {
            if pos.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: the plugin's out pointer
            unsafe { *pos = self.at.get() as int64 };
            kResultOk
        })
    }
}

/// The most a plugin can make a state stream grow to (or seek to)
const MAX_STREAM: usize = 256 << 20;

impl Stream {
    pub fn new(data: Vec<u8>) -> ComWrapper<Stream> {
        ComWrapper::new(Stream { data: RefCell::new(data), at: Cell::new(0) })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The editor's frame: the host side of a plugin's window (resize requests; on Linux also the run loop it needs)

struct Timer {
    handler: ComPtr<Linux::ITimerHandler>,
    every: std::time::Duration,
    next: std::time::Instant,
}

/// What the plugin's view calls on the host. Main thread only (VST3's UI thread): plain cells, no locks
#[derive(Default)]
pub(crate) struct Frame {
    /// The size the view asked for (`resizeView`), taken by the helper's loop
    pub resize: Cell<Option<(u32, u32)>>,
    handlers: RefCell<Vec<(ComPtr<Linux::IEventHandler>, i32)>>,
    timers: RefCell<Vec<Timer>>,
}

impl Class for Frame {
    type Interfaces = (IPlugFrame, Linux::IRunLoop);
}

impl IPlugFrameTrait for Frame {
    unsafe fn resizeView(&self, _view: *mut IPlugView, new_size: *mut ViewRect) -> tresult {
        guard(kInternalError, || {
            if new_size.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: the view's rect, valid for the call
            let r = unsafe { *new_size };
            let (w, h) = (r.right.saturating_sub(r.left), r.bottom.saturating_sub(r.top));
            self.resize.set(Some((w.max(1) as u32, h.max(1) as u32)));
            kResultOk
        })
    }
}

impl Linux::IRunLoopTrait for Frame {
    unsafe fn registerEventHandler(&self, handler: *mut Linux::IEventHandler, fd: Linux::FileDescriptor) -> tresult {
        guard(kInternalError, || {
            // SAFETY: the plugin's handler, valid for the call; `to_com_ptr` takes our own reference
            let Some(h) = (unsafe { ComRef::from_raw(handler) }) else { return kInvalidArgument };
            self.handlers.borrow_mut().push((h.to_com_ptr(), fd));
            kResultOk
        })
    }

    unsafe fn unregisterEventHandler(&self, handler: *mut Linux::IEventHandler) -> tresult {
        guard(kInternalError, || {
            self.handlers.borrow_mut().retain(|(h, _)| h.as_ptr() != handler);
            kResultOk
        })
    }

    unsafe fn registerTimer(&self, handler: *mut Linux::ITimerHandler, milliseconds: Linux::TimerInterval) -> tresult {
        guard(kInternalError, || {
            // SAFETY: the plugin's handler, valid for the call; `to_com_ptr` takes our own reference
            let Some(h) = (unsafe { ComRef::from_raw(handler) }) else { return kInvalidArgument };
            let every = std::time::Duration::from_millis(milliseconds.max(1));
            self.timers.borrow_mut().push(Timer {
                handler: h.to_com_ptr(),
                every,
                next: std::time::Instant::now() + every,
            });
            kResultOk
        })
    }

    unsafe fn unregisterTimer(&self, handler: *mut Linux::ITimerHandler) -> tresult {
        guard(kInternalError, || {
            self.timers.borrow_mut().retain(|t| t.handler.as_ptr() != handler);
            kResultOk
        })
    }
}

impl Frame {
    /// The plugin's own event loop, run by the helper's main thread: the file descriptors that are readable and the
    /// timers that are due get their callbacks (copies of the lists are walked, so a callback may register or
    /// unregister)
    pub(crate) fn tick(&self) {
        use Linux::{IEventHandlerTrait, ITimerHandlerTrait};
        let handlers: Vec<(ComPtr<Linux::IEventHandler>, i32)> =
            self.handlers.borrow().iter().map(|(h, fd)| (h.clone(), *fd)).collect();
        if !handlers.is_empty() {
            let mut fds: Vec<libc::pollfd> =
                handlers.iter().map(|(_, fd)| libc::pollfd { fd: *fd, events: libc::POLLIN, revents: 0 }).collect();
            // SAFETY: `fds` is a valid array of its length
            let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 0) };
            if n > 0 {
                for ((h, fd), p) in handlers.iter().zip(&fds) {
                    if p.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
                        // SAFETY: the plugin's handler, kept alive by our reference; the main thread
                        unsafe { h.onFDIsSet(*fd) };
                    }
                }
            }
        }
        let now = std::time::Instant::now();
        let due: Vec<ComPtr<Linux::ITimerHandler>> = self
            .timers
            .borrow_mut()
            .iter_mut()
            .filter(|t| now >= t.next)
            .map(|t| {
                t.next = now + t.every;
                t.handler.clone()
            })
            .collect();
        for h in due {
            // SAFETY: the plugin's handler, kept alive by our reference; the main thread
            unsafe { h.onTimer() };
        }
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    #[test]
    fn a_wild_seek_or_write_is_refused_not_allocated() {
        let s = Stream { data: RefCell::new(Vec::new()), at: Cell::new(0) };
        let mut out: int64 = 0;
        // SAFETY: valid out pointer and buffers
        unsafe {
            assert_eq!(s.seek(i64::MAX, IBStream_::IStreamSeekMode_::kIBSeekSet as i32, &mut out), kInvalidArgument);
            assert_eq!(s.seek(1, IBStream_::IStreamSeekMode_::kIBSeekEnd as i32, &mut out), kResultOk);
            s.at.set(MAX_STREAM);
            let mut byte = 1u8;
            assert_eq!(s.write((&mut byte as *mut u8).cast(), 1, std::ptr::null_mut()), kResultFalse);
        }
        assert!(s.data.borrow().is_empty());
    }
}
