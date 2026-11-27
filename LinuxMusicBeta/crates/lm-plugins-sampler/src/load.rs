//! What reads files for the Sampler's build (off the audio thread): the samples an SFZ cites and the compressed
//! samples of an `.sf3`, through libsndfile where the pure WAVE reader does not apply.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use lm_base::{Error, Result};
use lm_sampler::import::sf2::Sf3Decoder;
use lm_sampler::{Pcm, SampleLoader, read_wav};
use lm_sys_sndfile::Reader;

/// The most samples (frames times channels) a decoded file may hold: 1 GiB of floats
const MAX_SAMPLES: usize = 1 << 28;
/// How many frames are read per step
const CHUNK_FRAMES: usize = 1 << 16;

/// A file's frames as a [`Pcm`] by libsndfile (FLAC, Ogg, AIFF, MP3, WAVE of any depth)
pub fn decode_file(path: &Path) -> Result<Pcm> {
    decode_reader(Reader::open(path)?)
}

fn decode_reader(mut reader: Reader) -> Result<Pcm> {
    let info = *reader.info();
    let channels = info.channels.max(1);
    // The frame count is the header's claim: the buffer grows with what is really read, up to a ceiling
    let ceiling = usize::try_from(info.frames).unwrap_or(0).min(MAX_SAMPLES / channels).saturating_mul(channels);
    let mut data: Vec<f32> = Vec::new();
    let mut at = 0;
    while at < ceiling {
        let end = (at + CHUNK_FRAMES * channels).min(ceiling);
        data.resize(end, 0.0);
        let got = reader.read_f32(&mut data[at..end])?;
        if got == 0 {
            break;
        }
        at += got * channels;
    }
    data.truncate(at - at % channels);
    let left: Vec<f32> = data.chunks_exact(channels).map(|f| f[0]).collect();
    if channels == 1 {
        return Ok(Pcm::mono(info.rate, left));
    }
    let right: Vec<f32> = data.chunks_exact(channels).map(|f| f[1]).collect();
    Ok(Pcm::stereo(info.rate, left, right))
}

/// The loader of the SFZ importer: WAVE by the pure reader, anything else by libsndfile
#[derive(Clone, Copy, Debug, Default)]
pub struct FileLoader;

impl SampleLoader for FileLoader {
    fn load(&mut self, path: &Path) -> Result<Pcm> {
        let bytes = std::fs::read(path).map_err(|e| {
            Error::new("sampler.read_failed").arg("path", path.display().to_string()).arg("error", e.to_string())
        })?;
        match read_wav(&bytes) {
            Ok(p) => Ok(p),
            Err(_) => decode_file(path),
        }
    }
}

/// The decoder of the compressed samples of an `.sf3`: each Ogg stream goes through an anonymous temporary file, as
/// libsndfile reads files, not memory
#[derive(Debug, Default)]
pub struct OggDecoder;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A file made for this stream alone and already unlinked: created exclusively (`O_EXCL`, which also refuses a symlink
/// planted at the name) with owner-only permissions and a name nobody can guess (the OS-seeded hasher over the pid, a
/// counter and the clock), then removed from the folder at once, so it vanishes with the descriptor whatever happens
/// next, a crash included. Holds `ogg` and is rewound
fn anonymous_file(ogg: &[u8]) -> std::io::Result<std::fs::File> {
    use std::hash::{BuildHasher, Hasher};
    use std::io::{Seek, Write};
    use std::os::unix::fs::OpenOptionsExt;

    let random = std::collections::hash_map::RandomState::new();
    for _ in 0..16 {
        let mut h = random.build_hasher();
        h.write_u32(std::process::id());
        h.write_u32(COUNTER.fetch_add(1, Ordering::Relaxed));
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        let path = std::env::temp_dir().join(format!("lm-sampler-{:016x}.ogg", h.finish()));
        match std::fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path) {
            Ok(mut file) => {
                let _ = std::fs::remove_file(&path);
                file.write_all(ogg)?;
                file.rewind()?;
                return Ok(file);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free temporary name"))
}

impl Sf3Decoder for OggDecoder {
    /// `rate` is the sample header's; the stream's own rate rules the decoded data (as before, it is not resampled)
    fn decode(&mut self, ogg: &[u8], _rate: u32) -> Result<Vec<f32>> {
        let file = anonymous_file(ogg).map_err(|e| Error::new("sampler.sf3.temp").arg("error", e.to_string()))?;
        let pcm = decode_reader(Reader::from_file(file, Path::new("sf3 sample"))?)?;
        Ok(pcm.left().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mono 16-bit WAVE of `frames` frames whose header claims `claimed` bytes of data
    fn wav(frames: usize, claimed: u32) -> Vec<u8> {
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(36 + claimed).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&44_100u32.to_le_bytes());
        v.extend_from_slice(&88_200u32.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&claimed.to_le_bytes());
        v.extend((0..frames).flat_map(|i| ((i % 1000) as i16).to_le_bytes()));
        v
    }

    fn file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("lm-load-test-{}-{name}.wav", std::process::id()));
        std::fs::write(&path, bytes).expect("writes");
        path
    }

    #[test]
    fn the_temporary_file_comes_back_rewound_and_each_call_makes_its_own() {
        use std::io::Read;
        let mut a = anonymous_file(b"abc").expect("made");
        let mut b = anonymous_file(b"xyz").expect("made");
        let (mut ta, mut tb) = (String::new(), String::new());
        a.read_to_string(&mut ta).expect("rewound");
        b.read_to_string(&mut tb).expect("rewound");
        assert_eq!((ta.as_str(), tb.as_str()), ("abc", "xyz"));
    }

    #[test]
    fn a_file_longer_than_one_chunk_reads_whole() {
        let frames = CHUNK_FRAMES * 2 + 17;
        let path = file("long", &wav(frames, (frames * 2) as u32));
        let pcm = decode_file(&path).expect("decodes");
        std::fs::remove_file(&path).ok();
        assert_eq!(pcm.frames(), frames);
    }

    #[test]
    fn a_header_that_claims_more_than_the_file_holds_gives_what_is_there() {
        let path = file("lying", &wav(100, u32::MAX - 100));
        let pcm = decode_file(&path);
        std::fs::remove_file(&path).ok();
        if let Ok(p) = pcm {
            assert!(p.frames() <= 100);
        }
    }
}
