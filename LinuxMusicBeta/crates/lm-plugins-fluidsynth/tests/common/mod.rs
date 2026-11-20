//! What the tests share: the test SoundFont, given for any SoundFont resource

#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, OnceLock};

use lm_plugin::Loaded;
use lm_state::Resource;

/// The test font (a sine at 440 Hz as program 0, noise as program 1, bank 0) for any `soundfont` resource
pub fn test_resources(resource: &Resource, _rate: u32) -> Option<Loaded> {
    static FONT: OnceLock<Arc<Path>> = OnceLock::new();
    if resource.kind() != "soundfont" {
        return None;
    }
    let path = FONT.get_or_init(|| {
        let path = std::env::temp_dir().join(format!("lm-plugins-fluidsynth-{}.sf2", std::process::id()));
        lm_sys_fluidsynth::test_font::standard(&path).expect("the test font");
        Arc::from(path.as_path())
    });
    Some(Loaded::File(Arc::clone(path)))
}
