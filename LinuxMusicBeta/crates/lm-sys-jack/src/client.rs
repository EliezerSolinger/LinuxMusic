//! The client: opened on a worker thread, its ports registered, then activated with the program's process and
//! notification handlers. The process handler runs on JACK's real-time thread through a trampoline that never lets a
//! panic cross into C and never hands out two references to one buffer: audio is copied in and out.

use std::cell::UnsafeCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use lm_base::{Error, Result};

use crate::ffi;

/// What a port carries
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortKind {
    Audio,
    Midi,
}

/// Which way a port goes, seen from its client: an input receives, an output sends
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Input,
    Output,
}

/// One of the client's own ports: its index in registration order, and the client that made it (a port of another
/// client is refused instead of indexing this one's list)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Port {
    owner: u64,
    index: usize,
}

impl Port {
    pub fn index(self) -> usize {
        self.index
    }
}

/// Gives each `Client` of the process its own id
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// The two latencies JACK keeps for a port: from the capture (the converter to here) or to the playback
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LatencyMode {
    Capture,
    Playback,
}

/// A latency range, frames
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LatencyRange {
    pub min: u32,
    pub max: u32,
}

/// Which ports to list: by kind, by direction (as seen from their owner) and physical (a card's) or not
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PortFilter {
    pub kind: Option<PortKind>,
    pub direction: Option<Direction>,
    pub physical: bool,
}

/// The cycle's times (JACK's clock: microseconds, monotonic)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CycleTimes {
    /// The frame time of the cycle's first frame
    pub frames: u32,
    /// When the cycle's first frame was (estimated), us
    pub usecs: u64,
    /// When the next cycle's first frame will be, us
    pub next_usecs: u64,
    pub period_usecs: f32,
}

/// What runs every cycle on JACK's real-time thread. It must not allocate, lock or block
pub trait Process: Send + 'static {
    fn process(&mut self, cycle: &mut Cycle<'_>);
}

/// What JACK says on its other threads (never the real-time one). Called while the process handler may run
pub trait Notify: Send + Sync + 'static {
    /// The server was late (an xrun), by `delayed_usecs`
    fn xrun(&self, delayed_usecs: f32) {
        let _ = delayed_usecs;
    }
    /// The server went away or threw the client out: no more cycles come. The client may only be closed
    fn shutdown(&self, reason: &str) {
        let _ = reason;
    }
    /// The period changed
    fn buffer_size(&self, frames: u32) {
        let _ = frames;
    }
    /// A port appeared or went away (any client's)
    fn port_registration(&self, name: &str, registered: bool) {
        let _ = (name, registered);
    }
    /// Two ports were connected or disconnected (any clients')
    fn port_connection(&self, source: &str, destination: &str, connected: bool) {
        let _ = (source, destination, connected);
    }
}

#[derive(Clone, Copy, Debug)]
struct PortEntry {
    raw: NonNull<ffi::jack_port_t>,
    kind: PortKind,
    direction: Direction,
}

/// An open JACK client, not active: register its ports here
#[derive(Debug)]
pub struct Client {
    raw: NonNull<ffi::jack_client_t>,
    name: String,
    ports: Vec<PortEntry>,
    owner: u64,
}

// SAFETY: a jack_client_t may be used from any thread for the non-real-time API (JACK serialises its server requests);
// Client takes &mut self for what changes its own ports and is not Sync, so one thread uses it at a time
unsafe impl Send for Client {}

fn cstring(s: &str, what: &'static str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::new("jack.bad_name").arg("what", what).arg("name", s.to_owned()))
}

/// A C string from JACK, copied (empty for null)
///
/// # Safety
/// `p` is null or a valid NUL-terminated string that lives for this call
unsafe fn copy_str(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the caller's promise
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

static HOOKS: std::sync::Once = std::sync::Once::new();

/// JACK's own messages go to the program's log, not to stderr
unsafe extern "C" fn silent(_msg: *const c_char) {}

impl Client {
    /// Connects to the server `server` (JACK's default when `None`) as a client named `name` (JACK may change it if
    /// the name is taken). Never starts a server: none running is `jack.server_not_running`. Blocks: not on the main
    /// thread
    pub fn open(name: &str, server: Option<&str>) -> Result<Client> {
        let cname = cstring(name, "client")?;
        let mut status: ffi::jack_status_t = 0;
        // The hooks are process-global: set once, not at every open
        HOOKS.call_once(|| {
            // SAFETY: setting the global message hooks to a function that does nothing; JACK calls them from any thread
            unsafe {
                ffi::jack_set_error_function(Some(silent));
                ffi::jack_set_info_function(Some(silent));
            }
        });
        let raw = match server {
            Some(server) => {
                let cserver = cstring(server, "server")?;
                // SAFETY: the strings are NUL-terminated and live for the call; with JackServerName the variadic
                // argument is the server name, a const char*
                unsafe {
                    ffi::jack_client_open(
                        cname.as_ptr(),
                        ffi::JACK_NO_START_SERVER | ffi::JACK_SERVER_NAME,
                        &mut status,
                        cserver.as_ptr(),
                    )
                }
            }
            // SAFETY: the name is NUL-terminated; no variadic argument is read without JackServerName
            None => unsafe { ffi::jack_client_open(cname.as_ptr(), ffi::JACK_NO_START_SERVER, &mut status) },
        };
        let Some(raw) = NonNull::new(raw) else {
            let key = if status & ffi::JACK_VERSION_ERROR != 0 {
                "jack.version_mismatch"
            } else if status & (ffi::JACK_SERVER_FAILED | ffi::JACK_SERVER_ERROR) != 0
                || status & ffi::JACK_FAILURE != 0
            {
                "jack.server_not_running"
            } else {
                "jack.open_failed"
            };
            return Err(Error::new(key)
                .arg("status", i64::from(status))
                .arg("server", server.unwrap_or("default").to_owned()));
        };
        // SAFETY: the client is valid; the name it returns lives as long as the client and is copied now
        let name = unsafe { copy_str(ffi::jack_get_client_name(raw.as_ptr())) };
        Ok(Client { raw, name, ports: Vec::new(), owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed) })
    }

    /// The client's name as the server knows it
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The server's rate
    pub fn sample_rate(&self) -> u32 {
        // SAFETY: the client is valid
        unsafe { ffi::jack_get_sample_rate(self.raw.as_ptr()) }
    }

    /// The server's period, frames
    pub fn buffer_size(&self) -> u32 {
        // SAFETY: the client is valid
        unsafe { ffi::jack_get_buffer_size(self.raw.as_ptr()) }
    }

    /// The DSP load the server measures, percent
    pub fn cpu_load(&self) -> f32 {
        // SAFETY: the client is valid
        unsafe { ffi::jack_cpu_load(self.raw.as_ptr()) }
    }

    /// Registers a port `short_name` (shown as `<client>:<short_name>`). Before activating
    pub fn register(&mut self, short_name: &str, kind: PortKind, direction: Direction) -> Result<Port> {
        let cname = cstring(short_name, "port")?;
        let port_type = match kind {
            PortKind::Audio => ffi::JACK_DEFAULT_AUDIO_TYPE,
            PortKind::Midi => ffi::JACK_DEFAULT_MIDI_TYPE,
        };
        let flags = match direction {
            Direction::Input => ffi::JACK_PORT_IS_INPUT,
            Direction::Output => ffi::JACK_PORT_IS_OUTPUT,
        };
        // SAFETY: the client is valid, the strings NUL-terminated (the type is a static C string)
        let raw = unsafe {
            ffi::jack_port_register(
                self.raw.as_ptr(),
                cname.as_ptr(),
                port_type.as_ptr().cast(),
                std::ffi::c_ulong::from(flags),
                0,
            )
        };
        let raw = NonNull::new(raw)
            .ok_or_else(|| Error::new("jack.port_register_failed").arg("port", short_name.to_owned()))?;
        self.ports.push(PortEntry { raw, kind, direction });
        Ok(Port { owner: self.owner, index: self.ports.len() - 1 })
    }

    /// The entry of one of this client's ports; `None` for a port another client made
    fn entry(&self, port: Port) -> Option<PortEntry> {
        if port.owner != self.owner {
            return None;
        }
        self.ports.get(port.index).copied()
    }

    /// The full name (`client:port`) of one of the client's ports (empty for a port of another client)
    pub fn port_name(&self, port: Port) -> String {
        let Some(entry) = self.entry(port) else { return String::new() };
        // SAFETY: the port is the client's and valid while it is open
        unsafe { copy_str(ffi::jack_port_name(entry.raw.as_ptr())) }
    }

    /// The ports on the server that match (all clients'), by full name
    pub fn ports(&self, filter: PortFilter) -> Vec<String> {
        let type_pattern: &[u8] = match filter.kind {
            None => b"\0",
            Some(PortKind::Audio) => ffi::JACK_DEFAULT_AUDIO_TYPE,
            Some(PortKind::Midi) => ffi::JACK_DEFAULT_MIDI_TYPE,
        };
        let mut flags = match filter.direction {
            None => 0,
            Some(Direction::Input) => ffi::JACK_PORT_IS_INPUT,
            Some(Direction::Output) => ffi::JACK_PORT_IS_OUTPUT,
        };
        if filter.physical {
            flags |= ffi::JACK_PORT_IS_PHYSICAL;
        }
        // SAFETY: the client is valid; a null name pattern matches all; the type pattern is a NUL-terminated
        // regular expression (the exact type name, which has no special characters)
        let list = unsafe {
            ffi::jack_get_ports(
                self.raw.as_ptr(),
                std::ptr::null(),
                type_pattern.as_ptr().cast(),
                std::ffi::c_ulong::from(flags),
            )
        };
        let mut names = Vec::new();
        if list.is_null() {
            return names;
        }
        let mut i = 0;
        loop {
            // SAFETY: the list is a null-terminated array of strings; i stays within it
            let p = unsafe { *list.add(i) };
            if p.is_null() {
                break;
            }
            // SAFETY: each entry is a NUL-terminated string living until the list is freed
            names.push(unsafe { copy_str(p) });
            i += 1;
        }
        // SAFETY: the list came from jack_get_ports and is freed once, with jack_free
        unsafe { ffi::jack_free(list.cast()) };
        names
    }

    /// Connects `source` (an output) to `destination` (an input), by full names. Connecting what is connected is fine
    pub fn connect(&self, source: &str, destination: &str) -> Result<()> {
        let (s, d) = (cstring(source, "port")?, cstring(destination, "port")?);
        // SAFETY: the client is valid, the names NUL-terminated
        let code = unsafe { ffi::jack_connect(self.raw.as_ptr(), s.as_ptr(), d.as_ptr()) };
        // (EEXIST: already connected)
        if code == 0 || code == libc_eexist() {
            Ok(())
        } else {
            Err(Error::new("jack.connect_failed")
                .arg("source", source.to_owned())
                .arg("destination", destination.to_owned())
                .arg("code", i64::from(code)))
        }
    }

    pub fn disconnect(&self, source: &str, destination: &str) -> Result<()> {
        let (s, d) = (cstring(source, "port")?, cstring(destination, "port")?);
        // SAFETY: the client is valid, the names NUL-terminated
        let code = unsafe { ffi::jack_disconnect(self.raw.as_ptr(), s.as_ptr(), d.as_ptr()) };
        if code == 0 {
            Ok(())
        } else {
            Err(Error::new("jack.disconnect_failed")
                .arg("source", source.to_owned())
                .arg("destination", destination.to_owned())
                .arg("code", i64::from(code)))
        }
    }

    /// The total latency JACK computed for one of the client's ports: for an input, from the capture to here; for an
    /// output, from here to the playback (zero for a port of another client)
    pub fn latency(&self, port: Port, mode: LatencyMode) -> LatencyRange {
        let Some(entry) = self.entry(port) else { return LatencyRange::default() };
        latency_of(entry.raw.as_ptr(), mode)
    }

    /// The latency of any port, by full name (none if there is no such port)
    pub fn port_latency(&self, name: &str, mode: LatencyMode) -> Option<LatencyRange> {
        let cname = CString::new(name).ok()?;
        // SAFETY: the client is valid, the name NUL-terminated
        let raw = unsafe { ffi::jack_port_by_name(self.raw.as_ptr(), cname.as_ptr()) };
        (!raw.is_null()).then(|| latency_of(raw, mode))
    }

    /// Activates the client: from now `process` runs every cycle on JACK's thread and `notify` hears the server
    pub fn activate(
        self,
        process: Box<dyn Process>,
        notify: Arc<dyn Notify>,
    ) -> std::result::Result<ActiveClient, (Error, Client)> {
        let state = Box::new(State {
            process: UnsafeCell::new(process),
            notify,
            ports: self.ports.clone(),
            owner: self.owner,
            client: self.raw,
            panicked: AtomicBool::new(false),
        });
        let arg = (&*state as *const State).cast_mut().cast::<c_void>();
        let raw = self.raw.as_ptr();
        // SAFETY: the client is valid and not active; `arg` points to the State, which lives (boxed, never moved) until
        // the client is deactivated and closed, after which JACK calls none of these
        let codes = unsafe {
            [
                ffi::jack_set_process_callback(raw, Some(process_trampoline), arg),
                ffi::jack_set_xrun_callback(raw, Some(xrun_trampoline), arg),
                ffi::jack_set_buffer_size_callback(raw, Some(buffer_size_trampoline), arg),
                ffi::jack_set_port_registration_callback(raw, Some(registration_trampoline), arg),
                ffi::jack_set_port_connect_callback(raw, Some(connect_trampoline), arg),
            ]
        };
        // SAFETY: as above (the shutdown hook returns nothing)
        unsafe { ffi::jack_on_info_shutdown(raw, Some(shutdown_trampoline), arg) };
        if codes.iter().any(|&c| c != 0) {
            return Err((Error::new("jack.callback_failed"), release_callbacks(state, raw, self)));
        }
        // SAFETY: the client is valid with its callbacks set
        let code = unsafe { ffi::jack_activate(raw) };
        if code != 0 {
            let error = Error::new("jack.activate_failed").arg("code", i64::from(code));
            return Err((error, release_callbacks(state, raw, self)));
        }
        Ok(ActiveClient { client: Some(self), state: Some(state), active: true })
    }
}

/// The activation failed: JACK still holds `state`'s address in the callbacks already registered, and the client goes
/// back to the caller alive. The callbacks are cleared (allowed while inactive) before the state is freed; if any
/// cannot be, the state is leaked rather than left dangling
fn release_callbacks(state: Box<State>, raw: *mut ffi::jack_client_t, client: Client) -> Client {
    // SAFETY: the client is valid and not active; a null callback unregisters
    let cleared = unsafe {
        let null = std::ptr::null_mut();
        let codes = [
            ffi::jack_set_process_callback(raw, None, null),
            ffi::jack_set_xrun_callback(raw, None, null),
            ffi::jack_set_buffer_size_callback(raw, None, null),
            ffi::jack_set_port_registration_callback(raw, None, null),
            ffi::jack_set_port_connect_callback(raw, None, null),
        ];
        ffi::jack_on_info_shutdown(raw, None, null);
        codes.iter().all(|&c| c == 0)
    };
    if cleared {
        drop(state);
    } else {
        std::mem::forget(state);
    }
    client
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: the client is valid and closed once; closing unregisters its ports and disconnects them
        unsafe { ffi::jack_client_close(self.raw.as_ptr()) };
    }
}

fn libc_eexist() -> c_int {
    17
}

fn latency_of(port: *mut ffi::jack_port_t, mode: LatencyMode) -> LatencyRange {
    let mut range = ffi::jack_latency_range_t::default();
    let mode = match mode {
        LatencyMode::Capture => ffi::JACK_CAPTURE_LATENCY,
        LatencyMode::Playback => ffi::JACK_PLAYBACK_LATENCY,
    };
    // SAFETY: the port is valid (the caller's) and the range a valid out pointer
    unsafe { ffi::jack_port_get_latency_range(port, mode, &mut range) };
    LatencyRange { min: range.min, max: range.max }
}

/// What the trampolines reach: the handlers and the ports
struct State {
    process: UnsafeCell<Box<dyn Process>>,
    notify: Arc<dyn Notify>,
    ports: Vec<PortEntry>,
    owner: u64,
    client: NonNull<ffi::jack_client_t>,
    panicked: AtomicBool,
}

/// An active client. Dropping it deactivates and closes it (the process handler's last cycle has returned by then)
pub struct ActiveClient {
    client: Option<Client>,
    state: Option<Box<State>>,
    active: bool,
}

impl std::fmt::Debug for ActiveClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveClient").field("client", &self.client).field("active", &self.active).finish()
    }
}

// SAFETY: as Client; the State is reached by JACK's threads only through the trampolines, and by this type only after
// deactivation
unsafe impl Send for ActiveClient {}

impl ActiveClient {
    pub fn client(&self) -> &Client {
        self.client.as_ref().expect("the client is there until the ActiveClient goes")
    }

    /// True if the process handler panicked (the client has played silence since)
    pub fn panicked(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.panicked.load(Ordering::Acquire))
    }

    /// Deactivates the client: returns once the last cycle has returned (no handler runs after). Blocks: not on the
    /// main thread. Then it can be closed by dropping it
    pub fn deactivate(&mut self) {
        if self.active {
            if let Some(client) = &self.client {
                // SAFETY: the client is valid and active; jack_deactivate waits for the process thread to leave
                unsafe { ffi::jack_deactivate(client.raw.as_ptr()) };
            }
            self.active = false;
        }
    }

    /// Deactivates and closes the client, and gives back the process handler
    pub fn close(mut self) -> Box<dyn Process> {
        self.deactivate();
        // Closing first: after jack_client_close no callback can come (the notification threads included)
        drop(self.client.take());
        let state = self.state.take().expect("the state is there until closed");
        state.process.into_inner()
    }
}

impl Drop for ActiveClient {
    fn drop(&mut self) {
        self.deactivate();
        drop(self.client.take());
        drop(self.state.take());
    }
}

/// One cycle's access to the client's ports. Audio is copied in and out (a buffer is never borrowed: an input may
/// share memory with an output); MIDI is read by a closure and written by events. The outputs start the cycle silent
pub struct Cycle<'a> {
    frames: u32,
    ports: &'a [PortEntry],
    owner: u64,
    client: NonNull<ffi::jack_client_t>,
}

impl std::fmt::Debug for Cycle<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cycle").field("frames", &self.frames).finish()
    }
}

impl Cycle<'_> {
    pub fn frames(&self) -> u32 {
        self.frames
    }

    fn buffer(&self, port: Port, kind: PortKind, direction: Direction) -> Option<*mut c_void> {
        if port.owner != self.owner {
            return None;
        }
        let entry = self.ports.get(port.index)?;
        if entry.kind != kind || entry.direction != direction {
            return None;
        }
        // SAFETY: the port is the client's, called within its process callback with the cycle's frame count
        let p = unsafe { ffi::jack_port_get_buffer(entry.raw.as_ptr(), self.frames) };
        (!p.is_null()).then_some(p)
    }

    /// Copies an audio input into `dst` (as many frames as both have)
    pub fn read_audio(&self, port: Port, dst: &mut [f32]) {
        let Some(p) = self.buffer(port, PortKind::Audio, Direction::Input) else { return };
        let n = dst.len().min(self.frames as usize);
        // SAFETY: an audio port's buffer holds `frames` floats for this cycle; `dst` is ours, so no overlap
        unsafe { std::ptr::copy_nonoverlapping(p.cast::<f32>(), dst.as_mut_ptr(), n) };
    }

    /// Copies `src` into an audio output (as many frames as both have)
    pub fn write_audio(&mut self, port: Port, src: &[f32]) {
        let Some(p) = self.buffer(port, PortKind::Audio, Direction::Output) else { return };
        let n = src.len().min(self.frames as usize);
        // SAFETY: as read_audio; `src` is ours
        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), p.cast::<f32>(), n) };
    }

    /// Every event of a MIDI input, in order: its frame in the cycle and its bytes
    pub fn read_midi(&self, port: Port, mut sink: impl FnMut(u32, &[u8])) {
        let Some(p) = self.buffer(port, PortKind::Midi, Direction::Input) else { return };
        // SAFETY: a MIDI port's buffer, within the cycle
        let count = unsafe { ffi::jack_midi_get_event_count(p) };
        for i in 0..count {
            let mut event = ffi::jack_midi_event_t { time: 0, size: 0, buffer: std::ptr::null_mut() };
            // SAFETY: the index is below the count, the event a valid out pointer
            if unsafe { ffi::jack_midi_event_get(&mut event, p, i) } != 0 || event.buffer.is_null() {
                continue;
            }
            // SAFETY: the event's bytes live in the port buffer for the cycle
            let bytes = unsafe { std::slice::from_raw_parts(event.buffer, event.size) };
            sink(event.time, bytes);
        }
    }

    /// Writes an event to a MIDI output at frame `time` of the cycle (times in order). False if it did not fit (the
    /// buffer is full, the time is past the cycle or before the last event's)
    pub fn write_midi(&mut self, port: Port, time: u32, bytes: &[u8]) -> bool {
        let Some(p) = self.buffer(port, PortKind::Midi, Direction::Output) else { return false };
        if time >= self.frames {
            return false;
        }
        // SAFETY: a MIDI output's buffer, cleared at the cycle's start; the bytes are copied in
        unsafe { ffi::jack_midi_event_write(p, time, bytes.as_ptr(), bytes.len()) == 0 }
    }

    /// The cycle's times, to place what happened in it on the monotonic clock
    pub fn times(&self) -> Option<CycleTimes> {
        let mut t = CycleTimes::default();
        let mut frames = 0;
        // SAFETY: the client is valid; called within its process callback, as JACK asks
        let code = unsafe {
            ffi::jack_get_cycle_times(
                self.client.as_ptr(),
                &mut frames,
                &mut t.usecs,
                &mut t.next_usecs,
                &mut t.period_usecs,
            )
        };
        t.frames = frames;
        (code == 0).then_some(t)
    }

    /// Silences every output (audio zeroed, MIDI cleared)
    fn clear_outputs(&mut self) {
        for (i, entry) in self.ports.iter().enumerate() {
            if entry.direction != Direction::Output {
                continue;
            }
            let Some(p) = self.buffer(Port { owner: self.owner, index: i }, entry.kind, Direction::Output) else { continue };
            match entry.kind {
                // SAFETY: an audio output's buffer of `frames` floats
                PortKind::Audio => unsafe { std::ptr::write_bytes(p.cast::<f32>(), 0, self.frames as usize) },
                // SAFETY: a MIDI output's buffer, cleared once per cycle as JACK asks
                PortKind::Midi => unsafe { ffi::jack_midi_clear_buffer(p) },
            }
        }
    }
}

/// JACK's clock now, microseconds (the one the cycle times are on)
pub fn jack_time() -> u64 {
    // SAFETY: a pure query
    unsafe { ffi::jack_get_time() }
}

/// Runs a callback body, never unwinding into JACK (a panic across the C frames is not survivable): `default` is what
/// the callback returns when it panics. Every trampoline below runs its whole body through this
fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

unsafe extern "C" fn process_trampoline(frames: ffi::jack_nframes_t, arg: *mut c_void) -> c_int {
    // SAFETY: arg is the State, alive while the client is active (Client::activate)
    let state = unsafe { &*(arg as *const State) };
    guard(0, || {
        let mut cycle = Cycle { frames, ports: &state.ports, owner: state.owner, client: state.client };
        cycle.clear_outputs();
        if state.panicked.load(Ordering::Relaxed) {
            return 0;
        }
        // SAFETY: JACK runs the process callback on one thread at a time, and the State's handler is reached only
        // here until the client is deactivated: the exclusive reference is unique
        let process = unsafe { &mut *state.process.get() };
        if catch_unwind(AssertUnwindSafe(|| process.process(&mut cycle))).is_err() {
            // (a bug: silence from now on, never unwinding into C)
            state.panicked.store(true, Ordering::Release);
            cycle.clear_outputs();
        }
        0
    })
}

unsafe extern "C" fn xrun_trampoline(arg: *mut c_void) -> c_int {
    // SAFETY: as process_trampoline; only the shared parts are touched
    let state = unsafe { &*(arg as *const State) };
    // SAFETY: the client is valid while callbacks come
    let delayed = unsafe { ffi::jack_get_xrun_delayed_usecs(state.client.as_ptr()) };
    guard(0, || {
        state.notify.xrun(delayed);
        0
    })
}

unsafe extern "C" fn shutdown_trampoline(_code: ffi::jack_status_t, reason: *const c_char, arg: *mut c_void) {
    // SAFETY: as xrun_trampoline
    let state = unsafe { &*(arg as *const State) };
    guard((), || {
        // SAFETY: the reason is a NUL-terminated string for the call (or null)
        let reason = unsafe { copy_str(reason) };
        state.notify.shutdown(&reason);
    })
}

unsafe extern "C" fn buffer_size_trampoline(frames: ffi::jack_nframes_t, arg: *mut c_void) -> c_int {
    // SAFETY: as xrun_trampoline
    let state = unsafe { &*(arg as *const State) };
    guard(0, || {
        state.notify.buffer_size(frames);
        0
    })
}

unsafe extern "C" fn registration_trampoline(id: u32, register: c_int, arg: *mut c_void) {
    // SAFETY: as xrun_trampoline
    let state = unsafe { &*(arg as *const State) };
    guard((), || {
        // SAFETY: the client is valid; a port being unregistered is still known by its id during this call
        let port = unsafe { ffi::jack_port_by_id(state.client.as_ptr(), id) };
        // SAFETY: a valid port or null
        let name = if port.is_null() { String::new() } else { unsafe { copy_str(ffi::jack_port_name(port)) } };
        state.notify.port_registration(&name, register != 0);
    })
}

unsafe extern "C" fn connect_trampoline(a: u32, b: u32, connect: c_int, arg: *mut c_void) {
    // SAFETY: as xrun_trampoline
    let state = unsafe { &*(arg as *const State) };
    guard((), || {
        let name = |id: u32| {
            // SAFETY: the client is valid; the ports are known by id during this call
            let port = unsafe { ffi::jack_port_by_id(state.client.as_ptr(), id) };
            // SAFETY: a valid port or null
            if port.is_null() { String::new() } else { unsafe { copy_str(ffi::jack_port_name(port)) } }
        };
        let (source, destination) = (name(a), name(b));
        state.notify.port_connection(&source, &destination, connect != 0);
    })
}
