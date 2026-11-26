//! What the tests share: the test `.gig`, given for any `gig` resource

#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, OnceLock};

use lm_plugin::Loaded;
use lm_state::Resource;

/// The test file (a sine at 440 Hz as bank 0 program 0, an octave up as program 1) for any `gig` resource
pub fn test_resources(resource: &Resource, _rate: u32) -> Option<Loaded> {
    static FILE: OnceLock<Arc<Path>> = OnceLock::new();
    if resource.kind() != "gig" {
        return None;
    }
    let path = FILE.get_or_init(|| {
        let path = std::env::temp_dir().join(format!("lm-plugins-gigplayer-{}.gig", std::process::id()));
        lm_sys_gig::test_file(&path).expect("the test file");
        Arc::from(path.as_path())
    });
    Some(Loaded::File(Arc::clone(path)))
}
