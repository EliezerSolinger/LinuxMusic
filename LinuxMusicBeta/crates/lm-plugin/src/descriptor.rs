//! The descriptor (interfaces.md, I-9.1; arquitetura-nova.md, piece 7): what a plugin is, its schema, its ports for a
//! state, what it does with velocity, and how it is built from the tree.

use std::fmt;

use lm_base::Result;
use lm_doc::Ports;
use lm_render::{ProcessSpec, Processor};
use lm_state::{NodeSchema, Value};

use crate::resources::ResourceView;

/// An instrument plays notes into its outputs; an effect (an insert) works in place on its channel's channels; a
/// note effect turns a track's note stream into another one before its instrument (no audio, no ports: chords, an
/// arpeggio, a transposition, a velocity curve), see [`crate::note_effect`]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginKind {
    Instrument,
    Effect,
    NoteEffect,
}

/// What a plugin does with a note's velocity (U28, decided in decisoes-autonomas.md: the velocity is 0 to 1, linear,
/// the response curve is the host's, per track; the plugin declares what it does with it, and the generated test
/// checks only that the response grows and agrees with the declaration)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Velocity {
    /// The velocity changes nothing: the same notes at any velocity give the same audio
    Ignored,
    /// The level follows it linearly: a note's gain is its velocity (1 is full level, 0.5 is −6 dB)
    Level,
    /// It shapes the timbre (and maybe the level): the sound changes with it, and its level never falls as the
    /// velocity grows
    Timbre,
}

/// The live data a plugin's window reads (F2, I-9.5): a fixed frame of this many bytes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplaySpec {
    pub bytes: usize,
}

/// What a build gets besides the state
#[derive(Debug)]
pub struct BuildContext<'a> {
    /// The project's rate and the negotiated block (C7)
    pub spec: ProcessSpec,
    /// The resources the state cites, loaded (C11: a build starts only when every one of them is)
    pub resources: &'a ResourceView,
    /// An effect: the channels of the channel it is inserted in (it works in place on all of them). An instrument:
    /// 0 (its outputs are its ports)
    pub channels: u16,
}

/// Builds a processor from a state (complete: every field and every parameter present, see
/// [`crate::Registry::build`]). Runs in an `Operation`, never on the audio thread: everything the processor will ever
/// need, for any value of any parameter, is allocated here (U31)
pub type BuildFn = fn(&Value, &BuildContext<'_>) -> Result<Box<dyn Processor>>;

/// The plugin's ports for a state (I-9.1): its outputs (the first is the main one) and its sidechain inputs. Port
/// keys are permanent, like parameter keys (C5)
pub type StatePortsFn = fn(&Value) -> Ports;

/// A state the plugin is pitched with, for the generated tuning tests (U35, U36): `key` played on `state` sounds
/// with fundamental `hz` (the period of the whole sound, which the test measures)
#[derive(Clone, Copy)]
pub struct TuningProbe {
    pub state: fn() -> Value,
    pub key: u8,
    pub hz: f64,
}

impl fmt::Debug for TuningProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TuningProbe").field("key", &self.key).field("hz", &self.hz).finish()
    }
}

/// What the generated test suite needs to know besides the schema
#[derive(Clone, Copy, Debug)]
pub struct TestHints {
    /// A pitched instrument's probe; `None` for an effect or an unpitched instrument (the tuning tests are skipped,
    /// the level tests use the default state)
    pub tuning: Option<TuningProbe>,
    /// Fields whose change the plugin cannot adopt (U32): the generated test expects `Adopted::No` for them, and
    /// `Adopted::Yes` for every other field
    pub no_adopt: &'static [&'static str],
    /// The voices the plugin has (the tests play more notes than this)
    pub voices: usize,
    /// The largest sample a valid state may give from the tests' input (about 0.5 at its peak): a sample above it, or
    /// one not finite, is a blowup. 1000 (+60 dB) by default; a plugin whose knobs legitimately add more gain than
    /// that (a multiband compressor with ±48 dB of input, output and band volume and 96 dB of upward range) declares
    /// its own, from its ranges
    pub max_level: f32,
    /// How long, in seconds, a note of the default state may still sound after its note-off: its own release (an
    /// emulated chip's envelope, a long decay). The short-notes test waits this long, then wants silence. 3.4 s by
    /// default
    pub tail: f64,
}

/// The default [`TestHints::tail`]
pub const DEFAULT_TAIL: f64 = 3.4;

impl TestHints {
    pub const fn new() -> Self {
        Self { tuning: None, no_adopt: &[], voices: 16, max_level: 1.0e3, tail: DEFAULT_TAIL }
    }
}

impl Default for TestHints {
    fn default() -> Self {
        Self::new()
    }
}

/// A plugin (I-9.1). Plugins declare one as a `static`
pub struct Descriptor {
    /// The permanent id (`"tripleoscillator"`): lowercase letters, digits and `-`. The state's schema is
    /// `"plugin:<id>"`
    pub id: &'static str,
    /// The translation key of its name (rule 7: never a ready text)
    pub name: &'static str,
    pub kind: PluginKind,
    /// The schemas: the first is `"plugin:<id>"` (its fields, its parameters in `"params"`, its version, upgrades
    /// and parameter aliases); the others are the object types it nests, named `"plugin:<id>.<name>"`
    pub schemas: fn() -> Vec<NodeSchema>,
    pub ports: StatePortsFn,
    pub velocity: Velocity,
    /// F2 (I-9.5)
    pub display: Option<DisplaySpec>,
    pub build: BuildFn,
    pub tests: TestHints,
}

impl fmt::Debug for Descriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Descriptor")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("velocity", &self.velocity)
            .finish_non_exhaustive()
    }
}

impl Descriptor {
    /// The type of its state in the schema registry
    pub fn type_name(&self) -> String {
        plugin_type(self.id)
    }

    /// Its main schema (the first of [`Descriptor::schemas`])
    pub fn schema(&self) -> NodeSchema {
        (self.schemas)().into_iter().next().unwrap_or_else(|| NodeSchema::new("plugin:invalid", 1))
    }
}

/// The schema type of a plugin's state (the same as `lm_doc::plugin_type`)
pub fn plugin_type(id: &str) -> String {
    lm_doc::schemas::plugin_type(id)
}

/// Whether an id is valid: lowercase ASCII letters, digits and `-`, starting with a letter, at most 64 bytes
pub fn is_valid_plugin_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}
