//! The interface's action registry (I-19): every command the user can give, with its id, label, group, default keys
//! and where the keys work. The keybindings are built from it (with the user's changes from the preferences'
//! `keymaps`), the command palette lists it, the menus will read it. Commands of the core go through
//! `CorePort::run_action`; the others are the interface's own.
//!
//! Keys are written with `secondary` for Cmd on macOS and Ctrl elsewhere; this GPUI's parser has no `secondary`, so
//! `keys_for_platform` writes it out.

use std::rc::Rc;

use gpui::{App, KeyBinding, KeyBindingContextPredicate, actions};
use lm_port::Value;

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
        ImportFiles,
        ExportMidi,
        // the shell's menus (P3)
        SaveVersion,
        SaveTemplate,
        ShowPianoRoll,
        ShowAutomation,
        ShowSteps,
        ToggleBrowser,
        ToggleFullscreen,
        DetachPage,
        DetachAll,
        DockAll,
        TapTempo,
        ToggleMasterMute,
        OpenManual,
    ]
);

actions!(
    composition,
    [
        Split,
        Duplicate,
        DeleteItem,
        EmptyItem,
        OpenItem,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        AddMarker,
        AddRegion,
        TempoHere,
        Marker1,
        Marker2,
        Marker3,
        Marker4,
        Marker5,
        Marker6,
        Marker7,
        Marker8,
        Marker9,
        ArrCopy,
        ArrCut,
        ArrPaste,
        Glue,
        SelectAllItems,
        SelectNoItems,
        BarLeft,
        BarRight,
        InsertBar,
        RemoveBar,
        ToolDraw,
        ToolKnife,
        ToolSelect
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
        PrMoveBarRight,
        PrPrevClip,
        PrNextClip,
        PrKbOctaveDown,
        PrKbOctaveUp,
        PrStepRest,
        PrStepBack,
        PrStepShorter,
        PrStepLonger,
        PrScrollUp,
        PrScrollDown,
        PrHome,
        PrLen0,
        PrLen1,
        PrLen2,
        PrLen3,
        PrLen4,
        PrLen5,
        PrLen6,
        PrLen7,
        PrLen8,
        PrLen9,
        PrGrid0,
        PrGrid1,
        PrGrid2,
        PrGrid3,
        PrGrid4,
        PrGrid5,
        PrGrid6,
        PrGrid7,
        PrGrid8,
        PrGrid9
    ]
);

actions!(
    automation,
    [
        AuSelectAll,
        AuSelectNone,
        AuCopy,
        AuCut,
        AuPaste,
        AuDelete,
        AuLeft,
        AuRight,
        AuHome,
        AuPlay,
        AuToolDraw,
        AuToolErase,
        AuToolOut,
        AuToolPencil,
        AuToolSelect
    ]
);

actions!(
    browser,
    [BrowserUp, BrowserDown, BrowserSend, BrowserOpenFolder, BrowserCloseFolder, BrowserRefresh, BrowserFind]
);

/// One entry of the registry
#[derive(Clone, Copy, Debug)]
pub struct ActionDef {
    /// The registry's id (I-19), stable: the preferences' `keymaps` and the presets use it
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
    a("file.import", "lm::ImportFiles", "Import (MIDI, audio)", "Project", &["secondary-i"], None),
    a("file.export_midi", "lm::ExportMidi", "Export MIDI", "Project", &["secondary-m"], None),
    a("view.notices", "lm::OpenNotices", "Notices and activities", "View", &[], None),
    a("file.save_version", "lm::SaveVersion", "Save as a new version", "Project", &["secondary-alt-s"], None),
    a("file.save_template", "lm::SaveTemplate", "Save as a template", "Project", &[], None),
    a("view.piano_roll", "lm::ShowPianoRoll", "Piano Roll", "View", &["secondary-4"], None),
    a("view.automation", "lm::ShowAutomation", "Automation", "View", &["secondary-5"], None),
    a("view.steps", "lm::ShowSteps", "Step editor", "View", &["secondary-6"], None),
    a("view.browser", "lm::ToggleBrowser", "Browser", "View", &["secondary-8"], None),
    a("view.fullscreen", "lm::ToggleFullscreen", "Full screen", "View", &["f11"], None),
    a("view.detach", "lm::DetachPage", "Detach the page", "View", &[], None),
    a("view.detach_all", "lm::DetachAll", "Detach all", "View", &[], None),
    a("view.dock_all", "lm::DockAll", "Dock all", "View", &[], None),
    a("tools.tap_tempo", "lm::TapTempo", "Tap Tempo", "Tools", &["secondary-t"], None),
    a("transport.master_mute", "lm::ToggleMasterMute", "Mute the master", "Transport", &[], None),
    a("help.manual", "lm::OpenManual", "Manual", "Help", &[], None),
    a("help.about", "lm::OpenAbout", "About LinuxMusic", "Help", &["f1"], None),
    a("arrange.split_at_cursor", "composition::Split", "Split at the cursor", "Arrange", &["s"], Some("Composition")),
    a(
        "arrange.duplicate_selection",
        "composition::Duplicate",
        "Duplicate",
        "Arrange",
        &["secondary-d"],
        Some("Composition"),
    ),
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
    a("arrange.copy_selection", "composition::ArrCopy", "Copy", "Arrange", &["secondary-c"], Some("Composition")),
    a("arrange.cut_selection", "composition::ArrCut", "Cut", "Arrange", &["secondary-x"], Some("Composition")),
    a(
        "arrange.paste_at_cursor",
        "composition::ArrPaste",
        "Paste at the cursor",
        "Arrange",
        &["secondary-v"],
        Some("Composition"),
    ),
    a(
        "clip.glue_selection",
        "composition::Glue",
        "Join the selected items",
        "Arrange",
        &["secondary-j"],
        Some("Composition"),
    ),
    a(
        "arrange.select_all",
        "composition::SelectAllItems",
        "Select all items",
        "Arrange",
        &["secondary-a"],
        Some("Composition"),
    ),
    a(
        "arrange.select_none",
        "composition::SelectNoItems",
        "Select none",
        "Arrange",
        &["secondary-shift-a", "escape"],
        Some("Composition"),
    ),
    a("arrange.bar_left", "composition::BarLeft", "A bar back", "Arrange", &["left"], Some("Composition")),
    a("arrange.bar_right", "composition::BarRight", "A bar forward", "Arrange", &["right"], Some("Composition")),
    a(
        "arrange.insert_bar_at_cursor",
        "composition::InsertBar",
        "Insert a bar",
        "Arrange",
        &["shift-insert", "shift-enter"],
        Some("Composition"),
    ),
    a(
        "arrange.remove_bar_at_cursor",
        "composition::RemoveBar",
        "Remove a bar",
        "Arrange",
        &["shift-delete", "shift-backspace"],
        Some("Composition"),
    ),
    a("arrange.tool_draw", "composition::ToolDraw", "Draw tool", "Arrange", &["d"], Some("Composition")),
    a("arrange.tool_knife", "composition::ToolKnife", "Knife tool", "Arrange", &["k"], Some("Composition")),
    a("arrange.tool_select", "composition::ToolSelect", "Select tool", "Arrange", &["v"], Some("Composition")),
    a("arrange.zoom_in", "composition::ZoomIn", "Zoom in", "Arrange", &["="], Some("Composition")),
    a("arrange.zoom_out", "composition::ZoomOut", "Zoom out", "Arrange", &["-"], Some("Composition")),
    a("arrange.zoom_reset", "composition::ZoomReset", "Reset the zoom", "Arrange", &["ctrl-0"], Some("Composition")),
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
    a(
        "tempo.here",
        "composition::TempoHere",
        "Tempo change at the cursor",
        "Arrange",
        &["shift-t"],
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
    a(
        "desk.fader_down_fine",
        "mixing::FaderDownFine",
        "Fader down 0.1 dB",
        "Desk",
        &["secondary-down"],
        Some("Mixing"),
    ),
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
        "notes.prev_clip",
        "piano_roll::PrPrevClip",
        "Previous clip of the track",
        "Notes",
        &["alt-left"],
        Some("PianoRoll"),
    ),
    a(
        "notes.next_clip",
        "piano_roll::PrNextClip",
        "Next clip of the track",
        "Notes",
        &["alt-right"],
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
    a("notes.kb_octave_down", "piano_roll::PrKbOctaveDown", "Keyboard octave down", "Notes", &["-"], Some("PianoRoll")),
    a("notes.kb_octave_up", "piano_roll::PrKbOctaveUp", "Keyboard octave up", "Notes", &["="], Some("PianoRoll")),
    a("notes.step_rest", "piano_roll::PrStepRest", "Step input: a rest", "Notes", &["right"], Some("PianoRoll")),
    a("notes.step_back", "piano_roll::PrStepBack", "Step input: back", "Notes", &["left"], Some("PianoRoll")),
    a(
        "notes.step_shorter",
        "piano_roll::PrStepShorter",
        "Step input: shorter",
        "Notes",
        &["alt-left"],
        Some("PianoRoll"),
    ),
    a(
        "notes.step_longer",
        "piano_roll::PrStepLonger",
        "Step input: longer",
        "Notes",
        &["alt-right"],
        Some("PianoRoll"),
    ),
    a("notes.scroll_up", "piano_roll::PrScrollUp", "Scroll up", "Notes", &["up"], Some("PianoRoll")),
    a("notes.scroll_down", "piano_roll::PrScrollDown", "Scroll down", "Notes", &["down"], Some("PianoRoll")),
    a("notes.home", "piano_roll::PrHome", "Back to the start", "Notes", &["home"], Some("PianoRoll")),
    a(
        "notes.length_0",
        "piano_roll::PrLen0",
        "Note length: the last one",
        "Notes",
        &["secondary-0"],
        Some("PianoRoll"),
    ),
    a("notes.length_1", "piano_roll::PrLen1", "Note length 1", "Notes", &["secondary-1"], Some("PianoRoll")),
    a("notes.length_2", "piano_roll::PrLen2", "Note length 2", "Notes", &["secondary-2"], Some("PianoRoll")),
    a("notes.length_3", "piano_roll::PrLen3", "Note length 3", "Notes", &["secondary-3"], Some("PianoRoll")),
    a("notes.length_4", "piano_roll::PrLen4", "Note length 4", "Notes", &["secondary-4"], Some("PianoRoll")),
    a("notes.length_5", "piano_roll::PrLen5", "Note length 5", "Notes", &["secondary-5"], Some("PianoRoll")),
    a("notes.length_6", "piano_roll::PrLen6", "Note length 6", "Notes", &["secondary-6"], Some("PianoRoll")),
    a("notes.length_7", "piano_roll::PrLen7", "Note length 7", "Notes", &["secondary-7"], Some("PianoRoll")),
    a("notes.length_8", "piano_roll::PrLen8", "Note length 8", "Notes", &["secondary-8"], Some("PianoRoll")),
    a("notes.length_9", "piano_roll::PrLen9", "Note length 9", "Notes", &["secondary-9"], Some("PianoRoll")),
    a("notes.grid_0", "piano_roll::PrGrid0", "Quantization 0", "Notes", &["alt-0"], Some("PianoRoll")),
    a("notes.grid_1", "piano_roll::PrGrid1", "Quantization 1", "Notes", &["alt-1"], Some("PianoRoll")),
    a("notes.grid_2", "piano_roll::PrGrid2", "Quantization 2", "Notes", &["alt-2"], Some("PianoRoll")),
    a("notes.grid_3", "piano_roll::PrGrid3", "Quantization 3", "Notes", &["alt-3"], Some("PianoRoll")),
    a("notes.grid_4", "piano_roll::PrGrid4", "Quantization 4", "Notes", &["alt-4"], Some("PianoRoll")),
    a("notes.grid_5", "piano_roll::PrGrid5", "Quantization 5", "Notes", &["alt-5"], Some("PianoRoll")),
    a("notes.grid_6", "piano_roll::PrGrid6", "Quantization 6", "Notes", &["alt-6"], Some("PianoRoll")),
    a("notes.grid_7", "piano_roll::PrGrid7", "Quantization 7", "Notes", &["alt-7"], Some("PianoRoll")),
    a("notes.grid_8", "piano_roll::PrGrid8", "Quantization 8", "Notes", &["alt-8"], Some("PianoRoll")),
    a("notes.grid_9", "piano_roll::PrGrid9", "Quantization 9", "Notes", &["alt-9"], Some("PianoRoll")),
    a(
        "auto.select_all",
        "automation::AuSelectAll",
        "Select all points",
        "Automation",
        &["secondary-a"],
        Some("AutomationEditor"),
    ),
    a(
        "auto.select_none",
        "automation::AuSelectNone",
        "Select no points",
        "Automation",
        &["secondary-shift-a", "escape"],
        Some("AutomationEditor"),
    ),
    a("auto.copy", "automation::AuCopy", "Copy", "Automation", &["secondary-c"], Some("AutomationEditor")),
    a("auto.cut", "automation::AuCut", "Cut", "Automation", &["secondary-x"], Some("AutomationEditor")),
    a("auto.paste", "automation::AuPaste", "Paste", "Automation", &["secondary-v"], Some("AutomationEditor")),
    a(
        "auto.delete",
        "automation::AuDelete",
        "Delete points",
        "Automation",
        &["delete", "backspace"],
        Some("AutomationEditor"),
    ),
    a("auto.left", "automation::AuLeft", "Cursor a step back", "Automation", &["left"], Some("AutomationEditor")),
    a("auto.right", "automation::AuRight", "Cursor a step forward", "Automation", &["right"], Some("AutomationEditor")),
    a("auto.home", "automation::AuHome", "Back to the start", "Automation", &["home"], Some("AutomationEditor")),
    a("auto.play", "automation::AuPlay", "Play or stop", "Automation", &["space"], Some("AutomationEditor")),
    a("auto.tool_draw", "automation::AuToolDraw", "Draw tool", "Automation", &["shift-d"], Some("AutomationEditor")),
    a("auto.tool_erase", "automation::AuToolErase", "Erase tool", "Automation", &["shift-e"], Some("AutomationEditor")),
    a(
        "auto.tool_out",
        "automation::AuToolOut",
        "Outgoing value tool",
        "Automation",
        &["shift-c"],
        Some("AutomationEditor"),
    ),
    a(
        "auto.tool_pencil",
        "automation::AuToolPencil",
        "Pencil tool",
        "Automation",
        &["shift-p"],
        Some("AutomationEditor"),
    ),
    a(
        "auto.tool_select",
        "automation::AuToolSelect",
        "Select tool",
        "Automation",
        &["shift-s"],
        Some("AutomationEditor"),
    ),
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

/// The keys of an action: the user's change, else the preset chosen for its context, else the defaults
pub fn keys_of(def: &ActionDef, preferences: &Value) -> Vec<String> {
    crate::keymap::keys_for(def, preferences, &crate::keymap::user_presets())
}

/// Binds every action of the registry, with the user's changes. A stroke that does not parse is skipped (and listed,
/// so the settings can say which)
pub fn bind_keys(cx: &mut App, preferences: &Value) -> Vec<&'static str> {
    crate::keymap::install(preferences);
    let mut bad = Vec::new();
    let mut bindings = Vec::new();
    for def in REGISTRY {
        let Ok(action) = cx.build_action(def.gpui, None) else {
            bad.push(def.id);
            continue;
        };
        for keys in keys_of(def, preferences) {
            // plain typing belongs to a text field: such a key works only while none has the focus
            let text = match (def.context, crate::keymap::is_typing(&keys)) {
                (Some(c), true) => format!("{c} && !TextEntry"),
                (None, true) => "!TextEntry".to_string(),
                (Some(c), false) => c.to_string(),
                (None, false) => String::new(),
            };
            let context = KeyBindingContextPredicate::parse(&text).ok().filter(|_| !text.is_empty()).map(Rc::new);
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

    /// An id of the interface's arrangement actions must not be a core action's (a MIDI mapping runs a core action
    /// without the interface's parameters)
    #[test]
    fn the_arrangement_ids_are_not_core_actions() {
        for d in REGISTRY.iter().filter(|d| d.id.starts_with("arrange.") || d.id.starts_with("clip.")) {
            assert!(lm_session::actions::find(d.id).is_none(), "{} is also a core action", d.id);
        }
    }

    #[test]
    fn secondary_is_written_out() {
        let k = keys_for_platform("secondary-shift-z");
        assert!(k == "cmd-shift-z" || k == "ctrl-shift-z");
        assert_eq!(keys_for_platform("space"), "space");
    }

    #[test]
    fn the_users_keys_win() {
        let global = Value::object([("keys", Value::object([("project.save", Value::text("f2"))]))]);
        let prefs = Value::object([("keymaps", Value::object([("Global", global)]))]);
        assert_eq!(keys_of(find("project.save").unwrap(), &prefs), vec!["f2".to_string()]);
        assert_eq!(keys_of(find("edit.redo").unwrap(), &prefs).len(), 2);
    }
}
