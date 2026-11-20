//! LV2 through `lm-sys-lilv`: the listing from the Turtle data (no plugin code), the instance, its control ports as
//! parameters (in port order), MIDI in through its atom port, the latency port, the state as lilv's chunk
use std::ffi::OsStr;
use std::path::PathBuf;

use lm_base::{Error, Result};
use lm_render::EventKind;
use lm_sys_lilv::{Direction, Instance, InstanceConfig, PluginInfo, PortKind, World};

use crate::backend::{Listed, Param, Plugin};

/// A world with the given folders (a bundle `*.lv2`, or a folder of bundles), or the system's LV2 path
pub fn world(paths: &[PathBuf]) -> Result<World> {
    let world = World::new()?;
    if paths.is_empty() {
        world.load_all();
    }
    let mut folders = Vec::new();
    for p in paths {
        if p.extension().is_some_and(|e| e == "lv2") {
            world.load_bundle(p)?;
        } else {
            folders.push(p.display().to_string());
        }
    }
    if !folders.is_empty() {
        world.load_path(OsStr::new(&folders.join(":")))?;
    }
    Ok(world)
}

fn params_of(info: &PluginInfo, rate: f64) -> Vec<(u32, Param)> {
    info.ports
        .iter()
        .filter(|p| p.kind == PortKind::Control && p.direction == Direction::Input)
        .map(|p| {
            let scale = |x: Option<f32>| x.map(|v| if p.sample_rate { f64::from(v) * rate } else { f64::from(v) });
            let min = scale(p.minimum).unwrap_or(0.0);
            let max = scale(p.maximum).unwrap_or(1.0).max(min);
            (
                p.index,
                Param {
                    key: p.symbol.clone(),
                    name: p.name.clone(),
                    min,
                    max,
                    default: f64::from(p.initial_value(rate)).clamp(min, max),
                    integer: p.integer || p.enumeration,
                    toggled: p.toggled,
                    logarithmic: p.logarithmic && min > 0.0,
                },
            )
        })
        .collect()
}

fn listed(info: &PluginInfo) -> Listed {
    let count = |k: PortKind, d: Direction| info.ports.iter().filter(|p| p.kind == k && p.direction == d).count();
    let midi_in = info.ports.iter().any(|p| p.kind == PortKind::Atom && p.direction == Direction::Input && p.midi);
    let inputs = count(PortKind::Audio, Direction::Input);
    Listed {
        format: "lv2".into(),
        uri: info.uri.clone(),
        name: info.name.clone(),
        instrument: info.class_uri.ends_with("InstrumentPlugin") || (midi_in && inputs == 0),
        inputs,
        outputs: count(PortKind::Audio, Direction::Output),
        midi_in,
        params: params_of(info, 48_000.0).into_iter().map(|(_, p)| p).collect(),
        latency: info.latency_port.is_some(),
        has_window: false,
    }
}

pub fn list(paths: &[PathBuf]) -> Result<Vec<Listed>> {
    Ok(world(paths)?.plugins().iter().map(listed).collect())
}

pub struct Lv2 {
    _world: World,
    instance: Instance,
    ports: Vec<u32>,
    params: Vec<Param>,
    midi: Option<u32>,
}

impl Lv2 {
    pub fn open(paths: &[PathBuf], uri: &str, rate: u32, max_block: usize) -> Result<Lv2> {
        let world = world(paths)?;
        let info = world.plugin(uri).ok_or_else(|| Error::new("plugin_host.not_found").arg("uri", uri))?;
        let mut config = InstanceConfig::new(f64::from(rate), u32::try_from(max_block).unwrap_or(u32::MAX));
        config.live = true;
        let mut instance = world.instantiate(uri, &config)?;
        let (ports, params): (Vec<u32>, Vec<Param>) = params_of(&info, f64::from(rate)).into_iter().unzip();
        for (port, p) in ports.iter().zip(&params) {
            instance.set_control(*port, p.default as f32)?;
        }
        let midi = info
            .ports
            .iter()
            .find(|p| p.kind == PortKind::Atom && p.direction == Direction::Input && p.midi)
            .map(|p| p.index);
        instance.activate();
        Ok(Lv2 { _world: world, instance, ports, params, midi })
    }
}

fn clamp7(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 127.0).round() as u8
}

impl Plugin for Lv2 {
    fn params(&self) -> &[Param] {
        &self.params
    }

    fn latency(&self) -> u32 {
        self.instance.latency().unwrap_or(0)
    }

    fn param(&self, index: u32) -> f32 {
        self.ports.get(index as usize).and_then(|p| self.instance.control(*p)).unwrap_or(0.0)
    }

    fn set_param(&mut self, index: u32, value: f32) {
        if let Some(port) = self.ports.get(index as usize) {
            let _ = self.instance.set_control(*port, value);
        }
    }

    fn process(&mut self, frames: usize, inputs: &[&[f32]], outputs: &mut [&mut [f32]], events: &[(u32, EventKind)]) {
        for (offset, e) in events {
            let midi = |b: [u8; 3]| Some(b);
            let msg = match *e {
                EventKind::NoteOn { channel, key, velocity, .. } => {
                    midi([0x90 | (channel & 15), key & 127, clamp7(velocity).max(1)])
                }
                EventKind::NoteOff { channel, key, velocity, .. } => {
                    midi([0x80 | (channel & 15), key & 127, clamp7(velocity)])
                }
                EventKind::Midi { bytes } => midi(bytes),
                EventKind::AllNotesRelease => midi([0xb0, 123, 0]),
                EventKind::AllSoundOff => midi([0xb0, 120, 0]),
                EventKind::Param { index, value } => {
                    self.set_param(index, value);
                    None
                }
                // LV2 has no per-note expression: pan and tuning reach the plugins that have them (CLAP)
                EventKind::NoteExpression { .. } => None,
            };
            if let (Some(m), Some(port)) = (msg, self.midi) {
                let len = match m[0] & 0xf0 {
                    0xc0 | 0xd0 => 2,
                    _ => 3,
                };
                let _ = self.instance.push_midi(port, *offset, &m[..len]);
            }
        }
        if self.instance.run(frames, inputs, outputs).is_err() {
            for o in outputs.iter_mut() {
                o[..frames].fill(0.0);
            }
        }
    }

    fn save(&mut self) -> Result<Option<Vec<u8>>> {
        let mut chunk = self.instance.save_state()?.unwrap_or_default();
        // The control values go with it, ahead of the chunk: `n`, then `n` × (port u32, value f32), then the chunk
        let mut out = Vec::with_capacity(4 + self.ports.len() * 8 + chunk.len());
        out.extend_from_slice(&(self.ports.len() as u32).to_le_bytes());
        for port in &self.ports {
            out.extend_from_slice(&port.to_le_bytes());
            out.extend_from_slice(&self.instance.control(*port).unwrap_or(0.0).to_le_bytes());
        }
        out.append(&mut chunk);
        Ok(Some(out))
    }

    fn restore(&mut self, chunk: &[u8]) -> Result<()> {
        let bad = || Error::new("plugin_host.state");
        let n = u32::from_le_bytes(chunk.get(..4).ok_or_else(bad)?.try_into().map_err(|_| bad())?) as usize;
        let mut at = 4;
        for _ in 0..n {
            let p = chunk.get(at..at + 8).ok_or_else(bad)?;
            let port = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            let value = f32::from_le_bytes([p[4], p[5], p[6], p[7]]);
            if self.ports.contains(&port) {
                self.instance.set_control(port, value)?;
            }
            at += 8;
        }
        let rest = &chunk[at..];
        if !rest.is_empty() {
            self.instance.restore_state(rest)?;
        }
        Ok(())
    }
}
