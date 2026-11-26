//! LADSPA plugins in the helper, through `lm-sys-ladspa`: the files (found without running plugin code), the listing
//! of one file (opening it runs the library's constructors: the scan does it in a helper of its own per file), and
//! an instance behind [`Plugin`].
//!
//! - The plugin is identified by `<file stem>:<label>` (`uri`; a label is unique only inside its file); the file it
//!   was found in is tried first, and when it moved the search paths are looked through.
//! - A parameter's key is `p<port index>`. The control input ports are the parameters, with the hint ranges and
//!   defaults of the spec; a port with an open side gets the default (or 0 / 1) as that side of its range.
//! - LADSPA has no state beyond the parameters (the core keeps their values) and no events: a parameter change is
//!   applied at its offset by cutting the block there; notes are ignored (a LADSPA plugin is an effect or a generator).
//! - An output control port named `latency` is the plugin's latency.
//! - The bundled sets (calf, caps, cmt, swh, tap) are looked for in `LINUXMUSIC_LADSPA_PATH`, then next to the
//!   program (`ladspa/`, `../lib/linuxmusic/ladspa`); see [`lm_sys_ladspa::search_paths`].

use std::path::{Path, PathBuf};

use lm_base::{Error, Result};
use lm_render::EventKind;
use lm_sys_ladspa::{Descriptor, Instance, Library, Port};

use crate::backend::{Listed, Param, Plugin, Transport};

/// The most audio channels a LADSPA plugin gets (the block's channel lists are on the stack)
const MAX_CHANNELS: usize = 32;

/// The shared libraries under the given folders and the system's (`LADSPA_PATH`, the install's, the user's, the system's)
pub fn files(paths: &[PathBuf], system: bool) -> Vec<PathBuf> {
    let mut dirs = paths.to_vec();
    if system {
        dirs.extend(lm_sys_ladspa::search_paths());
    }
    lm_sys_ladspa::find_files(&dirs)
}

/// `<file stem>:<label>`
pub fn uri_of(file: &Path, label: &str) -> String {
    format!("{}:{label}", file.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()))
}

fn param_of(p: &Port, rate: f64) -> Param {
    let default = p.default(rate);
    let (lo, hi) = p.range(rate);
    let min = lo.unwrap_or_else(|| default.min(0.0));
    let max = hi.unwrap_or_else(|| default.max(1.0)).max(min);
    Param {
        key: format!("p{}", p.index),
        name: p.name.clone(),
        min,
        max,
        default,
        integer: p.integer() || p.toggled(),
        toggled: p.toggled(),
        logarithmic: p.logarithmic() && min > 0.0,
    }
}

fn listed(file: &Path, d: &Descriptor) -> Listed {
    Listed {
        format: "ladspa".into(),
        uri: uri_of(file, &d.label),
        name: d.name.clone(),
        instrument: false,
        inputs: d.audio_inputs(),
        outputs: d.audio_outputs(),
        midi_in: false,
        params: d.controls().map(|p| param_of(p, 48_000.0)).collect(),
        latency: d.latency_port().is_some(),
        has_window: false,
        file: Some(file.display().to_string()),
    }
}

/// The plugins of one file (or of every file found)
pub fn list(paths: &[PathBuf], file: Option<&Path>, system: bool) -> Result<Vec<Listed>> {
    let all = match file {
        Some(f) => vec![f.to_path_buf()],
        None => files(paths, system),
    };
    let mut out = Vec::new();
    for f in all {
        let lib = match Library::open(&f) {
            Ok(l) => l,
            Err(e) if file.is_some() => return Err(e),
            Err(_) => continue,
        };
        out.extend(lib.descriptors().iter().map(|d| listed(&f, d)));
    }
    Ok(out)
}

/// One LADSPA plugin
pub struct Ladspa {
    inst: Instance,
    params: Vec<Param>,
}

impl Ladspa {
    pub fn open(paths: &[PathBuf], file: Option<&Path>, uri: &str, rate: u32, max_block: usize) -> Result<Ladspa> {
        let found = |f: &Path| -> Option<(std::sync::Arc<Library>, String)> {
            let lib = Library::open(f).ok()?;
            let label = lib.descriptors().iter().find(|d| uri_of(f, &d.label) == uri)?.label.clone();
            Some((lib, label))
        };
        let (lib, label) = file
            .filter(|f| f.exists())
            .and_then(found)
            .or_else(|| files(paths, true).iter().find_map(|f| found(f)))
            .ok_or_else(|| Error::new("plugin_host.not_found").arg("uri", uri))?;
        let inst = Instance::new(&lib, &label, f64::from(rate), max_block)?;
        let params = inst.params().into_iter().map(|p| param_of(p, f64::from(rate))).collect();
        Ok(Ladspa { inst, params })
    }

    /// One piece of a block, `from..to`, with the channel lists cut there (on the stack: nothing is allocated)
    fn run(&mut self, from: usize, to: usize, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        if to <= from {
            return;
        }
        let mut iref: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        let mut oref: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| <&mut [f32]>::default());
        let (ni, no) = (inputs.len().min(MAX_CHANNELS), outputs.len().min(MAX_CHANNELS));
        for (r, b) in iref.iter_mut().zip(inputs) {
            *r = &b[from..to];
        }
        for (r, b) in oref.iter_mut().zip(outputs.iter_mut()) {
            *r = &mut b[from..to];
        }
        self.inst.process(to - from, &iref[..ni], &mut oref[..no]);
    }
}

impl Plugin for Ladspa {
    fn params(&self) -> &[Param] {
        &self.params
    }

    fn io(&self) -> Option<(usize, usize)> {
        Some((self.inst.audio_inputs(), self.inst.audio_outputs()))
    }

    fn latency(&self) -> u32 {
        self.inst.latency()
    }

    fn param(&self, index: u32) -> f32 {
        self.inst.param(index as usize).unwrap_or(0.0)
    }

    fn set_param(&mut self, index: u32, value: f32) {
        self.inst.set_param(index as usize, value);
    }

    fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[(u32, EventKind)],
        _transport: &Transport,
    ) {
        let mut at = 0;
        for (offset, e) in events {
            if let EventKind::Param { index, value } = *e {
                let t = (*offset as usize).min(frames);
                if t > at {
                    self.run(at, t, inputs, outputs);
                    at = t;
                }
                self.set_param(index, value);
            }
        }
        self.run(at, frames, inputs, outputs);
    }

    /// Nothing beyond the parameters, which the core keeps
    fn save(&mut self) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    fn restore(&mut self, _chunk: &[u8]) -> Result<()> {
        Ok(())
    }
}
