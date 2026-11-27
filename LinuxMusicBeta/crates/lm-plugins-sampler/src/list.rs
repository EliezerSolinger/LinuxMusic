//! The presets a bank file offers, for the panel's list and the `sampler.list_presets` action: read off the audio
//! thread, from the file's bytes, without decoding any sample.

use lm_base::{Error, Result};
use lm_sampler::PresetInfo;
use lm_sampler::import::{gig, sf2};
use lm_state::Value;

/// The kind of resource field a format reads its file from (`None`: the format has no presets)
pub fn field_of(format: &str) -> Option<&'static str> {
    match format {
        "sf2" => Some("soundfont"),
        "gig" => Some("gig"),
        _ => None,
    }
}

/// The presets of a bank file of `format` (`sf2` or `gig`). Keys: the importers' own, `sampler.list.format`
pub fn presets(format: &str, bytes: &[u8]) -> Result<Vec<PresetInfo>> {
    match format {
        "sf2" => sf2::list_presets(bytes),
        "gig" => gig::list_presets(bytes),
        _ => Err(Error::new("sampler.list.format").arg("format", format.to_owned())),
    }
}

/// The presets as the tree the panels read: a list of `{bank, program, name}`
pub fn to_value(presets: &[PresetInfo]) -> Value {
    Value::list(presets.iter().map(|p| {
        Value::object([
            ("bank", Value::int(i64::from(p.bank))),
            ("program", Value::int(i64::from(p.program))),
            ("name", Value::text(p.name.as_str())),
        ])
    }))
}
