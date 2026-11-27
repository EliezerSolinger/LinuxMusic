//! One running plugin: instantiation with the host features, ports, run, worker, state

use crate::ffi;
use crate::state::{self, RestoreContext, SaveContext};
use crate::urid::{Urids, uri};
use crate::worker::{self, RestoreWorker, Schedule, Scratch, Target, WorkerMode, WorkerThread};
use crate::world::{Direction, PluginInfo, PortKind, Shared, World};
use lm_base::{Error, Result};
use rtrb::{Consumer, RingBuffer};
use std::collections::BTreeMap;
use std::ffi::{CStr, c_void};
use std::fmt;
use std::ptr::NonNull;
use std::sync::Arc;

/// How an instance is made: the rate and the block sizes it will be run with, and what the host promises
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceConfig {
    pub sample_rate: f64,
    /// The most frames one [`run`](Instance::run) gets (`bufsz:maxBlockLength`)
    pub max_block: u32,
    /// The fewest frames one `run` gets (`bufsz:minBlockLength`); `run` refuses fewer
    pub min_block: u32,
    /// Every `run` gets exactly `max_block` frames (`bufsz:fixedBlockLength` is offered; `run` enforces it)
    pub fixed_block: bool,
    /// `max_block` and every block are powers of two (`bufsz:powerOf2BlockLength` is offered; `run` enforces it)
    pub power_of_two_block: bool,
    /// The bytes of each atom port's buffer (at least the port's `rsz:minimumSize`; `bufsz:sequenceSize`)
    pub atom_capacity: u32,
    pub worker: WorkerMode,
    /// The bytes of each worker ring (requests and responses); a message must fit in it
    pub worker_capacity: u32,
    /// Offer `lv2:isLive` (the instance is driven by a live audio device)
    pub live: bool,
}

impl InstanceConfig {
    /// Threaded worker, blocks from 1 to `max_block` frames, 8 KiB atom buffers, 64 KiB worker rings
    pub fn new(sample_rate: f64, max_block: u32) -> Self {
        Self {
            sample_rate,
            max_block,
            min_block: 1,
            fixed_block: false,
            power_of_two_block: false,
            atom_capacity: 8192,
            worker: WorkerMode::Threaded,
            worker_capacity: 1 << 16,
            live: false,
        }
    }

    fn check(&self) -> Result<()> {
        let bad = |what: &'static str| Error::new("lilv.bad_config").arg("field", what);
        if !self.sample_rate.is_finite() || self.sample_rate <= 0.0 {
            return Err(bad("sample_rate"));
        }
        // (the CV ports allocate `max_block` floats each: bounded to a million frames)
        if self.max_block == 0 || self.max_block > 1 << 20 {
            return Err(bad("max_block"));
        }
        if self.min_block > self.max_block {
            return Err(bad("min_block"));
        }
        if self.power_of_two_block && !self.max_block.is_power_of_two() {
            return Err(bad("power_of_two_block"));
        }
        if !(64..=1 << 30).contains(&self.atom_capacity) {
            return Err(bad("atom_capacity"));
        }
        if !(64..=1 << 30).contains(&self.worker_capacity) {
            return Err(bad("worker_capacity"));
        }
        Ok(())
    }
}

/// Whether this host provides a feature a plugin requires
pub(crate) fn host_provides(feature: &str, config: &InstanceConfig) -> bool {
    let is = |u: &CStr| u.to_bytes() == feature.as_bytes();
    is(uri::URID_MAP)
        || is(uri::URID_UNMAP)
        || is(uri::OPTIONS)
        || is(uri::BOUNDED_BLOCK_LENGTH)
        || is(uri::WORKER_SCHEDULE)
        || is(uri::STATE_MAP_PATH)
        || is(uri::STATE_FREE_PATH)
        // Properties some plugins list as features: satisfied by how this host connects ports (never in place)
        || is(uri::HARD_RT_CAPABLE)
        || is(uri::IN_PLACE_BROKEN)
        || (is(uri::FIXED_BLOCK_LENGTH) && config.fixed_block)
        || (is(uri::POWER_OF_2_BLOCK_LENGTH) && config.power_of_two_block)
        || (is(uri::IS_LIVE) && config.live)
}

/// Whether this host passes an option a plugin requires
pub(crate) fn host_option(option: &str) -> bool {
    [
        uri::PARAM_SAMPLE_RATE,
        uri::MIN_BLOCK_LENGTH,
        uri::MAX_BLOCK_LENGTH,
        uri::NOMINAL_BLOCK_LENGTH,
        uri::SEQUENCE_SIZE,
    ]
    .iter()
    .any(|u| u.to_bytes() == option.as_bytes())
}

/// A heap array with a stable address, reached only through its raw pointer (the plugin holds the same pointer, so no
/// Rust reference may claim it exclusively while the plugin could use it)
struct RawBuf<T: Copy> {
    ptr: NonNull<T>,
    len: usize,
}

impl<T: Copy> RawBuf<T> {
    fn new(len: usize, value: T) -> Self {
        let boxed: Box<[T]> = vec![value; len].into_boxed_slice();
        let raw = Box::into_raw(boxed);
        // SAFETY: Box::into_raw never returns NULL (a dangling, non-null pointer for an empty slice)
        let ptr = unsafe { NonNull::new_unchecked(raw.cast::<T>()) };
        Self { ptr, len }
    }

    fn ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }

    fn get(&self, i: usize) -> Option<T> {
        // SAFETY: in bounds; plain Copy read; the plugin only writes during run, which needs &mut of the owner
        (i < self.len).then(|| unsafe { self.ptr().add(i).read() })
    }

    fn set(&mut self, i: usize, value: T) {
        if i < self.len {
            // SAFETY: in bounds; the owner holds &mut, so the plugin is not running
            unsafe { self.ptr().add(i).write(value) };
        }
    }
}

impl<T: Copy> Drop for RawBuf<T> {
    fn drop(&mut self) {
        // SAFETY: the pointer and length came from Box::into_raw of a boxed slice in `new`; freed once
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.ptr(), self.len)) });
    }
}

const ATOM_HEADER: usize = 8; // LV2_Atom: u32 size, u32 type
const SEQUENCE_HEADER: usize = 16; // LV2_Atom_Sequence: atom header + u32 unit + u32 pad
const EVENT_HEADER: usize = 16; // LV2_Atom_Event: i64 frames + LV2_Atom body header

/// One atom port's buffer
struct AtomPort {
    index: u32,
    input: bool,
    buf: RawBuf<u64>,
    capacity: usize,
    last_frame: i64,
}

impl AtomPort {
    fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: the buffer is `capacity` initialised bytes (u64 storage); the owner holds &mut, so the plugin is not
        // running and nothing else reads or writes it during the borrow
        unsafe { std::slice::from_raw_parts_mut(self.buf.ptr().cast::<u8>(), self.capacity) }
    }

    fn view(&self) -> &[u8] {
        // SAFETY: as in `bytes`, shared: the plugin only writes during run, which needs &mut of the owner
        unsafe { std::slice::from_raw_parts(self.buf.ptr().cast::<u8>(), self.capacity) }
    }

    /// An empty input sequence (before the first run and after each)
    fn clear_input(&mut self, sequence: u32) {
        let b = self.bytes();
        b[0..4].copy_from_slice(&8_u32.to_le_bytes());
        b[4..8].copy_from_slice(&sequence.to_le_bytes());
        b[8..16].fill(0); // unit 0 (frames), pad
        self.last_frame = 0;
    }

    /// An output buffer offered to the plugin: an atom:Chunk as big as the space (LV2 atom spec)
    fn offer_output(&mut self, chunk: u32) {
        let space = (self.capacity - ATOM_HEADER) as u32;
        let b = self.bytes();
        b[0..4].copy_from_slice(&space.to_le_bytes());
        b[4..8].copy_from_slice(&chunk.to_le_bytes());
    }

    fn push(&mut self, frame: u32, type_urid: u32, data: &[u8]) -> Result<()> {
        if i64::from(frame) < self.last_frame {
            return Err(Error::new("lilv.event_order"));
        }
        let capacity = self.capacity;
        let b = self.bytes();
        let body = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        let at = ATOM_HEADER + body;
        let size = u32::try_from(data.len()).map_err(|_| Error::new("lilv.atom_full"))?;
        let need = EVENT_HEADER + data.len().next_multiple_of(8);
        if at + need > capacity {
            return Err(Error::new("lilv.atom_full"));
        }
        b[at..at + 8].copy_from_slice(&i64::from(frame).to_le_bytes());
        b[at + 8..at + 12].copy_from_slice(&size.to_le_bytes());
        b[at + 12..at + 16].copy_from_slice(&type_urid.to_le_bytes());
        b[at + 16..at + 16 + data.len()].copy_from_slice(data);
        b[at + 16 + data.len()..at + need].fill(0);
        b[0..4].copy_from_slice(&((body + need) as u32).to_le_bytes());
        self.last_frame = i64::from(frame);
        Ok(())
    }
}

/// One event of an atom sequence
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtomEvent<'a> {
    /// The frame within the block
    pub frame: i64,
    /// The URID of the event's type ([`Urids::midi_event`] for MIDI)
    pub type_urid: u32,
    pub data: &'a [u8],
}

/// The events a plugin wrote to an atom output during the last run. Stops at the first malformed event (a plugin's
/// bad write never reads outside the buffer). Does not allocate
#[derive(Clone, Debug)]
pub struct AtomEvents<'a> {
    bytes: &'a [u8],
    at: usize,
    end: usize,
}

impl<'a> Iterator for AtomEvents<'a> {
    type Item = AtomEvent<'a>;

    fn next(&mut self) -> Option<AtomEvent<'a>> {
        let b = self.bytes;
        let at = self.at;
        if at + EVENT_HEADER > self.end {
            return None;
        }
        let frame = i64::from_le_bytes(b[at..at + 8].try_into().ok()?);
        let size = u32::from_le_bytes(b[at + 8..at + 12].try_into().ok()?) as usize;
        let type_urid = u32::from_le_bytes(b[at + 12..at + 16].try_into().ok()?);
        let data_end = at.checked_add(EVENT_HEADER)?.checked_add(size)?;
        if data_end > self.end {
            self.at = self.end;
            return None;
        }
        self.at = at + EVENT_HEADER + size.next_multiple_of(8);
        Some(AtomEvent { frame, type_urid, data: &b[at + EVENT_HEADER..data_end] })
    }
}

/// The host side of the features, at a stable address for the plugin's life (it may keep these pointers)
struct HostData {
    map: ffi::LV2_URID_Map,
    unmap: ffi::LV2_URID_Unmap,
    rate: f32,
    min_block: i32,
    max_block: i32,
    nominal_block: i32,
    sequence_size: i32,
    options: [ffi::LV2_Options_Option; 6],
    schedule: ffi::LV2_Worker_Schedule,
    map_path: ffi::LV2_State_Map_Path,
    free_path: ffi::LV2_State_Free_Path,
    features: [ffi::LV2_Feature; 10],
    /// The features handed to lilv, NULL-terminated
    list: [*const ffi::LV2_Feature; 11],
}

fn empty_option() -> ffi::LV2_Options_Option {
    ffi::LV2_Options_Option {
        context: ffi::LV2_OPTIONS_INSTANCE,
        subject: 0,
        key: 0,
        size: 0,
        type_: 0,
        value: std::ptr::null(),
    }
}

fn empty_feature() -> ffi::LV2_Feature {
    ffi::LV2_Feature { uri: std::ptr::null(), data: std::ptr::null_mut() }
}

/// Builds the features in place. The `HostData` is already at its final address
///
/// # Safety
/// `host` must point to a live `HostData` that is not moved or freed while the plugin exists
unsafe fn fill_features(host: *mut HostData, urids: &Urids, config: &InstanceConfig) {
    // SAFETY: the caller's contract; only raw-pointer field writes, no reference outlives a statement
    unsafe {
        let int = urids.atom_int;
        let values: [(u32, u32, *const c_void); 5] = [
            (urids.param_sample_rate, urids.atom_float, std::ptr::addr_of!((*host).rate).cast()),
            (urids.min_block_length, int, std::ptr::addr_of!((*host).min_block).cast()),
            (urids.max_block_length, int, std::ptr::addr_of!((*host).max_block).cast()),
            (urids.nominal_block_length, int, std::ptr::addr_of!((*host).nominal_block).cast()),
            (urids.sequence_size, int, std::ptr::addr_of!((*host).sequence_size).cast()),
        ];
        for (i, (key, type_, value)) in values.into_iter().enumerate() {
            (*host).options[i] =
                ffi::LV2_Options_Option { context: ffi::LV2_OPTIONS_INSTANCE, subject: 0, key, size: 4, type_, value };
        }
        // options[5] stays zeroed: the terminator

        let mut list: Vec<(&'static CStr, *mut c_void)> = vec![
            (uri::URID_MAP, std::ptr::addr_of_mut!((*host).map).cast()),
            (uri::URID_UNMAP, std::ptr::addr_of_mut!((*host).unmap).cast()),
            (uri::OPTIONS, std::ptr::addr_of_mut!((*host).options).cast()),
            (uri::BOUNDED_BLOCK_LENGTH, std::ptr::null_mut()),
            (uri::WORKER_SCHEDULE, std::ptr::addr_of_mut!((*host).schedule).cast()),
            (uri::STATE_MAP_PATH, std::ptr::addr_of_mut!((*host).map_path).cast()),
            (uri::STATE_FREE_PATH, std::ptr::addr_of_mut!((*host).free_path).cast()),
        ];
        if config.fixed_block {
            list.push((uri::FIXED_BLOCK_LENGTH, std::ptr::null_mut()));
        }
        if config.power_of_two_block {
            list.push((uri::POWER_OF_2_BLOCK_LENGTH, std::ptr::null_mut()));
        }
        if config.live {
            list.push((uri::IS_LIVE, std::ptr::null_mut()));
        }
        for (i, (feature_uri, data)) in list.into_iter().enumerate() {
            (*host).features[i] = ffi::LV2_Feature { uri: feature_uri.as_ptr(), data };
            (*host).list[i] = std::ptr::addr_of!((*host).features[i]);
        }
    }
}

/// A plugin instance: made by [`World::instantiate`], activated, run, saved and restored, and freed in `Drop` with
/// its worker thread **joined first**.
///
/// `Send`, not `Sync`: every call takes `&mut self`, which serialises the LV2 threading classes (no `run` during a
/// `restore`, no two `run`s). [`run`](Self::run), [`set_control`](Self::set_control), [`control`](Self::control),
/// [`push_event`](Self::push_event), [`push_midi`](Self::push_midi), [`output_events`](Self::output_events) and
/// [`latency`](Self::latency) do not allocate or lock on the host side: for the audio thread (of the plugin-host
/// helper). Everything else is not real-time safe.
pub struct Instance {
    info: PluginInfo,
    config: InstanceConfig,
    urids: Urids,
    raw: NonNull<ffi::LilvInstance>,
    handle: ffi::LV2_Handle,
    descriptor: *const ffi::LV2_Descriptor,
    host: NonNull<HostData>,
    schedule: NonNull<Schedule>,
    worker_iface: *const ffi::LV2_Worker_Interface,
    state_iface: *const ffi::LV2_State_Interface,
    worker: Option<WorkerThread>,
    responses: Option<Consumer<u8>>,
    response_scratch: Scratch,
    controls: RawBuf<f32>,
    cv: Vec<RawBuf<f32>>,
    atoms: Vec<AtomPort>,
    audio_in: Box<[u32]>,
    audio_out: Box<[u32]>,
    active: bool,
    /// Keeps the world (lilv's plugin library list) and the URID map alive; dropped after the plugin
    world: Arc<Shared>,
}

// SAFETY: an LV2 instance has no thread affinity (the spec's threading classes say which calls may run concurrently,
// not on which thread); every call here takes `&mut self`, so they are serialised. The worker thread only calls `work`
// and is joined before the instance is freed. The raw buffers are owned by the instance and used by one thread at a
// time. `Instance` is not `Sync` (raw pointers).
unsafe impl Send for Instance {}

impl World {
    /// Makes an instance of the plugin with this URI. Checks first that the host provides every required feature and
    /// option and knows every port that is not optional; then instantiates (this is where a broken plugin crashes:
    /// in the plugin-host helper process), connects the control, CV and atom ports to buffers the instance owns, and
    /// starts the worker thread if the plugin has a worker interface. Not activated yet.
    ///
    /// Errors: `lilv.bad_config`, `lilv.no_plugin`, `lilv.missing_feature` (with `missing`, the URIs separated by
    /// spaces), `lilv.unsupported_port`, `lilv.instantiate_failed`, `lilv.worker_failed`.
    pub fn instantiate(&self, plugin_uri: &str, config: &InstanceConfig) -> Result<Instance> {
        config.check()?;
        let shared = Arc::clone(&self.shared);
        let urids = shared.urids;
        let core = shared.lock();
        let plugin = core.find(plugin_uri)?;
        let info = core.info(plugin);

        let missing = info.missing(config);
        if !missing.is_empty() {
            return Err(Error::new("lilv.missing_feature").arg("uri", plugin_uri).arg("missing", missing.join(" ")));
        }
        for port in &info.ports {
            if (port.kind == PortKind::Unknown || port.direction == Direction::Unknown) && !port.connection_optional {
                return Err(Error::new("lilv.unsupported_port")
                    .arg("uri", plugin_uri)
                    .arg("port", port.symbol.clone()));
            }
        }

        // The worker's context and rings
        let capacity = config.worker_capacity as usize;
        let worker_shared = Arc::new(worker::Shared::default());
        let (requests, thread_rings, sync_responses, responses) = match config.worker {
            WorkerMode::Threaded => {
                let (req_tx, req_rx, resp_tx, resp_rx) = worker::rings(capacity);
                (Some(req_tx), Some((req_rx, resp_tx)), None, resp_rx)
            }
            WorkerMode::Synchronous => {
                let (resp_tx, resp_rx) = RingBuffer::new(capacity);
                (None, None, Some(resp_tx), resp_rx)
            }
        };
        let schedule = Box::new(Schedule {
            mode: config.worker,
            target: None,
            requests,
            thread: None,
            sync_responses,
            shared: Arc::clone(&worker_shared),
        });
        let schedule = Box::into_non_null(schedule);

        let (map, unmap) = shared.urid_map.features();
        let block = config.max_block as i32;
        let host = Box::new(HostData {
            map,
            unmap,
            rate: config.sample_rate as f32,
            min_block: if config.fixed_block { block } else { config.min_block as i32 },
            max_block: block,
            nominal_block: block,
            sequence_size: config.atom_capacity as i32,
            options: std::array::from_fn(|_| empty_option()),
            schedule: ffi::LV2_Worker_Schedule {
                handle: schedule.as_ptr().cast(),
                schedule_work: worker::schedule_work,
            },
            map_path: ffi::LV2_State_Map_Path {
                handle: std::ptr::null_mut(),
                abstract_path: state::map_path,
                absolute_path: state::map_path,
            },
            free_path: ffi::LV2_State_Free_Path { handle: std::ptr::null_mut(), free_path: state::free_path },
            features: std::array::from_fn(|_| empty_feature()),
            list: [std::ptr::null(); 11],
        });
        let host = Box::into_non_null(host);
        // SAFETY: `host` is at its final address and lives until after lilv_instance_free (Instance's Drop, or the
        // failure path just below)
        unsafe { fill_features(host.as_ptr(), &urids, config) };

        // SAFETY: plugin from this world's collection, lock held; the features are valid and outlive the instance
        let raw = unsafe {
            ffi::lilv_plugin_instantiate(plugin, config.sample_rate, std::ptr::addr_of!((*host.as_ptr()).list).cast())
        };
        let Some(raw) = NonNull::new(raw) else {
            drop(core);
            // SAFETY: both came from Box::into_raw above, the plugin was never made (nothing holds them); freed once
            unsafe {
                drop(Box::from_raw(host.as_ptr()));
                drop(Box::from_raw(schedule.as_ptr()));
            }
            return Err(Error::new("lilv.instantiate_failed").arg("uri", plugin_uri));
        };
        drop(core);

        // SAFETY: a LilvInstance just returned by lilv: its descriptor and handle are valid until lilv_instance_free
        let (descriptor, handle) = unsafe { ((*raw.as_ptr()).lv2_descriptor, (*raw.as_ptr()).lv2_handle) };
        let extension = |u: &CStr| -> *const c_void {
            // SAFETY: the descriptor is valid; extension_data takes a NUL-terminated URI and returns static data
            unsafe { (*descriptor).extension_data.map_or(std::ptr::null(), |f| f(u.as_ptr())) }
        };
        let worker_iface = extension(uri::WORKER_INTERFACE).cast::<ffi::LV2_Worker_Interface>();
        let state_iface = extension(uri::STATE_INTERFACE).cast::<ffi::LV2_State_Interface>();

        let ports = info.ports.len();
        let mut instance = Instance {
            urids,
            raw,
            handle,
            descriptor,
            host,
            schedule,
            worker_iface,
            state_iface,
            worker: None,
            responses: (!worker_iface.is_null()).then_some(responses),
            response_scratch: Scratch::new(capacity),
            controls: RawBuf::new(ports, 0.0),
            cv: Vec::new(),
            atoms: Vec::new(),
            audio_in: info
                .ports
                .iter()
                .filter(|p| p.kind == PortKind::Audio && p.direction == Direction::Input)
                .map(|p| p.index)
                .collect(),
            audio_out: info
                .ports
                .iter()
                .filter(|p| p.kind == PortKind::Audio && p.direction == Direction::Output)
                .map(|p| p.index)
                .collect(),
            active: false,
            world: shared.clone(),
            config: *config,
            info,
        };
        drop(shared);
        instance.connect_owned_ports();
        instance.start_worker(thread_rings, worker_shared)?;
        Ok(instance)
    }
}

impl Instance {
    fn connect(&mut self, index: u32, data: *mut c_void) {
        // SAFETY: the descriptor and handle are valid until Drop; connect_port is in the audio threading class and we
        // hold &mut self; `data` is a buffer that stays valid while connected (owned buffers live until after
        // lilv_instance_free; audio buffers are reconnected before every run)
        unsafe {
            if let Some(connect) = (*self.descriptor).connect_port {
                connect(self.handle, index, data);
            }
        }
    }

    fn connect_owned_ports(&mut self) {
        let rate = self.config.sample_rate;
        let max_block = self.config.max_block as usize;
        let atom_capacity = self.config.atom_capacity as usize;
        let sequence = self.urids.atom_sequence;
        let ports = self.info.ports.clone();
        for port in &ports {
            match port.kind {
                PortKind::Control => {
                    let value = if port.direction == Direction::Input { port.initial_value(rate) } else { 0.0 };
                    self.controls.set(port.index as usize, value);
                    let data = self.controls.ptr().wrapping_add(port.index as usize);
                    self.connect(port.index, data.cast());
                }
                PortKind::Cv => {
                    let value = if port.direction == Direction::Input { port.initial_value(rate) } else { 0.0 };
                    let buf = RawBuf::new(max_block, value);
                    let data = buf.ptr();
                    self.cv.push(buf);
                    self.connect(port.index, data.cast());
                }
                PortKind::Atom => {
                    let wanted = atom_capacity.max(port.minimum_size.unwrap_or(0) as usize).max(SEQUENCE_HEADER);
                    let capacity = wanted.next_multiple_of(8);
                    let mut atom = AtomPort {
                        index: port.index,
                        input: port.direction == Direction::Input,
                        buf: RawBuf::new(capacity / 8, 0),
                        capacity,
                        last_frame: 0,
                    };
                    atom.clear_input(sequence);
                    let data = atom.buf.ptr();
                    self.atoms.push(atom);
                    self.connect(port.index, data.cast());
                }
                // Audio ports are connected to the host's buffers at every run; unknown optional ports stay unconnected
                PortKind::Audio => {}
                PortKind::Unknown => self.connect(port.index, std::ptr::null_mut()),
            }
        }
    }

    fn start_worker(
        &mut self,
        thread_rings: Option<(Consumer<u8>, rtrb::Producer<u8>)>,
        shared: Arc<worker::Shared>,
    ) -> Result<()> {
        if self.worker_iface.is_null() {
            // No worker interface: a stray schedule_work is refused (no target, no ring)
            // SAFETY: the Schedule is ours; the plugin is not running (we are still building the instance)
            unsafe { (*self.schedule.as_ptr()).requests = None };
            return Ok(());
        }
        let target = Target { handle: self.handle, iface: self.worker_iface };
        // SAFETY: as above
        unsafe { (*self.schedule.as_ptr()).target = Some(target) };
        if let Some((requests, responses)) = thread_rings {
            let thread = WorkerThread::start(target, requests, responses, self.config.worker_capacity as usize, shared)
                .map_err(|e| Error::new("lilv.worker_failed").arg("message", e.to_string()))?;
            // SAFETY: as above
            unsafe { (*self.schedule.as_ptr()).thread = thread.thread() };
            self.worker = Some(thread);
        }
        Ok(())
    }

    pub fn info(&self) -> &PluginInfo {
        &self.info
    }

    pub fn config(&self) -> &InstanceConfig {
        &self.config
    }

    /// The ids of the URIs the host uses (the MIDI event type for [`push_event`](Self::push_event))
    pub fn urids(&self) -> &Urids {
        &self.urids
    }

    /// The URID map the plugin was given (shared with every instance of the world)
    pub fn urid_map(&self) -> &crate::UridMap {
        &self.world.urid_map
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The audio input ports, in index order: the slices [`run`](Self::run) takes as `inputs`
    pub fn audio_inputs(&self) -> &[u32] {
        &self.audio_in
    }

    /// The audio output ports, in index order: the slices `run` takes as `outputs`
    pub fn audio_outputs(&self) -> &[u32] {
        &self.audio_out
    }

    /// The plugin has a worker interface (and, in threaded mode, a worker thread)
    pub fn has_worker(&self) -> bool {
        !self.worker_iface.is_null()
    }

    /// The plugin has a state interface ([`save_state`](Self::save_state) gives a chunk)
    pub fn has_state(&self) -> bool {
        !self.state_iface.is_null()
    }

    /// Activates the plugin (LV2 instantiation class: may allocate; not on the audio thread). Atom inputs start empty
    pub fn activate(&mut self) {
        if self.active {
            return;
        }
        let sequence = self.urids.atom_sequence;
        for atom in self.atoms.iter_mut().filter(|a| a.input) {
            atom.clear_input(sequence);
        }
        // SAFETY: valid descriptor and handle; &mut self serialises the call
        unsafe {
            if let Some(activate) = (*self.descriptor).activate {
                activate(self.handle);
            }
        }
        self.active = true;
    }

    /// Deactivates the plugin (instantiation class: not on the audio thread). Responses still queued from the worker
    /// are delivered first
    pub fn deactivate(&mut self) {
        if !self.active {
            return;
        }
        self.deliver_responses();
        // SAFETY: valid descriptor and handle; &mut self serialises the call
        unsafe {
            if let Some(deactivate) = (*self.descriptor).deactivate {
                deactivate(self.handle);
            }
        }
        self.active = false;
    }

    /// Sets a control input, clamped to the port's range. Does not allocate. Error: `lilv.not_control_input`
    /// (without arguments)
    pub fn set_control(&mut self, index: u32, value: f32) -> Result<()> {
        let Some(port) = self.info.ports.get(index as usize) else {
            return Err(Error::new("lilv.not_control_input"));
        };
        if port.kind != PortKind::Control || port.direction != Direction::Input || !value.is_finite() {
            return Err(Error::new("lilv.not_control_input"));
        }
        let scale = if port.sample_rate { self.config.sample_rate as f32 } else { 1.0 };
        let mut v = value;
        if let Some(min) = port.minimum {
            v = v.max(min * scale);
        }
        if let Some(max) = port.maximum {
            v = v.min(max * scale);
        }
        self.controls.set(index as usize, v);
        Ok(())
    }

    /// The value of a control port (an input as set, an output as the plugin last wrote it). Does not allocate
    pub fn control(&self, index: u32) -> Option<f32> {
        let port = self.info.ports.get(index as usize)?;
        if port.kind != PortKind::Control {
            return None;
        }
        self.controls.get(index as usize)
    }

    /// The latency the plugin reports, in frames (its latency port after the last run; 0 before any). Does not
    /// allocate
    pub fn latency(&self) -> Option<u32> {
        let value = self.control(self.info.latency_port?)?;
        Some(if value.is_finite() && value > 0.0 { value.round() as u32 } else { 0 })
    }

    fn atom(&mut self, index: u32, input: bool) -> Option<&mut AtomPort> {
        self.atoms.iter_mut().find(|a| a.index == index && a.input == input)
    }

    /// Adds an event to an atom input for the next run, at `frame` within the block (events in order). The inputs
    /// are emptied after every run. Does not allocate. Errors (without arguments): `lilv.not_atom_input`,
    /// `lilv.event_order`, `lilv.atom_full`
    pub fn push_event(&mut self, port: u32, frame: u32, type_urid: u32, data: &[u8]) -> Result<()> {
        let Some(atom) = self.atom(port, true) else {
            return Err(Error::new("lilv.not_atom_input"));
        };
        atom.push(frame, type_urid, data)
    }

    /// [`push_event`](Self::push_event) of a MIDI message
    pub fn push_midi(&mut self, port: u32, frame: u32, message: &[u8]) -> Result<()> {
        let midi = self.urids.midi_event;
        self.push_event(port, frame, midi, message)
    }

    /// The events the plugin wrote to an atom output in the last run. Does not allocate. Error:
    /// `lilv.not_atom_output` (without arguments)
    pub fn output_events(&self, port: u32) -> Result<AtomEvents<'_>> {
        let Some(atom) = self.atoms.iter().find(|a| a.index == port && !a.input) else {
            return Err(Error::new("lilv.not_atom_output"));
        };
        let bytes = atom.view();
        let size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let type_ = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let end = ATOM_HEADER.saturating_add(size);
        let valid = type_ == self.urids.atom_sequence && size >= 8 && end <= atom.capacity;
        Ok(AtomEvents { bytes, at: SEQUENCE_HEADER, end: if valid { end } else { SEQUENCE_HEADER } })
    }

    /// Runs one block of `frames` frames: connects the audio ports to `inputs` and `outputs` (one slice per audio
    /// input and output port, in index order, each at least `frames` long; never aliased, so `lv2:inPlaceBroken`
    /// plugins are safe), runs the plugin, delivers the worker's responses and calls `end_run`, and empties the atom
    /// inputs. **Does not allocate, lock or do I/O on the host side** (the plugin's own code is the plugin's
    /// business: `lv2:hardRTCapable` says it promises the same).
    ///
    /// Errors (without arguments, so that making them does not allocate): `lilv.not_active`, `lilv.bad_block` (more
    /// than `max_block`, fewer than `min_block`, not `max_block` with a fixed block, not a power of two when
    /// promised), `lilv.bad_buffers` (wrong number of slices, or one too short).
    pub fn run(&mut self, frames: usize, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) -> Result<()> {
        if !self.active {
            return Err(Error::new("lilv.not_active"));
        }
        let c = &self.config;
        let Ok(n) = u32::try_from(frames) else {
            return Err(Error::new("lilv.bad_block"));
        };
        if n > c.max_block
            || n < c.min_block
            || (c.fixed_block && n != c.max_block)
            || (c.power_of_two_block && !n.is_power_of_two())
        {
            return Err(Error::new("lilv.bad_block"));
        }
        if inputs.len() != self.audio_in.len()
            || outputs.len() != self.audio_out.len()
            || inputs.iter().any(|b| b.len() < frames)
            || outputs.iter().any(|b| b.len() < frames)
        {
            return Err(Error::new("lilv.bad_buffers"));
        }
        for (i, buffer) in inputs.iter().enumerate() {
            // LV2's connect_port takes a mutable pointer for every port; the plugin must not write an input
            self.connect(self.audio_in[i], buffer.as_ptr().cast_mut().cast());
        }
        for (i, buffer) in outputs.iter_mut().enumerate() {
            self.connect(self.audio_out[i], buffer.as_mut_ptr().cast());
        }
        let chunk = self.urids.atom_chunk;
        for atom in self.atoms.iter_mut().filter(|a| !a.input) {
            atom.offer_output(chunk);
        }
        // SAFETY: active instance, every port connected (audio just now, the rest to owned buffers), &mut self; the
        // slices outlive the call, and the plugin may not touch port buffers outside run (LV2 core)
        unsafe {
            if let Some(run) = (*self.descriptor).run {
                run(self.handle, n);
            }
        }
        self.deliver_responses();
        // SAFETY: the worker interface (if any) is valid for the plugin's life; end_run is in the audio class
        unsafe {
            if let Some(iface) = self.worker_iface.as_ref()
                && let Some(end_run) = iface.end_run
            {
                end_run(self.handle);
            }
        }
        let sequence = self.urids.atom_sequence;
        for atom in self.atoms.iter_mut().filter(|a| a.input) {
            atom.clear_input(sequence);
        }
        Ok(())
    }

    /// Delivers every queued worker response to `work_response`. Does not allocate
    fn deliver_responses(&mut self) {
        let Some(responses) = self.responses.as_mut() else {
            return;
        };
        // SAFETY: the worker interface is non-null whenever `responses` is set
        let Some(work_response) = (unsafe { &*self.worker_iface }).work_response else {
            while worker::pop_message(responses, self.response_scratch.bytes()).is_some() {}
            return;
        };
        while let Some(len) = worker::pop_message(responses, self.response_scratch.bytes()) {
            // SAFETY: valid handle; the response is in 8-aligned scratch, valid for the call; work_response is in the
            // audio class and &mut self serialises it with run
            unsafe { work_response(self.handle, len as u32, self.response_scratch.ptr()) };
        }
    }

    /// Saves the plugin's state as a chunk (`None` if the plugin has no state interface). Allocates: not on the
    /// audio thread. The chunk is the same bytes for the same state. A plugin whose `save` fails without storing
    /// anything gives an empty chunk (as lilv does: many plugins answer so when they hold no state).
    ///
    /// Errors: `lilv.state_not_pod` (the plugin stored a value the host cannot copy), `lilv.state_bad_key`,
    /// `lilv.state_save_failed` (with the plugin's `status`, when it failed after storing part of its state).
    pub fn save_state(&mut self) -> Result<Option<Vec<u8>>> {
        // SAFETY: the state interface, if any, lives as long as the plugin
        let Some(save) = (unsafe { self.state_iface.as_ref() }).and_then(|s| s.save) else {
            return Ok(None);
        };
        let mut ctx = SaveContext { map: &self.world.urid_map, properties: BTreeMap::new(), refused: None };
        let mut map_path = ffi::LV2_State_Map_Path {
            handle: std::ptr::null_mut(),
            abstract_path: state::map_path,
            absolute_path: state::map_path,
        };
        let mut free_path = ffi::LV2_State_Free_Path { handle: std::ptr::null_mut(), free_path: state::free_path };
        let features = [
            ffi::LV2_Feature { uri: uri::STATE_MAP_PATH.as_ptr(), data: std::ptr::addr_of_mut!(map_path).cast() },
            ffi::LV2_Feature { uri: uri::STATE_FREE_PATH.as_ptr(), data: std::ptr::addr_of_mut!(free_path).cast() },
        ];
        let list = [&features[0] as *const ffi::LV2_Feature, &features[1], std::ptr::null()];
        let flags = ffi::LV2_STATE_IS_POD | ffi::LV2_STATE_IS_PORTABLE;
        // SAFETY: valid handle; `ctx` and the features live on this stack frame for the whole call, and the plugin may
        // not keep them after save returns (LV2 state); &mut self keeps save from running with restore
        let status =
            unsafe { save(self.handle, state::store, std::ptr::addr_of_mut!(ctx).cast(), flags, list.as_ptr()) };
        if let Some(refused) = ctx.refused {
            return Err(refused);
        }
        // A failure with nothing stored is "no state to save": many plugins answer so when they hold nothing beyond
        // their ports (Carla's do), and lilv itself saves them as an empty state. A failure after storing something
        // is a real failure: the chunk would be partial
        if status != ffi::LV2_STATE_SUCCESS && !ctx.properties.is_empty() {
            return Err(Error::new("lilv.state_save_failed").arg("status", i64::from(status)));
        }
        Ok(Some(state::encode(&ctx.properties)))
    }

    /// Restores a state saved by [`save_state`](Self::save_state) of the same plugin URI. Waits for the worker to
    /// finish the work already scheduled and delivers its responses first (so that no older response lands after the
    /// restore); the plugin may schedule work from `restore`, which runs right there and whose responses are
    /// delivered before this returns. Allocates and blocks: not on the audio thread.
    ///
    /// Errors: `lilv.bad_chunk`, `lilv.chunk_version`, `lilv.no_state_interface` (a non-empty chunk for a plugin
    /// without state), `lilv.state_restore_failed` (with the plugin's `status`).
    pub fn restore_state(&mut self, chunk: &[u8]) -> Result<()> {
        let properties = state::decode(chunk)?;
        // SAFETY: the state interface, if any, lives as long as the plugin
        let Some(restore) = (unsafe { self.state_iface.as_ref() }).and_then(|s| s.restore) else {
            return if properties.is_empty() { Ok(()) } else { Err(Error::new("lilv.no_state_interface")) };
        };
        if let Some(worker) = &self.worker {
            worker.wait_idle();
        }
        self.deliver_responses();

        let ctx = RestoreContext::new(&self.world.urid_map, &properties);
        let mut restore_worker = (!self.worker_iface.is_null()).then(|| RestoreWorker {
            target: Target { handle: self.handle, iface: self.worker_iface },
            responses: Vec::new(),
        });
        let mut map_path = ffi::LV2_State_Map_Path {
            handle: std::ptr::null_mut(),
            abstract_path: state::map_path,
            absolute_path: state::map_path,
        };
        let mut free_path = ffi::LV2_State_Free_Path { handle: std::ptr::null_mut(), free_path: state::free_path };
        let mut schedule = ffi::LV2_Worker_Schedule {
            handle: restore_worker.as_mut().map_or(std::ptr::null_mut(), |w| std::ptr::from_mut(w).cast()),
            schedule_work: worker::schedule_restore_work,
        };
        let features = [
            ffi::LV2_Feature { uri: uri::STATE_MAP_PATH.as_ptr(), data: std::ptr::addr_of_mut!(map_path).cast() },
            ffi::LV2_Feature { uri: uri::STATE_FREE_PATH.as_ptr(), data: std::ptr::addr_of_mut!(free_path).cast() },
            ffi::LV2_Feature { uri: uri::WORKER_SCHEDULE.as_ptr(), data: std::ptr::addr_of_mut!(schedule).cast() },
        ];
        let with_worker = restore_worker.is_some();
        let list = [
            &features[0] as *const ffi::LV2_Feature,
            &features[1],
            if with_worker { &features[2] } else { std::ptr::null() },
            std::ptr::null(),
        ];
        // SAFETY: valid handle; the context, the restore worker and the features live on this stack frame for the
        // whole call and the plugin may not keep them (LV2 state); no run or work runs meanwhile (&mut self, and the
        // worker thread is idle: everything scheduled is done and nothing new can be scheduled without run)
        let status = unsafe {
            restore(self.handle, state::retrieve, std::ptr::from_ref(&ctx).cast_mut().cast(), 0, list.as_ptr())
        };
        // SAFETY: the worker interface (if any) lives as long as the plugin
        let iface = unsafe { self.worker_iface.as_ref() };
        if let (Some(w), Some(work_response)) = (restore_worker, iface.and_then(|i| i.work_response)) {
            for response in &w.responses {
                let mut aligned = Scratch::new(response.len());
                aligned.bytes()[..response.len()].copy_from_slice(response);
                // SAFETY: valid handle; the response bytes are valid for the call; &mut self serialises it
                unsafe { work_response(self.handle, response.len() as u32, aligned.ptr()) };
            }
        }
        if status != ffi::LV2_STATE_SUCCESS {
            return Err(Error::new("lilv.state_restore_failed").arg("status", i64::from(status)));
        }
        Ok(())
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // 1. The worker thread first: it works what is still queued and is joined, so no `work` can run on a freed
        //    plugin (the legacy program's use-after-free, lacuna 2)
        if let Some(mut worker) = self.worker.take() {
            worker.stop_and_join();
        }
        // 2. Its last responses, then deactivate (the plugin frees what it handed over through them)
        self.deactivate();
        // 3. The plugin itself (cleanup, and lilv closes the library: that touches the world, so under its lock)
        {
            let _core = self.world.lock();
            // SAFETY: the instance came from lilv_plugin_instantiate and is freed once; no thread uses it any more
            unsafe { ffi::lilv_instance_free(self.raw.as_ptr()) };
        }
        // 4. What the plugin was given: only now that it is gone
        // SAFETY: both came from Box::into_raw at instantiation and are freed once, after the plugin
        unsafe {
            drop(Box::from_raw(self.host.as_ptr()));
            drop(Box::from_raw(self.schedule.as_ptr()));
        }
        // 5. The owned port buffers and the world's Arc drop with the fields, after this
    }
}

impl fmt::Debug for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Instance")
            .field("uri", &self.info.uri)
            .field("config", &self.config)
            .field("active", &self.active)
            .field("worker", &self.has_worker())
            .field("state", &self.has_state())
            .finish_non_exhaustive()
    }
}
