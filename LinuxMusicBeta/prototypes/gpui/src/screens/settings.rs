//! Owner: agent D. The settings, as approved design 08 says: sections in a list on the left with a search over every
//! label and hint (without accents, F-CFG-001); every option says, before it is changed, whether it applies at once
//! or asks for a restart; what applies at once is applied (a device setting reopens the device by itself, 0.4 s after
//! the last change, as an operation with its loading); what needs a restart shows the band "Restart now" (F-CFG-002).
//!
//! Pages: General (F-CFG-010), Audio (F-CFG-011, U7 explicit input and output, drift correction between cards,
//! latency calibration with progress and the residual per device/rate/edge converter H19 I-10.4, manual offset
//! F-REC-082, real-time permission on Linux H1), MIDI (F-CFG-012, ports of this machine P-36, ALSA seq/raw P-16,
//! latency per port P-15), Recording (F-CFG-013, P-11, P-12), Plugins (F-CFG-014), Folders (F-CFG-015, the folders a
//! project may read C9), Appearance (F-CFG-016, theme as data P-08, language), Keyboard (F-CFG-017: every action of
//! the registry), Autosave and backup (F-PRJ-010, F-PRJ-016, spec item 5), Server (I-15), Advanced (F-CFG-018).
//!
//! Everything goes through the port (I-15): the settings are read from `port().services()` and changed by the
//! action "prefs.set" with `key=value`; the core knows which ones need a restart.
//!
//! The small form pieces here (`segmented`, `stepper`, `switch`, `radio_card`, `badge`, `note`) are also used by the
//! other screens of agent D.

use std::rc::Rc;
use std::time::Duration;

use fake_core::actions;
use fake_core::project::{
    self, Apply, AudioSystem, Detached, Follow, Link, MetronomeWhen, MidiBackend, MidiMode, RATES, Services, TakeFormat,
};
use fake_core::{Loading, Target, Value};
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, Render, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_component::switch::Switch;

use super::ScreenCx;
use super::notices::ago;
use super::palette::{expand_home, kbd, keys_label, keys_of, port_action, take_screen_arg};
use crate::ui::{THEME, button, c, caption, loading_component, tbutton, tr, trf};

// ---------------------------------------------------------------------------------------------------------------
// Form pieces (shared by agent D's screens)

/// A row of choices, one on; `enabled == false` draws it dimmed and without a click
pub fn segmented<V: 'static>(
    id: &'static str,
    items: Vec<(SharedString, bool, bool)>,
    cx: &Context<V>,
    pick: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
) -> Div {
    let pick = Rc::new(pick);
    div().flex().flex_wrap().gap(px(4.)).children(items.into_iter().enumerate().map(|(i, (label, on, enabled))| {
        let p = pick.clone();
        let b = tbutton((id, i), label, on, THEME.green).debug_selector(move || format!("{id}-{i}"));
        if enabled { b.on_click(cx.listener(move |v, _, w, cx| p(v, i, w, cx))) } else { b.opacity(0.4).cursor_default() }
    }))
}

/// − value +
pub fn stepper<V: 'static>(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
    cx: &Context<V>,
    step: impl Fn(&mut V, i32, &mut Context<V>) + 'static,
) -> Div {
    let id: SharedString = id.into();
    let step = Rc::new(step);
    let (a, b) = (step.clone(), step);
    let (i1, i2) = (id.clone(), id.clone());
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            tbutton(ElementId::Name(format!("{id}-down").into()), "−", false, 0)
                .debug_selector(move || format!("{i1}-down"))
                .on_click(cx.listener(move |v, _, _, cx| a(v, -1, cx))),
        )
        .child(div().min_w(px(70.)).flex().justify_center().text_size(px(13.)).child(text.into()))
        .child(
            tbutton(ElementId::Name(format!("{id}-up").into()), "+", false, 0)
                .debug_selector(move || format!("{i2}-up"))
                .on_click(cx.listener(move |v, _, _, cx| b(v, 1, cx))),
        )
}

/// An on/off switch (gpui-component's, in our theme)
pub fn switch<V: 'static>(id: impl Into<SharedString>, on: bool, cx: &Context<V>, set: impl Fn(&mut V, bool, &mut Context<V>) + 'static) -> Div {
    let id: SharedString = id.into();
    let sel = id.clone();
    div().flex().child(
        div()
            .debug_selector(move || sel.to_string())
            .child(Switch::new(ElementId::Name(id)).checked(on).on_click(cx.listener(move |v, on: &bool, _, cx| set(v, *on, cx)))),
    )
}

/// A choosable card in a list (devices, templates, recent projects): a dot, a title, a detail line
pub fn radio_card(id: impl Into<ElementId>, on: bool, title: impl Into<SharedString>, detail: impl Into<SharedString>) -> gpui::Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .py(px(4.))
        .rounded(px(6.))
        .border_1()
        .border_color(c(if on { THEME.green } else { THEME.line2 }))
        .bg(c(if on { THEME.bg4 } else { THEME.bg3 }))
        .cursor_pointer()
        .child(
            div()
                .flex_none()
                .size(px(12.))
                .rounded(px(6.))
                .border_2()
                .border_color(c(if on { THEME.green } else { THEME.text3 }))
                .when(on, |d| d.bg(c(THEME.green))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(px(13.)).child(title.into()))
                .child(div().text_size(px(11.)).text_color(c(THEME.text3)).child(detail.into())),
        )
}

/// A small colored word ("applies at once", "Bluetooth", "busy")
pub fn badge(text: impl Into<SharedString>, color: u32, filled: bool) -> Div {
    div()
        .flex_none()
        .px(px(5.))
        .rounded(px(3.))
        .border_1()
        .border_color(c(color))
        .when(filled, |d| d.bg(c(color)))
        .text_size(px(10.))
        .text_color(c(if filled { THEME.on_accent } else { color }))
        .child(text.into())
}

/// A line of warning or information inside a page
pub fn note(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex()
        .gap(px(8.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(c(color))
        .text_size(px(12.))
        .child(div().flex_none().w(px(3.)).rounded(px(2.)).bg(c(color)))
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// The loading of a target with the cancel of its operation (the shared component, I-2.3)
pub fn target_loading(scx: &ScreenCx, id: &'static str, target: Target, cx: &App) -> Option<AnyElement> {
    let port = scx.core.read(cx).port();
    let loading: Loading = port.loading(&target);
    let op = port.activity().into_iter().find(|a| a.target == target).map(|a| a.op);
    let core = scx.core.clone();
    loading_component(id, &loading, move |_, cx| {
        if let Some(op) = op {
            core.update(cx, |c, cx| {
                c.port_mut().cancel(op);
                cx.notify();
            });
        }
    })
}

/// A modal's size: what it wants, or the window less a margin when the window is smaller
pub fn modal_size(window: &Window, w: f32, h: f32) -> (f32, f32) {
    let v = window.viewport_size();
    (w.min(v.width.as_f32() - 40.0), h.min(v.height.as_f32() - 30.0))
}

pub fn khz(r: u32) -> String {
    let k = r as f64 / 1000.0;
    if (k - k.round()).abs() < 1e-9 { format!("{k:.0} kHz") } else { format!("{k:.1} kHz").replace('.', ",") }
}

fn apply_badge(a: Apply) -> Div {
    match a {
        Apply::Now => badge(tr("applies at once"), THEME.green, false),
        Apply::ReopensDevice => badge(tr("applies at once, reopens the device"), THEME.green, false),
        Apply::Restart => badge(tr("asks for a restart"), THEME.yellow, false),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The pages

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Audio,
    Midi,
    Recording,
    Plugins,
    Folders,
    Appearance,
    Keys,
    Backup,
    Server,
    Advanced,
}

impl Page {
    pub const ALL: [Page; 11] = [
        Page::General,
        Page::Audio,
        Page::Midi,
        Page::Recording,
        Page::Plugins,
        Page::Folders,
        Page::Appearance,
        Page::Keys,
        Page::Backup,
        Page::Server,
        Page::Advanced,
    ];
    fn title(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Audio => "Audio",
            Page::Midi => "MIDI",
            Page::Recording => "Recording and metronome",
            Page::Plugins => "Plugins",
            Page::Folders => "Folders",
            Page::Appearance => "Appearance",
            Page::Keys => "Keyboard and shortcuts",
            Page::Backup => "Autosave and backup",
            Page::Server => "Server",
            Page::Advanced => "Advanced",
        }
    }
    fn about(self) -> &'static str {
        match self {
            Page::General => "Projects and how the program behaves.",
            Page::Audio => "Everything here applies as it changes: the device opens again by itself.",
            Page::Midi => "The MIDI ports of this machine; they are not saved in the project.",
            Page::Recording => "Recording audio and MIDI, and the metronome.",
            Page::Plugins => "Plugin windows, and the plugins kept off after a crash.",
            Page::Folders => "Where the program keeps and looks for files, and the folders a project may read.",
            Page::Appearance => "The theme, the language and how the windows look and behave.",
            Page::Keys => "Every action of the program; click Change and press the new keys.",
            Page::Backup => "The journal keeps every change; autosave and copies are on top of it.",
            Page::Server => "The interface here, the core and the sound on another machine.",
            Page::Advanced => "Safety and performance.",
        }
    }
    fn from_arg(a: &str) -> Option<Page> {
        Some(match a {
            "audio" => Page::Audio,
            "midi" => Page::Midi,
            "keys" => Page::Keys,
            "folders" => Page::Folders,
            "server" => Page::Server,
            _ => return None,
        })
    }
}

/// One setting: its label, when it applies, a hint, and its control; `wide` puts the control under the label
struct Row {
    label: &'static str,
    apply: Option<Apply>,
    hint: Option<String>,
    control: AnyElement,
    wide: bool,
}

fn row(label: &'static str, apply: Apply, control: impl IntoElement) -> Row {
    Row { label, apply: Some(apply), hint: None, control: control.into_any_element(), wide: false }
}
fn wide(label: &'static str, apply: Option<Apply>, control: impl IntoElement) -> Row {
    Row { label, apply, hint: None, control: control.into_any_element(), wide: true }
}
impl Row {
    fn hint(mut self, h: impl Into<String>) -> Row {
        self.hint = Some(h.into());
        self
    }
}

/// A theme file found, with the colors its preview card shows
#[derive(Clone)]
struct ThemeFile {
    name: String,
    path: std::path::PathBuf,
    swatch: Vec<u32>,
}

fn themes() -> Vec<ThemeFile> {
    let dirs = [
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/themes")),
        std::path::PathBuf::from(expand_home("~/.config/linuxmusic/themes")),
    ];
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "txt")).collect();
        files.sort();
        for p in files {
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let get = |k: &str| {
                text.lines()
                    .filter_map(|l| l.split_once('='))
                    .find(|(a, _)| a.trim() == k)
                    .and_then(|(_, v)| u32::from_str_radix(v.trim().trim_start_matches('#').get(..6)?, 16).ok())
            };
            let swatch = ["window", "bg2", "bg4", "text", "green", "track1", "track3", "track5"].iter().filter_map(|k| get(k)).collect();
            let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            out.push(ThemeFile { name, path: p, swatch });
        }
    }
    out
}

/// The port's action for a value: `true`/`false`, numbers, text
fn b(v: bool) -> &'static str {
    if v { "true" } else { "false" }
}

pub struct Settings {
    scx: ScreenCx,
    page: Page,
    search: Entity<InputState>,
    block: Entity<SliderState>,
    new_folder: Entity<InputState>,
    new_plugin_dir: Entity<InputState>,
    projects_dir: Entity<InputState>,
    soundfont: Entity<InputState>,
    server: Entity<InputState>,
    token: Entity<InputState>,
    /// The shortcut being changed, and the keys of a conflict waiting for a second press
    capturing: Option<&'static str>,
    pending_keys: Option<String>,
    focus: FocusHandle,
    band_hidden: bool,
    themes: Vec<ThemeFile>,
    _reopen: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl Settings {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let arg = take_screen_arg(cx);
        let sv = scx.core.read(cx).port().services();
        let (prefs, first_server) = (sv.prefs.clone(), sv.servers.first().cloned().unwrap_or_default());
        let input = |v: &str, ph: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            let v = v.to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(v).placeholder(tr(ph)))
        };
        let search = input("", "Search settings…", window, cx);
        let new_folder = input("", "/a/folder/to/allow", window, cx);
        let new_plugin_dir = input("", "/a/plugin/folder", window, cx);
        let projects_dir = input(&prefs.projects_dir, "", window, cx);
        let soundfont = input(&prefs.default_soundfont, "", window, cx);
        let server = input(&first_server, "host:port or /path/to/socket", window, cx);
        let token = input("", "pairing token", window, cx);
        let block = cx.new(|_| SliderState::new().min(32.0).max(2048.0).step(32.0).default_value(prefs.block as f32));
        let subs = vec![
            cx.subscribe(&search, |_: &mut Settings, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&block, |s: &mut Settings, _, ev: &SliderEvent, cx| {
                if let SliderEvent::Change(SliderValue::Single(v)) = ev {
                    s.pref(cx, "audio.block", (*v as u32).max(32));
                    s.reopen_soon(cx);
                }
            }),
            // text settings apply on Enter or when the field loses focus
            cx.subscribe(&projects_dir, |s: &mut Settings, st, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = st.read(cx).value().to_string();
                    s.pref(cx, "folders.projects", v);
                }
            }),
            cx.subscribe(&soundfont, |s: &mut Settings, st, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = st.read(cx).value().to_string();
                    s.pref(cx, "folders.soundfont", v);
                }
            }),
            cx.subscribe(&new_folder, |s: &mut Settings, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    s.add_folder(cx);
                }
            }),
        ];
        Settings {
            scx,
            page: arg.and_then(Page::from_arg).unwrap_or(Page::Audio),
            search,
            block,
            new_folder,
            new_plugin_dir,
            projects_dir,
            soundfont,
            server,
            token,
            capturing: None,
            pending_keys: None,
            focus: cx.focus_handle(),
            band_hidden: false,
            themes: themes(),
            _reopen: None,
            _subs: subs,
        }
    }

    fn sv<'a>(&self, cx: &'a App) -> &'a Services {
        self.scx.core.read(cx).port().services()
    }

    /// One setting through the port ("prefs.set", `key=value`): it applies at once
    fn pref(&mut self, cx: &mut Context<Self>, key: &str, v: impl ToString) {
        port_action(&self.scx, "prefs.set", Value::Text(format!("{key}={}", v.to_string())), cx);
        cx.notify();
    }

    fn act(&mut self, cx: &mut Context<Self>, id: &str, params: Value) {
        port_action(&self.scx, id, params, cx);
        cx.notify();
    }

    /// A device setting changed: the device reopens 0.4 s after the last change (F-REC-120), as an operation
    fn reopen_soon(&mut self, cx: &mut Context<Self>) {
        let scx = self.scx.clone();
        self._reopen = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let _ = cx.update(|cx| port_action(&scx, "device.reopen", Value::Text(String::new()), cx));
        }));
    }

    fn add_folder(&mut self, cx: &mut Context<Self>) {
        let v = self.new_folder.read(cx).value().trim().to_string();
        if v.starts_with('/') || v.starts_with("~/") {
            self.pref(cx, "folders.allow", v);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.capturing else { return };
        cx.stop_propagation();
        if ev.keystroke.key == "escape" {
            self.capturing = None;
            self.pending_keys = None;
            cx.notify();
            return;
        }
        let Some(keys) = keys_of(ev) else { return };
        let conflict = actions::conflict(&self.sv(cx).prefs, &keys, action).map(|a| a.id);
        match conflict {
            // a second press of the same keys takes them from the other action
            Some(other) if self.pending_keys.as_deref() == Some(keys.as_str()) => {
                self.pref(cx, &format!("keys.{other}"), "");
                self.pref(cx, &format!("keys.{action}"), &keys);
                self.capturing = None;
                self.pending_keys = None;
            }
            Some(_) => self.pending_keys = Some(keys),
            None => {
                self.pref(cx, &format!("keys.{action}"), &keys);
                self.capturing = None;
                self.pending_keys = None;
            }
        }
        cx.notify();
    }

    /// Restart now (F-GER-013): saves the project, starts the program again with the new theme and language, and
    /// opens the settings again
    fn restart(&mut self, cx: &mut Context<Self>) {
        let p = &self.sv(cx).prefs;
        let (theme, lang) = (p.theme.clone(), p.language);
        let path = self.themes.iter().find(|t| t.name == theme).map(|t| t.path.clone());
        if !self.sv(cx).project.read_only() {
            self.act(cx, "file.save", Value::Text(String::new()));
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut cmd = std::process::Command::new(exe);
            cmd.env("LM_OPEN", "Settings").env("LM_LANG", lang);
            if let Some(p) = path {
                cmd.env("LM_THEME", p);
            }
            if cmd.spawn().is_ok() {
                cx.quit();
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Each page's rows

impl Settings {
    fn rows(&self, page: Page, cx: &mut Context<Self>) -> Vec<Row> {
        match page {
            Page::General => self.general(cx),
            Page::Audio => self.audio(cx),
            Page::Midi => self.midi(cx),
            Page::Recording => self.recording(cx),
            Page::Plugins => self.plugins(cx),
            Page::Folders => self.folders(cx),
            Page::Appearance => self.appearance(cx),
            Page::Keys => self.keys(cx),
            Page::Backup => self.backup(cx),
            Page::Server => self.server_page(cx),
            Page::Advanced => self.advanced(cx),
        }
    }

    fn general(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let p = self.sv(cx).prefs.clone();
        let project_rate = self.sv(cx).project.rate;
        vec![
            row("Reopen the last project on startup", Apply::Now, switch("reopen-last", p.reopen_last, cx, |s, v, cx| s.pref(cx, "general.reopen_last", b(v)))),
            row("Ask before deleting a track", Apply::Now, switch("ask-track", p.ask_delete_track, cx, |s, v, cx| s.pref(cx, "general.ask_track", b(v)))),
            row(
                "Ask before deleting a mixer channel in use",
                Apply::Now,
                switch("ask-channel", p.ask_delete_channel, cx, |s, v, cx| s.pref(cx, "general.ask_channel", b(v))),
            ),
            row(
                "Long audio files become audio tracks",
                Apply::Now,
                stepper("long-audio", trf("{0} s", &[&p.long_audio_seconds]), cx, move |s, d, cx| {
                    s.pref(cx, "general.long_audio", (p.long_audio_seconds as i32 + d * 5).max(1))
                }),
            )
            .hint(tr("Longer than this, an audio file dropped in the project is an audio track; shorter, a sample")),
            row("Previews play to the end", Apply::Now, switch("previews-end", p.previews_to_end, cx, |s, v, cx| s.pref(cx, "general.previews_to_end", b(v))))
                .hint(tr("Off: a sample previewed in the browser stops when the mouse button is released")),
            row("New projects use the analog desk", Apply::Now, switch("analog", p.analog_desk, cx, |s, v, cx| s.pref(cx, "general.analog_desk", b(v))))
                .hint(tr("Off: the clean digital desk, without modelling")),
            row(
                "Sample rate of new projects",
                Apply::Now,
                segmented(
                    "new-rate",
                    RATES.iter().map(|r| (khz(*r).into(), p.new_project_rate == *r, true)).collect(),
                    cx,
                    |s, i, _, cx| s.pref(cx, "general.new_rate", RATES[i]),
                ),
            )
            .hint(trf("Each project keeps its own rate; this one is at {0}", &[&khz(project_rate)])),
            row("Portable mode", Apply::Restart, switch("portable", p.portable, cx, |s, v, cx| s.pref(cx, "general.portable", b(v))))
                .hint(tr("The settings stay in a folder beside the program, to carry it on a USB stick")),
        ]
    }

    fn audio(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let port = self.scx.core.read(cx).port();
        let sv = port.services();
        let p = sv.prefs.clone();
        let devs = sv.devices.clone();
        let project_rate = sv.project.rate;
        let recording = port.telemetry().position.recording;
        let measuring = port.activity().iter().any(|a| a.stage == "Measuring the latency");
        let device_loading = target_loading(&self.scx, "device-loading", Target::Device, cx);
        let scan_loading = target_loading(&self.scx, "scan-loading", Target::Scan, cx);
        let mut rows = Vec::new();

        rows.push(
            row(
                "Audio system",
                Apply::ReopensDevice,
                segmented(
                    "audio-system",
                    vec![
                        (tr("ALSA").into(), p.audio_system == AudioSystem::Alsa, true),
                        (tr("JACK (phase 2)").into(), p.audio_system == AudioSystem::Jack, false),
                        (tr("No device").into(), p.audio_system == AudioSystem::Dummy, true),
                    ],
                    cx,
                    |s, i, _, cx| {
                        s.pref(cx, "audio.system", ["alsa", "jack", "dummy"][i]);
                        s.reopen_soon(cx);
                    },
                ),
            )
            .hint(tr("ALSA directly, the card used alone (hw); PipeWire and PulseAudio are asked to release it")),
        );

        // the device's state: opening (the loading component), open, or not open
        let state: AnyElement = if let Some(l) = device_loading {
            div().h(px(30.)).child(l).into_any_element()
        } else if let Some(o) = &devs.opened {
            let name = |id: &str| devs.find(id).map_or(id.to_string(), |d| d.name.clone());
            let ms = |f: u32| format!("{:.1}", f as f64 * 1000.0 / o.rate as f64);
            note(
                trf(
                    "Open: {0} → {1} at {2}, block {3} (agreed with the device), latency in {4} ms, out {5} ms{6}",
                    &[
                        &name(&o.input),
                        &name(&o.output),
                        &khz(o.rate),
                        &o.block,
                        &ms(o.latency_in),
                        &ms(o.latency_out),
                        &if o.edge_resampler { tr(", converted at the edge") } else { "" },
                    ],
                ),
                THEME.green,
            )
            .into_any_element()
        } else {
            note(tr("No audio device is open: the program plays and records nothing"), THEME.red).into_any_element()
        };
        let scan: AnyElement = match scan_loading {
            Some(l) => div().h(px(28.)).w(px(320.)).child(l).into_any_element(),
            None => button("scan-devices", tr("Look for devices again"), false, 0)
                .debug_selector(|| "scan-devices".into())
                .on_click(cx.listener(|s, _, _, cx| s.act(cx, "device.scan", Value::Text(String::new()))))
                .into_any_element(),
        };
        rows.push(wide(
            "Device",
            None,
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(state)
                .child(div().flex().items_center().gap(px(8.)).child(scan).child(caption(trf("Listed {0}", &[&ago(devs.scanned.elapsed())])))),
        ));

        // explicit input and output (U7): never joined by the program on its own
        let list = |input: bool, cx: &mut Context<Self>| {
            let chosen = if input { p.input_device.clone() } else { p.output_device.clone() };
            let sel: &'static str = if input { "input-dev" } else { "output-dev" };
            let mut cards: Vec<AnyElement> = Vec::new();
            for (i, d) in devs.audio.iter().enumerate().filter(|(_, d)| if input { d.inputs > 0 } else { d.outputs > 0 }) {
                let on = chosen.as_deref() == Some(d.id.as_str());
                let ch = if input { d.inputs } else { d.outputs };
                let rates = d.rates.iter().map(|r| khz(*r)).collect::<Vec<_>>().join(", ");
                let id = d.id.clone();
                cards.push(
                    radio_card((sel, i), on, d.name.clone(), trf("{0} · {1} channels · {2}", &[&d.id, &ch, &rates]))
                        .debug_selector(move || format!("{sel}-{i}"))
                        .when(d.bluetooth, |c| c.child(badge(tr("Bluetooth"), THEME.orange, false)))
                        .when_some(d.busy_by, |c, by| c.child(badge(trf("used by {0}", &[&by]), THEME.yellow, false)))
                        .on_click(cx.listener(move |s, _, _, cx| {
                            s.pref(cx, if input { "audio.input" } else { "audio.output" }, &id);
                            s.reopen_soon(cx);
                        }))
                        .into_any_element(),
                );
            }
            if input {
                cards.push(
                    radio_card((sel, 99usize), chosen.is_none(), tr("None"), tr("Plays only; nothing is recorded"))
                        .on_click(cx.listener(|s, _, _, cx| {
                            s.pref(cx, "audio.input", "");
                            s.reopen_soon(cx);
                        }))
                        .into_any_element(),
                );
            }
            div().flex().flex_col().gap(px(4.)).children(cards)
        };
        rows.push(wide("Input device", Some(Apply::ReopensDevice), list(true, cx)));
        rows.push(wide("Output device", Some(Apply::ReopensDevice), list(false, cx)));

        let different = matches!((&p.input_device, &p.output_device), (Some(a), Some(b)) if a != b);
        rows.push(
            row(
                "Correct the drift between cards",
                Apply::ReopensDevice,
                switch("drift", p.drift_correction, cx, |s, v, cx| {
                    s.pref(cx, "audio.drift", b(v));
                    s.reopen_soon(cx);
                }),
            )
            .hint(if different && !p.drift_correction {
                tr("The input and the output are on different cards (different clocks): they only open with this on, or with one card for both")
            } else {
                tr("Only for an input and an output on different cards: an adaptive resampler on the input, which adds latency")
            }),
        );
        if let Some(d) = p.input_device.as_deref().and_then(|i| devs.find(i)).filter(|d| d.bluetooth) {
            rows.push(wide(
                "Bluetooth microphone",
                None,
                note(trf("{0}: using its microphone drops the output to the call profile, over 150 ms of latency", &[&d.name]), THEME.orange),
            ));
        }

        // rate: the project's, or one the device takes, converted at the edge (spec item 8)
        let out = p.output_device.as_deref().and_then(|o| devs.find(o));
        let mut items: Vec<(SharedString, bool, bool)> = vec![(trf("The project's ({0})", &[&khz(project_rate)]).into(), p.device_rate.is_none(), true)];
        for r in RATES {
            items.push((khz(r).into(), p.device_rate == Some(r), out.is_none_or(|d| d.rates.contains(&r))));
        }
        let rate = p.device_rate.unwrap_or(project_rate);
        rows.push(
            row(
                "Sample rate",
                Apply::ReopensDevice,
                segmented("device-rate", items, cx, |s, i, _, cx| {
                    s.pref(cx, "audio.rate", if i == 0 { String::new() } else { RATES[i - 1].to_string() });
                    s.reopen_soon(cx);
                }),
            )
            .hint(if rate != project_rate {
                trf("The device runs at {0}; the project's {1} is converted at the edge, one high-quality converter each way", &[&khz(rate), &khz(project_rate)])
            } else {
                tr("The device opens at the project's rate; a rate it does not take is greyed").to_string()
            }),
        );

        // block size, with the latency measured by the device (F-REC-083) and the warnings (F-CFG-011)
        let out_extra = out.map_or(0, |d| d.reported_extra);
        let ms = |f: u32| format!("{:.1}", f as f64 * 1000.0 / rate as f64);
        let mut block_hint = trf(
            "{0} ms of buffer; {1} ms of output latency (measured by the device)",
            &[&ms(p.block), &ms(devs.opened.as_ref().map_or(p.block + out_extra, |o| o.latency_out))],
        );
        if !p.block.is_power_of_two() {
            block_hint = format!("{block_hint} · {}", tr("Not a power of 2: the device agrees on the next one"));
        }
        if p.block <= 32 {
            block_hint = format!("{block_hint} · {}", tr("32 frames or fewer: some plugins may not be available"));
        }
        rows.push(
            row(
                "Buffer size",
                Apply::ReopensDevice,
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().w(px(200.)).child(Slider::new(&self.block)))
                    .child(div().w(px(100.)).text_size(px(13.)).child(trf("{0} frames", &[&p.block])))
                    .when(p.block != 256, |d| {
                        d.child(button("block-default", trf("Back to the default ({0})", &[&256]), false, 0).on_click(cx.listener(
                            |s, _, window, cx| {
                                s.block.update(cx, |b, cx| b.set_value(256.0, window, cx));
                                s.pref(cx, "audio.block", 256);
                                s.reopen_soon(cx);
                            },
                        )))
                    }),
            )
            .hint(block_hint),
        );

        // latency calibration (I-10.4, H19): the residual per device pair, rate and edge converter
        let pair = project::pair_name(p.input_device.as_deref().unwrap_or("-"), p.output_device.as_deref().unwrap_or("-"));
        let edge = devs.opened.as_ref().is_some_and(|o| o.edge_resampler);
        let current = devs.calibration(&pair, rate, edge).cloned();
        let table = div().flex().flex_col().gap(px(2.)).children(devs.calibrations.iter().filter(|k| k.devices == pair).map(|k| {
            let this = k.rate == rate && k.edge_resampler == edge;
            div()
                .flex()
                .gap(px(14.))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(4.))
                .when(this, |d| d.bg(c(THEME.bg4)))
                .text_size(px(12.))
                .child(div().w(px(80.)).child(khz(k.rate)))
                .child(div().w(px(150.)).child(if k.edge_resampler { tr("with edge converter") } else { tr("without converter") }))
                .child(div().w(px(240.)).child(trf(
                    "{0} frames ({1} ms) beyond the reported",
                    &[&k.residual, &format!("{:.2}", k.residual as f64 * 1000.0 / k.rate as f64)],
                )))
                .child(div().w(px(90.)).text_color(c(THEME.text3)).child(trf("spread {0}", &[&k.spread])))
                .child(div().text_color(c(THEME.text3)).child(ago(k.at.elapsed())))
        }));
        rows.push(
            wide(
                "Latency calibration",
                Some(Apply::Now),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(match &current {
                        Some(k) => note(
                            trf("Calibrated for {0} at {1}: recordings move {2} frames beyond what the device reports", &[&pair, &khz(rate), &k.residual]),
                            THEME.green,
                        ),
                        None => note(
                            trf("{0} at {1} is not calibrated: recording uses only the latency the device reports, and warns", &[&pair, &khz(rate)]),
                            THEME.orange,
                        ),
                    })
                    .child(table)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                tbutton("measure", if measuring { tr("Measuring…") } else { tr("Measure…") }, measuring, THEME.green)
                                    .debug_selector(|| "measure".into())
                                    .when(!measuring && !recording, |b| {
                                        b.on_click(cx.listener(|s, _, _, cx| s.act(cx, "device.calibrate", Value::Text(String::new()))))
                                    }),
                            )
                            .child(caption(tr("Connect an output to an input with a cable: a pulse plays five times and the median is kept"))),
                    ),
            )
            .hint(tr("Kept per device, rate and edge converter, so it stays right when the buffer size changes")),
        );
        let off = p.latency_offset;
        rows.push(
            row(
                "Manual latency offset",
                Apply::Now,
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(stepper("latency-offset", trf("{0} frames", &[&off]), cx, move |s, d, cx| s.pref(cx, "audio.offset", off + d * 8)))
                    .child(caption(trf("{0} ms", &[&format!("{:+.2}", off as f64 * 1000.0 / rate as f64)])))
                    .when(off != 0, |d| d.child(button("offset-zero", tr("Back to 0"), false, 0).on_click(cx.listener(|s, _, _, cx| s.pref(cx, "audio.offset", 0))))),
            )
            .hint(tr("Added to the calibration, so that recordings line up with what was heard")),
        );

        // real-time permission on Linux (H1)
        let rt = &devs.realtime;
        let line = |ok: bool, text: String| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .child(badge(if ok { tr("ok") } else { tr("missing") }, if ok { THEME.green } else { THEME.red }, true))
                .child(text)
        };
        let all_ok = rt.priority_ok() && rt.memlock_ok();
        let memlock = rt.memlock_kb.map_or(tr("unlimited").to_string(), |k| format!("{} MB", k / 1024));
        rows.push(wide(
            "Real-time permission (Linux)",
            None,
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(line(rt.rtkit, tr("rtkit (D-Bus) grants real-time priority").to_string()))
                .child(line(rt.rtprio >= 70, trf("RLIMIT_RTPRIO is {0} (70 or more lets the program ask directly)", &[&rt.rtprio])))
                .child(line(rt.memlock_ok(), trf("RLIMIT_MEMLOCK is {0} (the audio memory is locked, so a page fault never clicks)", &[&memlock])))
                .child(line(rt.audio_group, trf("{0} is in the audio group", &[&rt.user])))
                .when(!all_ok, |d| {
                    d.child(note(tr("Without it the program plays anyway, but the audio may crackle under load. To configure, then log in again:"), THEME.orange))
                        .child(
                            div()
                                .p(px(8.))
                                .rounded(px(6.))
                                .bg(c(THEME.lcd_bg))
                                .text_size(px(12.))
                                .font_family("Menlo")
                                .flex()
                                .flex_col()
                                .child(format!("sudo usermod -aG audio {}", rt.user))
                                .child("echo '@audio - rtprio 95' | sudo tee -a /etc/security/limits.d/audio.conf")
                                .child("echo '@audio - memlock unlimited' | sudo tee -a /etc/security/limits.d/audio.conf"),
                        )
                })
                .child(div().child(button("rt-check", tr("Check again"), false, 0).on_click(cx.listener(|s, _, _, cx| s.act(cx, "device.scan", Value::Text(String::new())))))),
        ));
        rows
    }

    fn midi(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let sv = self.sv(cx);
        let p = sv.prefs.clone();
        let ports = sv.devices.midi.clone();
        let mut port_rows: Vec<AnyElement> = Vec::new();
        for (i, port) in ports.iter().enumerate() {
            let ch = if port.channel == 0 { tr("every channel").to_string() } else { trf("channel {0}", &[&port.channel]) };
            let (channel, latency) = (port.channel as i32, port.latency_ms);
            port_rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .bg(c(THEME.bg3))
                    .child(switch(format!("midi-{i}"), port.enabled, cx, move |s, v, cx| s.pref(cx, &format!("midi.port.{i}.enabled"), b(v))))
                    .child(div().flex_1().min_w_0().flex().flex_col().child(div().text_size(px(13.)).child(port.name.clone())).child(caption(port.id.clone())))
                    .child(segmented(
                        ["midi-mode-0", "midi-mode-1", "midi-mode-2", "midi-mode-3", "midi-mode-4", "midi-mode-5", "midi-mode-6", "midi-mode-7"][i % 8],
                        vec![
                            (tr("Plays the selected track").into(), port.mode == MidiMode::SelectedTrack, true),
                            (tr("Controllers only").into(), port.mode == MidiMode::Controllers, true),
                        ],
                        cx,
                        move |s, k, _, cx| s.pref(cx, &format!("midi.port.{i}.mode"), ["selected", "controllers"][k]),
                    ))
                    .child(stepper(format!("midi-ch-{i}"), ch, cx, move |s, d, cx| s.pref(cx, &format!("midi.port.{i}.channel"), channel + d)))
                    .child(stepper(format!("midi-lat-{i}"), trf("{0} ms", &[&latency]), cx, move |s, d, cx| {
                        s.pref(cx, &format!("midi.port.{i}.latency"), latency + d as f64)
                    }))
                    .into_any_element(),
            );
        }
        vec![
            row(
                "MIDI system",
                Apply::Now,
                segmented(
                    "midi-backend",
                    vec![
                        (tr("ALSA sequencer").into(), p.midi_backend == MidiBackend::Seq, true),
                        (tr("ALSA raw MIDI").into(), p.midi_backend == MidiBackend::Raw, true),
                    ],
                    cx,
                    |s, i, _, cx| s.pref(cx, "midi.backend", ["seq", "raw"][i]),
                ),
            )
            .hint(tr("The sequencer shares ports with other programs; raw MIDI talks to the device alone")),
            wide(
                "Inputs",
                Some(Apply::Now),
                div().flex().flex_col().gap(px(4.)).children(port_rows).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(button("midi-scan", tr("Look for devices again"), false, 0).on_click(cx.listener(|s, _, _, cx| s.act(cx, "device.scan", Value::Text(String::new())))))
                        .child(caption(tr("Per port: on or off, what it plays, its channel, and its own input latency, subtracted from what it records"))),
                ),
            )
            .hint(tr("These connections belong to this machine, not to the project: a project opens anywhere")),
            row("Quantize notes recorded in the Piano Roll", Apply::Now, switch("quantize", p.quantize_midi, cx, |s, v, cx| s.pref(cx, "midi.quantize", b(v))))
                .hint(tr("Off: recorded at the finest resolution")),
        ]
    }

    fn recording(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let p = self.sv(cx).prefs.clone();
        let db = p.metronome_db;
        vec![
            row(
                "Metronome",
                Apply::Now,
                segmented(
                    "metro-when",
                    vec![
                        (tr("Off").into(), p.metronome_when == MetronomeWhen::Off, true),
                        (tr("Only while recording").into(), p.metronome_when == MetronomeWhen::Recording, true),
                        (tr("While playing and recording").into(), p.metronome_when == MetronomeWhen::PlayingAndRecording, true),
                    ],
                    cx,
                    |s, i, _, cx| s.pref(cx, "rec.metronome_when", ["off", "recording", "always"][i]),
                ),
            )
            .hint(tr("The metronome button on the transport turns it on and off; this says when it clicks")),
            row("Metronome volume", Apply::Now, stepper("metro-db", trf("{0} dB", &[&format!("{db:.0}")]), cx, move |s, d, cx| s.pref(cx, "rec.metronome_db", db + d as f64))),
            row("Accent on the first beat", Apply::Now, switch("accent", p.metronome_accent, cx, |s, v, cx| s.pref(cx, "rec.accent", b(v)))),
            row("The metronome clicks during the pre-roll", Apply::Now, switch("click-preroll", p.click_in_preroll, cx, |s, v, cx| s.pref(cx, "rec.click_preroll", b(v))))
                .hint(tr("The count-in and the pre-roll themselves are the project's, on the transport bar")),
            row("Takes in lanes", Apply::Now, switch("lanes", p.takes_in_lanes, cx, |s, v, cx| s.pref(cx, "rec.lanes", b(v))))
                .hint(tr("Each pass on a lane of its own, the latest on top; off: the simple recorder, the new take replaces what it covers")),
            row(
                "When a take closes",
                Apply::Now,
                segmented(
                    "take-format",
                    vec![
                        (tr("Keep 32-bit float").into(), p.take_format == TakeFormat::KeepFloat, true),
                        (tr("Convert to FLAC").into(), p.take_format == TakeFormat::Flac, true),
                        (tr("Convert to 24 bits").into(), p.take_format == TakeFormat::Int24, true),
                    ],
                    cx,
                    |s, i, _, cx| s.pref(cx, "rec.take_format", ["float", "flac", "int24"][i]),
                ),
            )
            .hint(tr("Takes are always recorded as RF64 float, which survives a crash; this is what they become afterwards")),
            wide("Disk space", None, note(tr("Arming warns when the armed inputs do not fit 10 minutes on the disk, and refuses below 30 seconds"), THEME.blue)),
        ]
    }

    fn plugins(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let p = self.sv(cx).prefs.clone();
        let crashed: Vec<AnyElement> = p
            .not_loaded
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let n = name.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().text_size(px(13.)).child(name.clone()))
                    .child(button(("load-again", i), tr("Load again"), false, 0).on_click(cx.listener(move |s, _, _, cx| s.pref(cx, "plugins.load_again", &n))))
                    .into_any_element()
            })
            .collect();
        vec![
            row("Plugin windows on top", Apply::Now, switch("plugin-top", p.plugin_windows_on_top, cx, |s, v, cx| s.pref(cx, "plugins.on_top", b(v))))
                .hint(tr("For the plugin windows opened from now on")),
            row("One instrument window at a time", Apply::Now, switch("one-inst", p.one_instrument_window, cx, |s, v, cx| s.pref(cx, "plugins.one_window", b(v))))
                .hint(tr("Opening one closes the other")),
            wide(
                "Not loaded since a crash",
                Some(Apply::Now),
                div().flex().flex_col().gap(px(4.)).children(crashed).when(p.not_loaded.is_empty(), |d| d.child(caption(tr("None")))),
            )
            .hint(tr("The program closed unexpectedly while they ran; the projects keep their settings, and they load again from the next project opened")),
            wide(
                "Third-party plugins",
                None,
                note(tr("They run in a process of their own: if one falls, its track goes silent with a notice and it is recreated from the project"), THEME.blue),
            ),
        ]
    }

    fn folders(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let p = self.sv(cx).prefs.clone();
        let list = |id: &'static str, items: &[String], key: &'static str, cx: &mut Context<Self>| {
            div().flex().flex_col().gap(px(3.)).children(items.iter().enumerate().map(|(i, f)| {
                let f2 = f.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(c(THEME.bg3))
                    .child(div().flex_1().text_size(px(12.)).font_family("Menlo").child(f.clone()))
                    .child(button((id, i), tr("Remove"), false, 0).debug_selector(move || format!("{id}-{i}")).on_click(cx.listener(move |s, _, _, cx| s.pref(cx, key, &f2))))
            }))
        };
        vec![
            row("Working folder", Apply::Restart, div().w(px(360.)).child(Input::new(&self.projects_dir)))
                .hint(tr("Projects, templates, samples and presets; changing it takes effect after the program starts again")),
            row("Default SoundFont", Apply::Now, div().w(px(360.)).child(Input::new(&self.soundfont))).hint(tr("For new Sf2 Player instruments")),
            wide(
                "Plugin folders",
                Some(Apply::Now),
                div().flex().flex_col().gap(px(4.)).child(list("plugin-dir", &p.plugin_dirs, "folders.plugin_remove", cx)).child(
                    div().flex().gap(px(6.)).child(div().w(px(360.)).child(Input::new(&self.new_plugin_dir))).child(button("add-plugin-dir", tr("Add"), false, 0).on_click(
                        cx.listener(|s, _, _, cx| {
                            let v = s.new_plugin_dir.read(cx).value().trim().to_string();
                            if !v.is_empty() {
                                s.pref(cx, "folders.plugin_add", v)
                            }
                        }),
                    )),
                ),
            )
            .hint(tr("VST, LV2, CLAP and LADSPA folders, looked through when plugins are scanned")),
            wide(
                "Folders a project may read",
                Some(Apply::Now),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(note(
                        tr("A project's own files are in its folder. Files outside it (SoundFonts and kits of the system) are only read inside these folders; anything else shows as missing, with \"Allow this folder\"."),
                        THEME.blue,
                    ))
                    .child(list("allowed", &p.allowed, "folders.disallow", cx))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(div().w(px(360.)).child(Input::new(&self.new_folder)))
                            .child(button("add-allowed", tr("Allow"), false, 0).debug_selector(|| "add-allowed".into()).on_click(cx.listener(|s, _, _, cx| s.add_folder(cx)))),
                    ),
            ),
        ]
    }

    fn appearance(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let p = self.sv(cx).prefs.clone();
        let cards: Vec<AnyElement> = self
            .themes
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let on = p.theme == t.name;
                let name = t.name.clone();
                let label = match t.name.as_str() {
                    "dark" => tr("Dark").to_string(),
                    "light" => tr("Light").to_string(),
                    n => n.to_string(),
                };
                div()
                    .id(("theme", i))
                    .debug_selector(move || format!("theme-{i}"))
                    .w(px(170.))
                    .p(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .rounded(px(8.))
                    .border_2()
                    .border_color(c(if on { THEME.green } else { THEME.line2 }))
                    .cursor_pointer()
                    // the preview shows the theme file's own colors: data read from it, not this program's colors
                    .child(div().flex().h(px(34.)).rounded(px(4.)).overflow_hidden().children(t.swatch.iter().map(|col| div().flex_1().bg(c(*col)))))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(div().text_size(px(13.)).child(label))
                            .when(t.name == p.started_theme, |d| d.child(caption(tr("in use")))),
                    )
                    .on_click(cx.listener(move |s, _, _, cx| s.pref(cx, "look.theme", &name)))
                    .into_any_element()
            })
            .collect();
        vec![
            wide("Theme", Some(Apply::Restart), div().flex().flex_wrap().gap(px(8.)).children(cards))
                .hint(tr("A theme is a text file of colors (themes/*.txt, or ~/.config/linuxmusic/themes): copy one to make your own")),
            row(
                "Language",
                Apply::Restart,
                segmented(
                    "language",
                    vec![("Português (Brasil)".into(), p.language == "pt-BR", true), ("English".into(), p.language == "en", true)],
                    cx,
                    |s, i, _, cx| s.pref(cx, "look.language", ["pt-BR", "en"][i]),
                ),
            )
            .hint(tr("The legacy's other languages come where the text still matches")),
            row("Tooltips", Apply::Now, switch("tooltips", p.tooltips, cx, |s, v, cx| s.pref(cx, "look.tooltips", b(v)))),
            row("Compact track headers", Apply::Now, switch("compact", p.compact_headers, cx, |s, v, cx| s.pref(cx, "look.compact", b(v))))
                .hint(tr("Narrower; each header can still be widened by dragging its edge")),
            row("Browser on the right", Apply::Now, switch("browser-right", p.browser_right, cx, |s, v, cx| s.pref(cx, "look.browser_right", b(v)))),
            row(
                "Following the playhead",
                Apply::Now,
                segmented(
                    "follow",
                    vec![
                        (tr("Off").into(), p.follow == Follow::Off, true),
                        (tr("A page at a time").into(), p.follow == Follow::Page, true),
                        (tr("Continuous (kept in the middle)").into(), p.follow == Follow::Continuous, true),
                    ],
                    cx,
                    |s, i, _, cx| s.pref(cx, "look.follow", ["off", "page", "continuous"][i]),
                ),
            ),
            row("Smooth scrolling", Apply::Now, switch("smooth", p.smooth_scroll, cx, |s, v, cx| s.pref(cx, "look.smooth", b(v)))),
            row("Note names in the Piano Roll", Apply::Now, switch("note-names", p.note_names, cx, |s, v, cx| s.pref(cx, "look.note_names", b(v)))),
            row("Fader ticks every 6 dB", Apply::Now, switch("fader-ticks", p.fader_ticks, cx, |s, v, cx| s.pref(cx, "look.fader_ticks", b(v)))),
            row(
                "Detached windows",
                Apply::Now,
                segmented(
                    "detached",
                    vec![
                        (tr("Dock and show when closed").into(), p.detached == Detached::DockAndShow, true),
                        (tr("Dock and hide when closed").into(), p.detached == Detached::DockAndHide, true),
                        (tr("Always detached").into(), p.detached == Detached::Always, true),
                    ],
                    cx,
                    |s, i, _, cx| s.pref(cx, "look.detached", ["show", "hide", "always"][i]),
                ),
            ),
        ]
    }

    fn keys(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let p = self.sv(cx).prefs.clone();
        let mut groups: Vec<&'static str> = Vec::new();
        for a in actions::REGISTRY.iter().filter(|a| a.palette) {
            if !groups.contains(&a.group) {
                groups.push(a.group);
            }
        }
        let mut out = Vec::new();
        for g in groups {
            let lines: Vec<AnyElement> = actions::REGISTRY
                .iter()
                .enumerate()
                .filter(|(_, a)| a.palette && a.group == g)
                .map(|(i, a)| {
                    let id = a.id;
                    let keys = actions::shortcut(&p, id);
                    let capturing = self.capturing == Some(id);
                    let changed = p.keys.contains_key(id);
                    let conflict = if capturing { self.pending_keys.as_deref().and_then(|k| actions::conflict(&p, k, id)) } else { None };
                    let shown: AnyElement = if capturing {
                        match conflict {
                            Some(o) => caption(trf("Already used by \"{0}\": press them again to take them, Esc cancels", &[&tr(o.label)])).into_any_element(),
                            None => caption(tr("Press the new keys… (Esc cancels)")).into_any_element(),
                        }
                    } else {
                        match keys {
                            Some(k) => kbd(keys_label(&k)).into_any_element(),
                            None => caption(tr("none")).into_any_element(),
                        }
                    };
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(4.))
                        .when(capturing, |d| d.bg(c(THEME.bg4)))
                        .child(div().flex_1().text_size(px(13.)).child(tr(a.label)))
                        .child(div().w(px(330.)).flex().justify_end().child(shown))
                        .child(button(("key-change", i), tr("Change"), capturing, THEME.green).debug_selector(move || format!("key-change-{i}")).on_click(
                            cx.listener(move |s, _, window, cx| {
                                s.capturing = Some(id);
                                s.pending_keys = None;
                                window.focus(&s.focus, cx);
                                cx.notify();
                            }),
                        ))
                        .child(button(("key-clear", i), tr("Remove"), false, 0).on_click(cx.listener(move |s, _, _, cx| s.pref(cx, &format!("keys.{id}"), ""))))
                        .child(div().w(px(70.)).when(changed, |d| {
                            d.child(button(("key-default", i), tr("Default"), false, 0).on_click(cx.listener(move |s, _, _, cx| s.pref(cx, &format!("keys.{id}"), "default"))))
                        }))
                        .into_any_element()
                })
                .collect();
            out.push(wide(g, Some(Apply::Now), div().flex().flex_col().children(lines)));
        }
        out
    }

    fn backup(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let sv = self.sv(cx);
        let p = sv.prefs.clone();
        let journal = sv.project.journal;
        let (auto, kept) = (p.autosave_minutes as i32, p.backups_kept as i32);
        vec![
            wide(
                "Journal",
                None,
                note(
                    trf("Always on: every change goes to backup/diario.jsonl as it happens ({0} since the last save), and after a crash the project comes back to the last action", &[&journal]),
                    THEME.blue,
                ),
            ),
            row(
                "Autosave",
                Apply::Now,
                stepper("autosave", if auto == 0 { tr("off").to_string() } else { trf("every {0} min", &[&auto]) }, cx, move |s, d, cx| s.pref(cx, "backup.autosave", auto + d)),
            )
            .hint(tr("A copy of projeto.json in backup/, also while playing and recording")),
            row("Copies kept", Apply::Now, stepper("backups-kept", kept.to_string(), cx, move |s, d, cx| s.pref(cx, "backup.kept", kept + d))),
            row("Keep a backup when saving", Apply::Now, switch("backup-save", p.backup_on_save, cx, |s, v, cx| s.pref(cx, "backup.on_save", b(v))))
                .hint(tr("The previous version is kept in backup/; a project from an older version is always kept as projeto.json.v<N>.bak")),
        ]
    }

    fn server_page(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let sv = self.sv(cx);
        let link = sv.link.clone();
        let servers = sv.servers.clone();
        vec![wide("Connect to a server", Some(Apply::Now), connect_panel(&link, &servers, &self.server, &self.token, cx, |s: &mut Settings| (s.scx.clone(), s.server.clone())))]
    }

    fn advanced(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let p = self.sv(cx).prefs.clone();
        vec![
            row("Clean corrupted audio", Apply::Now, switch("sanitize", p.sanitize, cx, |s, v, cx| s.pref(cx, "adv.sanitize", b(v))))
                .hint(tr("NaN and infinite values from a plugin become silence, and the plugin is marked")),
            wide("Hearing protection", None, note(tr("Fixed: a peak above +24 dBFS on the master cuts the output with a very short fade, and warns"), THEME.blue)),
            wide(
                "Effects in silence",
                None,
                note(tr("Each effect declares its tail; after its input stays silent longer than the tail, it is not called, sparing the processor"), THEME.blue),
            ),
        ]
    }
}

/// "Connect to a server" (I-15, I-20): the address, the pairing token, the state, and where the sound comes out.
/// Shared by the settings and the project dialog; `parts` gives the view's screen context and address field.
pub fn connect_panel<V: 'static>(
    link: &Link,
    servers: &[String],
    address: &Entity<InputState>,
    token: &Entity<InputState>,
    cx: &mut Context<V>,
    parts: fn(&mut V) -> (ScreenCx, Entity<InputState>),
) -> Div {
    let state = match link {
        Link::Local => note(tr("The core runs in this program (local)"), THEME.blue),
        Link::Connecting { address } => note(trf("Connecting to {0}…", &[address]), THEME.yellow),
        Link::Connected { address, server, round_trip_ms, .. } => {
            note(trf("Connected to {0} ({1}), {2} ms round trip", &[address, server, &format!("{round_trip_ms:.1}")]), THEME.green)
        }
        Link::Lost { address, at } => note(trf("The connection to {0} was lost {1}; the server goes on", &[address, &ago(at.elapsed())]), THEME.red),
    };
    let token2 = token.clone();
    let connected = matches!(link, Link::Connected { .. } | Link::Connecting { .. });
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(state)
        .child(note(tr("The sound comes out on the server's machine: the speakers and the audio interface are plugged in there"), THEME.orange))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(div().w(px(300.)).child(Input::new(address)))
                .child(div().w(px(180.)).child(Input::new(token)))
                .child(if connected {
                    button("disconnect", tr("Disconnect"), false, 0)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            let (scx, _) = parts(v);
                            port_action(&scx, "remote.disconnect", Value::Text(String::new()), cx);
                        }))
                        .into_any_element()
                } else {
                    tbutton("connect", tr("Connect"), true, THEME.green)
                        .debug_selector(|| "connect".into())
                        .on_click(cx.listener(move |v, _, _, cx| {
                            let (scx, addr) = parts(v);
                            let a = addr.read(cx).value().to_string();
                            let t = token2.read(cx).value().to_string();
                            port_action(&scx, "remote.connect_to", actions::param_lines(&[("address", a), ("token", t)]), cx);
                        }))
                        .into_any_element()
                }),
        )
        .child(caption(tr("A local socket (/run/user/…/linuxmusic.sock) or host:port, always encrypted (TLS); the token pairs this machine once")))
        .child(div().flex().flex_wrap().gap(px(6.)).children(servers.iter().enumerate().map(|(i, a)| {
            let a2 = a.clone();
            button(("server", i), a.clone(), false, 0).on_click(cx.listener(move |v, _, window, cx| {
                let (_, addr) = parts(v);
                let a = a2.clone();
                addr.update(cx, |st, cx| st.set_value(a, window, cx));
            }))
        })))
}

// ---------------------------------------------------------------------------------------------------------------

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = modal_size(window, 1100.0, 760.0);
        let query = self.search.read(cx).value().to_string();
        let searching = !query.trim().is_empty();
        let pending: Vec<&'static str> = self.sv(cx).prefs.pending_restart.clone();
        let fold = |s: &str| s.chars().map(actions::fold).collect::<String>();
        let q = fold(query.trim());

        // the rows shown: the page's, or the rows of every page that match the search (labels, hints, page titles)
        let mut sections: Vec<(Page, Vec<Row>)> = Vec::new();
        if searching {
            for p in Page::ALL {
                let title = fold(tr(p.title()));
                let rows: Vec<Row> = self
                    .rows(p, cx)
                    .into_iter()
                    .filter(|r| title.contains(&q) || fold(tr(r.label)).contains(&q) || r.hint.as_deref().is_some_and(|h| fold(h).contains(&q)))
                    .collect();
                if !rows.is_empty() {
                    sections.push((p, rows));
                }
            }
        } else {
            sections.push((self.page, self.rows(self.page, cx)));
        }
        let found = sections.iter().map(|(_, r)| r.len()).sum::<usize>();

        let render_row = |r: Row| {
            let hint = r.hint.map(|h| div().text_size(px(11.)).text_color(c(THEME.text3)).child(h));
            if r.wide {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr(r.label)))
                            .children(r.apply.map(apply_badge)),
                    )
                    .child(r.control)
                    .children(hint)
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .w(px(250.))
                            .flex_none()
                            .child(div().text_size(px(13.)).child(tr(r.label)))
                            .children(r.apply.map(|a| div().flex().child(apply_badge(a)))),
                    )
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.)).child(r.control).children(hint))
            }
        };

        let content = div()
            .id("settings-content")
            .flex_1()
            .overflow_y_scroll()
            .px(px(24.))
            .py(px(18.))
            .flex()
            .flex_col()
            .children(sections.into_iter().map(|(p, rows)| {
                div()
                    .flex()
                    .flex_col()
                    .mb(px(18.))
                    .child(div().text_size(px(19.)).font_weight(FontWeight::BOLD).child(tr(p.title())))
                    .child(div().text_size(px(12.)).text_color(c(THEME.text3)).mb(px(8.)).child(tr(p.about())))
                    .children(rows.into_iter().map(render_row))
            }))
            .when(searching && found == 0, |d| d.child(div().text_color(c(THEME.text3)).child(trf("No setting has \"{0}\"", &[&query]))));

        let band = (!pending.is_empty() && !self.band_hidden).then(|| {
            let names = pending.iter().map(|k| tr(k)).collect::<Vec<_>>().join(", ");
            div()
                .id("restart-band")
                .debug_selector(|| "restart-band".into())
                .mx(px(24.))
                .mb(px(16.))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(14.))
                .py(px(10.))
                .rounded(px(8.))
                .border_1()
                .border_color(c(THEME.yellow))
                .bg(c(THEME.bg3))
                .child(div().text_color(c(THEME.yellow)).child("⟳"))
                .child(div().flex_1().text_size(px(13.)).child(trf("{0}: changes after the program restarts.", &[&names])))
                .child(button("later", tr("Later"), false, 0).on_click(cx.listener(|s, _, _, cx| {
                    s.band_hidden = true;
                    cx.notify();
                })))
                .child(tbutton("restart-now", tr("Restart now"), true, THEME.green).on_click(cx.listener(|s, _, _, cx| s.restart(cx))))
        });

        let nav = div()
            .w(px(230.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(12.))
            .bg(c(THEME.bg1))
            .border_r_1()
            .border_color(c(THEME.line))
            .child(div().mb(px(8.)).child(Input::new(&self.search)))
            .children(Page::ALL.iter().map(|&p| {
                let on = p == self.page && !searching;
                div()
                    .id(("settings-page", p as usize))
                    .debug_selector(move || format!("settings-page-{p:?}"))
                    .px(px(12.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .when(on, |d| d.bg(c(THEME.bg3)).font_weight(FontWeight::BOLD))
                    .text_color(c(if on { THEME.text } else { THEME.text2 }))
                    .cursor_pointer()
                    .child(tr(p.title()))
                    .on_click(cx.listener(move |s, _, window, cx| {
                        s.page = p;
                        s.search.update(cx, |st, cx| st.set_value("", window, cx));
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(button("close-settings", tr("Close"), false, 0).on_click(cx.listener(|s, _, _, cx| s.scx.close(cx))));

        div()
            .id("settings")
            .w(px(w))
            .h(px(h))
            .flex()
            .rounded(px(10.))
            .overflow_hidden()
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(nav)
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().child(content).children(band))
    }
}

#[cfg(test)]
mod tests {
    use super::super::Screen;
    use super::super::palette::{ScreenArg, testing::*};
    use super::*;

    struct Snap {
        prefs: fake_core::project::Prefs,
    }
    fn services(core: &gpui::Entity<crate::core::Core>, cx: &gpui::VisualTestContext) -> Snap {
        core.read_with(cx, |c, _| Snap { prefs: c.port().services().prefs.clone() })
    }

    /// Design 08: choosing another output applies at once and reopens the device by itself after 0.4 s, as an operation
    #[gpui::test]
    fn a_device_choice_applies_at_once(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        open_screen(&ws, cx, Screen::Settings);
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        click(cx, "output-dev-0");
        assert_eq!(services(&core, cx).prefs.output_device.as_deref(), Some("hw:0"));
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        core.read_with(cx, |c, _| assert_eq!(c.port().loading(&Target::Device), Loading::Pending, "the reopen started"));
        click(cx, "settings-page-General");
        click(cx, "reopen-last");
        assert!(services(&core, cx).prefs.reopen_last, "a switch applies at once");
    }

    /// A restart-only setting shows the band; the search finds rows of every page, without accents
    #[gpui::test]
    fn restart_band_and_search(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        open_screen(&ws, cx, Screen::Settings);
        click(cx, "settings-page-Appearance");
        click(cx, "language-1");
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        assert_eq!(services(&core, cx).prefs.pending_restart, vec!["Language"]);
        assert!(cx.debug_bounds("restart-band").is_some());
        click(cx, "language-0");
        assert!(services(&core, cx).prefs.pending_restart.is_empty(), "back to the running language: no restart");
    }

    /// A shortcut is changed by pressing it; a conflict asks for a second press, which takes the keys
    #[gpui::test]
    fn change_a_shortcut(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        cx.update(|_, cx| cx.set_global(ScreenArg(Some("keys"))));
        open_screen(&ws, cx, Screen::Settings);
        let i = actions::REGISTRY.iter().position(|a| a.id == "transport.stop").unwrap();
        let sel: &'static str = Box::leak(format!("key-change-{i}").into_boxed_str());
        click(cx, sel);
        cx.simulate_keystrokes("alt-x");
        cx.run_until_parked();
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        assert_eq!(actions::shortcut(&services(&core, cx).prefs, "transport.stop").as_deref(), Some("alt-x"));
        click(cx, sel);
        cx.simulate_keystrokes("r");
        cx.run_until_parked();
        assert_eq!(actions::shortcut(&services(&core, cx).prefs, "transport.stop").as_deref(), Some("alt-x"), "a conflict waits");
        cx.simulate_keystrokes("r");
        cx.run_until_parked();
        let p = services(&core, cx).prefs;
        assert_eq!(actions::shortcut(&p, "transport.stop").as_deref(), Some("r"));
        assert_eq!(actions::shortcut(&p, "transport.record"), None, "taken from Record");
    }
}
