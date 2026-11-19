//! What a build sees of the project's resources (interfaces.md, I-12, C11): every resource its state cites, already
//! loaded by `lm-io`'s `ResourceStore`. Immutable and shared: a processor keeps the `Arc` it needs, and the processor
//! goes back to the main thread to be dropped (rule 5), so no resource is ever freed on the audio thread.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use lm_base::{Error, Result};
use lm_render::SampleData;
use lm_state::Resource;

/// A loaded resource
#[derive(Clone, Debug, PartialEq)]
pub enum Loaded {
    /// Decoded audio at the project's rate (`kind` `"audio"`)
    Audio(Arc<SampleData>),
    /// Bytes read whole (a wave table, an opaque state of more than 64 KB, U24)
    Bytes(Arc<[u8]>),
    /// A file a library reads itself (a SoundFont, a kit): its path, inside a folder the user allowed (C9)
    File(Arc<Path>),
}

/// The loaded resources a build may read, by [`Resource`]
#[derive(Clone, Debug, Default)]
pub struct ResourceView {
    loaded: BTreeMap<Resource, Loaded>,
}

impl ResourceView {
    /// No resource (a state that cites none)
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, resource: Resource, loaded: Loaded) -> &mut Self {
        self.loaded.insert(resource, loaded);
        self
    }

    #[must_use]
    pub fn with(mut self, resource: Resource, loaded: Loaded) -> Self {
        self.insert(resource, loaded);
        self
    }

    pub fn get(&self, resource: &Resource) -> Option<&Loaded> {
        self.loaded.get(resource)
    }

    pub fn contains(&self, resource: &Resource) -> bool {
        self.loaded.contains_key(resource)
    }

    /// Every resource held
    pub fn resources(&self) -> impl Iterator<Item = &Resource> + '_ {
        self.loaded.keys()
    }

    /// The decoded audio of a resource. Keys: `plugin.resource.missing` (not loaded: the session should not have
    /// started the build, C11), `plugin.resource.kind` (loaded as something else)
    pub fn audio(&self, resource: &Resource) -> Result<Arc<SampleData>> {
        match self.loaded.get(resource) {
            Some(Loaded::Audio(a)) => Ok(Arc::clone(a)),
            Some(_) => Err(kind_error(resource, "audio")),
            None => Err(missing(resource)),
        }
    }

    /// The bytes of a resource (keys as [`ResourceView::audio`])
    pub fn bytes(&self, resource: &Resource) -> Result<Arc<[u8]>> {
        match self.loaded.get(resource) {
            Some(Loaded::Bytes(b)) => Ok(Arc::clone(b)),
            Some(_) => Err(kind_error(resource, "bytes")),
            None => Err(missing(resource)),
        }
    }

    /// The path of a file resource (keys as [`ResourceView::audio`])
    pub fn file(&self, resource: &Resource) -> Result<Arc<Path>> {
        match self.loaded.get(resource) {
            Some(Loaded::File(p)) => Ok(Arc::clone(p)),
            Some(_) => Err(kind_error(resource, "file")),
            None => Err(missing(resource)),
        }
    }
}

fn missing(resource: &Resource) -> Error {
    Error::new("plugin.resource.missing").arg("kind", resource.kind()).arg("key", resource.key())
}

fn kind_error(resource: &Resource, expected: &'static str) -> Error {
    Error::new("plugin.resource.kind").arg("key", resource.key()).arg("expected", expected)
}
