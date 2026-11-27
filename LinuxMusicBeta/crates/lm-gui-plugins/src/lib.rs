//! The native plugins' own panels, faithful to the legacy (interfaces.md, I-22; docs/plugins-visual.md): the
//! registry `plugin id → panel`, the legacy widgets, and one panel per plugin that had its own look.
//!
//! Only the `CorePort`: a panel reads the plugin node in the port's tree, moves its parameters by gestures and
//! `param_write`, its choices and switches by edits, and draws the plugin's live data from `display(node)`. It
//! keeps no pointer to the processor, so it works the same in the client (RemotePort).
//!
//! How the interface hosts a panel (agreed with lm-gui): the instrument window (design 05) and the effect window
//! ask `has_panel(plugin)`; with a panel they show `open(...)` in place of the generic editor, at the panel's
//! fixed size (`panel_size`); without one they show the generic editor. The host is lm-gui's core entity, which
//! implements [`PortHost`].
//!
//! Layout: `assets` (the legacy files, embedded), `raster` (the CPU painter with QPainter's semantics), `widgets`
//! (the LMMS widgets), `spec` (a panel as data), `panel` (the view, the gestures, the menu), `panels/<plugin>.rs`
//! (one per plugin), `i18n` (the texts; the legacy English strings are the keys).

#![forbid(unsafe_code)]

pub mod assets;
pub mod graphs;
pub mod gxwidgets;
pub mod host;
pub mod i18n;
pub mod panel;
pub mod panels;
pub mod raster;
pub mod spec;
pub mod widgets;

use gpui::{AnyView, App, AppContext, Entity, Window};
use lm_port::NodeId;

pub use host::{Gesture, Gestures, Outcome, PortHost, guard_activation, release_guard};
pub use i18n::set_language;
pub use panel::{PanelEvent, PluginPanel};

/// Whether the plugin has its own panel (else the window shows the generic editor)
pub fn has_panel(plugin: &str) -> bool {
    panels::spec(plugin).is_some()
}

/// The panel's fixed size in logical pixels (the legacy view's)
pub fn panel_size(plugin: &str) -> Option<(f32, f32)> {
    panels::spec(plugin).map(|s| s.size)
}

/// The panel of a plugin node, or `None` for a plugin without one
pub fn open<H: PortHost>(
    plugin: &str,
    node: NodeId,
    host: Entity<H>,
    _window: &mut Window,
    cx: &mut App,
) -> Option<AnyView> {
    let spec = panels::spec(plugin)?;
    Some(cx.new(|cx| PluginPanel::new(host, node, spec, cx)).into())
}

/// The panel as its entity, for a host that subscribes to its [`panel::PanelEvent`]s (the binding window)
pub fn open_entity<H: PortHost>(
    plugin: &str,
    node: NodeId,
    host: Entity<H>,
    cx: &mut App,
) -> Option<Entity<PluginPanel<H>>> {
    let spec = panels::spec(plugin)?;
    Some(cx.new(|cx| PluginPanel::new(host, node, spec, cx)))
}

/// The plugins that have a panel, in the registry's order
pub fn plugins_with_panels() -> Vec<&'static str> {
    panels::ALL.iter().map(|(id, _)| *id).chain(panels::guitarix::ids()).collect()
}

// the views' tests find controls by `debug_selector`, which GPUI records only with debug assertions
#[cfg(all(test, debug_assertions))]
mod tests;
#[cfg(test)]
mod tests_data;
