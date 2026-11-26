//! GIG (GigaStudio): **not implemented yet**. The GigPlayer read `.gig` files with libgig; the sampler does not link
//! it (decisoes-autonomas.md, 26/11, item 3), so a pure-Rust reader is the way, and it is a separate piece of work:
//! the DLS RIFF structure (`lins`, `lrgn`, `rgn`, `wvpl`), the `3ewa` chunk's dimension regions (envelopes, loops,
//! velocity attenuation, pitch tracking) and the dimension selection (velocity splits, release trigger, sustain
//! pedal), then mapping each dimension region to a [`crate::Zone`] (the release trigger to [`crate::Trigger::Release`],
//! the loop to [`crate::LoopMode`], the EG1 to [`crate::Ahdsr`]).
//!
//! Until then the importer fails with the key `sampler.gig.todo`, and the plugin keeps the GigPlayer in the registry
//! (deprecated) so that projects that use it still play.

use lm_base::{Error, Result};

use crate::model::{Instrument, PresetInfo};

/// TODO(gig): the pure-Rust reader of the GigPlayer's format. Always `sampler.gig.todo`
pub fn list_presets(_bytes: &[u8]) -> Result<Vec<PresetInfo>> {
    Err(Error::new("sampler.gig.todo"))
}

/// TODO(gig): see the module's documentation. Always `sampler.gig.todo`
pub fn import(_bytes: &[u8], _bank: u16, _program: u16) -> Result<Instrument> {
    Err(Error::new("sampler.gig.todo"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_is_a_marked_todo() {
        assert_eq!(super::import(&[], 0, 0).expect_err("todo").key(), "sampler.gig.todo");
    }
}
