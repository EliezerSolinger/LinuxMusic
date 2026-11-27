//! The registry: one entry per plugin with its own panel. To add one: its file, its line in `ALL`.

use crate::spec::PanelSpec;

pub mod ampandir;
pub mod bitinvader;
pub mod compressor;
pub mod crossovereq;
pub mod delay;
pub mod effects;
pub mod eq;
pub mod freeboy;
pub mod frequencyshifter;
pub mod granularpitchshifter;
pub mod guitarix;
mod guitarix_units;
pub mod kicker;
pub mod lb302;
pub mod lomm;
pub mod monstro;
pub mod multitapecho;
pub mod nes;
pub mod opulenz;
pub mod organic;
pub mod oscilloscope;
pub mod sampler;
mod sampler_wave;
pub mod sfxr;
pub mod shapers;
pub mod sid;
pub mod slicert;
pub mod slewdistortion;
pub mod spectrumanalyzer;
pub mod tripleoscillator;
pub mod vectorscope;
pub mod vibed;
pub mod watsyn;
pub mod xpressive;

type SpecFn = fn() -> &'static PanelSpec;

/// `plugin id → panel`
pub static ALL: &[(&str, SpecFn)] = &[
    (tripleoscillator::ID, || &tripleoscillator::PANEL),
    (ampandir::ID, || &ampandir::PANEL),
    (bitinvader::ID, || &bitinvader::PANEL),
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
    (frequencyshifter::ID, || &frequencyshifter::PANEL),
    (granularpitchshifter::ID, || &granularpitchshifter::PANEL),
    (lomm::ID, || &lomm::PANEL),
    (crossovereq::ID, || &crossovereq::PANEL),
    (delay::ID, || &delay::PANEL),
    (compressor::ID, || &compressor::PANEL),
    (eq::ID, || &eq::PANEL),
    (monstro::ID, || &monstro::PANEL),
    (vibed::ID, || &vibed::PANEL),
    (watsyn::ID, || &watsyn::PANEL),
    (xpressive::ID, || &xpressive::PANEL),
    (sampler::ID, || &sampler::PANEL),
    (slicert::ID, || &slicert::PANEL),
    (slewdistortion::ID, || &slewdistortion::PANEL),
    (multitapecho::ID, || &multitapecho::PANEL),
    (oscilloscope::ID, || &oscilloscope::PANEL),
    (vectorscope::ID, || &vectorscope::PANEL),
    (spectrumanalyzer::ID, || &spectrumanalyzer::PANEL),
    (shapers::DYNAMICS_ID, || &shapers::DYNAMICS_PANEL),
    (shapers::WAVESHAPER_ID, || &shapers::WAVESHAPER_PANEL),
];

pub fn spec(plugin: &str) -> Option<&'static PanelSpec> {
    if let Some(p) = guitarix::spec(plugin) {
        return Some(p);
    }
    ALL.iter().find(|(id, _)| *id == plugin).map(|(_, f)| f())
}

/// Every panel's images (the asset test decodes them all)
pub static ASSETS: &[&[&crate::assets::Asset]] = &[
    tripleoscillator::ASSETS,
    bitinvader::ASSETS,
    kicker::ASSETS,
    effects::ASSETS,
    lb302::ASSETS,
    sid::ASSETS,
    organic::ASSETS,
    sfxr::ASSETS,
    nes::ASSETS,
    frequencyshifter::ASSETS,
    granularpitchshifter::ASSETS,
    lomm::ASSETS,
    crossovereq::ASSETS,
    delay::ASSETS,
    compressor::ASSETS,
    eq::ASSETS,
    monstro::ASSETS,
    vibed::ASSETS,
    watsyn::ASSETS,
    xpressive::ASSETS,
    sampler::ASSETS,
    slicert::ASSETS,
    slewdistortion::ASSETS,
    multitapecho::ASSETS,
    oscilloscope::ASSETS,
    spectrumanalyzer::ASSETS,
    shapers::DYNAMICS_ASSETS,
    shapers::WAVESHAPER_ASSETS,
];
