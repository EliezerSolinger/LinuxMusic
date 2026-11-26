//! Plugins the Sampler replaces (decisoes-autonomas.md, 26/11, item 3). They stay in the registry so that projects
//! that use them still open and play, but they are left out of the lists a new instrument is chosen from; the old
//! crates are removed in a later pass.

/// (the deprecated plugin, what replaces it)
pub const REPLACED_BY_SAMPLER: &[(&str, &str)] = &[
    ("sf2player", "sampler (format sf2)"),
    ("gigplayer", "sampler (format gig, not implemented yet)"),
    ("patman", "sampler (format pat)"),
    ("audiofileprocessor", "sampler (format audio)"),
    ("avldrums", "sampler (format kit, an AVL kit)"),
    ("drummachines", "sampler (format kit, a DrumMachines kit)"),
];

/// Whether a plugin is left out of the lists of what to add to a new track
pub fn is_deprecated(id: &str) -> bool {
    REPLACED_BY_SAMPLER.iter().any(|(old, _)| *old == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_six_players_are_deprecated_and_the_sampler_is_not() {
        assert_eq!(REPLACED_BY_SAMPLER.len(), 6);
        assert!(is_deprecated("sf2player") && is_deprecated("drummachines"));
        assert!(!is_deprecated("sampler") && !is_deprecated("tripleoscillator"));
    }

    #[test]
    fn every_deprecated_plugin_is_still_registered() {
        let registry = crate::session::internal_plugins().expect("registry");
        for (old, _) in REPLACED_BY_SAMPLER {
            assert!(registry.get(old).is_some(), "{old} is not in the registry");
        }
        assert!(registry.get("sampler").is_some());
    }
}
