//! The helper's own test plugins (format `"test"`), for the tests of the out-of-process path without any plugin
//! library: `gain` (an effect with a latency it reports and delays by), `synth` (an instrument: a level while a note
//! is held), `crash` (aborts the helper when its `crash` parameter is on) and `hang` (never returns from a block when
//! its `hang` parameter is on); and, for the scan, `bomb` (aborts as it is made) and `stall` (never finishes being made)

use lm_base::{Error, Result};
use lm_render::EventKind;

use crate::backend::{Listed, Param, Plugin, Transport};

fn param(key: &str, min: f64, max: f64, default: f64, integer: bool) -> Param {
    Param { key: key.into(), name: key.into(), min, max, default, integer, toggled: false, logarithmic: false }
}

fn listed(uri: &str, instrument: bool, params: Vec<Param>) -> Listed {
    Listed {
        format: "test".into(),
        uri: uri.into(),
        name: format!("Test {uri}"),
        instrument,
        inputs: if instrument {
            0
        } else if uri == "mono" {
            1
        } else {
            2
        },
        outputs: if uri == "mono" { 1 } else { 2 },
        midi_in: instrument,
        params,
        latency: uri == "gain",
        has_window: false,
        file: None,
    }
}

pub fn list() -> Vec<Listed> {
    vec![
        listed("gain", false, vec![param("gain", 0.0, 4.0, 1.0, false), param("latency", 0.0, 4096.0, 0.0, true)]),
        listed("synth", true, vec![param("level", 0.0, 1.0, 0.5, false)]),
        listed("crash", false, vec![param("crash", 0.0, 1.0, 0.0, true)]),
        listed("hang", false, vec![param("hang", 0.0, 1.0, 0.0, true)]),
        // For the scan: one that crashes as it is made, one that never finishes being made
        listed("bomb", false, Vec::new()),
        listed("stall", false, Vec::new()),
        // A mono effect: on a stereo channel the helper runs it as two instances (dual mono)
        listed("mono", false, vec![param("gain", 0.0, 4.0, 1.0, false)]),
    ]
}

pub struct Test {
    kind: String,
    params: Vec<Param>,
    values: Vec<f32>,
    /// gain: the delay line; synth: notes held
    line: Vec<Vec<f32>>,
    pos: usize,
    held: u32,
    /// synth: a note arrived (its "state" is the last velocity-free note count, a test of `StateChanged`)
    dirty: bool,
}

impl Test {
    pub fn open(uri: &str, max_block: usize) -> Result<Test> {
        let l = list()
            .into_iter()
            .find(|l| l.uri == uri)
            .ok_or_else(|| Error::new("plugin_host.not_found").arg("uri", uri))?;
        match uri {
            "bomb" => std::process::abort(),
            "stall" => loop {
                std::thread::park();
            },
            _ => {}
        }
        let values = l.params.iter().map(|p| p.default as f32).collect();
        let _ = max_block;
        Ok(Test {
            kind: uri.into(),
            params: l.params,
            values,
            line: vec![vec![0.0; 4096 + 1]; 2],
            pos: 0,
            held: 0,
            dirty: false,
        })
    }
}

impl Plugin for Test {
    fn params(&self) -> &[Param] {
        &self.params
    }

    fn io(&self) -> Option<(usize, usize)> {
        (self.kind == "mono").then_some((1, 1))
    }

    fn latency(&self) -> u32 {
        if self.kind == "gain" { self.values[1] as u32 } else { 0 }
    }

    fn param(&self, index: u32) -> f32 {
        self.values.get(index as usize).copied().unwrap_or(0.0)
    }

    fn set_param(&mut self, index: u32, value: f32) {
        if let Some(v) = self.values.get_mut(index as usize) {
            *v = value;
        }
    }

    fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[(u32, EventKind)],
        _transport: &Transport,
    ) {
        for (_, e) in events {
            match *e {
                EventKind::Param { index, value } => self.set_param(index, value),
                EventKind::NoteOn { .. } => {
                    self.held += 1;
                    self.dirty = self.kind == "synth";
                }
                EventKind::NoteOff { .. } => self.held = self.held.saturating_sub(1),
                EventKind::AllNotesRelease | EventKind::AllSoundOff => self.held = 0,
                _ => {}
            }
        }
        match self.kind.as_str() {
            "crash" if self.values[0] >= 0.5 => std::process::abort(),
            "hang" if self.values[0] >= 0.5 => loop {
                std::thread::park();
            },
            "gain" => {
                let g = self.values[0];
                let delay = (self.values[1] as usize).min(4096);
                let len = self.line[0].len();
                for (ch, out) in outputs.iter_mut().enumerate() {
                    let input = inputs.get(ch).copied().unwrap_or(&[]);
                    let line = &mut self.line[ch.min(1)];
                    for (k, o) in out[..frames].iter_mut().enumerate() {
                        let x = input.get(k).copied().unwrap_or(0.0);
                        let w = (self.pos + k) % len;
                        line[w] = x;
                        let r = (self.pos + k + len - delay) % len;
                        *o = line[r] * g;
                    }
                }
                self.pos = (self.pos + frames) % len;
            }
            // One channel only: the first; a plugin run once on a stereo block leaves the second silent
            "mono" => {
                let g = self.values[0];
                for (ch, out) in outputs.iter_mut().enumerate() {
                    match (ch, inputs.first()) {
                        (0, Some(i)) => {
                            for (o, x) in out[..frames].iter_mut().zip(i.iter()) {
                                *o = x * g;
                            }
                        }
                        _ => out[..frames].fill(0.0),
                    }
                }
            }
            "synth" => {
                let v = if self.held > 0 { self.values[0] } else { 0.0 };
                for out in outputs.iter_mut() {
                    out[..frames].fill(v);
                }
            }
            _ => {
                for (ch, out) in outputs.iter_mut().enumerate() {
                    match inputs.get(ch) {
                        Some(i) => out[..frames].copy_from_slice(&i[..frames]),
                        None => out[..frames].fill(0.0),
                    }
                }
            }
        }
    }

    fn state_changed(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn save(&mut self) -> Result<Option<Vec<u8>>> {
        Ok(Some(self.values.iter().flat_map(|v| v.to_le_bytes()).collect()))
    }

    fn restore(&mut self, chunk: &[u8]) -> Result<()> {
        let (words, _) = chunk.as_chunks::<4>();
        if words.len() != self.values.len() {
            return Err(Error::new("plugin_host.state"));
        }
        for ((v, b), p) in self.values.iter_mut().zip(words).zip(&self.params) {
            let x = f32::from_le_bytes(*b);
            if !x.is_finite() {
                return Err(Error::new("plugin_host.state"));
            }
            *v = x.clamp(p.min as f32, p.max as f32);
        }
        Ok(())
    }
}
