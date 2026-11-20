//! The registry: one entry per plugin with its own panel. To add one: its file, its line in `ALL`.

use crate::spec::PanelSpec;

pub mod kicker;
pub mod tripleoscillator;

type SpecFn = fn() -> &'static PanelSpec;

/// `plugin id → panel`
pub static ALL: &[(&str, SpecFn)] =
    &[(tripleoscillator::ID, || &tripleoscillator::PANEL), (kicker::ID, || &kicker::PANEL)];

pub fn spec(plugin: &str) -> Option<&'static PanelSpec> {
    ALL.iter().find(|(id, _)| *id == plugin).map(|(_, f)| f())
}

/// Every panel's images (the asset test decodes them all)
pub static ASSETS: &[&[&crate::assets::Asset]] = &[tripleoscillator::ASSETS, kicker::ASSETS];
