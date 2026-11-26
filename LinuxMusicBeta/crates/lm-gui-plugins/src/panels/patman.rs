//! Patman (legado/plugins/Patman/Patman.cpp, `PatmanView`): the artwork, the patch's file name painted at (8, 116) in
//! the small font, the open button (227, 86) and the loop and tune switches (195, 138 and 223, 138). A patch is a
//! Gravis Ultrasound `.pat`: the open button asks for the file and sends its bytes to the core's `patman.load_patch`
//! action (`{node, field, bytes}`), which reads it and sets the `patch` resource; a `.pat` dropped on the panel does
//! the same. The name shows the resource's kind and the start of its hash until the core keeps the file's name.

use std::sync::LazyLock;

use crate::asset;
use crate::assets::Asset;
use crate::spec::{Bind, FileDrop, PanelSpec, button, readout};

pub const ID: &str = "patman";

static ARTWORK: Asset = asset!("patman/artwork.png");
static SELECT_FILE: Asset = asset!("patman/select_file.png");
static SELECT_FILE_ON: Asset = asset!("patman/select_file_on.png");
static LOOP_ON: Asset = asset!("patman/loop_on.png");
static LOOP_OFF: Asset = asset!("patman/loop_off.png");
static TUNE_ON: Asset = asset!("patman/tune_on.png");
static TUNE_OFF: Asset = asset!("patman/tune_off.png");
pub static ASSETS: &[&Asset] = &[&ARTWORK, &SELECT_FILE, &SELECT_FILE_ON, &LOOP_ON, &LOOP_OFF, &TUNE_ON, &TUNE_OFF];

/// The core's action that reads a patch file into the `patch` field
pub const LOAD_ACTION: &str = "patman.load_patch";

pub static PANEL: LazyLock<PanelSpec> = LazyLock::new(|| PanelSpec {
    plugin: ID,
    size: (250.0, 250.0),
    background: Some(&ARTWORK),
    view_backgrounds: &[],
    fill: 0x000000,
    controls: vec![
        // a 16-pixel line centered on the baseline the legacy gives (8, 116, 235, 16)
        readout("patch", 8.0, 118.0, 235.0, 0xd1d8e4, "No file selected").small(),
        button(
            Bind::Open { field: "patch", action: Some(LOAD_ACTION), extensions: &["pat"] },
            "openPatch",
            227.0,
            86.0,
            &SELECT_FILE_ON,
            &SELECT_FILE,
            "Open patch",
        )
        .tip("Open patch"),
        button(Bind::Switch { param: "looped" }, "looped", 195.0, 138.0, &LOOP_ON, &LOOP_OFF, "Loop").tip("Loop mode"),
        button(Bind::Switch { param: "tuned" }, "tuned", 223.0, 138.0, &TUNE_ON, &TUNE_OFF, "Tune").tip("Tune mode"),
    ],
    labels: Vec::new(),
    file_drop: Some(FileDrop::Action { action: LOAD_ACTION, extensions: &["pat"] }),
});
