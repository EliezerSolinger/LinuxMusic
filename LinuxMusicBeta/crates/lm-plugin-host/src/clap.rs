//! CLAP plugins in the helper, through `lm-sys-clap`: the files (found without running plugin code), the listing of
//! one file (it runs the plugin's code: the scan does it in a helper of its own per file), and an instance behind
//! [`Plugin`].
//!
//! - A parameter's key is `p<id>`: CLAP's parameter ids are stable across versions of a plugin. Hidden parameters
//!   are left out.
//! - The plugin is identified by its CLAP id (`uri`); the file it was found in is tried first, and when it moved the
//!   search paths are looked through.
//! - Notes go as CLAP note events with their note id; a note expression's ramp is sent as a value every
//!   [`RAMP_STEP`](crate::expressions::RAMP_STEP) frames until it ends; pan goes from −1..1 to CLAP's 0..1; "all notes release" is a wildcard
//!   note-off, "all sound off" a wildcard choke.

use std::path::{Path, PathBuf};

use lm_base::{Error, Result};
use lm_render::{EventKind, Expression};
use lm_sys_clap::{Event, Instance, Library, MAX_EVENTS};

use crate::backend::{Listed, Param, Plugin, Transport, WindowRequest, WindowState};
use crate::expressions::{Expressions, Note, sort_by_time};

/// The `.clap` files under the given folders and the system's (`CLAP_PATH`, the user's, the system's)
pub fn files(paths: &[PathBuf], system: bool) -> Vec<PathBuf> {
    let mut dirs = paths.to_vec();
    if system {
        dirs.extend(lm_sys_clap::search_paths());
    }
    lm_sys_clap::find_files(&dirs)
}

fn params_of(inst: &Instance) -> (Vec<u32>, Vec<Param>) {
    inst.params()
        .iter()
        .filter(|p| !p.hidden)
        .map(|p| {
            let toggled = p.stepped && p.min == 0.0 && p.max == 1.0;
            (
                p.id,
                Param {
                    key: format!("p{}", p.id),
                    name: p.name.clone(),
                    min: p.min,
                    max: p.max,
                    default: p.default,
                    integer: p.stepped,
                    toggled,
                    logarithmic: false,
                },
            )
        })
        .unzip()
}

fn channels(ports: &[lm_sys_clap::AudioPort]) -> usize {
    ports.iter().map(|p| p.channels as usize).sum()
}

/// The plugins of one file (or of every file found): each one is made once to read its parameters and ports
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
        for d in lib.descriptors() {
            let inst = match Instance::new(&lib, &d.id, 48_000.0, 256) {
                Ok(i) => i,
                Err(e) if file.is_some() => return Err(e),
                Err(_) => continue,
            };
            let (_, params) = params_of(&inst);
            let takes_notes = inst.takes_notes();
            out.push(Listed {
                format: "clap".into(),
                uri: d.id.clone(),
                name: d.name.clone(),
                instrument: d.is_instrument() || (takes_notes && inst.audio_inputs().is_empty()),
                inputs: channels(inst.audio_inputs()),
                outputs: channels(inst.audio_outputs()),
                midi_in: takes_notes,
                params,
                latency: inst.latency() > 0,
                has_window: inst.has_gui(),
                file: Some(f.display().to_string()),
            });
        }
    }
    Ok(out)
}

/// One CLAP plugin
pub struct Clap {
    inst: Instance,
    ids: Vec<u32>,
    params: Vec<Param>,
    events: Vec<Event>,
    notes: Expressions,
    gui: Gui,
    title: String,
}

/// The plugin's window as the helper holds it
enum Gui {
    None,
    /// The plugin made its own top-level window
    Floating,
    /// The plugin's gui is embedded in a window made here
    Embedded(lm_sys_gui::Window),
}

fn clap_id(note: u32) -> i32 {
    (note & 0x7fff_ffff) as i32
}

fn clap_value(expression: Expression, v: f64) -> (i32, f64) {
    match expression {
        Expression::Pan => (lm_sys_clap::CLAP_NOTE_EXPRESSION_PAN, ((v + 1.0) / 2.0).clamp(0.0, 1.0)),
        Expression::Tuning => (lm_sys_clap::CLAP_NOTE_EXPRESSION_TUNING, v.clamp(-120.0, 120.0)),
        Expression::Pressure => (lm_sys_clap::CLAP_NOTE_EXPRESSION_PRESSURE, v.clamp(0.0, 1.0)),
        Expression::Timbre => (lm_sys_clap::CLAP_NOTE_EXPRESSION_BRIGHTNESS, v.clamp(0.0, 1.0)),
    }
}

fn expression_event(time: u32, n: &Note, expression: Expression, value: f64) -> Event {
    let (id, v) = clap_value(expression, value);
    Event::Expression { time, expression: id, note_id: clap_id(n.id), channel: n.channel, key: n.key, value: v }
}

/// Room kept for the events that end a sound: when a block is full of the others, a note-off still fits (one lost
/// would leave a note hanging)
const RELEASE_RESERVE: usize = 256;

fn push(events: &mut Vec<Event>, e: Event) {
    let room = if matches!(e, Event::NoteOff { .. } | Event::Choke { .. }) {
        events.capacity()
    } else {
        events.capacity().saturating_sub(RELEASE_RESERVE)
    };
    if events.len() < room {
        events.push(e);
    }
}

impl Clap {
    pub fn open(paths: &[PathBuf], file: Option<&Path>, uri: &str, rate: u32, max_block: usize) -> Result<Clap> {
        let has = |lib: &Library| lib.descriptors().iter().any(|d| d.id == uri);
        let lib = file
            .filter(|f| f.exists())
            .and_then(|f| Library::open(f).ok())
            .filter(|l| has(l))
            .or_else(|| files(paths, true).iter().filter_map(|f| Library::open(f).ok()).find(|l| has(l)))
            .ok_or_else(|| Error::new("plugin_host.not_found").arg("uri", uri))?;
        let inst = Instance::new(&lib, uri, f64::from(rate), u32::try_from(max_block).unwrap_or(u32::MAX))?;
        let (ids, params) = params_of(&inst);
        let title = lib.descriptors().into_iter().find(|d| d.id == uri).map_or_else(|| uri.to_owned(), |d| d.name);
        Ok(Clap {
            inst,
            ids,
            params,
            events: Vec::with_capacity(MAX_EVENTS),
            notes: Expressions::new(),
            gui: Gui::None,
            title,
        })
    }

    /// Shows the plugin's window: its own floating one when it can make one with the window system, otherwise its gui
    /// embedded in a window made here
    fn show(&mut self) -> Result<WindowState> {
        if let Gui::Embedded(w) = &mut self.gui {
            w.show();
            return Ok(WindowState::open(None));
        }
        if matches!(self.gui, Gui::Floating) {
            return Ok(WindowState::open(None));
        }
        if !self.inst.has_gui() {
            return Ok(WindowState::none("clap.gui.none"));
        }
        if lm_sys_gui::init().is_err() {
            return Ok(WindowState::none("gui.unsupported"));
        }
        let api = lm_sys_gui::api();
        if self.inst.gui_supported(api, true) {
            self.inst.gui_open(api, &self.title)?;
            self.inst.take_gui_closed();
            self.gui = Gui::Floating;
            return Ok(WindowState::open(self.inst.gui_size()));
        }
        if self.inst.gui_supported(api, false) {
            let mut window = lm_sys_gui::Window::new(&self.title, 640, 480)?;
            let size = match self.inst.gui_open_embedded(api, window.handle()) {
                Ok(size) => size,
                Err(e) => return Ok(WindowState::none(e.key())),
            };
            window.set_size(size.0, size.1);
            window.show();
            self.gui = Gui::Embedded(window);
            return Ok(WindowState::open(Some(size)));
        }
        Ok(WindowState::none("clap.gui.api"))
    }

    fn close(&mut self) {
        if !matches!(self.gui, Gui::None) {
            self.inst.gui_close();
            self.inst.take_gui_closed();
        }
        self.gui = Gui::None;
    }
}

impl Drop for Clap {
    fn drop(&mut self) {
        // The window goes before the plugin does
        self.close();
    }
}

impl Plugin for Clap {
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
                    notes.note_on(note.0, i16::from(channel), i16::from(key));
                    let (channel, key, velocity) = (i16::from(channel), i16::from(key), f64::from(velocity));
                    push(out, Event::NoteOn { time, note_id: clap_id(note.0), channel, key, velocity });
                }
                EventKind::NoteOff { note, channel, key, velocity } => {
                    notes.note_off(note.0);
                    let (channel, key, velocity) = (i16::from(channel), i16::from(key), f64::from(velocity));
                    push(out, Event::NoteOff { time, note_id: clap_id(note.0), channel, key, velocity });
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
                EventKind::AllNotesRelease => {
                    notes.clear();
                    push(out, Event::NoteOff { time, note_id: -1, channel: -1, key: -1, velocity: 0.0 });
                }
                EventKind::AllSoundOff => {
                    notes.clear();
                    push(out, Event::Choke { time, note_id: -1, channel: -1, key: -1 });
                }
            }
        }
        notes.run(frames as u32, &mut |t, n, x, v| push(out, expression_event(t, n, x, v)));
        // In time order, stable (the ramps' values were added after the block's own events)
        sort_by_time(&mut self.events, time_of);
        let t = lm_sys_clap::Transport {
            playing: transport.playing,
            recording: transport.recording,
            beats: transport.beats(),
            seconds: transport.seconds(),
            tempo: transport.bpm,
            signature: (transport.signature.0 > 0 && transport.signature.1 > 0)
                .then(|| (u16::from(transport.signature.0), u16::from(transport.signature.1))),
        };
        self.inst.process(frames, inputs, outputs, &self.events, Some(&t));
    }

    fn save(&mut self) -> Result<Option<Vec<u8>>> {
        self.inst.save_state()
    }

    fn state_saver(&self) -> Option<crate::backend::StateSaver> {
        let saver = self.inst.state_saver()?;
        Some(Box::new(move || saver.save()))
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
                if matches!(self.gui, Gui::None) {
                    return Ok(WindowState::closed());
                }
                if let Gui::Embedded(window) = &mut self.gui {
                    window.set_size(w, h);
                }
                let size = self.inst.gui_resize(w, h);
                if let (Gui::Embedded(window), Some((w, h))) = (&mut self.gui, size) {
                    window.set_size(w, h);
                }
                Ok(WindowState::open(size))
            }
        }
    }

    fn window_closed(&mut self) -> bool {
        let closed = match &mut self.gui {
            Gui::None => false,
            Gui::Floating => self.inst.take_gui_closed(),
            Gui::Embedded(w) => w.take_closed(),
        };
        if closed {
            self.close();
        }
        closed
    }

    fn idle(&mut self) {
        self.inst.idle();
        // An embedded gui asked for another size: the window follows
        if let Some((w, h)) = self.inst.take_gui_resize()
            && let Gui::Embedded(window) = &mut self.gui
        {
            window.set_size(w, h);
            self.inst.gui_resize(w, h);
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

fn time_of(e: &Event) -> u32 {
    match *e {
        Event::NoteOn { time, .. }
        | Event::NoteOff { time, .. }
        | Event::Choke { time, .. }
        | Event::Expression { time, .. }
        | Event::Param { time, .. }
        | Event::Midi { time, .. } => time,
    }
}
