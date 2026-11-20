//! The interface's action registry (I-19): every command the user can give, with its id, label, group, default keys
//! and where the keys work. The keybindings are built from it (with the user's changes from the preferences'
//! `shortcuts`), the command palette lists it, the menus will read it. Commands of the core go through
//! `CorePort::run_action`; the others are the interface's own.
//!
//! Keys are written with `secondary` for Cmd on macOS and Ctrl elsewhere; this GPUI's parser has no `secondary`, so
//! `keys_for_platform` writes it out.

use std::rc::Rc;

use gpui::{App, KeyBinding, KeyBindingContextPredicate, actions};
use lm_port::{CorePort, PathStep, Value};

actions!(
    lm,
    [
        ShowComposition,
        ShowMixing,
        ShowMidiMaps,
        NextPage,
        TogglePlay,
        Stop,
        Record,
        Rewind,
        OpenPalette,
        OpenSettings,
        CloseModal,
        Undo,
        Redo,
        Save,
        Quit,
        ToggleLoop,
        // agent D's screens
        OpenExport,
        OpenProject,
        SaveAs,
        OpenNewProject,
        OpenAbout,
        OpenNotices,
    ]
);

actions!(
    composition,
    [
        Split, Duplicate, DeleteItem, EmptyItem, OpenItem, ZoomIn, ZoomOut, AddMarker, AddRegion, Marker1, Marker2,
        Marker3, Marker4, Marker5, Marker6, Marker7, Marker8, Marker9
    ]
);

actions!(
    mixing,
    [
        SelectLeft,
        SelectRight,
        FaderUp,
        FaderDown,
        FaderUpCoarse,
        FaderDownCoarse,
        FaderUpFine,
        FaderDownFine,
        AddChannel,
        MoveChannelLeft,
        MoveChannelRight,
        RemoveChannel,
        RenameChannel,
    ]
);

actions!(
    piano_roll,
    [
        PrPlay,
        PrModeDraw,
        PrModeErase,
        PrModeSelect,
        PrModePitch,
        PrModeKnife,
        PrModeStrum,
        PrGlue,
        PrFill,
        PrCutOverlaps,
        PrReverse,
        PrSelectAll,
        PrSelectNone,
        PrCopy,
        PrCut,
        PrPaste,
        PrDelete,
        PrTransposeUp,
        PrTransposeDown,
        PrOctaveUp,
        PrOctaveDown,
        PrMoveLeft,
        PrMoveRight,
        PrMoveBarLeft,
        PrMoveBarRight
    ]
);

actions!(
    browser,
    [BrowserUp, BrowserDown, BrowserSend, BrowserOpenFolder, BrowserCloseFolder, BrowserRefresh, BrowserFind]
);

/// One entry of the registry
#[derive(Clone, Copy, Debug)]
pub struct ActionDef {
    /// The registry's id (I-19), stable: the preferences' `shortcuts` use it
    pub id: &'static str,
    /// GPUI's name of the action type (`lm::TogglePlay`)
    pub gpui: &'static str,
    /// English key, translated by `ui::tr`
    pub label: &'static str,
    pub group: &'static str,
    pub keys: &'static [&'static str],
    /// Where the keys work: `None` everywhere, or a key context ("Pages", "Composition", "Mixing")
    pub context: Option<&'static str>,
}

const fn a(
    id: &'static str,
    gpui: &'static str,
    label: &'static str,
    group: &'static str,
    keys: &'static [&'static str],
    context: Option<&'static str>,
) -> ActionDef {
    ActionDef { id, gpui, label, group, keys, context }
}

pub const REGISTRY: &[ActionDef] = &[
    a("view.composition", "lm::ShowComposition", "Composition", "View", &["secondary-1"], None),
    a("view.mixing", "lm::ShowMixing", "Mixing", "View", &["secondary-2"], None),
    a("view.midi_maps", "lm::ShowMidiMaps", "MIDI maps", "View", &["secondary-3"], None),
    a("view.next_page", "lm::NextPage", "Next page", "View", &["ctrl-tab"], None),
    a("view.palette", "lm::OpenPalette", "Command palette", "View", &["secondary-k"], None),
    a("view.settings", "lm::OpenSettings", "Settings", "View", &["secondary-,"], None),
    a("view.close", "lm::CloseModal", "Close", "View", &["escape"], None),
    a("transport.toggle", "lm::TogglePlay", "Play or stop", "Transport", &["space"], Some("Pages")),
    a("transport.stop", "lm::Stop", "Stop", "Transport", &["shift-space"], Some("Pages")),
    a("transport.record", "lm::Record", "Record", "Transport", &["r"], Some("Pages")),
    a("transport.rewind", "lm::Rewind", "Rewind", "Transport", &["home"], Some("Pages")),
    a("transport.loop", "lm::ToggleLoop", "Loop", "Transport", &["l"], Some("Pages")),
    a("edit.undo", "lm::Undo", "Undo", "Edit", &["secondary-z"], None),
    a("edit.redo", "lm::Redo", "Redo", "Edit", &["secondary-shift-z", "ctrl-y"], None),
    a("project.save", "lm::Save", "Save", "Project", &["secondary-s"], None),
    a("app.quit", "lm::Quit", "Quit", "Project", &["secondary-q"], None),
    a("file.open", "lm::OpenProject", "Open project", "Project", &["secondary-o"], None),
    a("file.save_as", "lm::SaveAs", "Save as", "Project", &["secondary-shift-s"], None),
    a("file.new", "lm::OpenNewProject", "New project", "Project", &["secondary-n"], None),
    a("file.export", "lm::OpenExport", "Export audio", "Project", &["secondary-e"], None),
    a("view.notices", "lm::OpenNotices", "Notices and activities", "View", &[], None),
    a("help.about", "lm::OpenAbout", "About LinuxMusic", "Help", &["f1"], None),
    a("arrange.split", "composition::Split", "Split at the cursor", "Arrange", &["s"], Some("Composition")),
    a("arrange.duplicate", "composition::Duplicate", "Duplicate", "Arrange", &["secondary-d"], Some("Composition")),
    a("arrange.delete", "composition::DeleteItem", "Delete", "Arrange", &["delete", "backspace"], Some("Composition")),
    a("arrange.empty_item", "composition::EmptyItem", "Empty item", "Arrange", &["shift-e"], Some("Composition")),
    a(
        "arrange.open_item",
        "composition::OpenItem",
        "Open the item's editor",
        "Arrange",
        &["enter"],
        Some("Composition"),
    ),
    a("arrange.zoom_in", "composition::ZoomIn", "Zoom in", "Arrange", &["="], Some("Composition")),
    a("arrange.zoom_out", "composition::ZoomOut", "Zoom out", "Arrange", &["-"], Some("Composition")),
    a(
        "marks.add_marker",
        "composition::AddMarker",
        "Add a marker at the cursor",
        "Arrange",
        &["m"],
        Some("Composition"),
    ),
    a(
        "marks.add_region",
        "composition::AddRegion",
        "Add a region over the loop",
        "Arrange",
        &["shift-r"],
        Some("Composition"),
    ),
    a("marks.jump_1", "composition::Marker1", "Go to marker 1", "Arrange", &["1"], Some("Composition")),
    a("marks.jump_2", "composition::Marker2", "Go to marker 2", "Arrange", &["2"], Some("Composition")),
    a("marks.jump_3", "composition::Marker3", "Go to marker 3", "Arrange", &["3"], Some("Composition")),
    a("marks.jump_4", "composition::Marker4", "Go to marker 4", "Arrange", &["4"], Some("Composition")),
    a("marks.jump_5", "composition::Marker5", "Go to marker 5", "Arrange", &["5"], Some("Composition")),
    a("marks.jump_6", "composition::Marker6", "Go to marker 6", "Arrange", &["6"], Some("Composition")),
    a("marks.jump_7", "composition::Marker7", "Go to marker 7", "Arrange", &["7"], Some("Composition")),
    a("marks.jump_8", "composition::Marker8", "Go to marker 8", "Arrange", &["8"], Some("Composition")),
    a("marks.jump_9", "composition::Marker9", "Go to marker 9", "Arrange", &["9"], Some("Composition")),
    a("desk.select_left", "mixing::SelectLeft", "Previous channel", "Desk", &["left"], Some("Mixing")),
    a("desk.select_right", "mixing::SelectRight", "Next channel", "Desk", &["right"], Some("Mixing")),
    a("desk.fader_up", "mixing::FaderUp", "Fader up 1 dB", "Desk", &["up", "="], Some("Mixing")),
    a("desk.fader_down", "mixing::FaderDown", "Fader down 1 dB", "Desk", &["down", "-"], Some("Mixing")),
    a("desk.fader_up_coarse", "mixing::FaderUpCoarse", "Fader up 3 dB", "Desk", &["shift-up"], Some("Mixing")),
    a("desk.fader_down_coarse", "mixing::FaderDownCoarse", "Fader down 3 dB", "Desk", &["shift-down"], Some("Mixing")),
    a("desk.fader_up_fine", "mixing::FaderUpFine", "Fader up 0.1 dB", "Desk", &["secondary-up"], Some("Mixing")),
    a("desk.fader_down_fine", "mixing::FaderDownFine", "Fader down 0.1 dB", "Desk", &["secondary-down"], Some("Mixing")),
    a("desk.add_channel", "mixing::AddChannel", "Add a channel", "Desk", &["shift-insert"], Some("Mixing")),
    a("notes.play", "piano_roll::PrPlay", "Play/pause current clip", "Notes", &["space"], Some("PianoRoll")),
    a("notes.mode_draw", "piano_roll::PrModeDraw", "Draw mode", "Notes", &["shift-d"], Some("PianoRoll")),
    a("notes.mode_erase", "piano_roll::PrModeErase", "Erase mode", "Notes", &["shift-e"], Some("PianoRoll")),
    a("notes.mode_select", "piano_roll::PrModeSelect", "Select mode", "Notes", &["shift-s"], Some("PianoRoll")),
    a("notes.mode_pitch", "piano_roll::PrModePitch", "Pitch Bend mode", "Notes", &["shift-t"], Some("PianoRoll")),
    a("notes.mode_knife", "piano_roll::PrModeKnife", "Knife", "Notes", &["shift-k"], Some("PianoRoll")),
    a("notes.mode_strum", "piano_roll::PrModeStrum", "Strum", "Notes", &["shift-j"], Some("PianoRoll")),
    a("notes.glue", "piano_roll::PrGlue", "Glue", "Notes", &["shift-g"], Some("PianoRoll")),
    a("notes.fill", "piano_roll::PrFill", "Fill", "Notes", &["shift-f"], Some("PianoRoll")),
    a("notes.cut_overlaps", "piano_roll::PrCutOverlaps", "Cut overlaps", "Notes", &["shift-c"], Some("PianoRoll")),
    a("notes.reverse", "piano_roll::PrReverse", "Reverse Notes", "Notes", &["shift-r"], Some("PianoRoll")),
    a("notes.select_all", "piano_roll::PrSelectAll", "Select all notes", "Notes", &["secondary-a"], Some("PianoRoll")),
    a(
        "notes.select_none",
        "piano_roll::PrSelectNone",
        "Select no notes",
        "Notes",
        &["secondary-shift-a", "escape"],
        Some("PianoRoll"),
    ),
    a("notes.copy", "piano_roll::PrCopy", "Copy", "Notes", &["secondary-c"], Some("PianoRoll")),
    a("notes.cut", "piano_roll::PrCut", "Cut", "Notes", &["secondary-x"], Some("PianoRoll")),
    a("notes.paste", "piano_roll::PrPaste", "Paste", "Notes", &["secondary-v"], Some("PianoRoll")),
    a("notes.delete", "piano_roll::PrDelete", "Delete notes", "Notes", &["delete", "backspace"], Some("PianoRoll")),
    a(
        "notes.transpose_up",
        "piano_roll::PrTransposeUp",
        "Transpose a semitone up",
        "Notes",
        &["shift-up"],
        Some("PianoRoll"),
    ),
    a(
        "notes.transpose_down",
        "piano_roll::PrTransposeDown",
        "Transpose a semitone down",
        "Notes",
        &["shift-down"],
        Some("PianoRoll"),
    ),
    a(
        "notes.octave_up",
        "piano_roll::PrOctaveUp",
        "Transpose an octave up",
        "Notes",
        &["secondary-up"],
        Some("PianoRoll"),
    ),
    a(
        "notes.octave_down",
        "piano_roll::PrOctaveDown",
        "Transpose an octave down",
        "Notes",
        &["secondary-down"],
        Some("PianoRoll"),
    ),
    a(
        "notes.move_left",
        "piano_roll::PrMoveLeft",
        "Move the notes a step left",
        "Notes",
        &["shift-left"],
        Some("PianoRoll"),
    ),
    a(
        "notes.move_right",
        "piano_roll::PrMoveRight",
        "Move the notes a step right",
        "Notes",
        &["shift-right"],
        Some("PianoRoll"),
    ),
    a(
        "notes.move_bar_left",
        "piano_roll::PrMoveBarLeft",
        "Move the notes a bar left",
        "Notes",
        &["secondary-left"],
        Some("PianoRoll"),
    ),
    a(
        "notes.move_bar_right",
        "piano_roll::PrMoveBarRight",
        "Move the notes a bar right",
        "Notes",
        &["secondary-right"],
        Some("PianoRoll"),
    ),
    a("desk.move_left", "mixing::MoveChannelLeft", "Move the channel left", "Desk", &["alt-left"], Some("Mixing")),
    a("desk.move_right", "mixing::MoveChannelRight", "Move the channel right", "Desk", &["alt-right"], Some("Mixing")),
    a("desk.remove_channel", "mixing::RemoveChannel", "Remove the channel", "Desk", &["delete"], Some("Mixing")),
    a("desk.rename_channel", "mixing::RenameChannel", "Rename the channel", "Desk", &["enter", "f2"], Some("Mixing")),
    a("browser.down", "browser::BrowserDown", "Next entry (previews it)", "Browser", &["down"], Some("Browser")),
    a("browser.up", "browser::BrowserUp", "Previous entry (previews it)", "Browser", &["up"], Some("Browser")),
    a(
        "browser.send",
        "browser::BrowserSend",
        "Send to the active track or channel",
        "Browser",
        &["enter"],
        Some("Browser"),
    ),
    a("browser.open_folder", "browser::BrowserOpenFolder", "Open the folder", "Browser", &["right"], Some("Browser")),
    a("browser.close_folder", "browser::BrowserCloseFolder", "Close the folder", "Browser", &["left"], Some("Browser")),
    a("browser.refresh", "browser::BrowserRefresh", "Read the folders again", "Browser", &["f5"], Some("Browser")),
    a("browser.find", "browser::BrowserFind", "Search the browser", "Browser", &["secondary-f"], Some("Browser")),
];

/// `secondary` written out for this platform: Cmd on macOS, Ctrl elsewhere
pub fn keys_for_platform(keys: &str) -> String {
    keys.split(' ')
        .map(|stroke| {
            if let Some(rest) = stroke.strip_prefix("secondary-") {
                if cfg!(target_os = "macos") { format!("cmd-{rest}") } else { format!("ctrl-{rest}") }
            } else {
                stroke.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The keys of an action: the user's change from the preferences' `shortcuts`, else the defaults
pub fn keys_of(def: &ActionDef, preferences: &Value) -> Vec<String> {
    match preferences.get("shortcuts").and_then(|s| s.get(def.id)).and_then(Value::as_text) {
        Some("") => Vec::new(),
        Some(user) => vec![user.to_string()],
        None => def.keys.iter().map(|k| k.to_string()).collect(),
    }
}

/// Binds every action of the registry, with the user's changes. A stroke that does not parse is skipped (and listed,
/// so the settings can say which)
pub fn bind_keys(cx: &mut App, preferences: &Value) -> Vec<&'static str> {
    let mut bad = Vec::new();
    let mut bindings = Vec::new();
    for def in REGISTRY {
        let Ok(action) = cx.build_action(def.gpui, None) else {
            bad.push(def.id);
            continue;
        };
        let context = def.context.and_then(|c| KeyBindingContextPredicate::parse(c).ok()).map(Rc::new);
        for keys in keys_of(def, preferences) {
            match KeyBinding::load(&keys_for_platform(&keys), action.boxed_clone(), context.clone(), false, None, cx) {
                Ok(b) => bindings.push(b),
                Err(_) => bad.push(def.id),
            }
        }
    }
    cx.bind_keys(bindings);
    bad
}

/// The registry's entry of an id
pub fn find(id: &str) -> Option<&'static ActionDef> {
    REGISTRY.iter().find(|d| d.id == id)
}

/// Changes an action's keys in the preferences (the settings' shortcuts page); the bindings are rebuilt by the caller
#[expect(dead_code, reason = "the settings' shortcuts page (agent C's) uses it")]
pub fn set_keys(port: &mut dyn CorePort, id: &str, keys: &str) {
    let _ = port.set_preference(vec![PathStep::key("shortcuts"), PathStep::key(id)], Value::text(keys));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_gpui_names_are_unique() {
        let mut ids: Vec<_> = REGISTRY.iter().map(|d| d.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), REGISTRY.len());
    }

    #[test]
    fn secondary_is_written_out() {
        let k = keys_for_platform("secondary-shift-z");
        assert!(k == "cmd-shift-z" || k == "ctrl-shift-z");
        assert_eq!(keys_for_platform("space"), "space");
    }

    #[test]
    fn the_users_keys_win() {
        let prefs = Value::object([("shortcuts", Value::object([("project.save", Value::text("f2"))]))]);
        assert_eq!(keys_of(find("project.save").unwrap(), &prefs), vec!["f2".to_string()]);
        assert_eq!(keys_of(find("edit.redo").unwrap(), &prefs).len(), 2);
    }
}
