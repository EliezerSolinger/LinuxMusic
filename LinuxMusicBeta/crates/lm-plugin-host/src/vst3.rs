//! VST3 plugins in the helper, through `lm-sys-vst3`: the `.vst3` files (found without running plugin code), the
//! listing of one file (it runs the plugin's code: the scan does it in a helper of its own per file), and an instance
//! behind [`Plugin`].
//!
//! - The plugin is identified by its class id (32 hex digits, the `uri`); the file it was found in is tried first.
//! - A parameter's key is `p<id>` (VST3 parameter ids are stable) and its value is VST3's normalized 0..1: the
//!   plain value is the controller's business, and asking it would mean a main-thread call from the audio side.
//!   Hidden parameters are left out.
//! - Note expressions: pan from −1..1 to 0..1, tuning to 0.5 ± semitones/240, ramps as values every
//!   [`RAMP_STEP`](crate::expressions::RAMP_STEP) frames. "All sound off" releases every held note (VST3 has no
//!   choke). Controllers, pitch bend and pressure reach the plugin through its own MIDI mapping.

use std::path::{Path, PathBuf};

use lm_base::{Error, Result};
use lm_render::{EventKind, Expression};
use lm_sys_vst3::{Event, Instance, MAX_EVENTS, Module};

use crate::backend::{Listed, Param, Plugin, Transport, WindowRequest, WindowState};
use crate::expressions::{Expressions, Note, sort_by_time};

/// The `.vst3` files under the given folders and the system's
pub fn files(paths: &[PathBuf], system: bool) -> Vec<PathBuf> {
    let mut dirs = paths.to_vec();
    if system {
        dirs.extend(lm_sys_vst3::search_paths());
    }
    lm_sys_vst3::find_files(&dirs)
}

fn params_of(inst: &Instance) -> (Vec<u32>, Vec<Param>) {
    inst.params()
        .iter()
        .filter(|p| !p.hidden)
        .map(|p| {
            (
                p.id,
                Param {
                    key: format!("p{}", p.id),
                    name: p.title.clone(),
                    min: 0.0,
                    max: 1.0,
                    default: p.default.clamp(0.0, 1.0),
                    integer: false,
                    toggled: p.steps == 1,
                    logarithmic: false,
                },
            )
        })
        .unzip()
}

fn channels(buses: &[lm_sys_vst3::Bus]) -> usize {
    buses.iter().map(|b| b.channels as usize).sum()
}

/// The plugins of one file (or of every file found): each one is made once to read its parameters and buses
pub fn list(paths: &[PathBuf], file: Option<&Path>, system: bool) -> Result<Vec<Listed>> {
    let all = match file {
        Some(f) => vec![f.to_path_buf()],
        None => files(paths, system),
    };
    let mut out = Vec::new();
    for f in all {
        let module = match Module::open(&f) {
            Ok(m) => m,
            Err(e) if file.is_some() => return Err(e),
            Err(_) => continue,
        };
        for c in module.classes() {
            let inst = match Instance::new(&module, &c.cid, 48_000.0, 256) {
                Ok(i) => i,
                Err(e) if file.is_some() => return Err(e),
                Err(_) => continue,
            };
            let (_, params) = params_of(&inst);
            out.push(Listed {
                format: "vst3".into(),
                uri: c.cid.clone(),
                name: c.name.clone(),
                instrument: c.is_instrument() || (inst.takes_notes() && inst.audio_inputs().is_empty()),
                inputs: channels(inst.audio_inputs()),
                outputs: channels(inst.audio_outputs()),
                midi_in: inst.takes_notes(),
                params,
                latency: inst.latency() > 0,
                has_window: inst.has_editor(),
                file: Some(f.display().to_string()),
            });
        }
    }
    Ok(out)
}

/// One VST3 plugin
pub struct Vst3 {
    inst: Instance,
    ids: Vec<u32>,
    params: Vec<Param>,
    events: Vec<Event>,
    notes: Expressions,
    /// The native window the editor view is attached to, while it is shown
    window: Option<lm_sys_gui::Window>,
    title: String,
}

fn note_id(note: u32) -> i32 {
    (note & 0x7fff_ffff) as i32
}

fn expression_event(time: u32, n: &Note, expression: Expression, value: f64) -> Event {
    let (kind, v) = match expression {
        Expression::Pan => (lm_sys_vst3::Expression::Pan, ((value + 1.0) / 2.0).clamp(0.0, 1.0)),
        Expression::Tuning => (lm_sys_vst3::Expression::Tuning, (0.5 + value / 240.0).clamp(0.0, 1.0)),
        Expression::Pressure => (lm_sys_vst3::Expression::Pressure, value.clamp(0.0, 1.0)),
        Expression::Timbre => (lm_sys_vst3::Expression::Brightness, value.clamp(0.0, 1.0)),
    };
    Event::Expression { time, kind, note_id: note_id(n.id), value: v }
}

fn push(events: &mut Vec<Event>, e: Event) {
    if events.len() < events.capacity() {
        events.push(e);
    }
}

fn time_of(e: &Event) -> u32 {
    match *e {
        Event::NoteOn { time, .. }
        | Event::NoteOff { time, .. }
        | Event::ReleaseAll { time }
        | Event::Expression { time, .. }
        | Event::Param { time, .. }
        | Event::Midi { time, .. } => time,
    }
}

impl Vst3 {
    pub fn open(paths: &[PathBuf], file: Option<&Path>, uri: &str, rate: u32, max_block: usize) -> Result<Vst3> {
        let has = |m: &Module| m.classes().iter().any(|c| c.cid == uri);
        let module = file
            .filter(|f| f.exists())
            .and_then(|f| Module::open(f).ok())
            .filter(|m| has(m))
            .or_else(|| files(paths, true).iter().filter_map(|f| Module::open(f).ok()).find(|m| has(m)))
            .ok_or_else(|| Error::new("plugin_host.not_found").arg("uri", uri))?;
        let inst = Instance::new(&module, uri, f64::from(rate), u32::try_from(max_block).unwrap_or(u32::MAX))?;
        let (ids, params) = params_of(&inst);
        let title = module.classes().into_iter().find(|c| c.cid == uri).map_or_else(|| uri.to_owned(), |c| c.name);
        Ok(Vst3 {
            inst,
            ids,
            params,
            events: Vec::with_capacity(MAX_EVENTS),
            notes: Expressions::new(),
            window: None,
            title,
        })
    }

    /// Shows the editor: a native window made here, the view attached to it
    fn show(&mut self) -> Result<WindowState> {
        if let Some(w) = &mut self.window {
            w.show();
            return Ok(WindowState::open(None));
        }
        if !self.inst.has_editor() {
            return Ok(WindowState::none("vst3.editor.none"));
        }
        if lm_sys_gui::init().is_err() {
            return Ok(WindowState::none("gui.unsupported"));
        }
        let mut window = lm_sys_gui::Window::new(&self.title, 640, 480)?;
        let size = match self.inst.editor_open(window.handle(), lm_sys_gui::api()) {
            Ok(size) => size,
            Err(e) => return Ok(WindowState::none(e.key())),
        };
        window.set_size(size.0, size.1);
        window.show();
        self.window = Some(window);
        Ok(WindowState::open(Some(size)))
    }

    fn close(&mut self) {
        self.inst.editor_close();
        self.window = None;
    }
}

impl Plugin for Vst3 {
    fn params(&self) -> &[Param] {
        &self.params
    }

    fn io(&self) -> Option<(usize, usize)> {
        Some((channels(self.inst.audio_inputs()), channels(self.inst.audio_outputs())))
    }

    fn latency(&self) -> u32 {
        self.inst.latency()
    }

    fn param(&self, index: u32) -> f32 {
        self.ids.get(index as usize).and_then(|id| self.inst.param_value(*id)).unwrap_or(0.0) as f32
    }

    fn set_param(&mut self, index: u32, value: f32) {
        if let Some(id) = self.ids.get(index as usize) {
            self.inst.set_param(*id, f64::from(value));
        }
    }

    fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[(u32, EventKind)],
        transport: &Transport,
    ) {
        self.events.clear();
        let (notes, out) = (&mut self.notes, &mut self.events);
        for (time, e) in events {
            let time = *time;
            match *e {
                EventKind::NoteOn { note, channel, key, velocity } => {
                    let (channel, key) = (i16::from(channel), i16::from(key));
                    notes.note_on(note.0, channel, key);
                    push(out, Event::NoteOn { time, note_id: note_id(note.0), channel, key, velocity });
                }
                EventKind::NoteOff { note, channel, key, velocity } => {
                    notes.note_off(note.0);
                    let (channel, key) = (i16::from(channel), i16::from(key));
                    push(out, Event::NoteOff { time, note_id: note_id(note.0), channel, key, velocity });
                }
                EventKind::Param { index, value } => {
                    if let Some(id) = self.ids.get(index as usize) {
                        push(out, Event::Param { time, id: *id, value: f64::from(value) });
                    }
                }
                EventKind::Midi { bytes } => push(out, Event::Midi { time, data: bytes }),
                EventKind::NoteExpression { note, expression, value, ramp } => {
                    notes.set(time, note.0, expression, f64::from(value), ramp, &mut |t, n, x, v| {
                        push(out, expression_event(t, n, x, v))
                    });
                }
                EventKind::AllNotesRelease | EventKind::AllSoundOff => {
                    notes.clear();
                    push(out, Event::ReleaseAll { time });
                }
            }
        }
        notes.run(frames as u32, &mut |t, n, x, v| push(out, expression_event(t, n, x, v)));
        sort_by_time(&mut self.events, time_of);
        let t = lm_sys_vst3::Transport {
            playing: transport.playing,
            recording: transport.recording,
            samples: transport.position,
            beats: transport.beats(),
            tempo: transport.bpm,
            signature: (transport.signature.0 > 0 && transport.signature.1 > 0)
                .then(|| (i32::from(transport.signature.0), i32::from(transport.signature.1))),
        };
        self.inst.process(frames, inputs, outputs, &self.events, Some(&t));
    }

    fn save(&mut self) -> Result<Option<Vec<u8>>> {
        self.inst.save_state().map(Some)
    }

    fn restore(&mut self, chunk: &[u8]) -> Result<()> {
        self.inst.load_state(chunk)
    }

    fn window(&mut self, request: WindowRequest) -> Result<WindowState> {
        match request {
            WindowRequest::Show => self.show(),
            WindowRequest::Hide => {
                self.close();
                Ok(WindowState::closed())
            }
            WindowRequest::Resize(w, h) => {
                let Some(window) = &mut self.window else { return Ok(WindowState::closed()) };
                window.set_size(w, h);
                let size = self.inst.editor_resize(w, h);
                if let Some((w, h)) = size {
                    window.set_size(w, h);
                }
                Ok(WindowState::open(size))
            }
        }
    }

    fn window_closed(&mut self) -> bool {
        let closed = self.window.as_mut().is_some_and(lm_sys_gui::Window::take_closed);
        if closed {
            self.close();
        }
        closed
    }

    fn idle(&mut self) {
        self.inst.idle();
        if self.window.is_some() {
            self.inst.editor_tick();
            // The view asked for another size: the window follows, and the view is told it was given
            if let Some((w, h)) = self.inst.take_editor_resize()
                && let Some(window) = &mut self.window
            {
                window.set_size(w, h);
                self.inst.editor_resize(w, h);
            }
        }
    }

    fn gestures(&mut self, out: &mut Vec<(u32, bool)>) {
        for (id, begin) in self.inst.gestures() {
            if let Some(i) = self.ids.iter().position(|x| x == id)
                && out.len() < out.capacity()
            {
                out.push((i as u32, *begin));
            }
        }
    }

    fn state_changed(&mut self) -> bool {
        self.inst.take_state_dirty()
    }

    fn changes(&mut self, out: &mut Vec<(u32, f32)>) {
        for (id, v) in self.inst.changes() {
            if let Some(i) = self.ids.iter().position(|x| x == id)
                && out.len() < out.capacity()
            {
                out.push((i as u32, *v as f32));
            }
        }
    }
}
