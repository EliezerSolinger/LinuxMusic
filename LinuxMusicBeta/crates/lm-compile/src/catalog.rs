//! What the compiler needs to know of the plugins and of the resources, as two small traits.
//!
//! The compiler never builds a plugin (T6): it only needs a plugin's **shape** for a state (its main input, its
//! output ports, its sidechain ports, whether it takes notes) and its **parameters** (from the plugin's schema in
//! the plugin's schema, in the schema's order: the index of a parameter is its position there). `lm-plugin`'s
//! [`lm_plugin::Registry`] implements [`PluginCatalog`], and its [`lm_plugin::ResourceView`] implements
//! [`Resources`].

use std::sync::Arc;

use lm_doc::Port;
use lm_render::SampleData;
use lm_state::{ParamSpec, Resource, Value};

/// The audio and note shape of a plugin for one state
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginShape {
    /// Channels of the main input: 0 for an instrument
    pub inputs: u16,
    /// The output ports, the first being the main one. An insert uses only the main one
    pub outputs: Vec<Port>,
    /// The sidechain input ports a send can feed (`"port"` of a send)
    pub sidechains: Vec<Port>,
    /// It takes notes (an instrument): the track's MIDI items and live MIDI go to it
    pub notes: bool,
}

impl PluginShape {
    /// An instrument with these output ports
    pub fn instrument(outputs: Vec<Port>) -> Self {
        Self { inputs: 0, outputs, sidechains: Vec::new(), notes: true }
    }

    /// An effect working on `channels` channels in and out
    pub fn effect(channels: u16) -> Self {
        Self {
            inputs: channels,
            outputs: vec![Port::new("main", u32::from(channels))],
            sidechains: Vec::new(),
            notes: false,
        }
    }

    /// The channels of all the output ports together
    pub fn output_channels(&self) -> u16 {
        self.outputs.iter().map(|p| clamp_channels(p.channels)).fold(0u16, u16::saturating_add)
    }

    /// The channels of all the sidechain ports together
    pub fn sidechain_channels(&self) -> u16 {
        self.sidechains.iter().map(|p| clamp_channels(p.channels)).fold(0u16, u16::saturating_add)
    }

    /// Where port `key` starts among the outputs, and its channels
    pub fn output_range(&self, key: &str) -> Option<(u16, u16)> {
        range_of(&self.outputs, key)
    }

    /// Where sidechain port `key` starts among the sidechain channels, and its channels
    pub fn sidechain_range(&self, key: &str) -> Option<(u16, u16)> {
        range_of(&self.sidechains, key)
    }
}

pub(crate) fn clamp_channels(c: u32) -> u16 {
    u16::try_from(c.min(64)).unwrap_or(64)
}

fn range_of(ports: &[Port], key: &str) -> Option<(u16, u16)> {
    let mut start = 0u16;
    for p in ports {
        let c = clamp_channels(p.channels);
        if &*p.key == key {
            return Some((start, c));
        }
        start = start.saturating_add(c);
    }
    None
}

/// The plugin layer, as the compiler sees it (`lm-plugin`'s `Registry` implements it)
pub trait PluginCatalog: Send + Sync {
    /// The shape of `plugin` at `version` for `state`. `channels` is the width of the desk channel an insert sits
    /// in (an effect that adapts answers with that width); for an instrument it is the track's `channels`. `None`:
    /// a plugin the catalog does not know — the node stays silent and is never built
    fn shape(&self, plugin: &str, version: u32, state: &Value, channels: u16) -> Option<PluginShape>;

    /// The parameters of `plugin`, in their index order (the processor's `Param { index }`). `None` for an unknown
    /// plugin
    fn params(&self, plugin: &str) -> Option<&[ParamSpec]>;

    /// An effect: built for the width of its channel (`BuildContext::channels`); an instrument is built with 0
    fn is_effect(&self, plugin: &str) -> bool;
}

impl PluginCatalog for lm_plugin::Registry {
    fn shape(&self, plugin: &str, _version: u32, state: &Value, channels: u16) -> Option<PluginShape> {
        let kind = self.kind(plugin)?;
        let ports = self.ports(plugin, state)?;
        let mut shape = match kind {
            lm_plugin::PluginKind::Instrument => PluginShape::instrument(ports.outputs),
            // An effect works in place on all the channels of the channel it is inserted in
            lm_plugin::PluginKind::Effect => PluginShape::effect(channels),
        };
        shape.sidechains = ports.sidechains;
        Some(shape)
    }

    fn params(&self, plugin: &str) -> Option<&[ParamSpec]> {
        lm_plugin::Registry::params(self, plugin)
    }

    fn is_effect(&self, plugin: &str) -> bool {
        self.kind(plugin) == Some(lm_plugin::PluginKind::Effect)
    }
}

/// Whether an audio resource can be played
#[derive(Clone, Debug)]
pub enum ResourceState {
    /// Decoded at the project's rate
    Ready(Arc<SampleData>),
    /// Long (P-32, above 30 s): not decoded in memory but read from the disk ahead of the playhead, at the
    /// project's rate. The compiler makes a stream per played segment (`Compiled::streams`)
    OnDisk { rate: u32, channels: usize, frames: u64 },
    /// Being loaded: the items that use it are compiled as waiting (silence) until it is
    Loading,
    /// Not where the project says (H17): silence, and the session shows "locate file"
    Missing,
}

/// What is loaded now (`lm-io`'s `ResourceStore::view()` implements it; immutable, so it can go to the compile
/// thread)
pub trait Resources: Send + Sync {
    fn audio(&self, resource: &Resource) -> ResourceState;
}

/// No resources at all (every audio item waits)
#[derive(Clone, Copy, Debug, Default)]
pub struct NoResources;

impl Resources for NoResources {
    fn audio(&self, _: &Resource) -> ResourceState {
        ResourceState::Loading
    }
}

impl Resources for lm_plugin::ResourceView {
    fn audio(&self, resource: &Resource) -> ResourceState {
        match self.get(resource) {
            Some(lm_plugin::Loaded::Audio(data)) => ResourceState::Ready(data.clone()),
            // Not audio: an item cannot play it
            Some(_) => ResourceState::Missing,
            None => ResourceState::Loading,
        }
    }
}
