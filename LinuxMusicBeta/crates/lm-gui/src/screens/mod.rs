//! The registry of screens other than the two pages. Each screen is its own GPUI view (an entity) in its own file,
//! made by `open` and placed by the workspace by its `placement`: a dock under the page, a side dock, or a modal.
//! A screen gets a `ScreenCx`: the core (through its port only), the workspace (to close itself or open another
//! screen) and, when it edits one, its target node (the item of the Piano Roll, the plugin of a window).
//!
//! Owners (docs/prototipo-interface.md): A = shell, pages, routing; B = piano roll, automation, steps; C = instrument,
//! effect, browser; D = settings, export, project, notices, palette, new project, about. Each ports its screens from
//! prototypes/gpui to the real tree (model.rs) and the port. To add a screen: a variant here, its line in `title`,
//! `placement` and `open`, and its file. ONE LINE EACH.

use gpui::{AnyView, App, AppContext, Entity, WeakEntity, Window};
use lm_port::NodeId;

use crate::core::Core;
use crate::workspace::Workspace;

pub mod about;
pub mod automation;
pub mod browser;
pub mod effect;
pub mod export;
pub mod files;
pub mod instrument;
pub mod live;
pub mod manual;
pub mod midi_binding;
pub mod new_project;
pub mod notices;
pub mod palette;
pub mod piano_roll;
mod piano_roll_bars;
pub mod project;
pub mod routing;
pub mod settings;
pub mod steps;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    PianoRoll,
    Automation,
    Steps,
    Instrument,
    Effect,
    Browser,
    Routing,
    Settings,
    Export,
    Project,
    Notices,
    Palette,
    NewProject,
    About,
    MidiBinding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// under the page (the editors)
    Bottom,
    /// beside the page
    Left,
    Right,
    /// over the window, centered
    Modal,
}

/// What a screen receives: the core, the workspace (to close itself or open another screen), its target node
#[derive(Clone)]
pub struct ScreenCx {
    pub core: Entity<Core>,
    pub workspace: WeakEntity<Workspace>,
    pub screen: Screen,
    pub target: Option<NodeId>,
    /// The control the MIDI binding window shows (I-21.2); none for the other screens
    pub binding: Option<lm_port::MapTarget>,
}

impl ScreenCx {
    pub fn close(&self, cx: &mut App) {
        let s = self.screen;
        let _ = self.workspace.update(cx, |ws, cx| ws.close(s, cx));
    }
    pub fn open(&self, other: Screen, target: Option<NodeId>, window: &mut Window, cx: &mut App) {
        let _ = self.workspace.update(cx, |ws, cx| ws.open(other, target, window, cx));
    }
}

impl Screen {
    pub const ALL: [Screen; 15] = [
        Screen::PianoRoll,
        Screen::Automation,
        Screen::Steps,
        Screen::Instrument,
        Screen::Effect,
        Screen::Browser,
        Screen::Routing,
        Screen::Settings,
        Screen::Export,
        Screen::Project,
        Screen::Notices,
        Screen::Palette,
        Screen::NewProject,
        Screen::About,
        Screen::MidiBinding,
    ];

    /// The English key of the title (translated by `ui::tr`)
    pub fn title(self) -> &'static str {
        match self {
            Screen::PianoRoll => "Piano Roll",
            Screen::Automation => "Automation",
            Screen::Steps => "Step editor",
            Screen::Instrument => "Instrument",
            Screen::Effect => "Effect",
            Screen::Browser => "Browser",
            Screen::Routing => "Routing",
            Screen::Settings => "Settings",
            Screen::Export => "Export",
            Screen::Project => "Project",
            Screen::Notices => "Notices",
            Screen::Palette => "Command palette",
            Screen::NewProject => "New project",
            Screen::About => "About",
            Screen::MidiBinding => "MIDI binding",
        }
    }

    /// The legacy's icon for the screen (its window's or its toolbar button's)
    pub fn icon(self) -> &'static str {
        match self {
            Screen::PianoRoll => "piano",
            Screen::Automation => "automation",
            Screen::Steps => "pattern_track",
            Screen::Instrument => "instrument_track",
            Screen::Effect => "effect_plugin",
            Screen::Browser => "plugins",
            Screen::Routing => "mixer",
            Screen::Settings => "setup_general",
            Screen::Export => "project_export",
            Screen::Project => "project_open",
            Screen::Notices => "error",
            Screen::Palette => "setup_shortcuts",
            Screen::NewProject => "project_new",
            Screen::About => "help",
            Screen::MidiBinding => "setup_midi",
        }
    }

    pub fn placement(self) -> Placement {
        match self {
            Screen::PianoRoll | Screen::Automation | Screen::Steps => Placement::Bottom,
            Screen::Browser => Placement::Left,
            // the effect window is the legacy's effects rack, a column beside the page
            Screen::Instrument | Screen::Effect | Screen::Notices => Placement::Right,
            Screen::Routing
            | Screen::Settings
            | Screen::Export
            | Screen::Project
            | Screen::Palette
            | Screen::NewProject
            | Screen::About
            | Screen::MidiBinding => Placement::Modal,
        }
    }
}

/// Makes the screen's view
pub fn open(scx: ScreenCx, window: &mut Window, cx: &mut App) -> AnyView {
    match scx.screen {
        Screen::PianoRoll => cx.new(|cx| piano_roll::PianoRoll::new(scx, window, cx)).into(),
        Screen::Automation => cx.new(|cx| automation::AutomationEditor::new(scx, window, cx)).into(),
        Screen::Steps => cx.new(|cx| steps::StepEditor::new(scx, window, cx)).into(),
        Screen::Instrument => cx.new(|cx| instrument::InstrumentWindow::new(scx, window, cx)).into(),
        Screen::Effect => cx.new(|cx| effect::EffectWindow::new(scx, window, cx)).into(),
        Screen::Browser => cx.new(|cx| browser::Browser::new(scx, window, cx)).into(),
        Screen::Routing => cx.new(|cx| routing::Routing::new(scx, window, cx)).into(),
        Screen::Settings => cx.new(|cx| settings::Settings::new(scx, window, cx)).into(),
        Screen::Export => cx.new(|cx| export::Export::new(scx, window, cx)).into(),
        Screen::Project => cx.new(|cx| project::ProjectDialog::new(scx, window, cx)).into(),
        Screen::Notices => cx.new(|cx| notices::Notices::new(scx, window, cx)).into(),
        Screen::Palette => cx.new(|cx| palette::Palette::new(scx, window, cx)).into(),
        Screen::NewProject => cx.new(|cx| new_project::NewProject::new(scx, window, cx)).into(),
        Screen::About => cx.new(|cx| about::About::new(scx, window, cx)).into(),
        Screen::MidiBinding => cx.new(|cx| midi_binding::MidiBinding::new(scx, window, cx)).into(),
    }
}
