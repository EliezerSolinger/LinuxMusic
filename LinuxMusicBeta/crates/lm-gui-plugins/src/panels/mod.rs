//! The registry: one entry per plugin with its own panel. To add one: its file, its line in `ALL`.

use crate::spec::PanelSpec;

pub mod effects;
pub mod freeboy;
pub mod kicker;
pub mod lb302;
pub mod nes;
pub mod opulenz;
pub mod organic;
pub mod sfxr;
pub mod shapers;
pub mod sid;
pub mod tripleoscillator;

type SpecFn = fn() -> &'static PanelSpec;

/// `plugin id → panel`
pub static ALL: &[(&str, SpecFn)] = &[
    (tripleoscillator::ID, || &tripleoscillator::PANEL),
    (kicker::ID, || &kicker::PANEL),
    ("bitcrush", || &effects::BITCRUSH_PANEL),
    ("bassbooster", || &effects::BASSBOOSTER_PANEL),
    ("reverbsc", || &effects::REVERBSC_PANEL),
    ("flanger", || &effects::FLANGER_PANEL),
    ("amplifier", || &effects::AMPLIFIER_PANEL),
    ("stereomatrix", || &effects::STEREOMATRIX_PANEL),
    ("dualfilter", || &effects::DUALFILTER_PANEL),
    (lb302::ID, || &lb302::PANEL),
    (sid::ID, || &sid::PANEL),
    (organic::ID, || &organic::PANEL),
    (sfxr::ID, || &sfxr::PANEL),
    (nes::ID, || &nes::PANEL),
    (freeboy::ID, || &freeboy::PANEL),
    (opulenz::ID, || &opulenz::PANEL),
    (shapers::DYNAMICS_ID, || &shapers::DYNAMICS_PANEL),
    (shapers::WAVESHAPER_ID, || &shapers::WAVESHAPER_PANEL),
];

pub fn spec(plugin: &str) -> Option<&'static PanelSpec> {
    ALL.iter().find(|(id, _)| *id == plugin).map(|(_, f)| f())
}

/// Every panel's images (the asset test decodes them all)
pub static ASSETS: &[&[&crate::assets::Asset]] = &[
    tripleoscillator::ASSETS,
    kicker::ASSETS,
    effects::ASSETS,
    lb302::ASSETS,
    sid::ASSETS,
    organic::ASSETS,
    sfxr::ASSETS,
    nes::ASSETS,
    shapers::DYNAMICS_ASSETS,
    shapers::WAVESHAPER_ASSETS,
];
