//! The build's reading of the source: which format the state names, and the importer that turns it into the
//! instrument. Runs in an `Operation`, off the audio thread.

use std::sync::Arc;

use lm_base::{Error, Result};
use lm_plugin::{BuildContext, Loaded, StateView};
use lm_sampler::import::audio::{self, AudioOptions};
use lm_sampler::import::drums::{self, Piece};
use lm_sampler::import::pat::{self, PatOptions};
use lm_sampler::import::{avl, gig, sf2, sfz};
use lm_sampler::stream::StreamPlan;
use lm_sampler::{Instrument, LoopMode, Pcm, Quality, Window};
use lm_state::Value;

use crate::kits::{self, AvlKit, DmKit};
use crate::load::{FileLoader, OggDecoder, decode_file};

/// The formats the state may name
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    None,
    Sf2,
    Sfz,
    Pat,
    Audio,
    Kit,
    Gig,
}

impl Format {
    pub fn from_key(key: &str) -> Option<Format> {
        Some(match key {
            "none" => Format::None,
            "sf2" => Format::Sf2,
            "sfz" => Format::Sfz,
            "pat" => Format::Pat,
            "audio" => Format::Audio,
            "kit" => Format::Kit,
            "gig" => Format::Gig,
            _ => return None,
        })
    }
}

/// What a build reads from the state, besides the parameters
#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub format: Format,
    pub quality: Quality,
    pub multi: bool,
}

/// The result of the import
#[derive(Debug)]
pub struct Built {
    pub instrument: Arc<Instrument>,
    pub source: Source,
}

fn read_bytes(cx: &BuildContext<'_>, r: &lm_state::Resource) -> Result<Arc<[u8]>> {
    match cx.resources.get(r) {
        Some(Loaded::Bytes(b)) => Ok(b.clone()),
        Some(Loaded::File(path)) => std::fs::read(path).map(Into::into).map_err(|e| {
            Error::new("sampler.read_failed").arg("path", path.display().to_string()).arg("error", e.to_string())
        }),
        _ => cx.resources.bytes(r),
    }
}

/// A file from this size on streams its long samples: the SoundFonts and GIGs that fit in memory stay whole
const STREAM_FILE_BYTES: u64 = 256 << 20;

/// The plan to stream the samples of the file a resource is, when it is a file large enough (the bytes the importer
/// parses are read from the same file)
fn stream_plan(cx: &BuildContext<'_>, r: &lm_state::Resource) -> Option<StreamPlan> {
    let Some(Loaded::File(path)) = cx.resources.get(r) else { return None };
    let file = std::fs::File::open(path).ok()?;
    (file.metadata().ok()?.len() >= STREAM_FILE_BYTES).then(|| StreamPlan::new(Arc::new(file)))
}

fn pcm_of(data: &lm_render::SampleData) -> Pcm {
    if data.channels() > 1 {
        Pcm::stereo(data.rate(), data.channel(0).to_vec(), data.channel(1).to_vec())
    } else {
        Pcm::mono(data.rate(), data.channel(0).to_vec())
    }
}

/// The sample of a state's kit piece
fn kit_pieces(kit: &DmKit, rate: u32) -> Result<Vec<Piece>> {
    let dir = kits::samples_dir().ok_or_else(|| {
        Error::new("plugin.sampler.no_samples").arg("folders", format!("{:?}", kits::samples_folders()))
    })?;
    let folder = dir.join(kit.folder);
    // One generator for the kit's pieces, as the legacy decoder's per-thread one
    let mut rand = drums::kit_rand();
    let mut out = Vec::new();
    for piece in kit.pieces {
        let path = folder.join(piece.file);
        let pcm = if piece.file.ends_with(".ds") {
            std::fs::read(&path).ok().and_then(|text| drums::render_ds(&text, rate, &mut rand))
        } else {
            decode_file(&path).ok().map(Arc::new)
        };
        // (a piece that cannot be read stays silent: it is left out)
        if let Some(pcm) = pcm {
            out.push(Piece { note: piece.note, choke: u32::try_from(piece.choke.max(0)).unwrap_or(0), pcm });
        }
    }
    Ok(out)
}

pub fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Built> {
    let s = StateView::new(state);
    let format = s.choice("format", Format::from_key)?;
    let quality = s.choice("interpolation", Quality::from_key)?;
    let multi = crate::multi(state);
    let source = Source { format, quality, multi };
    let instrument = match format {
        Format::None => Instrument::new("empty"),
        Format::Sf2 => {
            let Some(r) = s.resource("soundfont")? else { return Ok(empty(source)) };
            let bytes = read_bytes(cx, r)?;
            let bank = u16::try_from(s.int("bank")?).unwrap_or(0);
            let program = u16::try_from(s.int("program")?).unwrap_or(0);
            let font = sf2::SoundFont::parse(&bytes)?;
            let font = match stream_plan(cx, r) {
                Some(plan) => font.streaming(plan),
                None => font,
            };
            font.import(bank, program, Some(&mut OggDecoder))?
        }
        Format::Sfz => {
            let Some(r) = s.resource("sfz")? else { return Ok(empty(source)) };
            let path = cx.resources.file(r)?;
            let text = std::fs::read_to_string(&path).map_err(|e| {
                Error::new("sampler.read_failed").arg("path", path.display().to_string()).arg("error", e.to_string())
            })?;
            let folder = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
            sfz::import(&text, &folder, &mut FileLoader)?
        }
        Format::Pat => {
            let Some(r) = s.resource("patch")? else { return Ok(empty(source)) };
            let bytes = read_bytes(cx, r)?;
            pat::import(&bytes, PatOptions { tuned: s.boolean("patchTuned")?, looped: s.boolean("patchLooped")? })?
        }
        Format::Audio => {
            let Some(r) = s.resource("sample")? else { return Ok(empty(source)) };
            let data = cx.resources.audio(r)?;
            let loop_mode = s.choice("loop", |k| match k {
                "off" => Some(LoopMode::Off),
                "on" => Some(LoopMode::Forward),
                "pingPong" => Some(LoopMode::PingPong),
                _ => None,
            })?;
            let options =
                AudioOptions { loop_mode, window: Window::default(), reverse: false, stutter: false, amplify: 1.0 };
            audio::import(Arc::new(pcm_of(&data)), options)?
        }
        Format::Kit => {
            let key = s.text("kit")?;
            if let Some(avl_kit) = AvlKit::from_key(key) {
                let dir = kits::kits_dir().ok_or_else(|| {
                    Error::new("plugin.sampler.no_kits").arg("folders", format!("{:?}", kits::kit_folders()))
                })?;
                let bytes = std::fs::read(dir.join(avl_kit.file)).map_err(|e| {
                    Error::new("sampler.read_failed").arg("kit", avl_kit.name).arg("error", e.to_string())
                })?;
                avl::import(&bytes, multi, Some(&mut OggDecoder)).map_err(|e| e.arg("kit", avl_kit.name))?
            } else if let Some(dm) = DmKit::from_key(key) {
                let pieces = kit_pieces(dm, cx.spec.sample_rate)?;
                drums::import(&pieces, multi)?
            } else {
                return Err(Error::new("plugin.state.choice").arg("key", "kit").arg("value", key));
            }
        }
        Format::Gig => {
            let Some(r) = s.resource("gig")? else { return Ok(empty(source)) };
            let bytes = read_bytes(cx, r)?;
            let bank = u16::try_from(s.int("bank")?).unwrap_or(0);
            let program = u16::try_from(s.int("program")?).unwrap_or(0);
            match stream_plan(cx, r) {
                Some(plan) => gig::import_streaming(&bytes, bank, program, plan)?,
                None => gig::import(&bytes, bank, program)?,
            }
        }
    };
    Ok(Built { instrument: Arc::new(instrument), source })
}

fn empty(source: Source) -> Built {
    Built { instrument: Arc::new(Instrument::new("empty")), source }
}
