//! What reads files for the Sampler's build (off the audio thread): the samples an SFZ cites and the compressed
//! samples of an `.sf3`, through libsndfile where the pure WAVE reader does not apply.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use lm_base::{Error, Result};
use lm_sampler::import::sf2::Sf3Decoder;
use lm_sampler::{Pcm, SampleLoader, read_wav};
use lm_sys_sndfile::Reader;

/// A file's frames as a [`Pcm`] by libsndfile (FLAC, Ogg, AIFF, MP3, WAVE of any depth)
pub fn decode_file(path: &Path) -> Result<Pcm> {
    let mut reader = Reader::open(path)?;
    let info = *reader.info();
    let channels = info.channels.max(1);
    let total = usize::try_from(info.frames).unwrap_or(0);
    let mut data = vec![0.0f32; total * channels];
    let mut at = 0;
    while at < data.len() {
        let got = reader.read_f32(&mut data[at..])?;
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

/// The decoder of the compressed samples of an `.sf3`: each Ogg stream goes through a temporary file, as
/// libsndfile opens files
#[derive(Debug, Default)]
pub struct OggDecoder;

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl Sf3Decoder for OggDecoder {
    fn decode(&mut self, ogg: &[u8], rate: u32) -> Result<Vec<f32>> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("lm-sampler-{}-{n}.ogg", std::process::id()));
        std::fs::write(&path, ogg).map_err(|e| Error::new("sampler.sf3.temp").arg("error", e.to_string()))?;
        let decoded = decode_file(&path);
        // (a file left behind in the temp folder is harmless)
        let _ = std::fs::remove_file(&path);
        let pcm = decoded?;
        let _ = rate;
        Ok(pcm.left().to_vec())
    }
}
