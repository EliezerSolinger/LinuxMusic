//! Safe binding to libsndfile (at least 1.1, for MP3), the system's library found by pkg-config.
//!
//! - [`Reader`]: WAV, RF64, AIFF, FLAC, OGG (Vorbis, Opus) and MP3, as interleaved `f32`, `i32` or `i16`, with seek.
//! - [`Writer`]: WAV and RF64 (float 32, int 16, int 24), AIFF, FLAC, OGG Vorbis and MP3 (export). The recorder
//!   writes RF64 float and calls [`Writer::sync`] every ~2 s: the header is rewritten with the frames written so
//!   far and the data reaches the disk, so a crash leaves a readable file that ends at the last sync (I-11, U5,
//!   C10). [`Writer::reserve`] pre-reserves the disk space ahead of the recording (H3).
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | `SNDFILE*`: one [`Reader`] or one [`Writer`], closed by `sf_close` in its `Drop` (or [`Writer::close`], which reports the error). The file descriptor is owned by a Rust [`File`] in the same value; libsndfile only borrows it (`sf_open_fd` with `close_desc = 0`) and the `File` is dropped after `sf_close`. |
//! | Which thread may call each function? | `Reader` and `Writer` are `Send` and not `Sync`, and every call that touches the handle takes `&mut self`: one thread at a time, and the value can move to the disk thread (U6) or a worker. Opening is serialised by a lock of this crate, because libsndfile reports an open failure through a global variable. |
//! | Allocates, locks, does I/O? | **Every function does I/O** (and opening allocates and takes the open lock): none may be called from a processor or the audio thread. The recorder calls the `Writer` from its disk thread only (U6). [`version`] and [`is_supported`] do no I/O but allocate nothing either and are cheap. |
//! | Creates threads? | No (libsndfile and its codecs, FLAC, Vorbis, Opus, mpg123, LAME, run on the calling thread). |
//! | Calls back into our code? | No: the virtual I/O API is not used. |
//! | Tests | `tests/roundtrip.rs` (bit-exact round trips of every lossless format and depth, lossy formats keep the signal, seek, info), `tests/crash.rs` (an RF64 left without `sf_close` after a `sync` is readable up to it; disk reservation), `tests/errors.rs` (error keys). ASan in CI. |

use lm_base::{Error, Result};
use std::ffi::{CStr, c_char, c_double, c_int, c_void};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Mutex;

mod ffi {
    use super::{c_char, c_int, c_void};

    pub type SfCount = i64;

    #[repr(C)]
    pub struct Sndfile {
        _private: [u8; 0],
    }

    /// `SF_INFO` of sndfile.h
    #[repr(C)]
    #[derive(Default)]
    pub struct SfInfo {
        pub frames: SfCount,
        pub samplerate: c_int,
        pub channels: c_int,
        pub format: c_int,
        pub sections: c_int,
        pub seekable: c_int,
    }

    pub const SFM_READ: c_int = 0x10;
    pub const SFM_WRITE: c_int = 0x20;
    pub const SEEK_SET: c_int = 0;

    pub const SF_FORMAT_WAV: c_int = 0x01_0000;
    pub const SF_FORMAT_AIFF: c_int = 0x02_0000;
    pub const SF_FORMAT_FLAC: c_int = 0x17_0000;
    pub const SF_FORMAT_OGG: c_int = 0x20_0000;
    pub const SF_FORMAT_RF64: c_int = 0x22_0000;
    pub const SF_FORMAT_MPEG: c_int = 0x23_0000;

    pub const SF_FORMAT_PCM_S8: c_int = 0x0001;
    pub const SF_FORMAT_PCM_16: c_int = 0x0002;
    pub const SF_FORMAT_PCM_24: c_int = 0x0003;
    pub const SF_FORMAT_PCM_32: c_int = 0x0004;
    pub const SF_FORMAT_PCM_U8: c_int = 0x0005;
    pub const SF_FORMAT_FLOAT: c_int = 0x0006;
    pub const SF_FORMAT_DOUBLE: c_int = 0x0007;
    pub const SF_FORMAT_VORBIS: c_int = 0x0060;
    pub const SF_FORMAT_OPUS: c_int = 0x0064;
    pub const SF_FORMAT_MPEG_LAYER_I: c_int = 0x0080;
    pub const SF_FORMAT_MPEG_LAYER_II: c_int = 0x0081;
    pub const SF_FORMAT_MPEG_LAYER_III: c_int = 0x0082;

    pub const SF_FORMAT_SUBMASK: c_int = 0x0000_FFFF;
    pub const SF_FORMAT_TYPEMASK: c_int = 0x0FFF_0000;

    pub const SFC_SET_ADD_PEAK_CHUNK: c_int = 0x1050;
    pub const SFC_UPDATE_HEADER_NOW: c_int = 0x1060;
    pub const SFC_SET_CLIPPING: c_int = 0x10C0;
    pub const SFC_SET_VBR_ENCODING_QUALITY: c_int = 0x1300;
    pub const SFC_SET_COMPRESSION_LEVEL: c_int = 0x1301;

    pub const SF_FALSE: c_int = 0;
    pub const SF_TRUE: c_int = 1;

    unsafe extern "C" {
        pub fn sf_open_fd(fd: c_int, mode: c_int, info: *mut SfInfo, close_desc: c_int) -> *mut Sndfile;
        pub fn sf_close(file: *mut Sndfile) -> c_int;
        pub fn sf_error(file: *mut Sndfile) -> c_int;
        pub safe fn sf_error_number(errnum: c_int) -> *const c_char;
        pub fn sf_command(file: *mut Sndfile, command: c_int, data: *mut c_void, datasize: c_int) -> c_int;
        pub fn sf_format_check(info: *const SfInfo) -> c_int;
        pub fn sf_seek(file: *mut Sndfile, frames: SfCount, whence: c_int) -> SfCount;
        pub safe fn sf_version_string() -> *const c_char;
        pub fn sf_readf_short(file: *mut Sndfile, ptr: *mut i16, frames: SfCount) -> SfCount;
        pub fn sf_readf_int(file: *mut Sndfile, ptr: *mut i32, frames: SfCount) -> SfCount;
        pub fn sf_readf_float(file: *mut Sndfile, ptr: *mut f32, frames: SfCount) -> SfCount;
        pub fn sf_writef_short(file: *mut Sndfile, ptr: *const i16, frames: SfCount) -> SfCount;
        pub fn sf_writef_int(file: *mut Sndfile, ptr: *const i32, frames: SfCount) -> SfCount;
        pub fn sf_writef_float(file: *mut Sndfile, ptr: *const f32, frames: SfCount) -> SfCount;
    }
}

/// The version of the linked library ("libsndfile-1.2.2")
pub fn version() -> &'static str {
    let text = ffi::sf_version_string();
    if text.is_null() {
        return "";
    }
    // SAFETY: sf_version_string returns a pointer to a static NUL-terminated string
    unsafe { CStr::from_ptr(text) }.to_str().unwrap_or("")
}

/// The message libsndfile has for an error code (static text, never translated: for the logs only)
fn error_message(code: c_int) -> String {
    let text = ffi::sf_error_number(code);
    if text.is_null() {
        return String::new();
    }
    // SAFETY: sf_error_number returns a pointer into a static table of NUL-terminated strings
    unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()
}

/// The container of a file
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    Wav,
    Rf64,
    Aiff,
    Flac,
    Ogg,
    Mpeg,
    /// Another container libsndfile reads (its `SF_FORMAT_*` major value)
    Other(i32),
}

impl Container {
    fn from_format(format: c_int) -> Self {
        match format & ffi::SF_FORMAT_TYPEMASK {
            ffi::SF_FORMAT_WAV => Self::Wav,
            ffi::SF_FORMAT_RF64 => Self::Rf64,
            ffi::SF_FORMAT_AIFF => Self::Aiff,
            ffi::SF_FORMAT_FLAC => Self::Flac,
            ffi::SF_FORMAT_OGG => Self::Ogg,
            ffi::SF_FORMAT_MPEG => Self::Mpeg,
            other => Self::Other(other),
        }
    }
}

/// How the samples are encoded in a file
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Encoding {
    PcmS8,
    PcmU8,
    Pcm16,
    Pcm24,
    Pcm32,
    Float32,
    Float64,
    Vorbis,
    Opus,
    MpegLayer1,
    MpegLayer2,
    Mp3,
    /// Another encoding libsndfile reads (its `SF_FORMAT_*` subtype value)
    Other(i32),
}

impl Encoding {
    fn from_format(format: c_int) -> Self {
        match format & ffi::SF_FORMAT_SUBMASK {
            ffi::SF_FORMAT_PCM_S8 => Self::PcmS8,
            ffi::SF_FORMAT_PCM_U8 => Self::PcmU8,
            ffi::SF_FORMAT_PCM_16 => Self::Pcm16,
            ffi::SF_FORMAT_PCM_24 => Self::Pcm24,
            ffi::SF_FORMAT_PCM_32 => Self::Pcm32,
            ffi::SF_FORMAT_FLOAT => Self::Float32,
            ffi::SF_FORMAT_DOUBLE => Self::Float64,
            ffi::SF_FORMAT_VORBIS => Self::Vorbis,
            ffi::SF_FORMAT_OPUS => Self::Opus,
            ffi::SF_FORMAT_MPEG_LAYER_I => Self::MpegLayer1,
            ffi::SF_FORMAT_MPEG_LAYER_II => Self::MpegLayer2,
            ffi::SF_FORMAT_MPEG_LAYER_III => Self::Mp3,
            other => Self::Other(other),
        }
    }

    /// Whether a sample read back is exactly the sample written
    pub fn is_lossless(self) -> bool {
        !matches!(self, Self::Vorbis | Self::Opus | Self::MpegLayer1 | Self::MpegLayer2 | Self::Mp3)
    }
}

/// What a file holds
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    /// Frames (one sample per channel). For a file left by a crash, the frames up to the last header update
    pub frames: u64,
    pub rate: u32,
    pub channels: usize,
    pub container: Container,
    pub encoding: Encoding,
    pub seekable: bool,
}

impl Info {
    fn from_raw(raw: &ffi::SfInfo) -> Result<Self> {
        let rate = u32::try_from(raw.samplerate).ok().filter(|r| *r > 0);
        let channels = usize::try_from(raw.channels).ok().filter(|c| *c > 0);
        let (Some(rate), Some(channels)) = (rate, channels) else {
            return Err(Error::new("sndfile.malformed")
                .arg("rate", i64::from(raw.samplerate))
                .arg("channels", i64::from(raw.channels)));
        };
        Ok(Self {
            frames: u64::try_from(raw.frames).unwrap_or(0),
            rate,
            channels,
            container: Container::from_format(raw.format),
            encoding: Encoding::from_format(raw.format),
            seekable: raw.seekable != 0,
        })
    }
}

/// The sample format of a WAV, RF64 or AIFF file
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sample {
    Int16,
    Int24,
    Float32,
}

impl Sample {
    fn subtype(self) -> c_int {
        match self {
            Self::Int16 => ffi::SF_FORMAT_PCM_16,
            Self::Int24 => ffi::SF_FORMAT_PCM_24,
            Self::Float32 => ffi::SF_FORMAT_FLOAT,
        }
    }
}

/// The bit depth of a FLAC file
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlacDepth {
    Int16,
    Int24,
}

/// The format a [`Writer`] writes
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WriteFormat {
    /// RIFF WAV: limited to 4 GiB; for exports
    Wav(Sample),
    /// RF64 (WAV without the 4 GiB limit): what the recorder writes (C10)
    Rf64(Sample),
    Aiff(Sample),
    /// FLAC, `compression` from 0 (fastest) to 1 (smallest)
    Flac {
        depth: FlacDepth,
        compression: f64,
    },
    /// OGG Vorbis, `quality` from 0 (smallest) to 1 (best)
    OggVorbis {
        quality: f64,
    },
    /// MP3 (MPEG-1/2 Layer III, through LAME), `quality` from 0 (smallest) to 1 (best). Only if the library was
    /// built with LAME ([`is_supported`])
    Mp3 {
        quality: f64,
    },
}

impl WriteFormat {
    fn code(self) -> c_int {
        match self {
            Self::Wav(s) => ffi::SF_FORMAT_WAV | s.subtype(),
            Self::Rf64(s) => ffi::SF_FORMAT_RF64 | s.subtype(),
            Self::Aiff(s) => ffi::SF_FORMAT_AIFF | s.subtype(),
            Self::Flac { depth: FlacDepth::Int16, .. } => ffi::SF_FORMAT_FLAC | ffi::SF_FORMAT_PCM_16,
            Self::Flac { depth: FlacDepth::Int24, .. } => ffi::SF_FORMAT_FLAC | ffi::SF_FORMAT_PCM_24,
            Self::OggVorbis { .. } => ffi::SF_FORMAT_OGG | ffi::SF_FORMAT_VORBIS,
            Self::Mp3 { .. } => ffi::SF_FORMAT_MPEG | ffi::SF_FORMAT_MPEG_LAYER_III,
        }
    }

    /// Whether [`Writer::sync`] leaves a file that reads back up to the sync if the process dies before
    /// [`Writer::close`]: the formats whose header holds the data size and is rewritten in place
    pub fn survives_a_crash(self) -> bool {
        matches!(self, Self::Wav(_) | Self::Rf64(_) | Self::Aiff(_))
    }
}

/// Whether the linked libsndfile can write `format` at `rate` with `channels` (MP3 needs LAME, OGG needs Vorbis)
pub fn is_supported(format: WriteFormat, rate: u32, channels: usize) -> bool {
    let (Ok(samplerate), Ok(channels)) = (c_int::try_from(rate), c_int::try_from(channels)) else {
        return false;
    };
    let info = ffi::SfInfo { samplerate, channels, format: format.code(), ..Default::default() };
    // SAFETY: `info` is a valid SF_INFO that sf_format_check only reads
    unsafe { ffi::sf_format_check(&info) != 0 }
}

/// libsndfile reports why an open failed in a global variable (`sf_error(NULL)`): the opens of this crate take turns,
/// so that the code read is the one of this open
static OPEN_LOCK: Mutex<()> = Mutex::new(());

/// The error of a failed open, by libsndfile's code
fn open_error(code: c_int, path: &Path) -> Error {
    let key = match code {
        1 => "sndfile.unrecognised_format",
        3 => "sndfile.malformed",
        4 => "sndfile.unsupported_encoding",
        _ => "sndfile.open_failed",
    };
    Error::new(key)
        .arg("path", path.display().to_string())
        .arg("code", i64::from(code))
        .arg("message", error_message(code))
}

fn io_error(key: &'static str, path: &Path, e: &std::io::Error) -> Error {
    Error::new(key).arg("path", path.display().to_string()).arg("message", e.to_string())
}

/// Opens `file` with libsndfile, which borrows its descriptor
fn open_fd(file: &File, mode: c_int, info: &mut ffi::SfInfo, path: &Path) -> Result<NonNull<ffi::Sndfile>> {
    let _turn = OPEN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: the descriptor is open and stays open while the handle lives (the File is kept beside the handle and
    // dropped after sf_close); close_desc = 0, so libsndfile never closes it; `info` is a valid SF_INFO
    let raw = unsafe { ffi::sf_open_fd(file.as_raw_fd(), mode, info, 0) };
    match NonNull::new(raw) {
        Some(handle) => Ok(handle),
        None => {
            // SAFETY: sf_error(NULL) reads the global error of the last open, ours (the opens take turns)
            let code = unsafe { ffi::sf_error(std::ptr::null_mut()) };
            Err(open_error(code, path))
        }
    }
}

fn to_count(frames: usize) -> ffi::SfCount {
    ffi::SfCount::try_from(frames).unwrap_or(ffi::SfCount::MAX)
}

/// A file open for reading. `Send`, not `Sync`; every call does I/O (never on the audio thread)
pub struct Reader {
    handle: NonNull<ffi::Sndfile>,
    info: Info,
    position: u64,
    path: Box<Path>,
    _file: File,
}

// SAFETY: a SNDFILE has no thread affinity (libsndfile keeps no thread-local state and its per-file state is plain
// memory). Every use takes &mut self, so only one thread uses it at a time, and the raw pointer keeps Reader !Sync.
unsafe impl Send for Reader {}

impl Reader {
    /// Opens a file of any format libsndfile reads (by its content, not its name).
    ///
    /// Errors: `sndfile.open_failed` (the OS could not open it, or another failure), `sndfile.unrecognised_format`,
    /// `sndfile.malformed`, `sndfile.unsupported_encoding`; each with the path
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| io_error("sndfile.open_failed", path, &e))?;
        Self::from_file(file, path)
    }

    /// Reads an already open file (rewound by the caller), which the reader
    /// then owns; `label` only names it in errors. For a file that has no path to open, as an unlinked temporary
    pub fn from_file(file: File, label: &Path) -> Result<Self> {
        let path = label;
        let mut raw = ffi::SfInfo::default();
        let handle = open_fd(&file, ffi::SFM_READ, &mut raw, path)?;
        let info = match Info::from_raw(&raw) {
            Ok(info) => info,
            Err(e) => {
                // SAFETY: the handle was just opened and no Reader owns it yet; `file` outlives the call (dropped after)
                unsafe { ffi::sf_close(handle.as_ptr()) };
                return Err(e);
            }
        };
        Ok(Self { handle, info, position: 0, path: path.into(), _file: file })
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    /// The frame the next read starts at
    pub fn position(&self) -> u64 {
        self.position
    }

    fn check_buffer(&self, len: usize) -> Result<usize> {
        if !len.is_multiple_of(self.info.channels) {
            return Err(Error::new("sndfile.bad_length")
                .arg("samples", len as i64)
                .arg("channels", self.info.channels as i64));
        }
        Ok(len / self.info.channels)
    }

    fn after_read(&mut self, got: ffi::SfCount) -> Result<usize> {
        if got < 0 {
            return Err(self.error("sndfile.read_failed"));
        }
        let got = got as usize;
        self.position += got as u64;
        Ok(got)
    }

    fn error(&self, key: &'static str) -> Error {
        // SAFETY: the handle is valid (owned by self)
        let code = unsafe { ffi::sf_error(self.handle.as_ptr()) };
        Error::new(key)
            .arg("path", self.path.display().to_string())
            .arg("code", i64::from(code))
            .arg("message", error_message(code))
    }

    /// Reads interleaved frames into `buf` (whole frames). Returns the frames read: fewer than asked only at the end
    /// of the file. Integer files are normalised to [-1, 1)
    pub fn read_f32(&mut self, buf: &mut [f32]) -> Result<usize> {
        let frames = self.check_buffer(buf.len())?;
        // SAFETY: the handle is valid and ours (&mut self); `buf` holds `frames` whole frames of `channels` floats,
        // and libsndfile writes at most that many
        let got = unsafe { ffi::sf_readf_float(self.handle.as_ptr(), buf.as_mut_ptr(), to_count(frames)) };
        self.after_read(got)
    }

    /// Reads as 32-bit integers, full scale (a 16-bit sample `s` reads `s << 16`, a 24-bit one `s << 8`)
    pub fn read_i32(&mut self, buf: &mut [i32]) -> Result<usize> {
        let frames = self.check_buffer(buf.len())?;
        // SAFETY: as in read_f32, with `frames` whole frames of i32
        let got = unsafe { ffi::sf_readf_int(self.handle.as_ptr(), buf.as_mut_ptr(), to_count(frames)) };
        self.after_read(got)
    }

    /// Reads as 16-bit integers (deeper files are truncated to 16 bits by libsndfile)
    pub fn read_i16(&mut self, buf: &mut [i16]) -> Result<usize> {
        let frames = self.check_buffer(buf.len())?;
        // SAFETY: as in read_f32, with `frames` whole frames of i16
        let got = unsafe { ffi::sf_readf_short(self.handle.as_ptr(), buf.as_mut_ptr(), to_count(frames)) };
        self.after_read(got)
    }

    /// Moves to `frame` (0 to `frames`). Error `sndfile.seek_failed` past the end or on a file that cannot seek
    pub fn seek(&mut self, frame: u64) -> Result<()> {
        let Ok(target) = ffi::SfCount::try_from(frame) else {
            return Err(Error::new("sndfile.seek_failed").arg("frame", i64::MAX));
        };
        if frame > self.info.frames {
            return Err(Error::new("sndfile.seek_failed")
                .arg("path", self.path.display().to_string())
                .arg("frame", target));
        }
        // SAFETY: the handle is valid and ours
        let at = unsafe { ffi::sf_seek(self.handle.as_ptr(), target, ffi::SEEK_SET) };
        if at < 0 {
            return Err(self.error("sndfile.seek_failed").arg("frame", target));
        }
        self.position = at as u64;
        Ok(())
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // SAFETY: the handle came from sf_open_fd, is owned by self alone and closed exactly once, here; the File
        // (a later field) is dropped after this, so the descriptor outlives the handle
        unsafe { ffi::sf_close(self.handle.as_ptr()) };
    }
}

impl fmt::Debug for Reader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("path", &self.path)
            .field("info", &self.info)
            .field("position", &self.position)
            .finish()
    }
}

/// What [`Writer::create`] does when the file exists
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Existing {
    /// Fail with `sndfile.exists` (the recorder's provisional `rec-<uuid>` file must be new)
    Fail,
    /// Replace it (an export the user agreed to overwrite)
    Replace,
}

/// What a [`Writer`] writes
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WriteSpec {
    pub format: WriteFormat,
    pub rate: u32,
    pub channels: usize,
}

/// A file open for writing. `Send`, not `Sync`; every call does I/O: the recorder calls it from its disk thread only
/// (U6), never from the audio thread
pub struct Writer {
    handle: NonNull<ffi::Sndfile>,
    spec: WriteSpec,
    frames: u64,
    path: Box<Path>,
    file: File,
}

// SAFETY: as for Reader: no thread affinity, &mut self on every use, and the raw pointer keeps Writer !Sync
unsafe impl Send for Writer {}

impl Writer {
    /// Creates the file and writes its header.
    ///
    /// Integer formats clip samples outside [-1, 1] instead of wrapping. Errors: `sndfile.exists`,
    /// `sndfile.open_failed`, `sndfile.unsupported_format` (this libsndfile cannot write it), `sndfile.bad_setting`
    pub fn create(path: &Path, spec: WriteSpec, existing: Existing) -> Result<Self> {
        if !is_supported(spec.format, spec.rate, spec.channels) {
            return Err(Error::new("sndfile.unsupported_format")
                .arg("path", path.display().to_string())
                .arg("format", format!("{:?}", spec.format))
                .arg("rate", i64::from(spec.rate))
                .arg("channels", spec.channels as i64));
        }
        let mut options = OpenOptions::new();
        options.write(true).read(true);
        match existing {
            Existing::Fail => options.create_new(true),
            Existing::Replace => options.create(true).truncate(true),
        };
        let file = options.open(path).map_err(|e| {
            let key =
                if e.kind() == std::io::ErrorKind::AlreadyExists { "sndfile.exists" } else { "sndfile.open_failed" };
            io_error(key, path, &e)
        })?;
        let mut raw = ffi::SfInfo {
            samplerate: spec.rate as c_int,
            channels: spec.channels as c_int,
            format: spec.format.code(),
            ..Default::default()
        };
        let handle = open_fd(&file, ffi::SFM_WRITE, &mut raw, path)?;
        let mut writer = Self { handle, spec, frames: 0, path: path.into(), file };

        // Settings that must come before the first write
        // SAFETY: SFC_SET_CLIPPING takes no data pointer (the flag goes in `datasize`); the handle is valid
        unsafe { ffi::sf_command(writer.handle.as_ptr(), ffi::SFC_SET_CLIPPING, std::ptr::null_mut(), ffi::SF_TRUE) };
        // No PEAK chunk: libsndfile stamps it with the wall-clock time, so two writes of the same samples would differ
        // in their bytes (T19: an export is bit-identical every time)
        // SAFETY: SFC_SET_ADD_PEAK_CHUNK takes no data pointer (the flag goes in `datasize`); the handle is valid
        unsafe {
            ffi::sf_command(writer.handle.as_ptr(), ffi::SFC_SET_ADD_PEAK_CHUNK, std::ptr::null_mut(), ffi::SF_FALSE)
        };
        match spec.format {
            WriteFormat::Flac { compression, .. } => {
                writer.set_double(ffi::SFC_SET_COMPRESSION_LEVEL, compression)?;
            }
            WriteFormat::OggVorbis { quality } => {
                writer.set_double(ffi::SFC_SET_VBR_ENCODING_QUALITY, quality)?;
            }
            WriteFormat::Mp3 { quality } => {
                // The MP3 encoder takes a compression level, the inverse of a quality
                if !(0.0..=1.0).contains(&quality) {
                    return Err(Error::new("sndfile.bad_setting").arg("value", quality));
                }
                writer.set_double(ffi::SFC_SET_COMPRESSION_LEVEL, 1.0 - quality)?;
            }
            _ => {}
        }
        Ok(writer)
    }

    fn set_double(&mut self, command: c_int, value: f64) -> Result<()> {
        if !(0.0..=1.0).contains(&value) {
            return Err(Error::new("sndfile.bad_setting").arg("value", value));
        }
        let mut data: c_double = value;
        // SAFETY: the command reads one double from `data`, whose size is passed; the handle is valid
        let ok = unsafe {
            ffi::sf_command(
                self.handle.as_ptr(),
                command,
                (&mut data as *mut c_double).cast::<c_void>(),
                size_of::<c_double>() as c_int,
            )
        };
        if ok != ffi::SF_TRUE {
            return Err(self.error("sndfile.bad_setting").arg("value", value));
        }
        Ok(())
    }

    fn error(&self, key: &'static str) -> Error {
        // SAFETY: the handle is valid (owned by self)
        let code = unsafe { ffi::sf_error(self.handle.as_ptr()) };
        Error::new(key)
            .arg("path", self.path.display().to_string())
            .arg("code", i64::from(code))
            .arg("message", error_message(code))
    }

    pub fn spec(&self) -> WriteSpec {
        self.spec
    }

    /// Frames written so far
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The file being written, for what the recorder does around libsndfile (`fstat`, its own `fsync`). Writing to
    /// it directly would corrupt the file
    pub fn file(&self) -> &File {
        &self.file
    }

    fn check_buffer(&self, len: usize) -> Result<usize> {
        if !len.is_multiple_of(self.spec.channels) {
            return Err(Error::new("sndfile.bad_length")
                .arg("samples", len as i64)
                .arg("channels", self.spec.channels as i64));
        }
        Ok(len / self.spec.channels)
    }

    fn after_write(&mut self, frames: usize, written: ffi::SfCount) -> Result<()> {
        let written = u64::try_from(written).unwrap_or(0);
        self.frames += written;
        if written != frames as u64 {
            // A short write is the disk full (or an I/O error): what was written stays, and the header can still be
            // updated to end there (H3)
            return Err(self.error("sndfile.write_failed").arg("written", written as i64).arg("asked", frames as i64));
        }
        Ok(())
    }

    /// Writes interleaved frames (whole frames). A short write (disk full) is the error `sndfile.write_failed`, with
    /// the frames that were written counted in [`frames`](Self::frames)
    pub fn write_f32(&mut self, samples: &[f32]) -> Result<()> {
        let frames = self.check_buffer(samples.len())?;
        // SAFETY: the handle is valid and ours (&mut self); `samples` holds `frames` whole frames that libsndfile only
        // reads
        let written = unsafe { ffi::sf_writef_float(self.handle.as_ptr(), samples.as_ptr(), to_count(frames)) };
        self.after_write(frames, written)
    }

    /// Writes 32-bit integers, full scale (a 24-bit file keeps the top 24 bits, a 16-bit file the top 16)
    pub fn write_i32(&mut self, samples: &[i32]) -> Result<()> {
        let frames = self.check_buffer(samples.len())?;
        // SAFETY: as in write_f32, with i32 frames
        let written = unsafe { ffi::sf_writef_int(self.handle.as_ptr(), samples.as_ptr(), to_count(frames)) };
        self.after_write(frames, written)
    }

    /// Writes 16-bit integers
    pub fn write_i16(&mut self, samples: &[i16]) -> Result<()> {
        let frames = self.check_buffer(samples.len())?;
        // SAFETY: as in write_f32, with i16 frames
        let written = unsafe { ffi::sf_writef_short(self.handle.as_ptr(), samples.as_ptr(), to_count(frames)) };
        self.after_write(frames, written)
    }

    /// Rewrites the header with the frames written so far (WAV, RF64, AIFF: the data size; other formats ignore it)
    pub fn update_header(&mut self) -> Result<()> {
        // SAFETY: SFC_UPDATE_HEADER_NOW takes no data; the handle is valid and ours
        unsafe { ffi::sf_command(self.handle.as_ptr(), ffi::SFC_UPDATE_HEADER_NOW, std::ptr::null_mut(), 0) };
        // SAFETY: as above
        let code = unsafe { ffi::sf_error(self.handle.as_ptr()) };
        if code != 0 {
            return Err(self.error("sndfile.write_failed"));
        }
        Ok(())
    }

    /// The recorder's checkpoint, every ~2 s (I-11): rewrites the header and makes data and header durable
    /// (`fdatasync`; `F_FULLFSYNC` on macOS, U6). If the process dies after this, the file reads back up to here
    /// ([`WriteFormat::survives_a_crash`])
    pub fn sync(&mut self) -> Result<()> {
        self.update_header()?;
        self.file.sync_data().map_err(|e| io_error("sndfile.sync_failed", &self.path, &e))
    }

    /// Makes sure the disk has room for the file up to `total_bytes` (H3: the recorder keeps ~30 s reserved ahead),
    /// without changing the file's size, so libsndfile and a reader after a crash see only what was written. Linux
    /// `fallocate(FALLOC_FL_KEEP_SIZE)`, macOS `F_PREALLOCATE`. Errors: `sndfile.disk_full` (no room: stop the
    /// recording now, with margin to close cleanly), `sndfile.reserve_unsupported` (the file system cannot
    /// reserve: fall back to watching the free space)
    pub fn reserve(&mut self, total_bytes: u64) -> Result<()> {
        reserve::reserve(&self.file, total_bytes).map_err(|e| {
            let key = match e.raw_os_error() {
                Some(code) if code == reserve::ENOSPC => "sndfile.disk_full",
                Some(code) if reserve::UNSUPPORTED.contains(&code) => "sndfile.reserve_unsupported",
                _ => "sndfile.reserve_failed",
            };
            io_error(key, &self.path, &e).arg("bytes", total_bytes as i64)
        })
    }

    /// Finishes the file (final header, codec flush) and makes it durable. Returns the frames written. Dropping a
    /// `Writer` also closes it, but without reporting an error
    pub fn close(self) -> Result<u64> {
        let this = std::mem::ManuallyDrop::new(self);
        // SAFETY: the handle is valid and closed exactly once: `this` is never dropped, so Drop does not close it again
        let code = unsafe { ffi::sf_close(this.handle.as_ptr()) };
        // SAFETY: the fields are moved out of `this` exactly once and `this` is never used or dropped afterwards
        let (file, path) = unsafe { (std::ptr::read(&this.file), std::ptr::read(&this.path)) };
        let frames = this.frames;
        if code != 0 {
            return Err(Error::new("sndfile.close_failed")
                .arg("path", path.display().to_string())
                .arg("code", i64::from(code))
                .arg("message", error_message(code)));
        }
        if matches!(this.spec.format, WriteFormat::Rf64(_)) {
            clear_peak_time(&file).map_err(|e| io_error("sndfile.write_failed", &path, &e))?;
        }
        if matches!(this.spec.format, WriteFormat::OggVorbis { .. }) {
            fix_ogg_serial(&file).map_err(|e| io_error("sndfile.write_failed", &path, &e))?;
        }
        file.sync_all().map_err(|e| io_error("sndfile.sync_failed", &path, &e))?;
        Ok(frames)
    }
}

/// Zeroes the timestamp of an RF64 file's PEAK chunk.
///
/// libsndfile honours `SFC_SET_ADD_PEAK_CHUNK` for WAV and AIFF but not for RF64, and stamps the chunk with the
/// wall-clock time: without this, the same samples would give different bytes a second later (T19). The peaks
/// themselves stay; the time is informational and 0 is a valid value
/// The serial number every page of our Ogg files carries (any fixed value: one logical stream per file)
const OGG_SERIAL: u32 = 0x4C4D_0001;

/// Gives every page of an Ogg file the same fixed serial number, with each page's CRC computed again.
///
/// libsndfile picks the stream's serial number at random (seeded by the clock): the same samples would give
/// different bytes at every export (T19). The serial only has to differ between the streams of one file, and our
/// files have one
fn fix_ogg_serial(file: &File) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    let len = file.metadata()?.len();
    let mut at = 0u64;
    let mut page = Vec::new();
    while at + 27 <= len {
        let mut head = [0u8; 27];
        file.read_exact_at(&mut head, at)?;
        if &head[..4] != b"OggS" {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not an Ogg page"));
        }
        let segments = usize::from(head[26]);
        let mut table = vec![0u8; segments];
        file.read_exact_at(&mut table, at + 27)?;
        let body: u64 = table.iter().map(|&b| u64::from(b)).sum();
        let size = 27 + segments as u64 + body;
        page.resize(usize::try_from(size).unwrap_or(0), 0);
        file.read_exact_at(&mut page, at)?;
        page[14..18].copy_from_slice(&OGG_SERIAL.to_le_bytes());
        page[22..26].fill(0);
        let crc = ogg_crc(&page);
        page[22..26].copy_from_slice(&crc.to_le_bytes());
        file.write_all_at(&page[14..26], at + 14)?;
        at += size;
    }
    Ok(())
}

/// The CRC of an Ogg page (polynomial 0x04C11DB7, no reflection, initial 0, as RFC 3533 says)
fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

fn clear_peak_time(file: &File) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    // RF64 + size + WAVE, then chunks (id, little-endian size, data padded to even) up to `data`
    let mut at = 12u64;
    let mut head = [0u8; 8];
    for _ in 0..64 {
        if file.read_at(&mut head, at)? < head.len() {
            return Ok(());
        }
        let size = u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]]));
        match &head[..4] {
            b"data" => return Ok(()),
            // version (4 bytes), then the timestamp (4 bytes)
            b"PEAK" if size >= 8 => return file.write_all_at(&[0; 4], at + 8 + 4),
            _ => at += 8 + size + (size & 1),
        }
    }
    Ok(())
}

impl AsFd for Writer {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // SAFETY: the handle came from sf_open_fd, is owned by self alone and closed exactly once, here (close() never
        // reaches Drop); the File is dropped after this
        unsafe { ffi::sf_close(self.handle.as_ptr()) };
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("path", &self.path)
            .field("spec", &self.spec)
            .field("frames", &self.frames)
            .finish()
    }
}

/// Reserving disk space without changing the file size
mod reserve {
    use std::fs::File;
    use std::io;

    #[cfg(target_os = "linux")]
    mod os {
        use std::ffi::c_int;
        use std::fs::File;
        use std::io;
        use std::os::fd::AsRawFd;

        pub const ENOSPC: i32 = 28;
        /// EOPNOTSUPP (95) and ENOSYS (38)
        pub const UNSUPPORTED: [i32; 2] = [95, 38];
        const FALLOC_FL_KEEP_SIZE: c_int = 1;

        unsafe extern "C" {
            fn fallocate(fd: c_int, mode: c_int, offset: i64, len: i64) -> c_int;
        }

        pub fn reserve(file: &File, total: u64) -> io::Result<()> {
            let len = i64::try_from(total).map_err(|_| io::Error::from_raw_os_error(27 /* EFBIG */))?;
            if len == 0 {
                return Ok(());
            }
            // SAFETY: fallocate on an open descriptor we own (borrowed for the call); it does not touch our memory
            let code = unsafe { fallocate(file.as_raw_fd(), FALLOC_FL_KEEP_SIZE, 0, len) };
            if code != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    #[cfg(target_os = "macos")]
    mod os {
        use std::ffi::c_int;
        use std::fs::File;
        use std::io;
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;

        pub const ENOSPC: i32 = 28;
        /// ENOTSUP (45) and EOPNOTSUPP (102)
        pub const UNSUPPORTED: [i32; 2] = [45, 102];
        const F_PREALLOCATE: c_int = 42;
        const F_ALLOCATECONTIG: u32 = 0x2;
        const F_ALLOCATEALL: u32 = 0x4;
        const F_PEOFPOSMODE: c_int = 3;

        /// `fstore_t` of `<sys/fcntl.h>`
        #[repr(C)]
        struct Fstore {
            fst_flags: u32,
            fst_posmode: c_int,
            fst_offset: i64,
            fst_length: i64,
            fst_bytesalloc: i64,
        }

        unsafe extern "C" {
            fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
        }

        pub fn reserve(file: &File, total: u64) -> io::Result<()> {
            // What the file already has on disk (allocated blocks of 512 bytes, F_PREALLOCATE included)
            let allocated = file.metadata()?.blocks().saturating_mul(512);
            if total <= allocated {
                return Ok(());
            }
            let length =
                i64::try_from(total - allocated).map_err(|_| io::Error::from_raw_os_error(27 /* EFBIG */))?;
            let fd = file.as_raw_fd();
            for flags in [F_ALLOCATECONTIG | F_ALLOCATEALL, F_ALLOCATEALL] {
                let mut store = Fstore {
                    fst_flags: flags,
                    fst_posmode: F_PEOFPOSMODE,
                    fst_offset: 0,
                    fst_length: length,
                    fst_bytesalloc: 0,
                };
                // SAFETY: F_PREALLOCATE reads and writes the fstore_t passed by pointer, valid for the call; the
                // descriptor is open (borrowed from `file`)
                let code = unsafe { fcntl(fd, F_PREALLOCATE, &mut store as *mut Fstore) };
                if code != -1 {
                    return Ok(());
                }
            }
            Err(io::Error::last_os_error())
        }
    }

    pub use os::{ENOSPC, UNSUPPORTED};

    pub fn reserve(file: &File, total: u64) -> io::Result<()> {
        os::reserve(file, total)
    }
}
