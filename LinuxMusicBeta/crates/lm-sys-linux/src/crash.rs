//! Native crashes into the safe mode (U21): when a plugin's native code faults inside the audio callback, the node
//! it was processing is written to the project's suspect file before the process dies, so the project opens next
//! time with that plugin bypassed.
//!
//! Everything the signal handler needs is made beforehand, off the audio thread: the file is opened in advance (a
//! hidden temporary next to the suspect file), and each node's note is formatted in advance by the session
//! ([`CrashNotes`]). The handler only reads atomics, `write(2)`s one buffer, `fsync(2)`s and `rename(2)`s it into
//! place (all async-signal-safe), and then chains to the handler that was there before (the sanitizers', Rust's
//! stack-overflow one) or to the default action: the process still dies the way it would have.
//!
//! The handler blames a node only when the faulting thread is a render thread ([`RenderThreadMark`]): a crash of
//! the main thread while the callback happens to be inside a plugin accuses nobody.
//!
//! Platforms: every Unix (Linux first; macOS for development). Signals: `SIGSEGV`, `SIGBUS`, `SIGILL`, `SIGABRT`.

use std::ffi::{CString, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use lm_base::{Error, Result};

/// The signals a native crash arrives as
pub const SIGNALS: [c_int; 4] = [libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGABRT];

/// What each render thread is processing right now, by its index (0 the device's thread, 1.. the workers): the
/// table slot plus one, or 0 between nodes. A plain table of atomics, so what the signal handler does with it is
/// two loads, whatever the caller does (a closure here could do anything inside the handler)
pub type Probe = Arc<[AtomicU32]>;

/// The notes the handler may write, formatted in advance: one per table slot that holds a plugin
pub struct CrashNotes {
    probe: Probe,
    /// Sorted by slot
    notes: Box<[(u32, Box<[u8]>)]>,
}

impl CrashNotes {
    pub fn new(probe: Probe, mut notes: Vec<(u32, Vec<u8>)>) -> Self {
        notes.sort_by_key(|(slot, _)| *slot);
        notes.dedup_by_key(|(slot, _)| *slot);
        Self { probe, notes: notes.into_iter().map(|(s, b)| (s, b.into_boxed_slice())).collect() }
    }

    fn note(&self, slot: u32) -> Option<&[u8]> {
        self.notes.binary_search_by_key(&slot, |(s, _)| *s).ok().map(|i| &*self.notes[i].1)
    }
}

impl std::fmt::Debug for CrashNotes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrashNotes").field("notes", &self.notes.len()).finish()
    }
}

/// The armed file: its descriptor, the temporary it is and the name it takes
struct Armed {
    fd: c_int,
    tmp: CString,
    dest: CString,
    notes: AtomicPtr<CrashNotes>,
}

static ARMED: AtomicPtr<Armed> = AtomicPtr::new(std::ptr::null_mut());
static IN_HANDLER: AtomicBool = AtomicBool::new(false);
static PREVIOUS: OnceLock<[libc::sigaction; 4]> = OnceLock::new();
/// What was replaced is kept for a while instead of freed: a handler running on another thread at that very moment
/// may still be reading it
static RETIRED: Mutex<Vec<Retired>> = Mutex::new(Vec::new());
const KEEP_RETIRED: usize = 8;

enum Retired {
    Notes(#[allow(dead_code)] Box<CrashNotes>),
    Armed(#[allow(dead_code)] Box<Armed>),
}

fn retire(r: Retired) {
    if let Ok(mut list) = RETIRED.lock() {
        list.push(r);
        let excess = list.len().saturating_sub(KEEP_RETIRED);
        list.drain(..excess);
    } else {
        // A poisoned list: leaking is the safe choice
        std::mem::forget(r);
    }
}

/// The render threads: the `pthread_self` of each and its index (0 = free)
const THREADS: usize = 64;
static THREAD_IDS: [AtomicUsize; THREADS] = [const { AtomicUsize::new(0) }; THREADS];
static THREAD_INDEX: [AtomicUsize; THREADS] = [const { AtomicUsize::new(0) }; THREADS];

fn this_thread() -> usize {
    // SAFETY: pthread_self has no preconditions and is async-signal-safe
    let id = unsafe { libc::pthread_self() };
    id as usize
}

/// Marks the current thread as render thread `index` while alive (the session's thread hook: the device's thread
/// is 0, worker `i` is `i + 1`). Not `Send`: made and dropped on the thread it marks
#[derive(Debug)]
pub struct RenderThreadMark {
    entry: Option<usize>,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl RenderThreadMark {
    pub fn enter(index: usize) -> Self {
        let me = this_thread();
        let entry = (0..THREADS).find(|&i| {
            if THREAD_IDS[i].compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                THREAD_INDEX[i].store(index, Ordering::Release);
                true
            } else {
                false
            }
        });
        Self { entry, _not_send: std::marker::PhantomData }
    }
}

impl Drop for RenderThreadMark {
    fn drop(&mut self) {
        if let Some(i) = self.entry {
            THREAD_IDS[i].store(0, Ordering::Release);
        }
    }
}

fn render_index_of_this_thread() -> Option<usize> {
    let me = this_thread();
    (0..THREADS).find(|&i| THREAD_IDS[i].load(Ordering::Acquire) == me).map(|i| THREAD_INDEX[i].load(Ordering::Acquire))
}

/// Installs the handlers once for the process (later calls do nothing)
fn install_handlers() -> Result<()> {
    if PREVIOUS.get().is_some() {
        return Ok(());
    }
    static INSTALLING: Mutex<()> = Mutex::new(());
    let _one = INSTALLING.lock().map_err(|_| Error::new("linux.crash.install_failed"))?;
    if PREVIOUS.get().is_some() {
        return Ok(());
    }
    // SAFETY: sigaction is plain data; all zeroes is a valid empty value that is filled in below
    let mut previous: [libc::sigaction; 4] = unsafe { std::mem::zeroed() };
    for (i, &sig) in SIGNALS.iter().enumerate() {
        // SAFETY: as above
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = on_signal as extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) as usize;
        // On the alternate stack when the thread has one (a stack overflow); the signal stays blocked while handled
        action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
        // SAFETY: the mask is ours, initialised by sigemptyset before use
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        // SAFETY: valid signal number, a fully initialised action and a place for the previous one
        if unsafe { libc::sigaction(sig, &action, &mut previous[i]) } != 0 {
            // Undo the ones already swapped: leaving ours installed with no `PREVIOUS` would make a retry record our
            // own handler as the previous one, and `chain` would then call it forever
            for (done, old) in SIGNALS.iter().zip(previous.iter()).take(i) {
                // SAFETY: restores what sigaction returned for that signal
                unsafe { libc::sigaction(*done, old, std::ptr::null_mut()) };
            }
            return Err(Error::new("linux.crash.install_failed").arg("signal", i64::from(sig)));
        }
    }
    let _ = PREVIOUS.set(previous);
    Ok(())
}

/// The armed suspect file: dropping it disarms (the temporary is removed)
#[derive(Debug)]
pub struct CrashFile {
    armed: *mut Armed,
}

// SAFETY: the pointer is only compared and swapped through atomics; the Armed it names is immutable but for its
// atomic notes, and is freed only through `retire`
unsafe impl Send for CrashFile {}

/// Arms the handlers to write `name` in `dir` (the project's `backup/`) when a render thread crashes inside a plugin.
/// The last one armed is the one written (one project per process in the program; tests run several sessions).
/// Errors: `linux.crash.open_failed`, `linux.crash.install_failed`, `linux.invalid_argument`
pub fn arm(dir: &Path, name: &str) -> Result<CrashFile> {
    // A name is one file name in `dir`: a separator or a dot name would put the note elsewhere
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        return Err(Error::new("linux.invalid_argument").arg("argument", "name"));
    }
    install_handlers()?;
    let dest = dir.join(name);
    let tmp = dir.join(format!(".{name}.crash"));
    let c = |p: &Path| CString::new(p.as_os_str().as_bytes()).map_err(|_| Error::new("linux.invalid_argument"));
    let (dest, tmp) = (c(&dest)?, c(&tmp)?);
    // SAFETY: a valid C string; the descriptor is ours and closed on disarm
    let fd =
        unsafe { libc::open(tmp.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC, 0o644) };
    if fd < 0 {
        return Err(Error::new("linux.crash.open_failed")
            .arg("path", tmp.to_string_lossy().into_owned())
            .arg("error", std::io::Error::last_os_error().to_string()));
    }
    let armed = Box::into_raw(Box::new(Armed { fd, tmp, dest, notes: AtomicPtr::new(std::ptr::null_mut()) }));
    // Another file may have been armed (another session): it stays alive in its own CrashFile, which frees it only
    // if it is still the armed one
    ARMED.store(armed, Ordering::Release);
    Ok(CrashFile { armed })
}

impl CrashFile {
    /// Replaces the notes (after each install of a program)
    pub fn set_notes(&self, notes: CrashNotes) {
        // SAFETY: `armed` stays alive while this CrashFile does (freed only by its drop, through retire)
        let armed = unsafe { &*self.armed };
        let old = armed.notes.swap(Box::into_raw(Box::new(notes)), Ordering::AcqRel);
        if !old.is_null() {
            // SAFETY: the pointer came from Box::into_raw above and is owned by nobody else now
            retire(Retired::Notes(unsafe { Box::from_raw(old) }));
        }
    }
}

impl Drop for CrashFile {
    fn drop(&mut self) {
        let _ = ARMED.compare_exchange(self.armed, std::ptr::null_mut(), Ordering::AcqRel, Ordering::Acquire);
        // SAFETY: the pointer came from Box::into_raw in arm and only this CrashFile frees it
        let armed = unsafe { Box::from_raw(self.armed) };
        // SAFETY: our descriptor and our C string; the temporary is removed (it is empty unless we crashed)
        unsafe {
            libc::close(armed.fd);
            libc::unlink(armed.tmp.as_ptr());
        }
        let notes = armed.notes.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !notes.is_null() {
            // SAFETY: from Box::into_raw in set_notes, owned by nobody else now
            retire(Retired::Notes(unsafe { Box::from_raw(notes) }));
        }
        retire(Retired::Armed(armed));
    }
}

/// Writes the whole buffer (async-signal-safe: `write(2)` only)
fn write_all(fd: c_int, mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        // SAFETY: a valid buffer of that length; fd is open
        let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if n < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return false;
        }
        if n == 0 {
            // A write that takes nothing would spin here forever, inside a signal handler
            return false;
        }
        bytes = &bytes[n as usize..];
    }
    true
}

/// The handler's own work: the note of the node this thread was in, if any
fn note_the_suspect() {
    let armed = ARMED.load(Ordering::Acquire);
    if armed.is_null() {
        return;
    }
    // SAFETY: an armed Armed is freed only after being retired KEEP_RETIRED times over (see `retire`)
    let armed = unsafe { &*armed };
    let notes = armed.notes.load(Ordering::Acquire);
    if notes.is_null() {
        return;
    }
    // SAFETY: as above, for the notes
    let notes = unsafe { &*notes };
    let Some(index) = render_index_of_this_thread() else { return };
    let Some(slot) = notes.probe.get(index).and_then(|a| a.load(Ordering::Relaxed).checked_sub(1)) else { return };
    let Some(note) = notes.note(slot) else { return };
    if write_all(armed.fd, note) {
        // SAFETY: fsync and rename are async-signal-safe; the descriptor and the C strings are alive
        unsafe {
            libc::fsync(armed.fd);
            libc::rename(armed.tmp.as_ptr(), armed.dest.as_ptr());
        }
    }
}

/// The signal handler. It cannot panic, so it has no `catch_unwind` (which would allocate the payload, in a signal
/// handler): every index is bounded (`bytes[n..]` with `n <= len`, `PREVIOUS[i]` with `i < SIGNALS.len()`, slots
/// through `get` and `binary_search`), nothing allocates and no arithmetic can overflow
extern "C" fn on_signal(sig: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
    // One note per process: a second crash (another thread, or inside this handler) goes straight on
    if !IN_HANDLER.swap(true, Ordering::AcqRel) {
        note_the_suspect();
    }
    chain(sig, info, context);
}

/// Hands the signal to whoever had it before; for the default action, restores it and raises the signal again: it
/// is blocked while we handle it, so it is delivered as we return, and the process dies as it would have
fn chain(sig: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
    let previous = PREVIOUS.get().and_then(|p| SIGNALS.iter().position(|&s| s == sig).map(|i| p[i]));
    match previous {
        // (never ourselves: that would recurse without end)
        Some(p)
            if p.sa_sigaction != libc::SIG_DFL
                && p.sa_sigaction != libc::SIG_IGN
                && p.sa_sigaction != on_signal as extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) as usize =>
        {
            if p.sa_flags & libc::SA_SIGINFO != 0 {
                // SAFETY: the previous handler was installed with SA_SIGINFO, so it has this signature
                let f: extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) =
                    unsafe { std::mem::transmute(p.sa_sigaction) };
                f(sig, info, context);
            } else {
                // SAFETY: without SA_SIGINFO the previous handler takes the signal number alone
                let f: extern "C" fn(c_int) = unsafe { std::mem::transmute(p.sa_sigaction) };
                f(sig);
            }
            // A handler that only restores the default and returns (Rust's, for a fault that is not a stack
            // overflow) counts on the fault happening again; a signal that was sent does not, so it is raised again
            // SAFETY: sigaction is plain data, zeroes are a valid place for the current action; both calls are
            // async-signal-safe
            unsafe {
                let mut now: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(sig, std::ptr::null(), &mut now) == 0 && now.sa_sigaction == libc::SIG_DFL {
                    libc::raise(sig);
                }
            }
        }
        _ => {
            // SAFETY: sigaction is plain data, zeroes plus SIG_DFL is the default action; sigaction and raise are
            // async-signal-safe
            unsafe {
                let mut dfl: libc::sigaction = std::mem::zeroed();
                dfl.sa_sigaction = libc::SIG_DFL;
                libc::sigemptyset(&mut dfl.sa_mask);
                libc::sigaction(sig, &dfl, std::ptr::null_mut());
                libc::raise(sig);
            }
        }
    }
}

/// For the crash tests: what a plugin with a null-pointer bug does (reads the eighth byte past address zero, which
/// no process maps). Never called by the program
#[doc(hidden)]
pub fn dereference_null() -> u64 {
    let p = std::ptr::without_provenance::<u64>(8);
    // SAFETY: none: this faults on purpose (SIGSEGV, or SIGBUS on some systems)
    unsafe { std::ptr::read_volatile(p) }
}
