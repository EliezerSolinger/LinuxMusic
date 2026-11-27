//! Owner: agent D. The settings, as approved design 08 says: the sections in a list on the left with a search over
//! every label and hint (without accents, F-CFG-001); each row says, before it is changed, whether it applies at once
//! or asks for a restart; what applies at once is applied (a device setting reopens the device by itself 0.4 s after
//! the last change, an operation with its loading, F-REC-120); a restart-only change shows the band "Restart now"
//! (F-CFG-002).
//!
//! Everything is the preferences tree (`CorePort::preferences`, shapes in `lm_port::trees`), changed by
//! `set_preference(path, value)`; the device's state, the real-time permission and the MIDI ports' hotplug state come
//! from `status()`, the devices from `devices()`. Pages: General, Audio (U7, H19, F-REC-081/082, H1), MIDI (I-21.1:
//! ports by client and port name, alias, role, input/output, latencies, connected or not; controller profiles
//! I-21.3), Recording and metronome, Plugins, Folders (C9), Appearance (themes as data, language), Keyboard (the
//! action registry), Autosave.
//!
//! The form pieces (`segmented`, `stepper`, `switch`, `badge`, `note`, `form_row`, `target_loading`, `modal_size`)
//! are shared by agent D's other screens.

use super::settings_mpe;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, Render, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::switch::Switch;
use lm_port::{DeviceInfo, Loading, PathStep, Pending, Target, Value};

mod keys;

use super::ScreenCx;
use super::palette::{fold, keys_label, keys_of_event, take_screen_arg};
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
    div().flex().flex_wrap().gap(px(3.)).children(items.into_iter().enumerate().map(|(i, (label, on, enabled))| {
        let p = pick.clone();
        let b =
            button((id, i), label, on, THEME.green).h(px(22.)).px(px(7.)).debug_selector(move || format!("{id}-{i}"));
        if enabled {
            b.on_click(cx.listener(move |v, _, w, cx| p(v, i, w, cx)))
        } else {
            b.opacity(0.4).cursor_default()
        }
    }))
}

/// − value +
pub fn stepper<V: 'static>(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
    cx: &Context<V>,
    step: impl Fn(&mut V, i64, &mut Context<V>) + 'static,
) -> Div {
    let id: SharedString = id.into();
    let step = Rc::new(step);
    let (a, b) = (step.clone(), step);
    let (i1, i2) = (id.clone(), id.clone());
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(
            button(ElementId::Name(format!("{id}-down").into()), "−", false, 0)
                .h(px(22.))
                .w(px(22.))
                .debug_selector(move || format!("{i1}-down"))
                .on_click(cx.listener(move |v, _, _, cx| a(v, -1, cx))),
        )
        .child(div().min_w(px(64.)).flex().justify_center().text_size(px(12.)).child(text.into()))
        .child(
            button(ElementId::Name(format!("{id}-up").into()), "+", false, 0)
                .h(px(22.))
                .w(px(22.))
                .debug_selector(move || format!("{i2}-up"))
                .on_click(cx.listener(move |v, _, _, cx| b(v, 1, cx))),
        )
}

/// An on/off switch (gpui-component's, in our theme)
pub fn switch<V: 'static>(
    id: impl Into<SharedString>,
    on: bool,
    cx: &Context<V>,
    set: impl Fn(&mut V, bool, &mut Context<V>) + 'static,
) -> Div {
    let id: SharedString = id.into();
    let sel = id.clone();
    div().flex().child(div().debug_selector(move || sel.to_string()).child(
        Switch::new(ElementId::Name(id)).checked(on).on_click(cx.listener(move |v, on: &bool, _, cx| set(v, *on, cx))),
    ))
}

/// A small colored word ("applies at once", "disconnected")
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

/// A line of warning or information
pub fn note(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex()
        .gap(px(8.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(c(color))
        .text_size(px(12.))
        .child(div().flex_none().w(px(3.)).rounded(px(2.)).bg(c(color)))
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// A form line: the label in a fixed column, the control beside it (the legacy dialogs' QFormLayout)
pub fn form_row(label: impl Into<SharedString>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .min_h(px(26.))
        .child(div().w(px(150.)).flex_none().text_size(px(12.)).text_color(c(THEME.text2)).child(label.into()))
        .child(div().flex_1().min_w_0().child(control))
}

/// The loading of a target with the cancel of its operation (the shared component, I-2.3)
pub fn target_loading(scx: &ScreenCx, id: &'static str, target: Target, cx: &App) -> Option<AnyElement> {
    let port = scx.core.read(cx).port();
    let loading: Loading = port.loading(&target);
    let op = port.activity().into_iter().find(|a| a.target == target).map(|a| a.id);
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

pub fn khz(r: i64) -> String {
    let k = r as f64 / 1000.0;
    if (k - k.round()).abs() < 1e-9 { format!("{k:.0} kHz") } else { format!("{k:.1} kHz").replace('.', ",") }
}

/// A key path from its steps
pub fn path(keys: &[&str]) -> Vec<PathStep> {
    keys.iter().map(|k| PathStep::key(k)).collect()
}

/// Runs a core action through the port; a `Waiting` answer arrives in a later pump (its effect shows in the trees)
pub fn port_action(scx: &ScreenCx, id: &str, params: Value, cx: &mut App) {
    scx.core.update(cx, |core, cx| {
        // a refusal comes back as a notice from the core; a Waiting answer arrives in a later pump
        core.report(|pt| pt.run_action(id.into(), params));
        cx.notify();
    });
}

/// An object `Value` from pairs
pub fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::object(pairs)
}

pub fn real(x: f64) -> Value {
    Value::real(x).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Apply {
    Now,
    Restart,
}

/// Only what asks for a restart is marked: "applies at once" is the rule, not news
fn apply_badge(a: Apply) -> Option<Div> {
    match a {
        Apply::Now => None,
        Apply::Restart => Some(badge(tr("asks for a restart"), THEME.yellow, false)),
    }
}

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
    Autosave,
}

impl Page {
    pub const ALL: [Page; 9] = [
        Page::General,
        Page::Audio,
        Page::Midi,
        Page::Recording,
        Page::Plugins,
        Page::Folders,
        Page::Appearance,
        Page::Keys,
        Page::Autosave,
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
            Page::Keys => "Shortcuts",
            Page::Autosave => "Autosave and backup",
        }
    }
    fn from_arg(a: &str) -> Option<Page> {
        Page::ALL.iter().copied().find(|p| format!("{p:?}").to_lowercase() == a)
    }
}

/// One setting: its label, when it applies, a hint, its control; `wide` puts the control under the label
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

fn b(v: bool) -> Value {
    Value::bool(v)
}

fn text_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_text).unwrap_or("").to_string()
}

const RATES: [i64; 5] = [44_100, 48_000, 88_200, 96_000, 192_000];
const BLOCKS: [i64; 8] = [32, 64, 128, 256, 512, 1024, 2048, 4096];

pub struct Settings {
    scx: ScreenCx,
    page: Page,
    search: Entity<InputState>,
    projects_dir: Entity<InputState>,
    soundfont: Entity<InputState>,
    new_folder: Entity<InputState>,
    /// The alias being typed, per MIDI port of the preferences (by index)
    aliases: Vec<Entity<InputState>>,
    devices: Option<Vec<DeviceInfo>>,
    /// The Shortcuts page: the context and tab shown, the preset menu open, the key or gesture being captured, the keys
    /// of a conflict waiting for a second press, the search and the pressed key it looks for, the new preset's name
    capturing: Option<keys::Capture>,
    pending_keys: Option<String>,
    kconfirm: Option<String>,
    kctx: &'static str,
    ktab: keys::KeysTab,
    kmenu: Option<&'static str>,
    ksearch: Entity<InputState>,
    kname: Entity<InputState>,
    kfind: Option<String>,
    kstatus: Option<&'static str>,
    focus: FocusHandle,
    band_hidden: bool,
    _reopen: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl Settings {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let arg = take_screen_arg(cx);
        let prefs = scx.core.read(cx).port().preferences().clone();
        let text_at = |p: &[&str]| prefs.at_path(&path(p)).and_then(Value::as_text).unwrap_or("").to_string();
        let input = |v: String, ph: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(v).placeholder(tr(ph)))
        };
        let search = input(String::new(), "Search settings…", window, cx);
        let projects_dir = input(text_at(&["projectsFolder"]), "", window, cx);
        let soundfont = input(text_at(&["folders", "soundfont"]), "/usr/share/sounds/sf2/FluidR3_GM.sf2", window, cx);
        let new_folder = input(String::new(), "/a/folder/to/allow", window, cx);
        let ksearch = input(String::new(), "Search shortcuts…", window, cx);
        let kname = input(String::new(), "Name", window, cx);
        let ports: Vec<Value> =
            prefs.at_path(&path(&["midi", "ports"])).map(|l| l.items().cloned().collect()).unwrap_or_default();
        let aliases: Vec<_> = ports.iter().map(|p| input(text_of(p, "alias"), "alias", window, cx)).collect();
        let mut subs = vec![
            cx.subscribe(&search, |_: &mut Settings, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&ksearch, |_: &mut Settings, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&projects_dir, |s: &mut Settings, st, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = st.read(cx).value().to_string();
                    s.set(cx, &["projectsFolder"], Value::text(v));
                }
            }),
            cx.subscribe(&soundfont, |s: &mut Settings, st, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = st.read(cx).value().to_string();
                    s.set(cx, &["folders", "soundfont"], Value::text(v));
                }
            }),
            cx.subscribe(&new_folder, |s: &mut Settings, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    s.add_folder(cx);
                }
            }),
        ];
        for (i, a) in aliases.iter().enumerate() {
            subs.push(cx.subscribe(a, move |s: &mut Settings, st, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = st.read(cx).value().trim().to_string();
                    s.set_in(cx, &["midi", "ports"], i, "alias", Value::text(v));
                }
            }));
        }
        let mut s = Settings {
            scx,
            page: arg.and_then(Page::from_arg).unwrap_or(Page::Audio),
            search,
            projects_dir,
            soundfont,
            new_folder,
            aliases,
            devices: None,
            capturing: None,
            pending_keys: None,
            kconfirm: None,
            kctx: "Global",
            ktab: keys::KeysTab::Keys,
            kmenu: None,
            ksearch,
            kname,
            kfind: None,
            kstatus: None,
            focus: cx.focus_handle(),
            band_hidden: false,
            _reopen: None,
            _subs: subs,
        };
        s.list_devices(cx);
        s
    }

    fn list_devices(&mut self, cx: &mut Context<Self>) {
        let r = self.scx.core.update(cx, |core, _| core.port_mut().devices());
        self.devices = match r {
            Pending::Ready(Ok(d)) => Some(d),
            // over the network the list comes later; asked again when the page draws
            Pending::Waiting(_) | Pending::Ready(Err(_)) => None,
        };
    }

    fn prefs<'a>(&self, cx: &'a App) -> &'a Value {
        self.scx.core.read(cx).port().preferences()
    }

    fn status<'a>(&self, cx: &'a App) -> &'a Value {
        self.scx.core.read(cx).port().status()
    }

    fn get<'a>(&self, cx: &'a App, p: &[&str]) -> Option<&'a Value> {
        self.prefs(cx).at_path(&path(p))
    }

    fn get_bool(&self, cx: &App, p: &[&str], default: bool) -> bool {
        self.get(cx, p).and_then(Value::as_bool).unwrap_or(default)
    }

    fn get_int(&self, cx: &App, p: &[&str], default: i64) -> i64 {
        self.get(cx, p).and_then(Value::as_int).unwrap_or(default)
    }

    fn get_text(&self, cx: &App, p: &[&str], default: &str) -> String {
        self.get(cx, p).and_then(Value::as_text).unwrap_or(default).to_string()
    }

    fn list_at(&self, cx: &App, p: &[&str]) -> Vec<String> {
        self.get(cx, p).map(|l| l.items().filter_map(|x| x.as_text().map(str::to_string)).collect()).unwrap_or_default()
    }

    /// One setting, applied at once (the core reads the tree)
    fn set_path(&mut self, cx: &mut Context<Self>, p: Vec<PathStep>, v: Value) {
        self.scx.core.update(cx, |core, cx| {
            core.report(|pt| pt.set_preference(p, v));
            cx.notify();
        });
        cx.notify();
    }

    fn set(&mut self, cx: &mut Context<Self>, p: &[&str], v: Value) {
        self.set_path(cx, path(p), v);
    }

    /// A field of the `i`-th entry of a list (`midi.ports[i].alias`, `controllers[i].bank`)
    fn set_in(&mut self, cx: &mut Context<Self>, list: &[&str], i: usize, key: &str, v: Value) {
        let mut p = path(list);
        p.push(PathStep::Index(i));
        p.push(PathStep::key(key));
        self.set_path(cx, p, v);
    }

    /// The settings changed in this run that wait for a restart: a language or a theme this run did not start with
    fn pending_restart(&self, cx: &App) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.get_text(cx, &["language"], "pt-BR") != crate::ui::i18n::language() {
            v.push("Language");
        }
        if self.get_text(cx, &["theme"], "dark") != crate::ui::theme::theme_name() {
            v.push("Theme");
        }
        if self.get_text(cx, &["projectsFolder"], "") != self.started_projects_folder(cx) {
            v.push("Working folder");
        }
        v
    }

    fn started_projects_folder(&self, cx: &App) -> String {
        // the folder the session started with is the status' (the project lives under it); the fake has none
        self.status(cx)
            .get("projectsFolder")
            .and_then(Value::as_text)
            .map_or_else(|| self.get_text(cx, &["projectsFolder"], ""), str::to_string)
    }

    /// A device setting changed: the device reopens 0.4 s after the last change (F-REC-120)
    fn reopen_soon(&mut self, cx: &mut Context<Self>) {
        let scx = self.scx.clone();
        self._reopen = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            cx.update(|cx| port_action(&scx, "device.reopen", Value::NULL, cx));
        }));
    }

    fn add_folder(&mut self, cx: &mut Context<Self>) {
        let v = self.new_folder.read(cx).value().trim().to_string();
        let mut all = self.list_at(cx, &["folders", "allowed"]);
        if (v.starts_with('/') || v.starts_with("~/")) && !all.contains(&v) {
            all.push(v);
            self.set(cx, &["folders", "allowed"], Value::list(all.into_iter().map(Value::text)));
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.capture_key(ev, cx);
    }

    /// The keybindings again, from the registry and the user's shortcuts
    fn rebind(&mut self, cx: &mut Context<Self>) {
        let prefs = self.prefs(cx).clone();
        cx.clear_key_bindings();
        let _ = crate::actions::bind_keys(cx, &prefs);
    }

    /// Restart now (F-GER-013): saves, and starts the program again on the same project (a client connects again,
    /// with the arguments it had); the language and the theme are read from the preferences
    fn restart(&mut self, cx: &mut Context<Self>) {
        let lang = self.get_text(cx, &["language"], "pt-BR");
        let folder = self
            .status(cx)
            .get("project")
            .and_then(|p| p.get("folder"))
            .and_then(Value::as_text)
            .filter(|f| !f.is_empty())
            .map(str::to_owned);
        let read_only = self.status(cx).get("project").and_then(|p| p.get("readOnly")).and_then(Value::as_bool);
        if read_only != Some(true) {
            port_action(&self.scx, "project.save", Value::NULL, cx);
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut args: Vec<String> = std::env::args().skip(1).collect();
            if args.is_empty() {
                args.extend(folder);
            }
            let spawned =
                std::process::Command::new(exe).args(args).env("LM_OPEN", "Settings").env("LM_LANG", lang).spawn();
            if spawned.is_ok() {
                cx.quit();
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The pages

impl Settings {
    fn rows(&mut self, page: Page, cx: &mut Context<Self>) -> Vec<Row> {
        match page {
            Page::General => self.general(cx),
            Page::Audio => self.audio(cx),
            Page::Midi => self.midi(cx),
            Page::Recording => self.recording(cx),
            Page::Plugins => self.plugins(cx),
            Page::Folders => self.folders(cx),
            Page::Appearance => self.appearance(cx),
            Page::Keys => self.keys(cx),
            Page::Autosave => self.autosave(cx),
        }
    }

    fn toggle(
        &self,
        label: &'static str,
        id: &'static str,
        p: &'static [&'static str],
        default: bool,
        cx: &mut Context<Self>,
    ) -> Row {
        let on = self.get_bool(cx, p, default);
        row(label, Apply::Now, switch(id, on, cx, move |s, v, cx| s.set(cx, p, b(v))))
    }

    fn general(&mut self, cx: &mut Context<Self>) -> Vec<Row> {
        let rate = self.get_int(cx, &["general", "newProjectRate"], 48_000);
        let project_rate = crate::model::sample_rate(self.scx.core.read(cx).port().root()) as i64;
        vec![
            self.toggle("Reopen the last project on startup", "reopen-last", &["general", "reopenLast"], false, cx),
            self.toggle("Ask before closing with changes not saved", "ask-closing", &["askBeforeClosing"], true, cx),
            self.toggle("Ask before deleting a track", "ask-track", &["interface", "confirmTrackRemoval"], true, cx),
            row(
                "Sample rate of new projects",
                Apply::Now,
                segmented(
                    "new-rate",
                    RATES.iter().map(|r| (khz(*r).into(), *r == rate, true)).collect(),
                    cx,
                    |s, i, _, cx| s.set(cx, &["general", "newProjectRate"], Value::int(RATES[i])),
                ),
            )
            .hint(trf("Each project keeps its own rate; this one is at {0}", &[&khz(project_rate)])),
        ]
    }

    fn audio(&mut self, cx: &mut Context<Self>) -> Vec<Row> {
        if self.devices.is_none() {
            self.list_devices(cx);
        }
        let devices = self.devices.clone().unwrap_or_default();
        let system = self.get_text(cx, &["audio", "system"], "alsa");
        let input = self.get_text(cx, &["audio", "input"], "");
        let output = self.get_text(cx, &["audio", "output"], "");
        let drift = self.get_bool(cx, &["audio", "driftCorrection"], false);
        let block = self.get_int(cx, &["audio", "block"], 256);
        let rate = self.get_int(cx, &["audio", "rate"], 48_000);
        let offset = self.get_int(cx, &["audio", "latencyFrames"], 0);
        let status = self.status(cx).clone();
        let opened = status.get("device").filter(|d| !d.is_null()).cloned();
        let port = self.scx.core.read(cx).port();
        let project_rate = crate::model::sample_rate(port.root()) as i64;
        let measuring = port.activity().iter().any(|a| a.label.key == "Measuring the latency");
        let recording = port.telemetry().position.recording;
        let device_loading = target_loading(&self.scx, "device-loading", Target::Device, cx);
        let name_of = |id: &str| devices.iter().find(|d| &*d.id == id).map_or(id.to_string(), |d| d.name.to_string());
        let mut rows = Vec::new();

        // the audio systems this build can open (the core says which); with none listed, the current one
        let mut backends: Vec<String> = status
            .get("backends")
            .map(|l| l.items().filter_map(|x| x.as_text().map(str::to_owned)).collect())
            .unwrap_or_default();
        if backends.is_empty() {
            backends.push(system.clone());
        }
        let names: Vec<(SharedString, bool, bool)> = backends
            .iter()
            .map(|b| {
                let label = match b.as_str() {
                    "alsa" => "ALSA",
                    "coreaudio" => "CoreAudio",
                    "jack" => "JACK",
                    other => crate::ui::tr_str(other),
                };
                (label.into(), *b == system, true)
            })
            .collect();
        rows.push(row(
            "Audio system",
            Apply::Now,
            segmented("audio-system", names, cx, move |s, i, _, cx| {
                if let Some(b) = backends.get(i) {
                    s.set(cx, &["audio", "system"], Value::text(b));
                    s.reopen_soon(cx);
                }
            }),
        ));
        // the device's state (I-10)
        let state: AnyElement = if let Some(l) = device_loading {
            div().h(px(28.)).child(l).into_any_element()
        } else if let Some(o) = &opened {
            let r = o.get("rate").and_then(Value::as_int).unwrap_or(48_000);
            let ms =
                |k: &str| format!("{:.1}", o.get(k).and_then(Value::as_int).unwrap_or(0) as f64 * 1000.0 / r as f64);
            let edge = o.get("edgeConverter").and_then(Value::as_bool).unwrap_or(false);
            note(
                trf(
                    "Open: {0} → {1} at {2}, block {3}, {4} ms in, {5} ms out{6}",
                    &[
                        &name_of(&text_of(o, "input")),
                        &name_of(&text_of(o, "output")),
                        &khz(r),
                        &o.get("block").and_then(Value::as_int).unwrap_or(0),
                        &ms("latencyIn"),
                        &ms("latencyOut"),
                        &if edge { tr(", converted at the edge") } else { "" },
                    ],
                ),
                THEME.green,
            )
            .into_any_element()
        } else {
            note(tr("No audio device open"), THEME.red).into_any_element()
        };
        rows.push(wide(
            "Device",
            None,
            div().flex().flex_col().gap(px(4.)).child(state).child(
                div().flex().child(
                    button("scan-devices", tr("Look for devices again"), false, 0)
                        .debug_selector(|| "scan-devices".into())
                        .on_click(cx.listener(|s, _, _, cx| {
                            port_action(&s.scx, "devices.scan", Value::NULL, cx);
                            s.list_devices(cx);
                            cx.notify();
                        })),
                ),
            ),
        ));
        // explicit input and output (U7): the program never joins devices by itself
        let list = |is_input: bool, cx: &mut Context<Self>| {
            let chosen = if is_input { input.clone() } else { output.clone() };
            let sel: &'static str = if is_input { "input-dev" } else { "output-dev" };
            let usable: Vec<&DeviceInfo> =
                devices.iter().filter(|d| if is_input { d.inputs > 0 } else { d.outputs > 0 }).collect();
            let mut items: Vec<(SharedString, bool, bool)> = usable
                .iter()
                .map(|d| {
                    let label = match d.details.get("busyBy").and_then(Value::as_text) {
                        Some(by) => trf("{0} (used by {1})", &[&d.name, &by]),
                        None => d.name.to_string(),
                    };
                    (label.into(), *d.id == *chosen, true)
                })
                .collect();
            let ids: Vec<String> = usable.iter().map(|d| d.id.to_string()).collect();
            if is_input {
                items.push((tr("None").into(), chosen.is_empty(), true));
            }
            segmented(sel, items, cx, move |s, i, _, cx| {
                let id = ids.get(i).cloned().unwrap_or_default();
                s.set(cx, &["audio", if is_input { "input" } else { "output" }], Value::text(id));
                s.reopen_soon(cx);
            })
        };
        rows.push(row("Input device", Apply::Now, list(true, cx)));
        rows.push(row("Output device", Apply::Now, list(false, cx)));
        let different = !input.is_empty() && input != output;
        rows.push(
            row(
                "Correct the drift between cards",
                Apply::Now,
                switch("drift", drift, cx, |s, v, cx| {
                    s.set(cx, &["audio", "driftCorrection"], b(v));
                    s.reopen_soon(cx);
                }),
            )
            .hint(if different && !drift {
                tr("The input and the output are on different cards (different clocks): they only open with this on, or with one card for both")
            } else {
                tr("Only for an input and an output on different cards: an adaptive resampler on the input, which adds latency")
            }),
        );
        if devices.iter().any(|d| *d.id == *input && d.details.get("bluetooth").and_then(Value::as_bool) == Some(true))
        {
            rows.push(wide("Bluetooth microphone", None, note(tr("The microphone adds over 150 ms"), THEME.orange)));
        }
        let out_rates: Vec<u32> =
            devices.iter().find(|d| *d.id == *output).map(|d| d.rates.clone()).unwrap_or_default();
        rows.push(
            row(
                "Sample rate",
                Apply::Now,
                segmented(
                    "device-rate",
                    RATES
                        .iter()
                        .map(|r| {
                            let label: SharedString =
                                if *r == project_rate { trf("{0} (project)", &[&khz(*r)]).into() } else { khz(*r).into() };
                            (label, *r == rate, out_rates.is_empty() || out_rates.contains(&(*r as u32)))
                        })
                        .collect(),
                    cx,
                    |s, i, _, cx| {
                        s.set(cx, &["audio", "rate"], Value::int(RATES[i]));
                        s.reopen_soon(cx);
                    },
                ),
            )
            .hint(if rate != project_rate {
                trf(
                    "The device runs at {0}; the project's {1} is converted at the edge, one high-quality converter each way",
                    &[&khz(rate), &khz(project_rate)],
                )
            } else {
                tr("The device opens at the project's rate; a rate it does not take is greyed").to_string()
            }),
        );
        let out_latency = opened.as_ref().and_then(|o| o.get("latencyOut")).and_then(Value::as_int).unwrap_or(block);
        let ms = |f: i64| format!("{:.1}", f as f64 * 1000.0 / rate.max(1) as f64);
        let mut block_hint =
            trf("{0} ms of buffer; {1} ms of output latency (measured by the device)", &[&ms(block), &ms(out_latency)]);
        if block <= 32 {
            block_hint = format!("{block_hint} · {}", tr("32 frames or fewer: some plugins may not be available"));
        }
        rows.push(
            row(
                "Buffer size",
                Apply::Now,
                segmented(
                    "block",
                    BLOCKS.iter().map(|k| (k.to_string().into(), *k == block, true)).collect(),
                    cx,
                    |s, i, _, cx| {
                        s.set(cx, &["audio", "block"], Value::int(BLOCKS[i]));
                        s.reopen_soon(cx);
                    },
                ),
            )
            .hint(block_hint),
        );
        // latency calibration (I-10.4, H19): the residual per device pair, rate and edge converter
        let pair = format!("{input}|{output}");
        let edge = opened.as_ref().and_then(|o| o.get("edgeConverter")).and_then(Value::as_bool).unwrap_or(false);
        let cals: Vec<Value> =
            self.get(cx, &["audio", "calibrations"]).map(|l| l.items().cloned().collect()).unwrap_or_default();
        let ours: Vec<&Value> = cals.iter().filter(|k| text_of(k, "devices") == pair).collect();
        let calibrated = ours.iter().any(|k| {
            k.get("rate").and_then(Value::as_int) == Some(rate)
                && k.get("edgeConverter").and_then(Value::as_bool) == Some(edge)
        });
        let table = div().flex().flex_col().children(ours.iter().map(|k| {
            let r = k.get("rate").and_then(Value::as_int).unwrap_or(0);
            let res = k.get("residual").and_then(Value::as_int).unwrap_or(0);
            let conv = k.get("edgeConverter").and_then(Value::as_bool) == Some(true);
            div()
                .flex()
                .gap(px(14.))
                .text_size(px(12.))
                .child(div().w(px(70.)).child(khz(r)))
                .child(div().w(px(150.)).child(if conv { tr("with edge converter") } else { tr("without converter") }))
                .child(div().w(px(240.)).child(trf(
                    "{0} frames ({1} ms) beyond the reported",
                    &[&res, &format!("{:.2}", res as f64 * 1000.0 / r.max(1) as f64)],
                )))
                .child(div().text_color(c(THEME.text3)).child(text_of(k, "measured")))
        }));
        rows.push(
            wide(
                "Latency calibration",
                Some(Apply::Now),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(if calibrated {
                        note(tr("Calibrated"), THEME.green)
                    } else {
                        note(tr("Not calibrated"), THEME.orange)
                    })
                    .child(table)
                    .child(
                        div().flex().items_center().gap(px(10.)).child(
                            tbutton(
                                "measure",
                                if measuring { tr("Measuring…") } else { tr("Measure…") },
                                measuring,
                                THEME.green,
                            )
                            .flex_none()
                            .debug_selector(|| "measure".into())
                            .when(!measuring && !recording, |b| {
                                b.on_click(
                                    cx.listener(|s, _, _, cx| port_action(&s.scx, "device.calibrate", Value::NULL, cx)),
                                )
                            }),
                        ),
                    ),
            )
            .hint(tr("Kept per device, rate and edge converter, so it stays right when the buffer size changes")),
        );
        rows.push(
            row(
                "Manual latency offset",
                Apply::Now,
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(stepper("latency-offset", trf("{0} frames", &[&offset]), cx, move |s, d, cx| {
                        s.set(cx, &["audio", "latencyFrames"], Value::int((offset + d * 8).clamp(-48_000, 48_000)))
                    }))
                    .child(caption(trf("{0} ms", &[&format!("{:+.2}", offset as f64 * 1000.0 / rate.max(1) as f64)]))),
            )
            .hint(tr("Added to the calibration, so that recordings line up with what was heard")),
        );
        // real-time permission on Linux (H1)
        let rt = status.get("realtime").cloned().unwrap_or(Value::NULL);
        let rtkit = rt.get("rtkit").and_then(Value::as_bool).unwrap_or(false);
        let rtprio = rt.get("rtprio").and_then(Value::as_int).unwrap_or(0);
        let memlock = rt.get("memlockKb").and_then(Value::as_int).unwrap_or(-1);
        let group = rt.get("audioGroup").and_then(Value::as_bool).unwrap_or(false);
        let user = rt.get("user").and_then(Value::as_text).unwrap_or("$USER").to_string();
        let memlock_ok = !(0..512 * 1024).contains(&memlock);
        let ok = (rtkit || rtprio >= 70) && memlock_ok;
        let line = |ok: bool, text: String| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .child(badge(if ok { tr("ok") } else { tr("missing") }, if ok { THEME.green } else { THEME.red }, true))
                .child(text)
        };
        let memlock_text = if memlock < 0 { tr("unlimited").to_string() } else { format!("{} MB", memlock / 1024) };
        rows.push(wide(
            "Real-time permission (Linux)",
            None,
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(line(
                    rtkit || rtprio >= 70,
                    trf(
                        "Real-time priority: rtkit {0}, RLIMIT_RTPRIO {1}",
                        &[&if rtkit { tr("yes") } else { tr("no") }, &rtprio],
                    ),
                ))
                .child(line(memlock_ok, trf("Locked memory: RLIMIT_MEMLOCK {0}", &[&memlock_text])))
                .child(line(group, trf("{0} in the audio group", &[&user])))
                .when(!ok, |d| {
                    d.child(caption(tr("To configure, then log in again:"))).child(
                        div()
                            .p(px(6.))
                            .rounded(px(4.))
                            .bg(c(THEME.lcd_bg))
                            .text_size(px(11.))
                            .font_family("Menlo")
                            .flex()
                            .flex_col()
                            .child(format!("sudo usermod -aG audio {user}"))
                            .child("echo '@audio - rtprio 95' | sudo tee -a /etc/security/limits.d/audio.conf")
                            .child("echo '@audio - memlock unlimited' | sudo tee -a /etc/security/limits.d/audio.conf"),
                    )
                }),
        ));
        rows
    }

    fn midi(&mut self, cx: &mut Context<Self>) -> Vec<Row> {
        let ports: Vec<Value> =
            self.get(cx, &["midi", "ports"]).map(|l| l.items().cloned().collect()).unwrap_or_default();
        let state: Vec<Value> =
            self.status(cx).get("midiPorts").map(|l| l.items().cloned().collect()).unwrap_or_default();
        let scan_loading = target_loading(&self.scx, "midi-scan", Target::Scan(lm_port::ScanKind::MidiPorts), cx);
        let same = |a: &Value, b: &Value| {
            text_of(a, "client") == text_of(b, "client") && text_of(a, "port") == text_of(b, "port")
        };
        // the ports this machine knows (preferences), with their state now (status), by client and port name
        let mut lines: Vec<AnyElement> = Vec::new();
        for (i, p) in ports.iter().enumerate() {
            let connected = state
                .iter()
                .find(|x| same(x, p))
                .and_then(|x| x.get("connected"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let role = text_of(p, "role");
            let lat = p.get("inputLatencyMs").and_then(Value::as_real).unwrap_or(0.0);
            let out_lat = p.get("outputLatencyMs").and_then(Value::as_real).unwrap_or(0.0);
            let alias = self.aliases.get(i).cloned();
            lines.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .py(px(4.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(
                        div()
                            .w(px(200.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .gap(px(6.))
                                    .items_center()
                                    .child(div().text_size(px(12.)).child(text_of(p, "client")))
                                    .child(if connected {
                                        badge(tr("connected"), THEME.green, false)
                                    } else {
                                        badge(tr("disconnected"), THEME.text3, false)
                                    }),
                            )
                            .child(caption(format!("{} · {}", text_of(p, "port"), text_of(p, "backend")))),
                    )
                    .child(div().w(px(110.)).flex_none().children(alias.map(|a| crate::ui::text_entry(Input::new(&a)))))
                    .child(switch(
                        format!("midi-in-{i}"),
                        p.get("input").and_then(Value::as_bool).unwrap_or(false),
                        cx,
                        move |s, v, cx| s.set_in(cx, &["midi", "ports"], i, "input", b(v)),
                    ))
                    .child(caption(tr("in")))
                    .child(switch(
                        format!("midi-out-{i}"),
                        p.get("output").and_then(Value::as_bool).unwrap_or(false),
                        cx,
                        move |s, v, cx| s.set_in(cx, &["midi", "ports"], i, "output", b(v)),
                    ))
                    .child(caption(tr("out")))
                    .child(segmented(
                        [
                            "midi-role-0",
                            "midi-role-1",
                            "midi-role-2",
                            "midi-role-3",
                            "midi-role-4",
                            "midi-role-5",
                            "midi-role-6",
                            "midi-role-7",
                        ][i % 8],
                        vec![
                            (tr("Instrument").into(), role == "instrument", true),
                            (tr("Controller").into(), role == "controller", true),
                            (tr("Both").into(), role == "both", true),
                        ],
                        cx,
                        move |s, k, _, cx| {
                            s.set_in(
                                cx,
                                &["midi", "ports"],
                                i,
                                "role",
                                Value::text(["instrument", "controller", "both"][k]),
                            )
                        },
                    ))
                    .child(stepper(format!("midi-lat-{i}"), trf("in {0} ms", &[&lat]), cx, move |s, d, cx| {
                        s.set_in(cx, &["midi", "ports"], i, "inputLatencyMs", real((lat + d as f64).clamp(0.0, 100.0)))
                    }))
                    .child(stepper(format!("midi-mpe-{i}"), trf("MPE {0}", &[&settings_mpe::zone(p)]), cx, {
                        let port = p.clone();
                        move |s, d, cx| {
                            s.set_in(cx, &["midi", "ports"], i, "mpe", Value::text(settings_mpe::step_zone(&port, d)))
                        }
                    }))
                    .children((settings_mpe::zone(p) != "off").then(|| {
                        let members = settings_mpe::members(p);
                        stepper(format!("midi-mpe-members-{i}"), trf("{0} ch", &[&members]), cx, move |s, d, cx| {
                            s.set_in(cx, &["midi", "ports"], i, "mpeMembers", Value::int((members + d).clamp(1, 15)))
                        })
                    }))
                    .child(stepper(format!("midi-olat-{i}"), trf("out {0} ms", &[&out_lat]), cx, move |s, d, cx| {
                        s.set_in(
                            cx,
                            &["midi", "ports"],
                            i,
                            "outputLatencyMs",
                            real((out_lat + d as f64).clamp(0.0, 100.0)),
                        )
                    }))
                    .into_any_element(),
            );
        }
        // ports present that this machine does not know yet: one click adds them
        let new_ports: Vec<Value> = state.iter().filter(|x| !ports.iter().any(|p| same(p, x))).cloned().collect();
        let new_lines = new_ports.iter().enumerate().map(|(k, x)| {
            let entry = obj(vec![
                ("client", Value::text(text_of(x, "client"))),
                ("port", Value::text(text_of(x, "port"))),
                ("backend", Value::text(text_of(x, "backend"))),
                ("alias", Value::text("")),
                ("role", Value::text("instrument")),
                ("input", Value::bool(true)),
                ("output", Value::bool(false)),
                ("inputLatencyMs", real(0.0)),
                ("outputLatencyMs", real(0.0)),
            ]);
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(3.))
                .child(div().w(px(200.)).text_size(px(12.)).text_color(c(THEME.text2)).child(text_of(x, "client")))
                .child(caption(text_of(x, "port")))
                .child(
                    button(("midi-add", k), tr("Use this port"), false, 0)
                        .debug_selector(move || format!("midi-add-{k}"))
                        .on_click(cx.listener(move |s, _, _, cx| {
                            let mut all: Vec<Value> =
                                s.get(cx, &["midi", "ports"]).map(|l| l.items().cloned().collect()).unwrap_or_default();
                            all.push(entry.clone());
                            s.set(cx, &["midi", "ports"], Value::list(all));
                        })),
                )
        });
        let profiles: Vec<Value> =
            self.get(cx, &["controllers"]).map(|l| l.items().cloned().collect()).unwrap_or_default();
        let aliases: Vec<String> = ports.iter().map(|p| text_of(p, "alias")).filter(|a| !a.is_empty()).collect();
        let osc_on = self.get_bool(cx, &["osc", "on"], false);
        let osc_port = self.get_int(cx, &["osc", "port"], 9000);
        let osc_lan = self.get_bool(cx, &["osc", "lan"], false);
        let profile_lines = profiles.iter().enumerate().map(|(i, p)| {
            let profile = text_of(p, "profile");
            let bank = p.get("bank").and_then(Value::as_int).unwrap_or(0);
            let name = match profile.as_str() {
                "mcu" => tr("Mackie Control Universal").to_string(),
                "generic" => tr("Generic (editable)").to_string(),
                other => other.to_string(),
            };
            let port = text_of(p, "port");
            let choices = aliases.clone();
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().w(px(200.)).text_size(px(12.)).child(name))
                .child(segmented(
                    ["ctl-port-0", "ctl-port-1", "ctl-port-2", "ctl-port-3"][i % 4],
                    choices.iter().map(|a| (a.clone().into(), *a == port, true)).collect(),
                    cx,
                    move |s, k, _, cx| {
                        let a = choices.get(k).cloned().unwrap_or_default();
                        s.set_in(cx, &["controllers"], i, "port", Value::text(a));
                    },
                ))
                .child(stepper(
                    format!("ctl-bank-{i}"),
                    trf("channels {0}–{1}", &[&(bank * 8 + 1), &(bank * 8 + 8)]),
                    cx,
                    move |s, d, cx| s.set_in(cx, &["controllers"], i, "bank", Value::int((bank + d).max(0))),
                ))
        });
        vec![
            wide(
                "Ports",
                Some(Apply::Now),
                div()
                    .flex()
                    .flex_col()
                    .children(lines)
                    .children(new_lines)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .pt(px(4.))
                            .children(scan_loading.map(|l| div().h(px(24.)).w(px(300.)).child(l)))
                            .child(button("midi-scan", tr("Look for devices again"), false, 0).on_click(cx.listener(|s, _, _, cx| {
                                port_action(&s.scx, "devices.scan", Value::NULL, cx)
                            }))),
                    ),
            )
            .hint(tr(
                "A port is its client's and port's names, never ALSA's numbers; the project keeps only the aliases, and asks once for the port of an alias it does not know. A port that comes back reconnects by itself, with a notice",
            )),
            wide("Controllers", Some(Apply::Now), div().flex().flex_col().gap(px(3.)).children(profile_lines))
                .hint(tr("A profile maps a controller's faders, knobs and buttons, with the return to its lights and motorized faders; banks of 8")),
            row("OSC", Apply::Now, switch("osc-on", osc_on, cx, |s, v, cx| s.set(cx, &["osc", "on"], b(v)))),
            row(
                "OSC port",
                Apply::Now,
                stepper("osc-port", format!("{osc_port}"), cx, move |s, d, cx| {
                    s.set(cx, &["osc", "port"], Value::int((osc_port + d).clamp(1024, 65_535)))
                }),
            ),
            row("OSC from other machines", Apply::Now, switch("osc-lan", osc_lan, cx, |s, v, cx| s.set(cx, &["osc", "lan"], b(v)))),
        ]
    }

    fn recording(&mut self, cx: &mut Context<Self>) -> Vec<Row> {
        let when = self.get_text(cx, &["metronome", "when"], "recording");
        let count_in = self.get_int(cx, &["metronome", "countInBars"], 1);
        let format = self.get_text(cx, &["recording", "format"], "rf64-24");
        vec![
            row(
                "Metronome",
                Apply::Now,
                segmented(
                    "metro-when",
                    vec![
                        (tr("Never").into(), when == "never", true),
                        (tr("Only while recording").into(), when == "recording", true),
                        (tr("Always").into(), when == "always", true),
                    ],
                    cx,
                    |s, i, _, cx| s.set(cx, &["metronome", "when"], Value::text(["never", "recording", "always"][i])),
                ),
            ),
            row(
                "Count-in",
                Apply::Now,
                stepper("count-in", trf("{0} bars", &[&count_in]), cx, move |s, d, cx| {
                    s.set(cx, &["metronome", "countInBars"], Value::int((count_in + d).clamp(0, 4)))
                }),
            ),
            self.toggle("Takes in lanes", "lanes", &["recording", "lanes"], true, cx).hint(tr(
                "Each pass on a lane of its own, the latest on top; off: the simple recorder, the new take replaces what it covers",
            )),
            row(
                "Recording format",
                Apply::Now,
                segmented(
                    "rec-format",
                    vec![("RF64 24 bits".into(), format == "rf64-24", true), ("RF64 32 bits float".into(), format == "rf64-32f", true)],
                    cx,
                    |s, i, _, cx| s.set(cx, &["recording", "format"], Value::text(["rf64-24", "rf64-32f"][i])),
                ),
            )
            .hint(tr("RF64 survives a crash: the take is there up to the last second")),
        ]
    }

    fn plugins(&mut self, cx: &mut Context<Self>) -> Vec<Row> {
        // plugins that brought a scan or a probe down: (format, uri); `plugin.unblock` lifts one, the next scan tries it again
        let blocked: Vec<(String, String)> = self
            .get(cx, &["plugins", "blocked"])
            .map(|l| {
                l.items()
                    .map(|b| {
                        let t = |k: &str| b.get(k).and_then(Value::as_text).unwrap_or("").to_string();
                        (t("format"), t("uri"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let lines: Vec<_> = blocked
            .iter()
            .enumerate()
            .map(|(i, (format, uri))| {
                let (f, u) = (format.clone(), uri.clone());
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().w(px(60.)).text_size(px(11.)).text_color(c(THEME.text2)).child(format.to_uppercase()))
                    .child(div().w(px(300.)).text_size(px(12.)).child(uri.clone()))
                    .child(button(("unblock", i), tr("Unblock"), false, 0).on_click(cx.listener(move |s, _, _, cx| {
                        port_action(
                            &s.scx,
                            "plugin.unblock",
                            obj(vec![("format", Value::text(&f)), ("uri", Value::text(&u))]),
                            cx,
                        );
                    })))
            })
            .collect();
        vec![
            self.toggle("Plugin windows on top", "plugin-top", &["plugins", "windowsOnTop"], false, cx),
            wide(
                "Blocked plugins",
                Some(Apply::Now),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .when(blocked.is_empty(), |d| d.child(caption(tr("None"))))
                    .children(lines)
                    .when(!blocked.is_empty(), |d| {
                        d.child(button("unblock-all", tr("Unblock all"), false, 0).on_click(cx.listener(
                            |s, _, _, cx| {
                                port_action(&s.scx, "plugin.unblock", obj(vec![]), cx);
                            },
                        )))
                    }),
            ),
        ]
    }

    fn folders(&mut self, cx: &mut Context<Self>) -> Vec<Row> {
        let allowed = self.list_at(cx, &["folders", "allowed"]);
        let lines: Vec<_> = allowed
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let f2 = f.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().w(px(360.)).text_size(px(12.)).font_family("Menlo").child(f.clone()))
                    .child(button(("allowed", i), tr("Remove"), false, 0).on_click(cx.listener(move |s, _, _, cx| {
                        let rest: Vec<Value> = s
                            .list_at(cx, &["folders", "allowed"])
                            .into_iter()
                            .filter(|x| *x != f2)
                            .map(Value::text)
                            .collect();
                        s.set(cx, &["folders", "allowed"], Value::list(rest));
                    })))
            })
            .collect();
        vec![
            row("Working folder", Apply::Restart, div().w(px(360.)).child(crate::ui::text_entry(Input::new(&self.projects_dir)))),
            row("Default SoundFont", Apply::Now, div().w(px(360.)).child(crate::ui::text_entry(Input::new(&self.soundfont)))),
            wide(
                "Folders a project may read",
                Some(Apply::Now),
                div().flex().flex_col().gap(px(3.)).children(lines).child(
                    div().flex().gap(px(6.)).child(div().w(px(360.)).child(crate::ui::text_entry(Input::new(&self.new_folder)))).child(
                        button("add-allowed", tr("Allow"), false, 0)
                            .debug_selector(|| "add-allowed".into())
                            .on_click(cx.listener(|s, _, _, cx| s.add_folder(cx))),
                    ),
                ),
            )
            .hint(tr(
                "Files outside a project (SoundFonts, kits of the system) are read only inside these folders; anything else shows as missing, with \"Allow this folder\"",
            )),
        ]
    }

    fn appearance(&mut self, cx: &mut Context<Self>) -> Vec<Row> {
        let theme = self.get_text(cx, &["theme"], "dark");
        let lang = self.get_text(cx, &["language"], "pt-BR");
        vec![
            row(
                "Theme",
                Apply::Restart,
                segmented(
                    "theme",
                    vec![(tr("Dark").into(), theme == "dark", true), (tr("Light").into(), theme == "light", true)],
                    cx,
                    |s, i, _, cx| s.set(cx, &["theme"], Value::text(["dark", "light"][i])),
                ),
            )
            .hint(tr("A theme is a text file of colors (themes/*.txt)")),
            row(
                "Language",
                Apply::Restart,
                segmented(
                    "language",
                    vec![("Português (Brasil)".into(), lang == "pt-BR", true), ("English".into(), lang == "en", true)],
                    cx,
                    |s, i, _, cx| s.set(cx, &["language"], Value::text(["pt-BR", "en"][i])),
                ),
            ),
            self.toggle("Follow the cursor while playing", "follow", &["interface", "followCursor"], false, cx),
            self.toggle("Tooltips", "tooltips", &["interface", "tooltips"], true, cx),
            self.toggle("Note names in the Piano Roll", "note-names", &["interface", "noteNames"], true, cx),
        ]
    }

    fn autosave(&mut self, cx: &mut Context<Self>) -> Vec<Row> {
        let minutes = self.get_int(cx, &["backup", "autosaveMinutes"], 2);
        let kept = self.get_int(cx, &["backup", "kept"], 10);
        let every = if minutes == 0 { tr("off").to_string() } else { trf("every {0} min", &[&minutes]) };
        vec![
            row(
                "Autosave",
                Apply::Now,
                stepper("autosave", every, cx, move |s, d, cx| {
                    s.set(cx, &["backup", "autosaveMinutes"], Value::int((minutes + d).clamp(0, 30)))
                }),
            ),
            row(
                "Copies kept",
                Apply::Now,
                stepper("backups-kept", kept.to_string(), cx, move |s, d, cx| {
                    s.set(cx, &["backup", "kept"], Value::int((kept + d).clamp(1, 100)))
                }),
            ),
            self.toggle("Keep a backup when saving", "backup-save", &["backup", "onSave"], true, cx),
        ]
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = modal_size(window, 1040.0, 720.0);
        let query = self.search.read(cx).value().to_string();
        let searching = !query.trim().is_empty();
        let pending = self.pending_restart(cx);
        let q = fold(query.trim());

        // the page's rows, or every page's rows that match the search (labels, hints, page titles)
        let mut sections: Vec<(Page, Vec<Row>)> = Vec::new();
        if searching {
            for p in Page::ALL {
                let title = fold(tr(p.title()));
                let rows: Vec<Row> = self
                    .rows(p, cx)
                    .into_iter()
                    .filter(|r| {
                        title.contains(&q)
                            || fold(tr(r.label)).contains(&q)
                            || r.hint.as_deref().is_some_and(|h| fold(h).contains(&q))
                    })
                    .collect();
                if !rows.is_empty() {
                    sections.push((p, rows));
                }
            }
        } else {
            let page = self.page;
            sections.push((page, self.rows(page, cx)));
        }
        let found = sections.iter().map(|(_, r)| r.len()).sum::<usize>();

        let render_row = |r: Row| {
            let hint: Option<SharedString> = r.hint.map(Into::into);
            let label = div()
                .id(r.label)
                .when_some(hint, |d, h| {
                    d.tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(h.clone()).build(window, cx))
                })
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().text_size(px(13.)).child(tr(r.label)))
                .children(r.apply.and_then(apply_badge));
            if r.wide {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(label)
                    .child(r.control)
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(div().w(px(270.)).flex_none().child(label))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(3.)).child(r.control))
            }
        };

        let content = div()
            .id("settings-content")
            .flex_1()
            .overflow_y_scroll()
            .px(px(22.))
            .py(px(16.))
            .flex()
            .flex_col()
            .children(sections.into_iter().map(|(p, rows)| {
                div()
                    .flex()
                    .flex_col()
                    .mb(px(14.))
                    .child(div().text_size(px(18.)).font_weight(FontWeight::BOLD).child(tr(p.title())))
                    .children(rows.into_iter().map(render_row))
            }))
            .when(searching && found == 0, |d| {
                d.child(div().text_color(c(THEME.text3)).child(trf("No setting has \"{0}\"", &[&query])))
            });

        // design 08's band: what waits for a restart
        let band = (!pending.is_empty() && !self.band_hidden).then(|| {
            let names = pending.iter().map(|k| tr(k)).collect::<Vec<_>>().join(", ");
            div()
                .id("restart-band")
                .debug_selector(|| "restart-band".into())
                .mx(px(22.))
                .mb(px(14.))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(14.))
                .py(px(8.))
                .rounded(px(8.))
                .border_1()
                .border_color(c(THEME.yellow))
                .bg(c(THEME.bg3))
                .child(div().text_color(c(THEME.yellow)).child("⟳"))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .child(trf("{0}: changes after the program restarts.", &[&names])),
                )
                .child(button("later", tr("Later"), false, 0).flex_none().on_click(cx.listener(|s, _, _, cx| {
                    s.band_hidden = true;
                    cx.notify();
                })))
                .child(
                    tbutton("restart-now", tr("Restart now"), true, THEME.green)
                        .flex_none()
                        .on_click(cx.listener(|s, _, _, cx| s.restart(cx))),
                )
        });

        let nav = div()
            .w(px(220.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(12.))
            .bg(c(THEME.bg1))
            .border_r_1()
            .border_color(c(THEME.line))
            .child(div().mb(px(8.)).child(crate::ui::text_entry(Input::new(&self.search))))
            .children(Page::ALL.iter().map(|&p| {
                let on = p == self.page && !searching;
                div()
                    .id(("settings-page", p as usize))
                    .debug_selector(move || format!("settings-page-{p:?}"))
                    .px(px(12.))
                    .py(px(6.))
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
            .child(
                button("close-settings", tr("Close"), false, 0).on_click(cx.listener(|s, _, _, cx| s.scx.close(cx))),
            );

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
            // keymap-ok: the key capture
            .on_key_down(cx.listener(Self::on_key))
            .child(nav)
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().child(content).children(band))
    }
}
